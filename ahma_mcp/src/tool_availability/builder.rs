use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tracing::debug;

use crate::config::{AvailabilityCheck, SubcommandConfig, ToolConfig};

use super::types::{ProbePlan, ProbeTarget};

pub(super) fn build_probe_plans(
    configs: &HashMap<String, ToolConfig>,
    default_working_dir: &Path,
) -> Vec<ProbePlan> {
    let mut plans = Vec::new();

    for (tool_name, config) in configs {
        if !should_probe_tool(tool_name, config) {
            continue;
        }

        plans.push(create_tool_plan(tool_name, config, default_working_dir));

        if let Some(subcommands) = &config.subcommand {
            collect_subcommand_plans(
                &mut plans,
                tool_name,
                config,
                subcommands,
                Vec::new(),
                default_working_dir,
            );
        }
    }

    plans
}

fn should_probe_tool(tool_name: &str, config: &ToolConfig) -> bool {
    if !config.enabled || config.command == "sequence" {
        return false;
    }

    // Skip probe for project-relative commands without explicit availability_check.
    // Commands like "./gradlew" are project-specific and shouldn't be probed globally.
    if config.availability_check.is_none()
        && (config.command.starts_with("./") || config.command.starts_with("../"))
    {
        debug!(
            "Skipping availability probe for tool '{}' with project-relative command '{}'",
            tool_name, config.command
        );
        return false;
    }

    true
}

fn collect_subcommand_plans(
    plans: &mut Vec<ProbePlan>,
    tool_name: &str,
    config: &ToolConfig,
    subcommands: &[SubcommandConfig],
    prefix: Vec<String>,
    default_working_dir: &Path,
) {
    for sub in subcommands {
        if !sub.enabled {
            continue;
        }
        let mut path = prefix.clone();
        path.push(sub.name.clone());

        if let Some(nested) = &sub.subcommand {
            collect_subcommand_plans(
                plans,
                tool_name,
                config,
                nested,
                path.clone(),
                default_working_dir,
            );
        } else if sub.availability_check.is_some() {
            // Only create probe plans for leaf subcommands with explicit availability checks.
            plans.push(create_subcommand_plan(
                tool_name,
                config,
                sub,
                path,
                default_working_dir,
            ));
        }
    }
}

fn create_tool_plan(tool_name: &str, config: &ToolConfig, default_working_dir: &Path) -> ProbePlan {
    let check = config.availability_check.as_ref();
    let command = build_probe_command(config, check, ProbeCommandTarget::Tool);
    let success_codes = resolve_success_codes(check);
    let working_dir = resolve_working_dir(check, default_working_dir);

    ProbePlan {
        target: ProbeTarget::Tool {
            name: tool_name.to_string(),
        },
        command,
        working_dir,
        success_codes,
        timeout_ms: config.timeout_seconds.map(|v| v * 1000).unwrap_or(10_000),
        install_instructions: config.install_instructions.clone(),
    }
}

fn create_subcommand_plan(
    tool_name: &str,
    config: &ToolConfig,
    sub: &SubcommandConfig,
    path: Vec<String>,
    default_working_dir: &Path,
) -> ProbePlan {
    let check = sub
        .availability_check
        .as_ref()
        .or(config.availability_check.as_ref());
    let command = build_probe_command(
        config,
        check,
        ProbeCommandTarget::Subcommand {
            path: &path,
            has_own_check: sub.availability_check.is_some(),
        },
    );
    let success_codes = resolve_success_codes(check);
    let working_dir = resolve_working_dir(check, default_working_dir);

    ProbePlan {
        target: ProbeTarget::Subcommand {
            tool: tool_name.to_string(),
            path,
        },
        command,
        working_dir,
        success_codes,
        timeout_ms: sub
            .timeout_seconds
            .or(config.timeout_seconds)
            .map(|v| v * 1000)
            .unwrap_or(10_000),
        install_instructions: sub
            .install_instructions
            .clone()
            .or_else(|| config.install_instructions.clone()),
    }
}

enum ProbeCommandTarget<'a> {
    Tool,
    Subcommand {
        path: &'a [String],
        has_own_check: bool,
    },
}

fn build_probe_command(
    config: &ToolConfig,
    check: Option<&AvailabilityCheck>,
    target: ProbeCommandTarget<'_>,
) -> Vec<String> {
    let mut command = match check.and_then(|probe| probe.command.as_ref()) {
        Some(cmd) => split_command(cmd),
        None => split_command(&config.command),
    };

    match target {
        ProbeCommandTarget::Tool => apply_tool_probe_logic(&mut command, config, check),
        ProbeCommandTarget::Subcommand {
            path,
            has_own_check,
        } => apply_subcommand_probe_logic(&mut command, check, path, has_own_check),
    }

    command
}

fn apply_tool_probe_logic(
    command: &mut Vec<String>,
    config: &ToolConfig,
    check: Option<&AvailabilityCheck>,
) {
    match check {
        Some(probe) => command.extend(probe.args.clone()),
        None => {
            let base = command
                .first()
                .cloned()
                .filter(|segment| !segment.is_empty())
                .unwrap_or_else(|| config.name.clone());
            // `where` on Windows; `/usr/bin/env which` on Unix
            #[cfg(target_os = "windows")]
            {
                *command = vec!["where".to_string(), base];
            }
            #[cfg(not(target_os = "windows"))]
            {
                *command = vec!["/usr/bin/env".to_string(), "which".to_string(), base];
            }
        }
    }
}

fn apply_subcommand_probe_logic(
    command: &mut Vec<String>,
    check: Option<&AvailabilityCheck>,
    path: &[String],
    has_own_check: bool,
) {
    let skip_subcommand_args = check
        .map(|probe| probe.skip_subcommand_args)
        .unwrap_or(false);
    if !skip_subcommand_args && !has_own_check {
        command.extend(path.iter().filter(|segment| !segment.is_empty()).cloned());
    }

    match check {
        Some(probe) => command.extend(probe.args.clone()),
        None => command.push("--help".to_string()),
    }
}

fn resolve_success_codes(check: Option<&AvailabilityCheck>) -> Vec<i32> {
    check
        .and_then(|probe| probe.success_exit_codes.clone())
        .filter(|codes| !codes.is_empty())
        .unwrap_or_else(|| vec![0])
}

fn resolve_working_dir(check: Option<&AvailabilityCheck>, default_working_dir: &Path) -> PathBuf {
    check
        .and_then(|probe| probe.working_directory.as_deref())
        .map(PathBuf::from)
        .unwrap_or_else(|| default_working_dir.to_path_buf())
}

fn split_command(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
        .collect()
}
