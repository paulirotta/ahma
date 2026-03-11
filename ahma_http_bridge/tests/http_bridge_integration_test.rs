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

mod common;

use common::server::{ServerGuard, resolve_binary_path};
use common::uri::paths_equivalent;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use serial_test::serial;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::time::sleep;

/// Start the HTTP bridge server and return a ServerGuard
async fn start_http_bridge(
    tools_dir: &std::path::Path,
    sandbox_scope: &std::path::Path,
) -> ServerGuard {
    let binary = resolve_binary_path();

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
    #[cfg(target_os = "linux")]
    if ahma_mcp::sandbox::check_sandbox_prerequisites().is_err() {
        cmd.env("AHMA_NO_SANDBOX", "1");
    }
    #[cfg(windows)]
    if ahma_mcp::sandbox::check_windows_sandbox_available().is_err() {
        cmd.env("AHMA_NO_SANDBOX", "1");
    }

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
    let timeout = Duration::from_secs(30);
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
            return ServerGuard::new(child, port);
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
        Duration::from_secs(120)
    } else {
        Duration::from_secs(45)
    }
}

fn post_roots_configured_grace_timeout() -> Duration {
    if coverage_mode() {
        // Coverage jobs can miss the follow-up notification on this specific
        // GET SSE stream even after the bridge has accepted the roots/list
        // response. Give it extra time, but keep the wait bounded so later
        // tool-call retries can validate sandbox activation instead.
        Duration::from_secs(30)
    } else if cfg!(windows) {
        Duration::from_secs(45)
    } else {
        Duration::from_secs(3)
    }
}

