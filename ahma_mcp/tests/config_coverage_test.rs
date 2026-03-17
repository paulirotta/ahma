//! Test coverage for config module
//!
//! These tests verify the serialization, deserialization, and loading
//! functionality of tool configurations.

use ahma_mcp::config::{
    AvailabilityCheck, CommandOption, ItemsSpec, SequenceStep, SubcommandConfig, ToolConfig,
    ToolHints, load_mcp_config, load_tool_configs_sync,
};
use clap::Parser;
use serde_json::json;
use tempfile::tempdir;

// ============================================================================
// ToolConfig Serialization/Deserialization Tests
// ============================================================================

#[test]
fn test_tool_config_minimal_deserialization() {
    let json = json!({
        "name": "echo",
        "description": "Echo a message",
        "command": "echo"
    });

    let config: ToolConfig = serde_json::from_value(json).unwrap();
    assert_eq!(config.name, "echo");
    assert_eq!(config.description, "Echo a message");
    assert_eq!(config.command, "echo");
    assert!(config.enabled); // default is true
    assert!(config.subcommand.is_none());
    assert!(config.timeout_seconds.is_none());
}

#[test]
fn test_tool_config_full_deserialization() {
    let json = json!({
        "name": "cargo",
        "description": "Rust package manager",
        "command": "cargo",
        "timeout_seconds": 300,
        "force_synchronous": true,
        "enabled": true,
        "guidance_key": "cargo_hints",
        "step_delay_ms": 100,
        "subcommand": [
            {
                "name": "build",
                "description": "Build the project",
                "enabled": true
            }
        ]
    });

    let config: ToolConfig = serde_json::from_value(json).unwrap();
    assert_eq!(config.name, "cargo");
    assert_eq!(config.timeout_seconds, Some(300));
    assert_eq!(config.synchronous, Some(true));
    assert_eq!(config.guidance_key, Some("cargo_hints".to_string()));
    assert_eq!(config.step_delay_ms, Some(100));
    assert!(config.subcommand.is_some());
    assert_eq!(config.subcommand.as_ref().unwrap().len(), 1);
}

#[test]
fn test_tool_config_disabled() {
    let json = json!({
        "name": "disabled_tool",
        "description": "A disabled tool",
        "command": "disabled",
        "enabled": false
    });

    let config: ToolConfig = serde_json::from_value(json).unwrap();
    assert!(!config.enabled);
}

#[test]
fn test_tool_config_with_sequence() {
    let json = json!({
        "name": "quality_check",
        "description": "Run quality checks",
        "command": "",
        "sequence": [
            {
                "tool": "cargo",
                "subcommand": "fmt",
                "args": {},
                "description": "Format code"
            },
            {
                "tool": "cargo",
                "subcommand": "clippy",
                "args": {"fix": true}
            }
        ]
    });

    let config: ToolConfig = serde_json::from_value(json).unwrap();
    let sequence = config.sequence.unwrap();
    assert_eq!(sequence.len(), 2);
    assert_eq!(sequence[0].tool, "cargo");
    assert_eq!(sequence[0].subcommand, "fmt");
    assert_eq!(sequence[0].description, Some("Format code".to_string()));
    assert_eq!(sequence[1].subcommand, "clippy");
}

#[test]
fn test_tool_config_with_availability_check() {
    let json = json!({
        "name": "rust",
        "description": "Rust compiler",
        "command": "rustc",
        "availability_check": {
            "command": "rustc",
            "args": ["--version"],
            "success_exit_codes": [0]
        },
        "install_instructions": "Install Rust via rustup: https://rustup.rs"
    });

    let config: ToolConfig = serde_json::from_value(json).unwrap();
    let check = config.availability_check.unwrap();
    assert_eq!(check.command, Some("rustc".to_string()));
    assert_eq!(check.args, vec!["--version"]);
    assert_eq!(check.success_exit_codes, Some(vec![0]));
    assert_eq!(
        config.install_instructions,
        Some("Install Rust via rustup: https://rustup.rs".to_string())
    );
}

