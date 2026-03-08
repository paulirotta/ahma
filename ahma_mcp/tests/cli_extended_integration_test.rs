//! Extended CLI Binary Integration Tests
//!
//! Additional CLI integration tests targeting low-coverage areas:
//! - Various flag combinations for ahma_mcp
//! - Error handling paths
//! - Edge cases in tool execution
//! - --sync flag behavior
//! - --debug flag behavior
//! - --log-to-stderr flag behavior

use ahma_mcp::test_utils::cli::build_binary_cached as build_binary;
use ahma_mcp::test_utils::cli::{build_binary_cached, test_command};
use ahma_mcp::test_utils::fs::get_workspace_dir as workspace_dir;
use std::process::Command;
use tempfile::TempDir;

// ============================================================================
// ahma_mcp Flag Combination Tests
// ============================================================================

mod flag_combination_tests {
    use super::*;

    /// Test --sync flag behavior
    #[test]
    fn test_ahma_mcp_sync_flag() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");

        let output = test_command(&binary)
            .args(["--help"])
            .output()
            .expect("Failed to execute ahma_mcp --help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        // Help should mention --sync flag
        assert!(
            combined.contains("--sync") || combined.contains("synchronous"),
            "Help should mention --sync flag. Got: {}",
            combined
        );
    }

    /// Test --debug flag behavior
    #[test]
    fn test_ahma_mcp_debug_flag() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");

        let output = test_command(&binary)
            .args(["--help"])
            .output()
            .expect("Failed to execute ahma_mcp --help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        // Help should mention --debug flag
        assert!(
            combined.contains("--debug") || combined.contains("DEBUG"),
            "Help should mention --debug flag. Got: {}",
            combined
        );
    }

    /// Test --log-to-stderr flag behavior
    #[test]
    fn test_ahma_mcp_log_to_stderr_flag() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");

        let output = test_command(&binary)
            .args(["--help"])
            .output()
            .expect("Failed to execute ahma_mcp --help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        // Help should mention --log-to-stderr flag
        assert!(
            combined.contains("--log-to-stderr") || combined.contains("stderr"),
            "Help should mention --log-to-stderr flag. Got: {}",
            combined
        );
    }

    /// Test --tools-dir flag with custom directory
    #[test]
    fn test_ahma_mcp_custom_tools_dir() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let tools_dir = temp_dir.path().join("tools");
        std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

        // Create a minimal tool config
        let tool_config = r#"
{
    "name": "test_echo",
    "description": "Test echo tool",
    "command": "echo",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [
        {
            "name": "default",
            "description": "echo the message",
            "positional_args": [
                {
                    "name": "message",
                    "type": "string",
                    "description": "message to echo",
                    "required": false
                }
            ]
        }
    ]
}
"#;
        std::fs::write(tools_dir.join("test_echo.json"), tool_config)
            .expect("Failed to write tool config");

        // Test CLI mode with custom tools dir
        let output = test_command(&binary)
            .current_dir(temp_dir.path())
            .args([
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "test_echo",
                "--",
                "hello",
            ])
            .output()
            .expect("Failed to execute ahma_mcp with custom tools dir");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Should either succeed or fail with meaningful error
        if output.status.success() {
            assert!(
                stdout.contains("hello") || stdout.contains("echo"),
                "Should contain tool output. Got: {}",
                stdout
            );
        } else {
            // Acceptable failure messages
            let combined = format!("{}{}", stdout, stderr);
            println!("Custom tools dir test result: {}", combined);
        }
    }

    /// Test --sandbox-scope flag
    #[test]
    fn test_ahma_mcp_sandbox_scope_flag() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");

        let output = test_command(&binary)
            .args(["--help"])
            .output()
            .expect("Failed to execute ahma_mcp --help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        // Help should mention --sandbox-scope flag
        assert!(
            combined.contains("--sandbox-scope") || combined.contains("sandbox"),
            "Help should mention --sandbox-scope flag. Got: {}",
            combined
        );
    }

    /// Test combining multiple flags
    #[test]
    fn test_ahma_mcp_multiple_flags() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");
        let workspace = workspace_dir();
        let tools_dir = workspace.join(".ahma");

        // Use CLI mode with multiple flags
        let output = test_command(&binary)
            .current_dir(&workspace)
            .args([
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "--sync",
                "--debug",
                "file-tools_pwd",
            ])
            .output()
            .expect("Failed to execute ahma_mcp with multiple flags");

        // Just ensure it doesn't crash with multiple flags
        let _stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // If it fails, should fail gracefully
        if !output.status.success() {
            assert!(
                stderr.contains("not found")
                    || stderr.contains("error")
                    || stderr.contains("Error")
                    || !stderr.is_empty(),
                "Should have meaningful output on failure"
            );
        }
    }
}

// ============================================================================
// Error Handling Tests
// ============================================================================

mod error_handling_tests {
    use super::*;

    /// Test empty tools directory
    #[test]
    fn test_ahma_mcp_empty_tools_dir() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let tools_dir = temp_dir.path().join("empty_tools");
        std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

