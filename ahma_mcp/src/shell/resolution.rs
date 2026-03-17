//! # Tool Resolution Module
//!
//! Provides utilities for resolving tool configurations, finding matching tools,
//! and executing tool sequences.

use crate::{
    adapter::Adapter,
    config::{SubcommandConfig, ToolConfig},
};
use anyhow::{Context, Result, anyhow};
use std::{collections::HashMap, path::PathBuf, time::Duration};

/// Auto-detect .ahma directory or use explicitly provided tools_dir.
///
/// # Arguments
/// * `tools_dir` - Optional explicit tools directory from CLI.
///
/// # Returns
/// * `Some(PathBuf)` - Path to tools directory (explicit or auto-detected .ahma).
/// * `None` - No tools directory found (will use only built-in internal tools).
pub fn normalize_tools_dir(tools_dir: Option<PathBuf>) -> Option<PathBuf> {
    // If explicitly provided via CLI, use it (takes precedence)
    if let Some(explicit_dir) = tools_dir {
        // Handle legacy .ahma/tools format
        let is_legacy_tools_dir = explicit_dir
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s == "tools")
            && explicit_dir
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .is_some_and(|s| s == ".ahma");

        if is_legacy_tools_dir {
            return explicit_dir.parent().map(|p| p.to_path_buf());
        } else {
            return Some(explicit_dir);
        }
    }

    // No explicit --tools-dir, try auto-detection
    if let Ok(cwd) = std::env::current_dir() {
        let ahma_dir = cwd.join(".ahma");
        if ahma_dir.exists() && ahma_dir.is_dir() {
            tracing::info!("Auto-detected .ahma directory at: {}", ahma_dir.display());
            return Some(ahma_dir);
        } else {
            tracing::warn!(
                "No .ahma directory found in current working directory ({}). \
                 Falling back to built-in internal tools only (await, status, sandboxed_shell).",
                cwd.display()
            );
        }
    } else {
        tracing::warn!(
            "Could not determine current working directory. \
             Falling back to built-in internal tools only (await, status, sandboxed_shell)."
        );
    }

    // No tools directory found
    None
}

/// Find the best matching tool configuration by name prefix.
///
/// # Arguments
/// * `configs` - Loaded tool configs keyed by tool name.
/// * `tool_name` - Raw tool name (may include subcommand suffix).
///
/// # Returns
/// The matching key and config with the longest prefix match.
///
/// # Errors
/// Returns an error when no enabled tool matches the provided name.
pub fn find_matching_tool<'a>(
    configs: &'a HashMap<String, ToolConfig>,
    tool_name: &str,
) -> Result<(&'a str, &'a ToolConfig)> {
    configs
        .iter()
        .filter(|(_, config)| config.enabled)
        .filter_map(|(key, config)| {
            if tool_name.starts_with(key) {
                Some((key.as_str(), config))
            } else {
                None
            }
        })
        .max_by_key(|(key, _)| key.len())
        .ok_or_else(|| anyhow!("No matching tool configuration found for '{}'", tool_name))
}

/// Find a tool configuration by exact key or configured name.
///
/// # Arguments
/// * `configs` - Loaded tool configs keyed by tool name.
/// * `tool_name` - Name to look up.
///
/// # Returns
/// A tuple of the key and config if found, otherwise `None`.
pub fn find_tool_config<'a>(
    configs: &'a HashMap<String, ToolConfig>,
    tool_name: &str,
) -> Option<(&'a str, &'a ToolConfig)> {
    if let Some((key, config)) = configs.get_key_value(tool_name) {
        return Some((key.as_str(), config));
    }

    configs
        .iter()
        .find(|(_, config)| config.name == tool_name)
        .map(|(key, config)| (key.as_str(), config))
}

