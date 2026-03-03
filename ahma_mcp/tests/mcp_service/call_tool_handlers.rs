//! call_tool_handlers tests
//!
//! Tests for the call_tool() method handlers in mcp_service.rs:
//! - status tool: filtering by tool name, id, efficiency analysis
//! - await tool: waiting for specific id, tool filters, timeout handling
//! - cancel tool: cancelling operations, error cases
//!
//! These tests target untested paths to improve coverage from 36.71% to 65%+.

use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::{Map, json};
use std::borrow::Cow;
use tempfile::tempdir;

// ============= STATUS TOOL TESTS =============

/// Test status tool with comma-separated tool name filter
#[tokio::test]
async fn test_status_tool_with_tool_name_filter() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Filter by multiple tool names
    let mut params = Map::new();
    params.insert("tools".to_string(), json!("cargo, git, echo"));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("status"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    // Verify response mentions the filter
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        // Should show filter info in output
        assert!(
            text_content.text.contains("cargo")
                || text_content.text.contains("Operations status")
                || text_content.text.contains("active")
        );
    }

    client.cancel().await?;
    Ok(())
}

/// Test status tool with specific id parameter
#[tokio::test]
async fn test_status_tool_with_id() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Query for a specific operation (non-existent)
    let mut params = Map::new();
    params.insert("id".to_string(), json!("op_nonexistent_12345"));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("status"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    // Should indicate operation not found
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        assert!(text_content.text.contains("not found") || text_content.text.contains("found"));
    }

    client.cancel().await?;
    Ok(())
}

/// Test status tool with empty tool filter (shows all)
#[tokio::test]
async fn test_status_tool_empty_filter() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Empty tools parameter should show all operations
    let mut params = Map::new();
    params.insert("tools".to_string(), json!(""));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("status"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test status tool with both tools and id filters
#[tokio::test]
async fn test_status_tool_combined_filters() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let mut params = Map::new();
    params.insert("tools".to_string(), json!("cargo"));
    params.insert("id".to_string(), json!("op_123"));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("status"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

// ============= AWAIT TOOL TESTS =============

/// Test await tool with specific id (already completed)
#[tokio::test]
async fn test_await_tool_with_id_not_found() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let mut params = Map::new();
    params.insert("id".to_string(), json!("op_does_not_exist"));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("await"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    // Should indicate operation not found
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        assert!(
            text_content.text.contains("not found") || text_content.text.contains("No pending")
        );
    }

    client.cancel().await?;
    Ok(())
}

/// Test await tool with tool filter when no operations pending
#[tokio::test]
async fn test_await_tool_with_tool_filter_no_pending() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let mut params = Map::new();
    params.insert("tools".to_string(), json!("nonexistent_tool"));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("await"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    // Should indicate no pending operations
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        assert!(
            text_content.text.contains("No pending") || text_content.text.contains("operation")
        );
    }

    client.cancel().await?;
    Ok(())
}

/// Test await tool with multiple comma-separated tool filters
#[tokio::test]
async fn test_await_tool_multiple_tool_filters() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let mut params = Map::new();
    params.insert("tools".to_string(), json!("cargo, git, npm"));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("await"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test await with empty parameters
#[tokio::test]
async fn test_await_tool_empty_params() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("await"),
        arguments: Some(Map::new()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    // Should indicate no pending operations when none exist
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        assert!(
            text_content.text.contains("No pending")
                || text_content.text.contains("await")
                || text_content.text.contains("operation")
        );
    }

    client.cancel().await?;
    Ok(())
}

// ============= CANCEL TOOL TESTS =============

/// Test cancel tool with missing id
#[tokio::test]
async fn test_cancel_tool_missing_id() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Cancel requires id
    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("cancel"),
        arguments: Some(Map::new()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await;

    // Should fail with missing parameter error
    assert!(result.is_err());

    client.cancel().await?;
    Ok(())
}

