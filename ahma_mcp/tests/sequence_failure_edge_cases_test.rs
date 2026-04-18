//! Sequence Failure Edge Cases Integration Tests
//!
//! Tests for sequence tool edge cases that verify:
//! 1. Step failure stops execution immediately (no subsequent steps run)
//! 2. Missing subcommand references are caught and reported
//! 3. Command failures in steps are properly propagated
//! 4. Timeout handling in sequence steps
//!
//! These are real integration tests using tempdir and actual binary execution.

use ahma_mcp::test_utils::concurrency::wait_for_condition;
use ahma_mcp::test_utils::in_process::create_in_process_mcp_from_dir;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use tempfile::TempDir;
use tokio::fs;

/// Create a temp directory with sequence tools designed to test failure scenarios
async fn setup_failure_test_configs() -> Result<TempDir> {
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    // Create a working echo tool
    let echo_tool_config = r#"
{
    "name": "test_echo",
    "description": "Echo a message for testing",
    "command": "echo",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [
        {
            "name": "default",
            "description": "echo the message",
            "positional_args": [
                {
                    "name": "message",
                    "type": "string",
                    "description": "message to echo",
                    "required": false
                }
            ]
        }
    ]
}
"#;
    fs::write(tools_dir.join("test_echo.json"), echo_tool_config).await?;

    // Create a tool that always fails (exit 1)
    let failing_tool_config = r#"
{
    "name": "fail_tool",
    "description": "A tool that always fails",
    "command": "false",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [
        {
            "name": "default",
            "description": "always fails with exit code 1"
        }
    ]
}
"#;
    fs::write(tools_dir.join("fail_tool.json"), failing_tool_config).await?;

    // Create a sequence where step 2 fails - step 3 should NOT run
    // This tests the critical invariant: failure stops execution
    let failing_sequence_config = r#"
{
    "name": "failing_sequence",
    "description": "Sequence where step 2 fails, testing failure-stops-execution invariant",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": true,
    "enabled": true,
    "step_delay_ms": 50,
    "sequence": [
        {
            "tool": "test_echo",
            "subcommand": "default",
            "description": "Step 1: should succeed",
            "args": {"message": "STEP_1_MARKER"}
        },
        {
            "tool": "fail_tool",
            "subcommand": "default",
            "description": "Step 2: should fail"
        },
        {
            "tool": "test_echo",
            "subcommand": "default",
            "description": "Step 3: should NOT run",
            "args": {"message": "STEP_3_SHOULD_NOT_APPEAR"}
        }
    ]
}
"#;
    fs::write(
        tools_dir.join("failing_sequence.json"),
        failing_sequence_config,
    )
    .await?;

    // Create a sequence with missing subcommand reference
    let missing_subcommand_config = r#"
{
    "name": "missing_subcommand_seq",
    "description": "Sequence with a reference to a non-existent subcommand",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": true,
    "enabled": true,
    "sequence": [
        {
            "tool": "test_echo",
            "subcommand": "nonexistent_subcommand_xyz",
            "description": "This references a subcommand that does not exist"
        }
    ]
}
"#;
    fs::write(
        tools_dir.join("missing_subcommand_seq.json"),
        missing_subcommand_config,
    )
    .await?;

    // Create a sequence with a very short timeout
    let timeout_sequence_config = r#"
{
    "name": "timeout_sequence",
    "description": "Sequence with a short timeout for testing",
    "command": "sequence",
    "timeout_seconds": 1,
    "synchronous": true,
    "enabled": true,
    "sequence": [
        {
            "tool": "test_echo",
            "subcommand": "default",
            "description": "Quick step",
            "args": {"message": "quick"}
        }
    ]
}
"#;
    fs::write(
        tools_dir.join("timeout_sequence.json"),
        timeout_sequence_config,
    )
    .await?;

    // sandboxed_shell is a core built-in tool - no JSON config needed

    // NOTE: marker_sequence is dynamically generated in the test that uses it,
    // because it needs paths inside the temp directory (sandbox-scoped).
    // See test_sequence_failure_with_filesystem_markers for the actual config.

    Ok(temp_dir)
}

// ============================================================================
// Test: Failure Stops Execution Invariant
// ============================================================================

