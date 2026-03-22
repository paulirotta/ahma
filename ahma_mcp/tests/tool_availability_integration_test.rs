use ahma_mcp::config::load_tool_configs;
use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};
use ahma_mcp::tool_availability::{AvailabilitySummary, evaluate_tool_availability};
use anyhow::Result;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_tool_availability_integration_with_tempfile() -> Result<()> {
    // 1. Setup a temporary directory for our fake tool configs
    let temp_dir = TempDir::new()?;
    let tools_dir = temp_dir.path();

    // 2. Write an available tool config (echo)
    let available_json = r#"{
        "name": "echo_tool",
        "description": "Echoes text",
        "command": "echo",
        "availability_check": {
            "command": "echo",
            "args": ["hello"]
        }
    }"#;
    tokio::fs::write(tools_dir.join("echo.json"), available_json).await?;

    // 3. Write an unavailable tool config (does.not.exist.binary)
    let unavailable_json = r#"{
        "name": "missing_tool",
        "description": "A missing tool",
        "command": "does.not.exist.binary",
        "availability_check": {
            "command": "does.not.exist.binary",
            "args": ["--version"]
        },
        "install_instructions": "Please install does.not.exist.binary"
    }"#;
    tokio::fs::write(tools_dir.join("missing.json"), unavailable_json).await?;

    // 4. Load the tool configs from the temp directory exactly as the server does
    let raw_configs =
        load_tool_configs(&ahma_mcp::shell::cli::AppConfig::default(), Some(tools_dir)).await?;

    // We expect at least the 2 configs we just wrote (plus any global fallback configs default-loaded)
    assert!(
        raw_configs.len() >= 2,
        "Should have loaded at least two configs"
    );
    assert!(raw_configs.contains_key("echo_tool"));
    assert!(raw_configs.contains_key("missing_tool"));
    assert!(raw_configs.contains_key("missing_tool"));

    // 5. Evaluate availability
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let sandbox = ahma_mcp::sandbox::Sandbox::new_test();
    let working_dir = std::path::Path::new(".");

    let summary: AvailabilitySummary =
        evaluate_tool_availability(shell_pool, raw_configs, working_dir, &sandbox).await?;

    // 6. Assert standard expectations
    // - echo_tool should be enabled (and not in disabled_tools)
    let echo_config = summary.filtered_configs.get("echo_tool").unwrap();
    assert!(echo_config.enabled, "echo_tool should be enabled");

    // - missing_tool should be disabled and appear in disabled_tools
    let missing_config = summary.filtered_configs.get("missing_tool").unwrap();
    assert!(!missing_config.enabled, "missing_tool should be disabled");

    let disabled_names: Vec<&str> = summary
        .disabled_tools
        .iter()
        .map(|d| d.name.as_str())
        .collect();

    assert!(
        disabled_names.contains(&"missing_tool"),
        "missing_tool should be listed in disabled_tools"
    );

    assert!(
        !disabled_names.contains(&"echo_tool"),
        "echo_tool should NOT be listed in disabled_tools"
    );

    // Verify install instruction passed through
    let missing_tool_info = summary
        .disabled_tools
        .iter()
        .find(|t| t.name == "missing_tool")
        .unwrap();
    assert_eq!(
        missing_tool_info.install_instructions.as_deref(),
        Some("Please install does.not.exist.binary")
    );

    Ok(())
}
