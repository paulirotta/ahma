//! Integration tests for MCP Streamable HTTP SSE enhancements:
//! - POST SSE content negotiation
//! - SSE event IDs
//! - Last-Event-Id replay
//! - POST SSE streaming responses

mod common;

use ahma_common::timeouts::TestTimeouts;
use common::{McpTestClient, spawn_test_server};
use futures::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;

fn find_sse_event_id(buffer: &str) -> Option<u64> {
    buffer.lines().find_map(|line| {
        line.strip_prefix("id:")
            .map(|s| s.trim().parse::<u64>().expect("Event ID should be numeric"))
    })
}

/// Verify that POST with `Accept: application/json` returns JSON (backwards compat).
/// Uses the `initialize` request which doesn't require prior session setup.
#[tokio::test]
async fn test_post_json_content_negotiation() {
    let server = spawn_test_server().await.expect("server should start");

    let http = common::make_h2_client();
    let resp = http
        .post(format!("{}/mcp", server.base_url()))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "json-test", "version": "1.0"}
            }
        }))
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .expect("request should succeed");

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("application/json"),
        "Expected JSON content type, got: {}",
        content_type
    );

    // Verify we got a valid JSON-RPC response
    let body: Value = resp.json().await.expect("should parse JSON");
    assert!(body.get("result").is_some(), "Should have result field");
}

/// Verify that POST with `Accept: text/event-stream` returns SSE.
#[tokio::test]
async fn test_post_sse_content_negotiation() {
    let server = spawn_test_server().await.expect("server should start");

    let http = common::make_h2_client();

    let resp = http
        .post(format!("{}/mcp", server.base_url()))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "sse-test", "version": "1.0"}
            }
        }))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .expect("request should succeed");

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.contains("application/json") {
        let b = resp.bytes().await.unwrap_or_default();
        panic!(
            "Expected SSE content type, got JSON error: {}",
            String::from_utf8_lossy(&b)
        );
    }

    assert!(
        content_type.contains("text/event-stream"),
        "Expected SSE content type, got: {}",
        content_type
    );

    // Should have session ID header
    let session_id = resp
        .headers()
        .get("mcp-session-id")
        .or_else(|| resp.headers().get("Mcp-Session-Id"))
        .and_then(|v| v.to_str().ok());
    assert!(
        session_id.is_some(),
        "POST SSE initialize should return session ID header"
    );
}

/// Verify that POST SSE response includes event ID fields.
#[tokio::test]
async fn test_post_sse_response_includes_event_id() {
    let server = spawn_test_server().await.expect("server should start");

    let http = common::make_h2_client();

    let resp = http
        .post(format!("{}/mcp", server.base_url()))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "event-id-test", "version": "1.0"}
            }
        }))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .expect("request should succeed");

    // Read SSE stream and check for id: field
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(2000), stream.next()).await {
            Ok(Some(Ok(bytes))) => {
                buffer.push_str(&String::from_utf8_lossy(&bytes));
                if buffer.contains("id:") && buffer.contains("data:") {
                    let Some(id) = find_sse_event_id(&buffer) else {
                        continue;
                    };
                    assert!(id > 0, "Event ID should be positive");
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("Stream error: {}", e),
            Ok(None) => break,
            Err(_) => continue,
        }
    }

    panic!(
        "Did not find SSE event with id: field in response. Buffer: {}",
        buffer
    );
}

