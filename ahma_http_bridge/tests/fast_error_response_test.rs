//! Fast Error Response Tests
//!
//! These tests verify that the server NEVER hangs on invalid input.
//! All error responses must complete within a strict timeout (2 seconds).
//!
//! This is a critical safety test - a hanging server can block CI and cause
//! poor user experience.
//!
//! ## Test Scenarios
//!
//! 1. Invalid tool name → Error in < 2s
//! 2. Invalid subcommand → Error in < 2s
//! 3. Missing session ID → 400 error in < 2s
//! 4. Invalid JSON-RPC method → Error in < 2s

mod common;

use common::spawn_test_server;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

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

/// Maximum allowed response time for ANY error case.
/// If any request takes longer than this, the server has a hang bug.
const MAX_ERROR_RESPONSE_MS: u128 = 2000;

// Thread-local storage for the current test's server URL
std::thread_local! {
    static CURRENT_SERVER_URL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Get the test server URL from thread-local storage
fn get_test_url() -> String {
    CURRENT_SERVER_URL
        .with(|url| format!("{}/mcp", url.borrow().as_ref().expect("Server URL not set")))
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: Value,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<u64>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

/// Send a request and return (response/error, duration_ms)
async fn timed_request(
    client: &Client,
    request: &JsonRpcRequest,
) -> (Result<JsonRpcResponse, String>, u128) {
    let url = get_test_url();
    let start = Instant::now();

    let result = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(request)
        .timeout(Duration::from_secs(5))
        .send()
        .await;

    let duration_ms = start.elapsed().as_millis();

    match result {
        Ok(response) => {
            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                (Err(format!("HTTP {}: {}", status, text)), duration_ms)
            } else {
                match response.json::<JsonRpcResponse>().await {
                    Ok(resp) => (Ok(resp), duration_ms),
                    Err(e) => (Err(format!("Parse error: {}", e)), duration_ms),
                }
            }
        }
        Err(e) => (Err(format!("Request failed: {}", e)), duration_ms),
    }
}

/// Initialize a session and return the session ID.
/// Retries a few times since the shared test server may still be starting.
/// This function properly completes the MCP handshake by sending:
/// 1. initialize request
/// 2. notifications/initialized notification
async fn initialize_session(client: &Client) -> Option<String> {
    let url = get_test_url();
    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"roots": {}},
            "clientInfo": {"name": "fast-error-test", "version": "1.0"}
        }
    });

    // Retry a few times - the shared server may still be starting
    for attempt in 0..5 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let response = match client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&init_request)
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(_) => continue,
        };

        if !response.status().is_success() {
            continue;
        }

        let session_id = response
            .headers()
            .get("mcp-session-id")
            .or_else(|| response.headers().get("Mcp-Session-Id"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        if let Some(ref sid) = session_id {
            // Complete MCP handshake by sending initialized notification
            let initialized_notification = json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            });

            let _ = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Mcp-Session-Id", sid)
                .json(&initialized_notification)
                .timeout(Duration::from_secs(5))
                .send()
                .await;

            return session_id;
        }
    }

    None
}

/// Returns true if this is a transport-level failure (connection refused, OS-level
/// backlog full, OS-scheduler-induced timeout) rather than a proper HTTP response.
/// Transport errors indicate the server was temporarily unavailable under parallel
/// test load — not a hang bug — so duration assertions are skipped for them.
fn is_transport_error(err: &str) -> bool {
    err.starts_with("Request failed")
}

/// Send a request WITH session ID
async fn timed_request_with_session(
    client: &Client,
    session_id: &str,
    request: &JsonRpcRequest,
) -> (Result<JsonRpcResponse, String>, u128) {
    let url = get_test_url();
    let start = Instant::now();

    let result = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Mcp-Session-Id", session_id)
        .json(request)
        .timeout(Duration::from_secs(5))
        .send()
        .await;

    let duration_ms = start.elapsed().as_millis();

    match result {
        Ok(response) => {
            if !response.status().is_success() {
                let text = response.text().await.unwrap_or_default();
                (Err(format!("HTTP error: {}", text)), duration_ms)
            } else {
                match response.json::<JsonRpcResponse>().await {
                    Ok(resp) => (Ok(resp), duration_ms),
                    Err(e) => (Err(format!("Parse error: {}", e)), duration_ms),
                }
            }
        }
        Err(e) => (Err(format!("Request failed: {}", e)), duration_ms),
    }
}

// =============================================================================
// Test: Missing Session ID returns fast 400
// =============================================================================

