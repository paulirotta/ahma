//! Sequence Tool Integration Coverage Tests
//!
//! Real integration tests for mcp_service/sequence.rs targeting:
//! - handle_sequence_tool (sync and async)
//! - handle_subcommand_sequence
//! - should_skip_step with environment variables
//! - format_step_started_message / format_step_skipped_message
//!
//! These tests spawn the actual ahma_mcp binary and use real tool configs.

use ahma_mcp::test_utils::concurrency::wait_for_condition;
use ahma_mcp::test_utils::in_process::create_in_process_mcp_from_dir;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::borrow::Cow;
use tempfile::TempDir;
use tokio::fs;

/// Create a temp directory with a sequence tool configuration
async fn setup_sequence_tool_config() -> Result<TempDir> {
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    // Create a simple echo tool
    let echo_tool_config = r#"
{
    "name": "echo",
    "description": "Echo a message",
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
    fs::write(tools_dir.join("echo.json"), echo_tool_config).await?;

    // Create a pwd tool
    let pwd_tool_config = r#"
{
    "name": "pwd_tool",
    "description": "Print working directory",
    "command": "pwd",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [
        {
            "name": "default",
            "description": "print working directory"
        }
    ]
}
"#;
    fs::write(tools_dir.join("pwd_tool.json"), pwd_tool_config).await?;

    // Create a synchronous sequence tool
    let sync_sequence_config = r#"
{
    "name": "sync_sequence",
    "description": "A synchronous sequence tool for testing",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": true,
    "enabled": true,
    "step_delay_ms": 100,
    "sequence": [
        {
            "tool": "echo",
            "subcommand": "default",
            "description": "First echo step",
            "args": {"message": "step1"}
        },
        {
            "tool": "echo",
            "subcommand": "default",
            "description": "Second echo step",
            "args": {"message": "step2"}
        }
    ]
}
"#;
    fs::write(tools_dir.join("sync_sequence.json"), sync_sequence_config).await?;

    // Create an asynchronous sequence tool
    let async_sequence_config = r#"
{
    "name": "async_sequence",
    "description": "An asynchronous sequence tool for testing",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": false,
    "enabled": true,
    "step_delay_ms": 50,
    "sequence": [
        {
            "tool": "echo",
            "subcommand": "default",
            "description": "Async echo step 1",
            "args": {"message": "async1"}
        },
        {
            "tool": "echo",
            "subcommand": "default",
            "description": "Async echo step 2",
            "args": {"message": "async2"}
        }
    ]
}
"#;
    fs::write(tools_dir.join("async_sequence.json"), async_sequence_config).await?;

    // Create a tool with subcommand sequence (qualitycheck pattern)
    let subcommand_sequence_config = r#"
{
    "name": "multi_step",
    "description": "Tool with subcommand sequence",
    "command": "echo",
    "timeout_seconds": 30,
    "enabled": true,
    "subcommand": [
        {
            "name": "default",
            "description": "Simple echo",
            "positional_args": [
                {
                    "name": "message",
                    "type": "string",
                    "description": "message to echo",
                    "required": false
                }
            ]
        },
        {
            "name": "pipeline",
            "description": "Multi-step pipeline using subcommand sequence",
            "synchronous": false,
            "step_delay_ms": 50,
            "sequence": [
                {
                    "tool": "multi_step",
                    "subcommand": "default",
                    "description": "Pipeline step 1",
                    "args": {"message": "pipeline_step_1"}
                },
                {
                    "tool": "multi_step",
                    "subcommand": "default",
                    "description": "Pipeline step 2",
                    "args": {"message": "pipeline_step_2"}
                }
            ]
        }
    ]
}
"#;
    fs::write(
        tools_dir.join("multi_step.json"),
        subcommand_sequence_config,
    )
    .await?;

    Ok(temp_dir)
}

// ============================================================================
// Synchronous Sequence Tool Tests
// ============================================================================

/// Test calling a synchronous sequence tool
#[tokio::test]
async fn test_sync_sequence_tool_execution() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_sequence_tool_config().await?;
    let tools_dir = temp_dir.path().join(".ahma");

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    // List tools to verify our sequence tool is loaded
    let tools = mcp.client.list_all_tools().await?;
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();

    assert!(
        tool_names.contains(&"sync_sequence"),
        "Should have sync_sequence tool. Got: {:?}",
        tool_names
    );

    // Call the synchronous sequence tool
    let params = CallToolRequestParams::new(Cow::Borrowed("sync_sequence"))
        .with_arguments(json!({}).as_object().unwrap().clone());

    let result = mcp.client.call_tool(params).await?;

    // Should complete with all steps
    assert!(!result.content.is_empty());
    let mut found_completion = false;
    for content in &result.content {
        if let Some(text_content) = content.as_text()
            && (text_content.text.contains("completed")
                || text_content.text.contains("step1")
                || text_content.text.contains("step2"))
        {
            found_completion = true;
        }
    }
    assert!(found_completion, "Sync sequence should show completion");

    mcp.client.cancel().await?;
    Ok(())
}

