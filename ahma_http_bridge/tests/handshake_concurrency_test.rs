//! Handshake Concurrency and Race Condition Tests
//!
//! This module tests the robustness of the initial connection handshake,
//! specifically focusing on race conditions, timing issues, and ensuring
//! deterministic behavior when receiving roots from various clients.
//!
//! Key scenarios:
//! 1. Tool calls attempted *before* roots are received.
//! 2. Concurrent handshake and tool execution.
//! 3. Slow client responses to roots/list requests.
//! 4. Rapid connect/disconnect cycles.

mod common;

use common::{McpTestClient, SandboxTestEnv, ServerGuard};
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

#[cfg(target_os = "macos")]
fn should_skip_in_nested_sandbox() -> bool {
    matches!(
        ahma_mcp::sandbox::test_sandbox_exec_available(),
        Err(ahma_mcp::sandbox::SandboxError::NestedSandboxDetected)
    )
}

#[cfg(not(target_os = "macos"))]
fn should_skip_in_nested_sandbox() -> bool {
    false
}

// =============================================================================
// Test Infrastructure (Duplicated from sandbox_roots_handshake_test.rs)
// =============================================================================

fn find_available_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("Failed to bind to any port")
        .local_addr()
        .expect("Failed to get local address")
        .port()
}

fn get_ahma_mcp_binary() -> PathBuf {
    ahma_mcp::test_utils::cli::build_binary_cached("ahma_mcp", "ahma-mcp")
}

async fn start_deferred_sandbox_server(
    port: u16,
    tools_dir: &std::path::Path,
) -> std::process::Child {
    let binary = get_ahma_mcp_binary();

    let mut cmd = Command::new(&binary);
    cmd.args([
        "--mode",
        "http",
        "--http-port",
        &port.to_string(),
        "--sync",
        "--tools-dir",
        &tools_dir.to_string_lossy(),
        "--defer-sandbox",
        "--log-to-stderr",
    ]);

    SandboxTestEnv::configure(&mut cmd);
    SandboxTestEnv::apply_nested_sandbox_override(&mut cmd);

    let child = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to start HTTP bridge");

    let client = Client::new();
    let health_url = format!("http://127.0.0.1:{}/health", port);

    for _ in 0..50 {
        sleep(Duration::from_millis(300)).await;
        if let Ok(resp) = client.get(&health_url).send().await
            && resp.status().is_success()
        {
            return child;
        }
    }

    let mut child = child;
    let _ = child.kill();
    let _ = child.wait();
    panic!("HTTP bridge failed to start within timeout");
}

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
        .timeout(Duration::from_secs(60));

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

async fn send_mcp_request_raw(
    client: &Client,
    base_url: &str,
    request: &Value,
    session_id: Option<&str>,
) -> Result<(u16, Value), String> {
    let url = format!("{}/mcp", base_url);
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(60));

    if let Some(id) = session_id {
        req = req.header("Mcp-Session-Id", id);
    }

    let response = req
        .json(request)
        .send()
        .await
        .map_err(|e| format!("Request failed: {:?}", e))?;

    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let body = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }))
    };

    Ok((status, body))
}

// =============================================================================
// Test: Tool Call Before Roots Handshake
// =============================================================================

/// Test that tool calls are rejected if attempted before the roots handshake completes.
/// This ensures that no operations can bypass the sandbox check by racing the handshake.
#[tokio::test]
async fn test_tool_call_before_roots_handshake() {
    if should_skip_in_nested_sandbox() {
        eprintln!("Skipping strict sandbox handshake test in nested sandbox environment");
        return;
    }

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

    let port = find_available_port();
    let _server = ServerGuard::new(start_deferred_sandbox_server(port, &tools_dir).await, port);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = Client::new();
    let mut mcp_client = McpTestClient::with_url(&base_url);

    // 1. Initialize + initialized (without completing roots handshake yet)
    mcp_client
        .initialize_with_name("test-client")
        .await
        .expect("Initialize failed");
    let session_id = mcp_client.session_id().expect("No session ID").to_string();

    // 3. IMMEDIATELY try to call a tool (before answering roots/list)
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {"subcommand": "default"}
        }
    });

    let (status, body) = send_mcp_request_raw(&client, &base_url, &tool_call, Some(&session_id))
        .await
        .expect("Pre-handshake tools/call request failed");

    // 4. Verify strict gating behavior
    assert_eq!(
        status, 409,
        "Expected HTTP 409 during handshake; got status {} body {:?}",
        status, body
    );
    let code = body
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_i64())
        .unwrap_or_default();
    assert_eq!(
        code, -32001,
        "Expected JSON-RPC code -32001 during handshake; got body {:?}",
        body
    );

    // 5. Now complete the roots handshake
    let roots = vec![temp_dir.path().to_path_buf()];
    mcp_client
        .complete_roots_handshake_after_initialized(&roots)
        .await
        .expect("Roots handshake failed");

    // 6. Verify tool call now works
    let result = mcp_client
        .call_tool(
            "pwd",
            json!({
                "subcommand": "default",
                "working_directory": temp_dir.path().to_string_lossy()
            }),
        )
        .await;

    assert!(
        result.success,
        "Tool call should succeed after handshake: {:?}",
        result.error
    );
}

// =============================================================================
// Test: Slow Client Handshake
// =============================================================================

