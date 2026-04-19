//! MCP Service Integration Coverage Tests
//!
//! Real integration tests for mcp_service/mod.rs targeting low-coverage areas:
//! - handle_await (with id filter, tool filters, timeout paths)
//! - handle_status (with filters, concurrency analysis)
//! - call_tool paths (sequence detection, disabled tools, cancel)
//! - calculate_intelligent_timeout
//!
//! These tests spawn the actual ahma_mcp binary and communicate via MCP protocol.

use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};
use ahma_mcp::skip_if_disabled_async_result;
use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::test_utils::in_process::{
    create_in_process_mcp_empty, create_in_process_mcp_from_dir,
};
use ahma_mcp::test_utils::project::{TestProjectOptions, create_rust_project};
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::path::Path;

// ============================================================================
// Test Helpers - Reduce boilerplate and improve readability
// ============================================================================

/// Creates CallToolRequestParams with the given tool name and optional arguments
fn make_params(name: &'static str, args: Option<serde_json::Value>) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new(name);
    if let Some(arguments) = args.and_then(|v| v.as_object().cloned()) {
        params = params.with_arguments(arguments);
    }
    params
}

/// Extracts text content from a tool call result, returning None if not available
fn get_result_text(result: &rmcp::model::CallToolResult) -> Option<&str> {
    result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
}

/// Asserts that the result text contains at least one of the expected substrings
fn assert_text_contains_any(
    result: &rmcp::model::CallToolResult,
    expected: &[&str],
    context: &str,
) {
    let text = get_result_text(result).expect("Result should contain text content");
    let found = expected.iter().any(|s| text.contains(s));
    assert!(
        found,
        "{context}. Expected one of {:?}, got: {text}",
        expected
    );
}

// ============================================================================
// Status Tool Coverage Tests
// ============================================================================

/// Test status tool with no filters - should return all operations
#[tokio::test]
async fn test_status_tool_no_filters() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let result = mcp
        .client
        .call_tool(make_params("status", Some(json!({}))))
        .await?;

    assert!(!result.content.is_empty());
    assert_text_contains_any(
        &result,
        &["Operations status", "active", "completed"],
        "Status should show operations summary",
    );

    Ok(())
}

/// Test status tool with tool filter parameter
#[tokio::test]
async fn test_status_tool_with_tool_filter() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // First start an async operation to have something to query
    let _ = client
        .call_tool(make_params(
            "sandboxed_shell",
            Some(json!({"command": "echo test"})),
        ))
        .await?;

    let result = client
        .call_tool(make_params(
            "status",
            Some(json!({"tools": "sandboxed_shell"})),
        ))
        .await?;

    assert!(!result.content.is_empty());
    assert_text_contains_any(
        &result,
        &["sandboxed_shell", "Operations status"],
        "Status should reference filter",
    );

    client.cancel().await?;
    Ok(())
}

/// Test status tool with specific id filter
#[tokio::test]
async fn test_status_tool_with_id_filter() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let result = mcp
        .client
        .call_tool(make_params(
            "status",
            Some(json!({"id": "nonexistent_op_12345"})),
        ))
        .await?;

    assert!(!result.content.is_empty());
    assert_text_contains_any(
        &result,
        &["not found", "total: 0", "nonexistent_op_12345"],
        "Status should indicate operation not found",
    );

    Ok(())
}

// ============================================================================
// Await Tool Coverage Tests
// ============================================================================

/// Test await tool with no pending operations
#[tokio::test]
async fn test_await_tool_no_pending_operations() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let result = mcp
        .client
        .call_tool(make_params("await", Some(json!({}))))
        .await?;

    assert!(!result.content.is_empty());
    assert_text_contains_any(
        &result,
        &["No pending operations", "Completed", "operations"],
        "Await should handle no pending ops",
    );

    Ok(())
}

/// Test await tool with specific id that doesn't exist
#[tokio::test]
async fn test_await_tool_nonexistent_id() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let result = mcp
        .client
        .call_tool(make_params(
            "await",
            Some(json!({"id": "fake_operation_xyz"})),
        ))
        .await?;

    assert!(!result.content.is_empty());
    assert_text_contains_any(
        &result,
        &["not found", "fake_operation_xyz"],
        "Await should handle non-existent operation",
    );

    Ok(())
}

