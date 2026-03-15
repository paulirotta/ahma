//! tests for await_tool.rs
use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::{Map, json};

#[tokio::test]
async fn test_generate_input_schema_for_wait() -> Result<()> {
    let (service, _tmp) = ahma_mcp::test_utils::client::setup_test_environment().await;
    let schema = service.generate_input_schema_for_wait();

    assert_eq!(schema.get("type").unwrap(), "object");
    let props = schema.get("properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("tools"));
    assert!(props.contains_key("id"));
    Ok(())
}

#[tokio::test]
async fn test_calculate_intelligent_timeout() -> Result<()> {
    let (service, _tmp) = ahma_mcp::test_utils::client::setup_test_environment().await;

    // By default with no ops, it should be 240.0
    let timeout = service.calculate_intelligent_timeout(&[]).await;
    assert_eq!(timeout, 600.0);
    Ok(())
}

#[tokio::test]
async fn test_handle_await_with_pending_ops() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    // Start a fast async command so it creates an operation
    let mut args = Map::new();
    args.insert(
        "command".to_string(),
        json!(if cfg!(windows) {
            "Start-Sleep -Seconds 6; Write-Output 'done waiting'"
        } else {
            "sleep 6 && echo 'done waiting'"
        }),
    );
    args.insert("execution_mode".to_string(), json!("Asynchronous"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await?;
    let content = result
        .content
        .first()
        .unwrap()
        .as_text()
        .unwrap()
        .text
        .clone();

    // Extract op_id from response
    let start_idx = content
        .find("op_")
        .unwrap_or_else(|| panic!("Could not find op_ in content: {}", content));
    let end_idx = content[start_idx..]
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(content.len() - start_idx);
    let op_id = &content[start_idx..start_idx + end_idx];

    assert!(!op_id.is_empty(), "Expected operation ID in: {}", content);

    // Now call await for this operation
    let mut await_args = Map::new();
    await_args.insert("id".to_string(), json!(op_id));

    let await_param = CallToolRequestParams::new("await").with_arguments(await_args);

    let await_result = client.call_tool(await_param).await?;
    assert!(!await_result.content.is_empty());

    let await_text = await_result
        .content
        .first()
        .unwrap()
        .as_text()
        .unwrap()
        .text
        .clone();
    assert!(
        await_text.contains("Completed") || await_text.contains("completed"),
        "Should show operation completed, got: {}",
        await_text
    );

    Ok(())
}

#[tokio::test]
async fn test_handle_await_with_tool_filter() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    // Start an async command
    let mut args = Map::new();
    args.insert(
        "command".to_string(),
        json!(if cfg!(windows) {
            "Start-Sleep -Seconds 6"
        } else {
            "sleep 6"
        }),
    );
    args.insert("execution_mode".to_string(), json!("Asynchronous"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let _ = client.call_tool(call_param).await?;

    // Call await for tools="sandboxed_shell"
    let mut await_args = Map::new();
    await_args.insert("tools".to_string(), json!("sandboxed_shell"));

    let await_param = CallToolRequestParams::new("await").with_arguments(await_args);

    let await_result = client.call_tool(await_param).await?;
    assert!(!await_result.content.is_empty());

    let await_text = await_result
        .content
        .first()
        .unwrap()
        .as_text()
        .unwrap()
        .text
        .clone();
    assert!(
        await_text.contains("Completed")
            || await_text.contains("completed")
            || await_text.contains("operations"),
        "Should wait correctly unblocking with operation info, got: {}",
        await_text
    );

    Ok(())
}

#[tokio::test]
async fn test_await_tool_no_pending_ops_after_timeout() -> Result<()> {
    // Tests wait falling back to recent completed ops
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    let mut args = Map::new();
    args.insert("command".to_string(), json!("echo 'quick'"));
    // Synchronous mode so it completes immediately
    args.insert("execution_mode".to_string(), json!("Synchronous"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);
    let _ = client.call_tool(call_param).await?;

    // Call await to query recently completed operations
    let mut await_args = Map::new();
    await_args.insert("tools".to_string(), json!("sandboxed_shell"));

    let await_param = CallToolRequestParams::new("await").with_arguments(await_args);

    let await_result = client.call_tool(await_param).await?;
    assert!(!await_result.content.is_empty());

    let await_text = await_result
        .content
        .first()
        .unwrap()
        .as_text()
        .unwrap()
        .text
        .clone();
    assert!(
        await_text.contains("these operations recently completed:")
            || await_text.contains("No pending")
    );

    Ok(())
}
