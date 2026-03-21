use ahma_mcp::config::load_tool_configs;
use ahma_mcp::shell::cli::Cli;
use clap::Parser;
use tempfile::tempdir;

#[tokio::test]
async fn test_load_builtin_tools_async() {
    let temp_dir = tempdir().unwrap();

    let args_rust = vec!["ahma_mcp", "--tool", "rust"];
    let cli_rust = Cli::try_parse_from(args_rust).unwrap();
    let configs_rust = load_tool_configs(&cli_rust, Some(temp_dir.path()))
        .await
        .unwrap();
    assert!(
        configs_rust.contains_key("cargo"),
        "Should load bundled rust.json (named cargo)"
    );

    let args_python = vec!["ahma_mcp", "--tool", "python"];
    let cli_python = Cli::try_parse_from(args_python).unwrap();
    let configs_python = load_tool_configs(&cli_python, Some(temp_dir.path()))
        .await
        .unwrap();
    assert!(
        configs_python.contains_key("python"),
        "Should load bundled python.json"
    );

    let args_multiple = vec!["ahma_mcp", "--tool", "rust", "--tool", "python"];
    let cli_multiple = Cli::try_parse_from(args_multiple).unwrap();
    let configs_multiple = load_tool_configs(&cli_multiple, Some(temp_dir.path()))
        .await
        .unwrap();
    assert!(configs_multiple.contains_key("cargo"), "Should load cargo");
    assert!(
        configs_multiple.contains_key("python"),
        "Should load python"
    );
}

/// Verify that a user-provided .ahma/ tool definition overrides the bundled version.
#[tokio::test]
async fn test_filesystem_overrides_bundled_tool() {
    let temp_dir = tempdir().unwrap();

    // Create a local rust.json that defines "cargo" with a custom description
    let custom_cargo = r#"{
  "name": "cargo",
  "description": "Custom user-defined cargo tool",
  "command": "cargo",
  "enabled": true,
  "subcommand": [
    {
      "name": "build",
      "description": "Custom build",
      "options": [
        {
          "name": "release",
          "type": "boolean",
          "description": "Release mode"
        }
      ]
    }
  ]
}"#;
    std::fs::write(temp_dir.path().join("rust.json"), custom_cargo).unwrap();

    // Load with --tool rust flag (bundled) AND the local override
    let cli = Cli::try_parse_from(["ahma_mcp", "--tool", "rust"]).unwrap();
    let configs = load_tool_configs(&cli, Some(temp_dir.path()))
        .await
        .unwrap();

    assert!(configs.contains_key("cargo"), "Should have cargo tool");
    let cargo = &configs["cargo"];
    assert_eq!(
        cargo.description, "Custom user-defined cargo tool",
        "Local .ahma/ definition should override the bundled version"
    );
}

/// Verify that reserved tool names (core built-in tools) are rejected from .ahma/ files.
#[tokio::test]
async fn test_reserved_names_rejected() {
    let temp_dir = tempdir().unwrap();

    for reserved in &["await", "status", "sandboxed_shell", "cancel"] {
        let config = format!(
            r#"{{
  "name": "{}",
  "description": "Should be rejected",
  "command": "echo",
  "enabled": true,
  "subcommand": [{{ "name": "default", "description": "test" }}]
}}"#,
            reserved
        );
        std::fs::write(temp_dir.path().join(format!("{}.json", reserved)), &config).unwrap();

        let cli = Cli::try_parse_from(["ahma_mcp"]).unwrap();
        let result = load_tool_configs(&cli, Some(temp_dir.path())).await;
        assert!(
            result.is_err(),
            "Reserved name '{}' should be rejected",
            reserved
        );

        // Clean up for next iteration
        std::fs::remove_file(temp_dir.path().join(format!("{}.json", reserved))).unwrap();
    }
}

