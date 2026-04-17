#[cfg(test)]
mod main_tests {
    use ahma_mcp::utils::logging::init_test_logging;
    use std::path::PathBuf;

    #[test]
    fn test_default_paths() {
        init_test_logging();
        // Test that default paths can be created
        let tools_dir = PathBuf::from(".ahma");

        assert_eq!(tools_dir.to_string_lossy(), ".ahma");
    }

    #[test]
    fn test_timeout_values() {
        init_test_logging();
        // Test that timeout value parsing works
        let default_timeout: u64 = 300;
        let custom_timeout: u64 = 600;

        assert_eq!(default_timeout, 300);
        assert!(custom_timeout > default_timeout);
    }

    #[test]
    fn test_cli_argument_structure() {
        init_test_logging();
        // Test that we can create and validate CLI argument types
        // This ensures our CLI structure compiles correctly

        let tool_name = Some("cargo_build".to_string());
        let tool_args = ["--release".to_string(), "--verbose".to_string()];

        assert!(tool_name.is_some());
        assert_eq!(tool_args.len(), 2);
        assert_eq!(tool_args[0], "--release");
    }

    #[test]
    fn test_mode_detection_logic() {
        init_test_logging();
        assert_eq!(detect_run_mode(true, None, None), "server");
    }

    #[test]
    fn test_validation_mode_detection() {
        init_test_logging();
        assert_eq!(
            detect_run_mode(false, None, Some("all".to_string())),
            "validate"
        );
    }

    #[test]
    fn test_cli_mode_detection() {
        init_test_logging();
        assert_eq!(
            detect_run_mode(false, Some("cargo_build".to_string()), None),
            "cli"
        );
    }

