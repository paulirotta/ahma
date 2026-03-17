//! Unit tests for shell/cli.rs pure functions
//!
//! These tests cover the public helper functions in the CLI module:
//! - find_matching_tool
//! - find_tool_config
//! - resolve_cli_subcommand

use ahma_mcp::config::{SubcommandConfig, ToolConfig};
use ahma_mcp::shell::resolution::{find_matching_tool, find_tool_config, resolve_cli_subcommand};
use std::collections::HashMap;

// ============= Helper Functions =============

fn create_test_tool_config(name: &str, command: &str) -> ToolConfig {
    ToolConfig {
        name: name.to_string(),
        description: format!("{} tool", name),
        command: command.to_string(),
        subcommand: None,
        input_schema: None,
        timeout_seconds: None,
        synchronous: None,
        hints: Default::default(),
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
    }
}

fn create_tool_config_with_subcommands(
    name: &str,
    command: &str,
    subcommands: Vec<SubcommandConfig>,
) -> ToolConfig {
    ToolConfig {
        name: name.to_string(),
        description: format!("{} tool", name),
        command: command.to_string(),
        subcommand: Some(subcommands),
        input_schema: None,
        timeout_seconds: None,
        synchronous: None,
        hints: Default::default(),
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
    }
}

fn create_subcommand(name: &str) -> SubcommandConfig {
    SubcommandConfig {
        name: name.to_string(),
        description: format!("{} subcommand", name),
        timeout_seconds: None,
        enabled: true,
        subcommand: None,
        options: None,
        positional_args: None,
        positional_args_first: None,
        synchronous: None,
        guidance_key: None,
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    }
}

fn create_subcommand_with_nested(name: &str, nested: Vec<SubcommandConfig>) -> SubcommandConfig {
    SubcommandConfig {
        name: name.to_string(),
        description: format!("{} subcommand", name),
        timeout_seconds: None,
        enabled: true,
        subcommand: Some(nested),
        options: None,
        positional_args: None,
        positional_args_first: None,
        synchronous: None,
        guidance_key: None,
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    }
}

// ============= find_matching_tool Tests =============

#[test]
fn test_find_matching_tool_exact_match() {
    let mut configs = HashMap::new();
    configs.insert(
        "cargo".to_string(),
        create_test_tool_config("cargo", "cargo"),
    );

    let result = find_matching_tool(&configs, "cargo");
    assert!(result.is_ok());
    let (key, config) = result.unwrap();
    assert_eq!(key, "cargo");
    assert_eq!(config.name, "cargo");
}

#[test]
fn test_find_matching_tool_prefix_match() {
    let mut configs = HashMap::new();
    configs.insert(
        "cargo".to_string(),
        create_test_tool_config("cargo", "cargo"),
    );

    let result = find_matching_tool(&configs, "cargo_build");
    assert!(result.is_ok());
    let (key, _) = result.unwrap();
    assert_eq!(key, "cargo");
}

#[test]
fn test_find_matching_tool_longest_prefix_wins() {
    let mut configs = HashMap::new();
    configs.insert(
        "cargo".to_string(),
        create_test_tool_config("cargo", "cargo"),
    );
    configs.insert(
        "cargo_build".to_string(),
        create_test_tool_config("cargo_build", "cargo build"),
    );

    let result = find_matching_tool(&configs, "cargo_build_release");
    assert!(result.is_ok());
    let (key, _) = result.unwrap();
    assert_eq!(key, "cargo_build");
}

#[test]
fn test_find_matching_tool_no_match() {
    let mut configs = HashMap::new();
    configs.insert(
        "cargo".to_string(),
        create_test_tool_config("cargo", "cargo"),
    );

    let result = find_matching_tool(&configs, "npm_install");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No matching tool"));
}

#[test]
fn test_find_matching_tool_disabled_tool_ignored() {
    let mut configs = HashMap::new();
    let mut disabled_config = create_test_tool_config("cargo", "cargo");
    disabled_config.enabled = false;
    configs.insert("cargo".to_string(), disabled_config);

    let result = find_matching_tool(&configs, "cargo_build");
    assert!(result.is_err());
}

#[test]
fn test_find_matching_tool_empty_configs() {
    let configs = HashMap::new();

    let result = find_matching_tool(&configs, "anything");
    assert!(result.is_err());
}

// ============= find_tool_config Tests =============

#[test]
fn test_find_tool_config_by_key() {
    let mut configs = HashMap::new();
    configs.insert(
        "cargo".to_string(),
        create_test_tool_config("cargo", "cargo"),
    );

    let result = find_tool_config(&configs, "cargo");
    assert!(result.is_some());
    let (key, config) = result.unwrap();
    assert_eq!(key, "cargo");
    assert_eq!(config.name, "cargo");
}