/// Resolve a CLI tool name into a subcommand configuration and argv prefix.
///
/// # Arguments
/// * `config_key` - Tool config key (prefix used for matching).
/// * `config` - Tool configuration.
/// * `tool_name` - Raw CLI tool name (may include `_subcommand`).
/// * `subcommand_override` - Optional override for subcommand selection.
///
/// # Returns
/// A reference to the resolved subcommand config and the argv prefix.
///
/// # Errors
/// Returns an error when the subcommand path is invalid or not found.
pub fn resolve_cli_subcommand<'a>(
    config_key: &str,
    config: &'a ToolConfig,
    tool_name: &str,
    subcommand_override: Option<&str>,
) -> Result<(&'a SubcommandConfig, Vec<String>)> {
    let subcommand_source =
        subcommand_override.unwrap_or_else(|| tool_name.strip_prefix(config_key).unwrap_or(""));
    let trimmed = subcommand_source.trim();
    let is_default_call = trimmed.is_empty() || trimmed == "default";

    let subcommand_parts: Vec<&str> = if is_default_call {
        vec!["default"]
    } else {
        trimmed
            .trim_start_matches('_')
            .split('_')
            .filter(|segment| !segment.is_empty())
            .collect()
    };

    if subcommand_parts.is_empty() {
        anyhow::bail!("Invalid tool name format. Expected 'tool_subcommand'.");
    }

    let mut current_subcommands = config
        .subcommand
        .as_ref()
        .ok_or_else(|| anyhow!("Tool '{}' has no subcommands defined", config_key))?;
    let mut command_parts = vec![config.command.clone()];
    let mut found_subcommand = None;
    let error_path = if is_default_call {
        "default".to_string()
    } else {
        trimmed.trim_start_matches('_').to_string()
    };

    for (index, part) in subcommand_parts.iter().enumerate() {
        if let Some(sub) = current_subcommands
            .iter()
            .find(|candidate| candidate.name == *part && candidate.enabled)
        {
            if sub.name == "default" && is_default_call {
                // Logic to derive subcommand from tool name (e.g. cargo_build -> cargo build)
                // is removed because it causes issues for tools like bash (bash -c async).
                // If a tool needs a subcommand, it should be explicit in the config or the command.
            } else if sub.name != "default" {
                command_parts.push(sub.name.clone());
            }

            if index == subcommand_parts.len() - 1 {
                found_subcommand = Some(sub);
            } else if let Some(nested) = &sub.subcommand {
                current_subcommands = nested;
            } else {
                anyhow::bail!(
                    "Subcommand '{}' has no nested subcommands for remaining path in tool '{}'",
                    error_path,
                    config_key
                );
            }
        } else {
            anyhow::bail!(
                "Subcommand '{}' not found for tool '{}'",
                error_path,
                config_key
            );
        }
    }

    let subcommand_config = found_subcommand.ok_or_else(|| {
        anyhow!(
            "Subcommand '{}' not found for tool '{}'",
            error_path,
            config_key
        )
    })?;

    Ok((subcommand_config, command_parts))
}

