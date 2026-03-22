use ahma_mcp::config::{SubcommandConfig, ToolConfig};
use ahma_mcp::shell::cli::Cli;
use ahma_mcp::shell::resolution::{normalize_tools_dir, resolve_cli_subcommand};

use clap::Parser;

use std::fs;

use tempfile::TempDir;

#[test]
fn test_normalize_tools_dir_explicit() {
    let tmp_dir = TempDir::new().unwrap();
    let explicit_path = tmp_dir.path().join("custom_tools");
    fs::create_dir(&explicit_path).unwrap();

    let result = normalize_tools_dir(Some(explicit_path.clone()));
    assert_eq!(result, Some(explicit_path));
}

#[test]
fn test_normalize_tools_dir_legacy_structure() {
    let tmp_dir = TempDir::new().unwrap();
    let ahma_dir = tmp_dir.path().join(".ahma");
    let tools_dir = ahma_dir.join("tools");
    fs::create_dir_all(&tools_dir).unwrap();

    // If I pass .ahma/tools, it should return .ahma
    let result = normalize_tools_dir(Some(tools_dir));
    assert_eq!(result, Some(ahma_dir));
}

#[test]
fn test_resolve_cli_subcommand_basic() {
    let mut config = ToolConfig {
        name: "mytool".to_string(),
        command: "tool_cmd".to_string(),
        ..Default::default()
    };

    let sub = SubcommandConfig {
        name: "sub".to_string(),
        enabled: true,
        ..Default::default()
    };
    config.subcommand = Some(vec![sub]);

    let (resolved_sub, parts) =
        resolve_cli_subcommand("mytool", &config, "mytool_sub", None).unwrap();
    assert_eq!(resolved_sub.name, "sub");
    assert_eq!(parts, vec!["tool_cmd", "sub"]);
}

#[test]
fn test_resolve_cli_subcommand_nested() {
    let mut config = ToolConfig {
        name: "git".to_string(),
        command: "git".to_string(),
        ..Default::default()
    };

    let nested_sub = SubcommandConfig {
        name: "list".to_string(),
        enabled: true,
        ..Default::default()
    };

    let sub = SubcommandConfig {
        name: "remote".to_string(),
        enabled: true,
        subcommand: Some(vec![nested_sub]),
        ..Default::default()
    };
    config.subcommand = Some(vec![sub]);

    let (resolved_sub, parts) =
        resolve_cli_subcommand("git", &config, "git_remote_list", None).unwrap();
    assert_eq!(resolved_sub.name, "list");
    assert_eq!(parts, vec!["git", "remote", "list"]);
}

#[test]
fn test_resolve_cli_subcommand_errors() {
    let config = ToolConfig {
        name: "mytool".to_string(),
        subcommand: Some(vec![]),
        ..Default::default()
    };

    // Invalid format
    let res = resolve_cli_subcommand("mytool", &config, "invalidformat", None);
    assert!(res.is_err()); // expects tool_subcommand

    // Missing subcommand
    let res = resolve_cli_subcommand("mytool", &config, "mytool_missing", None);
    assert!(res.is_err());
}

#[test]
fn test_cli_argument_parsing() {
    use ahma_mcp::shell::cli::{ServeTransport, Subcommands};

    // Test `Cli::try_parse_from` with the new subcommand API.
    let args = vec!["ahma_mcp", "serve", "http", "--port", "8080"];
    let cli = Cli::try_parse_from(args).unwrap();

    let Subcommands::Serve(serve_args) = cli.command else {
        panic!("Expected serve subcommand");
    };
    let ServeTransport::Http(http_args) = serve_args.transport else {
        panic!("Expected http transport");
    };
    assert_eq!(http_args.port, 8080);
}
