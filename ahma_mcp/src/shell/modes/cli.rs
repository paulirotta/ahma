//! # CLI Mode
//!
//! Runs the ahma_mcp server in CLI mode, which executes a single tool and prints
//! the result to stdout.

use crate::shell::cli::AppConfig;
use crate::{
    adapter::Adapter,
    config::{SubcommandConfig, load_tool_configs},
    operation_monitor::{MonitorConfig, OperationMonitor},
    sandbox,
    shell::resolution::{find_matching_tool, resolve_cli_subcommand, run_cli_sequence},
    shell_pool::{ShellPoolConfig, ShellPoolManager},
    tool_availability::evaluate_tool_availability,
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{borrow::Cow, path::PathBuf, sync::Arc, time::Duration};

struct CliResolution<'a> {
    subcommand_config: Cow<'a, SubcommandConfig>,
    command_parts: Vec<String>,
}

struct ParsedCliArgs {
    raw_args: Vec<String>,
    working_directory: Option<String>,
    tool_args_map: serde_json::Map<String, serde_json::Value>,
}

/// Run in CLI mode (execute a single tool and print result).
///
/// # Arguments
/// * `config` - Immutable application configuration.
/// * `sandbox` - Sandbox configuration.
///
/// # Errors
/// Returns an error if the tool execution fails.
pub async fn run_cli_mode(config: AppConfig, sandbox: Arc<sandbox::Sandbox>) -> Result<()> {
    let tool_name = config.run_tool.clone().unwrap();

    let (adapter, configs) = initialize_cli_runtime(&config, sandbox).await?;

    if configs.is_empty() && tool_name != "sandboxed_shell" {
        tracing::error!("No external tool configurations found");
        anyhow::bail!("No tool '{}' found", tool_name);
    }

    let (tool_config_key, tool_config) = find_matching_tool(configs.as_ref(), &tool_name)?;

    let resolution = resolve_cli_invocation(tool_config_key, tool_config, &tool_name)?;
    let (args_map, working_dir_str) =
        build_cli_arguments(&config, resolution.subcommand_config.as_ref());

    if tool_config.command == "sequence" && resolution.subcommand_config.sequence.is_some() {
        run_cli_sequence(
            &adapter,
            configs.as_ref(),
            tool_config,
            resolution.subcommand_config.as_ref(),
            &working_dir_str,
        )
        .await?;
        return Ok(());
    }

    let base_command = resolution.command_parts.join(" ");

    let result = adapter
        .execute_sync_in_dir(
            &base_command,
            Some(args_map),
            &working_dir_str,
            resolution.subcommand_config.timeout_seconds,
            Some(resolution.subcommand_config.as_ref()),
        )
        .await;
    let _ = tool_config; // consumed by execute_sync_in_dir via subcommand

    match result {
        Ok(output) => {
            println!("{}", output);
            Ok(())
        }
        Err(e) => {
            print_cli_error(&e);
            Err(anyhow::anyhow!("Tool execution failed"))
        }
    }
}

async fn initialize_cli_runtime(
    config: &AppConfig,
    sandbox: Arc<sandbox::Sandbox>,
) -> Result<(
    Adapter,
    Arc<std::collections::HashMap<String, crate::config::ToolConfig>>,
)> {
    let monitor_config =
        MonitorConfig::with_timeout(std::time::Duration::from_secs(config.timeout_secs));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_pool_manager = Arc::new(ShellPoolManager::new(ShellPoolConfig {
        command_timeout: Duration::from_secs(config.timeout_secs),
        ..Default::default()
    }));

    let adapter = Adapter::new(
        operation_monitor,
        shell_pool_manager.clone(),
        sandbox.clone(),
    )?;

    let configs = load_cli_configs(config, shell_pool_manager, sandbox).await?;
    Ok((adapter, configs))
}

async fn load_cli_configs(
    config: &AppConfig,
    shell_pool_manager: Arc<ShellPoolManager>,
    sandbox: Arc<sandbox::Sandbox>,
) -> Result<Arc<std::collections::HashMap<String, crate::config::ToolConfig>>> {
    let raw_configs = load_tool_configs(config, config.tools_dir.as_deref())
        .await
        .context("Failed to load tool configurations")?;

    let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let availability_summary = evaluate_tool_availability(
        shell_pool_manager,
        raw_configs,
        working_dir.as_path(),
        sandbox.as_ref(),
    )
    .await?;

    log_disabled_tools(&availability_summary.disabled_tools);
    Ok(Arc::new(availability_summary.filtered_configs))
}

fn log_disabled_tools(disabled_tools: &[crate::tool_availability::DisabledTool]) {
    for disabled in disabled_tools {
        tracing::warn!(
            "Tool '{}' disabled at CLI startup. {}",
            disabled.name,
            disabled.message
        );
    }
}

fn resolve_cli_invocation<'a>(
    config_key: &'a str,
    config: &'a crate::config::ToolConfig,
    tool_name: &str,
) -> Result<CliResolution<'a>> {
    if is_top_level_sequence(config) {
        return Ok(CliResolution {
            subcommand_config: Cow::Owned(sequence_subcommand_config(config)),
            command_parts: vec![config.command.clone()],
        });
    }

    let (subcommand_config, command_parts) =
        resolve_cli_subcommand(config_key, config, tool_name, None)?;
    Ok(CliResolution {
        subcommand_config: Cow::Borrowed(subcommand_config),
        command_parts,
    })
}

fn is_top_level_sequence(config: &crate::config::ToolConfig) -> bool {
    config.command == "sequence" && config.sequence.is_some()
}

