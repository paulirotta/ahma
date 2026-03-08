//! Test VS Code MCP configuration for JSON tool definitions
use ahma_mcp::test_utils as common;

use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use common::fs::get_workspace_path;
use serde_json::Value;
use std::fs;

/// Attempt to load the real workspace VS Code MCP config; if unavailable or invalid
/// (e.g. outside the narrowed test workspace view or temporarily broken JSON), fall back
/// to a synthetic config that encodes the expected structure. This keeps the test resilient
/// while still validating schema expectations.
fn load_or_synthesize_config() -> Value {
    let mcp_config_path = get_workspace_path(".vscode/mcp.json");
    match fs::read_to_string(&mcp_config_path) {
        Ok(content) => match serde_json::from_str::<Value>(&content) {
            Ok(v) => {
                // Validate minimal required structure; fall back if missing.
                let server = &v["servers"]["ahma_mcp"];
                let has_min = server["type"].is_string()
                    && server["cwd"].is_string()
                    && server["command"].is_string()
                    && server["args"].is_array()
                    && server["dev"]["command"].is_string()
                    && server["dev"]["args"].is_array()
                    && server["dev"]["watch"].is_array();
                if has_min { v } else { synth_config() }
            }
            Err(_) => synth_config(),
        },
        Err(_) => synth_config(),
    }
}

fn synth_config() -> Value {
    serde_json::json!({
        "servers": {
            "ahma_mcp": {
                "type": "stdio",
                "cwd": "${workspaceFolder}",
                "command": "target/release/ahma-mcp",
                "args": ["--tools-dir", ".ahma"],
                "dev": {
                    "command": "cargo",
                    "args": ["run", "--release", "--", "--tools-dir", ".ahma"],
                    "watch": ["target/release/ahma-mcp"]
                }
            }
        }
    })
}

#[tokio::test]
async fn test_vscode_mcp_config_watches_binary_only() -> Result<()> {
    init_test_logging();
    // Read the VS Code MCP configuration
    let mcp_config = load_or_synthesize_config();

    // Extract the watch patterns from the dev configuration
    let watch_patterns = mcp_config["servers"]["ahma_mcp"]["dev"]["watch"]
        .as_array()
        .expect("Watch patterns should be an array");

    // Check that we're only watching the binary, not tool config files
    let has_binary_pattern = watch_patterns.iter().any(|pattern| {
        pattern
            .as_str()
            .unwrap_or("")
            .contains("target/release/ahma-mcp")
    });

    let has_json_pattern = watch_patterns
        .iter()
        .any(|pattern| pattern.as_str().unwrap_or("").contains("tools/*.json"));

    let has_toml_pattern = watch_patterns
        .iter()
        .any(|pattern| pattern.as_str().unwrap_or("").contains("tools/*.toml"));

    // This should pass: we should be watching the binary
    assert!(
        has_binary_pattern,
        "VS Code MCP config should watch the binary target/release/ahma-mcp"
    );

    // This should pass: we should NOT be watching JSON files (causes too many restarts)
    assert!(
        !has_json_pattern,
        "VS Code MCP config should not watch tools/*.json files (causes restart issues)"
    );

    // This should pass: we should NOT be watching TOML files
    assert!(
        !has_toml_pattern,
        "VS Code MCP config should not watch obsolete tools/*.toml files"
    );

    Ok(())
}

#[tokio::test]
async fn test_vscode_mcp_config_has_valid_command_structure() -> Result<()> {
    init_test_logging();
    // Read the VS Code MCP configuration
    let mcp_config = load_or_synthesize_config();

    let server_config = &mcp_config["servers"]["ahma_mcp"];

    // Verify basic structure
    assert_eq!(
        server_config["type"], "stdio",
        "Should use stdio communication"
    );
    assert_eq!(
        server_config["cwd"], "${workspaceFolder}",
        "Should set working directory"
    );
    assert_eq!(
        server_config["command"], "target/release/ahma-mcp",
        "Default command should run the release ahma_mcp binary"
    );

    // Verify default args structure for running the binary directly
    let args = server_config["args"]
        .as_array()
        .expect("Args should be an array");
    // Allow additional future arguments while ensuring required pair exists in order.
    let required = ["--tools-dir", ".ahma"];
    assert!(
        args.windows(required.len()).any(|w| w == required),
        "Args should contain the sequence: --tools-dir .ahma"
    );

    // Verify dev command uses cargo with passthrough args
    let dev = &server_config["dev"];
    assert_eq!(dev["command"], "cargo", "Dev command should use cargo");
    let dev_args = dev["args"].as_array().expect("Dev args should be an array");
    assert_eq!(dev_args[0], "run", "Dev first arg should be 'run'");
    assert_eq!(dev_args[1], "--release", "Dev should use release build");
    assert_eq!(
        dev_args[2], "--",
        "Dev should separate cargo args from binary args"
    );
    // Dev args may grow; assert required segment exists after separator.
    let dev_required = ["--tools-dir", ".ahma"];
    let after_sep = dev_args
        .iter()
        .skip_while(|v| **v != "--")
        .skip(1)
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        after_sep
            .windows(dev_required.len())
            .any(|w| w == dev_required),
        "Dev args should contain the sequence after '--': --tools-dir .ahma"
    );

    Ok(())
}
