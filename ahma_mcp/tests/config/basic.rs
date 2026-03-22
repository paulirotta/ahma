use ahma_mcp::config::{
    CommandOption, SequenceStep, SubcommandConfig, ToolConfig, ToolHints, load_tool_configs_sync,
};
use ahma_mcp::shell::cli::AppConfig;
use ahma_mcp::utils::logging::init_test_logging;
use serde_json::json;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_tool_config_defaults() {
    init_test_logging();
    let config = ToolConfig {
        name: "test_tool".to_string(),
        description: "Test tool description".to_string(),
        command: "test_command".to_string(),
        subcommand: None,
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
    };

    assert_eq!(config.name, "test_tool");
    assert_eq!(config.command, "test_command");
    assert!(config.enabled);
    assert!(config.subcommand.is_none());
    assert!(config.timeout_seconds.is_none());
    assert!(config.synchronous.is_none());
    assert!(config.sequence.is_none());
    assert!(config.step_delay_ms.is_none());
}

#[test]
fn test_subcommand_config_defaults() {
    init_test_logging();
    let subcommand = SubcommandConfig {
        name: "build".to_string(),
        description: "Build the project".to_string(),
        options: None,
        positional_args_first: None, positional_args: None,
        synchronous: None,
        timeout_seconds: None,
        enabled: true,
        guidance_key: None,
        subcommand: None,

        sequence: None,

        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    };

    assert_eq!(subcommand.name, "build");
    assert!(subcommand.enabled);
    assert!(subcommand.options.is_none());
    assert!(subcommand.positional_args.is_none());
}

#[test]
fn test_option_config_structure() {
    init_test_logging();
    let option = CommandOption {
        name: "verbose".to_string(),
        alias: Some("v".to_string()),
        option_type: "boolean".to_string(),
        description: Some("Enable verbose output".to_string()),
        format: None,
        items: None,
        required: Some(false),
        file_arg: Some(false),
        file_flag: None,
    };

    assert_eq!(option.name, "verbose");
    assert_eq!(option.alias, Some("v".to_string()));
    assert_eq!(option.option_type, "boolean");
    assert_eq!(option.required, Some(false));
    assert_eq!(option.file_arg, Some(false));
}

#[test]
fn test_tool_hints_default() {
    init_test_logging();
    let hints = ToolHints::default();
    assert!(hints.build.is_none());
    assert!(hints.test.is_none());
    assert!(hints.dependencies.is_none());
    assert!(hints.clean.is_none());
    assert!(hints.run.is_none());
    assert!(hints.custom.is_none());
}

#[test]
fn test_tool_hints_with_operations() {
    init_test_logging();

    let hints = ToolHints {
        build: Some("Use --release for optimized builds".to_string()),
        test: Some("Run tests with --no-capture for debug output".to_string()),
        dependencies: None,
        clean: None,
        run: None,
        custom: None,
    };

    assert_eq!(
        hints.build,
        Some("Use --release for optimized builds".to_string())
    );
    assert_eq!(
        hints.test,
        Some("Run tests with --no-capture for debug output".to_string())
    );
}

#[test]
fn test_tool_config_serialization() {
    init_test_logging();
    let config = ToolConfig {
        name: "cargo".to_string(),
        description: "Rust package manager".to_string(),
        command: "cargo".to_string(),
        subcommand: Some(vec![SubcommandConfig {
            name: "build".to_string(),
            description: "Compile the current package".to_string(),
            options: Some(vec![CommandOption {
                name: "release".to_string(),
                alias: None,
                option_type: "boolean".to_string(),
                description: Some("Build artifacts in release mode".to_string()),
                format: None,
                items: None,
                required: Some(false),
                file_arg: Some(false),
                file_flag: None,
            }]),
            positional_args_first: None, positional_args: None,
            synchronous: Some(true),
            timeout_seconds: Some(300),
            enabled: true,
            guidance_key: Some("cargo_build".to_string()),
            subcommand: None,

            sequence: None,

            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
        }]),
        input_schema: None,
        timeout_seconds: Some(600),
        synchronous: Some(true),
        hints: ToolHints {
            build: Some("Use --release for production builds".to_string()),
            test: None,
            dependencies: None,
            clean: None,
            run: None,
            custom: None,
        },
        enabled: true,
        guidance_key: Some("cargo_main".to_string()),
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
        monitor_level: None,
        monitor_stream: None,
    };

    let json = serde_json::to_string_pretty(&config).unwrap();
    assert!(json.contains("\"name\": \"cargo\""));
    assert!(json.contains("\"command\": \"cargo\""));
    assert!(json.contains("\"subcommand\""));
    assert!(json.contains("\"build\""));
    assert!(json.contains("\"release\""));

    // Test round-trip
    let deserialized: ToolConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.name, config.name);
    assert_eq!(deserialized.command, config.command);
    assert_eq!(deserialized.subcommand.as_ref().unwrap().len(), 1);
}

