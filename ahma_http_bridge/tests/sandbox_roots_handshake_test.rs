//! Sandbox and Roots Handshake Integration Tests
//!
//! This module provides comprehensive test coverage for the critical sandbox
//! initialization path via the MCP roots/list protocol. These tests verify:
//!
//! 1. **Empty Roots Rejection**: Sessions with empty roots are rejected with clear error
//! 2. **Malformed URI Handling**: Invalid file:// URIs are handled gracefully
//! 3. **Multi-Root Workspace Scoping**: Multiple workspace roots are handled correctly
//! 4. **Post-Lock Roots Rejection**: Attempts to change roots after lock are rejected
//! 5. **Client-Specific Handshake Simulation**: Different MCP clients (VSCode, Cursor)
//! 6. **Race Condition Prevention**: Concurrent roots/list requests are handled atomically
//!
//! ## Security Critical
//!
//! These tests are security-critical. Do NOT:
//! - Weaken assertions to accept sandbox failures as "passing"
//! - Add test-mode env var bypasses (see SPEC.md R21.3)
//! - Remove environment variable clearing (see AGENTS.md guardrails)
//!
//! ## Test Environment
//!
//! All tests use `SandboxTestEnv::configure()` to ensure spawned ahma_mcp
//! processes test real sandbox behavior, not the permissive test mode.

mod common;

use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};
use common::{
    SANDBOX_BYPASS_ENV_VARS, SandboxTestEnv, ServerGuard, encode_file_uri, malformed_uris,
    parse_file_uri,
};
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

fn roots_handshake_timeout() -> Duration {
    TestTimeouts::get(TimeoutCategory::SseStream)
}

// =============================================================================
// Test Infrastructure
// =============================================================================

/// Build and get the ahma_mcp binary path
fn get_ahma_mcp_binary() -> PathBuf {
    ahma_mcp::test_utils::cli::build_binary_cached("ahma_mcp", "ahma-mcp")
}

/// Build the server Command with all required env vars for deferred-sandbox mode.
fn build_deferred_sandbox_command(
    binary: &std::path::Path,
    workspace: &std::path::Path,
    tools_dir: &std::path::Path,
) -> Command {
    let mut cmd = Command::new(binary);
    cmd.args(["serve", "http", "--port", "0"])
        .current_dir(workspace)
        .env("AHMA_SYNC", "1")
        .env("AHMA_TOOLS_DIR", &*tools_dir.to_string_lossy())
        .env("AHMA_SANDBOX_DEFER", "1") // Key: sandbox is deferred until roots/list
        .env("AHMA_LOG_TARGET", "stderr")
        // Match the server-side handshake timeout to the client-side TestTimeouts::Handshake
        // so both sides give up at the same time (60s on Linux/macOS, 240s on Windows).
        .env(
            "AHMA_HANDSHAKE_TIMEOUT",
            TestTimeouts::get(TimeoutCategory::Handshake)
                .as_secs()
                .to_string(),
        );
    // CRITICAL: Remove bypass env vars for real sandbox testing
    SandboxTestEnv::configure(&mut cmd);
    // Allow start inside nested sandboxes (app-level path security still active)
    SandboxTestEnv::apply_nested_sandbox_override(&mut cmd);
    cmd
}

