//! Comprehensive test coverage for tool_availability module
//!
//! These tests cover the helper functions and edge cases in tool_availability.rs

use ahma_mcp::config::{AvailabilityCheck, SubcommandConfig, ToolConfig, ToolHints};
use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};
use ahma_mcp::tool_availability::{
    AvailabilitySummary, DisabledSubcommand, DisabledTool, evaluate_tool_availability,
    format_install_guidance,
};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn platform_shell_program_for_test() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "powershell"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "bash"
    }
}

fn build_subcommand(
    name: &str,
    check: Option<AvailabilityCheck>,
    install: Option<&str>,
    enabled: bool,
) -> SubcommandConfig {
    SubcommandConfig {
        name: name.to_string(),
        description: format!("{} subcommand", name),
        options: None,
        positional_args: None,
        positional_args_first: None,
        synchronous: None,
        timeout_seconds: None,
        enabled,
        guidance_key: None,
        subcommand: None,
        sequence: None,
        step_delay_ms: None,
        availability_check: check,
        install_instructions: install.map(|s| s.to_string()),
    }
}

fn build_nested_subcommand(
    name: &str,
    nested: Vec<SubcommandConfig>,
    enabled: bool,
) -> SubcommandConfig {
    SubcommandConfig {
        name: name.to_string(),
        description: format!("{} nested subcommand", name),
        options: None,
        positional_args: None,
        positional_args_first: None,
        synchronous: None,
        timeout_seconds: None,
        enabled,
        guidance_key: None,
        subcommand: Some(nested),
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    }
}

fn base_tool(command: &str) -> ToolConfig {
    ToolConfig {
        name: format!("{}_tool", command.replace(' ', "_")),
        description: format!("{} wrapper", command),
        command: command.to_string(),
        subcommand: None,
        sequence: None,
        step_delay_ms: None,
        input_schema: None,
        timeout_seconds: Some(5),
        synchronous: None,
        hints: ToolHints::default(),
        enabled: true,
        guidance_key: None,
        availability_check: None,
        install_instructions: None,
        monitor_level: None,
        monitor_stream: None,
        tool_type: None,
        livelog: None,
    }
}

// === format_install_guidance Tests ===

#[test]
fn test_format_install_guidance_all_available() {
    let summary = AvailabilitySummary {
        filtered_configs: HashMap::new(),
        disabled_tools: Vec::new(),
        disabled_subcommands: Vec::new(),
    };

    let guidance = format_install_guidance(&summary);
    assert_eq!(guidance, "All configured tools are available.");
}

#[test]
fn test_format_install_guidance_disabled_tool_only() {
    let summary = AvailabilitySummary {
        filtered_configs: HashMap::new(),
        disabled_tools: vec![DisabledTool {
            name: "my_tool".to_string(),
            message: "binary not found".to_string(),
            install_instructions: Some("brew install my-tool".to_string()),
        }],
        disabled_subcommands: Vec::new(),
    };

    let guidance = format_install_guidance(&summary);
    assert!(guidance.contains("Disabled tools requiring attention:"));
    assert!(guidance.contains("my_tool"));
    assert!(guidance.contains("binary not found"));
    assert!(guidance.contains("install: brew install my-tool"));
    assert!(!guidance.contains("Disabled subcommands requiring attention:"));
}

#[test]
fn test_format_install_guidance_disabled_subcommand_only() {
    let summary = AvailabilitySummary {
        filtered_configs: HashMap::new(),
        disabled_tools: Vec::new(),
        disabled_subcommands: vec![DisabledSubcommand {
            tool: "cargo".to_string(),
            subcommand_path: "nextest".to_string(),
            message: "nextest not installed".to_string(),
            install_instructions: Some("cargo install cargo-nextest".to_string()),
        }],
    };

    let guidance = format_install_guidance(&summary);
    assert!(guidance.contains("Disabled subcommands requiring attention:"));
    assert!(guidance.contains("cargo::nextest"));
    assert!(guidance.contains("nextest not installed"));
    assert!(guidance.contains("install: cargo install cargo-nextest"));
    assert!(!guidance.contains("Disabled tools requiring attention:"));
}

