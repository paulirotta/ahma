//! Integration test for tool name flattening
//!
//! This test verifies that subcommands are correctly exposed as flattened tools
//! (e.g., "file-tools_pwd") and can be called directly.

use ahma_mcp::test_utils::in_process::create_in_process_mcp_from_dir;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::borrow::Cow;
use tokio::fs;

#[tokio::test]
async fn test_flattened_tool_calling() -> Result<()> {
    init_test_logging();

    // 1. Setup test tools directory with a tool that has subcommands
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    let tool_config = json!({
        "name": "file-tools",
        "description": "File manipulation tools",
        "command": "printf",
        "enabled": true,
        "subcommand": [
            {
                "name": "hello",
                "description": "Print hello",
                "enabled": true,
                "synchronous": true
            },
            {
                "name": "world",
                "description": "Print world",
                "enabled": true,
                "synchronous": true
            }
        ]
    });
    fs::write(
        tools_dir.join("file-tools.json"),
        serde_json::to_string(&tool_config)?,
    )
    .await?;

    // 2. Start the MCP server using in-process helper
    let mcp = create_in_process_mcp_from_dir(&tools_dir).await?;

    // 3. Verify that the flattened tools appear in list_tools
    let tools = mcp.client.list_all_tools().await.map_err(|e| {
        eprintln!("WARNING  test_flattened_tool_calling: list_tools failed: {e}. Skipping.");
        e
    })?;
    let tool_names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();

    assert!(
        tool_names.contains(&"file-tools_hello"),
        "Flattened tool 'file-tools_hello' should be listed. Got: {:?}",
        tool_names
    );
    assert!(
        tool_names.contains(&"file-tools_world"),
        "Flattened tool 'file-tools_world' should be listed. Got: {:?}",
        tool_names
    );

    // 4. Call the flattened tool directly
    let params = CallToolRequestParams::new(Cow::Borrowed("file-tools_hello"))
        .with_arguments(json!({}).as_object().unwrap().clone());

    let result = mcp.client.call_tool(params).await.map_err(|e| {
        eprintln!("WARNING  test_flattened_tool_calling: call_tool failed: {e}. Skipping.");
        e
    })?;

    // The call should succeed
    assert!(
        !result.is_error.unwrap_or(false),
        "Call to flattened tool 'file-tools_hello' should succeed. Error: {:?}",
        result
    );

    Ok(())
}
