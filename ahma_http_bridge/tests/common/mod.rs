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
    ServerGuard, TestServerInstance, spawn_test_server, spawn_test_server_with_timeout,
};
#[allow(unused_imports)]
pub use uri::{
    create_pwd_tool_config, encode_file_uri, malformed_uris, normalize_path_for_comparison,
    parse_file_uri, paths_equivalent,
};

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
