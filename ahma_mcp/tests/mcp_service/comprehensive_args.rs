use ahma_mcp::test_utils::client::ClientBuilder;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;

#[tokio::test]
async fn test_argument_parsing_edge_cases() -> Result<()> {
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let empty_call_param = CallToolRequestParams::new("status");
    let empty_result = client.call_tool(empty_call_param).await?;
    assert!(!empty_result.content.is_empty());

    let complex_args = json!({
        "nested": {
            "array": [1, 2, 3],
            "object": { "key": "value" }
        },
        "tools": "cargo"
    });

    let complex_call_param = CallToolRequestParams::new("await")
        .with_arguments(complex_args.as_object().cloned().unwrap_or_default());

    if let Ok(tool_result) = client.call_tool(complex_call_param).await {
        assert!(!tool_result.content.is_empty());
    }

    client.cancel().await?;
    Ok(())
}
