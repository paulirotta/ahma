//! HTTP MCP transport implementation.
//!
//! This module provides the `HttpMcpTransport`, which implements the MCP transport
//! over HTTP and manages optional OAuth2 authentication for Atlassian-compatible
//! providers.

use crate::error::{McpHttpError, Result};
use ahma_common::state_machine::StateMachine;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge, RedirectUrl,
    Scope, TokenResponse, TokenUrl, basic::BasicClient,
};
use rmcp::{
    RoleClient,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::Transport,
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{BufReader, BufWriter, Write},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::{Mutex, Notify, mpsc};
use tracing::{error, info};
use url::Url;

type ConfiguredOAuthClient = oauth2::Client<
    oauth2::StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
    oauth2::StandardTokenResponse<oauth2::EmptyExtraTokenFields, oauth2::basic::BasicTokenType>,
    oauth2::StandardTokenIntrospectionResponse<
        oauth2::EmptyExtraTokenFields,
        oauth2::basic::BasicTokenType,
    >,
    oauth2::StandardRevocableToken,
    oauth2::StandardErrorResponse<oauth2::RevocationErrorResponseType>,
    oauth2::EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointSet,
>;

const TOKEN_FILE_NAME: &str = "mcp_http_token.json";
/// Environment variable to override the token storage path.
const TOKEN_PATH_ENV: &str = "AHMA_HTTP_CLIENT_TOKEN_PATH";

/// HTTP Transport implementation for the Model Context Protocol (MCP).
///
/// This transport sends JSON-RPC messages over HTTP POST requests and supports
/// receiving responses. It handles OAuth2 authentication with Atlassian-compatible
/// providers, managing the token lifecycle (storage, loading).
///
/// # Token Storage
///
/// Tokens are stored in a JSON file. By default, this is `mcp_http_token.json` in the system's
/// temporary directory. You can override the full path to this file by setting the
/// `AHMA_HTTP_CLIENT_TOKEN_PATH` environment variable.
pub struct HttpMcpTransport {
    client: reqwest::Client,
    mcp_url: Url,

    auth_state: Arc<StateMachine<AuthState>>,
    #[allow(dead_code)] // Will be used for token refresh
    oauth_client: Option<ConfiguredOAuthClient>,
    receiver: Arc<Mutex<mpsc::Receiver<RxJsonRpcMessage<RoleClient>>>>,
    sender: mpsc::Sender<RxJsonRpcMessage<RoleClient>>,
}

#[derive(Debug)]
enum AuthState {
    Unauthenticated,
    Authenticating(Arc<Notify>),
    Authenticated(StoredToken),
}

impl HttpMcpTransport {
    /// Creates a new `HttpMcpTransport`.
    ///
    /// The `url` should point to the MCP server's endpoint.
    /// If OAuth2 authentication is required (e.g. for Atlassian), provide `atlassian_client_id`
    /// and `atlassian_client_secret`.
    ///
    /// # Arguments
    ///
    /// * `url` - The URL of the MCP server.
    /// * `atlassian_client_id` - Optional OAuth2 Client ID.
    /// * `atlassian_client_secret` - Optional OAuth2 Client Secret.
    ///
    /// # Example
    ///
    /// ```
    /// # use ahma_http_mcp_client::client::HttpMcpTransport;
    /// # use url::Url;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let url = Url::parse("http://localhost:8000/mcp")?;
    /// let transport = HttpMcpTransport::new(url, None, None)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        url: Url,
        atlassian_client_id: Option<String>,
        atlassian_client_secret: Option<String>,
    ) -> Result<Self> {
        let oauth_client = if let (Some(atlassian_client_id), Some(atlassian_client_secret)) =
            (atlassian_client_id, atlassian_client_secret)
        {
            let mut client = BasicClient::new(ClientId::new(atlassian_client_id))
                .set_client_secret(ClientSecret::new(atlassian_client_secret))
                .set_auth_uri(AuthUrl::new(
                    "https://auth.atlassian.com/authorize".to_string(),
                )?)
                .set_token_uri(TokenUrl::new(
                    "https://auth.atlassian.com/oauth/token".to_string(),
                )?);
            client =
                client.set_redirect_uri(RedirectUrl::new("http://localhost:8080".to_string())?);
            Some(client)
        } else {
            None
        };

        let (sender, receiver) = mpsc::channel(100);

        let initial_token = load_token()?;
        let initial_state = match initial_token {
            Some(token) => AuthState::Authenticated(token),
            None => AuthState::Unauthenticated,
        };

        // Build HTTP client preferring HTTP/3 (QUIC) when the server supports it.
        // reqwest with the `http3` feature automatically upgrades via Alt-Svc headers.
        let http_client = reqwest::Client::builder()
            .build()
            .map_err(|e| McpHttpError::Custom(format!("Failed to build HTTP client: {e}")))?;

        let transport = Self {
            client: http_client,
            mcp_url: url,
            auth_state: Arc::new(StateMachine::new(initial_state)),
            oauth_client,
            receiver: Arc::new(Mutex::new(receiver)),
            sender,
        };

        Ok(transport)
    }

    /// Checks if a valid token exists, and if not, initiates the OAuth2 flow.
    ///
    /// If an OAuth2 client was configured during creation (via `atlassian_client_id` and secret),
    /// this method will:
    /// 1. Check if a token is already loaded in memory.
    /// 2. (Future) Check for token expiration and refresh if needed.
    /// 3. If no token is present, start the interactive OAuth2 flow, prompting the user
    ///    to open a URL in their browser.
    /// 4. Wait for the callback on `localhost:8080`, exchange the code for a token,
    ///    and save it locally.
    ///
    /// If no OAuth2 client is configured, this returns an error if no token is present.
    #[allow(dead_code)] // Will be used when HTTP client is integrated
    pub async fn ensure_authenticated(&self) -> Result<()> {
        loop {
            enum Action {
                PerformAuth(Arc<Notify>),
                Wait(Arc<Notify>),
                Ok,
            }

            let action = self.auth_state.transition(|state| match state {
                AuthState::Authenticated(_) => Action::Ok,
                AuthState::Authenticating(notify) => Action::Wait(notify.clone()),
                AuthState::Unauthenticated => {
                    let notify = Arc::new(Notify::new());
                    *state = AuthState::Authenticating(notify.clone());
                    Action::PerformAuth(notify)
                }
            });

            match action {
                Action::Ok => return Ok(()),
                Action::Wait(notify) => {
                    notify.notified().await;
                    continue;
                }
                Action::PerformAuth(notify) => {
                    if let Some(oauth_client) = &self.oauth_client {
                        info!("No token found, starting authentication flow.");
                        match self.perform_oauth_flow(oauth_client).await {
                            Ok(new_token) => {
                                self.auth_state.transition(|state| {
                                    *state = AuthState::Authenticated(new_token);
                                });
                                info!("Authentication successful.");
                                notify.notify_waiters();
                                return Ok(());
                            }
                            Err(e) => {
                                self.auth_state.transition(|state| {
                                    *state = AuthState::Unauthenticated;
                                });
                                notify.notify_waiters();
                                return Err(e);
                            }
                        }
                    } else {
                        self.auth_state.transition(|state| {
                            *state = AuthState::Unauthenticated;
                        });
                        notify.notify_waiters();
                        return Err(McpHttpError::Auth(
                            "OAuth client not configured, but authentication is required."
                                .to_string(),
                        ));
                    }
                }
            }
        }
    }

    #[allow(dead_code)] // Will be used when HTTP client is integrated
    async fn perform_oauth_flow(
        &self,
        oauth_client: &ConfiguredOAuthClient,
    ) -> Result<StoredToken> {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

        let scopes = vec![
            "read:me",
            "read:confluence-content.summary",
            "read:confluence-space.summary",
            "read:confluence-props",
            "read:confluence-content.all",
            "read:confluence-user",
            "read:jira-user",
            "read:jira-work",
            "write:jira-work",
            "read:confluence-content.permission",
            "offline_access",
        ]
        .into_iter()
        .map(|s| Scope::new(s.to_string()));

        let (auth_url, csrf_token) = oauth_client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge)
            .add_scopes(scopes)
            .url();

        info!("Please open this URL in your browser to authenticate:");
        info!("{}", auth_url);

        if webbrowser::open(auth_url.as_str()).is_err() {
            error!(
                "Failed to open web browser automatically. Please copy the URL and open it manually."
            );
        }

        let (code, state) = self.listen_for_callback_async().await?;

        if state.secret() != csrf_token.secret() {
            return Err(McpHttpError::Auth("CSRF token mismatch".to_string()));
        }

        // Use oauth2's bundled reqwest client which implements AsyncHttpClient
        let http_client = oauth2::reqwest::Client::new();
        let token_result = oauth_client
            .exchange_code(AuthorizationCode::new(code))
            .set_pkce_verifier(pkce_verifier)
            .request_async(&http_client)
            .await
            .map_err(|e| McpHttpError::OAuth2(format!("{:?}", e)))?;

        let stored_token = StoredToken {
            access_token: token_result.access_token().secret().to_string(),
            refresh_token: token_result
                .refresh_token()
                .map(|rt| rt.secret().to_string()),
            expires_in: token_result.expires_in().map(|d| d.as_secs()),
            scopes: token_result
                .scopes()
                .map(|s| s.iter().map(|sc| sc.to_string()).collect()),
        };

        save_token(&stored_token)?;

        Ok(stored_token)
    }

    #[allow(dead_code)] // Will be used when HTTP client is integrated
    async fn listen_for_callback_async(&self) -> Result<(String, CsrfToken)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
        info!("Listening on http://127.0.0.1:8080 for OAuth callback.");

        let (mut stream, _) = listener.accept().await?;

        let (reader, mut writer) = tokio::io::split(&mut stream);
        let mut reader = tokio::io::BufReader::new(reader);

        let mut request_line = String::new();
        tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut request_line).await?;

        let redirect_url = request_line.split_whitespace().nth(1).unwrap_or("/");
        let url = Url::parse(&("http://localhost".to_string() + redirect_url))?;

        let code = url
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, value)| value.into_owned())
            .ok_or_else(|| McpHttpError::Auth("Missing auth code in callback".to_string()))?;

        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| CsrfToken::new(value.into_owned()))
            .ok_or_else(|| McpHttpError::Auth("Missing state in callback".to_string()))?;

        let message = "Authentication successful! You can close this tab.";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            message.len(),
            message
        );
        use tokio::io::AsyncWriteExt;
        writer.write_all(response.as_bytes()).await?;
        writer.flush().await?;

        Ok((code, state))
    }
}