/// Execute a sequence of tool steps.
///
/// # Arguments
/// * `adapter` - The adapter for executing commands.
/// * `configs` - All available tool configurations.
/// * `parent_config` - The parent tool configuration.
/// * `subcommand_config` - The subcommand configuration containing the sequence.
/// * `working_dir` - Working directory for execution.
///
/// # Errors
/// Returns an error if any step in the sequence fails.
pub async fn run_cli_sequence(
    adapter: &Adapter,
    configs: &HashMap<String, ToolConfig>,
    parent_config: &ToolConfig,
    subcommand_config: &SubcommandConfig,
    working_dir: &str,
) -> Result<()> {
    let sequence = subcommand_config
        .sequence
        .as_ref()
        .ok_or_else(|| anyhow!("Sequence not defined for tool '{}'", parent_config.name))?;

    let delay_ms = subcommand_config
        .step_delay_ms
        .or(parent_config.step_delay_ms)
        .unwrap_or(0);

    for (index, step) in sequence.iter().enumerate() {
        let (step_key, step_tool_config) = find_tool_config(configs, &step.tool)
            .ok_or_else(|| anyhow!("Sequence step tool '{}' not found", step.tool))?;

        let (step_subcommand_config, command_parts) =
            resolve_cli_subcommand(step_key, step_tool_config, step_key, Some(&step.subcommand))?;

        println!(
            "▶ Running sequence step {} ({} {}):",
            index + 1,
            step.tool,
            step.subcommand
        );

        let output = adapter
            .execute_sync_in_dir(
                &command_parts.join(" "),
                Some(step.args.clone()),
                working_dir,
                step_subcommand_config.timeout_seconds,
                Some(step_subcommand_config),
            )
            .await
            .with_context(|| format!("Sequence step '{} {}' failed", step.tool, step.subcommand))?;

        if !output.trim().is_empty() {
            println!("{}", output);
        } else {
            println!("OK Completed without output");
        }

        if delay_ms > 0 && index + 1 < sequence.len() {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ToolHints;

    // =========================================================================
    // Helper: Create a minimal ToolConfig for testing
    // =========================================================================
    fn make_tool_config(name: &str, command: &str, enabled: bool) -> ToolConfig {
        ToolConfig {
            name: name.to_string(),
            description: format!("{} tool", name),
            command: command.to_string(),
            enabled,
            timeout_seconds: None,
            synchronous: None,
            guidance_key: None,
            input_schema: None,
            hints: ToolHints::default(),
            subcommand: None,
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

    fn make_tool_config_with_subcommands(
        name: &str,
        command: &str,
        enabled: bool,
        subcommands: Vec<SubcommandConfig>,
    ) -> ToolConfig {
        ToolConfig {
            name: name.to_string(),
            description: format!("{} tool", name),
            command: command.to_string(),
            enabled,
            timeout_seconds: None,
            synchronous: None,
            guidance_key: None,
            input_schema: None,
            hints: ToolHints::default(),
            subcommand: Some(subcommands),
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

    fn make_subcommand(name: &str, enabled: bool) -> SubcommandConfig {
        SubcommandConfig {
            name: name.to_string(),
            description: format!("{} subcommand", name),
            subcommand: None,
            options: None,
            positional_args: None,
            positional_args_first: None,
            timeout_seconds: None,
            synchronous: None,
            enabled,
            guidance_key: None,
            sequence: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
        }
    }

    fn make_subcommand_with_nested(
        name: &str,
        enabled: bool,
        nested: Vec<SubcommandConfig>,
    ) -> SubcommandConfig {
        SubcommandConfig {
            name: name.to_string(),
            description: format!("{} subcommand", name),
            subcommand: Some(nested),
            options: None,
            positional_args: None,
            positional_args_first: None,
            timeout_seconds: None,
            synchronous: None,
            enabled,
            guidance_key: None,
            sequence: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
        }
    }

    // =========================================================================
    // Tests for: find_matching_tool
    // =========================================================================
    mod find_matching_tool_tests {
        use super::*;

        #[test]
        fn returns_exact_match_when_tool_name_matches_key() {
            let mut configs = HashMap::new();
            configs.insert(
                "cargo".to_string(),
                make_tool_config("cargo", "cargo", true),
            );

            let result = find_matching_tool(&configs, "cargo").unwrap();

            assert_eq!(result.0, "cargo");
            assert_eq!(result.1.name, "cargo");
        }

        #[test]
        fn returns_longest_prefix_match_when_multiple_tools_match() {
            let mut configs = HashMap::new();
            configs.insert(
                "cargo".to_string(),
                make_tool_config("cargo", "cargo", true),
            );
            configs.insert(
                "cargo_build".to_string(),
                make_tool_config("cargo_build", "cargo build", true),
            );

            let result = find_matching_tool(&configs, "cargo_build_release").unwrap();

            assert_eq!(result.0, "cargo_build");
        }

        #[test]
        fn ignores_disabled_tools() {
            let mut configs = HashMap::new();
            configs.insert(
                "cargo_build".to_string(),
                make_tool_config("cargo_build", "cargo build", false),
            );
            configs.insert(
                "cargo".to_string(),
                make_tool_config("cargo", "cargo", true),
            );

            let result = find_matching_tool(&configs, "cargo_build").unwrap();

            // Should match "cargo" since "cargo_build" is disabled
            assert_eq!(result.0, "cargo");
        }

        #[test]
        fn returns_error_when_no_tool_matches() {
            let mut configs = HashMap::new();
            configs.insert(
                "cargo".to_string(),
                make_tool_config("cargo", "cargo", true),
            );

            let result = find_matching_tool(&configs, "rustc");

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("No matching tool"));
        }

        #[test]
        fn returns_error_when_all_matching_tools_disabled() {
            let mut configs = HashMap::new();
            configs.insert(
                "cargo".to_string(),
                make_tool_config("cargo", "cargo", false),
            );

            let result = find_matching_tool(&configs, "cargo_test");

            assert!(result.is_err());
        }
    }

    // =========================================================================
    // Tests for: find_tool_config
    // =========================================================================
    mod find_tool_config_tests {
        use super::*;

        #[test]
        fn finds_tool_by_key() {
            let mut configs = HashMap::new();
            configs.insert(
                "cargo".to_string(),
                make_tool_config("cargo", "cargo", true),
            );

            let result = find_tool_config(&configs, "cargo");

            assert!(result.is_some());
            let (key, config) = result.unwrap();
            assert_eq!(key, "cargo");
            assert_eq!(config.name, "cargo");
        }

        #[test]
        fn finds_tool_by_name_when_key_differs() {
            let mut configs = HashMap::new();
            let mut tool = make_tool_config("cargo-tool", "cargo", true);
            tool.name = "cargo-tool".to_string();
            configs.insert("cargo_alias".to_string(), tool);

            let result = find_tool_config(&configs, "cargo-tool");

            assert!(result.is_some());
            let (key, config) = result.unwrap();
            assert_eq!(key, "cargo_alias");
            assert_eq!(config.name, "cargo-tool");
        }

        #[test]
        fn returns_none_when_tool_not_found() {
            let configs: HashMap<String, ToolConfig> = HashMap::new();

            let result = find_tool_config(&configs, "nonexistent");

            assert!(result.is_none());
        }

        #[test]
        fn prefers_exact_key_match_over_name_match() {
            let mut configs = HashMap::new();
            let mut tool1 = make_tool_config("other_name", "cmd1", true);
            tool1.name = "cargo".to_string();
            configs.insert("alias".to_string(), tool1);

            let mut tool2 = make_tool_config("cargo", "cmd2", true);
            tool2.name = "different_name".to_string();
            configs.insert("cargo".to_string(), tool2);

            let result = find_tool_config(&configs, "cargo");

            assert!(result.is_some());
            let (key, _) = result.unwrap();
            assert_eq!(key, "cargo"); // Key match should win
        }
    }

    // =========================================================================
    // Tests for: resolve_cli_subcommand
    // =========================================================================
    mod resolve_cli_subcommand_tests {
        use super::*;

        #[test]
        fn resolves_default_subcommand_when_tool_name_equals_config_key() {
            let config = make_tool_config_with_subcommands(
                "cargo",
                "cargo",
                true,
                vec![make_subcommand("default", true)],
            );

            let result = resolve_cli_subcommand("cargo", &config, "cargo", None);

            assert!(result.is_ok());
            let (sub, parts) = result.unwrap();
            assert_eq!(sub.name, "default");
            assert_eq!(parts, vec!["cargo"]);
        }

        #[test]
        fn resolves_explicit_subcommand() {
            let config = make_tool_config_with_subcommands(
                "cargo",
                "cargo",
                true,
                vec![
                    make_subcommand("default", true),
                    make_subcommand("build", true),
                ],
            );

            let result = resolve_cli_subcommand("cargo", &config, "cargo_build", None);

            assert!(result.is_ok());
            let (sub, parts) = result.unwrap();
            assert_eq!(sub.name, "build");
            assert_eq!(parts, vec!["cargo", "build"]);
        }

        #[test]
        fn resolves_nested_subcommand() {
            let config = make_tool_config_with_subcommands(
                "git",
                "git",
                true,
                vec![make_subcommand_with_nested(
                    "remote",
                    true,
                    vec![make_subcommand("add", true)],
                )],
            );

            let result = resolve_cli_subcommand("git", &config, "git_remote_add", None);

            assert!(result.is_ok());
            let (sub, parts) = result.unwrap();
            assert_eq!(sub.name, "add");
            assert_eq!(parts, vec!["git", "remote", "add"]);
        }

        #[test]
        fn returns_error_when_subcommand_not_found() {
            let config = make_tool_config_with_subcommands(
                "cargo",
                "cargo",
                true,
                vec![make_subcommand("build", true)],
            );

            let result = resolve_cli_subcommand("cargo", &config, "cargo_test", None);

            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Subcommand"));
            assert!(err.contains("not found"));
        }

        #[test]
        fn returns_error_when_no_subcommands_defined() {
            let config = make_tool_config("cargo", "cargo", true);

            let result = resolve_cli_subcommand("cargo", &config, "cargo_build", None);

            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("no subcommands defined"));
        }

        #[test]
        fn ignores_disabled_subcommands() {
            let config = make_tool_config_with_subcommands(
                "cargo",
                "cargo",
                true,
                vec![
                    make_subcommand("build", false), // disabled
                    make_subcommand("test", true),
                ],
            );

            let result = resolve_cli_subcommand("cargo", &config, "cargo_build", None);

            assert!(result.is_err());
        }

        #[test]
        fn uses_subcommand_override_when_provided() {
            let config = make_tool_config_with_subcommands(
                "cargo",
                "cargo",
                true,
                vec![
                    make_subcommand("build", true),
                    make_subcommand("test", true),
                ],
            );

            let result = resolve_cli_subcommand("cargo", &config, "cargo", Some("test"));

            assert!(result.is_ok());
            let (sub, parts) = result.unwrap();
            assert_eq!(sub.name, "test");
            assert_eq!(parts, vec!["cargo", "test"]);
        }

        #[test]
        fn handles_explicit_default_subcommand_override() {
            let config = make_tool_config_with_subcommands(
                "cargo",
                "cargo",
                true,
                vec![make_subcommand("default", true)],
            );

            let result =
                resolve_cli_subcommand("cargo", &config, "cargo_something", Some("default"));

            assert!(result.is_ok());
            let (sub, _) = result.unwrap();
            assert_eq!(sub.name, "default");
        }
    }
}