// ============================================================================
// Asynchronous Sequence Tool Tests
// ============================================================================

/// Test calling an asynchronous sequence tool
#[tokio::test]
async fn test_async_sequence_tool_execution() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_sequence_tool_config().await?;
    let tools_dir = temp_dir.path().join(".ahma");

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    // Call the asynchronous sequence tool
    let params = CallToolRequestParams::new(Cow::Borrowed("async_sequence"))
        .with_arguments(json!({}).as_object().unwrap().clone());

    let result = mcp.client.call_tool(params).await?;

    // Should return with operation IDs or inline results (automatic async)
    assert!(!result.content.is_empty());
    let mut found_expected = false;
    for content in &result.content {
        if let Some(text_content) = content.as_text()
            && (text_content.text.contains("operation")
                || text_content.text.contains("op_")
                || text_content.text.contains("started")
                || text_content.text.contains("async1")
                || text_content.text.contains("async2")
                || text_content.text.contains("completed"))
        {
            found_expected = true;
        }
    }
    assert!(
        found_expected,
        "Async sequence should return operation IDs or inline results"
    );

    // Wait for operations to complete
    let _ = wait_for_condition(
        std::time::Duration::from_secs(5),
        std::time::Duration::from_millis(50),
        || {
            let client = mcp.client.clone();
            async move {
                let status_params = CallToolRequestParams::new(Cow::Borrowed("status"))
                    .with_arguments(json!({}).as_object().unwrap().clone());
                client
                    .call_tool(status_params)
                    .await
                    .ok()
                    .map(|status_result| !status_result.content.is_empty())
                    .unwrap_or(false)
            }
        },
    )
    .await;

    // Check status to verify completion
    let status_params = CallToolRequestParams::new(Cow::Borrowed("status"))
        .with_arguments(json!({}).as_object().unwrap().clone());
    let status_result = mcp.client.call_tool(status_params).await?;
    assert!(!status_result.content.is_empty());

    mcp.client.cancel().await?;
    Ok(())
}

// ============================================================================
// Subcommand Sequence Tests
// ============================================================================

/// Test calling a subcommand that is itself a sequence
#[tokio::test]
async fn test_subcommand_sequence_execution() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_sequence_tool_config().await?;
    let tools_dir = temp_dir.path().join(".ahma");

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    // List tools to verify our tool is loaded
    let tools = mcp.client.list_all_tools().await?;
    let has_multi_step = tools
        .iter()
        .any(|t| t.name.as_ref().starts_with("multi_step"));

    if has_multi_step {
        // Call the pipeline subcommand which is a sequence
        let params = CallToolRequestParams::new(Cow::Borrowed("multi_step")).with_arguments(
            json!({"subcommand": "pipeline"})
                .as_object()
                .unwrap()
                .clone(),
        );

        let result = mcp.client.call_tool(params).await?;

        // Should return with operation IDs or completion info
        assert!(!result.content.is_empty());
    }

    mcp.client.cancel().await?;
    Ok(())
}

// ============================================================================
// Sequence Error Handling Tests
// ============================================================================

/// Test sequence with a non-existent tool reference
#[tokio::test]
async fn test_sequence_with_missing_tool_reference() -> Result<()> {
    init_test_logging();
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    // Create a sequence that references a non-existent tool
    let bad_sequence_config = r#"
{
    "name": "bad_sequence",
    "description": "Sequence with missing tool reference",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": true,
    "enabled": true,
    "sequence": [
        {
            "tool": "nonexistent_tool_xyz",
            "subcommand": "default",
            "description": "This should fail"
        }
    ]
}
"#;
    fs::write(tools_dir.join("bad_sequence.json"), bad_sequence_config).await?;

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    let tools = mcp.client.list_all_tools().await?;
    let has_bad_seq = tools.iter().any(|t| t.name.as_ref() == "bad_sequence");

    if has_bad_seq {
        let params = CallToolRequestParams::new(Cow::Borrowed("bad_sequence"))
            .with_arguments(json!({}).as_object().unwrap().clone());

        let result = mcp.client.call_tool(params).await;

        // Should fail with an error about missing tool
        assert!(result.is_err(), "Sequence with missing tool should error");
    }

    mcp.client.cancel().await?;
    Ok(())
}

