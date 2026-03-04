//! HTTP Bridge Integration Tests
//!
//! These tests verify end-to-end HTTP bridge functionality by:
//! 1. Starting the HTTP bridge with a real ahma_mcp subprocess
//! 2. Sending requests through the HTTP interface
//! 3. Verifying correct responses
//!
//! These tests reproduce the bug where calling a tool from a different project
//! (different working_directory) fails with "expect initialized request" error.
//!
//! NOTE: These tests spawn their own servers with specific sandbox configurations.
//! They use dynamic port allocation to avoid conflicts with other tests.
//! The shared test server singleton (port 5721) is NOT used here.

use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use serial_test::serial;
use std::os::unix::fs as unix_fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::time::sleep;

struct ServerGuard {
    child: Option<Child>,
}

impl ServerGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Build the ahma_mcp binary if needed and return the path
fn get_ahma_mcp_binary() -> PathBuf {
    let workspace_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get workspace dir")
        .to_path_buf();

    // Check for CARGO_TARGET_DIR
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_dir.join("target"));

    let binary_path = target_dir.join("debug/ahma_mcp");

    // Optimization: Skip manual build if binary already exists, especially in NEXTEST
    // where all binaries are pre-built to avoid cargo lock contention.
    if !binary_path.exists() {
        // Build ahma_mcp binary
        let output = Command::new("cargo")
            .current_dir(&workspace_dir)
            .args(["build", "--package", "ahma_mcp", "--bin", "ahma_mcp"])
            .output()
            .expect("Failed to run cargo build");

        assert!(
            output.status.success(),
            "Failed to build ahma_mcp: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    binary_path
}

/// Start the HTTP bridge server and return the process and URL
async fn start_http_bridge(
    tools_dir: &std::path::Path,
    sandbox_scope: &std::path::Path,
) -> (std::process::Child, u16) {
    let binary = get_ahma_mcp_binary();

    let mut cmd = Command::new(&binary);
    cmd.args([
        "--mode",
        "http",
        "--http-port",
        "0", // Use dynamic port
        "--sync",
        "--tools-dir",
        &tools_dir.to_string_lossy(),
        "--sandbox-scope",
        &sandbox_scope.to_string_lossy(),
        "--log-to-stderr",
    ]);

    // IMPORTANT:
    // These integration tests are explicitly verifying real sandbox-scope behavior.
    // ahma_mcp auto-enables a permissive "test mode" (sandbox bypass + best-effort scope "/")
    // when certain env vars are present (e.g. NEXTEST, CARGO_TARGET_DIR, RUST_TEST_THREADS).
    // That makes tests pass even when real-life behavior fails.
    //
    // So we *clear* those env vars for the spawned server process to ensure it behaves
    // like a real user-launched server.
    cmd.env_remove("NEXTEST")
        .env_remove("NEXTEST_EXECUTION_MODE")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUST_TEST_THREADS");

    // Detect nested sandbox (mcp_ahma_sandboxed_shell / VS Code / Docker) at runtime
    // and set AHMA_NO_SANDBOX so the child starts; app-level path checks still apply.
    #[cfg(target_os = "macos")]
    if ahma_mcp::sandbox::test_sandbox_exec_available().is_err() {
        cmd.env("AHMA_NO_SANDBOX", "1");
    }
    #[cfg(windows)]
    cmd.env("AHMA_NO_SANDBOX", "1");

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start HTTP bridge");

    // Capture stderr to find the bound port
    let stderr = child.stderr.take().expect("Failed to capture stderr");
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            eprintln!("{}", line); // Pass through to test output
            if line.contains("AHMA_BOUND_PORT=") {
                let _ = tx.send(line);
            }
        }
    });

    // Wait for port
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(10);
    let mut port = 0;

    while start.elapsed() < timeout {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(100))
            && let Some(idx) = line.find("AHMA_BOUND_PORT=")
        {
            let port_str = &line[idx + "AHMA_BOUND_PORT=".len()..];
            if let Ok(p) = port_str.trim().parse::<u16>() {
                port = p;
                break;
            }
        }

        // check if child died
        if let Ok(Some(status)) = child.try_wait() {
            panic!("Child process exited unexpectedly with status: {}", status);
        }
    }

    if port == 0 {
        let _ = child.kill();
        panic!("Timed out waiting for server to bind port");
    }

    // Wait for server to be ready
    let client = Client::new();
    let health_url = format!("http://127.0.0.1:{}/health", port);

    for _ in 0..150 {
        sleep(Duration::from_millis(200)).await;
        if let Ok(resp) = client.get(&health_url).send().await
            && resp.status().is_success()
        {
            return (child, port);
        }
    }

    // Kill and wait for the child to prevent zombie process
    let mut child = child;
    let _ = child.kill();
    let _ = child.wait();

    panic!("HTTP bridge failed to start within timeout");
}

