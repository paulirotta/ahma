//! Subcommand resolution for MCP tools.
//!
//! Contains functions for finding and resolving subcommand configurations.

use crate::config::{SubcommandConfig, ToolConfig};

/// Finds the configuration for a subcommand from the tool arguments.
pub fn find_subcommand_config_from_args(
    tool_config: &ToolConfig,
    subcommand_name: Option<String>,
) -> Option<(&SubcommandConfig, Vec<String>)> {
    if !tool_config.enabled {
        tracing::warn!(
            "Attempted to resolve subcommand on disabled tool '{}'",
            tool_config.name
        );
        return None;
    }

    let subcommand_path = subcommand_name.unwrap_or_else(|| "default".to_string());
    let subcommand_parts: Vec<&str> = subcommand_path.split('_').collect();

    tracing::debug!(
        "Finding subcommand for tool '{}': path='{}', parts={:?}, has_subcommands={}",
        tool_config.name,
        subcommand_path,
        subcommand_parts,
        tool_config.subcommand.is_some()
    );

    let mut current_subcommands = tool_config.subcommand.as_ref()?;
    let mut found_subcommand: Option<&SubcommandConfig> = None;
    let mut command_parts = vec![tool_config.command.clone()];

    for (i, part) in subcommand_parts.iter().enumerate() {
        tracing::debug!(
            "Searching for subcommand part '{}' (index {}/{}) in {} candidates",
            part,
            i,
            subcommand_parts.len() - 1,
            current_subcommands.len()
        );

        if let Some(sub) = current_subcommands
            .iter()
            .find(|s| s.name == *part && s.enabled)
        {
            tracing::debug!(
                "Found matching subcommand: name='{}', enabled={}",
                sub.name,
                sub.enabled
            );

            if sub.name != "default" {
                command_parts.push(sub.name.clone());
            }

            if i == subcommand_parts.len() - 1 {
                found_subcommand = Some(sub);
                break;
            }

            if let Some(nested) = &sub.subcommand {
                current_subcommands = nested;
            } else {
                tracing::debug!(
                    "Subcommand '{}' has no nested subcommands, but path continues",
                    sub.name
                );
                return None; // More parts in name, but no more nested subcommands
            }
        } else {
            tracing::debug!(
                "Subcommand part '{}' not found. Available: {:?}",
                part,
                current_subcommands
                    .iter()
                    .map(|s| &s.name)
                    .collect::<Vec<_>>()
            );
            return None; // Subcommand part not found
        }
    }

    found_subcommand.map(|sc| (sc, command_parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool_config(name: &str, subcommands: Option<Vec<SubcommandConfig>>) -> ToolConfig {
        ToolConfig {
            name: name.to_string(),
            description: format!("{} tool", name),
            command: name.to_string(),
            subcommand: subcommands,
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

    #[test]
    fn test_find_subcommand_default() {
        let subcommands = vec![make_subcommand("default", true)];
        let config = make_tool_config("test", Some(subcommands));

        let result = find_subcommand_config_from_args(&config, None);
        assert!(result.is_some());
        let (sub, parts) = result.unwrap();
        assert_eq!(sub.name, "default");
        assert_eq!(parts, vec!["test"]);
    }

    #[test]
    fn test_find_subcommand_explicit_name() {
        let subcommands = vec![
            make_subcommand("build", true),
            make_subcommand("test", true),
        ];
        let config = make_tool_config("cargo", Some(subcommands));

        let result = find_subcommand_config_from_args(&config, Some("build".to_string()));
        assert!(result.is_some());
        let (sub, parts) = result.unwrap();
        assert_eq!(sub.name, "build");
        assert_eq!(parts, vec!["cargo", "build"]);
    }

    #[test]
    fn test_find_subcommand_nested() {
        let nested = vec![make_subcommand("run", true)];
        let subcommands = vec![make_subcommand_with_nested("nextest", true, nested)];
        let config = make_tool_config("cargo", Some(subcommands));

        let result = find_subcommand_config_from_args(&config, Some("nextest_run".to_string()));
        assert!(result.is_some());
        let (sub, parts) = result.unwrap();
        assert_eq!(sub.name, "run");
        assert_eq!(parts, vec!["cargo", "nextest", "run"]);
    }

    #[test]
    fn test_find_subcommand_disabled_returns_none() {
        let subcommands = vec![make_subcommand("build", false)];
        let config = make_tool_config("cargo", Some(subcommands));

        let result = find_subcommand_config_from_args(&config, Some("build".to_string()));
        assert!(result.is_none());
    }

    #[test]
    fn test_find_subcommand_disabled_tool_returns_none() {
        let subcommands = vec![make_subcommand("build", true)];
        let mut config = make_tool_config("cargo", Some(subcommands));
        config.enabled = false;

        let result = find_subcommand_config_from_args(&config, Some("build".to_string()));
        assert!(result.is_none());
    }

    #[test]
    fn test_find_subcommand_not_found() {
        let subcommands = vec![make_subcommand("build", true)];
        let config = make_tool_config("cargo", Some(subcommands));

        let result = find_subcommand_config_from_args(&config, Some("nonexistent".to_string()));
        assert!(result.is_none());
    }

    #[test]
    fn test_find_subcommand_no_subcommands() {
        let config = make_tool_config("simple", None);

        let result = find_subcommand_config_from_args(&config, Some("anything".to_string()));
        assert!(result.is_none());
    }

    #[test]
    fn test_find_subcommand_deeply_nested() {
        let level3 = vec![make_subcommand("leaf", true)];
        let level2 = vec![make_subcommand_with_nested("mid", true, level3)];
        let level1 = vec![make_subcommand_with_nested("top", true, level2)];
        let config = make_tool_config("tool", Some(level1));

        let result = find_subcommand_config_from_args(&config, Some("top_mid_leaf".to_string()));
        assert!(result.is_some());
        let (sub, parts) = result.unwrap();
        assert_eq!(sub.name, "leaf");
        assert_eq!(parts, vec!["tool", "top", "mid", "leaf"]);
    }

    #[test]
    fn test_find_subcommand_partial_path_no_nested() {
        // If we try top_mid_nonexistent but mid has no nested subcommands
        let level2 = vec![make_subcommand("mid", true)]; // No nested
        let level1 = vec![make_subcommand_with_nested("top", true, level2)];
        let config = make_tool_config("tool", Some(level1));

        // "top_mid" should work (stops at mid)
        let result = find_subcommand_config_from_args(&config, Some("top_mid".to_string()));
        assert!(result.is_some());

        // "top_mid_extra" should fail (mid has no nested subcommands)
        let result2 = find_subcommand_config_from_args(&config, Some("top_mid_extra".to_string()));
        assert!(result2.is_none());
    }
}
