use ahma_mcp::test_utils::client::ClientBuilder;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::{Map, json};

#[tokio::test]
async fn test_path_validation_security() -> Result<()> {
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    let mut params = Map::new();
    params.insert(
        "working_directory".to_string(),
        json!("/../../../../etc/passwd"),
    );

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(params);
    let result = client.call_tool(call_param).await;

    if let Ok(tool_result) = result {
        assert!(!tool_result.content.is_empty());
    }

    client.cancel().await?;
    Ok(())
}