/// Test that the server handles a slow client gracefully.
/// The server should wait for the roots response before allowing tool calls.
#[tokio::test]
async fn test_slow_client_handshake() {
    if should_skip_in_nested_sandbox() {
        eprintln!("Skipping strict sandbox handshake test in nested sandbox environment");
        return;
    }

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

    let port = find_available_port();
    let _server = ServerGuard::new(start_deferred_sandbox_server(port, &tools_dir).await, port);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = Client::new();
    let mut mcp_client = McpTestClient::with_url(&base_url);

    mcp_client
        .initialize_with_name("slow-client")
        .await
        .expect("Initialize failed");
    let session_id = mcp_client.session_id().expect("No session ID").to_string();

    // Start SSE connection but DELAY sending the roots response
    let root_uri = common::encode_file_uri(temp_dir.path());
    let sse_client = client.clone();
    let sse_base_url = base_url.clone();
    let sse_session_id = session_id.clone();

    let sse_task = tokio::spawn(async move {
        // Connect to SSE to receive the request
        let url = format!("{}/mcp", sse_base_url);
        let resp = sse_client
            .get(&url)
            .header("Accept", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Mcp-Session-Id", sse_session_id.clone())
            .send()
            .await
            .expect("SSE connection failed");

        let mut stream = resp.bytes_stream();

        // Wait for roots/list request
        let mut request_id = None;
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.expect("SSE read error");
            let text = String::from_utf8_lossy(&bytes);
            if text.contains("roots/list") {
                // Extract ID (simplified parsing for test)
                if let Some(start) = text.find("\"id\":") {
                    let rest = &text[start + 5..];
                    if let Some(end) = rest.find(',') {
                        let id_str = &rest[..end].trim();
                        // Handle both number and string IDs
                        if let Ok(id_num) = id_str.parse::<i64>() {
                            request_id = Some(json!(id_num));
                        } else {
                            request_id = Some(json!(id_str.trim_matches('"')));
                        }
                        break;
                    }
                }
            }
        }

        let id = request_id.expect("Did not receive roots/list request");

        // SIMULATE DELAY (e.g. user prompt)
        sleep(Duration::from_secs(2)).await;

        // Send response
        let roots_json = vec![json!({"uri": root_uri, "name": "root"})];
        let response = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"roots": roots_json}
        });

        let _ =
            send_mcp_request(&sse_client, &sse_base_url, &response, Some(&sse_session_id)).await;

        // Wait for notifications/sandbox/configured so the sandbox is truly Active
        // before the task returns.  Without this, the retry tool call races with
        // the subprocess confirming sandbox activation and incorrectly gets 409.
        let configured_secs = if std::env::var_os("LLVM_PROFILE_FILE").is_some()
            || std::env::var_os("CARGO_LLVM_COV").is_some()
        {
            120
        } else if cfg!(windows) {
            45
        } else {
            15
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(configured_secs);
        while let Ok(Some(chunk)) = tokio::time::timeout_at(deadline, stream.next()).await {
            if let Ok(bytes) = chunk {
                let text = String::from_utf8_lossy(&bytes);
                if text.contains("notifications/sandbox/configured") {
                    break;
                }
            }
        }
    });

    // Try to call tool during the delay - should fail with strict gating
    sleep(Duration::from_secs(1)).await;
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {"subcommand": "default"}
        }
    });

    let (status, body) = send_mcp_request_raw(&client, &base_url, &tool_call, Some(&session_id))
        .await
        .expect("tools/call during slow handshake request failed");
    assert_eq!(
        status, 409,
        "Expected HTTP 409 while handshake is pending; got status {} body {:?}",
        status, body
    );
    let code = body
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_i64())
        .unwrap_or_default();
    assert_eq!(
        code, -32001,
        "Expected JSON-RPC code -32001 while handshake pending; got body {:?}",
        body
    );

    // Wait for handshake to complete (includes sandbox/configured)
    sse_task.await.expect("SSE task failed");

    // Now it should work
    let tool_call_retry = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "pwd",
            "arguments": {
                "subcommand": "default",
                "working_directory": temp_dir.path().to_string_lossy()
            }
        }
    });

    let (response, _) = send_mcp_request(&client, &base_url, &tool_call_retry, Some(&session_id))
        .await
        .expect("Tool call retry failed");

    assert!(
        response.get("error").is_none(),
        "Tool call should succeed after slow handshake: {:?}",
        response
    );
}

// =============================================================================
// Test: Rapid Connect/Disconnect
// =============================================================================

/// Test that rapid connect/disconnect cycles don't leave the server in a bad state.
/// A failed handshake should not prevent subsequent connections from working.
#[tokio::test]
async fn test_rapid_connect_disconnect() {
    if should_skip_in_nested_sandbox() {
        eprintln!("Skipping strict sandbox handshake test in nested sandbox environment");
        return;
    }

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

    let port = find_available_port();
    let _server = ServerGuard::new(start_deferred_sandbox_server(port, &tools_dir).await, port);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = Client::new();

    // Attempt 1: Connect, Initialize, then Abandon
    {
        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"roots": {}},
                "clientInfo": {"name": "abandoning-client", "version": "1.0.0"}
            }
        });

        let _ = send_mcp_request(&client, &base_url, &init_request, None).await;
        // Abandon session without completing handshake
    }

    // Attempt 2: Connect immediately
    {
        // Complete handshake for second client
        let mut second_client = McpTestClient::with_url(&base_url);
        second_client
            .initialize_with_name("second-client")
            .await
            .expect("Second initialize failed");
        second_client
            .complete_roots_handshake_after_initialized(&[temp_dir.path().to_path_buf()])
            .await
            .expect("Roots handshake failed for second client");

        // Verify tool call works
        let result = second_client
            .call_tool(
                "pwd",
                json!({
                    "subcommand": "default",
                    "working_directory": temp_dir.path().to_string_lossy()
                }),
            )
            .await;

        assert!(
            result.success,
            "Tool call should succeed for second client: {:?}",
            result.error
        );
    }
}
