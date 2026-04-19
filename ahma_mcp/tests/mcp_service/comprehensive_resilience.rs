use ahma_common::timeouts::TestTimeouts;
use ahma_mcp::test_utils::in_process::create_in_process_mcp_empty;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use serde_json::json;

async fn call_tool_gracefully(
    client: &RunningService<RoleClient, ()>,
    tool_name: &str,
    args: serde_json::Value,
) {
    let mut call_param = CallToolRequestParams::new(tool_name.to_string());
    if let Some(arguments) = args.as_object().cloned() {
        call_param = call_param.with_arguments(arguments);
    }
    if let Ok(Ok(tool_result)) =
        tokio::time::timeout(TestTimeouts::scale_secs(15), client.call_tool(call_param)).await
    {
        assert!(!tool_result.content.is_empty());
    }
}

#[tokio::test]
async fn test_service_resilience_stress() -> Result<()> {
    let mcp = create_in_process_mcp_empty().await?;

    let operations = vec![
        ("status", json!({})),
        ("invalid_tool_123", json!({})),
        ("await", json!({})),
        ("another_invalid_tool", json!({"invalid": "args"})),
        ("status", json!({"id": "stress_test"})),
    ];

    for (tool_name, args) in operations {
        call_tool_gracefully(&mcp.client, tool_name, args).await;
    }

    let final_result = tokio::time::timeout(
        TestTimeouts::scale_secs(15),
        mcp.client.call_tool(
            CallToolRequestParams::new("status")
                .with_arguments(json!({}).as_object().cloned().unwrap_or_default()),
        ),
    )
    .await
    .map_err(|_| anyhow::anyhow!("final status call timed out"))??;
    assert!(!final_result.content.is_empty());

    Ok(())
}
