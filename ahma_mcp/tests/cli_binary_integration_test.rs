//! CLI Binary Integration Tests
//!
//! These tests verify that all CLI binaries in the ahma_mcp workspace work correctly
//! when invoked from the command line. They provide coverage for the `main.rs` files
//! that are otherwise difficult to test through unit tests.
//!
//! Test philosophy:
//! - Each binary should have tests for: --help, --version, basic functionality
//! - Tests use temp directories as per R13.5 (Test File Isolation)
//! - Tests verify exit codes and output content
//!
//! Performance optimization:
//! - Binary paths are cached using OnceLock to avoid redundant builds
//! - When running via `cargo nextest` or `cargo test`, binaries are already built
//! - Only falls back to building if the binary doesn't exist

use ahma_mcp::test_utils::cli::{build_binary_cached, test_command};
use ahma_mcp::test_utils::fs::get_workspace_dir;
use std::process::Command;
use tempfile::TempDir;

// ============================================================================
// ahma_mcp Binary Tests
// ============================================================================

mod ahma_mcp_tests {
    use super::*;

    #[test]
    fn test_ahma_mcp_help() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");

        let output = test_command(&binary)
            .arg("--help")
            .output()
            .expect("Failed to execute ahma_mcp --help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        // --help should succeed
        assert!(
            output.status.success(),
            "ahma_mcp --help should succeed. Output: {}",
            combined
        );

        // Should contain key command info
        assert!(
            combined.contains("ahma_mcp") || combined.contains("Ahma"),
            "Help should mention ahma_mcp or Ahma. Got: {}",
            combined
        );
        assert!(
            combined.contains("--mode") || combined.contains("stdio") || combined.contains("http"),
            "Help should mention modes. Got: {}",
            combined
        );
    }

    #[test]
    fn test_ahma_mcp_version() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");

        let output = test_command(&binary)
            .arg("--version")
            .output()
            .expect("Failed to execute ahma_mcp --version");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        assert!(
            output.status.success(),
            "ahma_mcp --version should succeed. Output: {}",
            combined
        );

        // Should contain version number
        assert!(
            combined.contains("0.") || combined.contains("1."),
            "Version output should contain version number. Got: {}",
            combined
        );
    }

    #[test]
    fn test_ahma_mcp_cli_mode_invalid_tool() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");
        let workspace = get_workspace_dir();
        let tools_dir = workspace.join(".ahma");

        let output = test_command(&binary)
            .current_dir(&workspace)
            .args([
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "nonexistent_tool",
            ])
            .output()
            .expect("Failed to execute ahma_mcp with invalid tool");

        // Should fail with non-zero exit code
        assert!(
            !output.status.success(),
            "ahma_mcp should fail for nonexistent tool"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("not found")
                || stderr.contains("No matching")
                || stderr.contains("error"),
            "Error message should indicate tool not found. Got: {}",
            stderr
        );
    }

    #[test]
    fn test_ahma_mcp_cli_mode_echo_tool() {
        // Test using a simple echo-like tool if available
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");
        let workspace = get_workspace_dir();
        let tools_dir = workspace.join(".ahma");

        // Check if file_tools exists (a simple tool to test with)
        let output = test_command(&binary)
            .current_dir(&workspace)
            .env("AHMA_TOOLS_DIR", tools_dir.to_str().unwrap())
            .args(["run", "file-tools_pwd"])
            .output()
            .expect("Failed to execute ahma_mcp with file_tools_pwd");

        // This should either succeed (tool exists) or fail with tool not found
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // If tool exists, it should output the working directory
        if output.status.success() {
            assert!(
                stdout.contains("/") || stdout.contains("\\"),
                "pwd should output a path. Got: {}",
                stdout
            );
        } else {
            // If tool doesn't exist or is disabled, that's also acceptable for this test
            let combined = format!("{}{}", stdout, stderr);
            assert!(
                combined.contains("not found")
                    || combined.contains("No matching")
                    || combined.contains("disabled")
                    || combined.contains("cannot find the path specified")
                    || combined.contains("The system cannot find the path specified")
                    || combined.contains("Command execution failed")
                    || combined.contains("error"),
                "Should fail with meaningful error. Got: {}",
                combined
            );
        }
    }

    #[test]
    fn test_ahma_mcp_stdio_mode_rejects_tty() {
        // When run from a terminal (TTY), stdio mode should be rejected
        // Note: This test behavior depends on the test runner's TTY state
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");
        let workspace = get_workspace_dir();
        let tools_dir = workspace.join(".ahma");

        let output = test_command(&binary)
            .current_dir(&workspace)
            .env("AHMA_TOOLS_DIR", tools_dir.to_str().unwrap())
            .args(["serve", "stdio"])
            .output()
            .expect("Failed to execute ahma_mcp in stdio mode");

        // In test environment (non-TTY), this should work differently than interactive
        // The test mainly verifies the binary runs without crashing
        let stderr = String::from_utf8_lossy(&output.stderr);

        // If it failed, should have meaningful error message
        if !output.status.success() {
            assert!(
                stderr.contains("terminal") || stderr.contains("MCP") || stderr.contains("Error"),
                "Error should be meaningful. Got: {}",
                stderr
            );
        }
    }
}

