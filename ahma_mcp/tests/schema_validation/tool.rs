//! Validate all tool JSON files in /tools against the ToolConfig struct
use ahma_mcp::test_utils as common;

use ahma_mcp::config::load_tool_configs;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use common::fs::get_workspace_path;

#[tokio::test]
async fn test_all_tool_json_files_load_correctly() -> Result<()> {
    init_test_logging();
    let tools_dir = get_workspace_path(".ahma");
    let tool_configs = load_tool_configs(
        &ahma_mcp::shell::cli::AppConfig::default(),
        Some(&tools_dir),
    )
    .await;

    assert!(
        tool_configs.is_ok(),
        "Failed to load tool configs: {:#?}",
        tool_configs.err().unwrap()
    );

    let tool_configs = tool_configs.unwrap();
    assert!(!tool_configs.is_empty(), "No tool configs were loaded");

    println!(
        "Successfully loaded and validated {} tools:",
        tool_configs.len()
    );
    for (name, config) in tool_configs {
        println!("  - {}: {}", name, config.description);
    }

    Ok(())
}
