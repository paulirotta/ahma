//! tests/mcp_service_coverage_test.rs

use ahma_mcp::adapter::Adapter;
use ahma_mcp::config::{CommandOption, SubcommandConfig, ToolConfig, ToolHints};
use ahma_mcp::mcp_service::{AhmaMcpService, GuidanceConfig};
use ahma_mcp::operation_monitor::{MonitorConfig, OperationMonitor};
use ahma_mcp::sandbox::Sandbox;
use ahma_mcp::schema_validation::MtdfValidator;
use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};
use ahma_mcp::utils::logging::init_test_logging;
use rmcp::handler::server::ServerHandler;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn test_guidance_config_deserialization() {
    init_test_logging();
    let guidance_json = json!({
        "guidance_blocks": {
            "async_behavior": "**IMPORTANT:** This tool operates asynchronously...",
            "sync_behavior": "This tool runs synchronously and returns results immediately."
        },
        "templates": {
            "async_full": "**IMPORTANT:** This tool operates asynchronously..."
        }
    });

    let config: GuidanceConfig = serde_json::from_value(guidance_json).unwrap();
    assert!(config.guidance_blocks.contains_key("async_behavior"));
    assert!(config.guidance_blocks.contains_key("sync_behavior"));
    assert!(!config.templates.is_empty());
}

#[test]
fn test_mtdf_validator_creation() {
    init_test_logging();
    let _validator = MtdfValidator::new();
    // Just test that it can be created
    // Test passed if we reach this point
}

#[test]
fn test_guidance_config_legacy_structure() {
    init_test_logging();
    let legacy_json = json!({
        "guidance_blocks": {},
        "legacy_guidance": {
            "general_guidance": {
                "test": "legacy guidance text"
            },
            "tool_specific_guidance": {
                "cargo": {
                    "build": "build guidance"
                }
            }
        }
    });

    let config: GuidanceConfig = serde_json::from_value(legacy_json).unwrap();
    assert!(config.legacy_guidance.is_some());
    let legacy = config.legacy_guidance.as_ref().unwrap();
    assert!(legacy.general_guidance.contains_key("test"));
    assert!(legacy.tool_specific_guidance.contains_key("cargo"));
}

#[test]
fn test_guidance_config_empty() {
    init_test_logging();
    let empty_json = json!({
        "guidance_blocks": {}
    });

    let config: GuidanceConfig = serde_json::from_value(empty_json).unwrap();
    assert!(config.guidance_blocks.is_empty());
    assert!(config.templates.is_empty());
    assert!(config.legacy_guidance.is_none());
}