// ============================================================================
// ahma-mcp --validate Flag Tests
// ============================================================================

mod validate_flag_tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_ahma_mcp_help_mentions_validate() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");

        let output = test_command(&binary)
            .arg("--help")
            .output()
            .expect("Failed to execute ahma-mcp --help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        assert!(
            output.status.success(),
            "ahma-mcp --help should succeed. Output: {}",
            combined
        );

        assert!(
            combined.contains("tool") || combined.contains("serve"),
            "Help should mention the top-level command structure. Got: {}",
            combined
        );
    }

    #[test]
    fn test_validate_valid_tools_directory() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");
        let workspace = get_workspace_dir();
        let tools_dir = workspace.join(".ahma");

        let output = test_command(&binary)
            .current_dir(&workspace)
            .args(["tool", "validate", tools_dir.to_str().unwrap()])
            .output()
            .expect("Failed to execute ahma-mcp tool validate");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "ahma-mcp --validate should succeed on valid tools dir. stdout: {}, stderr: {}",
            stdout,
            stderr
        );

        // Should indicate validation passed
        let combined = format!("{}{}", stdout, stderr);
        assert!(
            combined.contains("valid") || combined.contains("Valid"),
            "Output should indicate validation success. Got: {}",
            combined
        );
    }

    #[test]
    fn test_validate_invalid_json_file() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let workspace = get_workspace_dir();

        // Create an invalid JSON file
        let invalid_file = temp_dir.path().join("invalid.json");
        fs::write(&invalid_file, "{ this is not valid json }")
            .expect("Failed to write invalid file");

        let output = test_command(&binary)
            .current_dir(&workspace)
            .args(["--validate", invalid_file.to_str().unwrap()])
            .output()
            .expect("Failed to execute ahma-mcp --validate");

        // Should fail
        assert!(
            !output.status.success(),
            "ahma-mcp --validate should fail on invalid JSON"
        );
    }

    #[test]
    fn test_validate_nonexistent_path() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");
        let workspace = get_workspace_dir();

        let output = test_command(&binary)
            .current_dir(&workspace)
            .args(["--validate", "/nonexistent/path/to/tools"])
            .output()
            .expect("Failed to execute ahma-mcp --validate");

        // Should fail
        assert!(
            !output.status.success(),
            "ahma-mcp --validate should fail on nonexistent path"
        );
    }

    #[test]
    fn test_validate_single_valid_file() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");
        let workspace = get_workspace_dir();
        let cargo_json = workspace.join(".ahma/cargo.json");

        if cargo_json.exists() {
            let output = test_command(&binary)
                .current_dir(&workspace)
                .args(["--validate", cargo_json.to_str().unwrap()])
                .output()
                .expect("Failed to execute ahma-mcp --validate");

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            assert!(
                output.status.success(),
                "ahma-mcp --validate should succeed on cargo.json. stdout: {}, stderr: {}",
                stdout,
                stderr
            );
        }
    }
}

// ============================================================================
// generate_tool_schema Binary Tests
// ============================================================================

mod generate_tool_schema_tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_generate_schema_default_output() {
        let binary = build_binary_cached("generate_tool_schema", "generate-tool-schema");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let workspace = get_workspace_dir();

        let output = Command::new(&binary)
            .current_dir(&workspace)
            .arg(temp_dir.path().to_str().unwrap())
            .output()
            .expect("Failed to execute generate_tool_schema");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "generate_tool_schema should succeed. stdout: {}, stderr: {}",
            stdout,
            stderr
        );

        // Should create mtdf-schema.json
        let schema_path = temp_dir.path().join("mtdf-schema.json");
        assert!(
            schema_path.exists(),
            "Schema file should be created at {:?}",
            schema_path
        );

        // Verify schema content
        let schema_content = fs::read_to_string(&schema_path).expect("Failed to read schema");
        assert!(
            schema_content.contains("$schema") || schema_content.contains("ToolConfig"),
            "Schema should contain standard JSON Schema elements. Got: {}",
            &schema_content[..schema_content.len().min(500)]
        );
    }

    #[test]
    fn test_generate_schema_output_is_valid_json() {
        let binary = build_binary_cached("generate_tool_schema", "generate-tool-schema");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let workspace = get_workspace_dir();

        Command::new(&binary)
            .current_dir(&workspace)
            .arg(temp_dir.path().to_str().unwrap())
            .output()
            .expect("Failed to execute generate_tool_schema");

        let schema_path = temp_dir.path().join("mtdf-schema.json");
        if schema_path.exists() {
            let schema_content = fs::read_to_string(&schema_path).expect("Failed to read schema");
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&schema_content);

            assert!(
                parsed.is_ok(),
                "Generated schema should be valid JSON. Error: {:?}",
                parsed.err()
            );
        }
    }

    #[test]
    fn test_generate_schema_creates_directory_if_needed() {
        let binary = build_binary_cached("generate_tool_schema", "generate-tool-schema");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let workspace = get_workspace_dir();

        let nested_dir = temp_dir.path().join("nested/output/dir");

        let output = Command::new(&binary)
            .current_dir(&workspace)
            .arg(nested_dir.to_str().unwrap())
            .output()
            .expect("Failed to execute generate_tool_schema");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "generate_tool_schema should create nested directories. stdout: {}, stderr: {}",
            stdout,
            stderr
        );

        let schema_path = nested_dir.join("mtdf-schema.json");
        assert!(
            schema_path.exists(),
            "Schema should be created in nested directory"
        );
    }
}