#[test]
fn test_format_install_guidance_both_tools_and_subcommands() {
    let summary = AvailabilitySummary {
        filtered_configs: HashMap::new(),
        disabled_tools: vec![DisabledTool {
            name: "tool_a".to_string(),
            message: "not installed".to_string(),
            install_instructions: Some("apt install tool-a".to_string()),
        }],
        disabled_subcommands: vec![DisabledSubcommand {
            tool: "tool_b".to_string(),
            subcommand_path: "feature_x".to_string(),
            message: "feature not available".to_string(),
            install_instructions: Some("enable feature".to_string()),
        }],
    };

    let guidance = format_install_guidance(&summary);
    assert!(guidance.contains("Disabled tools requiring attention:"));
    assert!(guidance.contains("Disabled subcommands requiring attention:"));
    assert!(guidance.contains("tool_a"));
    assert!(guidance.contains("tool_b::feature_x"));
}

#[test]
fn test_format_install_guidance_no_install_instructions() {
    let summary = AvailabilitySummary {
        filtered_configs: HashMap::new(),
        disabled_tools: vec![DisabledTool {
            name: "unknown_tool".to_string(),
            message: "cannot find binary".to_string(),
            install_instructions: None,
        }],
        disabled_subcommands: vec![DisabledSubcommand {
            tool: "some_tool".to_string(),
            subcommand_path: "some_cmd".to_string(),
            message: "probe failed".to_string(),
            install_instructions: None,
        }],
    };

    let guidance = format_install_guidance(&summary);
    assert!(guidance.contains("unknown_tool"));
    assert!(guidance.contains("cannot find binary"));
    assert!(!guidance.contains("install:"));
}

#[test]
fn test_format_install_guidance_whitespace_only_instructions() {
    let summary = AvailabilitySummary {
        filtered_configs: HashMap::new(),
        disabled_tools: vec![DisabledTool {
            name: "tool_with_ws".to_string(),
            message: "not available".to_string(),
            install_instructions: Some("   \n  ".to_string()),
        }],
        disabled_subcommands: Vec::new(),
    };

    let guidance = format_install_guidance(&summary);
    assert!(guidance.contains("tool_with_ws"));
    // Whitespace-only instructions should not result in an "install:" line
    // The implementation trims, so empty trimmed string won't add install line
    let lines: Vec<&str> = guidance.lines().collect();
    let has_install_line = lines.iter().any(|l| l.contains("install:"));
    assert!(
        !has_install_line,
        "Whitespace-only instructions should not produce install line"
    );
}

// === evaluate_tool_availability Tests ===