#[test]
fn test_tool_config_serialization_roundtrip() {
    let config = ToolConfig {
        name: "test".to_string(),
        description: "Test tool".to_string(),
        command: "test_cmd".to_string(),
        subcommand: None,
        input_schema: None,
        timeout_seconds: Some(60),
        synchronous: Some(false),
        hints: ToolHints::default(),
        enabled: true,
        guidance_key: None,
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
        monitor_level: None,
        monitor_stream: None,
        tool_type: None,
        livelog: None,
    };

    let serialized = serde_json::to_string(&config).unwrap();
    let deserialized: ToolConfig = serde_json::from_str(&serialized).unwrap();

    assert_eq!(config.name, deserialized.name);
    assert_eq!(config.description, deserialized.description);
    assert_eq!(config.timeout_seconds, deserialized.timeout_seconds);
}

// ============================================================================
// SubcommandConfig Tests
// ============================================================================

#[test]
fn test_subcommand_config_basic() {
    let json = json!({
        "name": "build",
        "description": "Build the project",
        "enabled": true
    });

    let config: SubcommandConfig = serde_json::from_value(json).unwrap();
    assert_eq!(config.name, "build");
    assert!(config.enabled);
    assert!(config.options.is_none());
}

#[test]
fn test_subcommand_config_with_options() {
    let json = json!({
        "name": "run",
        "description": "Run the project",
        "enabled": true,
        "options": [
            {
                "name": "release",
                "type": "boolean",
                "description": "Build in release mode"
            }
        ]
    });

    let config: SubcommandConfig = serde_json::from_value(json).unwrap();
    let options = config.options.unwrap();
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].name, "release");
    assert_eq!(options[0].option_type, "boolean");
}

#[test]
fn test_subcommand_config_with_nested_subcommands() {
    let json = json!({
        "name": "git",
        "description": "Git commands",
        "enabled": true,
        "subcommand": [
            {
                "name": "commit",
                "description": "Commit changes",
                "enabled": true
            },
            {
                "name": "push",
                "description": "Push changes",
                "enabled": true
            }
        ]
    });

    let config: SubcommandConfig = serde_json::from_value(json).unwrap();
    let nested = config.subcommand.unwrap();
    assert_eq!(nested.len(), 2);
    assert_eq!(nested[0].name, "commit");
    assert_eq!(nested[1].name, "push");
}

#[test]
fn test_subcommand_config_with_positional_args() {
    let json = json!({
        "name": "echo",
        "description": "Echo text",
        "enabled": true,
        "positional_args": [
            {
                "name": "message",
                "type": "string",
                "description": "The message to echo",
                "required": true
            }
        ]
    });

    let config: SubcommandConfig = serde_json::from_value(json).unwrap();
    let pos_args = config.positional_args.unwrap();
    assert_eq!(pos_args.len(), 1);
    assert_eq!(pos_args[0].name, "message");
    assert_eq!(pos_args[0].required, Some(true));
}

// ============================================================================
// CommandOption Tests
// ============================================================================

#[test]
fn test_command_option_basic() {
    let json = json!({
        "name": "verbose",
        "type": "boolean"
    });

    let option: CommandOption = serde_json::from_value(json).unwrap();
    assert_eq!(option.name, "verbose");
    assert_eq!(option.option_type, "boolean");
    assert!(option.description.is_none());
    assert!(option.required.is_none());
}

#[test]
fn test_command_option_full() {
    let json = json!({
        "name": "output",
        "type": "string",
        "description": "Output file path",
        "required": true,
        "format": "path",
        "file_arg": true,
        "file_flag": "-o",
        "alias": "out"
    });

    let option: CommandOption = serde_json::from_value(json).unwrap();
    assert_eq!(option.name, "output");
    assert_eq!(option.option_type, "string");
    assert_eq!(option.description, Some("Output file path".to_string()));
    assert_eq!(option.required, Some(true));
    assert_eq!(option.format, Some("path".to_string()));
    assert_eq!(option.file_arg, Some(true));
    assert_eq!(option.file_flag, Some("-o".to_string()));
    assert_eq!(option.alias, Some("out".to_string()));
}

#[test]
fn test_command_option_array_with_items() {
    let json = json!({
        "name": "files",
        "type": "array",
        "description": "List of files",
        "items": {
            "type": "string",
            "format": "path",
            "description": "File path"
        }
    });

    let option: CommandOption = serde_json::from_value(json).unwrap();
    assert_eq!(option.option_type, "array");
    let items = option.items.unwrap();
    assert_eq!(items.item_type, "string");
    assert_eq!(items.format, Some("path".to_string()));
    assert_eq!(items.description, Some("File path".to_string()));
}

