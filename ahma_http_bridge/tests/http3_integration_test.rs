//! Integration tests for HTTP/3 (QUIC) server support.
//!
//! These tests verify that the ahma HTTP bridge correctly:
//! - Starts a QUIC endpoint and advertises it via Alt-Svc headers
//! - Serves actual HTTP/3 requests from a QUIC-capable reqwest client
//! - Rejects SSE streams over QUIC (HTTP/3 does not support SSE)
//! - Continues serving HTTP/2 MCP requests normally alongside QUIC
//!
//! QUIC tests are skipped non-fatally when the server does not report a QUIC
//! port (e.g. if QUIC failed to start, or the test binary was built without
//! `reqwest_unstable`).

mod common;

use common::{McpTestClient, TransportMode, spawn_test_server};
use reqwest::Certificate;
use serde_json::json;
use std::time::Duration;

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Build a reqwest client that speaks HTTP/3 (QUIC).
///
/// `cert_der` is the self-signed DER certificate the test server emitted.
fn build_http3_client(cert_der: &[u8]) -> reqwest::Client {
    let cert = Certificate::from_der(cert_der).expect("parse DER cert");
    reqwest::Client::builder()
        .http3_prior_knowledge()
        .add_root_certificate(cert)
        .timeout(Duration::from_secs(30))
        .build()
        .expect("HTTP/3 reqwest client")
}

// ─── HTTP/3 QUIC tests (skip if QUIC did not start) ──────────────────────────

/// Verify a QUIC health-check returns HTTP/3 and a 200 OK.
#[tokio::test]
async fn test_http3_quic_health_check() {
    let server = spawn_test_server().await.expect("server should start");

    let (Some(quic_url), Some(cert_der)) = (server.quic_base_url(), server.quic_cert_der()) else {
        eprintln!("SKIP: server did not start a QUIC endpoint");
        return;
    };

    let client = build_http3_client(cert_der);
    let resp = client
        .get(format!("{}/health", quic_url))
        .version(reqwest::Version::HTTP_3)
        .send()
        .await
        .expect("HTTP/3 health check request");

    assert!(
        resp.status().is_success(),
        "health check should return 2xx, got {}",
        resp.status()
    );
    assert_eq!(
        resp.version(),
        reqwest::Version::HTTP_3,
        "expected HTTP/3 response"
    );
}

/// Verify a QUIC initialize POST returns HTTP/3 and a valid MCP result.
#[tokio::test]
async fn test_http3_quic_initialize() {
    let server = spawn_test_server().await.expect("server should start");

    let (Some(quic_url), Some(cert_der)) = (server.quic_base_url(), server.quic_cert_der()) else {
        eprintln!("SKIP: server did not start a QUIC endpoint");
        return;
    };

    let client = build_http3_client(cert_der);
    let resp = client
        .post(format!("{}/mcp", quic_url))
        .version(reqwest::Version::HTTP_3)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "http3-init-test", "version": "1.0"}
            }
        }))
        .send()
        .await
        .expect("HTTP/3 initialize request");

    assert!(
        resp.status().is_success(),
        "initialize should return 2xx via QUIC, got {}",
        resp.status()
    );
    assert_eq!(resp.version(), reqwest::Version::HTTP_3, "expected HTTP/3");

    let body: serde_json::Value = resp.json().await.expect("parse JSON body");
    assert!(
        body.get("result").is_some(),
        "initialize should return a result, got: {body}"
    );
}

/// Verify an SSE GET over HTTP/3 is rejected with 406 Not Acceptable.
///
/// SSE (text/event-stream) requires a long-lived, half-closed streaming
/// response which is not compatible with the HTTP/3 request model used here.
#[tokio::test]
async fn test_http3_quic_sse_get_returns_406() {
    let server = spawn_test_server().await.expect("server should start");

    let (Some(quic_url), Some(cert_der)) = (server.quic_base_url(), server.quic_cert_der()) else {
        eprintln!("SKIP: server did not start a QUIC endpoint");
        return;
    };

    let client = build_http3_client(cert_der);
    // We need a session first (initialize over QUIC).
    let init = client
        .post(format!("{}/mcp", quic_url))
        .version(reqwest::Version::HTTP_3)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "http3-sse-test", "version": "1.0"}
            }
        }))
        .send()
        .await
        .expect("initialize over QUIC");

    let session_id = init
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let mut get_req = client
        .get(format!("{}/mcp", quic_url))
        .version(reqwest::Version::HTTP_3)
        .header("Accept", "text/event-stream");

    if let Some(ref sid) = session_id {
        get_req = get_req.header("Mcp-Session-Id", sid.as_str());
    }

    let resp = get_req.send().await.expect("SSE GET over QUIC");

    assert_eq!(
        resp.status().as_u16(),
        406,
        "SSE GET over HTTP/3 should return 406 Not Acceptable, got {}",
        resp.status()
    );
}

// ─── Alt-Svc advertisement test (HTTP/2) ─────────────────────────────────────

/// Verify that TCP (HTTP/2) responses carry an Alt-Svc header when QUIC is
/// running, advertising the HTTP/3 alternative endpoint.
#[tokio::test]
async fn test_http2_alt_svc_advertised_when_quic_ready() {
    let server = spawn_test_server().await.expect("server should start");

    if server.quic_port().is_none() {
        eprintln!("SKIP: server did not start a QUIC endpoint");
        return;
    }

    let client = common::make_h2_client();
    let resp = client
        .get(format!("{}/health", server.base_url()))
        .send()
        .await
        .expect("HTTP/2 health check");

    assert!(resp.status().is_success(), "health check 200");

    let alt_svc = resp
        .headers()
        .get("alt-svc")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    assert!(
        alt_svc.contains("h3="),
        "Alt-Svc header should advertise HTTP/3 when QUIC is running, got: {alt_svc:?}"
    );
}

// ─── HTTP/2 MCP functional tests ─────────────────────────────────────────────

/// Full MCP handshake + tools/list via JSON transport (HTTP/2).
#[tokio::test]
async fn test_http2_mcp_tools_list_json() {
    run_mcp_tools_list(TransportMode::Json).await;
}

/// Full MCP handshake + tools/list via SSE transport (HTTP/2).
#[tokio::test]
async fn test_http2_mcp_tools_list_sse() {
    run_mcp_tools_list(TransportMode::Sse).await;
}

async fn run_mcp_tools_list(mode: TransportMode) {
    let server = spawn_test_server().await.expect("server should start");
    let mut mcp = McpTestClient::for_server(&server).with_transport(mode);

    let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();

    let init = mcp
        .initialize_with_roots("http3-test-client", &[workspace])
        .await;
    assert!(
        init.is_ok(),
        "MCP handshake should succeed: {:?}",
        init.err()
    );

    let tools = mcp.list_tools().await;
    assert!(
        tools.is_ok(),
        "tools/list should succeed: {:?}",
        tools.err()
    );
    assert!(
        !tools.unwrap().is_empty(),
        "server should expose at least one tool"
    );
}
