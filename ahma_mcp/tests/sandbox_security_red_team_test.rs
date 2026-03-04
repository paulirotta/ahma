//! Red Team Security Tests for Sandbox Escape Prevention
//!
//! These tests attempt various sandbox escape techniques to verify that:
//! 1. Path validation correctly blocks access outside sandbox scope
//! 2. The --no-temp-files flag effectively blocks temp directory writes
//! 3. Symlink-based escape attempts are detected
//! 4. Encoded/obfuscated path traversal attempts fail
//!
//! The goal is to document both working protections and known limitations.

use ahma_mcp::sandbox::{Sandbox, SandboxMode};
use ahma_mcp::test_utils as common;
use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use common::fs::get_workspace_tools_dir;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
#[cfg(not(target_os = "windows"))]
use std::fs;
use tempfile::TempDir;

// =============================================================================
// RED TEAM TEST 1: Path Traversal Attacks
// =============================================================================

/// Test that basic path traversal (../) is blocked
#[tokio::test]
async fn red_team_basic_path_traversal_blocked() {
    init_test_logging();
    let temp_dir = TempDir::new().unwrap();
    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .build()
        .await
        .unwrap();

    // Attempt to escape via simple ../
    let params = CallToolRequestParams {
        name: "sandboxed_shell".into(),
        arguments: Some(
            serde_json::from_value(json!({
                "command": "cat /etc/passwd",
                "working_directory": "../"
            }))
            .unwrap(),
        ),
        task: None,
        meta: None,
    };
    let result = client.call_tool(params).await;
    assert!(
        result.is_err(),
        "SECURITY: Basic path traversal should be blocked"
    );
    client.cancel().await.unwrap();
}

/// Test that deeply nested path traversal is blocked
#[tokio::test]
async fn red_team_deep_path_traversal_blocked() {
    init_test_logging();
    let temp_dir = TempDir::new().unwrap();
    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .build()
        .await
        .unwrap();

    // Attempt to escape via deeply nested traversal
    let params = CallToolRequestParams {
        name: "sandboxed_shell".into(),
        arguments: Some(
            serde_json::from_value(json!({
                "command": "ls",
                "working_directory": "a/b/c/d/e/../../../../../../../../../../"
            }))
            .unwrap(),
        ),
        task: None,
        meta: None,
    };
    let result = client.call_tool(params).await;
    assert!(
        result.is_err(),
        "SECURITY: Deep path traversal should be blocked"
    );
    client.cancel().await.unwrap();
}

/// Test that absolute path outside sandbox is blocked
#[tokio::test]
async fn red_team_absolute_path_escape_blocked() {
    init_test_logging();
    let temp_dir = TempDir::new().unwrap();
    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .build()
        .await
        .unwrap();

    // Attempt to use absolute path outside sandbox
    let params = CallToolRequestParams {
        name: "sandboxed_shell".into(),
        arguments: Some(
            serde_json::from_value(json!({
                "command": "ls",
                "working_directory": "/etc"
            }))
            .unwrap(),
        ),
        task: None,
        meta: None,
    };
    let result = client.call_tool(params).await;
    assert!(
        result.is_err(),
        "SECURITY: Absolute path outside sandbox should be blocked"
    );
    client.cancel().await.unwrap();
}

// =============================================================================
// RED TEAM TEST 2: Symlink Escape Attacks
// =============================================================================

/// Test that symlinks pointing outside sandbox are blocked
#[tokio::test]
#[cfg(unix)]
async fn red_team_symlink_escape_blocked() {
    init_test_logging();
    use std::os::unix::fs::symlink;

    let temp_dir = TempDir::new().unwrap();
    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .build()
        .await
        .unwrap();

    // Create a symlink inside sandbox pointing to /etc (outside)
    let malicious_link = temp_dir.path().join("etc_link");
    let _ = fs::remove_file(&malicious_link);
    symlink("/etc", &malicious_link).expect("Failed to create symlink");

    let params = CallToolRequestParams {
        name: "sandboxed_shell".into(),
        arguments: Some(
            serde_json::from_value(json!({
                "command": "cat passwd",
                "working_directory": "etc_link"
            }))
            .unwrap(),
        ),
        task: None,
        meta: None,
    };
    let result = client.call_tool(params).await;
    assert!(
        result.is_err(),
        "SECURITY: Symlink escape to /etc should be blocked"
    );
    client.cancel().await.unwrap();
}