#[test]
fn test_tool_config_deserialization() {
    init_test_logging();
    let json = r#"
    {
        "name": "git",
        "description": "Version control system",
        "command": "git",
        "timeout_seconds": 120,
        "force_synchronous": false,
        "enabled": true,
        "subcommand": [
            {
                "name": "commit",
                "description": "Record changes to the repository",
                "options": [
                    {
                        "name": "message",
                        "type": "string",
                        "description": "Use the given message as the commit message",
                        "required": true,
                        "file_arg": true,
                        "file_flag": "-F"
                    }
                ],
                "enabled": true
            }
        ],
        "hints": {
            "custom": {
                "default": "Git is a distributed version control system",
                "commit": "Always write descriptive commit messages"
            }
        }
    }
    "#;

    let config: ToolConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.name, "git");
    assert_eq!(config.command, "git");
    assert_eq!(config.timeout_seconds, Some(120));
    assert_eq!(config.synchronous, Some(false));
    assert!(config.enabled);

    let subcommands = config.subcommand.unwrap();
    assert_eq!(subcommands.len(), 1);
    assert_eq!(subcommands[0].name, "commit");

    let options = subcommands[0].options.as_ref().unwrap();
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].name, "message");
    assert_eq!(options[0].option_type, "string");
    assert_eq!(options[0].required, Some(true));
    assert_eq!(options[0].file_arg, Some(true));
    assert_eq!(options[0].file_flag, Some("-F".to_string()));

    assert_eq!(
        config.hints.custom.as_ref().and_then(|c| c.get("default")),
        Some(&"Git is a distributed version control system".to_string())
    );
    assert_eq!(
        config.hints.custom.as_ref().and_then(|c| c.get("commit")),
        Some(&"Always write descriptive commit messages".to_string())
    );
}

#[test]
fn test_nested_subcommands() {
    init_test_logging();
    let json = r#"
    {
        "name": "docker",
        "description": "Container management tool",
        "command": "docker",
        "enabled": true,
        "subcommand": [
            {
                "name": "container",
                "description": "Manage containers",
                "enabled": true,
                "subcommand": [
                    {
                        "name": "ls",
                        "description": "List containers",
                        "enabled": true
                    },
                    {
                        "name": "stop",
                        "description": "Stop containers",
                        "enabled": true,
                        "positional_args": [
                            {
                                "name": "container_id",
                                "type": "string",
                                "description": "Container ID or name",
                                "required": true
                            }
                        ]
                    }
                ]
            }
        ]
    }
    "#;

    let config: ToolConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.name, "docker");

    let subcommands = config.subcommand.unwrap();
    assert_eq!(subcommands.len(), 1);
    assert_eq!(subcommands[0].name, "container");

    let nested_subcommands = subcommands[0].subcommand.as_ref().unwrap();
    assert_eq!(nested_subcommands.len(), 2);
    assert_eq!(nested_subcommands[0].name, "ls");
    assert_eq!(nested_subcommands[1].name, "stop");

    let positional_args = nested_subcommands[1].positional_args.as_ref().unwrap();
    assert_eq!(positional_args.len(), 1);
    assert_eq!(positional_args[0].name, "container_id");
    assert_eq!(positional_args[0].required, Some(true));
}

#[test]
fn test_load_tool_configs_empty_directory() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();
    let configs = load_tool_configs_sync(&AppConfig::default(), Some(temp_dir.path())).unwrap();
    assert!(configs.is_empty());
}

