//! MCP Service Edge Cases Integration Tests
//!
//! Tests for the mcp_service module covering edge cases and error conditions for:
//! 1. Status tool (filtering, invalid IDs)
//! 2. Cancel tool (permissions, invalid IDs)
//! 3. Sandboxed Shell (validation, timeouts, execution modes)
//! 4. Await tool (empty states)

use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::time::Duration;
use tokio::fs;

/// Setup test tools directory with basic tools
async fn setup_test_env() -> Result<tempfile::TempDir> {
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;
    Ok(temp_dir)
}

async fn call_test_tool(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    name: &str,
    args: serde_json::Value,
) -> Result<rmcp::model::CallToolResult> {
    let mut params = CallToolRequestParams::new(name.to_string());
    if let Some(arguments) = args.as_object().cloned() {
        params = params.with_arguments(arguments);
    }

    Ok(client.call_tool(params).await?)
}

fn assert_success_and_get_text(result: &rmcp::model::CallToolResult) -> String {
    assert!(!result.is_error.unwrap_or(false));
    result_text(result)
}

// ============================================================================
// Test: Status Tool Edge Cases
// ============================================================================

/// Test status tool filtering by non-existent tool name
#[tokio::test]
async fn test_status_filter_nonexistent_tool() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_test_env().await?;
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let result =
        call_test_tool(&client, "status", json!({"tools": "nonexistent_tool_xyz"})).await?;
    let text = assert_success_and_get_text(&result);

    // Should indicate 0 active/completed for that filter
    assert!(text.contains("0 active"));
    assert!(text.contains("0 completed"));
    assert!(text.contains("total: 0"));

    client.cancel().await?;
    Ok(())
}

/// Test status tool query for non-existent operation ID
#[tokio::test]
async fn test_status_nonexistent_id() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_test_env().await?;
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let result = call_test_tool(&client, "status", json!({"id": "op_999999"})).await?;
    let text = assert_success_and_get_text(&result);

    assert!(text.contains("not found"));

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Cancel Tool Edge Cases
// ============================================================================

/// Test cancel missing id
#[tokio::test]
async fn test_cancel_missing_id() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_test_env().await?;
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let result = call_test_tool(&client, "cancel", json!({})).await;
    assert_required_param_error(result, "required");

    client.cancel().await?;
    Ok(())
}

fn assert_required_param_error<E: std::fmt::Debug>(
    result: Result<rmcp::model::CallToolResult, E>,
    keyword: &str,
) {
    if let Err(e) = result {
        let msg = format!("{:?}", e);
        assert!(
            msg.contains(keyword) || msg.contains("missing"),
            "Expected error validating '{}' or 'missing', got: {}",
            keyword,
            msg
        );
    } else if let Ok(r) = result {
        assert!(r.is_error.unwrap_or(false));
    }
}

/// Test cancel non-existent operation
#[tokio::test]
async fn test_cancel_nonexistent_operation() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_test_env().await?;
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let result = call_test_tool(&client, "cancel", json!({"id": "op_999999"})).await?;
    let text = assert_success_and_get_text(&result);

    assert!(text.contains("not found") || text.contains("completed"));

    client.cancel().await?;
    Ok(())
}

/// Test cancel with explicit reason
#[tokio::test]
async fn test_cancel_with_reason() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_test_env().await?;
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // First start a long running operation (must exceed AUTOMATIC_ASYNC_TIMEOUT_SECS)
    let start_result = call_test_tool(
        &client,
        "sandboxed_shell",
        json!({
            "command": "sleep 30",
            "execution_mode": "AsyncResultPush"
        }),
    )
    .await?;

    let start_text = result_text(&start_result);

    // Extract operation ID (format: "Asynchronous operation started with ID: op_X...")
    let op_id = start_text
        .split("ID: ")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .ok_or_else(|| anyhow::anyhow!("Could not extract op ID from: {}", start_text))?;

    // Cancel it with a reason
    let cancel_result = call_test_tool(
        &client,
        "cancel",
        json!({
            "id": op_id,
            "reason": "Test cancellation reason"
        }),
    )
    .await?;

    let cancel_text = assert_success_and_get_text(&cancel_result);

    assert!(cancel_text.contains("cancelled successfully"));
    assert!(cancel_text.contains("Test cancellation reason"));

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Sandboxed Shell Edge Cases
// ============================================================================