/// Send a JSON-RPC request to the MCP endpoint
async fn send_mcp_request(
    client: &Client,
    base_url: &str,
    request: &Value,
    session_id: Option<&str>,
) -> Result<(Value, Option<String>), String> {
    let url = format!("{}/mcp", base_url);

    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(120));

    if let Some(id) = session_id {
        req = req.header("Mcp-Session-Id", id);
    }

    let response = req
        .json(request)
        .send()
        .await
        .map_err(|e| format!("Request failed: {:?}", e))?;

    // Debug: print all headers
    eprintln!(
        "Response headers for request {}:",
        request
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
    );
    for (name, value) in response.headers().iter() {
        if name.as_str().eq_ignore_ascii_case("mcp-session-id") {
            eprintln!("  {}: <redacted>", name);
        } else {
            eprintln!("  {}: {:?}", name, value);
        }
    }

    // Get session ID from response header (case-insensitive)
    let new_session_id = response
        .headers()
        .get("mcp-session-id")
        .or_else(|| response.headers().get("Mcp-Session-Id"))
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", status, text));
    }

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok((body, new_session_id))
}

fn is_sandbox_initializing_error(response: &Value) -> bool {
    let error = response.get("error");
    let code = error
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_i64())
        .unwrap_or_default();
    let message = error
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("");

    code == -32001 || message.contains("Sandbox initializing")
}

fn is_transient_transport_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("http 409")
        || lower.contains("http 500")
        || lower.contains("http 502")
        || lower.contains("http 503")
        || lower.contains("http 504")
}

fn capped_backoff(base_ms: u64, attempt: usize, max_ms: u64) -> Duration {
    Duration::from_millis((base_ms.saturating_mul(attempt as u64)).min(max_ms))
}

fn coverage_mode() -> bool {
    std::env::var_os("LLVM_PROFILE_FILE").is_some() || std::env::var_os("CARGO_LLVM_COV").is_some()
}

fn roots_handshake_timeout() -> Duration {
    if coverage_mode() {
        Duration::from_secs(30)
    } else {
        Duration::from_secs(15)
    }
}

fn post_roots_configured_grace_timeout() -> Duration {
    if coverage_mode() {
        Duration::from_secs(8)
    } else {
        Duration::from_secs(3)
    }
}

/// Send tools/call with retries for handshake races and transient transport failures.
async fn send_tool_call_with_retry(
    client: &Client,
    base_url: &str,
    session_id: &str,
    tool_call: &Value,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut attempt = 0usize;

    loop {
        attempt += 1;
        match send_mcp_request(client, base_url, tool_call, Some(session_id)).await {
            Ok((response, _)) => {
                if is_sandbox_initializing_error(&response) {
                    if Instant::now() >= deadline {
                        panic!(
                            "Timed out waiting for sandbox initialization after {} attempts. Last response: {:?}",
                            attempt, response
                        );
                    }
                    sleep(capped_backoff(100, attempt, 1_000)).await;
                    continue;
                }
                return response;
            }
            Err(e) if is_transient_transport_error(&e) => {
                if Instant::now() >= deadline {
                    panic!(
                        "Timed out retrying tools/call after {} attempts. Last error: {}",
                        attempt, e
                    );
                }
                sleep(capped_backoff(200, attempt, 2_000)).await;
            }
            Err(e) => {
                panic!(
                    "Unexpected transport error during tools/call (attempt {}): {}",
                    attempt, e
                );
            }
        }
    }
}

