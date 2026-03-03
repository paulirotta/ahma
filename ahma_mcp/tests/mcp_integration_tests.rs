//! Integration tests for the ahma_mcp service.

use ahma_mcp::skip_if_disabled_async_result;

use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::{Map, json};
use std::borrow::Cow;

// Assuming common::test_client::new_client can be optimized for speed,
// e.g., by using in-memory setups or pre-initialized clients.
// If new_client involves file I/O, replace std::fs with tokio::fs for async ops.

#[tokio::test]
async fn test_list_tools() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;
    let result = client.list_all_tools().await?;

    // Should have at least the built-in 'await' tool
    assert!(!result.is_empty());
    let tool_names: Vec<_> = result.iter().map(|t| t.name.as_ref()).collect();
    assert!(tool_names.contains(&"await"));
    // Note: ls tool is optional and may not be present if ls.json was removed

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn test_call_tool_basic() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Use the await tool which should always be available - no timeout parameter needed
    let params = Map::new();

    let call_param = CallToolRequestParams {
        name: Cow::Borrowed("await"),
        arguments: Some(params),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_param).await?;

    // The result should contain operation status information
    assert!(!result.content.is_empty());
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        // Should contain information about operations or status
        assert!(
            text_content.text.contains("operation")
                || text_content.text.contains("status")
                || text_content.text.contains("completed")
        );
    }

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn test_async_notification_delivery() -> Result<()> {
    skip_if_disabled_async_result!("sandboxed_shell");
    init_test_logging();
    // Use --async flag to enable async execution
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Test that an async operation completes and we can check its status
    // This is a simpler but more reliable test of async notification delivery

    // 1. Start a long-running async operation using bash with sleep
    let async_tool_params = json!({
        "command": "sleep 1"
    });
    let call_params = CallToolRequestParams {
        name: Cow::Borrowed("sandboxed_shell"),
        arguments: async_tool_params.as_object().cloned(),
        task: None,
        meta: None,
    };

    let result = client.call_tool(call_params).await?;

    // The async tool should return immediately with operation info, or complete inline
    assert!(!result.content.is_empty());
    if let Some(content) = result.content.first()
        && let Some(text_content) = content.as_text()
    {
        // Should contain operation ID and status info (if executing async)
        // Or be empty if it completed inline due to automatic async behavior
        assert!(
            text_content.text.contains("id")
                || text_content.text.contains("started")
                || text_content.text.is_empty() // Success output for 'sleep 1'
        );
    }

    // 2. Use the await tool to check that async operations can be tracked - no timeout parameter needed
    let await_params = json!({});
    let await_call_params = CallToolRequestParams {
        name: Cow::Borrowed("await"),
        arguments: await_params.as_object().cloned(),
        task: None,
        meta: None,
    };

    let await_result = client.call_tool(await_call_params).await?;

    // The await should successfully track the async operation
    assert!(!await_result.content.is_empty());

    client.cancel().await?;
    Ok(())
}