#[test]
fn test_load_tool_configs_nonexistent_directory() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();
    let nonexistent_path = temp_dir.path().join("nonexistent");
    let configs = load_tool_configs_sync(&AppConfig::default(), Some(&nonexistent_path)).unwrap();
    assert!(configs.is_empty());
}

#[test]
fn test_load_tool_configs_valid_json() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();
    let tools_dir = temp_dir.path();

    // Create a valid tool config file
    let config_json = json!({
        "name": "test_tool",
        "description": "A test tool",
        "command": "test",
        "enabled": true,
        "timeout_seconds": 60
    });

    fs::write(
        tools_dir.join("test_tool.json"),
        serde_json::to_string_pretty(&config_json).unwrap(),
    )
    .unwrap();

    let configs = load_tool_configs_sync(&AppConfig::default(), Some(tools_dir)).unwrap();
    assert_eq!(configs.len(), 1);
    assert!(configs.contains_key("test_tool"));

    let config = &configs["test_tool"];
    assert_eq!(config.name, "test_tool");
    assert_eq!(config.command, "test");
    assert_eq!(config.timeout_seconds, Some(60));
}

#[test]
fn test_load_tool_configs_disabled_tool() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();
    let tools_dir = temp_dir.path();

    // Create a disabled tool config file
    let config_json = json!({
        "name": "disabled_tool",
        "description": "A disabled test tool",
        "command": "disabled",
        "enabled": false
    });

    fs::write(
        tools_dir.join("disabled_tool.json"),
        serde_json::to_string_pretty(&config_json).unwrap(),
    )
    .unwrap();

    let configs = load_tool_configs_sync(&AppConfig::default(), Some(tools_dir)).unwrap();
    assert!(configs.is_empty()); // Disabled tools should not be loaded
}

#[test]
fn test_load_tool_configs_multiple_files() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();
    let tools_dir = temp_dir.path();

    // Create multiple tool config files
    let configs_to_create = vec![
        ("tool1", "command1", true),
        ("tool2", "command2", true),
        ("tool3", "command3", false), // disabled
    ];

    for (name, command, enabled) in configs_to_create {
        let config_json = json!({
            "name": name,
            "description": format!("Description for {}", name),
            "command": command,
            "enabled": enabled
        });

        fs::write(
            tools_dir.join(format!("{}.json", name)),
            serde_json::to_string_pretty(&config_json).unwrap(),
        )
        .unwrap();
    }

    // Create a non-JSON file that should be ignored
    fs::write(tools_dir.join("readme.txt"), "This is not a JSON file").unwrap();

    let configs = load_tool_configs_sync(&AppConfig::default(), Some(tools_dir)).unwrap();
    assert_eq!(configs.len(), 2); // Only enabled tools
    assert!(configs.contains_key("tool1"));
    assert!(configs.contains_key("tool2"));
    assert!(!configs.contains_key("tool3")); // disabled
}

#[test]
fn test_load_tool_configs_invalid_json() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();
    let tools_dir = temp_dir.path();

    // Create an invalid JSON file
    fs::write(tools_dir.join("invalid.json"), "{ invalid json content").unwrap();

    // Should succeed but skip the invalid file (logged as warning)
    let result = load_tool_configs_sync(&AppConfig::default(), Some(tools_dir));
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty()); // No valid configs loaded
}

#[test]
fn test_tool_config_with_input_schema() {
    init_test_logging();
    let json = r#"
    {
        "name": "test_tool",
        "description": "Test tool with schema",
        "command": "test",
        "enabled": true,
        "input_schema": {
            "type": "object",
            "properties": {
                "param1": {
                    "type": "string",
                    "description": "First parameter"
                },
                "param2": {
                    "type": "integer",
                    "description": "Second parameter"
                }
            },
            "required": ["param1"]
        }
    }
    "#;

    let config: ToolConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.name, "test_tool");

    let schema = config.input_schema.unwrap();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"].is_object());
    assert!(schema["required"].is_array());
}