#[test]
fn test_tool_config_creation() {
    init_test_logging();
    let tool_config = ToolConfig {
        name: "cargo".to_string(),
        description: "Cargo build tool".to_string(),
        command: "cargo".to_string(),
        subcommand: Some(vec![SubcommandConfig {
            name: "build".to_string(),
            description: "Build project".to_string(),
            enabled: true,
            options: Some(vec![CommandOption {
                name: "verbose".to_string(),
                alias: Some("v".to_string()),
                option_type: "bool".to_string(),
                description: Some("Enable verbose output".to_string()),
                required: Some(false),
                format: Some("flag".to_string()),
                items: None,
                file_arg: Some(false),
                file_flag: None,
            }]),
            positional_args_first: None,
            positional_args: None,
            synchronous: None,
            timeout_seconds: None,
            guidance_key: None,
            subcommand: None,
            sequence: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
        }]),
        input_schema: None,
        timeout_seconds: None,
        synchronous: None,
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

    assert_eq!(tool_config.name, "cargo");
    assert_eq!(tool_config.command, "cargo");
    assert!(tool_config.enabled);
    assert!(tool_config.subcommand.is_some());
}

#[test]
fn test_subcommand_config_creation() {
    init_test_logging();
    let subcommand = SubcommandConfig {
        name: "build".to_string(),
        description: "Build project".to_string(),
        enabled: true,
        options: None,
        positional_args_first: None,
        positional_args: None,
        synchronous: None,
        timeout_seconds: Some(300),
        guidance_key: Some("build".to_string()),
        subcommand: None,
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    };

    assert_eq!(subcommand.name, "build");
    assert!(subcommand.enabled);
    assert_eq!(subcommand.synchronous, None);
    assert_eq!(subcommand.timeout_seconds, Some(300));
    assert_eq!(subcommand.guidance_key, Some("build".to_string()));
}

#[test]
fn test_option_config_creation() {
    init_test_logging();
    let option = CommandOption {
        name: "verbose".to_string(),
        alias: Some("v".to_string()),
        option_type: "bool".to_string(),
        description: Some("Enable verbose output".to_string()),
        required: Some(false),
        format: Some("flag".to_string()),
        items: None,
        file_arg: Some(false),
        file_flag: None,
    };

    assert_eq!(option.name, "verbose");
    assert_eq!(option.option_type, "bool");
    assert_eq!(option.required, Some(false));
    assert_eq!(option.format, Some("flag".to_string()));
    assert_eq!(option.alias, Some("v".to_string()));
    assert_eq!(option.file_arg, Some(false));
}

#[test]
fn test_tool_hints_creation() {
    init_test_logging();
    let hints = ToolHints {
        build: Some("Build hint".to_string()),
        test: Some("Test hint".to_string()),
        dependencies: None,
        clean: None,
        run: None,
        custom: None,
    };

    assert_eq!(hints.build, Some("Build hint".to_string()));
    assert_eq!(hints.test, Some("Test hint".to_string()));
}

#[tokio::test]
async fn test_service_creation_and_basic_functionality() {
    init_test_logging();
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));

    let sandbox = Arc::new(Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());
    let configs = Arc::new(HashMap::new());
    let guidance = Arc::new(None);

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        false,
    )
    .await
    .unwrap();

    // Test get_info
    let info = service.get_info();
    assert_eq!(info.protocol_version, rmcp::model::ProtocolVersion::LATEST);
    assert!(info.capabilities.tools.is_some());
}

#[tokio::test]
async fn test_service_with_configs() {
    init_test_logging();
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));

    let sandbox = Arc::new(Sandbox::new_test());
    let adapter = Arc::new(
        Adapter::new(
            Arc::clone(&operation_monitor),
            Arc::clone(&shell_pool),
            sandbox,
        )
        .unwrap(),
    );

    let mut configs = HashMap::new();
    let tool_config = ToolConfig {
        name: "test_tool".to_string(),
        command: "echo".to_string(),
        description: "Test tool".to_string(),
        synchronous: Some(true),
        timeout_seconds: Some(60),
        subcommand: Some(vec![SubcommandConfig {
            name: "test_sub".to_string(),
            description: "Test subcommand".to_string(),
            enabled: true,
            synchronous: Some(true),
            timeout_seconds: Some(30),
            options: Some(vec![CommandOption {
                name: "verbose".to_string(),
                alias: None,
                option_type: "bool".to_string(),
                description: Some("Enable verbose output".to_string()),
                required: Some(false),
                format: None,
                items: None,
                file_arg: Some(false),
                file_flag: Some("--verbose".to_string()),
            }]),
            positional_args_first: None,
            positional_args: None,
            guidance_key: Some("test_guidance".to_string()),
            subcommand: None,
            sequence: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
        }]),
        input_schema: None,
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
    configs.insert("test_tool".to_string(), tool_config);

    let guidance = Arc::new(Some(GuidanceConfig {
        guidance_blocks: {
            let mut blocks = HashMap::new();
            blocks.insert(
                "test_guidance".to_string(),
                "Test guidance text".to_string(),
            );
            blocks
        },
        templates: HashMap::new(),
        legacy_guidance: None,
    }));

    let service = AhmaMcpService::new(
        Arc::clone(&adapter),
        Arc::clone(&operation_monitor),
        Arc::new(configs),
        guidance,
        false,
        false,
        false,
    )
    .await
    .unwrap();

    assert!(service.configs.read().unwrap().contains_key("test_tool"));
}