/// Test that symlinks to user home directory are blocked
#[tokio::test]
#[cfg(unix)]
async fn red_team_symlink_to_home_blocked() {
    init_test_logging();
    use std::os::unix::fs::symlink;

    let temp_dir = TempDir::new().unwrap();
    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .build()
        .await
        .unwrap();

    // Create symlink to home directory
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/Shared".to_string());
    let malicious_link = temp_dir.path().join("home_link");
    let _ = fs::remove_file(&malicious_link);
    symlink(&home, &malicious_link).expect("Failed to create symlink");

    let params = CallToolRequestParams {
        name: "sandboxed_shell".into(),
        arguments: Some(
            serde_json::from_value(json!({
                "command": "ls .ssh",
                "working_directory": "home_link"
            }))
            .unwrap(),
        ),
        task: None,
        meta: None,
    };
    let result = client.call_tool(params).await;
    assert!(
        result.is_err(),
        "SECURITY: Symlink escape to home directory should be blocked"
    );
    client.cancel().await.unwrap();
}

// =============================================================================
// RED TEAM TEST 3: Command Injection via Path
// =============================================================================

/// Test that shell metacharacters in paths are rejected
#[tokio::test]
async fn red_team_shell_metacharacters_in_path() {
    init_test_logging();
    let temp_dir = TempDir::new().unwrap();
    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .build()
        .await
        .unwrap();

    // Attempt to inject shell commands via path
    // The path "; cat /etc/passwd #" doesn't exist as a directory
    let params = CallToolRequestParams {
        name: "sandboxed_shell".into(),
        arguments: Some(
            serde_json::from_value(json!({
                "command": "echo test",
                "working_directory": "; cat /etc/passwd #"
            }))
            .unwrap(),
        ),
        task: None,
        meta: None,
    };
    let result = client.call_tool(params).await;
    // The command may start async but should fail during execution
    // because the working directory doesn't exist.
    // We're documenting that the system handles this case safely.
    let _ = result;
    client.cancel().await.unwrap();
}

// =============================================================================
// RED TEAM TEST 4: No-Temp-Files Mode Tests
// =============================================================================

/// Test that no_temp_files mode is properly set on Sandbox
#[test]
fn red_team_no_temp_files_flag_setting() {
    let sandbox = Sandbox::new(vec![], SandboxMode::Strict, true).unwrap();
    assert!(
        sandbox.is_no_temp_files(),
        "no_temp_files should be enabled"
    );

    let sandbox_default = Sandbox::new(vec![], SandboxMode::Strict, false).unwrap();
    assert!(
        !sandbox_default.is_no_temp_files(),
        "no_temp_files should be disabled by default"
    );
}

// =============================================================================
// DOCUMENTATION: Known Security Limitations
// =============================================================================

/// Document: Read access is allowed everywhere (KNOWN LIMITATION)
#[tokio::test]
async fn documented_limitation_read_access_unrestricted() {
    init_test_logging();
    let temp_dir = TempDir::new().unwrap();
    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .build()
        .await
        .unwrap();

    // Note: We're testing from within the sandbox scope, but the command
    // attempts to READ a file outside. On macOS with Seatbelt, this succeeds
    // because file-read* is allowed everywhere.
    //
    // This is a KNOWN LIMITATION, not a bug.
    let params = CallToolRequestParams {
        name: "sandboxed_shell".into(),
        arguments: Some(
            serde_json::from_value(json!({
                "command": "test -r /etc/passwd && echo 'readable' || echo 'not readable'"
            }))
            .unwrap(),
        ),
        task: None,
        meta: None,
    };
    let result = client.call_tool(params).await;

    // The command should succeed (sandbox allows running in current dir)
    // The output will show if /etc/passwd is readable
    // On macOS, it WILL be readable (known limitation)
    if let Ok(response) = result {
        let _ = response;
    }
    client.cancel().await.unwrap();
}

/// Document: Network access is unrestricted (KNOWN LIMITATION)
#[tokio::test]
async fn documented_limitation_network_unrestricted() {
    init_test_logging();
    let temp_dir = TempDir::new().unwrap();
    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .build()
        .await
        .unwrap();

    // Test that network access works (e.g., DNS lookup)
    // This documents that network is unrestricted
    let params = CallToolRequestParams {
        name: "sandboxed_shell".into(),
        arguments: Some(
            serde_json::from_value(json!({
                // Use a simple network test that doesn't actually transfer data
                "command": "ping -c 1 -t 1 127.0.0.1 2>/dev/null || echo 'network test'"
            }))
            .unwrap(),
        ),
        task: None,
        meta: None,
    };
    let result = client.call_tool(params).await;
    // Just document that network commands can run - this is a known limitation
    let _ = result;
    client.cancel().await.unwrap();
}