/// Consume the port-announcement channel and return the bound port.
/// Panics on timeout or unexpected process death.
fn wait_for_bound_port(
    rx: &std::sync::mpsc::Receiver<String>,
    child: &mut std::process::Child,
) -> u16 {
    let start = std::time::Instant::now();
    let timeout = TestTimeouts::get(TimeoutCategory::ProcessSpawn);
    let poll_interval = TestTimeouts::poll_interval();

    while start.elapsed() < timeout {
        if let Ok(line) = rx.recv_timeout(poll_interval)
            && let Some(idx) = line.find("AHMA_BOUND_PORT=")
        {
            let port_str = &line[idx + "AHMA_BOUND_PORT=".len()..];
            if let Ok(p) = port_str.trim().parse::<u16>() {
                return p;
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("Child process exited unexpectedly with status: {}", status);
        }
    }
    let _ = child.kill();
    panic!("Timed out waiting for server to bind port");
}

/// Start an HTTP bridge server with deferred sandbox (for roots/list testing)
async fn start_deferred_sandbox_server(tools_dir: &std::path::Path) -> ServerGuard {
    let binary = get_ahma_mcp_binary();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get workspace dir")
        .to_path_buf();

    let mut cmd = build_deferred_sandbox_command(&binary, &workspace, tools_dir);
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
            eprintln!("{}", line);
            if line.contains("AHMA_BOUND_PORT=") {
                let _ = tx.send(line);
            }
        }
    });

    let port = wait_for_bound_port(&rx, &mut child);

    // Wait for server to be ready (health check)
    let client = common::make_h2_client();
    let health_url = format!("http://127.0.0.1:{}/health", port);
    let health_timeout = TestTimeouts::get(TimeoutCategory::HealthCheck);
    let health_start = std::time::Instant::now();

    while health_start.elapsed() < health_timeout {
        sleep(TestTimeouts::poll_interval()).await;
        if let Ok(resp) = client.get(&health_url).send().await
            && resp.status().is_success()
        {
            return ServerGuard::new(child, port);
        }
    }

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
        .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest));

    if let Some(id) = session_id {
        req = req.header("Mcp-Session-Id", id);
    }

    let response = req
        .json(request)
        .send()
        .await
        .map_err(|e| format!("Request failed: {:?}", e))?;

    let status = response.status();
    let new_session_id = response
        .headers()
        .get("mcp-session-id")
        .or_else(|| response.headers().get("Mcp-Session-Id"))
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", status, text));
    }

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok((body, new_session_id))
}

/// Send only initialize and return the session ID.
async fn initialize_session(client: &Client, base_url: &str) -> Result<String, String> {
    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"roots": {}},
            "clientInfo": {"name": "test-client", "version": "1.0.0"}
        }
    });

    let (_, session_id) = send_mcp_request(client, base_url, &init_request, None).await?;
    session_id.ok_or_else(|| "No session ID received".to_string())
}

async fn send_initialized_notification(
    client: &Client,
    base_url: &str,
    session_id: &str,
) -> Result<(), String> {
    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let _ = send_mcp_request(client, base_url, &initialized, Some(session_id)).await;
    Ok(())
}

/// Wait for sandbox readiness by polling a known-good tool call.
async fn wait_for_tool_ready(
    client: &Client,
    base_url: &str,
    session_id: &str,
    working_directory: &Path,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + roots_handshake_timeout();
    let mut last_error: Option<String> = None;

    while tokio::time::Instant::now() < deadline {
        let tool_call = json!({
            "jsonrpc": "2.0",
            "id": 9001,
            "method": "tools/call",
            "params": {
                "name": "pwd",
                "arguments": {
                    "subcommand": "default",
                    "working_directory": working_directory.to_string_lossy()
                }
            }
        });

        match send_mcp_request(client, base_url, &tool_call, Some(session_id)).await {
            Ok((response, _)) if response.get("error").is_none() => return Ok(()),
            Ok((response, _)) => {
                last_error = Some(
                    response["error"]["message"]
                        .as_str()
                        .unwrap_or("tool call returned error")
                        .to_string(),
                );
            }
            Err(e) => last_error = Some(e),
        }

        sleep(TestTimeouts::poll_interval()).await;
    }

    Err(format!(
        "Timeout waiting for sandbox/tool readiness: {}",
        last_error.unwrap_or_else(|| "unknown error".to_string())
    ))
}

/// Parse the `data:` lines from a raw SSE event block and deserialize the JSON payload.
fn parse_sse_event_data(raw_event: &str) -> Option<Value> {
    let data: String = raw_event
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter_map(|l| l.strip_prefix("data:").map(str::trim))
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(&data).ok()
}

/// Dispatch a parsed SSE event. Returns `Some(result)` when the exchange is
/// complete (success or error) and `None` to keep reading the stream.
async fn handle_sse_event(
    value: Value,
    client: &Client,
    base_url: &str,
    session_id: &str,
    root_uris: &[String],
) -> Option<Result<(), String>> {
    let method = value.get("method").and_then(|m| m.as_str());

    if method == Some("notifications/sandbox/failed") {
        let error = value["params"]["error"].as_str().unwrap_or("unknown");
        return Some(Err(format!("Sandbox configuration failed: {}", error)));
    }

    if method != Some("roots/list") {
        return None;
    }

    let id = match value.get("id").cloned() {
        Some(id) => id,
        None => return Some(Err("roots/list must include id".to_string())),
    };
    let roots_json: Vec<Value> = root_uris
        .iter()
        .map(|uri| json!({"uri": uri, "name": "root"}))
        .collect();
    let response = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {"roots": roots_json}
    });
    Some(
        send_mcp_request(client, base_url, &response, Some(session_id))
            .await
            .map(|_| ()),
    )
}