#[test]
fn test_find_tool_config_by_name() {
    let mut configs = HashMap::new();
    let mut config = create_test_tool_config("Cargo Build Tool", "cargo");
    config.name = "cargo_build".to_string();
    configs.insert("cargo".to_string(), config);

    // Search by name field
    let result = find_tool_config(&configs, "cargo_build");
    assert!(result.is_some());
}

#[test]
fn test_find_tool_config_not_found() {
    let mut configs = HashMap::new();
    configs.insert(
        "cargo".to_string(),
        create_test_tool_config("cargo", "cargo"),
    );

    let result = find_tool_config(&configs, "nonexistent");
    assert!(result.is_none());
}

#[test]
fn test_find_tool_config_prefers_exact_key_match() {
    let mut configs = HashMap::new();

    let mut config1 = create_test_tool_config("cargo", "cargo");
    config1.name = "cargo_tool".to_string();
    configs.insert("cargo".to_string(), config1);

    // Exact key match should win
    let result = find_tool_config(&configs, "cargo");
    assert!(result.is_some());
    let (key, _) = result.unwrap();
    assert_eq!(key, "cargo");
}

// ============= resolve_cli_subcommand Tests =============

#[test]
fn test_resolve_cli_subcommand_default() {
    let config = create_tool_config_with_subcommands(
        "cargo",
        "cargo",
        vec![create_subcommand("default"), create_subcommand("build")],
    );

    let result = resolve_cli_subcommand("cargo", &config, "cargo", None);
    assert!(result.is_ok());
    let (subcommand_config, command_parts) = result.unwrap();
    assert_eq!(subcommand_config.name, "default");
    assert_eq!(command_parts, vec!["cargo"]);
}

#[test]
fn test_resolve_cli_subcommand_explicit() {
    let config = create_tool_config_with_subcommands(
        "cargo",
        "cargo",
        vec![create_subcommand("default"), create_subcommand("build")],
    );

    let result = resolve_cli_subcommand("cargo", &config, "cargo_build", None);
    assert!(result.is_ok());
    let (subcommand_config, command_parts) = result.unwrap();
    assert_eq!(subcommand_config.name, "build");
    assert_eq!(command_parts, vec!["cargo", "build"]);
}

#[test]
fn test_resolve_cli_subcommand_with_override() {
    let config = create_tool_config_with_subcommands(
        "cargo",
        "cargo",
        vec![create_subcommand("default"), create_subcommand("test")],
    );

    let result = resolve_cli_subcommand("cargo", &config, "cargo_build", Some("test"));
    assert!(result.is_ok());
    let (subcommand_config, _) = result.unwrap();
    assert_eq!(subcommand_config.name, "test");
}

#[test]
fn test_resolve_cli_subcommand_nested() {
    let nested = vec![create_subcommand("release"), create_subcommand("debug")];
    let config = create_tool_config_with_subcommands(
        "cargo",
        "cargo",
        vec![
            create_subcommand("default"),
            create_subcommand_with_nested("build", nested),
        ],
    );

    let result = resolve_cli_subcommand("cargo", &config, "cargo_build_release", None);
    assert!(result.is_ok());
    let (subcommand_config, command_parts) = result.unwrap();
    assert_eq!(subcommand_config.name, "release");
    assert_eq!(command_parts, vec!["cargo", "build", "release"]);
}

#[test]
fn test_resolve_cli_subcommand_not_found() {
    let config = create_tool_config_with_subcommands(
        "cargo",
        "cargo",
        vec![create_subcommand("default"), create_subcommand("build")],
    );

    let result = resolve_cli_subcommand("cargo", &config, "cargo_nonexistent", None);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_resolve_cli_subcommand_disabled_subcommand_ignored() {
    let mut disabled = create_subcommand("build");
    disabled.enabled = false;

    let config = create_tool_config_with_subcommands(
        "cargo",
        "cargo",
        vec![create_subcommand("default"), disabled],
    );

    let result = resolve_cli_subcommand("cargo", &config, "cargo_build", None);
    assert!(result.is_err());
}

#[test]
fn test_resolve_cli_subcommand_no_subcommands() {
    let config = create_test_tool_config("cargo", "cargo");

    let result = resolve_cli_subcommand("cargo", &config, "cargo_build", None);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("no subcommands"));
}

// ============= Integration Tests =============

#[test]
fn test_find_and_resolve_workflow() {
    let mut configs = HashMap::new();
    configs.insert(
        "cargo".to_string(),
        create_tool_config_with_subcommands(
            "cargo",
            "cargo",
            vec![
                create_subcommand("default"),
                create_subcommand("build"),
                create_subcommand("test"),
            ],
        ),
    );

    // Find the tool
    let (key, config) = find_matching_tool(&configs, "cargo_build").unwrap();
    assert_eq!(key, "cargo");

    // Resolve the subcommand
    let (subcommand, command_parts) =
        resolve_cli_subcommand(key, config, "cargo_build", None).unwrap();
    assert_eq!(subcommand.name, "build");
    assert_eq!(command_parts, vec!["cargo", "build"]);
}