#[tokio::test]
async fn test_evaluate_empty_configs() -> Result<()> {
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let configs: HashMap<String, ToolConfig> = HashMap::new();

    let sandbox = ahma_mcp::sandbox::Sandbox::new_test();
    let summary = evaluate_tool_availability(shell_pool, configs, Path::new("."), &sandbox).await?;

    assert!(summary.filtered_configs.is_empty());
    assert!(summary.disabled_tools.is_empty());
    assert!(summary.disabled_subcommands.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_evaluate_tool_already_disabled() -> Result<()> {
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let mut disabled_tool = base_tool("some_cmd");
    disabled_tool.name = "disabled_tool".to_string();
    disabled_tool.enabled = false;

    let mut configs = HashMap::new();
    configs.insert(disabled_tool.name.clone(), disabled_tool);

    let sandbox = ahma_mcp::sandbox::Sandbox::new_test();
    let summary = evaluate_tool_availability(shell_pool, configs, Path::new("."), &sandbox).await?;

    // Tool was already disabled, so it shouldn't be probed or appear in disabled_tools
    let config = summary.filtered_configs.get("disabled_tool").unwrap();
    assert!(!config.enabled, "Tool should remain disabled");
    // Since it wasn't probed, it shouldn't be in disabled_tools
    assert!(
        !summary
            .disabled_tools
            .iter()
            .any(|d| d.name == "disabled_tool"),
        "Already disabled tool should not be probed"
    );

    Ok(())
}

#[tokio::test]
async fn test_evaluate_sequence_tool_skipped() -> Result<()> {
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let mut sequence_tool = base_tool("sequence");
    sequence_tool.name = "sequence_tool".to_string();
    sequence_tool.command = "sequence".to_string();

    let mut configs = HashMap::new();
    configs.insert(sequence_tool.name.clone(), sequence_tool);

    let sandbox = ahma_mcp::sandbox::Sandbox::new_test();
    let summary = evaluate_tool_availability(shell_pool, configs, Path::new("."), &sandbox).await?;

    // Sequence tools should be skipped (not probed)
    let config = summary.filtered_configs.get("sequence_tool").unwrap();
    assert!(config.enabled, "Sequence tool should remain enabled");
    assert!(
        summary.disabled_tools.is_empty(),
        "Sequence tool should not be in disabled list"
    );

    Ok(())
}

#[tokio::test]
async fn test_evaluate_project_relative_command_skipped() -> Result<()> {
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let mut relative_tool = base_tool("./gradlew");
    relative_tool.name = "gradlew_tool".to_string();
    // No availability_check, so it should be skipped

    let mut configs = HashMap::new();
    configs.insert(relative_tool.name.clone(), relative_tool);

    let sandbox = ahma_mcp::sandbox::Sandbox::new_test();
    let summary = evaluate_tool_availability(shell_pool, configs, Path::new("."), &sandbox).await?;

    // Project-relative commands without availability_check should be skipped
    let config = summary.filtered_configs.get("gradlew_tool").unwrap();
    assert!(
        config.enabled,
        "Project-relative tool without check should remain enabled"
    );
    assert!(
        summary.disabled_tools.is_empty(),
        "Project-relative tool should not be probed"
    );

    Ok(())
}

#[tokio::test]
async fn test_evaluate_tool_with_available_command() -> Result<()> {
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let mut echo_tool = base_tool("echo");
    echo_tool.name = "echo_tool".to_string();
    echo_tool.availability_check = Some(AvailabilityCheck {
        command: Some("echo".to_string()),
        args: vec!["test".to_string()],
        success_exit_codes: Some(vec![0]),
        ..Default::default()
    });

    let mut configs = HashMap::new();
    configs.insert(echo_tool.name.clone(), echo_tool);

    let sandbox = ahma_mcp::sandbox::Sandbox::new_test();
    let summary = evaluate_tool_availability(shell_pool, configs, Path::new("."), &sandbox).await?;

    // echo should be available on all systems
    let config = summary.filtered_configs.get("echo_tool").unwrap();
    assert!(config.enabled, "Echo tool should remain enabled");
    assert!(
        summary.disabled_tools.is_empty(),
        "Echo tool should not be disabled"
    );

    Ok(())
}

#[tokio::test]
async fn test_evaluate_tool_with_unavailable_command() -> Result<()> {
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let mut unavailable_tool = base_tool("nonexistent_cmd_xyz_12345");
    unavailable_tool.name = "unavailable_tool".to_string();
    unavailable_tool.install_instructions = Some("brew install xyz".to_string());

    let mut configs = HashMap::new();
    configs.insert(unavailable_tool.name.clone(), unavailable_tool);

    let sandbox = ahma_mcp::sandbox::Sandbox::new_test();
    let summary = evaluate_tool_availability(shell_pool, configs, Path::new("."), &sandbox).await?;

    // Tool should be disabled
    let config = summary.filtered_configs.get("unavailable_tool").unwrap();
    assert!(!config.enabled, "Unavailable tool should be disabled");
    assert!(
        summary
            .disabled_tools
            .iter()
            .any(|d| d.name == "unavailable_tool"),
        "Tool should be in disabled list"
    );

    Ok(())
}

#[tokio::test]
async fn test_evaluate_subcommand_disabled_when_probe_fails() -> Result<()> {
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let shell_prog = platform_shell_program_for_test();
    let shell_arg = if cfg!(target_os = "windows") {
        "-Command"
    } else {
        "-c"
    };

    let mut tool = base_tool(shell_prog);
    tool.name = format!("{}_tool", shell_prog);
    tool.availability_check = Some(AvailabilityCheck {
        command: Some(shell_prog.to_string()),
        args: vec![shell_arg.to_string(), "exit 0".to_string()],
        ..Default::default()
    });
    tool.subcommand = Some(vec![
        build_subcommand(
            "ok_sub",
            Some(AvailabilityCheck {
                command: Some(shell_prog.to_string()),
                args: vec![shell_arg.to_string(), "exit 0".to_string()],
                ..Default::default()
            }),
            None,
            true,
        ),
        build_subcommand(
            "fail_sub",
            Some(AvailabilityCheck {
                command: Some(shell_prog.to_string()),
                args: vec![shell_arg.to_string(), "exit 1".to_string()],
                ..Default::default()
            }),
            Some("Fix this"),
            true,
        ),
    ]);

    let tool_name = tool.name.clone();
    let mut configs = HashMap::new();
    configs.insert(tool_name.clone(), tool);

    let sandbox = ahma_mcp::sandbox::Sandbox::new_test();
    let summary = evaluate_tool_availability(shell_pool, configs, Path::new("."), &sandbox).await?;

    let config = summary.filtered_configs.get(&tool_name).unwrap();
    assert!(config.enabled, "Parent tool should remain enabled");

    let subs = config.subcommand.as_ref().unwrap();
    let ok_sub = subs.iter().find(|s| s.name == "ok_sub").unwrap();
    let fail_sub = subs.iter().find(|s| s.name == "fail_sub").unwrap();

    assert!(ok_sub.enabled, "ok_sub should remain enabled");
    assert!(!fail_sub.enabled, "fail_sub should be disabled");

    Ok(())
}

#[tokio::test]
async fn test_evaluate_already_disabled_subcommand_not_probed() -> Result<()> {
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let mut tool = base_tool("bash");
    tool.name = "bash_test".to_string();
    tool.availability_check = Some(AvailabilityCheck {
        command: Some("bash".to_string()),
        args: vec!["-c".to_string(), "exit 0".to_string()],
        ..Default::default()
    });
    tool.subcommand = Some(vec![build_subcommand(
        "pre_disabled",
        Some(AvailabilityCheck {
            command: Some("bash".to_string()),
            args: vec!["-c".to_string(), "exit 1".to_string()],
            ..Default::default()
        }),
        None,
        false, // Already disabled
    )]);

    let mut configs = HashMap::new();
    configs.insert(tool.name.clone(), tool);

    let sandbox = ahma_mcp::sandbox::Sandbox::new_test();
    let summary = evaluate_tool_availability(shell_pool, configs, Path::new("."), &sandbox).await?;

    // Subcommand was already disabled, so it shouldn't appear in disabled_subcommands
    assert!(
        !summary
            .disabled_subcommands
            .iter()
            .any(|d| d.subcommand_path == "pre_disabled"),
        "Pre-disabled subcommand should not be probed"
    );

    Ok(())
}

#[tokio::test]
async fn test_evaluate_nested_subcommands() -> Result<()> {
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let shell_prog = platform_shell_program_for_test();
    let shell_arg = if cfg!(target_os = "windows") {
        "-Command"
    } else {
        "-c"
    };

    let mut tool = base_tool(shell_prog);
    tool.name = "nested_tool".to_string();
    tool.availability_check = Some(AvailabilityCheck {
        command: Some(shell_prog.to_string()),
        args: vec![shell_arg.to_string(), "exit 0".to_string()],
        ..Default::default()
    });

    let nested_child = build_subcommand(
        "child",
        Some(AvailabilityCheck {
            command: Some(shell_prog.to_string()),
            args: vec![shell_arg.to_string(), "exit 1".to_string()], // Will fail
            ..Default::default()
        }),
        Some("fix nested child"),
        true,
    );

    let parent_sub = build_nested_subcommand("parent", vec![nested_child], true);

    tool.subcommand = Some(vec![parent_sub]);

    let tool_name = tool.name.clone();
    let mut configs = HashMap::new();
    configs.insert(tool_name.clone(), tool);

    let sandbox = ahma_mcp::sandbox::Sandbox::new_test();
    let summary = evaluate_tool_availability(shell_pool, configs, Path::new("."), &sandbox).await?;

    // The nested child should be disabled
    let config = summary.filtered_configs.get(&tool_name).unwrap();
    let parent = config
        .subcommand
        .as_ref()
        .unwrap()
        .iter()
        .find(|s| s.name == "parent")
        .unwrap();
    let child = parent
        .subcommand
        .as_ref()
        .unwrap()
        .iter()
        .find(|s| s.name == "child")
        .unwrap();

    assert!(!child.enabled, "Nested child should be disabled");

    Ok(())
}

#[tokio::test]
async fn test_evaluate_custom_success_exit_codes() -> Result<()> {
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let shell_prog = platform_shell_program_for_test();
    let shell_arg = if cfg!(target_os = "windows") {
        "-Command"
    } else {
        "-c"
    };

    let mut tool = base_tool(shell_prog);
    tool.name = "exit_code_tool".to_string();
    tool.availability_check = Some(AvailabilityCheck {
        command: Some(shell_prog.to_string()),
        args: vec![shell_arg.to_string(), "exit 42".to_string()],
        success_exit_codes: Some(vec![42, 43, 44]), // Accept 42 as success
        ..Default::default()
    });

    let tool_name = tool.name.clone();
    let mut configs = HashMap::new();
    configs.insert(tool_name.clone(), tool);

    let sandbox = ahma_mcp::sandbox::Sandbox::new_test();
    let summary = evaluate_tool_availability(shell_pool, configs, Path::new("."), &sandbox).await?;

    let config = summary.filtered_configs.get(&tool_name).unwrap();
    assert!(
        config.enabled,
        "Tool should remain enabled when exit code matches success_exit_codes"
    );

    Ok(())
}

#[test]
fn test_disabled_tool_struct() {
    let tool = DisabledTool {
        name: "test_tool".to_string(),
        message: "test message".to_string(),
        install_instructions: Some("test install".to_string()),
    };

    // Test Clone trait
    let cloned = tool.clone();
    assert_eq!(cloned.name, "test_tool");
    assert_eq!(cloned.message, "test message");
    assert_eq!(
        cloned.install_instructions,
        Some("test install".to_string())
    );

    // Test Debug trait
    let debug = format!("{:?}", tool);
    assert!(debug.contains("DisabledTool"));
    assert!(debug.contains("test_tool"));
}

#[test]
fn test_disabled_subcommand_struct() {
    let sub = DisabledSubcommand {
        tool: "parent_tool".to_string(),
        subcommand_path: "sub_a_sub_b".to_string(),
        message: "failed probe".to_string(),
        install_instructions: None,
    };

    let cloned = sub.clone();
    assert_eq!(cloned.tool, "parent_tool");
    assert_eq!(cloned.subcommand_path, "sub_a_sub_b");
    assert!(cloned.install_instructions.is_none());

    // Test Debug trait
    let debug = format!("{:?}", sub);
    assert!(debug.contains("DisabledSubcommand"));
    assert!(debug.contains("parent_tool"));
}

#[test]
fn test_availability_summary_struct() {
    let summary = AvailabilitySummary {
        filtered_configs: HashMap::new(),
        disabled_tools: vec![],
        disabled_subcommands: vec![],
    };

    // Test Clone trait
    let cloned = summary.clone();
    assert!(cloned.filtered_configs.is_empty());
    assert!(cloned.disabled_tools.is_empty());
    assert!(cloned.disabled_subcommands.is_empty());

    // Test Debug trait
    let debug = format!("{:?}", summary);
    assert!(debug.contains("AvailabilitySummary"));
}
