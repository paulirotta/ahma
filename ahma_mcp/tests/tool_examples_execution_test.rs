//! Tool Examples Configuration Validation Tests
//!
//! This test module validates that all tool configuration examples in the examples/configs
//! directory are valid and conform to the MTDF schema. These tests run directly against
//! the validation logic rather than spawning cargo processes, making them fast and reliable.
//!
//! # Performance Note
//! Previous implementation used `cargo run --example` which was extremely slow on CI
//! (110+ seconds on Android runners) due to compilation overhead. This direct validation
//! approach runs in milliseconds.

use ahma_mcp::schema_validation::MtdfValidator;
use std::path::PathBuf;

/// Get the path to the .ahma directory
fn configs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".ahma")
}

/// Validate a tool configuration and return a structured result for assertions
struct ValidatedTool {
    name: String,
    command: String,
    enabled: bool,
    subcommand_count: usize,
}

fn validate_tool_config(config_file: &str) -> ValidatedTool {
    let config_path = configs_dir().join(config_file);
    let content = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", config_file, e));

    let validator = MtdfValidator::new();
    let config = validator
        .validate_tool_config(&config_path, &content)
        .unwrap_or_else(|errors| {
            let error_msgs: Vec<_> = errors
                .iter()
                .map(|e| format!("{}: {}", e.field_path, e.message))
                .collect();
            panic!(
                "Validation failed for {}: {}",
                config_file,
                error_msgs.join("; ")
            );
        });

    ValidatedTool {
        name: config.name,
        command: config.command,
        enabled: config.enabled,
        subcommand_count: config.subcommand.as_ref().map(|s| s.len()).unwrap_or(0),
    }
}

#[test]
fn test_rust_tool_config_valid() {
    let tool = validate_tool_config("rust.json");

    assert_eq!(tool.name, "cargo", "Name should be 'cargo'");
    assert_eq!(tool.command, "cargo", "Command should be 'cargo'");
    assert!(tool.enabled, "Tool should be enabled");
    assert!(tool.subcommand_count > 0, "Should have subcommands");
}

#[test]
fn test_file_tools_config_valid() {
    let tool = validate_tool_config("file-tools.json");

    assert_eq!(tool.name, "file-tools", "Name should be 'file-tools'");
    assert_eq!(tool.command, "/bin/sh", "Command should be '/bin/sh'");
    assert!(tool.enabled, "Tool should be enabled");
    assert!(tool.subcommand_count > 0, "Should have subcommands");
}

#[test]
fn test_gh_tool_config_valid() {
    let tool = validate_tool_config("gh.json");

    assert_eq!(tool.name, "gh", "Name should be 'gh'");
    assert_eq!(tool.command, "gh", "Command should be 'gh'");
    assert!(tool.enabled, "Tool should be enabled");
    assert!(tool.subcommand_count > 0, "Should have subcommands");
}

#[test]
fn test_git_tool_config_valid() {
    let tool = validate_tool_config("git.json");

    assert_eq!(tool.name, "git", "Name should be 'git'");
    assert_eq!(tool.command, "git", "Command should be 'git'");
    assert!(tool.enabled, "Tool should be enabled");
    assert!(tool.subcommand_count > 0, "Should have subcommands");
}

#[test]
fn test_kotlin_tool_config_valid() {
    let tool = validate_tool_config("kotlin.json");

    assert_eq!(tool.name, "kotlin", "Name should be 'kotlin'");
    assert_eq!(tool.command, "./gradlew", "Command should be './gradlew'");
    assert!(tool.enabled, "Tool should be enabled");
    assert!(tool.subcommand_count > 0, "Should have subcommands");
}

#[test]
fn test_python_tool_config_valid() {
    let tool = validate_tool_config("python.json");

    assert_eq!(tool.name, "python", "Name should be 'python'");
    assert_eq!(tool.command, "python", "Command should be 'python'");
    assert!(tool.enabled, "Tool should be enabled");
    assert!(tool.subcommand_count > 0, "Should have subcommands");
}

#[test]
fn test_all_example_configs_have_subcommands() {
    let config_files = [
        "rust.json",
        "file-tools.json",
        "gh.json",
        "git.json",
        "kotlin.json",
        "python.json",
    ];

    for config_file in &config_files {
        let tool = validate_tool_config(config_file);
        assert!(
            tool.subcommand_count > 0,
            "{} should have at least one subcommand, found {}",
            config_file,
            tool.subcommand_count
        );
    }
}
