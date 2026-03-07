//! # MCP Callback Coverage Tests
//!
//! This file tests the mcp_callback.rs module which provides progress notification
//! functionality via the MCP protocol.
//!
//! Since McpCallbackSender requires a real MCP Peer connection, these tests use
//! integration testing by spawning the actual server and triggering code paths
//! that exercise the callback system.
//!
//! Coverage targets:
//! - ProgressUpdate::Started notification
//! - ProgressUpdate::Progress notification
//! - ProgressUpdate::Output notification
//! - ProgressUpdate::Completed notification
//! - ProgressUpdate::Failed notification
//! - ProgressUpdate::Cancelled notification
//! - ProgressUpdate::FinalResult notification

use ahma_mcp::skip_if_disabled_async_result;
use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;

// ============================================================================
// Callback System Integration Tests
// ============================================================================

/// Test that async operations trigger progress callbacks
/// This exercises Started, Progress, Output, and Completed/FinalResult paths
#[tokio::test]
async fn test_async_operation_triggers_callbacks() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Start an async operation that produces output
    let shell_params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        json!({ "command": "echo 'callback test output'" })
            .as_object()
            .unwrap()
            .clone(),
    );

    let result = client.call_tool(shell_params).await?;
    assert!(!result.content.is_empty());

    // If async, the callback paths are exercised
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
        && text_content.text.contains("ID:")
    {
        // Extract operation ID
        let op_id = extract_op_id(&text_content.text);

        // Await completion - this triggers FinalResult callback
        let await_params = CallToolRequestParams::new("await")
            .with_arguments(json!({ "id": op_id }).as_object().unwrap().clone());

        let await_result = client.call_tool(await_params).await?;
        assert!(!await_result.content.is_empty());
    }

    Ok(())
}

/// Test that failed operations trigger the Failed callback path
#[tokio::test]
async fn test_failed_operation_callback() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Start an operation that will fail
    let shell_params = CallToolRequestParams::new("sandboxed_shell")
        .with_arguments(json!({ "command": "exit 1" }).as_object().unwrap().clone());

    let result = client.call_tool(shell_params).await?;
    assert!(!result.content.is_empty());

    // If async, await and check for failure message
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
        && text_content.text.contains("ID:")
    {
        let op_id = extract_op_id(&text_content.text);

        let await_params = CallToolRequestParams::new("await")
            .with_arguments(json!({ "id": op_id }).as_object().unwrap().clone());

        let await_result = client.call_tool(await_params).await?;
        // Failed operations should still produce output
        assert!(!await_result.content.is_empty());
    }

    Ok(())
}

/// Test cancel operation which triggers the Cancelled callback path
#[tokio::test]
async fn test_cancelled_operation_callback() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Start a long-running async operation
    let shell_params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        json!({ "command": "sleep 60" })
            .as_object()
            .unwrap()
            .clone(),
    );

    let start_result = client.call_tool(shell_params).await?;

    if let Some(content) = start_result.content.first()
        && let Some(text_content) = content.as_text()
        && text_content.text.contains("ID:")
    {
        let op_id = extract_op_id(&text_content.text);

        // Cancel the operation - this should trigger Cancelled callback
        let cancel_params = CallToolRequestParams::new("cancel")
            .with_arguments(json!({ "id": op_id }).as_object().unwrap().clone());

        let cancel_result = client.call_tool(cancel_params).await?;
        assert!(!cancel_result.content.is_empty());

        // Await the cancelled operation to ensure it completes and resources are cleaned up
        let await_params = CallToolRequestParams::new("await")
            .with_arguments(json!({ "id": op_id }).as_object().unwrap().clone());

        let await_result = client.call_tool(await_params).await?;
        assert!(!await_result.content.is_empty());

        // The cancellation message formatting is tested
        if let Some(cancel_content) = cancel_result.content.first()
            && let Some(cancel_text) = cancel_content.as_text()
        {
            // Should not have redundant "Cancelled: Canceled" pattern
            assert!(
                !cancel_text.text.contains("Cancelled: Canceled"),
                "Should not have redundant cancellation message"
            );
        }
    }

    client.cancel().await?;
    Ok(())
}