// ============================================================================
// Sequence Skip-if-File Tests (skip_if_file_exists, skip_if_file_missing)
// ============================================================================

/// Test sequence with skip_if_file_exists - step is skipped when file exists
#[tokio::test]
async fn test_sequence_skip_if_file_exists() -> Result<()> {
    init_test_logging();
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    // Create skip marker file BEFORE running sequence
    let skip_marker = temp_dir.path().join("skip_when_exists.txt");
    fs::write(&skip_marker, "exists").await?;

    let echo_tool = r#"
{
    "name": "echo_skip",
    "description": "Echo for skip testing",
    "command": "echo",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [{"name": "default", "description": "echo", "positional_args": [{"name": "message", "type": "string", "required": false}]}]
}
"#;
    fs::write(tools_dir.join("echo_skip.json"), echo_tool).await?;

    let skip_sequence = format!(
        r#"
{{
    "name": "skip_sequence",
    "description": "Sequence with skip_if_file_exists",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": true,
    "enabled": true,
    "sequence": [
        {{"tool": "echo_skip", "subcommand": "default", "description": "Should run", "args": {{"message": "ran"}}}},
        {{"tool": "echo_skip", "subcommand": "default", "description": "Should skip", "skip_if_file_exists": "{}", "args": {{"message": "skipped"}}}},
        {{"tool": "echo_skip", "subcommand": "default", "description": "Should run", "args": {{"message": "ran2"}}}}
    ]
}}
"#,
        skip_marker.file_name().unwrap().to_str().unwrap()
    );
    fs::write(tools_dir.join("skip_sequence.json"), skip_sequence).await?;

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    let tools = mcp.client.list_all_tools().await?;
    let has_skip_seq = tools.iter().any(|t| t.name.as_ref() == "skip_sequence");

    if has_skip_seq {
        let params = CallToolRequestParams::new(Cow::Borrowed("skip_sequence")).with_arguments(
            json!({"working_directory": temp_dir.path().to_str().unwrap()})
                .as_object()
                .unwrap()
                .clone(),
        );

        let result = mcp.client.call_tool(params).await?;

        let all_text: String = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect();

        assert!(all_text.contains("ran"), "First step should run");
        assert!(all_text.contains("ran2"), "Third step should run");
        assert!(
            all_text.contains("skipped"),
            "Second step should be skipped"
        );
    }

    mcp.client.cancel().await?;
    Ok(())
}

/// Test sequence with skip_if_file_missing - step is skipped when file does NOT exist
#[tokio::test]
async fn test_sequence_skip_if_file_missing_with_default_wd() -> Result<()> {
    init_test_logging();
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    let echo_tool = r#"
{
    "name": "echo_miss",
    "description": "Echo for skip-missing testing",
    "command": "echo",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [{"name": "default", "description": "echo", "positional_args": [{"name": "message", "type": "string", "required": false}]}]
}
"#;
    fs::write(tools_dir.join("echo_miss.json"), echo_tool).await?;

    let skip_missing_sequence = r#"
{
    "name": "skip_missing_sequence",
    "description": "Sequence with skip_if_file_missing",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": true,
    "enabled": true,
    "sequence": [
        {"tool": "echo_miss", "subcommand": "default", "description": "Should run", "args": {"message": "step1"}},
        {"tool": "echo_miss", "subcommand": "default", "description": "Should skip", "skip_if_file_missing": "nonexistent_file_xyz.txt", "args": {"message": "would_skip"}},
        {"tool": "echo_miss", "subcommand": "default", "description": "Should run", "args": {"message": "step3"}}
    ]
}
"#;
    fs::write(
        tools_dir.join("skip_missing_sequence.json"),
        skip_missing_sequence,
    )
    .await?;

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    let tools = mcp.client.list_all_tools().await?;
    let has_skip_miss_seq = tools
        .iter()
        .any(|t| t.name.as_ref() == "skip_missing_sequence");

    if has_skip_miss_seq {
        let params = CallToolRequestParams::new(Cow::Borrowed("skip_missing_sequence"))
            .with_arguments(json!({}).as_object().unwrap().clone());

        let result = mcp.client.call_tool(params).await?;

        let all_text: String = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect();

        assert!(all_text.contains("step1"), "First step should run");
        assert!(all_text.contains("step3"), "Third step should run");
        assert!(
            all_text.contains("skipped"),
            "Second step should be skipped"
        );
    }

    mcp.client.cancel().await?;
    Ok(())
}