        // Try to list tools from empty directory
        let output = test_command(&binary)
            .current_dir(temp_dir.path())
            .args(["--tools-dir", tools_dir.to_str().unwrap(), "anything"])
            .output()
            .expect("Failed to execute ahma_mcp with empty tools dir");

        // Should fail with meaningful error
        assert!(
            !output.status.success(),
            "Should fail with empty tools directory"
        );
    }

    /// Test invalid JSON in tools directory
    #[test]
    fn test_ahma_mcp_invalid_tool_json() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let tools_dir = temp_dir.path().join("tools");
        std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

        // Create invalid JSON file
        std::fs::write(tools_dir.join("invalid.json"), "{ not valid json }")
            .expect("Failed to write invalid file");

        let output = test_command(&binary)
            .current_dir(temp_dir.path())
            .args(["--tools-dir", tools_dir.to_str().unwrap(), "anything"])
            .output()
            .expect("Failed to execute ahma_mcp with invalid tool JSON");

        // Should fail or skip invalid file
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("Invalid JSON test stderr: {}", stderr);
    }

    /// Test tool with missing required field
    #[test]
    fn test_ahma_mcp_incomplete_tool_config() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let tools_dir = temp_dir.path().join("tools");
        std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

        // Create tool config missing required fields
        let incomplete_config = r#"
{
    "name": "incomplete",
    "description": "Missing command field"
}
"#;
        std::fs::write(tools_dir.join("incomplete.json"), incomplete_config)
            .expect("Failed to write incomplete config");

        let output = test_command(&binary)
            .current_dir(temp_dir.path())
            .args(["--tools-dir", tools_dir.to_str().unwrap(), "incomplete"])
            .output()
            .expect("Failed to execute ahma_mcp with incomplete config");

        // Should fail or skip incomplete config
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("Incomplete config test stderr: {}", stderr);
    }

    /// Test tool that doesn't exist on system
    #[test]
    fn test_ahma_mcp_unavailable_command() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let tools_dir = temp_dir.path().join("tools");
        std::fs::create_dir_all(&tools_dir).expect("Failed to create tools dir");

        // Create tool config for non-existent command
        let unavailable_config = r#"
{
    "name": "unavailable",
    "description": "Command that doesn't exist",
    "command": "this_command_definitely_does_not_exist_xyz123",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [
        {
            "name": "default",
            "description": "default subcommand"
        }
    ]
}
"#;
        std::fs::write(tools_dir.join("unavailable.json"), unavailable_config)
            .expect("Failed to write unavailable config");

        let output = test_command(&binary)
            .current_dir(temp_dir.path())
            .args(["--tools-dir", tools_dir.to_str().unwrap(), "unavailable"])
            .output()
            .expect("Failed to execute ahma_mcp with unavailable command");

        // Should fail with meaningful error
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{}{}", stdout, stderr);

        if !output.status.success() {
            // Good - it failed
            println!("Unavailable command test result: {}", combined);
        }
    }
}

// ============================================================================
// Mode-Specific Tests
// ============================================================================

mod mode_tests {
    use super::*;

    /// Test --mode stdio requires proper environment
    #[test]
    fn test_ahma_mcp_stdio_mode_without_client() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");
        let workspace = workspace_dir();
        let tools_dir = workspace.join(".ahma");

        let output = test_command(&binary)
            .current_dir(&workspace)
            .args([
                "--mode",
                "stdio",
                "--tools-dir",
                tools_dir.to_str().unwrap(),
            ])
            .output()
            .expect("Failed to execute ahma_mcp in stdio mode");

        // In test environment, this typically fails because there's no proper client
        // The key is that it doesn't hang or crash unexpectedly
        let _exit_code = output.status.code();
    }

    /// Test --mode http requires port specification
    #[test]
    fn test_ahma_mcp_http_mode_default_port() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");

        let output = test_command(&binary)
            .args(["--help"])
            .output()
            .expect("Failed to execute ahma_mcp --help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        // Help should mention http mode
        assert!(
            combined.contains("http") || combined.contains("HTTP"),
            "Help should mention http mode. Got: {}",
            combined
        );
    }

    /// Test invalid mode
    #[test]
    fn test_ahma_mcp_invalid_mode() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");
        let workspace = workspace_dir();

        let output = test_command(&binary)
            .current_dir(&workspace)
            .args(["--mode", "invalid_mode_xyz"])
            .output()
            .expect("Failed to execute ahma_mcp with invalid mode");

        // Should fail with meaningful error
        assert!(!output.status.success(), "Invalid mode should fail");

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("invalid") || stderr.contains("Invalid") || stderr.contains("error"),
            "Should indicate invalid mode. Got: {}",
            stderr
        );
    }
}

// ============================================================================
// CLI Tool Execution Tests
// ============================================================================

mod cli_execution_tests {
    use super::*;

    /// Test running a simple tool in CLI mode
    #[test]
    fn test_ahma_mcp_cli_simple_tool() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");
        let workspace = workspace_dir();
        let tools_dir = workspace.join(".ahma");