/// Wait for a `roots/list` request over SSE and respond with the provided roots.
async fn answer_roots_list_over_sse(
    client: &Client,
    base_url: &str,
    session_id: &str,
    roots: &[PathBuf],
) {
    let url = format!("{}/mcp", base_url);
    let resp = client
        .get(&url)
        .header("Accept", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Mcp-Session-Id", session_id)
        .send()
        .await
        .expect("Failed to open SSE stream");

    assert!(
        resp.status().is_success(),
        "SSE stream must be available, got HTTP {}",
        resp.status()
    );

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut roots_answered = false;
    let mut post_roots_deadline: Option<tokio::time::Instant> = None;

    // Hard timeout: if session isolation is broken, we may never see roots/list/configured.
    let roots_deadline = tokio::time::Instant::now() + roots_handshake_timeout();
    loop {
        if let Some(timeout_at) = post_roots_deadline
            && tokio::time::Instant::now() > timeout_at
        {
            panic!(
                "Timed out waiting for notifications/sandbox/configured after roots/list response"
            );
        }

        if tokio::time::Instant::now() > roots_deadline {
            panic!(
                "Timed out waiting for roots/list + sandbox/configured over SSE (session isolation likely broken)"
            );
        }

        let chunk = tokio::time::timeout(Duration::from_millis(500), stream.next())
            .await
            .ok()
            .flatten();

        if let Some(next) = chunk {
            let bytes = next.expect("SSE stream read failed");
            let text = String::from_utf8_lossy(&bytes);
            buffer.push_str(&text);

            // SSE events are separated by a blank line.
            while let Some(idx) = buffer.find("\n\n") {
                let raw_event = buffer[..idx].to_string();
                buffer = buffer[idx + 2..].to_string();

                let mut data_lines: Vec<&str> = Vec::new();
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
                let Ok(value) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };

                let method = value.get("method").and_then(|m| m.as_str());

                if method == Some("notifications/sandbox/failed") {
                    let error = value
                        .get("params")
                        .and_then(|p| p.get("error"))
                        .and_then(|e| e.as_str())
                        .unwrap_or("unknown");
                    panic!("Sandbox configuration failed: {}", error);
                }

                if method == Some("notifications/sandbox/configured") {
                    if roots_answered {
                        return;
                    }
                    continue;
                }

                if method != Some("roots/list") {
                    continue;
                }

                let id = value
                    .get("id")
                    .cloned()
                    .expect("roots/list must include id");

                let roots_json: Vec<Value> = roots
                    .iter()
                    .map(|p| {
                        json!({
                            "uri": format!("file://{}", p.display()),
                            "name": p.file_name().and_then(|n| n.to_str()).unwrap_or("root")
                        })
                    })
                    .collect();

                let response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "roots": roots_json
                    }
                });

                let _ = send_mcp_request(client, base_url, &response, Some(session_id))
                    .await
                    .expect("Failed to send roots/list response");
                roots_answered = true;
                post_roots_deadline =
                    Some(tokio::time::Instant::now() + post_roots_configured_grace_timeout());
            }
        }
    }
}

fn percent_encode_path_for_file_uri(path: &std::path::Path) -> String {
    // Percent-encode path characters commonly present in IDE roots (spaces/unicode).
    // We keep '/' and unreserved characters. This is sufficient for file:// URIs.
    let s = path.to_string_lossy();
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
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

/// Wait for a `roots/list` request over SSE and respond with provided URI strings.
async fn answer_roots_list_over_sse_with_uris(
    client: &Client,
    base_url: &str,
    session_id: &str,
    root_uris: &[String],
) {
    let url = format!("{}/mcp", base_url);
    let resp = client
        .get(&url)
        .header("Accept", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Mcp-Session-Id", session_id)
        .send()
        .await
        .expect("Failed to open SSE stream");

    assert!(
        resp.status().is_success(),
        "SSE stream must be available, got HTTP {}",
        resp.status()
    );

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut roots_answered = false;
    let mut post_roots_deadline: Option<tokio::time::Instant> = None;

    let roots_deadline = tokio::time::Instant::now() + roots_handshake_timeout();
    loop {
        if let Some(timeout_at) = post_roots_deadline
            && tokio::time::Instant::now() > timeout_at
        {
            panic!(
                "Timed out waiting for notifications/sandbox/configured after roots/list response"
            );
        }

        if tokio::time::Instant::now() > roots_deadline {
            panic!(
                "Timed out waiting for roots/list + sandbox/configured over SSE (session isolation likely broken)"
            );
        }

        let chunk = tokio::time::timeout(Duration::from_millis(500), stream.next())
            .await
            .ok()
            .flatten();

        if let Some(next) = chunk {
            let bytes = next.expect("SSE stream read failed");
            let text = String::from_utf8_lossy(&bytes);
            buffer.push_str(&text);

            while let Some(idx) = buffer.find("\n\n") {
                let raw_event = buffer[..idx].to_string();
                buffer = buffer[idx + 2..].to_string();

                let mut data_lines: Vec<&str> = Vec::new();
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
                let Ok(value) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };

                let method = value.get("method").and_then(|m| m.as_str());

                if method == Some("notifications/sandbox/failed") {
                    let error = value
                        .get("params")
                        .and_then(|p| p.get("error"))
                        .and_then(|e| e.as_str())
                        .unwrap_or("unknown");
                    panic!("Sandbox configuration failed: {}", error);
                }

                if method == Some("notifications/sandbox/configured") {
                    if roots_answered {
                        return;
                    }
                    continue;
                }

                if method != Some("roots/list") {
                    continue;
                }

                let id = value
                    .get("id")
                    .cloned()
                    .expect("roots/list must include id");

                let roots_json: Vec<Value> = root_uris
                    .iter()
                    .map(|uri| {
                        json!({
                            "uri": uri,
                            "name": "root"
                        })
                    })
                    .collect();

                let response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "roots": roots_json
                    }
                });

                let _ = send_mcp_request(client, base_url, &response, Some(session_id))
                    .await
                    .expect("Failed to send roots/list response");
                roots_answered = true;
                post_roots_deadline =
                    Some(tokio::time::Instant::now() + post_roots_configured_grace_timeout());
            }
        }
    }
}

