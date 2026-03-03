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
use std::borrow::Cow;
use std::time::Duration;
use tokio::fs;

/// Setup test tools directory with basic tools
async fn setup_test_env() -> Result<tempfile::TempDir> {
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;
    Ok(temp_dir)
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

    // Call status with a filter that matches nothing
    let params = CallToolRequestParams {
        name: Cow::Borrowed("status"),
        arguments: Some(
            json!({"tools": "nonexistent_tool_xyz"})
                .as_object()
                .unwrap()
                .clone(),
        ),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await?;
    assert!(!result.is_error.unwrap_or(false));

    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

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

    let params = CallToolRequestParams {
        name: Cow::Borrowed("status"),
        arguments: Some(json!({"id": "op_999999"}).as_object().unwrap().clone()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await?;
    assert!(!result.is_error.unwrap_or(false));

    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

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

    let params = CallToolRequestParams {
        name: Cow::Borrowed("cancel"),
        arguments: Some(json!({}).as_object().unwrap().clone()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await;
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

    let params = CallToolRequestParams {
        name: Cow::Borrowed("cancel"),
        arguments: Some(json!({"id": "op_999999"}).as_object().unwrap().clone()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await?;
    // Cancel on non-existent ID usually returns success with a message saying it wasn't found or already done
    // This depends on implementation details, but based on code it returns successfully with a message.
    assert!(!result.is_error.unwrap_or(false));

    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

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
    let start_params = CallToolRequestParams {
        name: Cow::Borrowed("sandboxed_shell"),
        arguments: Some(
            json!({
                "command": "sleep 30",
                "execution_mode": "AsyncResultPush"
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        task: None,
        meta: None,
    };

    let start_result = client.call_tool(start_params).await?;
    let start_text: String = start_result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    // Extract operation ID (format: "Asynchronous operation started with ID: op_X...")
    let op_id = start_text
        .split("ID: ")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .ok_or_else(|| anyhow::anyhow!("Could not extract op ID from: {}", start_text))?;

    // Cancel it with a reason
    let cancel_params = CallToolRequestParams {
        name: Cow::Borrowed("cancel"),
        arguments: Some(
            json!({
                "id": op_id,
                "reason": "Test cancellation reason"
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        task: None,
        meta: None,
    };

    let cancel_result = client.call_tool(cancel_params).await?;
    assert!(!cancel_result.is_error.unwrap_or(false));

    let cancel_text: String = cancel_result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

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

    let params = CallToolRequestParams {
        name: Cow::Borrowed("sandboxed_shell"),
        arguments: Some(json!({}).as_object().unwrap().clone()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await;
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
    let sync_params = CallToolRequestParams {
        name: Cow::Borrowed("sandboxed_shell"),
        arguments: Some(
            json!({
                "command": "echo sync",
                "execution_mode": "Synchronous"
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        task: None,
        meta: None,
    };
    let sync_result = client.call_tool(sync_params).await?;
    let sync_text: String = sync_result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
    assert!(sync_text.contains("sync"));
    assert!(!sync_text.contains("ID: op_")); // Sync should NOT return op ID

    // 2. Explicit AsyncResultPush
    let async_params = CallToolRequestParams {
        name: Cow::Borrowed("sandboxed_shell"),
        arguments: Some(
            json!({
                "command": "echo async",
                "execution_mode": "AsyncResultPush"
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        task: None,
        meta: None,
    };
    let async_result = client.call_tool(async_params).await?;
    let async_text: String = async_result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
    // With automatic async, fast echo may return inline result instead of op ID
    assert!(
        async_text.contains("ID: op_") || async_text.contains("async"),
        "Async should return op ID or inline result. Got: {}",
        async_text
    );

    // 3. Invalid mode (should fallback to Async)
    let invalid_params = CallToolRequestParams {
        name: Cow::Borrowed("sandboxed_shell"),
        arguments: Some(
            json!({
                "command": "echo fallback",
                "execution_mode": "InvalidMode"
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        task: None,
        meta: None,
    };
    let invalid_result = client.call_tool(invalid_params).await?;
    let invalid_text: String = invalid_result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
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
    let params = CallToolRequestParams {
        name: Cow::Borrowed("sandboxed_shell"),
        arguments: Some(
            json!({
                "command": "sleep 2",
                "timeout_seconds": 1,
                "execution_mode": "Synchronous"
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await;
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
    let params = CallToolRequestParams {
        name: Cow::Borrowed("await"),
        arguments: Some(json!({}).as_object().unwrap().clone()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await?;
    let duration = start.elapsed();

    assert!(!result.is_error.unwrap_or(false));

    // Should return very quickly (e.g., < 100ms) since nothing to wait for
    assert!(duration < Duration::from_secs(1));

    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    assert!(text.contains("No pending operations to await for."));

    client.cancel().await?;
    Ok(())
}