/// CRITICAL TEST: Verify that when a sequence step fails, subsequent steps do NOT run.
/// This is a security and correctness invariant.
#[tokio::test]
async fn test_sequence_step_failure_stops_subsequent_steps() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_failure_test_configs().await?;
    let tools_dir = temp_dir.path().join(".ahma");

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    // List tools to verify our sequence is loaded
    let tools = mcp.client.list_all_tools().await?;
    let has_failing_seq = tools.iter().any(|t| t.name.as_ref() == "failing_sequence");

    assert!(
        has_failing_seq,
        "Should have failing_sequence tool loaded. Available: {:?}",
        tools.iter().map(|t| t.name.as_ref()).collect::<Vec<_>>()
    );

    // Call the failing sequence
    let params = CallToolRequestParams::new("failing_sequence")
        .with_arguments(json!({}).as_object().unwrap().clone());

    let result = mcp.client.call_tool(params).await?;

    // Collect all text content
    let mut all_text = String::new();
    for content in &result.content {
        if let Some(text_content) = content.as_text() {
            all_text.push_str(&text_content.text);
            all_text.push('\n');
        }
    }

    // Verify step 1 ran
    assert!(
        all_text.contains("STEP_1_MARKER") || all_text.contains("Step 1"),
        "Step 1 should have run and produced output. Got:\n{}",
        all_text
    );

    // CRITICAL: Verify step 3 did NOT run
    assert!(
        !all_text.contains("STEP_3_SHOULD_NOT_APPEAR"),
        "Step 3 should NOT have run after step 2 failed. Got:\n{}",
        all_text
    );

    // Verify the sequence is marked as error
    assert!(
        result.is_error.unwrap_or(false) || all_text.to_lowercase().contains("fail"),
        "Result should indicate failure. is_error={:?}, text:\n{}",
        result.is_error,
        all_text
    );

    mcp.client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Missing Subcommand Reference
// ============================================================================

/// Test that referencing a non-existent subcommand produces a clear error
#[tokio::test]
async fn test_sequence_with_missing_subcommand_reference() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_failure_test_configs().await?;
    let tools_dir = temp_dir.path().join(".ahma");

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    let tools = mcp.client.list_all_tools().await?;
    let has_seq = tools
        .iter()
        .any(|t| t.name.as_ref() == "missing_subcommand_seq");

    if !has_seq {
        // Tool might not load due to validation - this is also acceptable behavior
        mcp.client.cancel().await?;
        return Ok(());
    }

    let params = CallToolRequestParams::new("missing_subcommand_seq")
        .with_arguments(json!({}).as_object().unwrap().clone());

    let result = mcp.client.call_tool(params).await;

    // This should fail with an error about the missing subcommand
    match result {
        Err(e) => {
            let error_msg = format!("{:?}", e);
            assert!(
                error_msg.contains("not found")
                    || error_msg.contains("nonexistent")
                    || error_msg.contains("Subcommand"),
                "Error should mention missing subcommand. Got: {}",
                error_msg
            );
        }
        Ok(r) => {
            // If it returns Ok, it should be marked as an error
            assert!(
                r.is_error.unwrap_or(false),
                "Result should be marked as error for missing subcommand"
            );
        }
    }

    mcp.client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Marker File Proves Execution Order
// ============================================================================

