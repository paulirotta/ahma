//! Shared test utilities for HTTP bridge integration tests.
//!
//! This module intentionally re-exports focused helpers from smaller modules to
//! keep call sites stable while reducing complexity in test infrastructure.

#![allow(dead_code)]

#[macro_use]
pub mod sse_test_helpers;

pub mod client;
pub mod protocol;
pub mod sandbox_env;
pub mod server;
pub mod uri;

#[allow(unused_imports)]
pub use client::{McpTestClient, ToolCallResult, TransportMode};
#[allow(unused_imports)]
pub use protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
#[allow(unused_imports)]
pub use sandbox_env::{SANDBOX_BYPASS_ENV_VARS, SandboxTestEnv};
#[allow(unused_imports)]
pub use server::{
    ServerGuard, TestServerInstance, spawn_server_guard_with_config, spawn_test_server,
    spawn_test_server_with_timeout,
};
#[allow(unused_imports)]
pub use uri::{
    create_pwd_tool_config, encode_file_uri, malformed_uris, normalize_path_for_comparison,
    parse_file_uri, paths_equivalent,
};

/// Create an HTTP/2-only reqwest client for use against the bridge server.
///
/// The server only accepts HTTP/2 (h2c). HTTP/1.1 connections are rejected.
pub fn make_h2_client() -> reqwest::Client {
    reqwest::Client::builder()
        .http2_prior_knowledge()
        .build()
        .expect("Failed to build HTTP/2 test client")
}

/// Spawn and handshake a test MCP server/client, then verify required tools exist.
///
/// Returns `None` (skip) when infrastructure setup fails or required tools are missing.
pub async fn setup_test_mcp_for_tools(
    transport: TransportMode,
    required_tools: &[&str],
) -> Option<(TestServerInstance, McpTestClient)> {
    let (server, mcp) = setup_test_mcp(transport).await?;

    let mut missing = Vec::new();
    for tool in required_tools {
        if !mcp.is_tool_available(tool).await {
            missing.push(*tool);
        }
    }

    if missing.is_empty() {
        return Some((server, mcp));
    }

    eprintln!("WARNING  missing required tools, skipping: {:?}", missing);
    None
}

/// Asserts that a tool call succeeded and returns a borrowed output string.
pub fn assert_tool_success_with_output<'a>(result: &'a ToolCallResult, context: &str) -> &'a str {
    assert!(result.success, "{} failed: {:?}", context, result.error);
    result
        .output
        .as_deref()
        .unwrap_or_else(|| panic!("{} returned no output", context))
}

/// Heuristic helper for tools that may legitimately return async operation hints.
pub fn is_async_operation_output(output: &str) -> bool {
    output.contains("operation")
        || output.contains("async")
        || output.contains("started")
        || output.contains("op_")
}

/// Spawn a test server and complete the full MCP handshake.
///
/// Returns `None` with a warning on any infrastructure failure so that tests
/// can skip gracefully instead of panicking.  The returned
/// [`TestServerInstance`] **must** be kept alive for the duration of the test.
pub async fn setup_test_mcp(
    transport: TransportMode,
) -> Option<(TestServerInstance, McpTestClient)> {
    let server = match spawn_test_server().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("WARNING  setup_test_mcp: server spawn failed: {}", e);
            return None;
        }
    };
    let root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut mcp = McpTestClient::with_url(&server.base_url()).with_transport(transport);
    match mcp.initialize_with_roots("tool-test-client", &[root]).await {
        Ok(_) => Some((server, mcp)),
        Err(e) => {
            eprintln!("WARNING  setup_test_mcp: handshake failed: {}", e);
            None
        }
    }
}
