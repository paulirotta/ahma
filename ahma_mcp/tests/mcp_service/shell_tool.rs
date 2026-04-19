//! tests for shell_tool.rs
use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::{Result, anyhow};
use rmcp::model::CallToolRequestParams;
use serde_json::{Map, json};
use std::path::PathBuf;

/// Extract a filesystem path from shell current-directory output.
///
/// Shells format `pwd`/`Get-Location` output differently:
/// - Unix shells emit a single bare path line.
/// - PowerShell emits a table with a "Path" header, a dashed separator, and then the path.
///
/// This helper drops header noise and returns the last non-empty, non-header,
/// non-separator line as a `PathBuf`, preserving the original assertion semantics
/// without requiring a platform-specific command string.
///
/// Returns an error containing the full raw output when no path line can be found.
fn extract_shell_working_directory(output: &str) -> Result<PathBuf> {
    let path_line = output
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                // PowerShell table header
                && !line.eq_ignore_ascii_case("path")
                // PowerShell dashed separator (e.g. "----")
                && !line.chars().all(|c| c == '-')
        })
        .last()
        .ok_or_else(|| anyhow!("no path line found in shell output: {:?}", output))?;
    Ok(PathBuf::from(path_line))
}

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

#[cfg(test)]
mod extract_shell_working_directory_tests {
    use super::extract_shell_working_directory;

    #[test]
    fn plain_unix_path() {
        let out = "/home/user/project\n";
        assert_eq!(
            extract_shell_working_directory(out).unwrap(),
            std::path::PathBuf::from("/home/user/project")
        );
    }

    #[test]
    fn powershell_table_format() {
        // PowerShell `pwd` typically emits: "Path\n\n----\n\nC:\Users\user\dir\n\n"
        let out = "Path\n\n----\n\nC:\\Users\\user\\dir\n\n";
        assert_eq!(
            extract_shell_working_directory(out).unwrap(),
            std::path::PathBuf::from("C:\\Users\\user\\dir")
        );
    }

    #[test]
    fn trailing_newline_noise() {
        let out = "/tmp/my-dir\n\n";
        assert_eq!(
            extract_shell_working_directory(out).unwrap(),
            std::path::PathBuf::from("/tmp/my-dir")
        );
    }

    #[test]
    fn empty_output_returns_error() {
        assert!(extract_shell_working_directory("").is_err());
        assert!(extract_shell_working_directory("\n\n").is_err());
    }

    #[test]
    fn only_header_and_separator_returns_error() {
        // Edge case: PowerShell header with no path body
        assert!(extract_shell_working_directory("Path\n----\n").is_err());
    }
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

    // Create tempdir first so it's in scope for both the client (sandbox scope) and
    // the shell command (working_directory arg).  The server's cwd is the sandbox
    // root; pointing it at the tempdir ensures the shell's requested directory is
    // within the sandbox scope.
    let temp_dir = tempfile::tempdir()?;
    let client = ClientBuilder::new()
        .working_dir(temp_dir.path())
        .build()
        .await?;

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
        // On macOS, /var/folders/... may be a symlink to /private/var/folders/...;
        // on Windows, PowerShell pwd emits a formatted table rather than a bare path.
        // extract_shell_working_directory normalises both cases before canonicalization.
        let actual = extract_shell_working_directory(&text.text)?;
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