fn first_sse_event_boundary(buffer: &str) -> Option<(usize, usize)> {
    let lf = buffer.find("\n\n").map(|idx| (idx, 2));
    let crlf = buffer.find("\r\n\r\n").map(|idx| (idx, 4));
    match (lf, crlf) {
        (Some((lf_idx, lf_len)), Some((crlf_idx, crlf_len))) => {
            if lf_idx <= crlf_idx {
                Some((lf_idx, lf_len))
            } else {
                Some((crlf_idx, crlf_len))
            }
        }
        (Some(found), None) | (None, Some(found)) => Some(found),
        (None, None) => None,
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

/// Process an already-open SSE response to complete the MCP roots handshake.
/// Reads `roots/list`, responds with `roots_json`, and prefers to observe
/// `notifications/sandbox/configured` on the same GET SSE stream.
///
/// Under `cargo llvm-cov nextest`, the bridge can successfully lock the sandbox
/// after the `roots/list` response while the test client never observes the
/// follow-up notification on that specific SSE stream before its timeout.
/// Returning after a bounded grace period keeps the test focused on the real
/// invariant: roots were provided, and later `tools/call` requests must succeed
/// once sandbox activation completes.
///
/// Callers must open the SSE connection *before* sending
/// `notifications/initialized` to avoid the race where the server fires
/// `roots/list` before the client has a listener.
async fn process_sse_roots_handshake(
    sse_resp: reqwest::Response,
    client: &Client,
    base_url: &str,
    session_id: &str,
    roots_json: Vec<Value>,
) {
    assert!(
        sse_resp.status().is_success(),
        "SSE stream must be available, got HTTP {}",
        sse_resp.status()
    );

    let mut stream = sse_resp.bytes_stream();
    let mut buffer = String::new();
    let mut roots_answered = false;
    let mut configured_seen = false;
    let mut post_roots_deadline: Option<tokio::time::Instant> = None;

    let roots_deadline = tokio::time::Instant::now() + roots_handshake_timeout();
    loop {
        if let Some(timeout_at) = post_roots_deadline
            && tokio::time::Instant::now() > timeout_at
        {
            eprintln!(
                "WARNING: did not observe notifications/sandbox/configured after roots/list response; continuing and relying on tools/call retry to verify sandbox activation"
            );
            return;
        }

        if !roots_answered && tokio::time::Instant::now() > roots_deadline {
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

            while let Some((idx, delimiter_len)) = first_sse_event_boundary(&buffer) {
                let raw_event = buffer[..idx].to_string();
                buffer.drain(..idx + delimiter_len);

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
                    configured_seen = true;
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
                if configured_seen {
                    return;
                }
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

    let roots_json: Vec<Value> = roots
        .iter()
        .map(|p| {
            // Use percent_encode_path_for_file_uri for cross-platform file URI formatting
            // (Windows needs file:///C:/... format, not file://C:\...)
            json!({
                "uri": format!("file://{}", percent_encode_path_for_file_uri(p)),
                "name": p.file_name().and_then(|n| n.to_str()).unwrap_or("root")
            })
        })
        .collect();

    process_sse_roots_handshake(resp, client, base_url, session_id, roots_json).await;
}

fn percent_encode_path_for_file_uri(path: &std::path::Path) -> String {
    // Produce an RFC 8089-compatible file URI *path component* from a
    // filesystem path.  This string is meant to be appended directly after
    // the authority (e.g. `file://localhost` or `file://`).
    //
    // On Windows:
    //   1. Strip any `\\?\` extended-length prefix.
    //   2. Convert backslashes to forward slashes.
    //   3. Prepend "/" so the drive letter (e.g. "C:") is in the *path*
    //      component, not the authority field of the URL.
    //      Without this step `file://localhostC%3A...` would put
    //      `localhostC%3A...` in the host field, breaking `url::Url::parse`.
    let mut s = path.to_string_lossy().into_owned();

    // Strip \\?\ prefix (Windows extended-length paths).
    if s.starts_with(r"\\?\") {
        s = s[4..].to_string();
    }
    // Normalise path separators.
    s = s.replace('\\', "/");

    let mut out = String::with_capacity(s.len() + 1);

    // If this is a Windows drive-letter path (e.g. "C:/…"), the path
    // component must begin with "/" so the drive letter is not confused with
    // the URL authority.  This applies whether the caller will prepend
    // "file://" or "file://localhost".
    #[cfg(target_os = "windows")]
    {
        let is_drive =
            s.len() >= 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':';
        if is_drive {
            out.push('/');
        }
    }

    for b in s.as_bytes() {
        let b = *b;
        // Keep unreserved chars and forward slashes.
        // Keep ':' so Windows drive letters (C:/) are not encoded.
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

    let roots_json: Vec<Value> = root_uris
        .iter()
        .map(|uri| json!({"uri": uri, "name": "root"}))
        .collect();

    process_sse_roots_handshake(resp, client, base_url, session_id, roots_json).await;
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

    common::uri::create_pwd_tool_config(&tools_dir);

    // Server sandbox scope (what used to incorrectly apply to all clients)
    let sandbox_scope = server_scope_dir.path().to_path_buf();

    // Client workspace scope (what must apply to THIS session after roots/list)
    let different_project_path = client_scope_dir.path().to_path_buf();

    let server = start_http_bridge(&tools_dir, &sandbox_scope).await;
    let base_url = server.base_url();
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
    assert!(
        paths_equivalent(output_text, &different_project_path),
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

    common::uri::create_pwd_tool_config(&tools_dir);

    let sandbox_scope = temp_dir.path().to_path_buf();
    let server = start_http_bridge(&tools_dir, &sandbox_scope).await;
    let base_url = server.base_url();
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

    common::uri::create_pwd_tool_config(&tools_dir);

    // Make a workspace root with space + unicode in the path.
    let client_root = client_scope_dir.path().join("my proj OK");
    tokio::fs::create_dir_all(&client_root)
        .await
        .expect("Failed to create client root");

    let server = start_http_bridge(&tools_dir, server_scope_dir.path()).await;
    let base_url = server.base_url();
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

    // Open SSE stream BEFORE sending notifications/initialized so the server's
    // roots/list request is not lost if it fires immediately on initialized.
    let sse_resp = client
        .get(format!("{}/mcp", base_url))
        .header("Accept", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Mcp-Session-Id", &session_id)
        .send()
        .await
        .expect("Failed to open SSE stream");

    let roots_json = vec![json!({"uri": uri, "name": "root"})];
    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_task = tokio::spawn(async move {
        process_sse_roots_handshake(
            sse_resp,
            &sse_client,
            &sse_base_url,
            &sse_session_id,
            roots_json,
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
        paths_equivalent(output_text, &client_root),
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
    let (server, _stderr) = start_http_bridge_dynamic(&tools_dir, server_scope_dir.path()).await;
    let base_url = server.base_url();
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

    let server = start_http_bridge(&tools_dir, server_scope_dir.path()).await;
    let base_url = server.base_url();
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
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_symlink_escape_attempt_is_blocked() {
    use std::os::unix::fs as unix_fs;
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

    let server = start_http_bridge(&tools_dir, server_scope_dir.path()).await;
    let base_url = server.base_url();
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

    common::uri::create_pwd_tool_config(&tools_dir);

    let sandbox_scope = temp_dir.path().to_path_buf();
    let server = start_http_bridge(&tools_dir, &sandbox_scope).await;
    let base_url = server.base_url();
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
/// Returns (ServerGuard, stderr_receiver).
/// Stderr is forwarded to test stderr and captured.
async fn start_http_bridge_dynamic(
    tools_dir: &std::path::Path,
    sandbox_scope: &std::path::Path,
) -> (ServerGuard, std::sync::Arc<std::sync::Mutex<String>>) {
    let binary = resolve_binary_path();
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
    #[cfg(target_os = "linux")]
    if ahma_mcp::sandbox::check_sandbox_prerequisites().is_err() {
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
    let port = match rx.recv_timeout(Duration::from_secs(30)) {
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

    (ServerGuard::new(child, port), stderr_buffer)
}