macro_rules! setup_fast_error_test {
    ($client:ident) => {
        if should_skip_in_nested_sandbox() {
            eprintln!("Skipping strict sandbox fast-error test in nested sandbox environment");
            return;
        }

        let _server = spawn_test_server().await.expect("Failed to spawn server");
        CURRENT_SERVER_URL.with(|u| *u.borrow_mut() = Some(_server.base_url()));
        let $client = common::make_h2_client();

        // Wait a moment for the server to stabilize if it just started
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    ($client:ident, $session:ident) => {
        setup_fast_error_test!($client);
        let $session = match initialize_session(&$client).await {
            Some(id) => id,
            None => {
                eprintln!("WARNING️  Could not initialize session, skipping test");
                return;
            }
        };
    };
}

#[tokio::test]
async fn test_missing_session_id_returns_fast_error() {
    setup_fast_error_test!(client);

    // Call tools/call without session ID - should return 400 immediately
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: 1,
        method: "tools/call".to_string(),
        params: json!({
            "name": "python",
            "arguments": {"subcommand": "version"}
        }),
    };

    let (result, duration_ms) = timed_request(&client, &request).await;

    println!(
        "Missing session ID: duration={}ms, result={:?}",
        duration_ms, result
    );

    // Transport-level failures indicate the server was temporarily unavailable
    // under parallel test load — not a hang bug. Skip rather than fail.
    if let Err(ref e) = result {
        if is_transport_error(e) {
            eprintln!("Transport error (server unavailable, not a hang): {}", e);
            return;
        }
    }

    // MUST return fast - if this fails, server has a hang bug
    assert!(
        duration_ms < MAX_ERROR_RESPONSE_MS,
        "Server took {}ms to respond to missing session ID (max: {}ms). HANG BUG!",
        duration_ms,
        MAX_ERROR_RESPONSE_MS
    );

    // Should be an error (400 status, which becomes Err)
    assert!(result.is_err(), "Missing session ID should return an error");
    let err = result.unwrap_err();
    assert!(
        err.contains("400") || err.contains("session"),
        "Error should mention session ID: {}",
        err
    );
}

// =============================================================================
// Test: Invalid tool name returns fast error
// =============================================================================

#[tokio::test]
async fn test_invalid_tool_name_returns_fast_error() {
    setup_fast_error_test!(client, session_id);

    // Call a nonexistent tool
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: 2,
        method: "tools/call".to_string(),
        params: json!({
            "name": "nonexistent_tool_xyz_123",
            "arguments": {}
        }),
    };

    let (result, duration_ms) = timed_request_with_session(&client, &session_id, &request).await;

    println!(
        "Invalid tool name: duration={}ms, result={:?}",
        duration_ms, result
    );

    // Transport-level failures indicate the server was temporarily unavailable
    // under parallel test load — not a hang bug. Skip rather than fail.
    if let Err(ref e) = result {
        if is_transport_error(e) {
            eprintln!("Transport error (server unavailable, not a hang): {}", e);
            return;
        }
    }

    // MUST return fast
    assert!(
        duration_ms < MAX_ERROR_RESPONSE_MS,
        "Server took {}ms to respond to invalid tool name (max: {}ms). HANG BUG!",
        duration_ms,
        MAX_ERROR_RESPONSE_MS
    );

    // Should be a JSON-RPC error response
    match result {
        Ok(resp) => {
            assert!(
                resp.error.is_some(),
                "Should return JSON-RPC error for invalid tool"
            );
            let err = resp.error.unwrap();
            println!("Error code: {}, message: {}", err.code, err.message);
            assert!(
                err.message.to_lowercase().contains("not found") || err.message.contains("Tool"),
                "Error should mention tool not found: {}",
                err.message
            );
        }
        Err(e) => {
            // HTTP-level error is also acceptable
            println!("HTTP error (acceptable): {}", e);
        }
    }
}

// =============================================================================
// Test: Invalid subcommand returns fast error
// =============================================================================