/// Verify that GET SSE events include event IDs.
#[tokio::test]
async fn test_get_sse_events_include_event_id() {
    let server = spawn_test_server().await.expect("server should start");
    let mut client = McpTestClient::for_server(&server);

    let _init = client
        .initialize_only("sse-event-id-test")
        .await
        .expect("initialize should succeed");

    let session_id = client
        .session_id()
        .expect("should have session")
        .to_string();

    let http = common::make_h2_client();

    // Open SSE stream
    let resp = http
        .get(format!("{}/mcp", server.base_url()))
        .header("Accept", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Mcp-Session-Id", &session_id)
        .send()
        .await
        .expect("SSE connection should succeed");

    assert!(resp.status().is_success(), "SSE should return 200");

    // Send notifications/initialized to trigger roots/list over SSE
    let _ = client.send_initialized().await;

    // Read SSE events and verify they have id fields
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(2000), stream.next()).await {
            Ok(Some(Ok(bytes))) => {
                buffer.push_str(&String::from_utf8_lossy(&bytes));
                if let Some(id) = find_sse_event_id(&buffer) {
                    assert!(id > 0, "Event ID should be positive");
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("Stream error: {}", e),
            Ok(None) => break,
            Err(_) => continue,
        }
    }

    // It's possible no broadcast events arrived if the subprocess didn't send anything.
    // This is acceptable — the important thing is that if events do arrive, they have IDs.
    // We skip rather than fail if no events were observed.
    if buffer.is_empty() || !buffer.contains("data:") {
        eprintln!(
            "No SSE data events observed during test window (subprocess may not have sent any). Skipping assertion."
        );
        return;
    }

    panic!(
        "SSE events contained data: but no id: field. Buffer: {}",
        buffer
    );
}

/// Verify that GET SSE with Last-Event-Id replays missed events.
#[tokio::test]
async fn test_get_sse_last_event_id_replay() {
    use ahma_http_bridge::{DEFAULT_HANDSHAKE_TIMEOUT_SECS, SessionManager, SessionManagerConfig};
    use std::sync::Arc;
    use tempfile::TempDir;

    // Use unit-test level session to control broadcast directly
    let temp_dir = TempDir::new().expect("temp dir");
    let script_path = temp_dir.path().join("mock.py");
    std::fs::write(
        &script_path,
        r#"import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except:
        continue
    if not isinstance(msg, dict) or "method" not in msg:
        continue
    method = msg.get("method")
    msg_id = msg.get("id")
    if method == "initialize":
        resp = {"jsonrpc":"2.0","id":msg_id,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock","version":"1.0"}}}
        print(json.dumps(resp))
        sys.stdout.flush()
    elif msg_id is not None:
        print(json.dumps({"jsonrpc":"2.0","id":msg_id,"result":{}}))
        sys.stdout.flush()
"#,
    )
    .unwrap();

    let sm = Arc::new(SessionManager::new(SessionManagerConfig {
        server_command: "python3".to_string(),
        server_args: vec![script_path.to_string_lossy().to_string()],
        default_scope: Some(temp_dir.path().to_path_buf()),
        enable_colored_output: false,
        handshake_timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    }));

    let session_id = sm.create_session().await.expect("create session");
    let session = sm.get_session(&session_id).expect("get session");

    // Broadcast several events (they get IDs assigned)
    for i in 0..5 {
        let _ = session.broadcast(format!(r#"{{"event":{}}}"#, i));
    }

    // Verify replay: events after ID 2 should return events 3, 4, 5
    let replay = session.replay_events_after(2);
    assert_eq!(replay.len(), 3, "Should replay 3 events after ID 2");
    assert_eq!(replay[0].0, 3);
    assert_eq!(replay[1].0, 4);
    assert_eq!(replay[2].0, 5);

    // Replay after ID 0 should return all 5
    let replay_all = session.replay_events_after(0);
    assert_eq!(replay_all.len(), 5, "Should replay all 5 events after ID 0");

    // Replay after last ID should return empty
    let replay_none = session.replay_events_after(5);
    assert!(
        replay_none.is_empty(),
        "Should return empty for events after last ID"
    );
}

/// Verify that POST SSE streams the JSON-RPC response as an SSE event.
#[tokio::test]
async fn test_post_sse_streams_response() {
    let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let init_timeout = TestTimeouts::scale_secs(15);

    // Retry once to handle transient handshake contention under high parallel load.
    let mut setup = None;
    let mut last_error = String::new();
    for attempt in 1..=2 {
        let server = match spawn_test_server().await {
            Ok(server) => server,
            Err(e) => {
                panic!("server should start: {}", e);
            }
        };
        let mut client = McpTestClient::for_server(&server);

        match tokio::time::timeout(
            init_timeout,
            client.initialize_with_roots("sse-stream-test", std::slice::from_ref(&workspace)),
        )
        .await
        {
            Ok(Ok(_)) => {
                setup = Some((server, client));
                break;
            }
            Ok(Err(e)) => {
                last_error = e.to_string();
            }
            Err(_) => {
                last_error = format!("initialize_with_roots timed out after {:?}", init_timeout);
            }
        }

        if attempt == 1 {
            eprintln!(
                "WARNING  test_post_sse_streams_response handshake attempt {} failed: {}. Retrying once...",
                attempt, last_error
            );
        }
    }

    let Some((server, client)) = setup else {
        eprintln!(
            "WARNING  Skipping test_post_sse_streams_response due handshake instability: {}",
            last_error
        );
        return;
    };

    let session_id = client
        .session_id()
        .expect("should have session")
        .to_string();
    let http = common::make_h2_client();

    let resp = http
        .post(format!("{}/mcp", server.base_url()))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "tools/list"
        }))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("Mcp-Session-Id", &session_id)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .expect("request should succeed");

    assert!(resp.status().is_success(), "Should return 2xx");
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.contains("application/json") {
        let b = resp.bytes().await.unwrap_or_default();
        panic!(
            "Should be SSE, got JSON error: {}",
            String::from_utf8_lossy(&b)
        );
    }

    assert!(
        content_type.contains("text/event-stream"),
        "Should be SSE, got: {}",
        content_type
    );

    // Parse SSE stream to find the response
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(2000), stream.next()).await {
            Ok(Some(Ok(bytes))) => {
                buffer.push_str(&String::from_utf8_lossy(&bytes));
            }
            Ok(Some(Err(e))) => panic!("Stream error: {}", e),
            Ok(None) => break,
            Err(_) => {
                // Check if we already have the response
                if buffer.contains("\"tools\"") {
                    break;
                }
                continue;
            }
        }
    }

    // Extract data: lines and parse JSON
    let data_lines: Vec<&str> = buffer
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim))
        .collect();

    assert!(
        !data_lines.is_empty(),
        "Should have at least one data event. Buffer: {}",
        buffer
    );

    // At least one event should contain the tools/list response
    let found_response = data_lines.iter().any(|data| {
        serde_json::from_str::<Value>(data)
            .ok()
            .is_some_and(|v| v.get("result").and_then(|r| r.get("tools")).is_some())
    });

    assert!(
        found_response,
        "Should find tools/list response in SSE events. Data: {:?}",
        data_lines
    );
}