// ============================================================================
// ToolHints Tests
// ============================================================================

#[test]
fn test_tool_hints_default() {
    let hints = ToolHints::default();
    assert!(hints.build.is_none());
    assert!(hints.test.is_none());
    assert!(hints.dependencies.is_none());
    assert!(hints.clean.is_none());
    assert!(hints.run.is_none());
    assert!(hints.custom.is_none());
}

#[test]
fn test_tool_hints_full() {
    let json = json!({
        "build": "Use --release for production builds",
        "test": "Run with --no-capture to see output",
        "dependencies": "Use cargo add to add dependencies",
        "clean": "Removes the target directory",
        "run": "Use -- to pass args to the binary",
        "custom": {
            "audit": "Check for security vulnerabilities",
            "fmt": "Format code with rustfmt"
        }
    });

    let hints: ToolHints = serde_json::from_value(json).unwrap();
    assert_eq!(
        hints.build,
        Some("Use --release for production builds".to_string())
    );
    assert_eq!(
        hints.test,
        Some("Run with --no-capture to see output".to_string())
    );
    let custom = hints.custom.unwrap();
    assert_eq!(
        custom.get("audit"),
        Some(&"Check for security vulnerabilities".to_string())
    );
    assert_eq!(
        custom.get("fmt"),
        Some(&"Format code with rustfmt".to_string())
    );
}

// ============================================================================
// AvailabilityCheck Tests
// ============================================================================

#[test]
fn test_availability_check_default() {
    let check = AvailabilityCheck::default();
    assert!(check.command.is_none());
    assert!(check.args.is_empty());
    assert!(check.working_directory.is_none());
    assert!(check.success_exit_codes.is_none());
    assert!(!check.skip_subcommand_args);
}

#[test]
fn test_availability_check_full() {
    let json = json!({
        "command": "node",
        "args": ["--version"],
        "working_directory": "/tmp",
        "success_exit_codes": [0, 1],
        "skip_subcommand_args": true
    });

    let check: AvailabilityCheck = serde_json::from_value(json).unwrap();
    assert_eq!(check.command, Some("node".to_string()));
    assert_eq!(check.args, vec!["--version"]);
    assert_eq!(check.working_directory, Some("/tmp".to_string()));
    assert_eq!(check.success_exit_codes, Some(vec![0, 1]));
    assert!(check.skip_subcommand_args);
}

// ============================================================================
// SequenceStep Tests
// ============================================================================

#[test]
fn test_sequence_step_minimal() {
    let json = json!({
        "tool": "cargo",
        "subcommand": "build"
    });

    let step: SequenceStep = serde_json::from_value(json).unwrap();
    assert_eq!(step.tool, "cargo");
    assert_eq!(step.subcommand, "build");
    assert!(step.args.is_empty());
    assert!(step.description.is_none());
}

#[test]
fn test_sequence_step_with_args() {
    let json = json!({
        "tool": "cargo",
        "subcommand": "build",
        "args": {
            "release": true,
            "target": "x86_64-unknown-linux-gnu"
        },
        "description": "Build release for Linux"
    });

    let step: SequenceStep = serde_json::from_value(json).unwrap();
    assert_eq!(step.args.get("release"), Some(&json!(true)));
    assert_eq!(
        step.args.get("target"),
        Some(&json!("x86_64-unknown-linux-gnu"))
    );
    assert_eq!(
        step.description,
        Some("Build release for Linux".to_string())
    );
}

// ============================================================================
// ItemsSpec Tests
// ============================================================================

#[test]
fn test_items_spec_basic() {
    let json = json!({
        "type": "string"
    });

    let spec: ItemsSpec = serde_json::from_value(json).unwrap();
    assert_eq!(spec.item_type, "string");
    assert!(spec.format.is_none());
    assert!(spec.description.is_none());
}

#[test]
fn test_items_spec_full() {
    let json = json!({
        "type": "string",
        "format": "path",
        "description": "A file path"
    });

    let spec: ItemsSpec = serde_json::from_value(json).unwrap();
    assert_eq!(spec.item_type, "string");
    assert_eq!(spec.format, Some("path".to_string()));
    assert_eq!(spec.description, Some("A file path".to_string()));
}

// ============================================================================
// load_tool_configs Tests
// ============================================================================