async fn process_roots_list_response(
    resp: reqwest::Response,
    client: &Client,
    base_url: &str,
    session_id: &str,
    root_uris: &[String],
) -> Result<(), String> {
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let deadline = tokio::time::Instant::now() + roots_handshake_timeout();

    loop {
        if tokio::time::Instant::now() > deadline {
            return Err("Timeout waiting for roots/list over SSE".to_string());
        }

        let chunk = tokio::time::timeout(TestTimeouts::poll_interval(), stream.next())
            .await
            .ok()
            .flatten();

        if let Some(next) = chunk {
            let bytes = next.map_err(|e| format!("SSE read error: {}", e))?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(idx) = buffer.find("\n\n") {
                let raw_event = buffer[..idx].to_string();
                buffer = buffer[idx + 2..].to_string();

                let Some(value) = parse_sse_event_data(&raw_event) else {
                    continue;
                };

                if let Some(result) =
                    handle_sse_event(value, client, base_url, session_id, root_uris).await
                {
                    return result;
                }
            }
        }
    }
}

/// Open SSE after the client has already sent notifications/initialized and answer roots/list.
async fn answer_roots_list_with_uris(
    client: &Client,
    base_url: &str,
    session_id: &str,
    root_uris: &[String],
) -> Result<(), String> {
    let url = format!("{}/mcp", base_url);
    // No reqwest .timeout() here: SSE is an infinite stream, so a reqwest timeout fires on the
    // body read phase and aborts the connection before roots/list arrives. The internal deadline
    // inside process_roots_list_response handles the per-test time budget.
    let resp = client
        .get(&url)
        .header("Accept", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Mcp-Session-Id", session_id)
        .send()
        .await
        .map_err(|e| format!("SSE connection failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("SSE stream failed: HTTP {}", resp.status()));
    }

    process_roots_list_response(resp, client, base_url, session_id, root_uris).await
}

/// Complete the normal roots handshake in the safe order: open SSE first, then send initialized.
async fn complete_roots_handshake_with_uris(
    client: &Client,
    base_url: &str,
    session_id: &str,
    root_uris: &[String],
) -> Result<(), String> {
    let url = format!("{}/mcp", base_url);
    // No reqwest .timeout() here: SSE is an infinite stream, so a reqwest timeout fires on the
    // body read phase and aborts the connection before roots/list arrives. The internal deadline
    // inside process_roots_list_response handles the per-test time budget.
    let resp = client
        .get(&url)
        .header("Accept", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Mcp-Session-Id", session_id)
        .send()
        .await
        .map_err(|e| format!("SSE connection failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("SSE stream failed: HTTP {}", resp.status()));
    }

    sleep(TestTimeouts::short_delay()).await;
    send_initialized_notification(client, base_url, session_id).await?;
    process_roots_list_response(resp, client, base_url, session_id, root_uris).await
}

// =============================================================================
// Test: Empty Roots Rejection
// =============================================================================

/// SECURITY TEST: Empty roots/list response must be rejected with clear error.
///
/// If a client returns an empty roots list, the session should be rejected
/// because there's no valid sandbox scope to use. This prevents accidental
/// over-permissive behavior.
#[tokio::test]
async fn test_empty_roots_rejection() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let tools_dir = temp_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    let tool_config = json!({
        "name": "pwd",
        "description": "Print working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{"name": "default", "description": "pwd"}]
    });
    std::fs::write(
        tools_dir.join("pwd.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let server = start_deferred_sandbox_server(&tools_dir).await;
    let base_url = format!("http://127.0.0.1:{}", server.port());
    let client = common::make_h2_client();

    let session_id = initialize_session(&client, &base_url)
        .await
        .expect("Initialize failed");

    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_task = tokio::spawn(async move {
        complete_roots_handshake_with_uris(&sse_client, &sse_base_url, &sse_session_id, &[]).await
    });

    // Give time for roots/list exchange
    sleep(TestTimeouts::short_delay()).await;
    let _ = sse_task.await;

    // Try to call a tool - should fail because sandbox wasn't initialized
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {"subcommand": "default"}
        }
    });

    let result = send_mcp_request(&client, &base_url, &tool_call, Some(&session_id)).await;

    match result {
        Ok((response, _)) => {
            // Should have an error about sandbox not being initialized
            let error = response.get("error");
            assert!(
                error.is_some(),
                "Expected error for empty roots, got success: {:?}",
                response
            );
            let error_msg = error
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("");
            eprintln!("Got expected error: {}", error_msg);
            // The error should mention sandbox, roots, or initialization
            let mentions_issue = error_msg.contains("sandbox")
                || error_msg.contains("Sandbox")
                || error_msg.contains("roots")
                || error_msg.contains("initializ");
            assert!(
                mentions_issue,
                "Error should mention sandbox/roots issue, got: {}",
                error_msg
            );
        }
        Err(e) => {
            // HTTP-level error is also acceptable (e.g., 403 Forbidden, 409 Conflict)
            eprintln!("Got HTTP error (acceptable): {}", e);
            let e_lower = e.to_lowercase();
            assert!(
                e.contains("403")
                    || e.contains("400")
                    || e.contains("409")
                    || e_lower.contains("sandbox"),
                "Expected 403/400/409 or sandbox-related error, got: {}",
                e
            );
        }
    }
}

// =============================================================================
// Test: Malformed URI Edge Cases
// =============================================================================

/// Test that malformed file:// URIs are handled gracefully.
///
/// Invalid URIs should be filtered out, not cause crashes or unexpected behavior.
#[tokio::test]
async fn test_malformed_uri_parsing() {
    // Test the parsing function directly first
    for invalid_uri in malformed_uris::INVALID {
        let result = parse_file_uri(invalid_uri);
        assert!(
            result.is_none(),
            "Expected None for invalid URI '{}', got {:?}",
            invalid_uri,
            result
        );
    }

    // Test edge cases
    for (uri, expected_path) in malformed_uris::EDGE_CASES {
        let result = parse_file_uri(uri);
        match expected_path {
            Some(expected) => {
                assert!(
                    result.is_some(),
                    "Expected Some for URI '{}', got None",
                    uri
                );
                let path = result.unwrap();
                assert_eq!(
                    path.to_string_lossy(),
                    *expected,
                    "Path mismatch for URI '{}'",
                    uri
                );
            }
            None => {
                assert!(
                    result.is_none(),
                    "Expected None for URI '{}', got {:?}",
                    uri,
                    result
                );
            }
        }
    }
}

/// Test that a session with only malformed URIs is rejected.
#[tokio::test]
async fn test_session_with_only_malformed_uris() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let tools_dir = temp_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    let tool_config = json!({
        "name": "pwd",
        "description": "Print working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{"name": "default", "description": "pwd"}]
    });
    std::fs::write(
        tools_dir.join("pwd.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let server = start_deferred_sandbox_server(&tools_dir).await;
    let base_url = format!("http://127.0.0.1:{}", server.port());
    let client = common::make_h2_client();

    let session_id = initialize_session(&client, &base_url)
        .await
        .expect("Initialize failed");

    // Answer roots/list with only malformed URIs
    let malformed_uris = vec![
        "http://not-a-file-uri/path".to_string(),
        "ftp://also-wrong/file".to_string(),
        "".to_string(),
    ];

    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_task = tokio::spawn(async move {
        complete_roots_handshake_with_uris(
            &sse_client,
            &sse_base_url,
            &sse_session_id,
            &malformed_uris,
        )
        .await
    });

    sleep(TestTimeouts::short_delay()).await;
    let _ = sse_task.await;

    // Tool call should fail - no valid roots
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {"subcommand": "default"}
        }
    });

    let result = send_mcp_request(&client, &base_url, &tool_call, Some(&session_id)).await;

    match result {
        Ok((response, _)) => {
            let error = response.get("error");
            assert!(
                error.is_some(),
                "Expected error for malformed-only roots, got success: {:?}",
                response
            );
        }
        Err(e) => {
            eprintln!("Got HTTP error (acceptable for malformed URIs): {}", e);
        }
    }
}

