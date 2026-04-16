use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};
use futures::StreamExt;
use reqwest::Client;
use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Transport mode for MCP POST requests: content negotiated via the `Accept` header.
///
/// - `Json` (default): `Accept: application/json` — the server returns a single JSON-RPC response.
/// - `Sse`: `Accept: text/event-stream` — the server streams an SSE response containing
///   zero or more notification events followed by the JSON-RPC response event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportMode {
    #[default]
    Json,
    Sse,
}

use super::protocol::{JsonRpcRequest, JsonRpcResponse};
use super::uri::encode_file_uri;

/// Result of a tool call.
#[derive(Debug)]
pub struct ToolCallResult {
    pub tool_name: String,
    pub success: bool,
    pub duration_ms: u128,
    pub error: Option<String>,
    pub output: Option<String>,
}

/// MCP test client that handles protocol/session details for integration tests.
pub struct McpTestClient {
    client: Client,
    base_url: String,
    session_id: Option<String>,
    transport_mode: TransportMode,
}

impl McpTestClient {
    /// Create a new MCP test client for a specific server URL.
    pub fn with_url(base_url: &str) -> Self {
        Self {
            client: Client::builder()
                .http2_prior_knowledge()
                .build()
                .expect("Failed to build HTTP/2 test client"),
            base_url: base_url.to_string(),
            session_id: None,
            transport_mode: TransportMode::Json,
        }
    }

    /// Create a new MCP test client from a running test server.
    pub fn for_server(server: &super::server::TestServerInstance) -> Self {
        Self::with_url(&server.base_url())
    }

    /// Set the transport mode for all subsequent `send_request` / `call_tool` calls.
    pub fn with_transport(mut self, mode: TransportMode) -> Self {
        self.transport_mode = mode;
        self
    }

    fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url)
    }

    fn extract_session_id(headers: &HeaderMap) -> Option<String> {
        headers
            .get("mcp-session-id")
            .or_else(|| headers.get("Mcp-Session-Id"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }

    fn required_session_id(&self) -> Result<&str, String> {
        self.session_id
            .as_deref()
            .ok_or_else(|| "No session ID received".to_string())
    }

    async fn send_initialize(&mut self, client_name: &str) -> Result<JsonRpcResponse, String> {
        let init_request = JsonRpcRequest::initialize(client_name);
        let response = self
            .client
            .post(self.mcp_url())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&init_request)
            .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
            .send()
            .await
            .map_err(|e| format!("Initialize request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("Initialize failed with HTTP {}: {}", status, text));
        }

        self.session_id = Self::extract_session_id(response.headers());

        let init_response: JsonRpcResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse initialize response: {}", e))?;

        if init_response.error.is_some() {
            return Err(format!(
                "Initialize returned error: {:?}",
                init_response.error
            ));
        }

        Ok(init_response)
    }

    async fn send_initialized_notification(&self, session_id: &str) -> Result<(), String> {
        let initialized_notification = JsonRpcRequest::initialized();
        let response = self
            .client
            .post(self.mcp_url())
            .header("Content-Type", "application/json")
            .header("Mcp-Session-Id", session_id)
            .json(&initialized_notification)
            .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
            .send()
            .await
            .map_err(|e| format!("initialized notification failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!(
                "initialized notification failed with HTTP {}: {}",
                status, text
            ));
        }

        Ok(())
    }

    fn roots_handshake_timeout() -> Duration {
        TestTimeouts::get(TimeoutCategory::Handshake)
    }

    fn pop_next_sse_event(buffer: &mut String) -> Option<String> {
        let idx = buffer.find("\n\n")?;
        let raw_event = buffer[..idx].to_string();
        *buffer = buffer[idx + 2..].to_string();
        Some(raw_event)
    }

    fn event_data_to_json(raw_event: &str) -> Option<Value> {
        let data: Vec<&str> = raw_event
            .lines()
            .filter_map(|line| line.trim_end_matches('\r').strip_prefix("data:"))
            .map(str::trim)
            .collect();

        if data.is_empty() {
            return None;
        }
        serde_json::from_str::<Value>(&data.join("\n")).ok()
    }

    async fn open_handshake_sse(&self, session_id: &str) -> Result<reqwest::Response, String> {
        let sse_resp = self
            .client
            .get(self.mcp_url())
            .header("Accept", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Mcp-Session-Id", session_id)
            .send()
            .await
            .map_err(|e| format!("SSE connection failed: {}", e))?;

        if !sse_resp.status().is_success() {
            return Err(format!("SSE stream failed with HTTP {}", sse_resp.status()));
        }

        Ok(sse_resp)
    }

    /// Dispatch a single SSE event. Returns `Ok(true)` when the handshake is complete.
    async fn handle_sse_event(
        &self,
        value: &Value,
        session_id: &str,
        roots: &[PathBuf],
        roots_answered: &mut bool,
    ) -> Result<bool, String> {
        let method = value.get("method").and_then(|m| m.as_str());

        if method == Some("notifications/sandbox/failed") {
            let error = value
                .get("params")
                .and_then(|p| p.get("error"))
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            return Err(format!("Sandbox configuration failed: {}", error));
        }

        if method == Some("notifications/sandbox/configured") && *roots_answered {
            return Ok(true);
        }

        if method == Some("roots/list") {
            let request_id = value
                .get("id")
                .cloned()
                .ok_or_else(|| "roots/list must include id".to_string())?;
            self.send_roots_response(session_id, request_id, roots)
                .await?;
            *roots_answered = true;
        }

        Ok(false)
    }

    async fn process_roots_handshake_stream(
        &self,
        sse_resp: reqwest::Response,
        session_id: &str,
        roots: &[PathBuf],
    ) -> Result<(), String> {
        let mut stream = sse_resp.bytes_stream();
        let mut buffer = String::new();
        let mut roots_answered = false;
        let deadline = Instant::now() + Self::roots_handshake_timeout();

        loop {
            if Instant::now() > deadline {
                return Err(
                    "Timeout waiting for roots/list + sandbox/configured over SSE".to_string(),
                );
            }

            let Some(chunk) = tokio::time::timeout(TestTimeouts::poll_interval(), stream.next())
                .await
                .ok()
                .flatten()
            else {
                continue;
            };

            let bytes = chunk.map_err(|e| format!("SSE read error: {}", e))?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(raw_event) = Self::pop_next_sse_event(&mut buffer) {
                let Some(value) = Self::event_data_to_json(&raw_event) else {
                    continue;
                };
                if self
                    .handle_sse_event(&value, session_id, roots, &mut roots_answered)
                    .await?
                {
                    return Ok(());
                }
            }
        }
    }

    async fn send_roots_response(
        &self,
        session_id: &str,
        request_id: Value,
        roots: &[PathBuf],
    ) -> Result<(), String> {
        let roots_json: Vec<Value> = roots
            .iter()
            .map(|path| {
                json!({
                    "uri": encode_file_uri(path),
                    "name": path.file_name().and_then(|n| n.to_str()).unwrap_or("root")
                })
            })
            .collect();

        let roots_response = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "roots": roots_json
            }
        });

        let _ = self
            .client
            .post(self.mcp_url())
            .header("Content-Type", "application/json")
            .header("Mcp-Session-Id", session_id)
            .json(&roots_response)
            .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
            .send()
            .await
            .map_err(|e| format!("Failed to send roots response: {}", e))?;

        Ok(())
    }

    /// Complete the MCP handshake: initialize + initialized notification.
    pub async fn initialize(&mut self) -> Result<JsonRpcResponse, String> {
        self.initialize_with_name("mcp-test-client").await
    }

    /// Send only initialize and capture the session ID.
    pub async fn initialize_only(&mut self, client_name: &str) -> Result<JsonRpcResponse, String> {
        self.send_initialize(client_name).await
    }

    /// Send notifications/initialized for the current session.
    pub async fn send_initialized(&self) -> Result<(), String> {
        let session_id = self.required_session_id()?;
        self.send_initialized_notification(session_id).await
    }

    /// Complete roots handshake (SSE + roots/list response + sandbox/configured)
    /// after the client has already sent notifications/initialized.
    pub async fn complete_roots_handshake_after_initialized(
        &self,
        roots: &[PathBuf],
    ) -> Result<(), String> {
        let session_id = self.required_session_id()?;
        let sse_resp = self.open_handshake_sse(session_id).await?;
        self.process_roots_handshake_stream(sse_resp, session_id, roots)
            .await
    }

    /// Complete the roots handshake in the correct protocol order:
    /// open the SSE stream *first*, then send notifications/initialized.
    ///
    /// Use this after `initialize_only` when you need the SSE listener
    /// established before the server can fire `roots/list`.
    pub async fn complete_handshake_with_roots(&self, roots: &[PathBuf]) -> Result<(), String> {
        let session_id = self.required_session_id()?;
        let sse_resp = self.open_handshake_sse(session_id).await?;
        // Avoid a race where initialized is processed before SSE subscription
        // registration is fully active in the bridge.
        sleep(TestTimeouts::short_delay()).await;
        self.send_initialized_notification(session_id).await?;
        self.process_roots_handshake_stream(sse_resp, session_id, roots)
            .await
    }

    /// Complete the MCP handshake with a custom client name.
    pub async fn initialize_with_name(
        &mut self,
        client_name: &str,
    ) -> Result<JsonRpcResponse, String> {
        let init_response = self.initialize_only(client_name).await?;
        self.send_initialized().await?;
        Ok(init_response)
    }

    /// Complete the MCP handshake with roots to lock sandbox scope.
    pub async fn initialize_with_roots(
        &mut self,
        client_name: &str,
        roots: &[PathBuf],
    ) -> Result<JsonRpcResponse, String> {
        let init_response = self.initialize_only(client_name).await?;
        let session_id = self.required_session_id()?;

        let sse_resp = self.open_handshake_sse(session_id).await?;
        // Avoid a race where initialized is processed before SSE subscription
        // registration is fully active in the bridge.
        sleep(TestTimeouts::short_delay()).await;
        self.send_initialized().await?;
        self.process_roots_handshake_stream(sse_resp, session_id, roots)
            .await?;

        Ok(init_response)
    }

    /// Send a raw JSON-RPC request with session handling.
    ///
    /// Dispatches to the JSON or SSE path based on `self.transport_mode`.
    pub async fn send_request(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse, String> {
        match self.transport_mode {
            TransportMode::Json => self.send_request_json(request).await,
            TransportMode::Sse => self.send_request_sse(request).await,
        }
    }

    /// JSON transport path: `Accept: application/json`, returns a single JSON-RPC response.
    async fn send_request_json(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse, String> {
        let mut req_builder = self
            .client
            .post(self.mcp_url())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");

        if let Some(ref session_id) = self.session_id {
            req_builder = req_builder.header("Mcp-Session-Id", session_id);
        }

        let response = req_builder
            .json(request)
            .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("HTTP {}: {}", status, text));
        }

        response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))
    }

    /// SSE transport path: `Accept: text/event-stream`.
    ///
    /// Reads the resulting SSE stream and returns the first event that looks
    /// like a JSON-RPC response (has `result` or `error`), skipping any
    /// interleaved notification events.
    async fn send_request_sse(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse, String> {
        let mut req_builder = self
            .client
            .post(self.mcp_url())
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");

        if let Some(ref session_id) = self.session_id {
            req_builder = req_builder.header("Mcp-Session-Id", session_id);
        }

        // No reqwest-level timeout: the SSE stream may take a while to deliver
        // the response event.  The deadline loop below bounds overall wait time.
        let response = req_builder
            .json(request)
            .send()
            .await
            .map_err(|e| format!("SSE request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("HTTP {}: {}", status, text));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let deadline = Instant::now() + TestTimeouts::get(TimeoutCategory::ToolCall);

        loop {
            if Instant::now() > deadline {
                return Err("Timeout waiting for SSE response event".to_string());
            }

            let Some(chunk) = tokio::time::timeout(TestTimeouts::poll_interval(), stream.next())
                .await
                .ok()
                .flatten()
            else {
                continue;
            };

            let bytes = chunk.map_err(|e| format!("SSE stream read error: {}", e))?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(raw_event) = Self::pop_next_sse_event(&mut buffer) {
                let Some(value) = Self::event_data_to_json(&raw_event) else {
                    continue;
                };
                // Notifications carry a `method` field; responses carry `result` or `error`.
                if value.get("method").is_some() {
                    continue;
                }
                if value.get("result").is_some() || value.get("error").is_some() {
                    return serde_json::from_value(value)
                        .map_err(|e| format!("Failed to deserialize SSE response: {}", e));
                }
            }
        }
    }

    /// Call a tool and return the result.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> ToolCallResult {
        let start = Instant::now();
        let request = JsonRpcRequest::call_tool(name, arguments);

        match self.send_request(&request).await {
            Ok(response) => {
                let duration_ms = start.elapsed().as_millis();
                if let Some(ref error) = response.error {
                    ToolCallResult {
                        tool_name: name.to_string(),
                        success: false,
                        duration_ms,
                        error: Some(format!("[{}] {}", error.code, error.message)),
                        output: None,
                    }
                } else {
                    ToolCallResult {
                        tool_name: name.to_string(),
                        success: true,
                        duration_ms,
                        error: None,
                        output: response.extract_tool_output(),
                    }
                }
            }
            Err(e) => ToolCallResult {
                tool_name: name.to_string(),
                success: false,
                duration_ms: start.elapsed().as_millis(),
                error: Some(e),
                output: None,
            },
        }
    }

    /// List available tools.
    pub async fn list_tools(&self) -> Result<Vec<Value>, String> {
        let request = JsonRpcRequest::list_tools();
        let response = self.send_request(&request).await?;

        response
            .result
            .and_then(|r| r.get("tools").cloned())
            .and_then(|t| t.as_array().cloned())
            .ok_or_else(|| "No tools array in response".to_string())
    }

    /// Check if a specific tool is available.
    pub async fn is_tool_available(&self, tool_name: &str) -> bool {
        match self.list_tools().await {
            Ok(tools) => tools.iter().any(|t| {
                t.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n == tool_name)
                    .unwrap_or(false)
            }),
            Err(_) => false,
        }
    }

    /// Get the current session ID (if initialized).
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Check if the client has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.session_id.is_some()
    }
}