/// Verify that bundled tools load even when NO tools directory exists.
/// This is the exact scenario when a user runs `ahma_mcp --rust --simplify`
/// from a repo that has no `.ahma/` directory and no `--tools-dir` flag.
#[tokio::test]
async fn test_bundled_tools_load_without_tools_dir() {
    // Pass --tool rust --tool simplify but NO --tools-dir, and tools_dir = None
    let cli = Cli::try_parse_from(["ahma_mcp", "--tool", "rust", "--tool", "simplify"]).unwrap();

    // Call with None — this is the code path that was previously broken
    let configs = load_tool_configs(&cli, None).await.unwrap();

    assert!(
        configs.contains_key("cargo"),
        "--tool rust flag should load bundled cargo tool even without tools_dir. Got keys: {:?}",
        configs.keys().collect::<Vec<_>>()
    );
    assert!(
        configs.contains_key("simplify"),
        "--tool simplify flag should load bundled simplify tool even without tools_dir. Got keys: {:?}",
        configs.keys().collect::<Vec<_>>()
    );

    // sandboxed_shell synthetic config should also be present
    assert!(
        configs.contains_key("sandboxed_shell"),
        "sandboxed_shell synthetic config should always be present"
    );
}

/// Verify that each individual bundled flag works without a tools directory.
#[tokio::test]
async fn test_each_bundled_flag_works_without_tools_dir() {
    let tool_and_expected: &[(&str, &str)] = &[
        ("rust", "cargo"),
        ("simplify", "simplify"),
        ("python", "python"),
        ("git", "git"),
        ("github", "gh"),
        ("fileutils", "file-tools"),
    ];

    for &(tool, expected_tool) in tool_and_expected {
        let cli = Cli::try_parse_from(["ahma_mcp", "--tool", tool]).unwrap();
        let configs = load_tool_configs(&cli, None).await.unwrap();
        assert!(
            configs.contains_key(expected_tool),
            "Tool flag '{}' should load bundled tool '{}' even without tools_dir. Got keys: {:?}",
            tool,
            expected_tool,
            configs.keys().collect::<Vec<_>>()
        );
    }
}

/// Verify that `load_tool_configs` never produces duplicate keys.
/// The synthetic `sandboxed_shell` config and RESERVED_TOOL_NAMES must be consistent.
#[tokio::test]
async fn test_no_duplicate_tool_names_in_config_output() {
    let temp_dir = tempdir().unwrap();

    // Create two tool configs in the temp dir
    for (file, name, desc) in &[
        ("tool_a.json", "tool_a", "Tool A"),
        ("tool_b.json", "tool_b", "Tool B"),
    ] {
        let json = format!(
            r#"{{
  "name": "{}",
  "description": "{}",
  "command": "echo",
  "enabled": true,
  "subcommand": [{{ "name": "default", "description": "test" }}]
}}"#,
            name, desc
        );
        std::fs::write(temp_dir.path().join(file), json).unwrap();
    }

    let cli = Cli::try_parse_from(["ahma_mcp"]).unwrap();
    let configs = load_tool_configs(&cli, Some(temp_dir.path()))
        .await
        .unwrap();

    // HashMap keys are inherently unique, but verify that all names match their keys
    for (key, config) in &configs {
        assert_eq!(
            key, &config.name,
            "HashMap key '{}' must match config.name '{}'",
            key, config.name
        );
    }

    // Verify sandboxed_shell synthetic config exists exactly once and is in RESERVED set
    assert!(
        configs.contains_key("sandboxed_shell"),
        "Synthetic sandboxed_shell config should always be present"
    );

    // Count occurrences of each name to confirm no logical duplicates
    let names: Vec<&str> = configs.values().map(|c| c.name.as_str()).collect();
    let mut seen = std::collections::HashSet::new();
    for name in &names {
        assert!(
            seen.insert(name),
            "Duplicate tool name '{}' found in configs",
            name
        );
    }
}