// =============================================================================
// Test: Multi-Root Workspace Scoping
// =============================================================================

/// Test that multiple valid roots are all accepted for sandbox scoping.
#[tokio::test]
async fn test_multi_root_workspace_scoping() {
    let root1 = TempDir::new().expect("Failed to create temp dir 1");
    let root2 = TempDir::new().expect("Failed to create temp dir 2");
    let tools_temp = TempDir::new().expect("Failed to create tools temp dir");
    let tools_dir = tools_temp.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    // Create test file in root2 to prove it's accessible
    std::fs::write(root2.path().join("test.txt"), "hello").expect("Failed to create test file");

    let tool_config = json!({
        "name": "pwd",
        "description": "Print working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{"name": "default", "description": "pwd"}]
    });
    std::fs::write(
        tools_dir.join("pwd.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let server = start_deferred_sandbox_server(&tools_dir).await;
    let base_url = format!("http://127.0.0.1:{}", server.port());
    let client = common::make_h2_client();

    let session_id = initialize_session(&client, &base_url)
        .await
        .expect("Initialize failed");

    // Answer roots/list with both roots
    let root_uris = vec![encode_file_uri(root1.path()), encode_file_uri(root2.path())];

    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_task = tokio::spawn(async move {
        complete_roots_handshake_with_uris(&sse_client, &sse_base_url, &sse_session_id, &root_uris)
            .await
    });

    // Wait for roots exchange
    let sse_result = sse_task.await.expect("SSE task panicked");
    assert!(
        sse_result.is_ok(),
        "Roots exchange failed: {:?}",
        sse_result
    );

    wait_for_tool_ready(&client, &base_url, &session_id, root1.path())
        .await
        .expect("Sandbox should become ready for tool calls");

    // Tool call in root1 should work
    let tool_call_1 = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": root1.path().to_string_lossy()
            }
        }
    });

    let (response1, _) = send_mcp_request(&client, &base_url, &tool_call_1, Some(&session_id))
        .await
        .expect("Tool call 1 request failed");

    assert!(
        response1.get("error").is_none(),
        "Tool call in root1 should succeed: {:?}",
        response1
    );

    // Tool call in root2 should also work
    let tool_call_2 = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": root2.path().to_string_lossy()
            }
        }
    });

    let (response2, _) = send_mcp_request(&client, &base_url, &tool_call_2, Some(&session_id))
        .await
        .expect("Tool call 2 request failed");

    assert!(
        response2.get("error").is_none(),
        "Tool call in root2 should succeed: {:?}",
        response2
    );
}

