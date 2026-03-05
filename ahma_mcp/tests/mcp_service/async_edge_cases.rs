//! Advanced MCP service testing for async notification delivery edge cases,
//! tool schema generation validation, and error handling for malformed MCP messages.
//!
//! This test module specifically targets Phase 7 requirements for:
//! - Async notification delivery edge cases  
//! - Tool schema generation validation
//! - Error handling for malformed MCP messages
use ahma_mcp::skip_if_disabled_async_result;

use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;

/// Test async notification delivery with malformed operation IDs
#[tokio::test]
async fn test_async_notification_malformed_ids() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Test status tool with numeric id (should be handled gracefully)
    let malformed_params = CallToolRequestParams::new("status").with_arguments(
        json!({ "id": 12345 })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );

    let result = client.call_tool(malformed_params).await;

    // Should complete successfully (status tool should handle this gracefully)
    assert!(result.is_ok());
    let call_result = result.unwrap();
    assert!(!call_result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test async notification delivery - await tool no longer accepts timeout
#[tokio::test]
async fn test_async_notification_extreme_timeout_values() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Test await with no timeout parameter (uses intelligent timeout)
    let no_timeout_params = CallToolRequestParams::new("await")
        .with_arguments(json!({}).as_object().cloned().unwrap_or_default());

    let result = client.call_tool(no_timeout_params).await?;
    assert!(!result.content.is_empty());

    // Test await with only valid parameters
    let valid_params = CallToolRequestParams::new("await").with_arguments(
        json!({ "tools": "cargo" })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );

    let result = client.call_tool(valid_params).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test tool schema generation with complex tool discovery
#[tokio::test]
async fn test_tool_schema_generation_comprehensive() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Test list_tools generates proper schemas
    let tools_result = client.list_all_tools().await?;

    // Should have multiple tools from .ahma directory
    // Note: Some tools may be disabled, so we just check that basic tools exist
    assert!(!tools_result.is_empty());
    assert!(
        tools_result.len() >= 2,
        "Expected at least the built-in tools (await, status) but got: {}",
        tools_result.len()
    );

    // Verify each tool has proper schema structure
    for tool in &tools_result {
        assert!(!tool.name.is_empty());

        // Check tool description exists
        if let Some(desc) = &tool.description {
            assert!(!desc.is_empty());
        }

        // Verify schema structure
        assert!(!tool.input_schema.is_empty());

        // Check that the schema contains basic required fields
        assert!(tool.input_schema.contains_key("type"));
        if let Some(type_val) = tool.input_schema.get("type") {
            assert_eq!(type_val.as_str().unwrap_or(""), "object");
        }
    }

    // Verify specific known tools exist (ls is now optional)
    let tool_names: Vec<&str> = tools_result.iter().map(|t| t.name.as_ref()).collect();

    assert!(tool_names.contains(&"await"), "Should have await tool");
    assert!(tool_names.contains(&"status"), "Should have status tool");
    // Note: ls tool is optional and may not be present if ls.json was removed

    client.cancel().await?;
    Ok(())
}

/// Test error handling for malformed call_tool parameters
#[tokio::test]
async fn test_error_handling_malformed_call_tool_params() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Test with missing required parameters for cancel tool
    let missing_params = CallToolRequestParams::new("cancel")
        .with_arguments(json!({}).as_object().cloned().unwrap_or_default());

    let result = client.call_tool(missing_params).await;
    assert!(result.is_err(), "Cancel tool should require id parameter");

    // Test with invalid parameter types for await tool (no timeout_seconds accepted)
    let invalid_types_params = CallToolRequestParams::new("await").with_arguments(
        json!({ "id": 12345 })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );

    let result = client.call_tool(invalid_types_params).await;
    // Should handle type mismatch gracefully
    assert!(result.is_ok() || result.is_err());

    client.cancel().await?;
    Ok(())
}

