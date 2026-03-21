//! SSE endpoint guardrail tests.
//!
//! These tests ensure the HTTP bridge hides the SSE endpoint unless a valid
//! session header is provided, returning 404 per protocol guidance.

mod common;

use common::spawn_test_server;
use reqwest::StatusCode;

#[tokio::test]
async fn test_sse_without_session_header_returns_404() {
    let server = spawn_test_server()
        .await
        .expect("Failed to spawn test server");
    let client = common::make_h2_client();

    let resp = client
        .get(format!("{}/mcp", server.base_url()))
        .send()
        .await
        .expect("SSE GET without session should not error at transport");

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_sse_with_unknown_session_returns_404() {
    let server = spawn_test_server()
        .await
        .expect("Failed to spawn test server");
    let client = common::make_h2_client();

    let resp = client
        .get(format!("{}/mcp", server.base_url()))
        .header("mcp-session-id", "00000000-0000-0000-0000-000000000000")
        .send()
        .await
        .expect("SSE GET with unknown session should not error at transport");

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