#[test]
fn test_option_config_all_fields() {
    init_test_logging();
    let json = r#"
    {
        "name": "file",
        "alias": "f",
        "type": "string",
        "description": "Input file path",
        "format": "path",
        "required": true,
        "file_arg": true,
        "file_flag": "--input-file"
    }
    }"#;

    let option: CommandOption = serde_json::from_str(json).unwrap();
    assert_eq!(option.name, "file");
    assert_eq!(option.alias, Some("f".to_string()));
    assert_eq!(option.option_type, "string");
    assert_eq!(option.description, Some("Input file path".to_string()));
    assert_eq!(option.format, Some("path".to_string()));
    assert_eq!(option.required, Some(true));
    assert_eq!(option.file_arg, Some(true));
    assert_eq!(option.file_flag, Some("--input-file".to_string()));
}

#[test]
fn test_subcommand_config_with_overrides() {
    init_test_logging();
    let json = r#"
    {
        "name": "long_task",
        "description": "A task that takes a long time",
        "force_synchronous": true,
        "timeout_seconds": 3600,
        "enabled": true,
        "guidance_key": "long_task_guidance"
    }
    "#;

    let subcommand: SubcommandConfig = serde_json::from_str(json).unwrap();
    assert_eq!(subcommand.name, "long_task");
    assert_eq!(subcommand.synchronous, Some(true));
    assert_eq!(subcommand.timeout_seconds, Some(3600));
    assert_eq!(
        subcommand.guidance_key,
        Some("long_task_guidance".to_string())
    );
    assert!(subcommand.enabled);
}

#[test]
fn test_sequence_step_serialization() {
    init_test_logging();
    use serde_json::Map;

    let mut args = Map::new();
    args.insert("fix".to_string(), json!(true));
    args.insert("allow-dirty".to_string(), json!(true));

    let step = SequenceStep {
        tool: "cargo".to_string(),
        subcommand: "clippy".to_string(),
        args: args.clone(),
        description: Some("Run clippy with auto-fix".to_string()),
    };

    let json = serde_json::to_string_pretty(&step).unwrap();
    assert!(json.contains("\"tool\": \"cargo\""));
    assert!(json.contains("\"subcommand\": \"clippy\""));
    assert!(json.contains("\"fix\": true"));
    assert!(json.contains("\"allow-dirty\": true"));

    // Test round-trip
    let deserialized: SequenceStep = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.tool, step.tool);
    assert_eq!(deserialized.subcommand, step.subcommand);
    assert_eq!(deserialized.args.len(), 2);
}

#[test]
fn test_sequence_tool_config() {
    init_test_logging();
    use serde_json::Map;

    let mut args = Map::new();
    args.insert("workspace".to_string(), json!(true));

    let config = ToolConfig {
        name: "example_sequence".to_string(),
        description: "Example sequence tool for testing".to_string(),
        command: "sequence".to_string(),
        subcommand: None,
        input_schema: None,
        timeout_seconds: Some(1200),
        synchronous: None,
        hints: ToolHints::default(),
        enabled: true,
        guidance_key: None,
        sequence: Some(vec![
            SequenceStep {
                tool: "cargo".to_string(),
                subcommand: "fmt".to_string(),
                args: Map::new(),
                description: Some("Format code".to_string()),
            },
            SequenceStep {
                tool: "cargo".to_string(),
                subcommand: "nextest_run".to_string(),
                args,
                description: Some("Run tests".to_string()),
            },
        ]),
        step_delay_ms: Some(100),
        availability_check: None,
        install_instructions: None,
        monitor_level: None,
        monitor_stream: None,
    };

    assert_eq!(config.name, "example_sequence");
    assert_eq!(config.command, "sequence");
    assert!(config.sequence.is_some());
    assert_eq!(config.sequence.as_ref().unwrap().len(), 2);
    assert_eq!(config.step_delay_ms, Some(100));

    // Test serialization
    let json = serde_json::to_string_pretty(&config).unwrap();
    assert!(json.contains("\"sequence\""));
    assert!(json.contains("\"step_delay_ms\""));

    // Test round-trip
    let deserialized: ToolConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.name, config.name);
    assert!(deserialized.sequence.is_some());
    assert_eq!(deserialized.sequence.as_ref().unwrap().len(), 2);
}
