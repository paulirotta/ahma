/// Test to verify that the guard rail system correctly detects hardcoded tool conflicts
use ahma_mcp::config::load_tool_configs_sync;
use ahma_mcp::utils::logging::init_test_logging;
use tempfile::TempDir;

#[test]
fn test_guard_rail_detects_hardcoded_tool_conflicts() {
    init_test_logging();
    println!("🧪 Testing guard rail system for hardcoded tool conflicts...");

    // Create a temporary directory with a conflicting tool configuration
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let tools_dir = temp_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools directory");

    // Create a conflicting "await.json" file
    let await_config = r#"{
  "name": "await",
  "description": "This should conflict with hardcoded await tool",
  "command": "echo",
  "enabled": true,
  "subcommand": [
    {
      "name": "default",
      "description": "Test command",
      "force_synchronous": true,
      "options": [
        {
          "name": "test",
          "type": "string",
          "description": "Test parameter"
        }
      ]
    }
  ]
}"#;

    std::fs::write(tools_dir.join("await.json"), await_config).expect("Failed to write await.json");

    // Also create a valid tool that shouldn't conflict
    let valid_config = r#"{
  "name": "valid_tool",
  "description": "A valid tool that doesn't conflict",
  "command": "echo",
  "enabled": true,
  "subcommand": [
    {
      "name": "default",
      "description": "Test command",
      "force_synchronous": true,
      "options": [
        {
          "name": "command",
          "type": "string",
          "description": "Command to execute"
        }
      ]
    }
  ]
}"#;

    std::fs::write(tools_dir.join("valid_tool.json"), valid_config)
        .expect("Failed to write valid_tool.json");

    // Try to load tool configurations - this should fail due to guard rail
    let result = load_tool_configs_sync(
        &ahma_mcp::shell::cli::AppConfig::default(),
        Some(&tools_dir),
    );

    match result {
        Err(e) => {
            let error_message = e.to_string();
            assert!(
                error_message.contains("await"),
                "Error should mention the conflicting 'await' tool, got: {}",
                error_message
            );
            assert!(
                error_message.contains("system tool") || error_message.contains("conflict"),
                "Error should mention system tool or conflict, got: {}",
                error_message
            );
            println!(
                "OK Guard rail correctly detected conflict: {}",
                error_message
            );
        }
        Ok(_) => {
            panic!("Expected guard rail to detect conflict and return error, but got Ok(_)");
        }
    }

    println!("OK Guard rail system test passed!");
}

#[test]
fn test_guard_rail_allows_valid_configurations() {
    init_test_logging();
    println!("🧪 Testing guard rail allows valid configurations...");

    // Create a temporary directory with only valid tool configurations
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let tools_dir = temp_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).expect("Failed to create tools directory");

    // Create valid tool (git) that doesn't conflict with hardcoded ones. ls omitted (optional).
    let git_config = r#"{
  "name": "git",
  "description": "Git operations",
  "command": "git",
  "enabled": true,
  "subcommand": [
    {
      "name": "default",
      "description": "Git operations",
      "force_synchronous": true,
      "options": [
        {
          "name": "subcommand",
          "type": "string",
          "description": "Git subcommand to execute"
        }
      ]
    }
  ]
}"#;

    std::fs::write(tools_dir.join("git.json"), git_config).expect("Failed to write git.json");

    // Try to load tool configurations - this should succeed
    let result = load_tool_configs_sync(
        &ahma_mcp::shell::cli::AppConfig::default(),
        Some(&tools_dir),
    );

    match result {
        Ok(configs) => {
            // ls tool optional; do not assert its presence
            assert!(configs.contains_key("git"), "Should load git tool");
            assert!(
                !configs.contains_key("await"),
                "Should not contain hardcoded await tool"
            );
            assert!(
                !configs.contains_key("status"),
                "Should not contain hardcoded status tool"
            );
            assert!(
                !configs.contains_key("cancel"),
                "Should not contain hardcoded cancel tool"
            );
            println!("OK Guard rail correctly allowed valid configurations");
        }
        Err(e) => {
            panic!(
                "Expected guard rail to allow valid configurations, but got error: {}",
                e
            );
        }
    }

    println!("OK Guard rail validation test passed!");
}
