//! tests for shell_tool.rs
use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::{Map, json};
#[tokio::test]
async fn test_generate_input_schema_for_sandboxed_shell() -> Result<()> {
    let (service, _tmp) = ahma_mcp::test_utils::client::setup_test_environment().await;
    let schema = service.generate_input_schema_for_sandboxed_shell();

    assert_eq!(schema.get("type").unwrap(), "object");
    let props = schema.get("properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("command"));
    assert!(props.contains_key("working_directory"));
    assert!(props.contains_key("monitor_level"));
    assert!(props.contains_key("monitor_stream"));

    let req = schema.get("required").unwrap().as_array().unwrap();
    assert_eq!(req[0], "command");
    Ok(())
}

#[tokio::test]
async fn test_build_shell_subcommand_config() -> Result<()> {
    let mode = ahma_mcp::adapter::ExecutionMode::Synchronous;
    let config = ahma_mcp::AhmaMcpService::build_shell_subcommand_config(Some(10), &mode);
    assert_eq!(config.name, "sandboxed_shell");
    assert_eq!(config.timeout_seconds, Some(10));
    assert_eq!(config.synchronous, Some(true));
    assert!(config.positional_args.is_some());
    assert!(config.options.is_some());
    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_missing_command() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    // Call sandboxed_shell with missing command argument
    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(Map::new());

    let result = client.call_tool(call_param).await;
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_sync() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    // Call sandboxed_shell synchronously
    let mut args = Map::new();
    args.insert("command".to_string(), json!("echo 'hello sync'"));
    args.insert("execution_mode".to_string(), json!("Synchronous"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    if let Some(content) = result.content.first()
        && let Some(text) = content.as_text()
    {
        assert!(
            text.text.contains("hello sync"),
            "Output should contain 'hello sync'"
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_async() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    // Call sandboxed_shell asynchronously
    let mut args = Map::new();
    args.insert("command".to_string(), json!("echo 'hello async'"));
    // default mode is async

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_working_directory() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    let current_dir = std::env::current_dir()?;
    let current_dir_str = current_dir.to_string_lossy();

    // Call sandboxed_shell
    let mut args = Map::new();
    // Using platform independent command to print working directory
    // Rust tests run on both Unix and Windows
    args.insert("command".to_string(), json!("cargo --version"));
    args.insert("working_directory".to_string(), json!(current_dir_str));
    args.insert("execution_mode".to_string(), json!("Synchronous"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    Ok(())
}