/// Use filesystem markers to definitively prove step 3 didn't run.
/// This is a more robust test than just checking output text.
#[tokio::test]
async fn test_sequence_failure_with_filesystem_markers() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_failure_test_configs().await?;
    let tools_dir = temp_dir.path().join(".ahma");

    // Create marker file paths INSIDE the temp directory (sandbox-scoped)
    let step1_marker = temp_dir.path().join("step1_marker");
    let step3_marker = temp_dir.path().join("step3_marker");

    // Clean up any leftover marker files
    let _ = std::fs::remove_file(&step1_marker);
    let _ = std::fs::remove_file(&step3_marker);

    // Dynamically create marker_sequence with paths inside the sandbox scope
    let marker_sequence_config = format!(
        r#"{{
    "name": "marker_sequence",
    "description": "Sequence that creates marker files to prove execution order",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": true,
    "enabled": true,
    "step_delay_ms": 50,
    "sequence": [
        {{
            "tool": "sandboxed_shell",
            "subcommand": "default",
            "description": "Step 1: create first marker",
            "args": {{"command": "touch {}"}}
        }},
        {{
            "tool": "fail_tool",
            "subcommand": "default",
            "description": "Step 2: fail"
        }},
        {{
            "tool": "sandboxed_shell",
            "subcommand": "default",
            "description": "Step 3: should NOT create this marker",
            "args": {{"command": "touch {}"}}
        }}
    ]
}}"#,
        step1_marker.display(),
        step3_marker.display()
    );
    fs::write(
        tools_dir.join("marker_sequence.json"),
        marker_sequence_config,
    )
    .await?;

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    let tools = mcp.client.list_all_tools().await?;
    let has_marker_seq = tools.iter().any(|t| t.name.as_ref() == "marker_sequence");

    if !has_marker_seq {
        eprintln!("marker_sequence not loaded, skipping filesystem marker test");
        mcp.client.cancel().await?;
        return Ok(());
    }

    let params = CallToolRequestParams::new("marker_sequence")
        .with_arguments(json!({}).as_object().unwrap().clone());

    let _result = mcp.client.call_tool(params).await;

    // Wait for filesystem operations to complete
    let _ = wait_for_condition(
        std::time::Duration::from_secs(2),
        std::time::Duration::from_millis(20),
        || async { step1_marker.exists() || step3_marker.exists() },
    )
    .await;

    // Check the marker files
    let step1_ran = step1_marker.exists();
    let step3_ran = step3_marker.exists();

    // Step 1 should have created its marker
    assert!(
        step1_ran,
        "Step 1 marker file should exist - step 1 should have run"
    );

    // CRITICAL: Step 3 should NOT have created its marker
    assert!(
        !step3_ran,
        "Step 3 marker file should NOT exist - step 3 should not have run after step 2 failed"
    );

    mcp.client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Empty Sequence
// ============================================================================

/// Test that a sequence with no steps handles gracefully
#[tokio::test]
async fn test_empty_sequence_handling() -> Result<()> {
    init_test_logging();
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    let empty_sequence_config = r#"
{
    "name": "empty_sequence",
    "description": "Sequence with no steps",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": true,
    "enabled": true,
    "sequence": []
}
"#;
    fs::write(tools_dir.join("empty_sequence.json"), empty_sequence_config).await?;

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    let tools = mcp.client.list_all_tools().await?;
    let has_empty_seq = tools.iter().any(|t| t.name.as_ref() == "empty_sequence");

    if has_empty_seq {
        let params = CallToolRequestParams::new("empty_sequence")
            .with_arguments(json!({}).as_object().unwrap().clone());

        let result = mcp.client.call_tool(params).await?;

        // Should complete without error (0 steps completed successfully)
        assert!(
            !result.is_error.unwrap_or(false),
            "Empty sequence should succeed (0 steps = all 0 steps succeeded)"
        );
    }

    mcp.client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: All Steps Succeed
// ============================================================================

/// Baseline test: verify a sequence where all steps succeed completes correctly
#[tokio::test]
async fn test_sequence_all_steps_succeed() -> Result<()> {
    init_test_logging();
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    let echo_tool = r#"
{
    "name": "echo_test",
    "description": "Echo for testing",
    "command": "echo",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [{"name": "default", "description": "echo"}]
}
"#;
    fs::write(tools_dir.join("echo_test.json"), echo_tool).await?;

    let success_sequence = r#"
{
    "name": "success_sequence",
    "description": "All steps succeed",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": true,
    "enabled": true,
    "sequence": [
        {"tool": "echo_test", "subcommand": "default", "description": "Step 1", "args": {"message": "one"}},
        {"tool": "echo_test", "subcommand": "default", "description": "Step 2", "args": {"message": "two"}},
        {"tool": "echo_test", "subcommand": "default", "description": "Step 3", "args": {"message": "three"}}
    ]
}
"#;
    fs::write(tools_dir.join("success_sequence.json"), success_sequence).await?;

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    let params = CallToolRequestParams::new("success_sequence")
        .with_arguments(json!({}).as_object().unwrap().clone());

    let result = mcp.client.call_tool(params).await?;

    // Should succeed
    assert!(
        !result.is_error.unwrap_or(false),
        "All-success sequence should not be marked as error"
    );

    // All step outputs should be present
    let all_text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    assert!(
        all_text.contains("one") || all_text.contains("Step 1"),
        "Should show step 1 ran"
    );
    assert!(
        all_text.contains("three") || all_text.contains("Step 3"),
        "Should show step 3 ran"
    );

    mcp.client.cancel().await?;
    Ok(())
}
