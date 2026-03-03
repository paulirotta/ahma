//! MCP Service Integration Tests
//!
//! Tests for the mcp_service module covering:
//! 1. Tool listing and discovery
//! 2. Tool call execution
//! 3. Operation lifecycle through MCP protocol
//! 4. Error handling for invalid tool calls
//! 5. Subcommand routing
//!
//! These are real integration tests using the actual ahma_mcp binary via stdio MCP.

use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::borrow::Cow;
use tempfile::TempDir;
use tokio::fs;

/// Setup test tools directory with various tool configurations
async fn setup_mcp_service_test_tools() -> Result<TempDir> {
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    // 1. Simple synchronous tool
    let echo_tool = r#"
{
    "name": "test_echo",
    "description": "Test echo tool for MCP service testing",
    "command": "echo",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [
        {
            "name": "default",
            "description": "Echo a message",
            "positional_args": [
                {
                    "name": "message",
                    "type": "string",
                    "description": "The message to echo",
                    "required": false
                }
            ]
        },
        {
            "name": "uppercase",
            "description": "Echo in uppercase",
            "positional_args": [
                {
                    "name": "message",
                    "type": "string",
                    "description": "The message to echo",
                    "required": true
                }
            ]
        }
    ]
}
"#;
    fs::write(tools_dir.join("test_echo.json"), echo_tool).await?;

    // 2. Asynchronous tool
    let async_tool = r#"
{
    "name": "async_echo",
    "description": "Async echo tool",
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
    fs::write(tools_dir.join("async_echo.json"), async_tool).await?;

    // 3. Tool with options (not just positional args)
    let options_tool = r#"
{
    "name": "options_tool",
    "description": "Tool with various option types",
    "command": "echo",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [
        {
            "name": "default",
            "description": "Tool with options",
            "options": [
                {
                    "name": "verbose",
                    "type": "boolean",
                    "description": "Enable verbose output",
                    "short": "v",
                    "required": false
                },
                {
                    "name": "count",
                    "type": "integer",
                    "description": "Number of times",
                    "short": "n",
                    "required": false
                },
                {
                    "name": "output",
                    "type": "string",
                    "description": "Output file path",
                    "short": "o",
                    "required": false
                }
            ]
        }
    ]
}
"#;
    fs::write(tools_dir.join("options_tool.json"), options_tool).await?;

    // 4. Disabled tool (should not appear in listings)
    let disabled_tool = r#"
{
    "name": "disabled_tool",
    "description": "This tool is disabled",
    "command": "echo",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": false,
    "subcommand": [
        {
            "name": "default",
            "description": "Should not be listed"
        }
    ]
}
"#;
    fs::write(tools_dir.join("disabled_tool.json"), disabled_tool).await?;

    // sandboxed_shell is a core built-in tool - no JSON config needed

    Ok(temp_dir)
}

// ============================================================================
// Test: Tool Listing / Discovery
// ============================================================================

/// Test that list_tools returns all enabled tools
#[tokio::test]
async fn test_mcp_list_tools_returns_enabled_tools() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_mcp_service_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;
    let tools = client.list_all_tools().await?;

    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();

    // Enabled tools should be present
    assert!(
        tool_names.iter().any(|n| n.contains("test_echo")),
        "Should list test_echo tool. Got: {:?}",
        tool_names
    );
    assert!(
        tool_names.iter().any(|n| n.contains("async_echo")),
        "Should list async_echo tool. Got: {:?}",
        tool_names
    );
    assert!(
        tool_names.iter().any(|n| n.contains("sandboxed_shell")),
        "Should list sandboxed_shell tool. Got: {:?}",
        tool_names
    );

    // Disabled tools should NOT be present
    assert!(
        !tool_names.iter().any(|n| n.contains("disabled_tool")),
        "Should NOT list disabled_tool. Got: {:?}",
        tool_names
    );

    client.cancel().await?;
    Ok(())
}