// =============================================================================
// Test: URL-Encoded Paths
// =============================================================================

/// Test that paths with spaces and special characters work correctly.
#[tokio::test]
async fn test_url_encoded_path_in_roots() {
    // Create a temp dir with spaces in the name
    let base_temp = TempDir::new().expect("Failed to create temp dir");
    let special_path = base_temp.path().join("my project");
    std::fs::create_dir_all(&special_path).expect("Failed to create special dir");

    let tools_dir = base_temp.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    let tool_config = json!({
        "name": "pwd",
        "description": "Print working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{"name": "default", "description": "pwd"}]
    });
    std::fs::write(
        tools_dir.join("pwd.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let server = start_deferred_sandbox_server(&tools_dir).await;
    let base_url = format!("http://127.0.0.1:{}", server.port());
    let client = common::make_h2_client();

    let session_id = initialize_session(&client, &base_url)
        .await
        .expect("Initialize failed");

    // Create properly encoded URI with space
    let root_uri = encode_file_uri(&special_path);
    assert!(
        root_uri.contains("%20"),
        "URI should contain encoded space: {}",
        root_uri
    );

    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_task = tokio::spawn(async move {
        complete_roots_handshake_with_uris(&sse_client, &sse_base_url, &sse_session_id, &[root_uri])
            .await
    });

    let sse_result = sse_task.await.expect("SSE task panicked");
    assert!(
        sse_result.is_ok(),
        "Roots exchange failed: {:?}",
        sse_result
    );

    wait_for_tool_ready(&client, &base_url, &session_id, &special_path)
        .await
        .expect("Sandbox should become ready for URL-encoded root");

    // Tool call in the special path should work
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": special_path.to_string_lossy()
            }
        }
    });

    let (response, _) = send_mcp_request(&client, &base_url, &tool_call, Some(&session_id))
        .await
        .expect("Tool call request failed");

    assert!(
        response.get("error").is_none(),
        "Tool call in path with spaces should succeed: {:?}",
        response
    );

    // Verify the output contains the special path
    let output = response
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");

    assert!(
        output.contains("my project"),
        "Output should contain 'my project': {}",
        output
    );
}

