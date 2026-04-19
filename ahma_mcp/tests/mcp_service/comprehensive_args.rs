use ahma_mcp::test_utils::in_process::create_in_process_mcp_from_dir;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;

#[tokio::test]
async fn test_argument_parsing_edge_cases() -> Result<()> {
    let mcp = create_in_process_mcp_from_dir(std::path::Path::new(".ahma")).await?;

    let empty_call_param = CallToolRequestParams::new("status");
    let empty_result = mcp.client.call_tool(empty_call_param).await?;
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

    if let Ok(tool_result) = mcp.client.call_tool(complex_call_param).await {
        assert!(!tool_result.content.is_empty());
    }

    Ok(())
}
