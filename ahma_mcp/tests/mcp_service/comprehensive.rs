/// Comprehensive integration tests for mcp_service.rs coverage improvement
///
/// Target: Improve mcp_service.rs coverage from 59.44% to 85%+
/// Focus: Hardcoded tools, schema generation, error handling, path validation
///
/// Uses the in-process helper — no subprocess, full MCP handshake, runs in <50 ms.
use ahma_mcp::test_utils::in_process::create_in_process_mcp_empty;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::{Map, json};

/// Test that hardcoded tools are properly listed
#[tokio::test]
async fn test_hardcoded_tools_listing() -> Result<()> {
    let mcp = create_in_process_mcp_empty().await?;
    let result = mcp.client.list_all_tools().await?;

    // Should have the hardcoded tools (await, status)
    assert!(!result.is_empty());
    let tool_names: Vec<_> = result.iter().map(|t| t.name.as_ref()).collect();

    // Verify hardcoded tools are present
    assert!(tool_names.contains(&"await"));
    assert!(tool_names.contains(&"status"));

    // Verify each tool has proper schema
    for tool in &result {
        assert!(!tool.name.is_empty());
        assert!(tool.description.is_some());
        assert!(!tool.description.as_ref().unwrap().is_empty());

        // Verify input schema exists and is valid JSON structure
        let schema_value = serde_json::to_value(&*tool.input_schema);
        assert!(schema_value.is_ok());
    }

    Ok(())
}

/// Test await tool functionality and error handling
#[tokio::test]
async fn test_await_tool_comprehensive() -> Result<()> {
    let mcp = create_in_process_mcp_empty().await?;

    // Test valid await call with no timeout parameter (uses intelligent timeout)
    let params = Map::new();

    let call_param = CallToolRequestParams::new("await").with_arguments(params);


    let result = mcp.client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    // Verify response contains operation information
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        assert!(
            text_content.text.contains("operation")
                || text_content.text.contains("await")
                || text_content.text.contains("complete")
        );
    }

    // Test await with only valid fields (no timeout_seconds)
    let mut valid_params = Map::new();
    valid_params.insert("tools".to_string(), json!("cargo"));

    let valid_call_param = CallToolRequestParams::new("await").with_arguments(valid_params);

    let valid_result = mcp.client.call_tool(valid_call_param).await?;
    assert!(!valid_result.content.is_empty());

    Ok(())
}

/// Test status tool functionality
#[tokio::test]
async fn test_status_tool_comprehensive() -> Result<()> {
    let mcp = create_in_process_mcp_empty().await?;

    // Test basic status call
    let params = Map::new();

    let call_param = CallToolRequestParams::new("status").with_arguments(params);

    let result = mcp.client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    // Verify status provides operation information
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        assert!(
            text_content.text.contains("Operations")
                || text_content.text.contains("status")
                || text_content.text.contains("operation")
        );
    }

    // Test status with id parameter
    let mut specific_params = Map::new();
    specific_params.insert("id".to_string(), json!("test_operation_123"));

    let specific_call_param = CallToolRequestParams::new("status").with_arguments(specific_params);

    let specific_result = mcp.client.call_tool(specific_call_param).await?;
    assert!(!specific_result.content.is_empty());

    Ok(())
}