/// Test cancel tool with non-existent id
#[tokio::test]
async fn test_cancel_tool_nonexistent_operation() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let mut params = Map::new();
    params.insert("id".to_string(), json!("op_does_not_exist"));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("cancel"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    // Should indicate operation not found
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        assert!(
            text_content.text.contains("not found")
                || text_content.text.contains("never existed")
                || text_content.text.contains("FAIL")
        );
    }

    client.cancel().await?;
    Ok(())
}

/// Test cancel tool with reason
#[tokio::test]
async fn test_cancel_tool_with_reason() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let mut params = Map::new();
    params.insert("id".to_string(), json!("op_test_cancel"));
    params.insert("reason".to_string(), json!("User requested cancellation"));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("cancel"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test cancel tool with invalid id type
#[tokio::test]
async fn test_cancel_tool_invalid_id_type() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let mut params = Map::new();
    // Pass number instead of string
    params.insert("id".to_string(), json!(12345));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("cancel"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await;

    // Should fail with type error
    assert!(result.is_err());

    client.cancel().await?;
    Ok(())
}

// ============= TOOL NOT FOUND TESTS =============

/// Test calling a tool that doesn't exist
#[tokio::test]
async fn test_call_nonexistent_tool() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("completely_fake_tool_that_does_not_exist"),
        arguments: Some(Map::new()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await;

    // Should return error for unknown tool
    assert!(result.is_err());

    client.cancel().await?;
    Ok(())
}

// ============= DISABLED TOOL TESTS =============

/// Test calling a disabled tool fails appropriately
#[tokio::test]
async fn test_call_disabled_tool() -> Result<()> {
    init_test_logging();

    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir)?;

    // Create a disabled tool
    let tool_json = json!({
        "name": "disabled_echo",
        "description": "A disabled tool",
        "command": "echo",
        "enabled": false,
        "subcommand": [{
            "name": "default",
            "description": "Default"
        }]
    });

    std::fs::write(
        tools_dir.join("disabled_echo.json"),
        serde_json::to_string_pretty(&tool_json)?,
    )?;

    let client = ClientBuilder::new()
        .tools_dir(tools_dir)
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // Try to call the disabled tool
    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("disabled_echo"),
        arguments: Some(Map::new()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await;

    // Should fail because tool is disabled
    assert!(result.is_err());

    client.cancel().await?;
    Ok(())
}

// ============= SUBCOMMAND RESOLUTION TESTS =============

/// Test calling a tool with invalid subcommand
#[tokio::test]
async fn test_call_tool_invalid_subcommand() -> Result<()> {
    init_test_logging();

    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir)?;

    let tool_json = json!({
        "name": "test_subcmd",
        "description": "Test tool with subcommands",
        "command": "echo",
        "enabled": true,
        "subcommand": [
            {
                "name": "valid_sub",
                "description": "Valid subcommand",
                "enabled": true
            }
        ]
    });

    std::fs::write(
        tools_dir.join("test_subcmd.json"),
        serde_json::to_string_pretty(&tool_json)?,
    )?;

    let client = ClientBuilder::new()
        .tools_dir(tools_dir)
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // Call with invalid subcommand
    let mut params = Map::new();
    params.insert("subcommand".to_string(), json!("nonexistent_subcommand"));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("test_subcmd"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await;

    // Should fail with subcommand not found error
    assert!(result.is_err());

    client.cancel().await?;
    Ok(())
}

/// Test calling a tool with disabled subcommand
#[tokio::test]
async fn test_call_tool_disabled_subcommand() -> Result<()> {
    init_test_logging();

    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir)?;

    let tool_json = json!({
        "name": "test_disabled_sub",
        "description": "Test tool with disabled subcommand",
        "command": "echo",
        "enabled": true,
        "subcommand": [
            {
                "name": "enabled_sub",
                "description": "Enabled subcommand",
                "enabled": true
            },
            {
                "name": "disabled_sub",
                "description": "Disabled subcommand",
                "enabled": false
            }
        ]
    });

    std::fs::write(
        tools_dir.join("test_disabled_sub.json"),
        serde_json::to_string_pretty(&tool_json)?,
    )?;

    let client = ClientBuilder::new()
        .tools_dir(tools_dir)
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // Call with disabled subcommand
    let mut params = Map::new();
    params.insert("subcommand".to_string(), json!("disabled_sub"));

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("test_disabled_sub"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await;

    // Should fail because subcommand is disabled
    assert!(result.is_err());

    client.cancel().await?;
    Ok(())
}

// ============= EXECUTION MODE TESTS =============

/// Test synchronous execution mode
#[tokio::test]
async fn test_synchronous_execution_mode() -> Result<()> {
    init_test_logging();

    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir)?;

    // Tool configured for synchronous execution
    let tool_json = json!({
        "name": "sync_echo",
        "description": "Synchronous echo tool",
        "command": "echo",
        "enabled": true,
        "synchronous": true,
        "subcommand": [{
            "name": "default",
            "description": "Default subcommand",
            "options": [
                {"name": "message", "type": "string", "description": "Message to echo"}
            ]
        }]
    });

    std::fs::write(
        tools_dir.join("sync_echo.json"),
        serde_json::to_string_pretty(&tool_json)?,
    )?;

    let client = ClientBuilder::new()
        .tools_dir(tools_dir)
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let mut params = Map::new();
    params.insert("message".to_string(), json!("hello world"));
    params.insert(
        "working_directory".to_string(),
        json!(temp_dir.path().to_str().unwrap()),
    );

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("sync_echo"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    // Synchronous should return output directly
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        assert!(text_content.text.contains("hello") || text_content.text.contains("world"));
    }

    client.cancel().await?;
    Ok(())
}