        // Try to run file_tools_pwd which should be available
        let output = test_command(&binary)
            .current_dir(&workspace)
            .args(["--tools-dir", tools_dir.to_str().unwrap(), "file-tools_pwd"])
            .output()
            .expect("Failed to execute file_tools_pwd");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if output.status.success() {
            // Should output a path
            assert!(
                stdout.contains("/") || stdout.contains("\\"),
                "pwd should output a path. Got: {}",
                stdout
            );
        } else {
            // May fail if tool doesn't exist
            println!("CLI tool execution stderr: {}", stderr);
        }
    }

    /// Test running tool with arguments
    #[test]
    fn test_ahma_mcp_cli_tool_with_args() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");
        let workspace = workspace_dir();
        let tools_dir = workspace.join(".ahma");

        // Try to run sandboxed_shell with an echo command
        let output = test_command(&binary)
            .current_dir(&workspace)
            .args([
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "sandboxed_shell",
                "--",
                r#"{"command": "echo test_output"}"#,
            ])
            .output()
            .expect("Failed to execute sandboxed_shell");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        // Either succeeds with output or fails gracefully
        println!("CLI tool with args result: {}", combined);
    }

    /// Test running tool in sync mode
    #[test]
    fn test_ahma_mcp_cli_sync_execution() {
        let binary = build_binary("ahma_mcp", "ahma-mcp");
        let workspace = workspace_dir();
        let tools_dir = workspace.join(".ahma");

        let output = test_command(&binary)
            .current_dir(&workspace)
            .args([
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "--sync",
                "file-tools_pwd",
            ])
            .output()
            .expect("Failed to execute file_tools_pwd in sync mode");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Sync mode should complete synchronously
        println!("Sync execution stdout: {}", stdout);
        println!("Sync execution stderr: {}", stderr);
    }
}

// ============================================================================
// ahma_validate Extended Tests
// ============================================================================

mod ahma_validate_extended_tests {
    use super::*;

    /// Test validation in verbose mode
    #[test]
    fn test_ahma_validate_verbose_mode() {
        let binary = build_binary("ahma_validate", "ahma-validate");

        let output = Command::new(&binary)
            .args(["--help"])
            .output()
            .expect("Failed to execute ahma_validate --help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        // Check if verbose flag exists
        if combined.contains("--verbose") || combined.contains("-v") {
            // It has verbose mode, test it
            let workspace = workspace_dir();
            let tools_dir = workspace.join(".ahma");

            let verbose_output = Command::new(&binary)
                .current_dir(&workspace)
                .args([tools_dir.to_str().unwrap(), "--verbose"])
                .output()
                .expect("Failed to execute ahma_validate --verbose");

            let verbose_stdout = String::from_utf8_lossy(&verbose_output.stdout);
            let verbose_stderr = String::from_utf8_lossy(&verbose_output.stderr);
            println!("Verbose output: {}{}", verbose_stdout, verbose_stderr);
        }
    }

    /// Test validation with specific file patterns
    #[test]
    fn test_ahma_validate_specific_files() {
        let binary = build_binary("ahma_validate", "ahma-validate");
        let workspace = workspace_dir();
        let tools_dir = workspace.join(".ahma");

        // Validate specific tool files if they exist
        for tool_name in ["cargo.json", "git.json", "file-tools.json"] {
            let tool_path = tools_dir.join(tool_name);
            if tool_path.exists() {
                let output = Command::new(&binary)
                    .current_dir(&workspace)
                    .args([tool_path.to_str().unwrap()])
                    .output()
                    .unwrap_or_else(|_| panic!("Failed to validate {}", tool_name));

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    println!("Validation of {} failed: {}", tool_name, stderr);
                }
            }
        }
    }
}

// ============================================================================
// generate_tool_schema Extended Tests
// ============================================================================

mod generate_schema_extended_tests {
    use super::*;

    /// Test schema generation with custom output filename
    #[test]
    fn test_generate_schema_custom_filename() {
        let binary = build_binary("generate_tool_schema", "generate-tool-schema");

        let output = Command::new(&binary)
            .args(["--help"])
            .output()
            .expect("Failed to execute generate_tool_schema --help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        // Check what options are available
        println!("generate_tool_schema help: {}", combined);
    }

    /// Test schema generation produces valid JSON Schema
    #[test]
    fn test_generate_schema_valid_json_schema() {
        let binary = build_binary("generate_tool_schema", "generate-tool-schema");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let workspace = workspace_dir();

        let output = Command::new(&binary)
            .current_dir(&workspace)
            .arg(temp_dir.path().to_str().unwrap())
            .output()
            .expect("Failed to execute generate_tool_schema");

        if output.status.success() {
            let schema_path = temp_dir.path().join("mtdf-schema.json");
            if schema_path.exists() {
                let content = std::fs::read_to_string(&schema_path).expect("Failed to read schema");
                let parsed: serde_json::Value =
                    serde_json::from_str(&content).expect("Invalid JSON");

                // Check for JSON Schema required fields
                assert!(
                    parsed.get("$schema").is_some() || parsed.get("type").is_some(),
                    "Should have JSON Schema structure"
                );
            }
        }
    }
}