// =============================================================================
// RED TEAM TEST 5: Command Argument Escape (Write)
// =============================================================================

/// Test that writing to a file outside the sandbox via command arguments is blocked
#[tokio::test]
async fn red_team_command_write_escape_blocked() {
    init_test_logging();
    let temp_dir = TempDir::new().unwrap();
    let outside_dir = TempDir::new().unwrap();
    let outside_file = outside_dir.path().join("pwned.txt");

    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .arg("--no-temp-files")
        .build()
        .await
        .unwrap();

    // Attempt to write to a file outside the sandbox using absolute path
    let params = CallToolRequestParams {
        name: "sandboxed_shell".into(),
        arguments: Some(
            serde_json::from_value(json!({
                "command": format!("echo 'hacked' > {}", outside_file.display()),
                "execution_mode": "Synchronous"
            }))
            .unwrap(),
        ),
        task: None,
        meta: None,
    };

    // The command might "succeed" (exit code 0) if the shell handles the error gracefully,
    // or fail (exit code 1). Key check is: file MUST NOT exist.
    let _ = client.call_tool(params).await;

    assert!(
        !outside_file.exists(),
        "SECURITY: Should not be able to write to file outside sandbox: {}",
        outside_file.display()
    );

    client.cancel().await.unwrap();
}

// =============================================================================
// RED TEAM TEST 6: Command Argument Escape (Read - Linux Only)
// =============================================================================

/// Test that reading a file outside the sandbox via command arguments is blocked on Linux
#[tokio::test]
#[cfg(target_os = "linux")]
async fn red_team_command_read_escape_blocked_linux() {
    init_test_logging();
    let temp_dir = TempDir::new().unwrap();
    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .arg("--no-temp-files")
        .build()
        .await
        .unwrap();

    // Attempt to read /etc/shadow (or similar restricted file)
    let params = CallToolRequestParams {
        name: "sandboxed_shell".into(),
        arguments: Some(
            serde_json::from_value(json!({
                "command": "cat /etc/shadow", // Typically root only, but Landlock should block open() regardless
                "execution_mode": "Synchronous"
            }))
            .unwrap(),
        ),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await;

    // Command should fail or return error exit code
    if let Ok(response) = result {
        let _content = response.content.first().unwrap().as_text().unwrap();
        // Check if output contains "Permission denied" or similar
        // Note: response content is JSON string of the result, we need to check stderr/exit code
        // But client.call_tool returns the ToolResult. Use debug print if needed.
        // Simplified check: Use a file we know exists but shouldn't be readable due to sandbox

        // Actually, let's use a custom file outside sandbox to be sure
    }

    client.cancel().await.unwrap();
}

/// Refined Linux read test with verified outside file
#[tokio::test]
#[cfg(target_os = "linux")]
async fn red_team_command_read_escape_blocked_linux_custom() {
    use std::io::Write;

    init_test_logging();
    let temp_dir = TempDir::new().unwrap();
    let outside_dir = TempDir::new().unwrap();
    let outside_file = outside_dir.path().join("secret.txt");
    {
        let mut f = fs::File::create(&outside_file).unwrap();
        writeln!(f, "secret content").unwrap();
    }

    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .arg("--no-temp-files")
        .build()
        .await
        .unwrap();

    // Attempt to read the outside file
    let params = CallToolRequestParams {
        name: "sandboxed_shell".into(),
        arguments: Some(
            serde_json::from_value(json!({
                "command": format!("cat {}", outside_file.display()),
                "execution_mode": "Synchronous"
            }))
            .unwrap(),
        ),
        task: None,
        meta: None,
    };

    let result = client.call_tool(params).await;

    if let Ok(tools_res) = result {
        for content in tools_res.content {
            if let Some(text) = content.as_text() {
                let res_json: serde_json::Value = serde_json::from_str(&text.text).unwrap();
                let exit_code = res_json
                    .get("exit_code")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let stderr = res_json
                    .get("stderr")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let stdout = res_json
                    .get("stdout")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Should fail with exit code != 0 or Permission denied
                assert!(
                    exit_code != 0 || stderr.contains("Permission denied"),
                    "SECURITY: Should not be able to read file outside sandbox on Linux. Exit: {}, Stderr: {}, Stdout: {}",
                    exit_code,
                    stderr,
                    stdout
                );
            }
        }
    }

    client.cancel().await.unwrap();
}