#[test]
fn test_load_tool_configs_empty_directory() {
    let temp_dir = tempdir().unwrap();
    let _configs = load_tool_configs_sync(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .unwrap();
    // May also load from examples directory, so can't assert empty
    // Just verify it doesn't error
}

#[test]
fn test_load_tool_configs_nonexistent_directory() {
    let nonexistent = std::path::PathBuf::from("/nonexistent/path/that/does/not/exist");
    let _configs = load_tool_configs_sync(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(&nonexistent),
    )
    .unwrap();
    // May still load from examples directory, so can't assert empty
    // Just verify it doesn't error
}

#[test]
fn test_load_tool_configs_single_tool() {
    let temp_dir = tempdir().unwrap();
    let tool_path = temp_dir.path().join("echo.json");

    std::fs::write(
        &tool_path,
        r#"{
        "name": "echo",
        "description": "Echo a message",
        "command": "echo"
    }"#,
    )
    .unwrap();

    let configs = load_tool_configs_sync(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .unwrap();
    // May also load from examples directory, so check for at least our tool
    assert!(configs.contains_key("echo"));
    assert_eq!(configs["echo"].command, "echo");
    assert!(!configs.is_empty());
}

#[test]
fn test_load_tool_configs_multiple_tools() {
    let temp_dir = tempdir().unwrap();

    std::fs::write(
        temp_dir.path().join("echo.json"),
        r#"{"name": "echo", "description": "Echo", "command": "echo"}"#,
    )
    .unwrap();

    std::fs::write(
        temp_dir.path().join("cat.json"),
        r#"{"name": "cat", "description": "Concatenate", "command": "cat"}"#,
    )
    .unwrap();

    let configs = load_tool_configs_sync(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .unwrap();
    // May also load from examples directory, so check for at least our 2 tools
    assert!(configs.contains_key("echo"));
    assert!(configs.contains_key("cat"));
    assert!(configs.len() >= 2);
}

#[test]
fn test_load_tool_configs_includes_disabled_tools() {
    let temp_dir = tempdir().unwrap();

    std::fs::write(
        temp_dir.path().join("disabled.json"),
        r#"{"name": "disabled_tool", "description": "Disabled", "command": "disabled", "enabled": false}"#,
    )
    .unwrap();

    let configs = load_tool_configs_sync(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .unwrap();
    // May also load from examples directory, so check for at least our disabled tool
    assert!(configs.contains_key("disabled_tool"));
    assert!(!configs.get("disabled_tool").unwrap().enabled);
    assert!(!configs.is_empty());
}

#[test]
fn test_load_tool_configs_skips_non_json_files() {
    let temp_dir = tempdir().unwrap();

    std::fs::write(
        temp_dir.path().join("echo.json"),
        r#"{"name": "echo", "description": "Echo", "command": "echo"}"#,
    )
    .unwrap();

    std::fs::write(temp_dir.path().join("readme.txt"), "This is a readme").unwrap();

    std::fs::write(
        temp_dir.path().join("config.yaml"),
        "name: not_loaded\ncommand: ignored",
    )
    .unwrap();

    let configs = load_tool_configs_sync(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .unwrap();
    // May also load from examples directory, so check for at least our echo tool
    assert!(configs.contains_key("echo"));
    assert!(!configs.is_empty());
}

#[test]
fn test_load_tool_configs_handles_invalid_json() {
    let temp_dir = tempdir().unwrap();

    // Valid tool
    std::fs::write(
        temp_dir.path().join("valid.json"),
        r#"{"name": "valid", "description": "Valid", "command": "valid"}"#,
    )
    .unwrap();

    // Invalid JSON (missing quotes)
    std::fs::write(temp_dir.path().join("invalid.json"), "{name: broken}").unwrap();

    let configs = load_tool_configs_sync(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .unwrap();
    // Should load valid tool, skip invalid
    // May also load from examples directory
    assert!(configs.contains_key("valid"));
    assert!(!configs.is_empty());
}

#[test]
fn test_load_tool_configs_reserved_name_await_fails() {
    let temp_dir = tempdir().unwrap();

    std::fs::write(
        temp_dir.path().join("await.json"),
        r#"{"name": "await", "description": "Reserved", "command": "await"}"#,
    )
    .unwrap();

    let result = load_tool_configs_sync(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("await"));
    assert!(err.contains("conflicts with a hardcoded system tool"));
}

#[test]
fn test_load_tool_configs_reserved_name_status_fails() {
    let temp_dir = tempdir().unwrap();

    std::fs::write(
        temp_dir.path().join("status.json"),
        r#"{"name": "status", "description": "Reserved", "command": "status"}"#,
    )
    .unwrap();

    let result = load_tool_configs_sync(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("status"));
    assert!(err.contains("conflicts with a hardcoded system tool"));
}

// ============================================================================
// load_mcp_config Tests
// ============================================================================

#[tokio::test]
async fn test_load_mcp_config_nonexistent_file() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("nonexistent.json");

    let config = load_mcp_config(&config_path).await.unwrap();
    assert!(config.servers.is_empty());
}

#[tokio::test]
async fn test_load_mcp_config_empty_servers() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("mcp.json");

    std::fs::write(&config_path, r#"{"servers": {}}"#).unwrap();

    let config = load_mcp_config(&config_path).await.unwrap();
    assert!(config.servers.is_empty());
}

#[tokio::test]
async fn test_load_mcp_config_child_process_server() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("mcp.json");

    std::fs::write(
        &config_path,
        r#"{
        "servers": {
            "test_server": {
                "type": "child_process",
                "command": "node",
                "args": ["server.js"]
            }
        }
    }"#,
    )
    .unwrap();

    let config = load_mcp_config(&config_path).await.unwrap();
    assert!(config.servers.contains_key("test_server"));
}

#[tokio::test]
async fn test_load_mcp_config_http_server() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("mcp.json");

    std::fs::write(
        &config_path,
        r#"{
        "servers": {
            "http_server": {
                "type": "http",
                "url": "http://localhost:8080",
                "atlassian_client_id": "client123",
                "atlassian_client_secret": "secret456"
            }
        }
    }"#,
    )
    .unwrap();

    let config = load_mcp_config(&config_path).await.unwrap();
    assert!(config.servers.contains_key("http_server"));
}

