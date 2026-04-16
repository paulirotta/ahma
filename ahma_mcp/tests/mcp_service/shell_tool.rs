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
async fn test_build_shell_subcommand_config_sync() -> Result<()> {
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
async fn test_build_shell_subcommand_config_async_mode() -> Result<()> {
    let mode = ahma_mcp::adapter::ExecutionMode::AsyncResultPush;
    let config = ahma_mcp::AhmaMcpService::build_shell_subcommand_config(Some(30), &mode);
    assert_eq!(config.name, "sandboxed_shell");
    assert_eq!(config.timeout_seconds, Some(30));
    assert_eq!(config.synchronous, Some(false));
    assert!(config.positional_args.is_some());
    assert!(config.options.is_some());
    Ok(())
}

#[tokio::test]
async fn test_build_shell_subcommand_config_no_timeout() -> Result<()> {
    let mode = ahma_mcp::adapter::ExecutionMode::Synchronous;
    let config = ahma_mcp::AhmaMcpService::build_shell_subcommand_config(None, &mode);
    assert_eq!(config.timeout_seconds, None);
    assert_eq!(config.synchronous, Some(true));
    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_missing_command() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(Map::new());

    let result = client.call_tool(call_param).await;
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_sync() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

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

    let mut args = Map::new();
    args.insert("command".to_string(), json!("echo 'hello async'"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_working_directory() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    // Use a tempdir so this test is fully isolated and doesn't compete with
    // cargo lock files when running under heavy nextest parallelism.
    let temp_dir = tempfile::tempdir()?;
    let temp_dir_str = temp_dir.path().to_string_lossy();

    let mut args = Map::new();
    // `pwd` is instant and has no external lock dependencies, unlike `cargo --version`.
    args.insert("command".to_string(), json!("pwd"));
    args.insert("working_directory".to_string(), json!(temp_dir_str));
    args.insert("execution_mode".to_string(), json!("Synchronous"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());
    // Verify the working directory was actually applied.
    if let Some(content) = result.content.first()
        && let Some(text) = content.as_text()
    {
        // On macOS, /var/folders/... may be a symlink to /private/var/folders/...
        // so compare using canonical forms.
        let actual = std::path::PathBuf::from(text.text.trim());
        let expected = dunce::canonicalize(temp_dir.path())?;
        let actual_canon = dunce::canonicalize(&actual).unwrap_or(actual);
        assert_eq!(
            actual_canon, expected,
            "shell should run in the requested working directory"
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_with_monitor_level_and_stream() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    let mut args = Map::new();
    args.insert("command".to_string(), json!("echo 'monitored'"));
    args.insert("monitor_level".to_string(), json!("info"));
    args.insert("monitor_stream".to_string(), json!("stdout"));
    args.insert("execution_mode".to_string(), json!("Synchronous"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());
    if let Some(content) = result.content.first()
        && let Some(text) = content.as_text()
    {
        assert!(text.text.contains("monitored"));
    }
    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_with_invalid_monitor_level_fallback() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    let mut args = Map::new();
    args.insert("command".to_string(), json!("echo 'fallback'"));
    args.insert("monitor_level".to_string(), json!("invalid_level"));
    args.insert("execution_mode".to_string(), json!("Synchronous"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_with_timeout_seconds() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    let mut args = Map::new();
    args.insert("command".to_string(), json!("echo 'timeout_test'"));
    args.insert("timeout_seconds".to_string(), json!(60));
    args.insert("execution_mode".to_string(), json!("Synchronous"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_sync_failing_command() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    let mut args = Map::new();
    args.insert(
        "command".to_string(),
        json!(if cfg!(windows) {
            "cmd /c \"exit 1\""
        } else {
            "false"
        }),
    );
    args.insert("execution_mode".to_string(), json!("Synchronous"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await;
    assert!(result.is_err());
    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_execution_mode_async_result_push() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    let mut args = Map::new();
    args.insert("command".to_string(), json!("echo 'async_push'"));
    args.insert("execution_mode".to_string(), json!("AsyncResultPush"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());
    if let Some(content) = result.content.first()
        && let Some(text) = content.as_text()
    {
        assert!(
            text.text.contains("op_") || text.text.contains("async_push"),
            "Should contain op ID or inline result: {}",
            text.text
        );
    }
    Ok(())
}

#[tokio::test]
async fn test_handle_sandboxed_shell_execution_mode_unknown_defaults_to_async() -> Result<()> {
    init_test_logging();
    let client = ClientBuilder::new().build().await?;

    let mut args = Map::new();
    args.insert("command".to_string(), json!("echo 'unknown_mode'"));
    args.insert("execution_mode".to_string(), json!("UnknownMode"));

    let call_param = CallToolRequestParams::new("sandboxed_shell").with_arguments(args);

    let result = client.call_tool(call_param).await?;
    assert!(!result.content.is_empty());
    Ok(())
}