/// Test shell missing command
#[tokio::test]
async fn test_shell_missing_command() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_test_env().await?;
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let result = call_test_tool(&client, "sandboxed_shell", json!({})).await;
    assert_required_param_error(result, "required");

    client.cancel().await?;
    Ok(())
}

/// Test shell explicit execution modes
#[tokio::test]
async fn test_shell_explicit_execution_modes() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_test_env().await?;
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // 1. Explicit Synchronous
    let sync_result = call_test_tool(
        &client,
        "sandboxed_shell",
        json!({
            "command": "echo sync",
            "execution_mode": "Synchronous"
        }),
    )
    .await?;
    let sync_text = result_text(&sync_result);
    assert!(sync_text.contains("sync"));
    assert!(!sync_text.contains("ID: op_")); // Sync should NOT return op ID

    // 2. Explicit AsyncResultPush
    let async_result = call_test_tool(
        &client,
        "sandboxed_shell",
        json!({
            "command": "echo async",
            "execution_mode": "AsyncResultPush"
        }),
    )
    .await?;
    let async_text = result_text(&async_result);
    // With automatic async, fast echo may return inline result instead of op ID
    assert!(
        async_text.contains("ID: op_") || async_text.contains("async"),
        "Async should return op ID or inline result. Got: {}",
        async_text
    );

    // 3. Invalid mode (should fallback to Async)
    let invalid_result = call_test_tool(
        &client,
        "sandboxed_shell",
        json!({
            "command": "echo fallback",
            "execution_mode": "InvalidMode"
        }),
    )
    .await?;
    let invalid_text = result_text(&invalid_result);
    // With automatic async, fast echo may return inline result instead of op ID
    assert!(
        invalid_text.contains("ID: op_") || invalid_text.contains("fallback"),
        "Fallback async should return op ID or inline result. Got: {}",
        invalid_text
    );

    client.cancel().await?;
    Ok(())
}

/// Test shell timeout
#[tokio::test]
async fn test_shell_timeout() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_test_env().await?;
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // Run a command that sleeps for 2s with 1s timeout
    let result = call_test_tool(
        &client,
        "sandboxed_shell",
        json!({
            "command": "sleep 2",
            "timeout_seconds": 1,
            "execution_mode": "Synchronous"
        }),
    )
    .await;

    assert_timeout_error(result);

    client.cancel().await?;
    Ok(())
}

fn result_text(r: &rmcp::model::CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect()
}

fn assert_timeout_error<E: std::fmt::Debug>(result: Result<rmcp::model::CallToolResult, E>) {
    if let Err(e) = result {
        let msg = format!("{:?}", e);
        assert!(msg.contains("timeout") || msg.contains("timed out"));
    } else if let Ok(r) = result {
        if !r.is_error.unwrap_or(false) {
            panic!(
                "Expected timeout failure, but shell command succeeded. Output: {}",
                result_text(&r)
            );
        }
        let text = result_text(&r);
        assert!(text.contains("timeout") || text.contains("timed out") || text.contains("killed"));
    }
}

// ============================================================================
// Test: Await Tool Edge Cases
// ============================================================================

/// Test await when no operations are active
#[tokio::test]
async fn test_await_no_active_operations() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_test_env().await?;
    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let start = std::time::Instant::now();
    let result = call_test_tool(&client, "await", json!({})).await?;
    let duration = start.elapsed();

    let text = assert_success_and_get_text(&result);

    // Should return very quickly (e.g., < 100ms) since nothing to wait for
    assert!(duration < Duration::from_secs(1));
    assert!(text.contains("No pending operations to await for."));

    client.cancel().await?;
    Ok(())
}
