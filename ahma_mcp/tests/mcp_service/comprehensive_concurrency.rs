use ahma_mcp::test_utils::client::ClientBuilder;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::{Map, json};

#[tokio::test]
async fn test_concurrent_tool_execution() -> Result<()> {
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;
    let mut handles = vec![];

    for i in 0..5 {
        let client_clone = ClientBuilder::new().tools_dir(".ahma").build().await?;
        let handle = tokio::spawn(async move {
            let mut params = Map::new();
            params.insert("id".to_string(), json!(format!("concurrent_test_{}", i)));
            let call_param = CallToolRequestParams::new("status").with_arguments(params);

            let result = client_clone.call_tool(call_param).await;
            client_clone.cancel().await.ok();
            result
        });
        handles.push(handle);
    }

    for handle in handles {
        let result = handle.await.expect("Task should not panic");
        assert!(result.is_ok());
        assert!(!result.unwrap().content.is_empty());
    }

    client.cancel().await?;
    Ok(())
}
