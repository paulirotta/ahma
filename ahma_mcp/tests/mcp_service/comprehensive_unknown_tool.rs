use ahma_mcp::test_utils::client::ClientBuilder;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::Map;

fn assert_error_in_text(text: &str) {
    let lower = text.to_lowercase();
    assert!(
        lower.contains("error")
            || lower.contains("not found")
            || lower.contains("unknown")
            || lower.contains("invalid"),
        "Expected error keywords in: {text}"
    );
}

#[tokio::test]
async fn test_unknown_tool_error_handling() -> Result<()> {
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;
    let call_param = CallToolRequestParams::new("unknown_tool").with_arguments(Map::new());

    if let Ok(tool_result) = client.call_tool(call_param).await
        && let Some(text) = tool_result.content.first().and_then(|c| c.as_text())
    {
        assert_error_in_text(&text.text);
    }

    client.cancel().await?;
    Ok(())
}