/// REGRESSION TEST (DO NOT WEAKEN): Cross-repo working_directory must succeed.
///
/// Real-world failure this guards against:
/// - Start the HTTP server from repo A (e.g. `ahma_mcp` checkout).
/// - Connect from VS Code opened on repo B.
/// - VS Code sends `tools/call` with `working_directory` in repo B.
/// - If the server is incorrectly scoped to repo A, it fails with:
///   "Path '...' is outside the sandbox root '...'".
///
/// The correct behavior is **per-session sandbox isolation**:
/// the sandbox scope must be derived from the client's `roots/list` response,
/// so repo B is allowed for that session even if the server was started elsewhere.
///
/// WARNING TO FUTURE AI/MAINTAINERS:
/// - Do NOT change this test to accept either success OR sandbox failure.
/// - Do NOT add test-mode env var bypasses (see SPEC.md R21.3).
/// - Fix scoping/session isolation if this fails.
#[tokio::test]
#[serial]
async fn test_tool_call_with_different_working_directory() {
    // Create temp directories:
    // - server_scope: where the HTTP server is started (repo A)
    // - client_scope: simulated VS Code workspace root (repo B)
    let server_scope_dir = TempDir::new().expect("Failed to create temp dir (server_scope)");
    let client_scope_dir = TempDir::new().expect("Failed to create temp dir (client_scope)");

    let tools_dir = server_scope_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    // Create a simple tool config that proves working_directory is honored.
    // We use `pwd` so we can assert the output contains the requested working directory.
    let tool_config = json!({
        "name": "pwd",
        "description": "Print current working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Print working directory"
        }]
    });
    std::fs::write(
        tools_dir.join("echo.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    // Server sandbox scope (what used to incorrectly apply to all clients)
    let sandbox_scope = server_scope_dir.path().to_path_buf();

    // Client workspace scope (what must apply to THIS session after roots/list)
    let different_project_path = client_scope_dir.path().to_path_buf();

    let (child, port) = start_http_bridge(&tools_dir, &sandbox_scope).await;
    let _server_guard = ServerGuard::new(child);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = Client::new();

    // Step 1: Send initialize request (no session ID - creates new session)
    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": { "listChanged": true }
            },
            "clientInfo": {
                "name": "test-client",
                "version": "1.0.0"
            }
        }
    });

    let result = send_mcp_request(&client, &base_url, &init_request, None).await;

    let (init_response, session_id) = match result {
        Ok(r) => r,
        Err(e) => panic!("Initialize request failed: {}", e),
    };

    // Debug: print the response
    eprintln!("Initialize response: {:?}", init_response);
    // Session IDs should not be logged verbatim (CodeQL).
    eprintln!("Session ID from header: <redacted>");

    let session_id = session_id.expect("Session isolation must return mcp-session-id header");

    // Verify initialize response
    assert!(
        init_response.get("result").is_some(),
        "Initialize should return result, got: {:?}",
        init_response
    );

    // Step 2: Send initialized notification
    let initialized_notification = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });

    // Open SSE and answer roots/list with the client workspace root.
    // This is what binds the per-session sandbox scope.
    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_root = different_project_path.clone();
    let sse_task = tokio::spawn(async move {
        answer_roots_list_over_sse(&sse_client, &sse_base_url, &sse_session_id, &[sse_root]).await;
    });

    send_mcp_request(
        &client,
        &base_url,
        &initialized_notification,
        Some(&session_id),
    )
    .await
    .expect("notifications/initialized should succeed");
    // Notifications don't return responses, that's OK

    // Ensure roots/list was observed and answered.
    sse_task.await.expect("roots/list SSE task panicked");

    // Step 3: Call a tool with working_directory OUTSIDE the server sandbox scope.
    // This MUST succeed because the session sandbox scope is derived from client roots.
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "execution_mode": "Synchronous",
                "working_directory": different_project_path.to_string_lossy()
            }
        }
    });

    let tool_response =
        send_tool_call_with_retry(&client, &base_url, &session_id, &tool_call).await;

    assert!(
        tool_response.get("error").is_none(),
        "Cross-repo tool call must succeed; got error: {:?}",
        tool_response
    );
    assert!(
        tool_response.get("result").is_some(),
        "Cross-repo tool call must return result; got: {:?}",
        tool_response
    );

    // Prove the working_directory was actually used.
    let output_text = tool_response
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let different_project_str = different_project_path.to_string_lossy();
    assert!(
        output_text.contains(different_project_str.as_ref()),
        "pwd output must include the requested working_directory. Output: {:?}",
        tool_response
    );
}