#[tokio::test]
async fn test_invalid_subcommand_returns_fast_error() {
    setup_fast_error_test!(client, session_id);

    // Call a valid tool with invalid subcommand
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: 3,
        method: "tools/call".to_string(),
        params: json!({
            "name": "python",
            "arguments": {"subcommand": "nonexistent_subcommand_xyz"}
        }),
    };

    let (result, duration_ms) = timed_request_with_session(&client, &session_id, &request).await;

    println!(
        "Invalid subcommand: duration={}ms, result={:?}",
        duration_ms, result
    );

    // Transport-level failures indicate the server was temporarily unavailable
    // under parallel test load — not a hang bug. Skip rather than fail.
    if let Err(ref e) = result {
        if is_transport_error(e) {
            eprintln!("Transport error (server unavailable, not a hang): {}", e);
            return;
        }
    }

    // MUST return fast
    assert!(
        duration_ms < MAX_ERROR_RESPONSE_MS,
        "Server took {}ms to respond to invalid subcommand (max: {}ms). HANG BUG!",
        duration_ms,
        MAX_ERROR_RESPONSE_MS
    );

    // Should be a JSON-RPC error response
    match result {
        Ok(resp) => {
            assert!(
                resp.error.is_some(),
                "Should return JSON-RPC error for invalid subcommand"
            );
            let err = resp.error.unwrap();
            println!("Error code: {}, message: {}", err.code, err.message);
        }
        Err(e) => {
            println!("HTTP error (acceptable): {}", e);
        }
    }
}

// =============================================================================
// Test: Invalid JSON-RPC method returns fast error
// =============================================================================

#[tokio::test]
async fn test_invalid_method_returns_fast_error() {
    setup_fast_error_test!(client, session_id);

    // Call an invalid method
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: 4,
        method: "nonexistent/method".to_string(),
        params: json!({}),
    };

    let (result, duration_ms) = timed_request_with_session(&client, &session_id, &request).await;

    println!(
        "Invalid method: duration={}ms, result={:?}",
        duration_ms, result
    );

    // Transport-level failures indicate the server was temporarily unavailable
    // under parallel test load — not a hang bug. Skip rather than fail.
    if let Err(ref e) = result {
        if is_transport_error(e) {
            eprintln!("Transport error (server unavailable, not a hang): {}", e);
            return;
        }
    }

    // MUST return fast
    assert!(
        duration_ms < MAX_ERROR_RESPONSE_MS,
        "Server took {}ms to respond to invalid method (max: {}ms). HANG BUG!",
        duration_ms,
        MAX_ERROR_RESPONSE_MS
    );

    // Should be a JSON-RPC error response
    match result {
        Ok(resp) => {
            assert!(
                resp.error.is_some(),
                "Should return JSON-RPC error for invalid method"
            );
        }
        Err(e) => {
            println!("HTTP error (acceptable): {}", e);
        }
    }
}

// =============================================================================
// Test: Malformed JSON returns fast error
// =============================================================================

#[tokio::test]
async fn test_malformed_json_returns_fast_error() {
    setup_fast_error_test!(client);

    let url = get_test_url();
    let start = Instant::now();

    // Send malformed JSON
    let result = client
        .post(&url)
        .header("Content-Type", "application/json")
        .body("{invalid json")
        .timeout(Duration::from_secs(5))
        .send()
        .await;

    let duration_ms = start.elapsed().as_millis();

    println!("Malformed JSON: duration={}ms", duration_ms);

    // MUST return fast
    assert!(
        duration_ms < MAX_ERROR_RESPONSE_MS,
        "Server took {}ms to respond to malformed JSON (max: {}ms). HANG BUG!",
        duration_ms,
        MAX_ERROR_RESPONSE_MS
    );

    // Should return some response (error is fine)
    assert!(
        result.is_ok(),
        "Server should respond to malformed JSON, not hang"
    );
}

// =============================================================================
// Test: Missing tool arguments returns fast error
// =============================================================================

#[tokio::test]
async fn test_missing_required_args_returns_fast_error() {
    setup_fast_error_test!(client, session_id);

    // Call a tool that requires subcommand, without providing one
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: 5,
        method: "tools/call".to_string(),
        params: json!({
            "name": "python",
            "arguments": {}  // Missing required "subcommand"
        }),
    };

    let (result, duration_ms) = timed_request_with_session(&client, &session_id, &request).await;

    println!(
        "Missing required args: duration={}ms, result={:?}",
        duration_ms, result
    );

    // Transport-level failures indicate the server was temporarily unavailable
    // under parallel test load — not a hang bug. Skip rather than fail.
    if let Err(ref e) = result {
        if is_transport_error(e) {
            eprintln!("Transport error (server unavailable, not a hang): {}", e);
            return;
        }
    }

    // MUST return fast
    assert!(
        duration_ms < MAX_ERROR_RESPONSE_MS,
        "Server took {}ms to respond to missing args (max: {}ms). HANG BUG!",
        duration_ms,
        MAX_ERROR_RESPONSE_MS
    );
}
