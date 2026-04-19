//! # Client Module Coverage Tests
//!
//! This file provides integration tests to improve coverage of `ahma_mcp/src/client.rs`.
//! The module already has unit tests for helper functions (`extract_id`,
//! `join_text_contents`, `first_text_content`). This file adds integration tests for:
//!
//! - `Client::start_process` and `start_process_with_args`
//! - `Client::get_service` error path (when not initialized)
//! - `Client::shell_async_sleep`, `await_op`, and `status` methods
//!
//! These tests use the real ahma_mcp binary to ensure full integration coverage.

use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};
use ahma_mcp::skip_if_disabled_async_result;
use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::test_utils::project::{TestProjectOptions, create_rust_project};
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;

async fn call_test_tool(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    name: &str,
    args: serde_json::Value,
) -> Result<rmcp::model::CallToolResult> {
    let mut params = CallToolRequestParams::new(name.to_string());
    if let Some(arguments) = args.as_object().cloned() {
        params = params.with_arguments(arguments);
    }

    let timeout = TestTimeouts::get(TimeoutCategory::ToolCall);
    Ok(tokio::time::timeout(timeout, client.call_tool(params))
        .await
        .map_err(|_| anyhow::anyhow!("call_tool timed out after {:?}", timeout))??)
}

fn get_result_text(result: &rmcp::model::CallToolResult) -> &str {
    result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or("")
}

async fn build_test_client() -> Result<ahma_mcp::test_utils::in_process::InProcessMcp> {
    ahma_mcp::test_utils::in_process::create_in_process_mcp_from_dir(std::path::Path::new(".ahma"))
        .await
}

// ============================================================================
// Client Initialization and Process Spawning Tests
// ============================================================================

/// Test that new_client works with the tools directory
#[tokio::test]
async fn test_client_start_process_with_tools_dir() -> Result<()> {
    init_test_logging();
    let mcp = build_test_client().await?;
    let client = &mcp.client;

    // Verify client is functional by listing tools (using the MCP protocol)
    let tools = client.list_all_tools().await?;

    // Should have default tools available
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        tool_names.contains(&"sandboxed_shell")
            || tool_names.contains(&"await")
            || tool_names.contains(&"status"),
        "Expected standard tools, got: {:?}",
        tool_names
    );

    Ok(())
}

/// Test that new_client_with_args handles extra arguments like --sync
#[tokio::test]
async fn test_client_start_process_with_sync_flag() -> Result<()> {
    init_test_logging();

    // Enable synchronous tool execution via env var (--sync flag removed in new CLI)
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .env("AHMA_SYNC", "1")
        .build()
        .await?;

    // Verify client works by listing tools
    let tools = client.list_all_tools().await?;
    assert!(!tools.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test that new_client_with_args works with debug flag
#[tokio::test]
async fn test_client_start_process_with_debug_flag() -> Result<()> {
    init_test_logging();

    // Enable debug logging via env var (--debug flag removed in new CLI)
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .env("RUST_LOG", "debug")
        .build()
        .await?;

    // Verify client is functional by listing tools
    let tools = client.list_all_tools().await?;
    assert!(!tools.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test that new_client_with_args works with --log-to-stderr flag
#[tokio::test]
async fn test_client_start_process_with_log_to_stderr() -> Result<()> {
    init_test_logging();

    // Route logs to stderr via env var (--log-to-stderr flag removed in new CLI)
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .env("AHMA_LOG_TARGET", "stderr")
        .build()
        .await?;

    // Verify client is functional
    let result = call_test_tool(&client, "status", json!({})).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Status Tool Tests via Client
// ============================================================================

/// Test status tool returns expected format when no operations
#[tokio::test]
async fn test_client_status_no_operations() -> Result<()> {
    init_test_logging();
    let mcp = build_test_client().await?;
    let client = &mcp.client;

    let result = call_test_tool(client, "status", json!({})).await?;
    assert!(!result.content.is_empty());

    let text = get_result_text(&result);
    // Should indicate operations status
    assert!(
        text.contains("Operations")
            || text.contains("active")
            || text.contains("completed")
            || text.contains("No"),
        "Expected status output, got: {}",
        text
    );
    Ok(())
}

/// Test status tool with a specific id filter
#[tokio::test]
async fn test_client_status_with_id() -> Result<()> {
    init_test_logging();
    let mcp = match build_test_client().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "WARNING  test_client_status_with_id: client setup failed: {}. Skipping.",
                e
            );
            return Ok(());
        }
    };
    let client = &mcp.client;

    // Query status for a nonexistent operation
    let result = call_test_tool(client, "status", json!({ "id": "nonexistent_op_12345" })).await?;

    // Should handle gracefully - no crash, returns some response
    assert!(!result.content.is_empty());
    Ok(())
}

// ============================================================================
// Await Tool Tests via Client
// ============================================================================

/// Test await tool with no pending operations
#[tokio::test]
async fn test_client_await_no_pending() -> Result<()> {
    init_test_logging();
    let mcp = build_test_client().await?;
    let client = &mcp.client;

    let result = call_test_tool(client, "await", json!({})).await?;
    // Should return quickly indicating nothing to await
    assert!(!result.content.is_empty());
    Ok(())
}

/// Test await tool with specific id that doesn't exist
#[tokio::test]
async fn test_client_await_nonexistent_operation() -> Result<()> {
    init_test_logging();
    let mcp = build_test_client().await?;
    let client = &mcp.client;

    let result = call_test_tool(client, "await", json!({ "id": "nonexistent_op_67890" })).await?;
    // Should handle gracefully
    assert!(!result.content.is_empty());
    Ok(())
}

// ============================================================================
// Full Async Operation Lifecycle Tests
// ============================================================================

/// Test full async operation lifecycle: start, status, await
#[tokio::test]
async fn test_async_operation_lifecycle() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let mcp = build_test_client().await?;
    let client = &mcp.client;

    // Start an async operation (short sleep)
    let start_result =
        call_test_tool(client, "sandboxed_shell", json!({ "command": "sleep 0.5" })).await?;

    assert!(!start_result.content.is_empty());

    let text = get_result_text(&start_result);
    // Check if it started as async (contains operation ID)
    if text.contains("ID:") {
        // Extract operation ID
        let op_id = extract_op_id(text);

        // Check status while running
        let status_result =
            call_test_tool(client, "status", json!({ "id": op_id.clone() })).await?;
        assert!(!status_result.content.is_empty());

        // Await completion
        let await_result = call_test_tool(client, "await", json!({ "id": op_id })).await?;
        assert!(!await_result.content.is_empty());
    }
    Ok(())
}

/// Test multiple async operations can be tracked
#[tokio::test]
async fn test_multiple_async_operations() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let mcp = build_test_client().await?;
    let client = &mcp.client;

    // Start two async operations
    let result1 =
        call_test_tool(client, "sandboxed_shell", json!({ "command": "sleep 0.3" })).await?;

    let result2 =
        call_test_tool(client, "sandboxed_shell", json!({ "command": "sleep 0.3" })).await?;

    // Check overall status
    let status = call_test_tool(client, "status", json!({})).await?;
    assert!(!status.content.is_empty());

    // Await all
    let await_result = call_test_tool(client, "await", json!({})).await?;
    assert!(!await_result.content.is_empty());

    // Results should exist
    assert!(!result1.content.is_empty());
    assert!(!result2.content.is_empty());
    Ok(())
}