// NOTE: test_cargo_target_dir_is_scoped_to_working_directory was removed.
// It tested cargo-specific CARGO_TARGET_DIR env var overrides that are no longer used.
// The sandbox now relies on OS-level restrictions (sandbox-exec on macOS, Landlock on Linux)
// which are generic and apply to all tools, not just cargo.

/// Test: Basic tool call within sandbox scope works correctly
#[tokio::test]
#[serial]
async fn test_basic_tool_call_within_sandbox() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let tools_dir = temp_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    // Create pwd tool config
    let tool_config = json!({
        "name": "pwd",
        "description": "Print current working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Print working directory"
        }]
    });
    std::fs::write(
        tools_dir.join("echo.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let sandbox_scope = temp_dir.path().to_path_buf();
    let (child, port) = start_http_bridge(&tools_dir, &sandbox_scope).await;
    let _server = ServerGuard::new(child);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = Client::new();

    // Initialize
    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": { "listChanged": true }
            },
            "clientInfo": {
                "name": "test-client",
                "version": "1.0.0"
            }
        }
    });

    let (init_response, session_id) = send_mcp_request(&client, &base_url, &init_request, None)
        .await
        .expect("Initialize should succeed");

    // Verify initialize succeeded
    assert!(
        init_response.get("result").is_some(),
        "Initialize should return result, got: {:?}",
        init_response
    );

    let session_id_for_requests =
        session_id.expect("Session isolation must return mcp-session-id header");

    // Send initialized notification
    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    // Open SSE and answer roots/list with the sandbox scope.
    // In always-on session isolation, the subprocess runs with --defer-sandbox and
    // tool execution is blocked until roots/list has been answered.
    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id_for_requests.clone();
    let sse_root = sandbox_scope.clone();
    let sse_task = tokio::spawn(async move {
        answer_roots_list_over_sse(&sse_client, &sse_base_url, &sse_session_id, &[sse_root]).await;
    });

    let _ = send_mcp_request(
        &client,
        &base_url,
        &initialized,
        Some(&session_id_for_requests),
    )
    .await;

    // Ensure roots/list was observed and answered.
    sse_task.await.expect("roots/list SSE task panicked");

    // Call tool WITHIN sandbox scope
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": sandbox_scope.to_string_lossy()
            }
        }
    });

    let response =
        send_tool_call_with_retry(&client, &base_url, &session_id_for_requests, &tool_call).await;

    // Should have result, not "expect initialized request" error
    let error = response.get("error");
    if let Some(err) = error {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("");
        assert!(
            !msg.contains("expect initialized request"),
            "Should NOT get 'expect initialized request' error. Got: {:?}",
            response
        );
    }
}