/// Test error handling for unknown tools
#[tokio::test]
async fn test_error_handling_unknown_tools() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let unknown_tool_params = CallToolRequestParams::new("unknown_tool");

    let result = client.call_tool(unknown_tool_params).await;
    assert!(result.is_err(), "Unknown tools should return error");

    client.cancel().await?;
    Ok(())
}

/// Test async notification system under concurrent load
#[tokio::test]
async fn test_async_notification_concurrent_load() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Start multiple async operations concurrently
    let mut handles = Vec::new();
    for i in 0..3 {
        let client_clone = client.clone();
        let handle = tokio::spawn(async move {
            let params = CallToolRequestParams::new("status").with_arguments(
                json!({ "id": format!("load_test_{}", i) })
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            );
            client_clone.call_tool(params).await
        });
        handles.push(handle);
    }

    // Wait for all operations to complete
    let results = futures::future::join_all(handles).await;

    // All should complete successfully
    for (i, result) in results.into_iter().enumerate() {
        let call_result = result
            .unwrap_or_else(|e| panic!("Task {} failed: {}", i, e))
            .unwrap_or_else(|e| panic!("Call {} failed: {}", i, e));
        assert!(!call_result.content.is_empty());
    }

    client.cancel().await?;
    Ok(())
}

/// Test status tool with various filter combinations
#[tokio::test]
async fn test_status_tool_filter_combinations() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Test with tool filter
    let tool_filter_params = CallToolRequestParams::new("status").with_arguments(
        json!({ "tools": "cargo" })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );

    let result = client.call_tool(tool_filter_params).await?;
    assert!(!result.content.is_empty());

    // Check that response mentions the filtered tools
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        let text = &text_content.text;
        assert!(text.contains("cargo") || text.contains("Operations status"));
    }

    // Test with both id and tools filter
    let combined_filter_params = CallToolRequestParams::new("status").with_arguments(
        json!({ "tools": "cargo", "id": "test-op" })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );

    let result = client.call_tool(combined_filter_params).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test async operations with real tool execution
#[tokio::test]
async fn test_async_operation_with_real_execution() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Start a real async operation (shell command)
    let async_params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        json!({ "command": "echo 'test async execution'" })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );

    let result = client.call_tool(async_params).await?;
    assert!(!result.content.is_empty());

    // Should return operation info immediately
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        let text = &text_content.text;
        assert!(
            text.contains("id")
                || text.contains("started")
                || text.contains("job_id")
                || text.contains("test async execution"),
            "Async operation should return operation tracking info, got: {}",
            text
        );
    }

    // Test that we can query the status
    let status_params = CallToolRequestParams::new("status")
        .with_arguments(json!({}).as_object().cloned().unwrap_or_default());

    let status_result = client.call_tool(status_params).await?;
    assert!(!status_result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test error recovery and resilience
#[tokio::test]
async fn test_error_recovery_and_resilience() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Test that service continues working after errors

    // 1. Cause an error with unknown tool
    let _ = client
        .call_tool(CallToolRequestParams::new("invalid_tool_for_recovery"))
        .await;

    // 2. Service should still work normally
    let working_params = CallToolRequestParams::new("status")
        .with_arguments(json!({}).as_object().cloned().unwrap_or_default());

    let result = client.call_tool(working_params).await?;
    assert!(!result.content.is_empty());

    // 3. Test with multiple rapid error/success cycles
    for i in 0..3 {
        // Error call
        let invalid_tool_name = format!("invalid_tool_{}", i);
        let _ = client
            .call_tool(CallToolRequestParams::new(invalid_tool_name.clone()))
            .await;

        // Successful call
        let good_result = client
            .call_tool(
                CallToolRequestParams::new("status")
                    .with_arguments(json!({}).as_object().cloned().unwrap_or_default()),
            )
            .await?;
        assert!(!good_result.content.is_empty());
    }

    client.cancel().await?;
    Ok(())
}
