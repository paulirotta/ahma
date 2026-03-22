//! HTTP DELETE Session Termination Tests (R8.4.7)
//!
//! These tests verify that HTTP DELETE with `Mcp-Session-Id` header properly
//! terminates sessions and their subprocesses.
//!
//! Per MCP specification (R8.4.7): HTTP DELETE with `Mcp-Session-Id` terminates
//! session and subprocess.

use reqwest::Client;
use serde_json::{Value, json};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

/// RAII guard for a raw `Child` server process.
/// Kills the child on drop to prevent leaky tests when assertions panic.
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

/// Find an available port for testing
fn find_available_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("Failed to bind to any port")
        .local_addr()
        .expect("Failed to get local address")
        .port()
}

/// Build the ahma_mcp binary if needed and return the path
fn get_ahma_mcp_binary() -> PathBuf {
    ahma_mcp::test_utils::cli::build_binary_cached("ahma_mcp", "ahma-mcp")
}

/// Start the HTTP bridge server and return the process
async fn start_http_bridge(
    port: u16,
    tools_dir: &std::path::Path,
    sandbox_scope: &std::path::Path,
) -> std::process::Child {
    let binary = get_ahma_mcp_binary();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get workspace dir")
        .to_path_buf();

    // Detect nested sandbox (ahma_mcp_sandboxed_shell / VS Code / Docker) and disable
    // OS-level sandboxing so the binary can start; app-level path checks still apply.
    #[cfg(target_os = "macos")]
    let no_sandbox = ahma_mcp::sandbox::test_sandbox_exec_available().is_err();
    #[cfg(not(target_os = "macos"))]
    let no_sandbox = cfg!(windows);

    let mut cmd = Command::new(&binary);
    cmd.args(["serve", "http", "--port", &port.to_string()])
        .current_dir(&workspace)
        .env("AHMA_SYNC", "1")
        .env("AHMA_TOOLS_DIR", &*tools_dir.to_string_lossy())
        .env("AHMA_SANDBOX_SCOPE", &*sandbox_scope.to_string_lossy())
        .env("AHMA_LOG_TARGET", "stderr")
        .env_remove("NEXTEST")
        .env_remove("NEXTEST_EXECUTION_MODE")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUST_TEST_THREADS");
    if no_sandbox {
        cmd.env("AHMA_DISABLE_SANDBOX", "1");
    }
    let child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start HTTP bridge");

    // Wait for server to be ready
    let client = Client::builder()
        .http2_prior_knowledge()
        .build()
        .expect("Failed to build HTTP/2 client");
    let health_url = format!("http://127.0.0.1:{}/health", port);

    for _ in 0..30 {
        sleep(Duration::from_millis(200)).await;
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
        .timeout(Duration::from_secs(60));

    if let Some(id) = session_id {
        req = req.header("Mcp-Session-Id", id);
    }

    let response = req
        .json(request)
        .send()
        .await
        .map_err(|e| format!("Request failed: {:?}", e))?;

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

/// Test that DELETE with valid session ID returns 204 and terminates the session (R8.4.7)
#[tokio::test]
async fn test_delete_session_terminates_subprocess() {
    if should_skip_in_nested_sandbox() {
        eprintln!("Skipping strict sandbox session deletion test in nested sandbox environment");
        return;
    }

    let port = find_available_port();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let base_url = format!("http://127.0.0.1:{}", port);

    let workspace_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get workspace dir")
        .to_path_buf();
    let tools_dir = workspace_dir.join(".ahma");

    let child = start_http_bridge(port, &tools_dir, temp_dir.path()).await;
    let _guard = ServerGuard::new(child);
    let client = Client::builder()
        .http2_prior_knowledge()
        .build()
        .expect("Failed to build HTTP/2 client");

    // Step 1: Initialize a session
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

    let (_, session_id) = send_mcp_request(&client, &base_url, &init_request, None)
        .await
        .expect("Initialize should succeed");

    let session_id = session_id.expect("Should receive session ID from initialize");
    eprintln!("Got session ID: {}", session_id);

    // Step 2: Send DELETE request to terminate the session
    let delete_url = format!("{}/mcp", base_url);
    let delete_response = client
        .delete(&delete_url)
        .header("Mcp-Session-Id", &session_id)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .expect("DELETE request should succeed");

    eprintln!("DELETE response status: {}", delete_response.status());

    // Step 3: Assert 204 No Content
    assert_eq!(
        delete_response.status().as_u16(),
        204,
        "DELETE should return 204 No Content"
    );

    // Step 4: Verify subsequent requests with same session ID return 404
    sleep(Duration::from_millis(100)).await;

    let post_response = client
        .post(format!("{}/mcp", base_url))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Mcp-Session-Id", &session_id)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .expect("POST request should complete");

    eprintln!("POST after DELETE status: {}", post_response.status());

    // Session should no longer exist - expect 403 Forbidden (per R8D.13: security response for
    // non-existent or terminated sessions) or 404 Not Found
    let status = post_response.status().as_u16();
    assert!(
        status == 403 || status == 404,
        "Requests to deleted session should return 403 Forbidden or 404 Not Found, got {}",
        status
    );
}

/// Test that DELETE without session ID returns 400 Bad Request
#[tokio::test]
async fn test_delete_without_session_id_returns_400() {
    if should_skip_in_nested_sandbox() {
        eprintln!("Skipping strict sandbox session deletion test in nested sandbox environment");
        return;
    }

    let port = find_available_port();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let base_url = format!("http://127.0.0.1:{}", port);

    let workspace_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get workspace dir")
        .to_path_buf();
    let tools_dir = workspace_dir.join(".ahma");

    let child = start_http_bridge(port, &tools_dir, temp_dir.path()).await;
    let _guard = ServerGuard::new(child);
    let client = Client::builder()
        .http2_prior_knowledge()
        .build()
        .expect("Failed to build HTTP/2 client");

    // Send DELETE without session ID
    let delete_url = format!("{}/mcp", base_url);
    let delete_response = client
        .delete(&delete_url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .expect("DELETE request should complete");

    eprintln!("DELETE response status: {}", delete_response.status());

    // Should return 400 Bad Request
    assert_eq!(
        delete_response.status().as_u16(),
        400,
        "DELETE without session ID should return 400 Bad Request"
    );
}

/// Test that DELETE with non-existent session ID returns 404 Not Found
#[tokio::test]
async fn test_delete_nonexistent_session_returns_404() {
    if should_skip_in_nested_sandbox() {
        eprintln!("Skipping strict sandbox session deletion test in nested sandbox environment");
        return;
    }

    let port = find_available_port();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let base_url = format!("http://127.0.0.1:{}", port);

    let workspace_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get workspace dir")
        .to_path_buf();
    let tools_dir = workspace_dir.join(".ahma");

    let child = start_http_bridge(port, &tools_dir, temp_dir.path()).await;
    let _guard = ServerGuard::new(child);
    let client = Client::builder()
        .http2_prior_knowledge()
        .build()
        .expect("Failed to build HTTP/2 client");

    // Send DELETE with a fake session ID
    let delete_url = format!("{}/mcp", base_url);
    let delete_response = client
        .delete(&delete_url)
        .header("Mcp-Session-Id", "non-existent-session-id-12345")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .expect("DELETE request should complete");

    eprintln!("DELETE response status: {}", delete_response.status());

    // Should return 404 Not Found
    assert_eq!(
        delete_response.status().as_u16(),
        404,
        "DELETE with non-existent session should return 404 Not Found"
    );
}