/// Roots URIs may be percent-encoded (spaces/unicode) by real IDE clients.
/// Session isolation must decode these correctly so sandbox scope matches the workspace.
#[tokio::test]
#[serial]
async fn test_roots_uri_parsing_percent_encoded_path() {
    let server_scope_dir = TempDir::new().expect("Failed to create temp dir (server_scope)");
    let client_scope_dir = TempDir::new().expect("Failed to create temp dir (client_scope)");

    let tools_dir = server_scope_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    // Create pwd tool config
    let tool_config = json!({
        "name": "pwd",
        "description": "Print current working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Print working directory"
        }]
    });
    std::fs::write(
        tools_dir.join("pwd.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    // Make a workspace root with space + unicode in the path.
    let client_root = client_scope_dir.path().join("my proj OK");
    tokio::fs::create_dir_all(&client_root)
        .await
        .expect("Failed to create client root");

    let (child, port) = start_http_bridge(&tools_dir, server_scope_dir.path()).await;
    let _server_guard = ServerGuard::new(child);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = Client::new();

    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": { "listChanged": true }
            },
            "clientInfo": {"name": "test-client", "version": "1.0.0"}
        }
    });
    let (_, session_id) = send_mcp_request(&client, &base_url, &init_request, None)
        .await
        .expect("Initialize should succeed");
    let session_id = session_id.expect("Session isolation must return mcp-session-id header");

    let encoded_path = percent_encode_path_for_file_uri(&client_root);
    let uri = format!("file://{}", encoded_path);

    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_uri = uri.clone();
    let sse_task = tokio::spawn(async move {
        answer_roots_list_over_sse_with_uris(
            &sse_client,
            &sse_base_url,
            &sse_session_id,
            &[sse_uri],
        )
        .await;
    });

    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let _ = send_mcp_request(&client, &base_url, &initialized, Some(&session_id)).await;
    sse_task.await.expect("roots/list SSE task panicked");

    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": client_root.to_string_lossy()
            }
        }
    });

    let resp = send_tool_call_with_retry(&client, &base_url, &session_id, &tool_call).await;

    assert!(
        resp.get("error").is_none(),
        "pwd must succeed, got: {resp:?}"
    );

    let output_text = resp
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    assert!(
        output_text.contains(client_root.to_string_lossy().as_ref()),
        "pwd output must include decoded client root path; got: {resp:?}"
    );
}

/// Some clients send file URIs in host form: file://localhost/abs/path
#[tokio::test]
#[serial]
async fn test_roots_uri_parsing_file_localhost() {
    let server_scope_dir = TempDir::new().expect("Failed to create temp dir (server_scope)");
    let client_scope_dir = TempDir::new().expect("Failed to create temp dir (client_scope)");

    let tools_dir = server_scope_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    let tool_config = json!({
        "name": "pwd",
        "description": "Print current working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Print working directory"
        }]
    });
    std::fs::write(
        tools_dir.join("pwd.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let client_root = client_scope_dir.path().join("my proj OK");
    tokio::fs::create_dir_all(&client_root)
        .await
        .expect("Failed to create client root");

    // Start server on dynamic port to avoid conflicts/flakiness
    let (child, port, _stderr) =
        start_http_bridge_dynamic(&tools_dir, server_scope_dir.path()).await;
    let _server_guard = ServerGuard::new(child);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = Client::new();

    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": { "listChanged": true }
            },
            "clientInfo": {"name": "test-client", "version": "1.0.0"}
        }
    });
    let (_, session_id) = send_mcp_request(&client, &base_url, &init_request, None)
        .await
        .expect("Initialize should succeed");
    let session_id = session_id.expect("Session isolation must return mcp-session-id header");

    let encoded_path = percent_encode_path_for_file_uri(&client_root);
    let uri = format!("file://localhost{}", encoded_path);

    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_uri = uri.clone();
    let sse_task = tokio::spawn(async move {
        answer_roots_list_over_sse_with_uris(
            &sse_client,
            &sse_base_url,
            &sse_session_id,
            &[sse_uri],
        )
        .await;
    });

    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let _ = send_mcp_request(&client, &base_url, &initialized, Some(&session_id)).await;
    sse_task.await.expect("roots/list SSE task panicked");

    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": client_root.to_string_lossy()
            }
        }
    });

    let resp = send_tool_call_with_retry(&client, &base_url, &session_id, &tool_call).await;

    assert!(
        resp.get("error").is_none(),
        "pwd must succeed, got: {resp:?}"
    );
}