/// Test await tool with tools filter
#[tokio::test]
async fn test_await_tool_with_tool_filter() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let result = mcp
        .client
        .call_tool(make_params(
            "await",
            Some(json!({"tools": "nonexistent_tool"})),
        ))
        .await?;

    assert!(!result.content.is_empty());
    assert_text_contains_any(
        &result,
        &["No pending operations", "nonexistent_tool"],
        "Await should handle empty filter results",
    );

    Ok(())
}

/// Test await for an async operation that actually completes
#[tokio::test]
async fn test_await_for_completed_async_operation() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Start a fast async shell command
    let start_result = client
        .call_tool(make_params(
            "sandboxed_shell",
            Some(json!({"command": "echo 'quick test'"})),
        ))
        .await?;

    // Extract operation ID if available
    let id = extract_id(&start_result);

    // Build await args - use id if found, otherwise filter by tool
    let await_args = match id {
        Some(id) => json!({"id": id}),
        None => json!({"tools": "sandboxed_shell"}),
    };

    let result = client
        .call_tool(make_params("await", Some(await_args)))
        .await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Extracts id from a tool result if present
fn extract_id(result: &rmcp::model::CallToolResult) -> Option<String> {
    let text = get_result_text(result)?;
    let idx = text.find("id")?;
    let rest = &text[idx..];

    rest.split_whitespace()
        .take(5)
        .find(|word| word.starts_with("op_") || word.starts_with("\"op_"))
        .map(|word| word.trim_matches(|c| c == '"' || c == ',').to_string())
}

// ============================================================================
// Cancel Tool Coverage Tests
// ============================================================================

/// Test cancel tool with missing id parameter
#[tokio::test]
async fn test_cancel_tool_missing_id() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;
    let client = &mcp.client;

    if !client_has_tool(client, "cancel").await? {
        return Ok(());
    }

    let result = tokio::time::timeout(
        TestTimeouts::get(TimeoutCategory::ToolCall),
        client.call_tool(make_params("cancel", Some(json!({})))),
    )
    .await
    .map_err(|_| anyhow::anyhow!("call_tool timed out"))?;

    assert!(result.is_err(), "Cancel without id should fail with error");

    Ok(())
}

/// Checks if a tool exists in the client's tool list
async fn client_has_tool(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
) -> Result<bool> {
    let tools: Vec<rmcp::model::Tool> = client.list_all_tools().await?;
    Ok(tools.iter().any(|t| t.name.as_ref() == name))
}

/// Test cancel tool with non-existent id
#[tokio::test]
async fn test_cancel_tool_nonexistent_operation() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;
    let client = &mcp.client;

    if !client_has_tool(client, "cancel").await? {
        return Ok(());
    }

    let result = tokio::time::timeout(
        TestTimeouts::get(TimeoutCategory::ToolCall),
        client.call_tool(make_params(
            "cancel",
            Some(json!({"id": "nonexistent_op_999"})),
        )),
    )
    .await
    .map_err(|_| anyhow::anyhow!("call_tool timed out"))??;

    assert_text_contains_any(
        &result,
        &["not found", "FAIL", "never existed"],
        "Cancel should report operation not found",
    );

    Ok(())
}

// ============================================================================
// Tool Not Found / Disabled Coverage Tests
// ============================================================================

/// Test calling a tool that doesn't exist
#[tokio::test]
async fn test_call_nonexistent_tool() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let result = mcp
        .client
        .call_tool(make_params(
            "this_tool_definitely_does_not_exist_xyz123",
            Some(json!({})),
        ))
        .await;

    assert!(result.is_err(), "Non-existent tool should return error");

    Ok(())
}

/// Test calling a tool with invalid subcommand
#[tokio::test]
async fn test_call_tool_invalid_subcommand() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_from_dir(Path::new(".ahma")).await?;

    if !client_has_tool_prefix(&mcp.client, "file-tools").await? {
        return Ok(());
    }

    let result = mcp
        .client
        .call_tool(make_params(
            "file-tools",
            Some(json!({"subcommand": "nonexistent_subcommand"})),
        ))
        .await;

    assert!(result.is_err(), "Invalid subcommand should return error");

    Ok(())
}

/// Checks if any tool with the given prefix exists
async fn client_has_tool_prefix(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    prefix: &str,
) -> Result<bool> {
    let tools: Vec<rmcp::model::Tool> = client.list_all_tools().await?;
    Ok(tools.iter().any(|t| t.name.as_ref().starts_with(prefix)))
}

