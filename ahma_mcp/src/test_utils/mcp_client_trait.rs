//! Unified `McpTestClient` trait for test helpers across all test layers.
//!
//! Provides a single high-level interface for calling tools and listing tools
//! regardless of whether the test is using:
//!
//! * **In-process** — `InProcessMcp` wired over a `tokio::io::duplex` channel.
//! * **Subprocess** — a `RunningService<RoleClient, ()>` connected to a spawned
//!   `ahma-mcp` binary via stdio transport.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ahma_mcp::test_utils::in_process::create_in_process_mcp_empty;
//! use ahma_mcp::test_utils::mcp_client_trait::McpTestClient;
//! use serde_json::json;
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! let mcp = create_in_process_mcp_empty().await?;
//! let result = mcp.call_tool("sandboxed_shell", json!({"subcommand": "default", "args": ["echo hi"]})).await;
//! assert!(result.success, "{:?}", result.error);
//! # Ok(())
//! # }
//! ```

use async_trait::async_trait;
use rmcp::{
    model::CallToolRequestParams,
    service::{RoleClient, RunningService},
};
use serde_json::Value;

use super::in_process::InProcessMcp;

// ─────────────────────────────────────────────────────────────────────────────
// Result type
// ─────────────────────────────────────────────────────────────────────────────

/// A simplified, transport-agnostic tool call result.
///
/// Normalises both success and error paths from the underlying MCP protocol
/// into a common shape that is easy to assert on in tests.
#[derive(Debug)]
pub struct ToolCallResult {
    /// `true` if the tool reported success (i.e. `isError` is absent or `false`).
    pub success: bool,
    /// The concatenated text output, if any.
    pub output: Option<String>,
    /// The error description when `success` is `false`.
    pub error: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Trait
// ─────────────────────────────────────────────────────────────────────────────

/// High-level MCP client interface shared across all test layers.
#[async_trait]
pub trait McpTestClient: Send + Sync {
    /// Call a tool by name, passing `args` as the JSON argument object.
    ///
    /// `args` should be a `serde_json::Value::Object`; a non-object value is
    /// treated as an empty arguments map.
    async fn call_tool(&self, name: &str, args: Value) -> ToolCallResult;

    /// List the names of all tools currently exposed by the server.
    async fn list_tools(&self) -> Result<Vec<String>, String>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────────────

fn build_params(name: &str, args: Value) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new(name.to_string());
    if let Value::Object(map) = args {
        params = params.with_arguments(map);
    }
    params
}

async fn call_tool_on_service(
    service: &RunningService<RoleClient, ()>,
    name: &str,
    args: Value,
) -> ToolCallResult {
    let params = build_params(name, args);
    // Use .peer() to explicitly target Peer<RoleClient>::call_tool and avoid
    // resolving to McpTestClient::call_tool (which has a different signature).
    match service.peer().call_tool(params).await {
        Err(e) => ToolCallResult {
            success: false,
            output: None,
            error: Some(e.to_string()),
        },
        Ok(result) => {
            let is_error = result.is_error.unwrap_or(false);
            let text = result
                .content
                .iter()
                .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                .collect::<Vec<_>>()
                .join("\n");
            ToolCallResult {
                success: !is_error,
                output: if text.is_empty() {
                    None
                } else {
                    Some(text.clone())
                },
                error: if is_error { Some(text) } else { None },
            }
        }
    }
}

async fn list_tools_on_service(
    service: &RunningService<RoleClient, ()>,
) -> Result<Vec<String>, String> {
    service
        .peer()
        .list_all_tools()
        .await
        .map(|tools| tools.into_iter().map(|t| t.name.into_owned()).collect())
        .map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Implementations
// ─────────────────────────────────────────────────────────────────────────────

/// In-process implementation (uses the `client` half of `InProcessMcp`).
#[async_trait]
impl McpTestClient for InProcessMcp {
    async fn call_tool(&self, name: &str, args: Value) -> ToolCallResult {
        call_tool_on_service(&self.client, name, args).await
    }

    async fn list_tools(&self) -> Result<Vec<String>, String> {
        list_tools_on_service(&self.client).await
    }
}

/// Subprocess implementation (wraps `RunningService<RoleClient, ()>` directly).
#[async_trait]
impl McpTestClient for RunningService<RoleClient, ()> {
    async fn call_tool(&self, name: &str, args: Value) -> ToolCallResult {
        call_tool_on_service(self, name, args).await
    }

    async fn list_tools(&self) -> Result<Vec<String>, String> {
        list_tools_on_service(self).await
    }
}