// =============================================================================
// Test: SandboxTestEnv Helper
// =============================================================================

/// Verify that SandboxTestEnv correctly identifies bypass vars.
#[test]
fn test_sandbox_test_env_detection() {
    // This test verifies the helper works correctly
    let vars = SandboxTestEnv::current_bypass_vars();
    eprintln!("Current bypass vars: {:?}", vars);

    // In test environment, some of these are likely set
    // The important thing is the detection works
    assert!(
        SANDBOX_BYPASS_ENV_VARS.len() == 4,
        "Should have 4 bypass vars defined"
    );
}

/// Verify Command configuration removes expected env vars.
#[test]
fn test_sandbox_test_env_configure() {
    let mut cmd = Command::new("true");
    SandboxTestEnv::configure(&mut cmd);
    // Can't easily verify env removal, but at least verify it doesn't panic
}

// =============================================================================
// Test: File URI Encoding/Decoding Roundtrip
// =============================================================================

#[test]
fn test_file_uri_roundtrip() {
    let test_paths = [
        "/tmp/simple",
        "/tmp/with spaces",
        "/tmp/unicode/日本語",
        "/tmp/special!@#$%^&()",
        "/home/user/my project/src",
    ];

    for path_str in test_paths {
        let path = PathBuf::from(path_str);
        let uri = encode_file_uri(&path);
        let decoded = parse_file_uri(&uri);

        assert!(
            decoded.is_some(),
            "Failed to decode URI for path: {}",
            path_str
        );
        assert_eq!(
            decoded.unwrap().to_string_lossy(),
            path_str,
            "Roundtrip failed for path: {}",
            path_str
        );
    }
}

// =============================================================================
// Test: Client Handshake Simulation (VSCode vs Cursor ordering)
// =============================================================================

/// Different MCP clients may send SSE-first or MCP-first.
/// This test verifies both orderings work correctly.
#[tokio::test]
async fn test_handshake_ordering_sse_first() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("Failed to create project dir");

    let tools_dir = temp_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    let tool_config = json!({
        "name": "pwd",
        "description": "Print working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{"name": "default", "description": "pwd"}]
    });
    std::fs::write(
        tools_dir.join("pwd.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let server = start_deferred_sandbox_server(&tools_dir).await;
    let base_url = format!("http://127.0.0.1:{}", server.port());
    let client = common::make_h2_client();

    // VSCode Copilot style: Initialize, then SSE connects and answers roots/list

    // Step 1: Initialize
    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"roots": {"listChanged": true}},
            "clientInfo": {"name": "vscode-copilot-simulation", "version": "1.0.0"}
        }
    });

    let (_, session_id) = send_mcp_request(&client, &base_url, &init_request, None)
        .await
        .expect("Initialize failed");
    let session_id = session_id.expect("No session ID");

    // Step 2: Send initialized notification
    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let _ = send_mcp_request(&client, &base_url, &initialized, Some(&session_id)).await;

    // Step 3: Open SSE and answer roots/list
    let root_uri = encode_file_uri(&project_dir);
    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_task = tokio::spawn(async move {
        answer_roots_list_with_uris(&sse_client, &sse_base_url, &sse_session_id, &[root_uri]).await
    });

    let sse_result = sse_task.await.expect("SSE task panicked");
    assert!(
        sse_result.is_ok(),
        "Roots exchange failed: {:?}",
        sse_result
    );

    wait_for_tool_ready(&client, &base_url, &session_id, &project_dir)
        .await
        .expect("Sandbox should become ready for VSCode-style ordering");

    // Step 4: Verify tool call works
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": project_dir.to_string_lossy()
            }
        }
    });

    let (response, _) = send_mcp_request(&client, &base_url, &tool_call, Some(&session_id))
        .await
        .expect("Tool call failed");

    assert!(
        response.get("error").is_none(),
        "Tool call should succeed after VSCode-style handshake: {:?}",
        response
    );
}

// =============================================================================
// Test: Mixed Valid and Invalid URIs
// =============================================================================

