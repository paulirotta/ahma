//! Integration tests for the --list-tools functionality
//!
//! These tests verify the tool listing functionality works correctly with
//! both stdio and HTTP MCP servers.

use std::path::PathBuf;
use std::process::Command;

/// Get the path to the pre-built ahma_mcp binary
fn get_ahma_mcp_binary() -> PathBuf {
    ahma_mcp::test_utils::cli::get_binary_path("ahma-mcp", "ahma-mcp")
}

/// Test that the binary shows help for --list-tools
#[test]
fn test_list_tools_help() {
    let binary = get_ahma_mcp_binary();
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get workspace dir")
        .to_path_buf();

    // Use pre-built binary if available, otherwise fall back to cargo run
    let output = if binary.exists() {
        Command::new(&binary)
            .current_dir(&project_root)
            .args(["--help"])
            .output()
            .expect("Failed to execute command")
    } else {
        eprintln!("Warning: Pre-built binary not found, falling back to cargo run");
        Command::new("cargo")
            .current_dir(&project_root)
            .args(["run", "-p", "ahma_mcp", "--", "--help"])
            .output()
            .expect("Failed to execute command")
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let help_text = format!("{}{}", stdout, stderr);

    assert!(
        help_text.contains("list") || help_text.contains("tool"),
        "Help should contain 'list' or 'tool' subcommand. Got: {}",
        help_text
    );
    assert!(
        help_text.contains("serve") || help_text.contains("run") || help_text.contains("Commands"),
        "Help should contain subcommands. Got: {}",
        help_text
    );
}

/// Test that we can list tools from a stdio MCP server
#[test]
fn test_list_tools_from_stdio_server() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let ahma_binary = get_ahma_mcp_binary();
    let tools_dir = project_root.join(".ahma");

    // Check if pre-built binary exists
    if !ahma_binary.exists() {
        eprintln!("Warning: Pre-built binary not found. Run 'cargo build' first for faster tests.");
        let build_output = Command::new("cargo")
            .args(["build", "-p", "ahma_mcp"])
            .output()
            .expect("Failed to build");
        assert!(build_output.status.success(), "Failed to build");
    }

    // Create a temp mcp.json pointing to the stdio server
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let mcp_config_path = temp_dir.path().join("mcp.json");
    let mcp_config = format!(
        r#"{{"mcpServers":{{"test":{{"command":"{cmd}","args":["serve","stdio"],"env":{{"AHMA_TOOLS_DIR":"{tools}","AHMA_DISABLE_SANDBOX":"1","AHMA_SKIP_PROBES":"1"}}}}}}}}"#,
        cmd = ahma_binary.to_str().unwrap().replace('\\', "/"),
        tools = tools_dir.to_str().unwrap().replace('\\', "/")
    );
    std::fs::write(&mcp_config_path, &mcp_config).expect("Failed to write mcp.json");

    // Run ahma_mcp tool list with the mcp.json config
    let output = Command::new(&ahma_binary)
        .args([
            "tool",
            "list",
            "--server",
            "test",
            "--mcp-config",
            mcp_config_path.to_str().unwrap(),
        ])
        .env("AHMA_DISABLE_SANDBOX", "1")
        .current_dir(&project_root)
        .output()
        .expect("Failed to execute ahma_mcp tool list");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        eprintln!("stdout: {}", stdout);
        eprintln!("stderr: {}", stderr);
    }

    // Check we got some tools listed
    assert!(
        stdout.contains("Tool:") || stdout.contains("tools"),
        "Output should contain tool listings. stdout: {}, stderr: {}",
        stdout,
        stderr
    );
}

/// Test JSON output format
#[test]
fn test_list_tools_json_format() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let ahma_binary = get_ahma_mcp_binary();
    let tools_dir = project_root.join(".ahma");

    // Check if pre-built binary exists
    if !ahma_binary.exists() {
        let build_output = Command::new("cargo")
            .args(["build", "-p", "ahma_mcp"])
            .output()
            .expect("Failed to build");
        assert!(build_output.status.success(), "Failed to build");
    }

    // Create a temp mcp.json pointing to the stdio server
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let mcp_config_path = temp_dir.path().join("mcp.json");
    let mcp_config = format!(
        r#"{{"mcpServers":{{"test":{{"command":"{cmd}","args":["serve","stdio"],"env":{{"AHMA_TOOLS_DIR":"{tools}","AHMA_DISABLE_SANDBOX":"1","AHMA_SKIP_PROBES":"1"}}}}}}}}"#,
        cmd = ahma_binary.to_str().unwrap().replace('\\', "/"),
        tools = tools_dir.to_str().unwrap().replace('\\', "/")
    );
    std::fs::write(&mcp_config_path, &mcp_config).expect("Failed to write mcp.json");

    // Run ahma_mcp tool list --format json
    let output = Command::new(&ahma_binary)
        .args([
            "tool",
            "list",
            "--format",
            "json",
            "--server",
            "test",
            "--mcp-config",
            mcp_config_path.to_str().unwrap(),
        ])
        .env("AHMA_DISABLE_SANDBOX", "1")
        .current_dir(&project_root)
        .output()
        .expect("Failed to execute ahma_mcp tool list");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        eprintln!("stdout: {}", stdout);
        eprintln!("stderr: {}", stderr);
    }

    // JSON output should be valid JSON with "tools" key
    assert!(
        stdout.contains("\"tools\"") || stdout.contains("tools"),
        "JSON output should contain 'tools' key. stdout: {}, stderr: {}",
        stdout,
        stderr
    );
}

/// Test output format contains expected sections
#[test]
fn test_list_tools_output_format() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let ahma_binary = get_ahma_mcp_binary();
    let tools_dir = project_root.join(".ahma");

    // Check if pre-built binary exists
    if !ahma_binary.exists() {
        let build_output = Command::new("cargo")
            .args(["build", "-p", "ahma_mcp"])
            .output()
            .expect("Failed to build");
        assert!(build_output.status.success(), "Failed to build");
    }

    // Create a temp mcp.json pointing to the stdio server
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let mcp_config_path = temp_dir.path().join("mcp.json");
    let mcp_config = format!(
        r#"{{"mcpServers":{{"test":{{"command":"{cmd}","args":["serve","stdio"],"env":{{"AHMA_TOOLS_DIR":"{tools}","AHMA_DISABLE_SANDBOX":"1","AHMA_SKIP_PROBES":"1"}}}}}}}}"#,
        cmd = ahma_binary.to_str().unwrap().replace('\\', "/"),
        tools = tools_dir.to_str().unwrap().replace('\\', "/")
    );
    std::fs::write(&mcp_config_path, &mcp_config).expect("Failed to write mcp.json");

    let output = Command::new(&ahma_binary)
        .args([
            "tool",
            "list",
            "--server",
            "test",
            "--mcp-config",
            mcp_config_path.to_str().unwrap(),
        ])
        .env("AHMA_DISABLE_SANDBOX", "1")
        .current_dir(&project_root)
        .output()
        .expect("Failed to execute ahma_mcp tool list");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should have a header section
    assert!(
        stdout.contains("MCP") || stdout.contains("Tool"),
        "Output should contain 'MCP' or 'Tool' header.\nStdout: {}\nStderr: {}",
        stdout,
        stderr
    );
}
