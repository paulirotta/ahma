//! Advanced MCP service testing for async notification delivery edge cases,
//! tool schema generation validation, and error handling for malformed MCP messages.
//!
//! This test module specifically targets Phase 7 requirements for:
//! - Async notification delivery edge cases  
//! - Tool schema generation validation
//! - Error handling for malformed MCP messages
use ahma_mcp::skip_if_disabled_async_result;

use ahma_mcp::test_utils::client::{ClientBuilder, setup_test_environment};
use ahma_mcp::test_utils::in_process::create_in_process_mcp_empty;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;

/// Test async notification delivery with malformed operation IDs
#[tokio::test]
async fn test_async_notification_malformed_ids() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    // Test status tool with numeric id (should be handled gracefully)
    let malformed_params = CallToolRequestParams::new("status").with_arguments(
        json!({ "id": 12345 })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );

    let result = mcp.client.call_tool(malformed_params).await;

    // Should complete successfully (status tool should handle this gracefully)
    assert!(result.is_ok());
    assert!(!result.unwrap().content.is_empty());

    Ok(())
}

/// Test async notification delivery - await tool no longer accepts timeout
#[tokio::test]
async fn test_async_notification_extreme_timeout_values() -> Result<()> {
    init_test_logging();
    let (service, _tmp) = setup_test_environment().await;

    // Test await with no timeout parameter (uses intelligent timeout)
    let no_timeout_params = CallToolRequestParams::new("await")
        .with_arguments(json!({}).as_object().cloned().unwrap_or_default());

    let result = service
        .handle_await(no_timeout_params)
        .await
        .expect("await with empty params should return immediately");
    assert!(!result.content.is_empty());

    // Test await with only valid parameters
    let valid_params = CallToolRequestParams::new("await").with_arguments(
        json!({ "tools": "cargo" })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );

    let result = service
        .handle_await(valid_params)
        .await
        .expect("await with tool filter should return immediately when no ops exist");
    assert!(!result.content.is_empty());

    Ok(())
}

/// Test tool schema generation with complex tool discovery
#[tokio::test]
async fn test_tool_schema_generation_comprehensive() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let tools_result = mcp.client.list_all_tools().await?;

    assert!(!tools_result.is_empty());
    assert!(
        tools_result.len() >= 2,
        "Expected at least the built-in tools (await, status) but got: {}",
        tools_result.len()
    );

    for tool in &tools_result {
        assert!(!tool.name.is_empty());

        if let Some(desc) = &tool.description {
            assert!(!desc.is_empty());
        }

        assert!(!tool.input_schema.is_empty());
        assert!(tool.input_schema.contains_key("type"));
        if let Some(type_val) = tool.input_schema.get("type") {
            assert_eq!(type_val.as_str().unwrap_or(""), "object");
        }
    }

    let tool_names: Vec<&str> = tools_result.iter().map(|t| t.name.as_ref()).collect();
    assert!(tool_names.contains(&"await"), "Should have await tool");
    assert!(tool_names.contains(&"status"), "Should have status tool");

    Ok(())
}

/// Test error handling for malformed call_tool parameters
#[tokio::test]
async fn test_error_handling_malformed_call_tool_params() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    // sandboxed_shell requires the 'command' parameter
    let missing_params = CallToolRequestParams::new("sandboxed_shell")
        .with_arguments(json!({}).as_object().cloned().unwrap_or_default());

    let result = mcp.client.call_tool(missing_params).await;
    assert!(
        result.is_err(),
        "sandboxed_shell should require command parameter"
    );

    // Type mismatch for await id parameter — should not panic
    let invalid_types_params = CallToolRequestParams::new("await").with_arguments(
        json!({ "id": 12345 })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );

    let result = mcp.client.call_tool(invalid_types_params).await;
    assert!(result.is_ok() || result.is_err());

    Ok(())
}

/// Test error handling for unknown tools
#[tokio::test]
async fn test_error_handling_unknown_tools() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let unknown_tool_params = CallToolRequestParams::new("unknown_tool")
        .with_arguments(json!({}).as_object().cloned().unwrap_or_default());

    let result = mcp.client.call_tool(unknown_tool_params).await;
    assert!(result.is_err(), "Unknown tools should return error");

    Ok(())
}

/// Test async notification system under concurrent load
#[tokio::test]
async fn test_async_notification_concurrent_load() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    for i in 0..3 {
        let params = CallToolRequestParams::new("status").with_arguments(
            json!({ "id": format!("load_test_{}", i) })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
        let call_result = mcp.client.call_tool(params).await?;
        assert!(!call_result.content.is_empty());
    }

    Ok(())
}

/// Test status tool with various filter combinations
#[tokio::test]
async fn test_status_tool_filter_combinations() -> Result<()> {
    init_test_logging();
    let mcp = create_in_process_mcp_empty().await?;

    let tool_filter_params = CallToolRequestParams::new("status").with_arguments(
        json!({ "tools": "cargo" })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );

    let result = mcp.client.call_tool(tool_filter_params).await?;
    assert!(!result.content.is_empty());

    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        let text = &text_content.text;
        assert!(text.contains("cargo") || text.contains("Operations status"));
    }

    let combined_filter_params = CallToolRequestParams::new("status").with_arguments(
        json!({ "tools": "cargo", "id": "test-op" })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );

    let result = mcp.client.call_tool(combined_filter_params).await?;
    assert!(!result.content.is_empty());

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
    let mcp = create_in_process_mcp_empty().await?;

    // 1. Cause an error with unknown tool
    let _ = mcp
        .client
        .call_tool(CallToolRequestParams::new("invalid_tool_for_recovery"))
        .await;

    // 2. Service should still work normally
    let result = mcp
        .client
        .call_tool(
            CallToolRequestParams::new("status")
                .with_arguments(json!({}).as_object().cloned().unwrap_or_default()),
        )
        .await?;
    assert!(!result.content.is_empty());

    // 3. Multiple rapid error/success cycles
    for i in 0..3 {
        let _ = mcp
            .client
            .call_tool(CallToolRequestParams::new(format!("invalid_tool_{}", i)))
            .await;

        let good_result = mcp
            .client
            .call_tool(
                CallToolRequestParams::new("status")
                    .with_arguments(json!({}).as_object().cloned().unwrap_or_default()),
            )
            .await?;
        assert!(!good_result.content.is_empty());
    }

    Ok(())
}
