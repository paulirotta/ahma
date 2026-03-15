//! Automatic Async Integration Tests
//!
//! Tests that verify the "automatic async" feature: when an async operation
//! completes within `AUTOMATIC_ASYNC_TIMEOUT_SECS` (5 seconds), the result
//! is returned inline instead of requiring a separate `await` call.

use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use tokio::fs;

/// Setup test tools directory with async tool configurations
async fn setup_automatic_async_test_tools() -> Result<tempfile::TempDir> {
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    // Asynchronous tool (fast command - should complete within automatic async window)
    let async_tool = r#"
{
    "name": "auto_async_echo",
    "description": "Fast async echo (should complete within automatic async window)",
    "command": "echo",
    "timeout_seconds": 30,
    "synchronous": false,
    "enabled": true,
    "subcommand": [
        {
            "name": "default",
            "description": "Echo asynchronously",
            "positional_args": [
                {
                    "name": "message",
                    "type": "string",
                    "description": "Message",
                    "required": false
                }
            ]
        }
    ]
}
"#;
    fs::write(tools_dir.join("auto_async_echo.json"), async_tool).await?;

    Ok(temp_dir)
}

// ============================================================================
// Test: Fast command returns result inline (automatic async)
// ============================================================================

/// A fast `echo` command should complete within the 5-second automatic async
/// window and return the actual output inline, not an operation ID.
#[tokio::test]
async fn test_automatic_async_fast_shell_returns_inline() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_automatic_async_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // Don't set execution_mode — let it default to AsyncResultPush
    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        json!({
            "command": "echo 'automatic_async_test_marker'"
        })
        .as_object()
        .unwrap()
        .clone(),
    );

    let result = client.call_tool(params).await?;
    assert!(
        !result.is_error.unwrap_or(false),
        "Shell command should succeed"
    );

    let all_text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    // Fast command should return inline result with the actual output
    assert!(
        all_text.contains("automatic_async_test_marker"),
        "Fast async command should return output inline via automatic async. Got: {}",
        all_text
    );

    // Should NOT contain async operation ID since it completed fast enough
    assert!(
        !all_text.contains("AHMA ID:"),
        "Fast command should NOT return async ID. Got: {}",
        all_text
    );

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Slow command falls back to async behavior
// ============================================================================

/// A slow command (sleep 30) should exceed the 5-second automatic async window
/// and return the traditional async operation ID.
#[tokio::test]
async fn test_automatic_async_slow_command_returns_async_id() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_automatic_async_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let start = std::time::Instant::now();

    let command_str = if cfg!(windows) {
        "Start-Sleep -Seconds 30; Write-Output done"
    } else {
        "sleep 30 && echo done"
    };

    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        json!({
            "command": command_str
        })
        .as_object()
        .unwrap()
        .clone(),
    );

    let result = client.call_tool(params).await?;
    let duration = start.elapsed();

    assert!(
        !result.is_error.unwrap_or(false),
        "Async shell call should succeed"
    );

    let all_text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    // Slow command should return async operation ID after automatic async timeout
    assert!(
        all_text.contains("AHMA ID: op_"),
        "Slow command should return async operation ID. Got: {}",
        all_text
    );

    // Should have waited approximately AUTOMATIC_ASYNC_TIMEOUT_SECS (5s) before returning
    assert!(
        duration.as_secs() >= 4,
        "Should have waited ~5 seconds before returning async ID. Actual: {:.1}s",
        duration.as_secs_f64()
    );
    assert!(
        duration.as_secs() <= 10,
        "Should not have waited too long. Actual: {:.1}s",
        duration.as_secs_f64()
    );

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Fast configured async tool returns inline result
// ============================================================================

/// A fast async tool (configured via JSON, not sandboxed_shell) should also
/// benefit from automatic async and return inline results.
#[tokio::test]
async fn test_automatic_async_configured_tool_returns_inline() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_automatic_async_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let params = CallToolRequestParams::new("auto_async_echo").with_arguments(
        json!({"message": "configured_tool_inline_result"})
            .as_object()
            .unwrap()
            .clone(),
    );

    let result = client.call_tool(params).await?;
    assert!(
        !result.is_error.unwrap_or(false),
        "Configured async tool should succeed"
    );

    let all_text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    // Fast configured tool should return inline result
    assert!(
        all_text.contains("configured_tool_inline_result"),
        "Fast configured async tool should return output inline. Got: {}",
        all_text
    );

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Synchronous mode is not affected by automatic async
// ============================================================================

/// Synchronous execution should work exactly as before (no automatic async delay).
#[tokio::test]
async fn test_automatic_async_does_not_affect_sync_mode() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_automatic_async_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let start = std::time::Instant::now();

    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        json!({
            "command": "echo sync_marker",
            "execution_mode": "Synchronous"
        })
        .as_object()
        .unwrap()
        .clone(),
    );

    let result = client.call_tool(params).await?;
    let duration = start.elapsed();

    assert!(
        !result.is_error.unwrap_or(false),
        "Sync command should succeed"
    );

    let all_text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    // Sync should return actual output immediately
    assert!(
        all_text.contains("sync_marker"),
        "Sync should return actual output. Got: {}",
        all_text
    );

    // Should complete quickly (not wait for automatic async timeout)
    assert!(
        duration.as_secs() < 3,
        "Sync mode should not be delayed by automatic async. Duration: {:.1}s",
        duration.as_secs_f64()
    );

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Multiple fast commands benefit from automatic async
// ============================================================================

/// Multiple sequential fast commands should all complete within their automatic
/// async windows and return results inline.
#[tokio::test]
async fn test_automatic_async_multiple_fast_commands() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_automatic_async_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    for i in 1..=3 {
        let marker = format!("fast_cmd_{}", i);
        let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
            json!({
                "command": format!("echo '{}'", marker)
            })
            .as_object()
            .unwrap()
            .clone(),
        );

        let result = client.call_tool(params).await?;
        assert!(
            !result.is_error.unwrap_or(false),
            "Command {} should succeed",
            i
        );

        let all_text: String = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect();

        assert!(
            all_text.contains(&marker),
            "Command {} should return inline result. Got: {}",
            i,
            all_text
        );
    }

    client.cancel().await?;
    Ok(())
}
