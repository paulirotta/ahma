//! Tests for handshake timeout behavior.
//!
//! These tests verify that the HTTP bridge provides actionable error messages
//! when the MCP handshake fails to complete within the timeout period.
//!
//! The handshake requires:
//! 1. Client sends POST /mcp with initialize request → server returns session ID
//! 2. Client opens GET /mcp SSE stream with session header
//! 3. Client sends POST /mcp with notifications/initialized
//! 4. Server sends roots/list request over SSE
//! 5. Client responds with roots → sandbox locks
//!
//! If any step is missing, tools/call should fail with an actionable error.

mod common;

use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};
use common::{spawn_test_server, spawn_test_server_with_timeout};
use reqwest::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{Value, json};

const MCP_SESSION_ID_HEADER: &str = "mcp-session-id";

/// Test that tools/call without completing handshake returns a clear timeout error.
/// This simulates a broken client that sends initialize but never opens SSE.
#[tokio::test]
async fn test_tools_call_without_sse_returns_handshake_timeout() {
    // Use 2 second timeout via CLI argument (not env var) for test isolation
    let server = spawn_test_server_with_timeout(Some(2))
        .await
        .expect("Failed to spawn test server");
    let client = Client::new();

    // Step 1: Initialize (creates session)
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": { "roots": {} },
            "clientInfo": { "name": "handshake_timeout_test", "version": "0.1" }
        }
    });

    let init_resp = client
        .post(format!("{}/mcp", server.base_url()))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        .json(&init_req)
        .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
        .send()
        .await
        .expect("initialize POST failed");

    assert!(init_resp.status().is_success(), "Initialize should succeed");

    let session_id = init_resp
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .expect("Should have session ID")
        .to_string();

    // Intentionally skip: SSE connection, initialized notification, roots/list response

    // Wait for handshake timeout (2s timeout + 1.5s margin for CI)
    tokio::time::sleep(TestTimeouts::scale_millis(3500)).await;

    // Try to call a tool - should get handshake timeout error
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cargo",
            "arguments": { "subcommand": "locate-project" }
        }
    });

    let resp = client
        .post(format!("{}/mcp", server.base_url()))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        .header(MCP_SESSION_ID_HEADER, &session_id)
        .json(&tool_call)
        .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
        .send()
        .await
        .expect("tools/call POST failed");

    // Should return 504 Gateway Timeout (not 409 Conflict)
    assert_eq!(
        resp.status().as_u16(),
        504,
        "Expected HTTP 504 for handshake timeout"
    );

    let body: Value = resp.json().await.expect("Response should be JSON");
    let error = body.get("error").expect("Should have error");

    // Verify error code is -32002 (handshake timeout)
    assert_eq!(
        error.get("code").and_then(|c| c.as_i64()),
        Some(-32002),
        "Expected error code -32002 for handshake timeout"
    );

    // Verify error message is actionable
    let msg = error
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or_default();
    assert!(
        msg.contains("Handshake timeout") && msg.contains("SSE"),
        "Error message should mention handshake timeout and SSE. Got: {msg}"
    );
    assert!(
        msg.contains("SSE connected: false"),
        "Error should indicate SSE not connected. Got: {msg}"
    );
}

/// Test that tools/call without initialized notification returns timeout error.
/// This simulates a client that opens SSE but forgets to send initialized.
#[tokio::test]
async fn test_tools_call_without_initialized_notification_returns_timeout() {
    // Use 2 second timeout via CLI argument (not env var) for test isolation
    let server = spawn_test_server_with_timeout(Some(2))
        .await
        .expect("Failed to spawn test server");
    let client = Client::new();

    // Step 1: Initialize
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": { "roots": {} },
            "clientInfo": { "name": "no_initialized_test", "version": "0.1" }
        }
    });

    let init_resp = client
        .post(format!("{}/mcp", server.base_url()))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        .json(&init_req)
        .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
        .send()
        .await
        .expect("initialize POST failed");

    let session_id = init_resp
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .expect("Should have session ID")
        .to_string();

    // Step 2: Open SSE stream (but don't send initialized)
    let _sse_resp = client
        .get(format!("{}/mcp", server.base_url()))
        .header(ACCEPT, "text/event-stream")
        .header(MCP_SESSION_ID_HEADER, &session_id)
        .send()
        .await
        .expect("SSE GET failed");

    // Intentionally skip: initialized notification, roots/list response

    // Wait for handshake timeout (2s timeout + 1.5s margin for CI)
    tokio::time::sleep(TestTimeouts::scale_millis(3500)).await;

    // Try to call a tool
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cargo",
            "arguments": { "subcommand": "locate-project" }
        }
    });

    let resp = client
        .post(format!("{}/mcp", server.base_url()))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        .header(MCP_SESSION_ID_HEADER, &session_id)
        .json(&tool_call)
        .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
        .send()
        .await
        .expect("tools/call POST failed");

    assert_eq!(resp.status().as_u16(), 504);

    let body: Value = resp.json().await.expect("Response should be JSON");
    let error = body.get("error").expect("Should have error");
    let msg = error
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or_default();

    // SSE should be connected, but MCP initialized should be false
    assert!(
        msg.contains("SSE connected: true") && msg.contains("MCP initialized: false"),
        "Error should show SSE connected but MCP not initialized. Got: {msg}"
    );
}

