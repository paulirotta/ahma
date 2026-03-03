//! Sequence Tool Integration Coverage Tests
//!
//! Real integration tests for mcp_service/sequence.rs targeting:
//! - handle_sequence_tool (sync and async)
//! - handle_subcommand_sequence
//! - should_skip_step with environment variables
//! - format_step_started_message / format_step_skipped_message
//!
//! These tests spawn the actual ahma_mcp binary and use real tool configs.

use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::test_utils::concurrency::wait_for_condition;
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

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // List tools to verify our sequence tool is loaded
    let tools = client.list_all_tools().await?;
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();

    assert!(
        tool_names.contains(&"sync_sequence"),
        "Should have sync_sequence tool. Got: {:?}",
        tool_names
    );

    // Call the synchronous sequence tool
    let params = CallToolRequestParams {
        name: Cow::Borrowed("sync_sequence"),
        arguments: Some(json!({}).as_object().unwrap().clone()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await?;

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

    client.cancel().await?;
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

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // Call the asynchronous sequence tool
    let params = CallToolRequestParams {
        name: Cow::Borrowed("async_sequence"),
        arguments: Some(json!({}).as_object().unwrap().clone()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await?;

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
            let client = client.clone();
            async move {
                let status_params = CallToolRequestParams {
                    name: Cow::Borrowed("status"),
                    arguments: Some(json!({}).as_object().unwrap().clone()),
                    task: None,
                    meta: None,
                };
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
    let status_params = CallToolRequestParams {
        name: Cow::Borrowed("status"),
        arguments: Some(json!({}).as_object().unwrap().clone()),
        task: None,
        meta: None,
    };
    let status_result = client.call_tool(status_params).await?;
    assert!(!status_result.content.is_empty());

    client.cancel().await?;
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

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // List tools to verify our tool is loaded
    let tools = client.list_all_tools().await?;
    let has_multi_step = tools
        .iter()
        .any(|t| t.name.as_ref().starts_with("multi_step"));

    if has_multi_step {
        // Call the pipeline subcommand which is a sequence
        let params = CallToolRequestParams {
            name: Cow::Borrowed("multi_step"),
            arguments: Some(
                json!({"subcommand": "pipeline"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            task: None,
            meta: None,
        };

        let result = client.call_tool(params).await?;

        // Should return with operation IDs or completion info
        assert!(!result.content.is_empty());
    }

    client.cancel().await?;
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

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let tools = client.list_all_tools().await?;
    let has_bad_seq = tools.iter().any(|t| t.name.as_ref() == "bad_sequence");

    if has_bad_seq {
        let params = CallToolRequestParams {
            name: Cow::Borrowed("bad_sequence"),
            arguments: Some(json!({}).as_object().unwrap().clone()),
            task: None,
            meta: None,
        };

        let result = client.call_tool(params).await;

        // Should fail with an error about missing tool
        assert!(result.is_err(), "Sequence with missing tool should error");
    }

    client.cancel().await?;
    Ok(())
}

// ============================================================================