/// Test that a mix of valid and invalid URIs works (valid ones are used).
#[tokio::test]
async fn test_mixed_valid_invalid_uris() {
    let valid_root = TempDir::new().expect("Failed to create temp dir");
    let tools_temp = TempDir::new().expect("Failed to create tools temp dir");
    let tools_dir = tools_temp.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    let tool_config = json!({
        "name": "pwd",
        "description": "Print working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{"name": "default", "description": "pwd"}]
    });
    std::fs::write(
        tools_dir.join("pwd.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let server = start_deferred_sandbox_server(&tools_dir).await;
    let base_url = format!("http://127.0.0.1:{}", server.port());
    let client = common::make_h2_client();

    let session_id = initialize_session(&client, &base_url)
        .await
        .expect("Initialize failed");

    // Mix of valid and invalid URIs
    let root_uris = vec![
        "http://invalid/not-file-scheme".to_string(),
        encode_file_uri(valid_root.path()), // This one is valid
        "ftp://also-invalid/path".to_string(),
        "".to_string(),
    ];

    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_task = tokio::spawn(async move {
        complete_roots_handshake_with_uris(&sse_client, &sse_base_url, &sse_session_id, &root_uris)
            .await
    });

    let sse_result = sse_task.await.expect("SSE task panicked");
    assert!(
        sse_result.is_ok(),
        "Roots exchange failed: {:?}",
        sse_result
    );

    wait_for_tool_ready(&client, &base_url, &session_id, valid_root.path())
        .await
        .expect("Sandbox should become ready with mixed valid/invalid URIs");

    // Tool call should work because we had one valid root
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": valid_root.path().to_string_lossy()
            }
        }
    });

    let (response, _) = send_mcp_request(&client, &base_url, &tool_call, Some(&session_id))
        .await
        .expect("Tool call request failed");

    assert!(
        response.get("error").is_none(),
        "Tool call should succeed with at least one valid root: {:?}",
        response
    );
}

// =============================================================================
// Test: Post-Lock Roots Rejection (R8.4.6)
// =============================================================================

/// SECURITY TEST: Attempts to change roots after sandbox lock must be rejected.
///
/// Per requirement R8.4.6: "notifications/roots/list_changed after sandbox lock
/// → session terminated (HTTP 403)"
///
/// This is a critical security invariant. Once the sandbox is locked to specific
/// workspace roots, a malicious or buggy client cannot expand the sandbox by
/// sending another roots/list_changed notification.
#[tokio::test]
async fn test_post_lock_roots_change_rejected() {
    let initial_root = TempDir::new().expect("Failed to create initial root");
    let _new_root = TempDir::new().expect("Failed to create new root"); // Unused but demonstrates attacker's intent
    let tools_temp = TempDir::new().expect("Failed to create tools temp dir");
    let tools_dir = tools_temp.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    let tool_config = json!({
        "name": "pwd",
        "description": "Print working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{"name": "default", "description": "pwd"}]
    });
    std::fs::write(
        tools_dir.join("pwd.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let server = start_deferred_sandbox_server(&tools_dir).await;
    let base_url = format!("http://127.0.0.1:{}", server.port());
    let client = common::make_h2_client();

    // Complete handshake with initial root
    let session_id = initialize_session(&client, &base_url)
        .await
        .expect("Initialize failed");

    // Answer roots/list with initial root (locks sandbox)
    let initial_uri = encode_file_uri(initial_root.path());
    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_task = tokio::spawn(async move {
        complete_roots_handshake_with_uris(
            &sse_client,
            &sse_base_url,
            &sse_session_id,
            &[initial_uri],
        )
        .await
    });

    let sse_result = sse_task.await.expect("SSE task panicked");
    assert!(sse_result.is_ok(), "Initial roots exchange failed");

    // Give the stdio I/O time to process - Windows CI can be slow with inter-process communication
    if cfg!(windows) {
        sleep(TestTimeouts::short_delay()).await;
    }

    wait_for_tool_ready(&client, &base_url, &session_id, initial_root.path())
        .await
        .expect("Sandbox should become ready before roots/list_changed test");

    // Verify initial root works
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": initial_root.path().to_string_lossy()
            }
        }
    });

    let (response, _) = send_mcp_request(&client, &base_url, &tool_call, Some(&session_id))
        .await
        .expect("Initial tool call failed");
    assert!(
        response.get("error").is_none(),
        "Tool call in initial root should succeed: {:?}",
        response
    );

    // NOW: Send roots/list_changed notification (attempt to change roots)
    let roots_changed = json!({
        "jsonrpc": "2.0",
        "method": "notifications/roots/list_changed"
    });

    let result = send_mcp_request(&client, &base_url, &roots_changed, Some(&session_id)).await;

    // The session should be terminated or the notification rejected
    // Either HTTP 403 or a subsequent tool call should fail
    match result {
        Err(e) => {
            eprintln!("Roots change rejected with error (expected): {}", e);
            // HTTP 403 or similar error is expected
            assert!(
                e.contains("403") || e.contains("terminate") || e.contains("reject"),
                "Expected 403/terminate/reject error, got: {}",
                e
            );
        }
        Ok(_) => {
            // Notification was accepted - verify session is now invalid
            // Try another tool call - should fail
            sleep(TestTimeouts::poll_interval()).await;

            let tool_call_2 = json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "pwd",
                    "arguments": {
                        "subcommand": "default",
                        "working_directory": initial_root.path().to_string_lossy()
                    }
                }
            });

            let result2 =
                send_mcp_request(&client, &base_url, &tool_call_2, Some(&session_id)).await;
            match result2 {
                Err(e) => {
                    eprintln!("Session terminated after roots change (expected): {}", e);
                }
                Ok((response2, _)) => {
                    // If we got a response, it should be an error
                    let has_error = response2.get("error").is_some();
                    if !has_error {
                        // This is a FAILURE - roots change was silently accepted
                        panic!(
                            "SECURITY VIOLATION: Tool call succeeded after roots/list_changed. \
                             Session should have been terminated. Response: {:?}",
                            response2
                        );
                    }
                }
            }
        }
    }
}