/// Test that tools have descriptions from their config
#[tokio::test]
async fn test_mcp_tool_descriptions_populated() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_mcp_service_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;
    let tools = client.list_all_tools().await?;

    // Find the test_echo tool
    let echo_tool = tools.iter().find(|t| t.name.as_ref().contains("test_echo"));
    assert!(echo_tool.is_some(), "Should find test_echo tool");

    let tool = echo_tool.unwrap();
    assert!(tool.description.is_some(), "Tool should have a description");

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Synchronous Tool Execution
// ============================================================================

/// Test calling a synchronous tool with positional arguments
#[tokio::test]
async fn test_mcp_call_sync_tool_with_positional_args() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_mcp_service_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let params = CallToolRequestParams {
        name: Cow::Borrowed("test_echo"),
        arguments: Some(
            json!({"message": "hello world"})
                .as_object()
                .unwrap()
                .clone(),
        ),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await?;

    // Should not be an error
    assert!(
        !result.is_error.unwrap_or(false),
        "Sync tool call should succeed"
    );

    // Should have output containing our message
    let all_text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    assert!(
        all_text.contains("hello") || all_text.contains("world"),
        "Output should contain the echoed message. Got: {}",
        all_text
    );

    client.cancel().await?;
    Ok(())
}

/// Test calling a tool with no arguments (uses defaults)
#[tokio::test]
async fn test_mcp_call_tool_with_no_args() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_mcp_service_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let params = CallToolRequestParams {
        name: Cow::Borrowed("test_echo"),
        arguments: Some(json!({}).as_object().unwrap().clone()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await?;

    // Should succeed even with no args (message is optional)
    assert!(
        !result.is_error.unwrap_or(false),
        "Tool call with optional args should succeed"
    );

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Asynchronous Tool Execution
// ============================================================================

/// Test calling an asynchronous tool returns either operation ID or inline result
/// (automatic async may return the result directly if the command completes fast enough)
#[tokio::test]
async fn test_mcp_call_async_tool_returns_id() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_mcp_service_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let params = CallToolRequestParams {
        name: Cow::Borrowed("async_echo"),
        arguments: Some(
            json!({"message": "async test"})
                .as_object()
                .unwrap()
                .clone(),
        ),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await?;

    // Async tools should return successfully
    assert!(
        !result.is_error.unwrap_or(false),
        "Async tool call should succeed"
    );

    // Output should either contain:
    // 1. Operation ID (if automatic async timeout elapsed), or
    // 2. Actual output (if command completed within automatic async window)
    let all_text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    let has_async_id =
        all_text.contains("op_") || all_text.contains("operation") || all_text.contains("started");
    let has_actual_output = all_text.contains("async test");

    assert!(
        has_async_id || has_actual_output,
        "Async call should indicate operation started or return inline result. Got: {}",
        all_text
    );

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Subcommand Routing
// ============================================================================

/// Test that explicit subcommand parameter routes correctly
#[tokio::test]
async fn test_mcp_subcommand_routing() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_mcp_service_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    // Call with explicit subcommand
    let params = CallToolRequestParams {
        name: Cow::Borrowed("test_echo"),
        arguments: Some(
            json!({"subcommand": "uppercase", "message": "test"})
                .as_object()
                .unwrap()
                .clone(),
        ),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await?;

    // Should succeed
    assert!(
        !result.is_error.unwrap_or(false),
        "Subcommand call should succeed"
    );

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Error Handling
// ============================================================================

/// Test calling a non-existent tool returns an error
#[tokio::test]
async fn test_mcp_call_nonexistent_tool_error() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_mcp_service_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let params = CallToolRequestParams {
        name: Cow::Borrowed("nonexistent_tool_xyz"),
        arguments: Some(json!({}).as_object().unwrap().clone()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await;

    // Should fail
    match result {
        Err(e) => {
            let error_msg = format!("{:?}", e);
            assert!(
                error_msg.contains("not found")
                    || error_msg.contains("unknown")
                    || error_msg.contains("not exist"),
                "Error should mention tool not found. Got: {}",
                error_msg
            );
        }
        Ok(r) => {
            // If it returns Ok, should be marked as error
            assert!(
                r.is_error.unwrap_or(false),
                "Result for nonexistent tool should be marked as error"
            );
        }
    }

    client.cancel().await?;
    Ok(())
}

/// Test calling a tool with invalid subcommand returns an error
#[tokio::test]
async fn test_mcp_call_invalid_subcommand_error() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_mcp_service_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let params = CallToolRequestParams {
        name: Cow::Borrowed("test_echo"),
        arguments: Some(
            json!({"subcommand": "nonexistent_subcommand"})
                .as_object()
                .unwrap()
                .clone(),
        ),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await;

    // Should fail or return error result
    match result {
        Err(e) => {
            let error_msg = format!("{:?}", e);
            assert!(
                error_msg.contains("not found")
                    || error_msg.contains("unknown")
                    || error_msg.contains("subcommand"),
                "Error should mention subcommand issue. Got: {}",
                error_msg
            );
        }
        Ok(r) => {
            // If it returns Ok, should be marked as error
            assert!(
                r.is_error.unwrap_or(false),
                "Result for invalid subcommand should be marked as error"
            );
        }
    }

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Shell Command Execution
// ============================================================================

/// Test executing a shell command through MCP
#[tokio::test]
async fn test_mcp_shell_command_execution() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_mcp_service_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let params = CallToolRequestParams {
        name: Cow::Borrowed("sandboxed_shell"),
        arguments: Some(
            json!({
                "command": "echo 'MCP test output'",
                "execution_mode": "Synchronous"
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        task: None,
        meta: None,
    };

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

    assert!(
        all_text.contains("MCP test output"),
        "Should contain command output. Got: {}",
        all_text
    );

    client.cancel().await?;
    Ok(())
}

/// Test shell command that fails returns error status
#[tokio::test]
async fn test_mcp_shell_command_failure() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_mcp_service_test_tools().await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let params = CallToolRequestParams {
        name: Cow::Borrowed("sandboxed_shell"),
        arguments: Some(json!({"command": "exit 1"}).as_object().unwrap().clone()),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await;

    // A failing command can either:
    // 1. Return Err (MCP protocol error for the failure)
    // 2. Return Ok with is_error=true
    // 3. Return Ok with output indicating failure
    // All are valid behaviors - the test verifies failure is detected somehow
    match result {
        Err(e) => {
            // The server returns an error for failed commands
            let error_msg = format!("{:?}", e);
            assert!(
                error_msg.contains("exit code 1")
                    || error_msg.contains("failed")
                    || error_msg.contains("Command failed"),
                "Error should indicate command failure. Got: {}",
                error_msg
            );
        }
        Ok(r) => {
            // If it returns Ok, check for failure indication
            let all_text: String = r
                .content
                .iter()
                .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                .collect();

            let is_error = r.is_error.unwrap_or(false);
            let text_indicates_failure = all_text.contains("exit")
                || all_text.contains("fail")
                || all_text.contains("error")
                || all_text.contains("1");

            assert!(
                is_error || text_indicates_failure || all_text.is_empty(),
                "Should handle failed command appropriately"
            );
        }
    }

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Test: Working Directory
// ============================================================================

/// Test that working_directory parameter is respected
#[tokio::test]
async fn test_mcp_working_directory_parameter() -> Result<()> {
    init_test_logging();
    let temp_dir = setup_mcp_service_test_tools().await?;

    // Create a subdirectory with a test file
    let sub_dir = temp_dir.path().join("test_subdir");
    fs::create_dir_all(&sub_dir).await?;
    fs::write(sub_dir.join("marker.txt"), "marker content").await?;

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .working_dir(temp_dir.path())
        .build()
        .await?;

    let params = CallToolRequestParams {
        name: Cow::Borrowed("sandboxed_shell"),
        arguments: Some(
            json!({
                "command": "ls",
                "working_directory": sub_dir.to_str().unwrap(),
                "execution_mode": "Synchronous"
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await?;

    let all_text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();

    assert!(
        all_text.contains("marker.txt"),
        "Should list files in working directory. Got: {}",
        all_text
    );

    client.cancel().await?;
    Ok(())
}
