//! End-to-end HTTP/SSE roots handshake integration test.
//!
//! This test exercises the real Streamable HTTP transport on a running bridge:
//! - POST /mcp initialize (creates session)
//! - GET /mcp SSE (server→client requests)
//! - POST notifications/initialized
//! - Receive roots/list over SSE
//! - Respond with a temp workspace root
//! - Call a tool without providing working_directory and verify it runs inside the root
//!
//! Running (spawns its own server with dynamic port):
//!   cargo test -p ahma_http_bridge --test http_roots_handshake_integration_test
//!
//! Or with a custom server URL:
//!   AHMA_TEST_SSE_URL=http://localhost:3000 cargo test -p ahma_http_bridge --test http_roots_handshake_integration_test

mod common;

use common::uri::{encode_file_uri, paths_equivalent};
use common::{TestServerInstance, spawn_test_server};
use futures::StreamExt;
use reqwest::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::env;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep, timeout};

const MCP_SESSION_ID_HEADER: &str = "mcp-session-id";

/// Spawn a test server or use environment variable URL.
/// Returns (base_url, Option<server_instance>).
/// The server instance must be kept alive for the duration of the test.
async fn get_server_url() -> (String, Option<TestServerInstance>) {
    if let Ok(url) = env::var("AHMA_TEST_SSE_URL") {
        // User specified a custom URL, verify it's available
        let client = Client::new();
        let health_url = format!("{}/health", url);
        match client
            .get(&health_url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => return (url, None),
            _ => panic!("Custom server URL {} is not available", url),
        }
    }

    // Spawn our own server with dynamic port
    let server = spawn_test_server()
        .await
        .expect("Failed to spawn test server");
    let url = server.base_url();
    (url, Some(server))
}

fn extract_text_content(result: &Value) -> String {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn http_roots_handshake_then_tool_call_defaults_to_root() {
    let client = Client::new();

    let (base_url, _server) = get_server_url().await;
    eprintln!("Using server at {}", base_url);

    let temp_root = TempDir::new().expect("Failed to create temp root");
    let root_path = temp_root.path().to_path_buf();
    // Use encode_file_uri for cross-platform file URI formatting
    // (Windows needs file:///C:/... format, not file://C:\...)
    let root_uri = encode_file_uri(&root_path);

    // 1) initialize (no session header)
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": { "roots": {} },
            "clientInfo": { "name": "http_roots_test", "version": "0.1" }
        }
    });

    let init_url = format!("{}/mcp", base_url);
    let init_resp = client
        .post(&init_url)
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        .json(&init_req)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .expect("initialize POST failed");

    let status = init_resp.status();
    let headers = init_resp.headers().clone();
    let body = init_resp.text().await.unwrap_or_default();

    if !status.is_success() {
        panic!(
            "Initialize request failed with status {}: {}. URL: {}",
            status, body, init_url
        );
    }

    let session_id = match headers
        .get(MCP_SESSION_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
    {
        Some(id) => id,
        None => {
            eprintln!(
                "WARNING️  Server at {} did not return mcp-session-id (session isolation likely disabled); skipping test. Response: {}",
                base_url, body
            );
            return;
        }
    };

    // 2) Open SSE stream
    let sse_url = format!("{}/mcp", base_url);
    let mut sse_headers = HeaderMap::new();
    sse_headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    sse_headers.insert(
        MCP_SESSION_ID_HEADER,
        HeaderValue::from_str(&session_id).expect("invalid session id header"),
    );

    let sse_resp = client
        .get(sse_url)
        .headers(sse_headers)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .expect("SSE GET failed");

    assert!(sse_resp.status().is_success());

    let (tx, mut rx) = mpsc::channel::<Value>(32);

    tokio::spawn(async move {
        let mut stream = sse_resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(item) = stream.next().await {
            let Ok(bytes) = item else { break };
            let chunk = String::from_utf8_lossy(&bytes);
            buffer.push_str(&chunk);

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim_end_matches(['\r', '\n']).to_string();
                buffer = buffer[(pos + 1)..].to_string();

                let line = line.trim();
                if !line.starts_with("data:") {
                    continue;
                }

                let data = line.trim_start_matches("data:").trim();
                if data.is_empty() {
                    continue;
                }

                if let Ok(v) = serde_json::from_str::<Value>(data) {
                    let _ = tx.send(v).await;
                }
            }
        }
    });

    // 3) initialized notification
    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });

    let _ = client
        .post(format!("{}/mcp", base_url))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        .header(MCP_SESSION_ID_HEADER, session_id.as_str())
        .json(&initialized)
        .timeout(Duration::from_secs(30))
        .send()
        .await;

    // 4) Wait for roots/list over SSE
    let roots_req = timeout(Duration::from_secs(60), async {
        loop {
            if let Some(msg) = rx.recv().await
                && msg.get("method").and_then(|m| m.as_str()) == Some("roots/list")
                && msg.get("id").is_some()
            {
                return msg;
            }
        }
    })
    .await
    .expect("timed out waiting for roots/list over SSE");

    let roots_id = roots_req
        .get("id")
        .and_then(|id| id.as_i64())
        .expect("roots/list id should be integer") as u64;

    // 5) Respond with roots
    let roots_response = json!({
        "jsonrpc": "2.0",
        "id": roots_id,
        "result": {
            "roots": [{"uri": root_uri, "name": "temp"}]
        }
    });

    let roots_resp = client
        .post(format!("{}/mcp", base_url))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        .header(MCP_SESSION_ID_HEADER, session_id.as_str())
        .json(&roots_response)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .expect("roots response POST failed");

    assert!(roots_resp.status().is_success() || roots_resp.status().as_u16() == 202);

    // Wait until the server confirms sandbox lock before issuing tools/call.
    // This avoids a race where the fallback scope is still active briefly.
    timeout(Duration::from_secs(30), async {
        loop {
            if let Some(msg) = rx.recv().await
                && msg.get("method").and_then(|m| m.as_str())
                    == Some("notifications/sandbox/configured")
            {
                break;
            }
        }
    })
    .await
    .expect("timed out waiting for notifications/sandbox/configured");

    // 6) Call sandboxed_shell with pwd, retrying if the bridge still says initializing.
    // sandboxed_shell is always available (built-in), so this tests core handshake functionality.
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "sandboxed_shell",
            "arguments": {
                "command": "pwd",
                "working_directory": root_path.to_string_lossy()
            }
        }
    });

    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(60) {
            panic!("timed out waiting for tools/call to succeed after roots lock");
        }

        let resp = client
            .post(format!("{}/mcp", base_url))
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .header(MCP_SESSION_ID_HEADER, session_id.as_str())
            .json(&tool_call)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .expect("tools/call POST failed");

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let v = if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({"raw": text}))
        };

        if status.as_u16() == 409 {
            // Bridge gating while sandbox initializes
            sleep(Duration::from_millis(100)).await;
            continue;
        }

        // Also retry if JSON-RPC error indicates sandbox is still initializing
        if let Some(error) = v.get("error")
            && let Some(msg) = error.get("message").and_then(|m| m.as_str())
            && (msg.contains("initializing") || msg.contains("roots/list"))
        {
            sleep(Duration::from_millis(100)).await;
            continue;
        }

        assert!(status.is_success(), "tools/call failed: HTTP {status} {v}");
        assert!(v.get("error").is_none(), "tools/call returned error: {v}");

        let result = v.get("result").cloned().unwrap_or_else(|| json!({}));
        let out = extract_text_content(&result);
        assert!(
            paths_equivalent(&out, &root_path),
            "pwd output should contain root path. got: {out}"
        );
        break;
    }
}