#[tokio::test]
async fn test_load_mcp_config_invalid_json() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("mcp.json");

    std::fs::write(&config_path, "not valid json").unwrap();

    let result = load_mcp_config(&config_path).await;
    assert!(result.is_err());
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_tool_config_unknown_field_fails() {
    let json = json!({
        "name": "test",
        "description": "Test",
        "command": "test",
        "unknown_field": "should fail"
    });

    let result: Result<ToolConfig, _> = serde_json::from_value(json);
    assert!(result.is_err());
}

#[test]
fn test_subcommand_config_unknown_field_fails() {
    let json = json!({
        "name": "test",
        "description": "Test",
        "enabled": true,
        "extra_field": "should fail"
    });

    let result: Result<SubcommandConfig, _> = serde_json::from_value(json);
    assert!(result.is_err());
}

#[test]
fn test_command_option_unknown_field_fails() {
    let json = json!({
        "name": "test",
        "type": "string",
        "unknown": "should fail"
    });

    let result: Result<CommandOption, _> = serde_json::from_value(json);
    assert!(result.is_err());
}

// ============================================================================
// Async load_tool_configs Tests
// ============================================================================

#[tokio::test]
async fn test_async_load_tool_configs_empty_directory() {
    use ahma_mcp::config::load_tool_configs;
    let temp_dir = tempdir().unwrap();
    let configs = load_tool_configs(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .await
    .unwrap();
    // Note: With multi-directory support, this may load tools from examples directory.
    // When the empty temp_dir is passed, the loader also checks .ahma,
    // so configs may not be empty. This is expected behavior for development/testing.
    // The important thing is that it doesn't fail.
    let _num_configs = configs.len();
    // Just verify it doesn't error - configs may include example tools
}

#[tokio::test]
async fn test_async_load_tool_configs_nonexistent_directory() {
    use ahma_mcp::config::load_tool_configs;
    let nonexistent = std::path::PathBuf::from("/nonexistent/tools/dir");
    let _configs = load_tool_configs(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(nonexistent.as_path()),
    )
    .await
    .unwrap();
    // May still load from examples directory, so can't assert empty
    // Just verify it doesn't error
}

#[tokio::test]
async fn test_async_load_tool_configs_single_tool() {
    use ahma_mcp::config::load_tool_configs;
    let temp_dir = tempdir().unwrap();
    let tool_json = json!({
        "name": "async_test",
        "description": "Async test tool",
        "command": "echo"
    });
    std::fs::write(
        temp_dir.path().join("async_test.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let configs = load_tool_configs(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .await
    .unwrap();
    // May also load from examples directory, so check for at least our tool
    assert!(configs.contains_key("async_test"));
    assert!(!configs.is_empty());
}

#[tokio::test]
async fn test_async_load_tool_configs_multiple_tools() {
    use ahma_mcp::config::load_tool_configs;
    let temp_dir = tempdir().unwrap();

    for i in 1..=3 {
        let tool_json = json!({
            "name": format!("tool_{}", i),
            "description": format!("Tool {}", i),
            "command": "echo"
        });
        std::fs::write(
            temp_dir.path().join(format!("tool_{}.json", i)),
            serde_json::to_string_pretty(&tool_json).unwrap(),
        )
        .unwrap();
    }

    let configs = load_tool_configs(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .await
    .unwrap();
    // May also load from examples directory, so check for at least our 3 tools
    assert!(configs.contains_key("tool_1"));
    assert!(configs.contains_key("tool_2"));
    assert!(configs.contains_key("tool_3"));
    assert!(configs.len() >= 3);
}

#[tokio::test]
async fn test_async_load_tool_configs_includes_disabled_tools() {
    use ahma_mcp::config::load_tool_configs;
    let temp_dir = tempdir().unwrap();
    let tool_json = json!({
        "name": "disabled_async",
        "description": "Disabled tool",
        "command": "disabled",
        "enabled": false
    });
    std::fs::write(
        temp_dir.path().join("disabled.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let configs = load_tool_configs(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .await
    .unwrap();
    // May also load from examples directory, so check for at least our disabled tool
    assert!(configs.contains_key("disabled_async"));
    assert!(!configs.get("disabled_async").unwrap().enabled);
    assert!(!configs.is_empty());
}

#[tokio::test]
async fn test_async_load_tool_configs_skips_non_json_files() {
    use ahma_mcp::config::load_tool_configs;
    let temp_dir = tempdir().unwrap();

    // Write a non-JSON file
    std::fs::write(temp_dir.path().join("readme.txt"), "Not a tool").unwrap();

    // Write a valid JSON tool
    let tool_json = json!({
        "name": "valid_tool",
        "description": "Valid tool",
        "command": "echo"
    });
    std::fs::write(
        temp_dir.path().join("valid.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let configs = load_tool_configs(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .await
    .unwrap();
    // May also load from examples directory, so check for at least our valid tool
    assert!(configs.contains_key("valid_tool"));
    assert!(!configs.is_empty());
}

#[tokio::test]
async fn test_async_load_tool_configs_handles_invalid_json() {
    use ahma_mcp::config::load_tool_configs;
    let temp_dir = tempdir().unwrap();

    // Write invalid JSON
    std::fs::write(temp_dir.path().join("invalid.json"), "not valid json {").unwrap();

    // Write a valid tool
    let tool_json = json!({
        "name": "valid_after_invalid",
        "description": "Valid tool",
        "command": "echo"
    });
    std::fs::write(
        temp_dir.path().join("valid.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let configs = load_tool_configs(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .await
    .unwrap();
    // Invalid JSON should be skipped; valid tool should be loaded
    // May also load from examples directory
    assert!(configs.contains_key("valid_after_invalid"));
    assert!(!configs.is_empty());
}

#[tokio::test]
async fn test_async_load_tool_configs_reserved_name_await_fails() {
    use ahma_mcp::config::load_tool_configs;
    let temp_dir = tempdir().unwrap();
    let tool_json = json!({
        "name": "await",
        "description": "Conflicts with system tool",
        "command": "echo"
    });
    std::fs::write(
        temp_dir.path().join("await.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let result = load_tool_configs(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("await"));
    assert!(err_msg.contains("reserved") || err_msg.contains("conflicts"));
}

#[tokio::test]
async fn test_async_load_tool_configs_reserved_name_status_fails() {
    use ahma_mcp::config::load_tool_configs;
    let temp_dir = tempdir().unwrap();
    let tool_json = json!({
        "name": "status",
        "description": "Conflicts with system tool",
        "command": "echo"
    });
    std::fs::write(
        temp_dir.path().join("status.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let result = load_tool_configs(
        &ahma_mcp::shell::cli::Cli::try_parse_from(["ahma_mcp"]).unwrap(),
        Some(temp_dir.path()),
    )
    .await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("status"));
}