/// Red-team: working_directory with '..' that resolves outside root must be rejected.
#[tokio::test]
#[serial]
async fn test_rejects_working_directory_path_traversal_outside_root() {
    let server_scope_dir = TempDir::new().expect("Failed to create temp dir (server_scope)");
    let sandbox_parent = TempDir::new().expect("Failed to create temp dir (sandbox_parent)");

    let client_root = sandbox_parent.path().join("root");
    let outside_dir = sandbox_parent.path().join("outside");
    tokio::fs::create_dir_all(&client_root)
        .await
        .expect("Failed to create client root");
    tokio::fs::create_dir_all(&outside_dir)
        .await
        .expect("Failed to create outside dir");

    let tools_dir = server_scope_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    let tool_config = json!({
        "name": "pwd",
        "description": "Print current working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Print working directory"
        }]
    });
    std::fs::write(
        tools_dir.join("pwd.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let (child, port) = start_http_bridge(&tools_dir, server_scope_dir.path()).await;
    let _server_guard = ServerGuard::new(child);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = Client::new();

    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": { "listChanged": true }
            },
            "clientInfo": {"name": "test-client", "version": "1.0.0"}
        }
    });
    let (_, session_id) = send_mcp_request(&client, &base_url, &init_request, None)
        .await
        .expect("Initialize should succeed");
    let session_id = session_id.expect("Session isolation must return mcp-session-id header");

    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_root = client_root.clone();
    let sse_task = tokio::spawn(async move {
        answer_roots_list_over_sse(&sse_client, &sse_base_url, &sse_session_id, &[sse_root]).await;
    });

    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let _ = send_mcp_request(&client, &base_url, &initialized, Some(&session_id)).await;
    sse_task.await.expect("roots/list SSE task panicked");

    let traversal = client_root
        .join("subdir")
        .join("..")
        .join("..")
        .join("outside");

    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": traversal.to_string_lossy()
            }
        }
    });

    let resp = send_tool_call_with_retry(&client, &base_url, &session_id, &tool_call).await;

    assert!(
        resp.get("error").is_some(),
        "Expected sandbox rejection, got: {resp:?}"
    );
    let msg = resp
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("");
    assert!(
        msg.contains("outside") && msg.contains("sandbox"),
        "Expected sandbox boundary error message, got: {msg:?}"
    );
}

/// Red-team: symlink inside root pointing outside must not allow writes outside root.
#[tokio::test]
#[serial]
async fn test_symlink_escape_attempt_is_blocked() {
    let server_scope_dir = TempDir::new().expect("Failed to create temp dir (server_scope)");
    let sandbox_parent = TempDir::new().expect("Failed to create temp dir (sandbox_parent)");

    let client_root = sandbox_parent.path().join("root");
    let outside_dir = sandbox_parent.path().join("outside");
    tokio::fs::create_dir_all(&client_root)
        .await
        .expect("Failed to create client root");
    tokio::fs::create_dir_all(&outside_dir)
        .await
        .expect("Failed to create outside dir");

    // Symlink inside root -> outside
    let escape_link = client_root.join("escape");
    unix_fs::symlink(&outside_dir, &escape_link).expect("Failed to create symlink");

    let tools_dir = server_scope_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    // Copy the workspace file-tools config so we can attempt a write.
    let workspace_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get workspace dir")
        .to_path_buf();
    std::fs::copy(
        workspace_dir.join(".ahma/file-tools.json"),
        tools_dir.join("file-tools.json"),
    )
    .expect("Failed to copy file-tools tool config");

    let (child, port) = start_http_bridge(&tools_dir, server_scope_dir.path()).await;
    let _server_guard = ServerGuard::new(child);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = Client::new();

    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": { "listChanged": true }
            },
            "clientInfo": {"name": "test-client", "version": "1.0.0"}
        }
    });
    let (_, session_id) = send_mcp_request(&client, &base_url, &init_request, None)
        .await
        .expect("Initialize should succeed");
    let session_id = session_id.expect("Session isolation must return mcp-session-id header");

    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_root = client_root.clone();
    let sse_task = tokio::spawn(async move {
        answer_roots_list_over_sse(&sse_client, &sse_base_url, &sse_session_id, &[sse_root]).await;
    });

    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let _ = send_mcp_request(&client, &base_url, &initialized, Some(&session_id)).await;
    sse_task.await.expect("roots/list SSE task panicked");

    // Attempt to create a file that would resolve outside the sandbox via symlink.
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "file-tools",
            "arguments": {
                "subcommand": "touch",
                "working_directory": client_root.to_string_lossy(),
                "files": ["escape/owned.txt"]
            }
        }
    });

    let resp = send_tool_call_with_retry(&client, &base_url, &session_id, &tool_call).await;

    assert!(
        !outside_dir.join("owned.txt").exists(),
        "Symlink escape must not create files outside sandbox root"
    );

    // file-tools failures may be represented either as a JSON-RPC error or as result.isError=true.
    let is_jsonrpc_error = resp.get("error").is_some();
    let is_tool_error = resp
        .get("result")
        .and_then(|r| r.get("isError"))
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    assert!(
        is_jsonrpc_error || is_tool_error,
        "Expected sandbox/tool rejection signal, got: {resp:?}"
    );
}