#[test]
fn test_guidance_config_with_legacy_fallback() {
    init_test_logging();
    let guidance_json = json!({
        "guidance_blocks": {
            "test_key": "Test guidance"
        },
        "legacy_guidance": {
            "general_guidance": {
                "await": "Wait guidance"
            },
            "tool_specific_guidance": {
                "cargo": {
                    "build": "Build guidance"
                }
            }
        }
    });

    let config: GuidanceConfig = serde_json::from_value(guidance_json).unwrap();
    assert!(config.guidance_blocks.contains_key("test_key"));
    assert!(config.legacy_guidance.is_some());
    let legacy = config.legacy_guidance.as_ref().unwrap();
    assert!(legacy.general_guidance.contains_key("await"));
    assert!(legacy.tool_specific_guidance.contains_key("cargo"));
}

#[test]
fn test_tool_config_with_nested_subcommands() {
    init_test_logging();
    let tool_config = ToolConfig {
        name: "cargo".to_string(),
        command: "cargo".to_string(),
        description: "Cargo tool".to_string(),
        subcommand: Some(vec![SubcommandConfig {
            name: "build".to_string(),
            description: "Build command".to_string(),
            enabled: true,
            options: None,
            positional_args_first: None,
            positional_args: None,
            synchronous: None,
            timeout_seconds: None,
            guidance_key: None,
            subcommand: Some(vec![SubcommandConfig {
                name: "release".to_string(),
                description: "Release build".to_string(),
                enabled: true,
                options: None,
                positional_args_first: None,
                positional_args: None,
                synchronous: None,
                timeout_seconds: None,
                guidance_key: None,
                subcommand: None,
                sequence: None,
                step_delay_ms: None,
                availability_check: None,
                install_instructions: None,
            }]),
            sequence: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
        }]),
        input_schema: None,
        timeout_seconds: None,
        synchronous: None,
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

    assert_eq!(tool_config.name, "cargo");
    assert!(tool_config.subcommand.is_some());
    let subcommands = tool_config.subcommand.as_ref().unwrap();
    assert_eq!(subcommands.len(), 1);
    assert_eq!(subcommands[0].name, "build");
    assert!(subcommands[0].subcommand.is_some());
    let nested = subcommands[0].subcommand.as_ref().unwrap();
    assert_eq!(nested.len(), 1);
    assert_eq!(nested[0].name, "release");
}

#[tokio::test]
async fn test_service_with_tool_configs() {
    init_test_logging();
    let mut configs = HashMap::new();
    let tool_config = ToolConfig {
        name: "cargo".to_string(),
        command: "cargo".to_string(),
        description: "Cargo tool".to_string(),
        subcommand: Some(vec![SubcommandConfig {
            name: "build".to_string(),
            description: "Build project".to_string(),
            enabled: true,
            options: Some(vec![CommandOption {
                name: "release".to_string(),
                option_type: "bool".to_string(),
                description: Some("Build in release mode".to_string()),
                required: None,
                format: None,
                items: None,
                file_arg: None,
                file_flag: None,
                alias: None,
            }]),
            positional_args_first: None,
            positional_args: None,
            synchronous: None,
            timeout_seconds: None,
            guidance_key: None,
            subcommand: None,
            sequence: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
        }]),
        input_schema: None,
        timeout_seconds: None,
        synchronous: None,
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
    configs.insert("cargo".to_string(), tool_config);

    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));

    let sandbox = Arc::new(Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());
    let configs = Arc::new(configs);
    let guidance = Arc::new(None);

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        false,
    )
    .await
    .unwrap();

    assert!(service.configs.read().unwrap().contains_key("cargo"));
    let cargo_config = service
        .configs
        .read()
        .unwrap()
        .get("cargo")
        .cloned()
        .unwrap();
    assert_eq!(cargo_config.name, "cargo");
    assert!(cargo_config.subcommand.is_some());
}