fn sequence_subcommand_config(config: &crate::config::ToolConfig) -> SubcommandConfig {
    SubcommandConfig {
        name: config.name.clone(),
        description: config.description.clone(),
        subcommand: None,
        options: None,
        positional_args: None,
        positional_args_first: None,
        timeout_seconds: config.timeout_seconds,
        synchronous: config.synchronous,
        enabled: true,
        guidance_key: config.guidance_key.clone(),
        sequence: config.sequence.clone(),
        step_delay_ms: config.step_delay_ms,
        availability_check: None,
        install_instructions: None,
    }
}

fn build_cli_arguments(
    config: &AppConfig,
    subcommand_config: &SubcommandConfig,
) -> (serde_json::Map<String, serde_json::Value>, String) {
    let ParsedCliArgs {
        mut raw_args,
        working_directory,
        tool_args_map,
    } = parse_cli_args(config);

    append_mapped_args(&mut raw_args, &tool_args_map);
    strip_default_subcommand_marker(&mut raw_args);

    let mut args_map = clone_tool_args(&tool_args_map);
    insert_positional_and_keyed_args(&mut args_map, subcommand_config, &raw_args);

    let working_dir = resolve_working_directory(working_directory, &tool_args_map);
    (args_map, working_dir)
}

fn parse_cli_args(config: &AppConfig) -> ParsedCliArgs {
    if let Some(tool_args_map) = tool_args_from_env() {
        return ParsedCliArgs {
            raw_args: Vec::new(),
            working_directory: None,
            tool_args_map,
        };
    }

    parse_cli_flag_args(config)
}

fn tool_args_from_env() -> Option<serde_json::Map<String, serde_json::Value>> {
    let env_args = std::env::var("AHMA_MCP_ARGS").ok()?;
    let json_value = serde_json::from_str::<serde_json::Value>(&env_args).ok()?;
    json_value.as_object().cloned()
}

fn parse_cli_flag_args(config: &AppConfig) -> ParsedCliArgs {
    let mut raw_args = Vec::new();
    let mut working_directory = None;
    let mut tool_args_map = serde_json::Map::new();
    let mut iter = config.run_tool_args.clone().into_iter().peekable();

    while let Some(arg) = iter.next() {
        if arg == "--" {
            raw_args.extend(iter);
            break;
        }

        if !arg.starts_with("--") {
            raw_args.push(arg);
            continue;
        }

        let key = arg.trim_start_matches("--").to_string();
        match next_cli_value(&mut iter) {
            Some(value) if key == "working-directory" => working_directory = Some(value),
            Some(value) => {
                tool_args_map.insert(key, serde_json::Value::String(value));
            }
            None => {
                tool_args_map.insert(key, serde_json::Value::Bool(true));
            }
        }
    }

    ParsedCliArgs {
        raw_args,
        working_directory,
        tool_args_map,
    }
}

fn next_cli_value<I>(iter: &mut std::iter::Peekable<I>) -> Option<String>
where
    I: Iterator<Item = String>,
{
    let next = iter.peek()?;
    if next.starts_with('-') {
        return None;
    }
    iter.next()
}

fn append_mapped_args(
    raw_args: &mut Vec<String>,
    tool_args_map: &serde_json::Map<String, serde_json::Value>,
) {
    if let Some(args_from_map) = tool_args_map.get("args").and_then(|v| v.as_array()) {
        raw_args.extend(
            args_from_map
                .iter()
                .filter_map(|value| value.as_str().map(String::from)),
        );
    }
}

fn strip_default_subcommand_marker(raw_args: &mut Vec<String>) {
    if raw_args.first().map(|s| s.as_str()) == Some("default") {
        raw_args.remove(0);
    }
}

fn clone_tool_args(
    tool_args_map: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    tool_args_map
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn insert_positional_and_keyed_args(
    args_map: &mut serde_json::Map<String, serde_json::Value>,
    subcommand_config: &SubcommandConfig,
    raw_args: &[String],
) {
    let mut positional_iter = subcommand_config
        .positional_args
        .as_ref()
        .map(|args| args.iter())
        .unwrap_or_else(|| [].iter());

    for arg in raw_args {
        if let Some((key, value)) = arg.split_once('=') {
            args_map.insert(key.to_string(), Value::String(value.to_string()));
        } else if let Some(positional_arg) = positional_iter.next() {
            args_map.insert(positional_arg.name.clone(), Value::String(arg.clone()));
        } else {
            args_map.insert(arg.clone(), Value::String(String::new()));
        }
    }
}

fn resolve_working_directory(
    working_directory: Option<String>,
    tool_args_map: &serde_json::Map<String, serde_json::Value>,
) -> String {
    working_directory
        .or_else(|| working_directory_from_args(tool_args_map))
        .or_else(default_working_directory)
        .unwrap_or_else(|| ".".to_string())
}

fn working_directory_from_args(
    tool_args_map: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    tool_args_map
        .get("working_directory")
        .and_then(|value| value.as_str())
        .map(String::from)
}

fn default_working_directory() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

fn print_cli_error(error: &anyhow::Error) {
    let error_message = error.to_string();
    if error_message.contains("Canceled: Canceled") {
        eprintln!(
            "Operation cancelled by user request (was: {})",
            error_message
        );
    } else if error_message.contains("task cancelled for reason") {
        eprintln!(
            "Operation cancelled by user request or system signal (detected MCP cancellation)"
        );
    } else if error_message.to_lowercase().contains("cancel") {
        eprintln!("Operation cancelled: {}", error_message);
    } else {
        eprintln!("Error executing tool: {}", error);
    }
}
