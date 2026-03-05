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
pub use client::{McpTestClient, ToolCallResult};
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