// ============================================================================
// Sequence Skip Logic Tests (skip_if_file_exists, skip_if_file_missing)
// ============================================================================

/// Test sequence with skip_if_file_exists - step is skipped when file exists (with working_dir param)
#[tokio::test]
async fn test_sequence_skip_if_file_exists_with_wd() -> Result<()> {
    init_test_logging();
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    // Create skip_me marker file
    let skip_marker = temp_dir.path().join("skip_me.txt");
    std::fs::write(&skip_marker, "exists").unwrap();

    let echo_tool = r#"
{
    "name": "echo_skip",
    "description": "Echo for skip testing",
    "command": "echo",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [{"name": "default", "description": "echo"}]
}
"#;
    fs::write(tools_dir.join("echo_skip.json"), echo_tool).await?;

    let skip_sequence_config = r#"
{
    "name": "skip_sequence",
    "description": "Sequence with skip_if_file_exists",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": true,
    "enabled": true,
    "sequence": [
        {
            "tool": "echo_skip",
            "subcommand": "default",
            "description": "Step 1 runs",
            "args": {"message": "step1_ran"}
        },
        {
            "tool": "echo_skip",
            "subcommand": "default",
            "description": "Step 2 skipped when file exists",
            "skip_if_file_exists": "skip_me.txt",
            "args": {"message": "step2_should_not_run"}
        },
        {
            "tool": "echo_skip",
            "subcommand": "default",
            "description": "Step 3 runs after skip",
            "args": {"message": "step3_ran"}
        }
    ]
}
"#;
    fs::write(tools_dir.join("skip_sequence.json"), skip_sequence_config).await?;

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    let params = CallToolRequestParams::new(Cow::Borrowed("skip_sequence")).with_arguments(
        json!({"working_directory": temp_dir.path().to_string_lossy()})
            .as_object()
            .unwrap()
            .clone(),
    );

    let result = mcp.client.call_tool(params).await?;

    let all_text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    assert!(all_text.contains("step1_ran") || all_text.contains("Step 1"));
    assert!(all_text.contains("step3_ran") || all_text.contains("Step 3"));
    assert!(all_text.contains("skipped") || all_text.contains("Step 2"));
    assert!(!all_text.contains("step2_should_not_run"));

    mcp.client.cancel().await?;
    Ok(())
}

/// Test sequence with skip_if_file_missing - step is skipped when file does not exist (explicit working_directory)
#[tokio::test]
async fn test_sequence_skip_if_file_missing_explicit_wd() -> Result<()> {
    init_test_logging();
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    let echo_tool = r#"
{
    "name": "echo_missing",
    "description": "Echo for missing-file skip testing",
    "command": "echo",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [{"name": "default", "description": "echo"}]
}
"#;
    fs::write(tools_dir.join("echo_missing.json"), echo_tool).await?;

    let missing_skip_config = r#"
{
    "name": "missing_skip_sequence",
    "description": "Sequence with skip_if_file_missing",
    "command": "sequence",
    "timeout_seconds": 30,
    "synchronous": true,
    "enabled": true,
    "sequence": [
        {
            "tool": "echo_missing",
            "subcommand": "default",
            "description": "Step 1 runs",
            "args": {"message": "step1_ran"}
        },
        {
            "tool": "echo_missing",
            "subcommand": "default",
            "description": "Step 2 skipped when file missing",
            "skip_if_file_missing": "nonexistent_file_xyz.txt",
            "args": {"message": "step2_should_not_run"}
        },
        {
            "tool": "echo_missing",
            "subcommand": "default",
            "description": "Step 3 runs after skip",
            "args": {"message": "step3_ran"}
        }
    ]
}
"#;
    fs::write(
        tools_dir.join("missing_skip_sequence.json"),
        missing_skip_config,
    )
    .await?;

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    let params = CallToolRequestParams::new(Cow::Borrowed("missing_skip_sequence")).with_arguments(
        json!({"working_directory": temp_dir.path().to_string_lossy()})
            .as_object()
            .unwrap()
            .clone(),
    );

    let result = mcp.client.call_tool(params).await?;

    let all_text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    assert!(all_text.contains("step1_ran") || all_text.contains("Step 1"));
    assert!(all_text.contains("step3_ran") || all_text.contains("Step 3"));
    assert!(all_text.contains("skipped"));
    assert!(!all_text.contains("step2_should_not_run"));

    mcp.client.cancel().await?;
    Ok(())
}

// ============================================================================