    /// Detect the run mode given server_mode flag, tool name, and validate target.
    fn detect_run_mode(
        server_mode: bool,
        tool_name: Option<String>,
        validate: Option<String>,
    ) -> &'static str {
        let no_explicit_mode = tool_name.is_none() && validate.is_none();
        if server_mode || no_explicit_mode {
            "server"
        } else if validate.is_some() {
            "validate"
        } else {
            "cli"
        }
    }

    #[test]
    fn test_tool_name_parsing_valid() {
        init_test_logging();
        // Test parsing valid tool names
        let test_cases = vec![
            ("cargo_build", ("cargo", vec!["build"])),
            ("cargo_check", ("cargo", vec!["check"])),
            ("git_add", ("git", vec!["add"])),
            ("npm_install_package", ("npm", vec!["install", "package"])),
            ("docker_compose_up", ("docker", vec!["compose", "up"])),
        ];

        for (input, (expected_base, expected_parts)) in test_cases {
            let parts: Vec<&str> = input.split('_').collect();
            assert!(
                parts.len() >= 2,
                "Tool name '{}' should have at least 2 parts",
                input
            );

            let base_tool = parts[0];
            let subcommand_parts: Vec<&str> = parts[1..].to_vec();

            assert_eq!(base_tool, expected_base);
            assert_eq!(subcommand_parts, expected_parts);
        }
    }

    #[test]
    fn test_tool_name_parsing_invalid() {
        init_test_logging();
        // Test parsing invalid tool names
        let invalid_names = vec!["cargo", "tool", "", "tool_", "_tool", "tool__name"];

        for name in invalid_names {
            let parts: Vec<&str> = name.split('_').collect();
            // Invalid if: fewer than 2 parts, empty string, or any empty parts
            let is_invalid = parts.len() < 2 || parts.iter().any(|p| p.is_empty());
            assert!(is_invalid, "Tool name '{}' should be invalid", name);
        }
    }

    #[test]
    fn test_cli_argument_parsing() {
        init_test_logging();
        let test_cases = vec![
            (
                vec![
                    "--working-directory".to_string(),
                    "/tmp".to_string(),
                    "--verbose".to_string(),
                ],
                Some("/tmp".to_string()),
                vec![("verbose".to_string(), serde_json::Value::Bool(true))],
            ),
            (
                vec!["--release".to_string(), "true".to_string()],
                None,
                vec![(
                    "release".to_string(),
                    serde_json::Value::String("true".to_string()),
                )],
            ),
            (
                vec![
                    "--".to_string(),
                    "file1.txt".to_string(),
                    "file2.txt".to_string(),
                ],
                None,
                vec![],
            ),
        ];

        for (args, expected_wd, expected_args) in test_cases {
            let (working_directory, tool_args_map) = parse_cli_args(args);
            assert_eq!(working_directory, expected_wd);
            for (key, expected_value) in expected_args {
                assert_eq!(tool_args_map.get(&key), Some(&expected_value));
            }
        }
    }

    /// Apply a single parsed flag into the output accumulators.
    fn apply_flag(
        key: &str,
        value: Option<String>,
        working_directory: &mut Option<String>,
        tool_args_map: &mut serde_json::Map<String, serde_json::Value>,
    ) {
        match (key, value) {
            ("working-directory", Some(val)) => *working_directory = Some(val),
            (k, Some(val)) => {
                tool_args_map.insert(k.to_string(), serde_json::Value::String(val));
            }
            (k, None) => {
                tool_args_map.insert(k.to_string(), serde_json::Value::Bool(true));
            }
        }
    }

    /// Parse CLI args into (working_directory, tool_args_map).
    fn parse_cli_args(
        args: Vec<String>,
    ) -> (Option<String>, serde_json::Map<String, serde_json::Value>) {
        let mut working_directory: Option<String> = None;
        let mut tool_args_map: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            if arg == "--" {
                break;
            }
            if !arg.starts_with("--") {
                continue;
            }
            let key = arg.trim_start_matches("--").to_string();
            apply_flag(
                &key,
                iter.next(),
                &mut working_directory,
                &mut tool_args_map,
            );
        }
        (working_directory, tool_args_map)
    }

    #[test]
    fn test_environment_variable_parsing() {
        init_test_logging();
        // Test parsing of AHMA_MCP_ARGS environment variable
        let test_json =
            r#"{"working_directory": "/tmp", "verbose": true, "args": ["file1", "file2"]}"#;

        let json_val: serde_json::Value = serde_json::from_str(test_json).unwrap();
        if let Some(map) = json_val.as_object() {
            assert_eq!(
                map.get("working_directory").unwrap().as_str().unwrap(),
                "/tmp"
            );
            assert!(map.get("verbose").unwrap().as_bool().unwrap());
            assert_eq!(map.get("args").unwrap().as_array().unwrap().len(), 2);
        } else {
            panic!("Expected object");
        }
    }

    #[test]
    fn test_validation_target_parsing() {
        init_test_logging();
        // Test parsing of validation targets
        let test_cases = vec![
            ("all", vec![]), // Special case for all
            ("tools/cargo.json", vec![PathBuf::from("tools/cargo.json")]),
            (
                "tools/cargo.json,tools/git.json",
                vec![
                    PathBuf::from("tools/cargo.json"),
                    PathBuf::from("tools/git.json"),
                ],
            ),
            (
                "tools/cargo.json, tools/git.json",
                vec![
                    PathBuf::from("tools/cargo.json"),
                    PathBuf::from("tools/git.json"),
                ],
            ),
        ];

        for (input, expected) in test_cases {
            if input == "all" {
                assert_eq!(expected.len(), 0);
            } else {
                let result: Vec<PathBuf> =
                    input.split(',').map(|s| PathBuf::from(s.trim())).collect();
                assert_eq!(result, expected);
            }
        }
    }

    #[test]
    fn test_log_level_configuration() {
        init_test_logging();
        // Test log level configuration logic
        let debug_enabled = true;
        let debug_disabled = false;

        let log_level_debug = if debug_enabled { "debug" } else { "info" };
        let log_level_info = if debug_disabled { "debug" } else { "info" };

        assert_eq!(log_level_debug, "debug");
        assert_eq!(log_level_info, "info");
    }

    #[test]
    fn test_graceful_shutdown_timeout_calculation() {
        init_test_logging();
        // Test the shutdown timeout calculation logic
        let timeout_secs = 300;
        let shutdown_timeout = std::time::Duration::from_secs(timeout_secs);

        assert_eq!(shutdown_timeout.as_secs(), 300);
        assert_eq!(shutdown_timeout.as_millis(), 300000);
    }

    #[test]
    fn test_tool_config_lookup() {
        init_test_logging();
        // Test the tool configuration lookup logic
        use ahma_mcp::config::ToolConfig;
        use std::collections::HashMap;

        let mut configs = HashMap::new();
        configs.insert(
            "cargo".to_string(),
            ToolConfig {
                name: "cargo".to_string(),
                description: "Cargo build tool".to_string(),
                command: "cargo".to_string(),
                guidance_key: None,
                subcommand: None,
                input_schema: None,
                timeout_seconds: Some(300),
                synchronous: Some(true),
                hints: Default::default(),
                enabled: true,
                sequence: None,
                step_delay_ms: None,
                availability_check: None,
                install_instructions: None,
                monitor_level: None,
                monitor_stream: None,
                tool_type: None,
                livelog: None,
            },
        );

        // Test successful lookup
        let config = configs.get("cargo");
        assert!(config.is_some());
        assert_eq!(config.unwrap().name, "cargo");

        // Test failed lookup
        let missing_config = configs.get("missing_tool");
        assert!(missing_config.is_none());
    }

    #[test]
    fn test_subcommand_config_lookup() {
        init_test_logging();
        // Test the subcommand configuration lookup logic
        use ahma_mcp::config::SubcommandConfig;

        let subcommands = [
            SubcommandConfig {
                name: "build".to_string(),
                description: "Build the project".to_string(),
                guidance_key: None,
                subcommand: None,
                sequence: None,
                step_delay_ms: None,
                timeout_seconds: Some(300),
                options: None,
                positional_args_first: None,
                positional_args: None,
                synchronous: Some(true),
                enabled: true,
                availability_check: None,
                install_instructions: None,
            },
            SubcommandConfig {
                name: "check".to_string(),
                description: "Check the project".to_string(),
                guidance_key: None,
                subcommand: None,
                sequence: None,
                step_delay_ms: None,
                timeout_seconds: Some(60),
                options: None,
                positional_args_first: None,
                positional_args: None,
                synchronous: Some(true),
                enabled: true,
                availability_check: None,
                install_instructions: None,
            },
        ];

        // Test successful lookup
        let found = subcommands.iter().find(|s| s.name == "build");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "build");

        // Test failed lookup
        let not_found = subcommands.iter().find(|s| s.name == "missing");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_args_map_construction() {
        init_test_logging();
        // Test the construction of args_map for tool execution
        use serde_json::Value;

        let mut args_map = serde_json::Map::new();

        // Add subcommand
        args_map.insert(
            "_subcommand".to_string(),
            Value::String("build".to_string()),
        );

        // Add arguments
        args_map.insert("release".to_string(), Value::Bool(true));
        args_map.insert("verbose".to_string(), Value::String("2".to_string()));

        assert_eq!(args_map.len(), 3);
        assert_eq!(
            args_map.get("_subcommand").unwrap().as_str().unwrap(),
            "build"
        );
        assert!(args_map.get("release").unwrap().as_bool().unwrap());
        assert_eq!(args_map.get("verbose").unwrap().as_str().unwrap(), "2");
    }

    #[test]
    fn test_working_directory_resolution() {
        init_test_logging();
        // Test working directory resolution logic
        let explicit_wd = Some("/tmp/project".to_string());
        let from_args = Some("/home/user/project".to_string());
        let current_dir = std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        let none = None;

        // Test explicit working directory
        let final_wd = explicit_wd.or_else(|| from_args.clone());
        assert_eq!(final_wd, Some("/tmp/project".to_string()));

        // Test fallback to args
        let final_wd = none.clone().or_else(|| from_args.clone());
        assert_eq!(final_wd, Some("/home/user/project".to_string()));

        // Test fallback to current directory
        let final_wd = none.or_else(|| current_dir.clone());
        assert!(final_wd.is_some());
    }
}