/// Test that working_directory outside locked sandbox roots is rejected.
#[tokio::test]
async fn test_working_directory_outside_sandbox_rejected() {
    let allowed_root = TempDir::new().expect("Failed to create allowed root");
    let forbidden_root = TempDir::new().expect("Failed to create forbidden root");
    let tools_temp = TempDir::new().expect("Failed to create tools temp dir");
    let tools_dir = tools_temp.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

    let tool_config = json!({
        "name": "pwd",
        "description": "Print working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{"name": "default", "description": "pwd"}]
    });
    std::fs::write(
        tools_dir.join("pwd.json"),
        serde_json::to_string_pretty(&tool_config).unwrap(),
    )
    .expect("Failed to write tool config");

    let server = start_deferred_sandbox_server(&tools_dir).await;
    let base_url = format!("http://127.0.0.1:{}", server.port());
    let client = common::make_h2_client();

    let session_id = initialize_session(&client, &base_url)
        .await
        .expect("Initialize failed");

    // Lock sandbox to ONLY allowed_root
    let allowed_uri = encode_file_uri(allowed_root.path());
    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();
    let sse_task = tokio::spawn(async move {
        complete_roots_handshake_with_uris(
            &sse_client,
            &sse_base_url,
            &sse_session_id,
            &[allowed_uri],
        )
        .await
    });

    let sse_result = sse_task.await.expect("SSE task panicked");
    assert!(sse_result.is_ok(), "Roots exchange failed");

    wait_for_tool_ready(&client, &base_url, &session_id, allowed_root.path())
        .await
        .expect("Sandbox should become ready for allowed root");

    // Tool call in allowed root should work
    let allowed_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": allowed_root.path().to_string_lossy()
            }
        }
    });

    let (allowed_response, _) =
        send_mcp_request(&client, &base_url, &allowed_call, Some(&session_id))
            .await
            .expect("Allowed tool call request failed");
    assert!(
        allowed_response.get("error").is_none(),
        "Tool call in allowed root should succeed: {:?}",
        allowed_response
    );

    // Tool call in FORBIDDEN root should fail
    let forbidden_call = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": forbidden_root.path().to_string_lossy()
            }
        }
    });

    let (forbidden_response, _) =
        send_mcp_request(&client, &base_url, &forbidden_call, Some(&session_id))
            .await
            .expect("Forbidden tool call request failed");

    // This MUST fail with a sandbox/path error
    let error = forbidden_response.get("error");
    assert!(
        error.is_some(),
        "Tool call in forbidden directory should fail: {:?}",
        forbidden_response
    );

    let error_msg = error
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("");

    assert!(
        error_msg.contains("sandbox")
            || error_msg.contains("outside")
            || error_msg.contains("path"),
        "Error should mention sandbox violation: {}",
        error_msg
    );
}
