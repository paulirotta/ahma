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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value, json};

    // ============= parse_cli_flag_args tests =============

    #[test]
    fn test_parse_cli_flag_args_basic_flags() {
        let config = AppConfig {
            run_tool_args: vec![
                "--key".to_string(),
                "value".to_string(),
                "--flag".to_string(),
            ],
            ..Default::default()
        };
        let parsed = parse_cli_flag_args(&config);
        assert_eq!(
            parsed.tool_args_map.get("key"),
            Some(&Value::String("value".into()))
        );
        assert_eq!(parsed.tool_args_map.get("flag"), Some(&Value::Bool(true)));
        assert!(parsed.raw_args.is_empty());
    }

    #[test]
    fn test_parse_cli_flag_args_working_directory() {
        let config = AppConfig {
            run_tool_args: vec!["--working-directory".to_string(), "/some/path".to_string()],
            ..Default::default()
        };
        let parsed = parse_cli_flag_args(&config);
        assert_eq!(parsed.working_directory, Some("/some/path".to_string()));
        assert!(!parsed.tool_args_map.contains_key("working-directory"));
    }

    #[test]
    fn test_parse_cli_flag_args_double_dash_terminator() {
        let config = AppConfig {
            run_tool_args: vec![
                "--key".to_string(),
                "val".to_string(),
                "--".to_string(),
                "raw1".to_string(),
                "raw2".to_string(),
            ],
            ..Default::default()
        };
        let parsed = parse_cli_flag_args(&config);
        assert_eq!(
            parsed.tool_args_map.get("key"),
            Some(&Value::String("val".into()))
        );
        assert_eq!(parsed.raw_args, vec!["raw1", "raw2"]);
    }

    #[test]
    fn test_parse_cli_flag_args_positional_args() {
        let config = AppConfig {
            run_tool_args: vec!["pos1".to_string(), "pos2".to_string()],
            ..Default::default()
        };
        let parsed = parse_cli_flag_args(&config);
        assert_eq!(parsed.raw_args, vec!["pos1", "pos2"]);
        assert!(parsed.tool_args_map.is_empty());
    }

    #[test]
    fn test_parse_cli_flag_args_flag_at_end_is_bool() {
        let config = AppConfig {
            run_tool_args: vec!["--verbose".to_string()],
            ..Default::default()
        };
        let parsed = parse_cli_flag_args(&config);
        assert_eq!(
            parsed.tool_args_map.get("verbose"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn test_parse_cli_flag_args_flag_before_another_flag_is_bool() {
        let config = AppConfig {
            run_tool_args: vec![
                "--verbose".to_string(),
                "--output".to_string(),
                "file.txt".to_string(),
            ],
            ..Default::default()
        };
        let parsed = parse_cli_flag_args(&config);
        assert_eq!(
            parsed.tool_args_map.get("verbose"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            parsed.tool_args_map.get("output"),
            Some(&Value::String("file.txt".into()))
        );
    }

    #[test]
    fn test_parse_cli_flag_args_empty() {
        let config = AppConfig {
            run_tool_args: vec![],
            ..Default::default()
        };
        let parsed = parse_cli_flag_args(&config);
        assert!(parsed.raw_args.is_empty());
        assert!(parsed.tool_args_map.is_empty());
        assert!(parsed.working_directory.is_none());
    }

    // ============= next_cli_value tests =============

    #[test]
    fn test_next_cli_value_returns_value() {
        let args = vec!["hello".to_string()];
        let mut iter = args.into_iter().peekable();
        assert_eq!(next_cli_value(&mut iter), Some("hello".to_string()));
    }

    #[test]
    fn test_next_cli_value_skips_flag() {
        let args = vec!["--next-flag".to_string()];
        let mut iter = args.into_iter().peekable();
        assert_eq!(next_cli_value(&mut iter), None);
    }

    #[test]
    fn test_next_cli_value_empty() {
        let args: Vec<String> = vec![];
        let mut iter = args.into_iter().peekable();
        assert_eq!(next_cli_value(&mut iter), None);
    }

    // ============= resolve_working_directory tests =============

    #[test]
    fn test_resolve_working_directory_explicit() {
        let map = Map::new();
        let result = resolve_working_directory(Some("/explicit".to_string()), &map);
        assert_eq!(result, "/explicit");
    }

    #[test]
    fn test_resolve_working_directory_from_args() {
        let mut map = Map::new();
        map.insert(
            "working_directory".to_string(),
            Value::String("/from/args".into()),
        );
        let result = resolve_working_directory(None, &map);
        assert_eq!(result, "/from/args");
    }

    #[test]
    fn test_resolve_working_directory_fallback_cwd() {
        let map = Map::new();
        let result = resolve_working_directory(None, &map);
        // Should fall back to current dir or "."
        assert!(!result.is_empty());
    }

    #[test]
    fn test_working_directory_from_args_present() {
        let mut map = Map::new();
        map.insert(
            "working_directory".to_string(),
            Value::String("/test/dir".into()),
        );
        assert_eq!(
            working_directory_from_args(&map),
            Some("/test/dir".to_string())
        );
    }

    #[test]
    fn test_working_directory_from_args_absent() {
        let map = Map::new();
        assert_eq!(working_directory_from_args(&map), None);
    }

    // ============= is_top_level_sequence tests =============

    #[test]
    fn test_is_top_level_sequence_true() {
        let config = crate::config::ToolConfig {
            name: "test".to_string(),
            description: "test".to_string(),
            command: "sequence".to_string(),
            sequence: Some(vec![]),
            subcommand: None,
            input_schema: None,
            timeout_seconds: None,
            synchronous: None,
            hints: Default::default(),
            enabled: true,
            guidance_key: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
            monitor_level: None,
            monitor_stream: None,
            tool_type: None,
            livelog: None,
        };
        assert!(is_top_level_sequence(&config));
    }

    #[test]
    fn test_is_top_level_sequence_false_no_sequence() {
        let config = crate::config::ToolConfig {
            name: "test".to_string(),
            description: "test".to_string(),
            command: "sequence".to_string(),
            sequence: None,
            subcommand: None,
            input_schema: None,
            timeout_seconds: None,
            synchronous: None,
            hints: Default::default(),
            enabled: true,
            guidance_key: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
            monitor_level: None,
            monitor_stream: None,
            tool_type: None,
            livelog: None,
        };
        assert!(!is_top_level_sequence(&config));
    }

    #[test]
    fn test_is_top_level_sequence_false_wrong_command() {
        let config = crate::config::ToolConfig {
            name: "test".to_string(),
            description: "test".to_string(),
            command: "cargo".to_string(),
            sequence: Some(vec![]),
            subcommand: None,
            input_schema: None,
            timeout_seconds: None,
            synchronous: None,
            hints: Default::default(),
            enabled: true,
            guidance_key: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
            monitor_level: None,
            monitor_stream: None,
            tool_type: None,
            livelog: None,
        };
        assert!(!is_top_level_sequence(&config));
    }

    // ============= sequence_subcommand_config tests =============

    #[test]
    fn test_sequence_subcommand_config_copies_fields() {
        let steps = vec![crate::config::SequenceStep {
            tool: "cargo".to_string(),
            subcommand: "build".to_string(),
            description: Some("Build".to_string()),
            args: Default::default(),
            skip_if_file_exists: None,
            skip_if_file_missing: None,
        }];
        let config = crate::config::ToolConfig {
            name: "quality".to_string(),
            description: "Run quality checks".to_string(),
            command: "sequence".to_string(),
            sequence: Some(steps.clone()),
            timeout_seconds: Some(120),
            synchronous: Some(true),
            guidance_key: Some("quality_key".to_string()),
            step_delay_ms: Some(500),
            subcommand: None,
            input_schema: None,
            hints: Default::default(),
            enabled: true,
            availability_check: None,
            install_instructions: None,
            monitor_level: None,
            monitor_stream: None,
            tool_type: None,
            livelog: None,
        };
        let sub = sequence_subcommand_config(&config);
        assert_eq!(sub.name, "quality");
        assert_eq!(sub.description, "Run quality checks");
        assert_eq!(sub.timeout_seconds, Some(120));
        assert_eq!(sub.synchronous, Some(true));
        assert_eq!(sub.guidance_key, Some("quality_key".to_string()));
        assert_eq!(sub.step_delay_ms, Some(500));
        assert!(sub.sequence.is_some());
    }

    // ============= append_mapped_args tests =============

    #[test]
    fn test_append_mapped_args_with_args_array() {
        let mut raw_args = vec!["existing".to_string()];
        let mut map = Map::new();
        map.insert("args".to_string(), json!(["--release", "--verbose"]));
        append_mapped_args(&mut raw_args, &map);
        assert_eq!(raw_args, vec!["existing", "--release", "--verbose"]);
    }

    #[test]
    fn test_append_mapped_args_no_args_key() {
        let mut raw_args = vec!["existing".to_string()];
        let map = Map::new();
        append_mapped_args(&mut raw_args, &map);
        assert_eq!(raw_args, vec!["existing"]);
    }

    #[test]
    fn test_append_mapped_args_non_string_values_skipped() {
        let mut raw_args = vec![];
        let mut map = Map::new();
        map.insert("args".to_string(), json!(["valid", 123, "also_valid"]));
        append_mapped_args(&mut raw_args, &map);
        assert_eq!(raw_args, vec!["valid", "also_valid"]);
    }

    // ============= strip_default_subcommand_marker tests =============

    #[test]
    fn test_strip_default_subcommand_marker_removes_default() {
        let mut args = vec!["default".to_string(), "--release".to_string()];
        strip_default_subcommand_marker(&mut args);
        assert_eq!(args, vec!["--release"]);
    }

    #[test]
    fn test_strip_default_subcommand_marker_no_default() {
        let mut args = vec!["build".to_string(), "--release".to_string()];
        strip_default_subcommand_marker(&mut args);
        assert_eq!(args, vec!["build", "--release"]);
    }

    #[test]
    fn test_strip_default_subcommand_marker_empty() {
        let mut args: Vec<String> = vec![];
        strip_default_subcommand_marker(&mut args);
        assert!(args.is_empty());
    }

    // ============= clone_tool_args tests =============

    #[test]
    fn test_clone_tool_args() {
        let mut map = Map::new();
        map.insert("key1".to_string(), Value::String("val1".into()));
        map.insert("key2".to_string(), Value::Bool(true));
        let cloned = clone_tool_args(&map);
        assert_eq!(cloned.len(), 2);
        assert_eq!(cloned.get("key1"), Some(&Value::String("val1".into())));
        assert_eq!(cloned.get("key2"), Some(&Value::Bool(true)));
    }

    // ============= insert_positional_and_keyed_args tests =============

    #[test]
    fn test_insert_positional_and_keyed_args_with_positionals() {
        let mut args_map = Map::new();
        let subcommand_config = SubcommandConfig {
            name: "test".to_string(),
            description: "test".to_string(),
            positional_args: Some(vec![crate::config::CommandOption {
                name: "path".to_string(),
                option_type: "string".to_string(),
                description: None,
                required: None,
                format: None,
                items: None,
                file_arg: None,
                file_flag: None,
                alias: None,
            }]),
            ..Default::default()
        };
        let raw_args = vec!["/some/path".to_string()];
        insert_positional_and_keyed_args(&mut args_map, &subcommand_config, &raw_args);
        assert_eq!(
            args_map.get("path"),
            Some(&Value::String("/some/path".into()))
        );
    }

    #[test]
    fn test_insert_positional_and_keyed_args_with_key_equals_value() {
        let mut args_map = Map::new();
        let subcommand_config = SubcommandConfig::default();
        let raw_args = vec!["key=value".to_string()];
        insert_positional_and_keyed_args(&mut args_map, &subcommand_config, &raw_args);
        assert_eq!(args_map.get("key"), Some(&Value::String("value".into())));
    }

    #[test]
    fn test_insert_positional_and_keyed_args_overflow_no_positionals() {
        let mut args_map = Map::new();
        let subcommand_config = SubcommandConfig::default();
        let raw_args = vec!["extra_arg".to_string()];
        insert_positional_and_keyed_args(&mut args_map, &subcommand_config, &raw_args);
        assert_eq!(
            args_map.get("extra_arg"),
            Some(&Value::String(String::new()))
        );
    }

    // ============= print_cli_error tests =============

    #[test]
    fn test_print_cli_error_canceled_canceled() {
        // Just verify it doesn't panic — output goes to stderr
        let err = anyhow::anyhow!("Canceled: Canceled something");
        print_cli_error(&err);
    }

    #[test]
    fn test_print_cli_error_task_cancelled() {
        let err = anyhow::anyhow!("task cancelled for reason: timeout");
        print_cli_error(&err);
    }

    #[test]
    fn test_print_cli_error_generic_cancel() {
        let err = anyhow::anyhow!("The operation was cancelled by the user");
        print_cli_error(&err);
    }

    #[test]
    fn test_print_cli_error_generic_error() {
        let err = anyhow::anyhow!("Something went wrong");
        print_cli_error(&err);
    }

    // ============= build_cli_arguments tests =============

    #[test]
    fn test_build_cli_arguments_basic() {
        let config = AppConfig {
            run_tool_args: vec![
                "--working-directory".to_string(),
                "/test/dir".to_string(),
                "--".to_string(),
                "--release".to_string(),
            ],
            ..Default::default()
        };
        let subcommand_config = SubcommandConfig::default();
        let (args_map, working_dir) = build_cli_arguments(&config, &subcommand_config);
        assert_eq!(working_dir, "/test/dir");
        assert!(args_map.contains_key("--release"));
    }

    // ============= tool_args_from_env tests =============

    #[test]
    fn test_tool_args_from_env_not_set() {
        // When AHMA_MCP_ARGS is not set (or not valid JSON), returns None
        // We just check the function doesn't panic; the env var may or may not be set
        let result = tool_args_from_env();
        // If AHMA_MCP_ARGS happens to be set in the environment, it could return Some
        // The important thing is it doesn't panic
        let _ = result;
    }
}
