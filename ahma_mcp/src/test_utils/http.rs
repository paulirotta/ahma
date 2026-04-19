use super::cli;
use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};
use anyhow::Context;
use reqwest::Client;
use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::time::sleep;

/// Encode a filesystem path as a file:// URI.
///
/// Windows: `C:\foo\bar` → `file:///C:/foo/bar`
/// Unix:    `/tmp/bar`   → `file:///tmp/bar`
fn encode_file_uri(path: &Path) -> String {
    let mut path_str = path.to_string_lossy().into_owned();

    // Strip Windows extended-length prefix (\\?\) if present.
    if path_str.starts_with(r"\\?\") {
        path_str = path_str[4..].to_string();
    }

    // Normalise path separators to forward slashes.
    path_str = path_str.replace('\\', "/");

    let mut out = String::with_capacity(path_str.len() + 10);
    out.push_str("file://");

    // On Windows a drive-letter path looks like "C:/Users/…".
    // RFC 8089 §2 requires the path to start with "/" so that it occupies
    // the path component, not the authority.
    #[cfg(target_os = "windows")]
    {
        let is_drive = path_str.len() >= 2
            && path_str.as_bytes()[0].is_ascii_alphabetic()
            && path_str.as_bytes()[1] == b':';
        if is_drive {
            out.push('/');
        }
    }

    for b in path_str.as_bytes() {
        let b = *b;
        let keep = matches!(
            b,
            b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'-'
                | b'.'
                | b'_'
                | b'~'
                | b'/'
                | b':'
        );
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

/// A running HTTP bridge instance for integration testing.
pub struct HttpBridgeTestInstance {
    pub child: Child,
    pub port: u16,
    pub temp_dir: TempDir,
}

impl HttpBridgeTestInstance {
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for HttpBridgeTestInstance {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Spawn a robust HTTP bridge for testing.
pub async fn spawn_http_bridge() -> anyhow::Result<HttpBridgeTestInstance> {
    use std::net::TcpListener;
    use std::process::{Command, Stdio};

    // Find available port
    let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();

    let binary = cli::build_binary_cached("ahma_mcp", "ahma-mcp");

    let temp_dir = TempDir::new()?;
    let tools_dir = temp_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir)?;

    let mut cmd = Command::new(&binary);
    cmd.args(["serve", "http", "--port", &port.to_string()])
        .env("AHMA_TOOLS_DIR", &*tools_dir.to_string_lossy())
        .env("AHMA_SANDBOX_SCOPE", &*temp_dir.path().to_string_lossy())
        .env("AHMA_LOG_TARGET", "stderr")
        // Give the bridge server a generous handshake window so slow CI runners
        // (especially Linux with Landlock sandbox setup) have enough time to complete
        // the roots/list exchange before the server declares a timeout (-32002).
        .env(
            "AHMA_HANDSHAKE_TIMEOUT",
            TestTimeouts::scale_secs(120).as_secs().to_string(),
        )
        .env_remove("NEXTEST")
        .env_remove("NEXTEST_EXECUTION_MODE")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUST_TEST_THREADS")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    // Detect nested sandbox (mcp_ahma_sandboxed_shell / VS Code / Docker) and use
    // AHMA_DISABLE_SANDBOX so the child can start; app-level path security still applies.
    #[cfg(target_os = "macos")]
    if crate::sandbox::test_sandbox_exec_available().is_err() {
        cmd.env("AHMA_DISABLE_SANDBOX", "1");
    }
    #[cfg(target_os = "windows")]
    if crate::sandbox::check_windows_sandbox_available().is_err() {
        cmd.env("AHMA_DISABLE_SANDBOX", "1");
    }

    let mut child = cmd.spawn()?;

    // Wait for server health
    let client = Client::builder()
        .http2_prior_knowledge()
        .build()
        .context("Failed to build HTTP/2 health-check client")?;
    let health_url = format!("http://127.0.0.1:{}/health", port);

    let start = Instant::now();
    let timeout = Duration::from_secs(10);

    while start.elapsed() < timeout {
        if client
            .get(&health_url)
            .send()
            .await
            .is_ok_and(|resp| resp.status().is_success())
        {
            return Ok(HttpBridgeTestInstance {
                child,
                port,
                temp_dir,
            });
        }
        sleep(Duration::from_millis(100)).await;
    }

    let _ = child.kill();
    let _ = child.wait();
    anyhow::bail!("Timed out waiting for HTTP bridge health");
}

/// A client for testing the MCP protocol over HTTP and SSE.
pub struct HttpMcpTestClient {
    pub client: Client,
    pub base_url: String,
    pub session_id: Option<String>,
}

impl HttpMcpTestClient {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::builder()
                .http2_prior_knowledge()
                .build()
                .expect("Failed to build HTTP/2 test client"),
            base_url,
            session_id: None,
        }
    }

    pub async fn send_request(
        &self,
        request: &serde_json::Value,
    ) -> anyhow::Result<(serde_json::Value, Option<String>)> {
        let url = format!("{}/mcp", self.base_url);
        let mut req = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");

        if let Some(ref sid) = self.session_id {
            req = req.header("Mcp-Session-Id", sid);
        }

        let resp = req.json(request).send().await.context("POST /mcp failed")?;

        let session_id = resp
            .headers()
            .get("mcp-session-id")
            .or_else(|| resp.headers().get("Mcp-Session-Id"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let body: serde_json::Value = resp.json().await.context("Failed to parse JSON response")?;

        Ok((body, session_id))
    }

    /// Send only the MCP initialize request and capture the session ID.
    pub async fn initialize_only(&mut self) -> anyhow::Result<()> {
        let init_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "1.0.0"}
            }
        });

        let (resp, sid) = self.send_request(&init_request).await?;
        if let Some(err) = resp.get("error") {
            anyhow::bail!("Initialize failed: {:?}", err);
        }
        self.session_id = sid;

        Ok(())
    }

    /// Send notifications/initialized for the current session.
    pub async fn send_initialized(&self) -> anyhow::Result<()> {
        self.session_id.as_ref().context("Not initialized")?;

        let initialized = serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        self.send_request(&initialized).await?;

        Ok(())
    }

    /// Complete the HTTP roots handshake in the safe protocol order and return an SSE receiver.
    ///
    /// This is the preferred API for tests that need an event stream plus automatic roots/list
    /// handling. It makes the correct ordering explicit and difficult to misuse:
    /// initialize -> open SSE -> short delay -> initialized -> answer roots/list.
    ///
    /// Returns only after the roots/list exchange is complete (the server has received the
    /// client's roots and is in the process of locking the sandbox).
    pub async fn initialize_with_roots_events(
        &mut self,
        roots: Vec<std::path::PathBuf>,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<serde_json::Value>> {
        self.initialize_only().await?;
        let (rx, roots_ready_rx) = self.start_sse_events(roots).await?;
        sleep(TestTimeouts::short_delay()).await;
        self.send_initialized().await?;
        // Wait until the SSE background task has sent the roots/list response.
        // This ensures that when the caller starts polling tools/call the sandbox
        // handshake is already in flight, avoiding a race where tools/call polls
        // that trigger the server's 45-second handshake timeout before the roots
        // exchange finishes (particularly on slow Linux CI with Landlock setup).
        tokio::time::timeout(
            TestTimeouts::get(TimeoutCategory::SseStream),
            roots_ready_rx,
        )
        .await
        .context("Timed out waiting for roots/list exchange to complete")?
        .context("roots_ready channel closed before roots/list exchange completed")?;
        Ok(rx)
    }

    /// Complete a minimal MCP handshake when explicit client roots are not required.
    pub async fn initialize(&mut self) -> anyhow::Result<()> {
        self.initialize_only().await?;
        self.send_initialized().await
    }

    /// Open an SSE stream for an existing session and auto-answer roots/list requests.
    ///
    /// Returns an event receiver and a one-shot receiver that fires once the roots/list
    /// POST response has been sent (i.e. the sandbox handshake is in flight).
    ///
    /// Callers should prefer `initialize_with_roots_events()` unless they intentionally need
    /// low-level control over handshake sequencing.
    async fn start_sse_events(
        &self,
        roots: Vec<std::path::PathBuf>,
    ) -> anyhow::Result<(
        tokio::sync::mpsc::Receiver<serde_json::Value>,
        tokio::sync::oneshot::Receiver<()>,
    )> {
        use futures::StreamExt;

        let sid = self.session_id.as_ref().context("Not initialized")?.clone();
        let url = format!("{}/mcp", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("Accept", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Mcp-Session-Id", &sid)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("SSE failed: {}", resp.status());
        }

        let mut stream = resp.bytes_stream();
        let (tx, rx) = tokio::sync::mpsc::channel::<serde_json::Value>(256);
        let (roots_ready_tx, roots_ready_rx) = tokio::sync::oneshot::channel::<()>();
        let client = self.client.clone();
        let base_url = self.base_url.clone();

        tokio::spawn(async move {
            let mut roots_ready_tx = Some(roots_ready_tx);
            let mut buffer = String::new();
            loop {
                let chunk = match stream.next().await {
                    Some(Ok(c)) => c,
                    _ => break,
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(idx) = buffer.find("\n\n") {
                    let raw_event = buffer[..idx].to_string();
                    buffer = buffer[idx + 2..].to_string();

                    let mut data_lines = Vec::new();
                    for line in raw_event.lines() {
                        let line = line.trim_end_matches('\r');
                        if let Some(rest) = line.strip_prefix("data:") {
                            data_lines.push(rest.trim());
                        }
                    }
                    if data_lines.is_empty() {
                        continue;
                    }

                    let data = data_lines.join("\n");
                    let value: serde_json::Value = match serde_json::from_str(&data) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    if value.get("method").and_then(|m| m.as_str()) == Some("roots/list") {
                        let id = value.get("id").cloned().expect("roots/list must have id");
                        let roots_json: Vec<serde_json::Value> = roots
                            .iter()
                            .map(|p| {
                                serde_json::json!({
                                    "uri": encode_file_uri(p),
                                    "name": p.file_name().and_then(|n| n.to_str()).unwrap_or("root")
                                })
                            })
                            .collect();
                        let response = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "roots": roots_json }
                        });

                        let _ = client
                            .post(format!("{}/mcp", base_url))
                            .header("Mcp-Session-Id", &sid)
                            .json(&response)
                            .send()
                            .await;

                        // Signal that the roots/list response has been sent.  The server
                        // will now proceed to lock the sandbox.
                        if let Some(tx) = roots_ready_tx.take() {
                            let _ = tx.send(());
                        }
                    }
                    let _ = tx.send(value).await;
                }
            }
        });

        Ok((rx, roots_ready_rx))
    }
}