/// Test that a properly completed handshake allows tools/call to succeed.
/// This validates the full VS Code-style handshake flow works correctly.
#[tokio::test]
async fn test_proper_vscode_handshake_allows_tool_calls() {
    use common::McpTestClient;
    use tempfile::TempDir;

    let server = spawn_test_server()
        .await
        .expect("Failed to spawn test server");

    // Create a temp directory for the test
    let temp_root = TempDir::new().expect("Failed to create temp root");
    let root_path = temp_root.path().to_path_buf();

    // Use the test client which implements proper handshake
    let mut client = McpTestClient::for_server(&server);

    // This performs the full handshake: initialize → SSE → initialized → roots/list response
    client
        .initialize_with_roots("vscode-handshake-test", std::slice::from_ref(&root_path))
        .await
        .expect("Handshake should complete successfully");

    // Use sandboxed_shell with pwd - this tool is always available (built-in)
    // This tests that tool calls work after proper handshake
    let result = client
        .call_tool(
            "sandboxed_shell",
            json!({
                "command": "echo ok",
                "working_directory": root_path.to_string_lossy()
            }),
        )
        .await;

    assert!(
        result.success,
        "Tool call should succeed after proper handshake. Error: {:?}",
        result.error
    );

    // Verify output contains "ok"
    let output = result.output.unwrap_or_default();
    assert!(
        output.contains("ok"),
        "Output should contain 'ok'. Got: {output}"
    );
}

/// Test that tools/call before handshake completion returns 409 (not timeout).
/// This is the expected behavior during the handshake window.
#[tokio::test]
async fn test_tools_call_during_handshake_returns_conflict() {
    // Use a long timeout so we hit 409, not 504
    let server = spawn_test_server_with_timeout(Some(60))
        .await
        .expect("Failed to spawn test server");
    let client = Client::new();

    // Initialize only
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": { "roots": {} },
            "clientInfo": { "name": "conflict_test", "version": "0.1" }
        }
    });

    let init_resp = client
        .post(format!("{}/mcp", server.base_url()))
        .header(CONTENT_TYPE, "application/json")
        .json(&init_req)
        .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
        .send()
        .await
        .expect("initialize POST failed");

    let session_id = init_resp
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .expect("Should have session ID")
        .to_string();

    // Immediately try tools/call (before timeout expires)
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cargo",
            "arguments": { "subcommand": "locate-project" }
        }
    });

    let resp = client
        .post(format!("{}/mcp", server.base_url()))
        .header(CONTENT_TYPE, "application/json")
        .header(MCP_SESSION_ID_HEADER, &session_id)
        .json(&tool_call)
        .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
        .send()
        .await
        .expect("tools/call POST failed");

    // Should return 409 Conflict (sandbox initializing, not timed out yet)
    assert_eq!(
        resp.status().as_u16(),
        409,
        "Expected HTTP 409 for sandbox initializing"
    );

    let body: Value = resp.json().await.expect("Response should be JSON");
    let error = body.get("error").expect("Should have error");

    // Verify error code is -32001 (sandbox initializing)
    assert_eq!(
        error.get("code").and_then(|c| c.as_i64()),
        Some(-32001),
        "Expected error code -32001 for sandbox initializing"
    );
}

/// Test that handshake timeout is configured per-server via CLI argument.
/// This verifies that the timeout is properly isolated per test (not using env vars).
#[tokio::test]
async fn test_handshake_timeout_is_per_server_via_cli() {
    // Spawn two servers with different timeouts
    // Server 1: 2 second timeout
    let server1 = spawn_test_server_with_timeout(Some(2))
        .await
        .expect("Failed to spawn server 1");

    // Server 2: 60 second timeout (won't timeout in this test)
    let server2 = spawn_test_server_with_timeout(Some(60))
        .await
        .expect("Failed to spawn server 2");

    let client = Client::new();

    // Initialize both servers
    let init_req = |name: &str| {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "roots": {} },
                "clientInfo": { "name": name, "version": "0.1" }
            }
        })
    };

    let resp1 = client
        .post(format!("{}/mcp", server1.base_url()))
        .header(CONTENT_TYPE, "application/json")
        .json(&init_req("server1-client"))
        .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
        .send()
        .await
        .expect("Initialize server1 failed");

    let session_id1 = resp1
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .expect("Should have session ID")
        .to_string();

    let resp2 = client
        .post(format!("{}/mcp", server2.base_url()))
        .header(CONTENT_TYPE, "application/json")
        .json(&init_req("server2-client"))
        .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
        .send()
        .await
        .expect("Initialize server2 failed");

    let session_id2 = resp2
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .expect("Should have session ID")
        .to_string();

    // Wait for server1's timeout to expire (2s + margin)
    tokio::time::sleep(TestTimeouts::scale_millis(3500)).await;

    // Server1 should return 504 (timeout)
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cargo",
            "arguments": { "subcommand": "locate-project" }
        }
    });

    let resp1_tool = client
        .post(format!("{}/mcp", server1.base_url()))
        .header(CONTENT_TYPE, "application/json")
        .header(MCP_SESSION_ID_HEADER, &session_id1)
        .json(&tool_call)
        .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
        .send()
        .await
        .expect("tools/call server1 failed");

    assert_eq!(
        resp1_tool.status().as_u16(),
        504,
        "Server1 should timeout (504)"
    );

    // Server2 should still return 409 (still in handshake, not timed out)
    let resp2_tool = client
        .post(format!("{}/mcp", server2.base_url()))
        .header(CONTENT_TYPE, "application/json")
        .header(MCP_SESSION_ID_HEADER, &session_id2)
        .json(&tool_call)
        .timeout(TestTimeouts::get(TimeoutCategory::HttpRequest))
        .send()
        .await
        .expect("tools/call server2 failed");

    assert_eq!(
        resp2_tool.status().as_u16(),
        409,
        "Server2 should still be in handshake (409), not timed out"
    );
}