/// Verify that when bundle flags are set and .ahma/ is auto-detected,
/// ALL local .ahma/ tools are loaded (not just those matching the flags).
/// Bundle flags serve as fallbacks for tools not found locally.
#[tokio::test]
async fn test_bundle_flags_with_auto_detected_ahma_loads_all_local_tools() {
    let temp_dir = tempdir().unwrap();

    // Create three local tool definitions: two match flags, one does not
    let tools = [
        ("rust.json", "cargo", "Local cargo tool"),
        ("simplify.json", "simplify", "Local simplify tool"),
        ("git.json", "git", "Local git tool"),
    ];
    for (file, name, desc) in &tools {
        let json = format!(
            r#"{{
  "name": "{}",
  "description": "{}",
  "command": "{}",
  "enabled": true,
  "subcommand": [{{ "name": "default", "description": "test" }}]
}}"#,
            name, desc, name
        );
        std::fs::write(temp_dir.path().join(file), json).unwrap();
    }

    // --tool rust --tool simplify flags, but NOT git. Auto-detected dir (not explicit).
    let mut cli =
        Cli::try_parse_from(["ahma_mcp", "--tool", "rust", "--tool", "simplify"]).unwrap();
    cli.explicit_tools_dir = false;

    let configs = load_tool_configs(&cli, Some(temp_dir.path()))
        .await
        .unwrap();

    // ALL three local tools should be loaded (local .ahma/ always fully loaded)
    assert!(
        configs.contains_key("cargo"),
        "Local cargo should be loaded. Keys: {:?}",
        configs.keys().collect::<Vec<_>>()
    );
    assert!(
        configs.contains_key("simplify"),
        "Local simplify should be loaded. Keys: {:?}",
        configs.keys().collect::<Vec<_>>()
    );
    assert!(
        configs.contains_key("git"),
        "Local git should be loaded even without --git flag. Keys: {:?}",
        configs.keys().collect::<Vec<_>>()
    );

    // Verify local definitions are used (not bundled fallbacks)
    assert_eq!(
        configs["cargo"].description, "Local cargo tool",
        "Local .ahma/ definition should be used, not the bundled version"
    );
    assert_eq!(
        configs["simplify"].description, "Local simplify tool",
        "Local .ahma/ definition should be used, not the bundled version"
    );
}

/// Verify that local .ahma/ definitions override bundled tools AND
/// other non-flagged local tools are also loaded.
#[tokio::test]
async fn test_local_ahma_overrides_bundled_with_all_loaded() {
    let temp_dir = tempdir().unwrap();

    // Create a local rust.json with custom description + a non-flagged tool
    let custom_cargo = r#"{
  "name": "cargo",
  "description": "Overridden cargo from local .ahma/",
  "command": "cargo",
  "enabled": true,
  "subcommand": [{ "name": "build", "description": "Build" }]
}"#;
    std::fs::write(temp_dir.path().join("rust.json"), custom_cargo).unwrap();

    let extra_tool = r#"{
  "name": "my_extra_tool",
  "description": "Extra tool not matching any flag",
  "command": "echo",
  "enabled": true,
  "subcommand": [{ "name": "default", "description": "test" }]
}"#;
    std::fs::write(temp_dir.path().join("extra.json"), extra_tool).unwrap();

    let mut cli = Cli::try_parse_from(["ahma_mcp", "--tool", "rust"]).unwrap();
    cli.explicit_tools_dir = false;

    let configs = load_tool_configs(&cli, Some(temp_dir.path()))
        .await
        .unwrap();

    // Local cargo should override bundled
    assert_eq!(
        configs["cargo"].description, "Overridden cargo from local .ahma/",
        "Local definition should override bundled"
    );

    // Extra non-flagged tool should also be loaded
    assert!(
        configs.contains_key("my_extra_tool"),
        "Non-flagged local tools should also be loaded. Keys: {:?}",
        configs.keys().collect::<Vec<_>>()
    );
}
