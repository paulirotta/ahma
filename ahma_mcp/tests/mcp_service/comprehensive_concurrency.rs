use ahma_mcp::test_utils::client::setup_test_environment;
use anyhow::Result;
use serde_json::{Map, json};
use std::sync::Arc;

#[tokio::test]
async fn test_concurrent_tool_execution() -> Result<()> {
    let (service, _tmp) = setup_test_environment().await;
    let service = Arc::new(service);
    let mut handles = vec![];

    for i in 0..5 {
        let service_clone = Arc::clone(&service);
        let handle = tokio::spawn(async move {
            let mut params = Map::new();
            params.insert("id".to_string(), json!(format!("concurrent_test_{}", i)));
            service_clone.handle_status(params).await
        });
        handles.push(handle);
    }

    for handle in handles {
        let result = handle.await.expect("Task should not panic");
        assert!(result.is_ok());
        assert!(!result.unwrap().content.is_empty());
    }

    Ok(())
}