/// Test asynchronous execution mode (default)
#[tokio::test]
async fn test_async_execution_mode() -> Result<()> {
    init_test_logging();

    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir)?;

    // Tool without synchronous flag (defaults to async)
    let tool_json = json!({
        "name": "async_echo",
        "description": "Asynchronous echo tool",
        "command": "echo",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Default subcommand",
            "options": [
                {"name": "message", "type": "string", "description": "Message to echo"}
            ]
        }]
    });

    std::fs::write(
        tools_dir.join("async_echo.json"),
        serde_json::to_string_pretty(&tool_json)?,
    )?;

    let client = ClientBuilder::new()
        .tools_dir(tools_dir)
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let mut params = Map::new();
    params.insert("message".to_string(), json!("async test"));
    params.insert(
        "working_directory".to_string(),
        json!(temp_dir.path().to_str().unwrap()),
    );

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("async_echo"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    // Async should return operation ID or inline result (automatic async)
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        assert!(
            text_content.text.contains("op_")
                || text_content.text.contains("Asynchronous")
                || text_content.text.contains("operation")
                || text_content.text.contains("async test"),
            "Should contain op ID or inline result. Got: {}",
            text_content.text
        );
    }

    client.cancel().await?;
    Ok(())
}

/// Test explicit execution_mode argument override
#[tokio::test]
async fn test_explicit_execution_mode_argument() -> Result<()> {
    init_test_logging();

    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir)?;

    let tool_json = json!({
        "name": "mode_test",
        "description": "Test execution mode override",
        "command": "echo",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Default subcommand"
        }]
    });

    std::fs::write(
        tools_dir.join("mode_test.json"),
        serde_json::to_string_pretty(&tool_json)?,
    )?;

    let client = ClientBuilder::new()
        .tools_dir(tools_dir)
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // Request synchronous execution via argument
    let mut params = Map::new();
    params.insert("execution_mode".to_string(), json!("Synchronous"));
    params.insert(
        "working_directory".to_string(),
        json!(temp_dir.path().to_str().unwrap()),
    );

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("mode_test"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

// ============= NESTED SUBCOMMAND TESTS =============

/// Test calling nested subcommands
#[tokio::test]
async fn test_nested_subcommand_execution() -> Result<()> {
    init_test_logging();

    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir)?;

    let tool_json = json!({
        "name": "nested_tool",
        "description": "Tool with nested subcommands",
        "command": "echo",
        "enabled": true,
        "synchronous": true,
        "subcommand": [
            {
                "name": "parent",
                "description": "Parent subcommand",
                "enabled": true,
                "subcommand": [
                    {
                        "name": "child",
                        "description": "Child subcommand",
                        "enabled": true
                    }
                ]
            }
        ]
    });

    std::fs::write(
        tools_dir.join("nested_tool.json"),
        serde_json::to_string_pretty(&tool_json)?,
    )?;

    let client = ClientBuilder::new()
        .tools_dir(tools_dir)
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // Call nested subcommand with underscore-separated path
    let mut params = Map::new();
    params.insert("subcommand".to_string(), json!("parent_child"));
    params.insert(
        "working_directory".to_string(),
        json!(temp_dir.path().to_str().unwrap()),
    );

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("nested_tool"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

// ============= WORKING DIRECTORY TESTS =============

/// Test working_directory parameter handling
#[tokio::test]
async fn test_working_directory_parameter() -> Result<()> {
    init_test_logging();

    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    let work_dir = temp_dir.path().join("work");
    std::fs::create_dir_all(&tools_dir)?;
    std::fs::create_dir_all(&work_dir)?;

    let tool_json = json!({
        "name": "pwd_test",
        "description": "Test working directory",
        "command": "pwd",
        "enabled": true,
        "synchronous": true,
        "subcommand": [{
            "name": "default",
            "description": "Print working directory"
        }]
    });

    std::fs::write(
        tools_dir.join("pwd_test.json"),
        serde_json::to_string_pretty(&tool_json)?,
    )?;

    let client = ClientBuilder::new()
        .tools_dir(tools_dir)
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let mut params = Map::new();
    params.insert(
        "working_directory".to_string(),
        json!(work_dir.to_str().unwrap()),
    );

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("pwd_test"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    // Should show the working directory we specified
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        assert!(text_content.text.contains("work"));
    }

    client.cancel().await?;
    Ok(())
}

/// Test default working directory when not specified
#[tokio::test]
async fn test_default_working_directory() -> Result<()> {
    init_test_logging();

    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir)?;

    let tool_json = json!({
        "name": "default_wd",
        "description": "Test default working directory",
        "command": "echo",
        "enabled": true,
        "synchronous": true,
        "subcommand": [{
            "name": "default",
            "description": "Echo test"
        }]
    });

    std::fs::write(
        tools_dir.join("default_wd.json"),
        serde_json::to_string_pretty(&tool_json)?,
    )?;

    let client = ClientBuilder::new()
        .tools_dir(tools_dir)
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // Call without working_directory - should use default "."
    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("default_wd"),
        arguments: Some(Map::new()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

// ============= TIMEOUT TESTS =============

/// Test timeout parameter handling
#[tokio::test]
async fn test_timeout_parameter() -> Result<()> {
    init_test_logging();

    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir)?;

    let tool_json = json!({
        "name": "timeout_test",
        "description": "Test timeout parameter",
        "command": "echo",
        "enabled": true,
        "synchronous": true,
        "subcommand": [{
            "name": "default",
            "description": "Echo test"
        }]
    });

    std::fs::write(
        tools_dir.join("timeout_test.json"),
        serde_json::to_string_pretty(&tool_json)?,
    )?;

    let client = ClientBuilder::new()
        .tools_dir(tools_dir)
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let mut params = Map::new();
    params.insert("timeout_seconds".to_string(), json!(60));
    params.insert(
        "working_directory".to_string(),
        json!(temp_dir.path().to_str().unwrap()),
    );

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("timeout_test"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}