/// Test: Tool call WITHOUT initialize should fail with proper error
///
/// This test reproduces the bug where the HTTP bridge allows a tools/call
/// to be sent before initialize, causing the subprocess to error with
/// "expect initialized request".
///
/// The expected behavior: The HTTP bridge should reject requests that come
/// before initialize, OR handle initialization automatically.
#[tokio::test]
#[serial]
async fn test_tool_call_without_initialize_returns_proper_error() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let tools_dir = temp_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    // Create pwd tool config
    let tool_config = json!({
        "name": "pwd",
        "description": "Print current working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Print working directory"
        }]
    });
    std::fs::write(
        tools_dir.join("echo.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let sandbox_scope = temp_dir.path().to_path_buf();
    let (child, port) = start_http_bridge(&tools_dir, &sandbox_scope).await;
    let _server_guard = ServerGuard::new(child);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = Client::new();

    // SKIP initialize - send tools/call directly
    // This reproduces the user's bug where the subprocess gets a tools/call first
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": sandbox_scope.to_string_lossy()
            }
        }
    });

    let result = send_mcp_request(&client, &base_url, &tool_call, None).await;

    eprintln!("Tool call without initialize result: {:?}", result);

    // This SHOULD fail - but the question is HOW it fails
    // Good: HTTP 400 or JSON-RPC error saying "not initialized" or similar
    // Bad: "expect initialized request" (means subprocess crashed)

    // Check HTTP response
    let response_error_msg = match &result {
        Ok((response, _)) => response
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string(),
        Err(e) => e.clone(),
    };

    // Also check HTTP response doesn't contain this error
    assert!(
        !response_error_msg.contains("expect initialized request"),
        "BUG: HTTP response contains 'expect initialized request': {}",
        response_error_msg
    );
}

/// Start HTTP bridge on random port (0) and parse the bound port from stderr.
/// Returns (child_process, port, stderr_receiver).
/// Stderr is forwarded to test stderr and captured.
async fn start_http_bridge_dynamic(
    tools_dir: &std::path::Path,
    sandbox_scope: &std::path::Path,
) -> (
    std::process::Child,
    u16,
    std::sync::Arc<std::sync::Mutex<String>>,
) {
    let binary = get_ahma_mcp_binary();
    let mut cmd = Command::new(&binary);
    cmd.args([
        "--mode",
        "http",
        "--http-port",
        "0",
        "--sync",
        "--tools-dir",
        &tools_dir.to_string_lossy(),
        "--sandbox-scope",
        &sandbox_scope.to_string_lossy(),
        "--log-to-stderr",
    ])
    .env_remove("NEXTEST")
    .env_remove("NEXTEST_EXECUTION_MODE")
    .env_remove("CARGO_TARGET_DIR")
    .env_remove("RUST_TEST_THREADS");

    #[cfg(target_os = "macos")]
    if ahma_mcp::sandbox::test_sandbox_exec_available().is_err() {
        cmd.env("AHMA_NO_SANDBOX", "1");
    }
    #[cfg(windows)]
    cmd.env("AHMA_NO_SANDBOX", "1");

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start HTTP bridge");

    let stderr = child.stderr.take().expect("Failed to capture stderr");
    let stderr_buffer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let buffer_clone = stderr_buffer.clone();
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(stderr);
        let mut port_found = false;
        for line in reader.lines() {
            let line = line.unwrap_or_default();
            {
                let mut buf = buffer_clone.lock().unwrap();
                buf.push_str(&line);
                buf.push('\n');
            }
            eprintln!("[server] {}", line); // Forward to test output

            if !port_found
                && let Some(port_str) = line.trim().strip_prefix("AHMA_BOUND_PORT=")
                && let Ok(port) = port_str.parse::<u16>()
            {
                let _ = tx.send(port);
                port_found = true;
            }
        }
    });

    // Wait for port with timeout
    let port = match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(p) => p,
        Err(_) => {
            let _ = child.kill();
            let buf = stderr_buffer.lock().unwrap();
            panic!(
                "Failed to start server (timeout waiting for port). Stderr:\n{}",
                *buf
            );
        }
    };

    // Wait for health check using the discovered port
    let client = Client::new();
    let health_url = format!("http://127.0.0.1:{}/health", port);

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            let buf = stderr_buffer.lock().unwrap();
            panic!(
                "Server started on port {} but health check failed. Stderr:\n{}",
                port, *buf
            );
        }
        if let Ok(resp) = client.get(&health_url).send().await
            && resp.status().is_success()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    (child, port, stderr_buffer)
}