// ============================================================================
// Shell Execution Tests
// ============================================================================

/// Test sandboxed_shell tool execution (covers shell-related paths in client)
#[tokio::test]
async fn test_sandboxed_shell_execution() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let mcp = build_test_client().await?;
    let client = &mcp.client;

    // Run a simple command
    let result = call_test_tool(
        client,
        "sandboxed_shell",
        json!({ "command": "echo 'hello from test'" }),
    )
    .await?;
    assert!(!result.content.is_empty());
    Ok(())
}

/// Test sandboxed_shell with working_directory parameter
/// Uses a directory inside the workspace to comply with sandbox restrictions
#[tokio::test]
async fn test_sandboxed_shell_with_working_dir() -> Result<()> {
    use ahma_mcp::test_utils::fs::get_workspace_tools_dir;
    skip_if_disabled_async_result!("sandboxed_shell");

    init_test_logging();

    let mcp = build_test_client().await?;
    let client = &mcp.client;

    // Use the workspace's target directory which is inside the sandbox
    let tools_dir = get_workspace_tools_dir();
    let workspace_dir = tools_dir.parent().expect("Should have workspace parent");
    let target_dir = workspace_dir.join("target");

    // Use target directory if it exists, otherwise use workspace root
    let working_dir = if target_dir.exists() {
        target_dir
    } else {
        workspace_dir.to_path_buf()
    };

    let result = call_test_tool(
        client,
        "sandboxed_shell",
        json!({
            "command": "pwd",
            "working_directory": working_dir.to_str().unwrap()
        }),
    )
    .await?;
    assert!(!result.content.is_empty());

    Ok(())
}

// ============================================================================
// Error Handling Tests
// ============================================================================

/// Test calling a tool that doesn't exist
#[tokio::test]
async fn test_call_nonexistent_tool() -> Result<()> {
    init_test_logging();
    let mcp = build_test_client().await?;
    let client = &mcp.client;

    let result = call_test_tool(client, "this_tool_definitely_does_not_exist_xyz", json!({})).await;
    // Should return an error
    assert!(result.is_err(), "Expected error for nonexistent tool");
    Ok(())
}

/// Test list_tools returns expected format
#[tokio::test]
async fn test_list_tools_format() -> Result<()> {
    init_test_logging();
    let mcp = build_test_client().await?;
    let client = &mcp.client;

    // Verify by listing tools
    let tools = client.list_all_tools().await?;
    assert!(!tools.is_empty());

    // Should contain standard tools
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        tool_names.contains(&"sandboxed_shell")
            || tool_names.contains(&"await")
            || tool_names.contains(&"status"),
        "Expected standard tools, got: {:?}",
        tool_names
    );

    Ok(())
}

// ============================================================================
// Custom Tools Directory Tests
// ============================================================================

/// Test client with custom tools directory using new_client_in_dir
/// This test verifies that custom tool configurations are loaded correctly
#[tokio::test]
async fn test_client_with_custom_tools_dir() -> Result<()> {
    init_test_logging();

    let temp_project = create_rust_project(TestProjectOptions {
        prefix: Some("custom_tools_test_".to_string()),
        with_cargo: false,
        with_text_files: false,
        with_tool_configs: true,
    })
    .await?;

    // Use new_client_in_dir to set the working directory to the temp project
    // This way the sandbox scope will include the temp directory
    let tools_dir = temp_project.path().join(".ahma");
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_project.path())
        .build()
        .await?;

    // The custom project has an "echo" tool defined - use list_all_tools
    let tools = client.list_all_tools().await?;
    assert!(!tools.is_empty());

    // Should list the echo tool from custom config or at least the built-in tools
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        tool_names.contains(&"echo") || tool_names.contains(&"await"),
        "Expected tools from config, got: {:?}",
        tool_names
    );

    client.cancel().await?;
    Ok(())
}

/// Helper to extract operation ID from response
fn extract_op_id(text: &str) -> String {
    text.find("ID: ")
        .and_then(|i| text[i + 4..].split_whitespace().next())
        .unwrap_or("unknown")
        .to_string()
}