impl Transport<RoleClient> for HttpMcpTransport {
    type Error = McpHttpError;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl std::future::Future<Output = std::result::Result<(), Self::Error>> + Send + 'static
    {
        let client = self.client.clone();
        let auth_state = self.auth_state.clone();
        let mcp_url = self.mcp_url.clone();
        let sender = self.sender.clone();

        async move {
            // Ensure authenticated
            let access_token = auth_state.transition(|state| match state {
                AuthState::Authenticated(token) => Ok(token.access_token.clone()),
                _ => Err(McpHttpError::MissingAccessToken),
            })?;

            let res = client
                .post(mcp_url)
                .bearer_auth(access_token)
                .json(&item)
                .send()
                .await?;

            if !res.status().is_success() {
                let status = res.status();
                let text = res.text().await.unwrap_or_default();
                let err_msg = format!("HTTP Error: {} - {}", status, text);
                error!("{}", err_msg);
                return Err(McpHttpError::Custom(err_msg));
            }

            // Try to parse response as JSON-RPC message and send to channel
            // Note: Notifications might not return a body, or return empty body
            // Try to parse response as JSON-RPC message and send to channel
            // Note: Notifications might not return a body, or return empty body
            // We read the full body bytes to determine if it's empty, rather than relying on
            // Content-Length header which might be missing or unreliable in some environments (e.g. CI).
            let body_bytes = res.bytes().await?;
            if !body_bytes.is_empty() {
                // Try to parse non-empty body
                match serde_json::from_slice::<RxJsonRpcMessage<RoleClient>>(&body_bytes) {
                    Ok(msg) => {
                        if let Err(e) = sender.send(msg).await {
                            error!("Failed to send response to channel: {}", e);
                            return Err(McpHttpError::Custom(format!("Channel error: {}", e)));
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse response body: {}", e);
                        // Start of body for debugging context (first 100 chars)
                        let preview = String::from_utf8_lossy(&body_bytes)
                            .chars()
                            .take(100)
                            .collect::<String>();
                        error!("Response body start: {:?}", preview);

                        return Err(McpHttpError::Json(e));
                    }
                }
            }

            Ok(())
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        let mut receiver = self.receiver.lock().await;
        receiver.recv().await
    }

    async fn close(&mut self) -> std::result::Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredToken {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    scopes: Option<Vec<String>>,
}

fn load_token() -> Result<Option<StoredToken>> {
    let path = token_file_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let token = serde_json::from_reader(reader)?;
    Ok(Some(token))
}

fn save_token(token: &StoredToken) -> Result<()> {
    let path = token_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = fs::File::create(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, token)?;
    writer.flush()?;
    Ok(())
}

fn token_file_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os(TOKEN_PATH_ENV) {
        return Ok(PathBuf::from(path));
    }
    Ok(env::temp_dir().join(TOKEN_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex as StdMutex, OnceLock};
    use tempfile::tempdir;

    fn token_env_guard() -> &'static StdMutex<()> {
        static GUARD: OnceLock<StdMutex<()>> = OnceLock::new();
        GUARD.get_or_init(|| StdMutex::new(()))
    }

    #[test]
    fn load_token_returns_none_when_override_missing() {
        let _guard = token_env_guard().lock().unwrap();
        let tmp = tempdir().unwrap();
        let token_path = tmp.path().join("custom_token.json");
        unsafe {
            env::set_var(TOKEN_PATH_ENV, token_path.to_str().unwrap());
        }

        let loaded = load_token().unwrap();
        assert!(loaded.is_none());

        unsafe {
            env::remove_var(TOKEN_PATH_ENV);
        }
    }

    #[test]
    fn save_token_round_trips_via_override_path() {
        let _guard = token_env_guard().lock().unwrap();
        let tmp = tempdir().unwrap();
        let token_path = tmp.path().join("custom_token.json");
        unsafe {
            env::set_var(TOKEN_PATH_ENV, token_path.to_str().unwrap());
        }

        let token = StoredToken {
            access_token: "abc123".to_string(),
            refresh_token: Some("ref-456".to_string()),
            expires_in: Some(3600),
            scopes: Some(vec!["scope1".to_string(), "scope2".to_string()]),
        };

        save_token(&token).unwrap();
        let loaded = load_token().unwrap().expect("token to exist");
        assert_eq!(loaded.access_token, token.access_token);
        assert_eq!(loaded.refresh_token, token.refresh_token);
        assert_eq!(loaded.expires_in, token.expires_in);
        assert_eq!(loaded.scopes, token.scopes);

        unsafe {
            env::remove_var(TOKEN_PATH_ENV);
        }
    }

    #[test]
    fn token_file_path_uses_env_override() {
        let _guard = token_env_guard().lock().unwrap();
        let custom_path = "/custom/path/token.json";
        unsafe {
            env::set_var(TOKEN_PATH_ENV, custom_path);
        }

        let path = token_file_path().unwrap();
        assert_eq!(path.to_str().unwrap(), custom_path);

        unsafe {
            env::remove_var(TOKEN_PATH_ENV);
        }
    }

    #[test]
    fn token_file_path_uses_temp_dir_default() {
        let _guard = token_env_guard().lock().unwrap();
        unsafe {
            env::remove_var(TOKEN_PATH_ENV);
        }

        let path = token_file_path().unwrap();
        assert!(path.ends_with(TOKEN_FILE_NAME));
        assert!(path.starts_with(env::temp_dir()));
    }

    #[test]
    fn save_token_creates_parent_directories() {
        let _guard = token_env_guard().lock().unwrap();
        let tmp = tempdir().unwrap();
        let token_path = tmp.path().join("nested/deep/token.json");
        unsafe {
            env::set_var(TOKEN_PATH_ENV, token_path.to_str().unwrap());
        }

        let token = StoredToken {
            access_token: "test".to_string(),
            refresh_token: None,
            expires_in: None,
            scopes: None,
        };

        save_token(&token).unwrap();
        assert!(token_path.exists());

        unsafe {
            env::remove_var(TOKEN_PATH_ENV);
        }
    }

    #[test]
    fn stored_token_minimal_fields() {
        let token = StoredToken {
            access_token: "test_token".to_string(),
            refresh_token: None,
            expires_in: None,
            scopes: None,
        };

        let json = serde_json::to_string(&token).unwrap();
        assert!(json.contains("test_token"));

        let parsed: StoredToken = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, "test_token");
        assert!(parsed.refresh_token.is_none());
    }

    #[test]
    fn stored_token_debug_display() {
        let token = StoredToken {
            access_token: "secret".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_in: Some(3600),
            scopes: Some(vec!["scope1".to_string()]),
        };

        let debug = format!("{:?}", token);
        assert!(debug.contains("StoredToken"));
        assert!(debug.contains("secret")); // Note: in real impl might want to redact
    }

    #[test]
    fn stored_token_clone() {
        let token = StoredToken {
            access_token: "abc".to_string(),
            refresh_token: Some("ref".to_string()),
            expires_in: Some(1800),
            scopes: Some(vec!["s1".to_string(), "s2".to_string()]),
        };

        let cloned = token.clone();
        assert_eq!(cloned.access_token, token.access_token);
        assert_eq!(cloned.refresh_token, token.refresh_token);
        assert_eq!(cloned.expires_in, token.expires_in);
        assert_eq!(cloned.scopes, token.scopes);
    }
}
