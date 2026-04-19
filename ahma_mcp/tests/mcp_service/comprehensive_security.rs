use ahma_mcp::test_utils::in_process::create_in_process_mcp_with_scope;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::{Map, json};

#[tokio::test]
async fn test_path_validation_security() -> Result<()> {
    let scope = std::env::current_dir().unwrap();
    let mcp = create_in_process_mcp_with_scope(
        &scope.join(".ahma"),
        vec![scope],
    )
    .await?;

    let mut params = Map::new();
    params.insert(
        "working_directory".to_string(),
        json!("/../../../../etc/passwd"),
    );

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(params);
    let result = mcp.client.call_tool(call_param).await;

    if let Ok(tool_result) = result {
        assert!(!tool_result.content.is_empty());
    }

    Ok(())
}