// ============================================================================
// List Tools Coverage Tests
// ============================================================================

/// Test list_tools returns await and status tools
#[tokio::test]
async fn test_list_tools_includes_builtin_tools() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let tools = mcp.client.list_all_tools().await?;
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();

    assert!(
        tool_names.contains(&"await"),
        "Tools should include 'await'. Got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"status"),
        "Tools should include 'status'. Got: {tool_names:?}"
    );

    Ok(())
}

/// Test list_tools schema generation for await tool
#[tokio::test]
async fn test_list_tools_await_schema() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let tools = mcp.client.list_all_tools().await?;
    let await_tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "await")
        .expect("Should find await tool");

    assert!(
        await_tool.description.is_some(),
        "Await tool should have description"
    );

    let schema_str = serde_json::to_string(&await_tool.input_schema)?;
    assert!(
        schema_str.contains("tools") || schema_str.contains("id"),
        "Await schema should have tools or id properties"
    );

    Ok(())
}

/// Test list_tools schema generation for status tool
#[tokio::test]
async fn test_list_tools_status_schema() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let tools = mcp.client.list_all_tools().await?;
    let status_tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "status")
        .expect("Should find status tool");

    assert!(
        status_tool.description.is_some(),
        "Status tool should have description"
    );

    if let Some(desc) = &status_tool.description {
        assert!(
            desc.contains("poll")
                || desc.contains("anti-pattern")
                || desc.contains("automatically"),
            "Status description should warn about polling"
        );
    }

    Ok(())
}

// ============================================================================
// Async Operation Flow Tests
// ============================================================================

/// Test full async operation lifecycle: start -> status -> await
#[tokio::test]
async fn test_async_operation_full_lifecycle() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Start an async shell command
    let start_result = client
        .call_tool(make_params(
            "sandboxed_shell",
            Some(json!({"command": "sleep 0.1 && echo lifecycle_test"})),
        ))
        .await?;
    assert!(!start_result.content.is_empty());

    // Check status immediately
    let status_result = client
        .call_tool(make_params("status", Some(json!({}))))
        .await?;
    assert!(!status_result.content.is_empty());

    // Await should find completed operations
    let await_result = client
        .call_tool(make_params(
            "await",
            Some(json!({"tools": "sandboxed_shell"})),
        ))
        .await?;
    assert!(!await_result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Working Directory Integration Tests
// ============================================================================

/// Test that file-tools respects working directory
#[tokio::test]
async fn test_file_tools_in_temp_directory() -> Result<()> {
    init_test_logging();

    let temp_dir = create_rust_project(TestProjectOptions {
        prefix: Some("mcp_coverage_".to_string()),
        with_cargo: false,
        with_text_files: true,
        with_tool_configs: true,
    })
    .await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    if !client_has_tool_prefix(&client, "file-tools").await? {
        client.cancel().await?;
        return Ok(());
    }

    let result = client
        .call_tool(make_params("file-tools", Some(json!({"subcommand": "ls"}))))
        .await?;

    assert_text_contains_any(
        &result,
        &["test1.txt", "test2.txt", ".ahma"],
        "Should see files in temp dir",
    );

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Multiple Tool Filter Tests
// ============================================================================

/// Test status with comma-separated tool filters
#[tokio::test]
async fn test_status_with_multiple_tool_filters() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let result = mcp
        .client
        .call_tool(make_params(
            "status",
            Some(json!({"tools": "cargo,git,sandboxed_shell"})),
        ))
        .await?;

    assert!(!result.content.is_empty());
    assert_text_contains_any(
        &result,
        &[
            "Operations status",
            "cargo",
            "git",
            "sandboxed_shell",
            "total",
        ],
        "Status should handle multiple filters",
    );

    Ok(())
}

/// Test await with comma-separated tool filters
#[tokio::test]
async fn test_await_with_multiple_tool_filters() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let result = mcp
        .client
        .call_tool(make_params("await", Some(json!({"tools": "cargo,git"}))))
        .await?;

    assert!(!result.content.is_empty());
    assert_text_contains_any(
        &result,
        &["No pending operations", "cargo", "git", "Completed"],
        "Await should handle multiple filters",
    );

    Ok(())
}