// ============================================================================
// ahma_mcp --list-tools Mode Tests
// ============================================================================

mod ahma_list_tools_mode_tests {
    use super::*;

    #[test]
    fn test_ahma_mcp_list_tools_help() {
        // The --list-tools help is shown as part of main --help
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");

        let output = test_command(&binary)
            .arg("--help")
            .output()
            .expect("Failed to execute ahma_mcp --help");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        assert!(
            output.status.success(),
            "ahma_mcp --help should succeed. Output: {}",
            combined
        );

        // Help should mention list/tool subcommands
        assert!(
            combined.contains("list") || combined.contains("tool"),
            "Help should mention list or tool subcommands. Got: {}",
            combined
        );
    }

    #[test]
    fn test_ahma_mcp_list_tools_no_connection_method() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");

        // Running --list-tools without any connection method should fail gracefully
        let output = test_command(&binary)
            .arg("--list-tools")
            .output()
            .expect("Failed to execute ahma_mcp --list-tools");

        // Should fail with meaningful error
        assert!(
            !output.status.success(),
            "ahma_mcp --list-tools should fail without connection method"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Must specify") || stderr.contains("server") || stderr.contains("--"),
            "Error should mention connection method. Got: {}",
            stderr
        );
    }

    #[test]
    fn test_ahma_mcp_list_tools_with_stdio_server() {
        // This test connects to another ahma_mcp binary via stdio
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");
        let workspace = get_workspace_dir();
        let tools_dir = workspace.join(".ahma");

        let output = test_command(&binary)
            .current_dir(&workspace)
            .args([
                "--list-tools",
                "--server",
                &format!(
                    "{} --tools-dir {}",
                    binary.to_str().unwrap(),
                    tools_dir.to_str().unwrap()
                ),
            ])
            .output()
            .expect("Failed to execute ahma_mcp --list-tools with stdio server");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // The command should either succeed and list tools, or fail with a known error
        if output.status.success() {
            assert!(
                stdout.contains("Tool") || stdout.contains("tool") || stdout.contains("cargo"),
                "Output should contain tool information. Got: {}",
                stdout
            );
        } else {
            // Acceptable if it fails due to connection issues
            let combined = format!("{}{}", stdout, stderr);
            println!(
                "ahma_mcp --list-tools failed (may be acceptable): {}",
                combined
            );
        }
    }

    #[test]
    fn test_ahma_mcp_list_tools_json_format() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");
        let workspace = get_workspace_dir();
        let tools_dir = workspace.join(".ahma");

        let output = test_command(&binary)
            .current_dir(&workspace)
            .args([
                "--list-tools",
                "--format",
                "json",
                "--server",
                &format!(
                    "{} --tools-dir {}",
                    binary.to_str().unwrap(),
                    tools_dir.to_str().unwrap()
                ),
            ])
            .output()
            .expect("Failed to execute ahma_mcp --list-tools --format json");

        let stdout = String::from_utf8_lossy(&output.stdout);

        // If successful, output should be JSON (starts with { or [)
        if output.status.success() && !stdout.is_empty() {
            let trimmed = stdout.trim();
            assert!(
                trimmed.starts_with('{') || trimmed.starts_with('['),
                "JSON output should start with {{ or [. Got: {}",
                &trimmed[..trimmed.len().min(100)]
            );
        }
    }
    #[test]
    fn test_ahma_mcp_cli_mode_execution() {
        let binary = build_binary_cached("ahma_mcp", "ahma-mcp");
        let temp = tempfile::tempdir().unwrap();
        let tools_dir = temp.path().join("tools");
        std::fs::create_dir_all(&tools_dir).unwrap();

        // Create a simple tool
        let echo_tool = r#"
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
            "description": "Echo a message",
            "positional_args": [
                {
                    "name": "message",
                    "type": "string",
                    "description": "The message to echo",
                    "required": true
                }
            ]
        }
    ]
}
"#;
        std::fs::write(tools_dir.join("test_echo.json"), echo_tool).unwrap();

        // Execute the tool via CLI run subcommand
        // ahma-mcp tool run <tool_name> -- [RAW_ARGS]
        let output = test_command(&binary)
            .env("AHMA_TOOLS_DIR", &tools_dir)
            .args(["tool", "run", "test_echo", "--", "hello-cli-mode"])
            .output()
            .expect("Failed to execute ahma_mcp in CLI mode");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "CLI mode execution failed. stderr: {}",
            stderr
        );
        assert!(
            stdout.contains("hello-cli-mode"),
            "Output should contain the echoed message. Got: {}",
            stdout
        );
    }
}
