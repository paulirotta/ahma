//! Regression test: the HTTP bridge should fail fast on malformed initialize requests.
//!
//! A missing `params.protocolVersion` previously caused the bridge to create a session,
//! forward the request to the stdio subprocess, and then hang until a timeout.
//!
//! This test ensures we return an immediate JSON-RPC error instead.

mod common;

use common::spawn_test_server;
use serde_json::Value;

#[tokio::test]
async fn test_initialize_missing_protocol_version_fails_fast() {
    let server = spawn_test_server()
        .await
        .expect("Failed to spawn test server");
    let client = common::make_h2_client();

    let malformed_initialize = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            // Intentionally missing: "protocolVersion"
            "capabilities": {"roots": {}},
            "clientInfo": {"name": "test-invalid-init", "version": "1.0"}
        }
    });

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        client
            .post(format!("{}/mcp", server.base_url()))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&malformed_initialize)
            .send(),
    )
    .await
    .expect("Request should fail fast, not time out")
    .expect("HTTP request should complete");

    assert!(
        !resp.headers().contains_key("mcp-session-id")
            && !resp.headers().contains_key("Mcp-Session-Id"),
        "Malformed initialize must not create a session"
    );

    // The bridge currently returns INTERNAL_SERVER_ERROR for JSON-RPC errors.
    assert_eq!(resp.status().as_u16(), 500);

    let body: Value = resp.json().await.expect("Response should be JSON");
    let error = body.get("error").expect("JSON-RPC error object expected");

    assert_eq!(
        error.get("code").and_then(|c| c.as_i64()),
        Some(-32602),
        "Expected invalid params error code -32602. Body: {body:?}"
    );

    let msg = error
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or_default();

    assert!(
        msg.contains("missing") && msg.contains("protocolVersion"),
        "Expected message to mention missing protocolVersion. Got: {msg}"
    );
}