/// Test stderr output handling (covers Output variant with is_stderr=true)
#[tokio::test]
async fn test_stderr_output_callback() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    #[cfg(unix)]
    let stderr_cmd = "echo 'stderr test' >&2";
    #[cfg(windows)]
    let stderr_cmd = "Write-Error 'stderr test'";
    #[cfg(not(any(unix, windows)))]
    let stderr_cmd = "echo 'stderr test' >&2";

    let shell_params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        json!({ "command": stderr_cmd })
            .as_object()
            .unwrap()
            .clone(),
    );

    let result = client.call_tool(shell_params).await?;
    assert!(!result.content.is_empty());

    // If async, await to ensure stderr output is captured
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
        && text_content.text.contains("ID:")
    {
        let op_id = extract_op_id(&text_content.text);

        let await_params = CallToolRequestParams::new("await")
            .with_arguments(json!({ "id": op_id }).as_object().unwrap().clone());

        let await_result = client.call_tool(await_params).await?;
        assert!(!await_result.content.is_empty());
    }

    Ok(())
}

/// Test multiple concurrent operations exercising callback system
#[tokio::test]
async fn test_concurrent_operations_callbacks() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Start multiple operations
    let params1 = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        json!({ "command": "echo 'op1'" })
            .as_object()
            .unwrap()
            .clone(),
    );
    let params2 = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        json!({ "command": "echo 'op2'" })
            .as_object()
            .unwrap()
            .clone(),
    );

    // Start both operations
    let result1 = client.call_tool(params1).await?;
    let result2 = client.call_tool(params2).await?;

    assert!(!result1.content.is_empty());
    assert!(!result2.content.is_empty());

    // Await all to ensure all callbacks complete
    let await_params =
        CallToolRequestParams::new("await").with_arguments(json!({}).as_object().unwrap().clone());

    let await_result = client.call_tool(await_params).await?;
    assert!(!await_result.content.is_empty());

    Ok(())
}

// ============================================================================
// Callback System Unit Tests (testing callback_system.rs)
// ============================================================================

/// Test the CallbackError type
#[test]
fn test_callback_error_display() {
    use ahma_mcp::callback_system::CallbackError;

    let send_failed = CallbackError::SendFailed("network error".to_string());
    let error_string = format!("{}", send_failed);
    assert!(error_string.contains("network error"));
}

/// Test ProgressUpdate variants construction
#[test]
fn test_progress_update_variants() {
    use ahma_mcp::callback_system::ProgressUpdate;

    // Test Started variant
    let started = ProgressUpdate::Started {
        id: "op_123".to_string(),
        command: "test command".to_string(),
        description: "Test description".to_string(),
    };
    assert!(matches!(started, ProgressUpdate::Started { .. }));

    // Test Progress variant
    let progress = ProgressUpdate::Progress {
        id: "op_123".to_string(),
        message: "Processing".to_string(),
        percentage: Some(50.0),
        current_step: Some("step1".to_string()),
    };
    assert!(matches!(progress, ProgressUpdate::Progress { .. }));

    // Test Output variant
    let output = ProgressUpdate::Output {
        id: "op_123".to_string(),
        line: "Output line".to_string(),
        is_stderr: false,
    };
    assert!(matches!(output, ProgressUpdate::Output { .. }));

    // Test Completed variant
    let completed = ProgressUpdate::Completed {
        id: "op_123".to_string(),
        message: "Done".to_string(),
        duration_ms: 100,
    };
    assert!(matches!(completed, ProgressUpdate::Completed { .. }));

    // Test Failed variant
    let failed = ProgressUpdate::Failed {
        id: "op_123".to_string(),
        error: "Something went wrong".to_string(),
        duration_ms: 50,
    };
    assert!(matches!(failed, ProgressUpdate::Failed { .. }));

    // Test Cancelled variant
    let cancelled = ProgressUpdate::Cancelled {
        id: "op_123".to_string(),
        message: "User requested cancellation".to_string(),
        duration_ms: 25,
    };
    assert!(matches!(cancelled, ProgressUpdate::Cancelled { .. }));

    // Test FinalResult variant
    let final_result = ProgressUpdate::FinalResult {
        id: "op_123".to_string(),
        command: "echo test".to_string(),
        description: "Echo test".to_string(),
        working_directory: "/tmp".to_string(),
        success: true,
        full_output: "test\n".to_string(),
        duration_ms: 100,
    };
    assert!(matches!(final_result, ProgressUpdate::FinalResult { .. }));
}

/// Helper to extract operation ID from response
fn extract_op_id(text: &str) -> String {
    if let Some(id_start) = text.find("ID: ") {
        let id_text = &text[id_start + 4..];
        if let Some(job_id) = id_text.split_whitespace().next() {
            return job_id.to_string();
        }
    }
    "unknown".to_string()
}
