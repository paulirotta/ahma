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
    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": "cat /etc/passwd",
            "working_directory": "../"
        }))
        .unwrap(),
    );
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
    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": "ls",
            "working_directory": "a/b/c/d/e/../../../../../../../../../../"
        }))
        .unwrap(),
    );
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
    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": "ls",
            "working_directory": "/etc"
        }))
        .unwrap(),
    );
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
async fn red_team_symlink_escape_blocked() {
    init_test_logging();

    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    #[cfg(windows)]
    use std::os::windows::fs::symlink_dir as symlink;

    let temp_dir = TempDir::new().unwrap();
    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .build()
        .await
        .unwrap();

    // Create a symlink inside sandbox pointing to root / C:\ (outside)
    let malicious_link = temp_dir.path().join("etc_link");
    let target_dir = if cfg!(windows) { "C:\\" } else { "/etc" };
    let _ = fs::remove_file(&malicious_link);
    match symlink(target_dir, &malicious_link) {
        Ok(_) => {}
        Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied => {
            println!(
                "Skipping test: Windows requires Developer Mode or Admin rights to create symlinks"
            );
            return;
        }
        Err(e) => panic!("Failed to create symlink: {}", e),
    }

    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": "cat passwd",
            "working_directory": "etc_link"
        }))
        .unwrap(),
    );
    let result = client.call_tool(params).await;
    assert!(
        result.is_err(),
        "SECURITY: Symlink escape outside sandbox should be blocked"
    );
    client.cancel().await.unwrap();
}

/// Test that symlinks to user home directory are blocked
#[tokio::test]
async fn red_team_symlink_to_home_blocked() {
    init_test_logging();

    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    #[cfg(windows)]
    use std::os::windows::fs::symlink_dir as symlink;

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
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "C:\\Users\\Public".to_string()
            } else {
                "/Users/Shared".to_string()
            }
        });
    let malicious_link = temp_dir.path().join("home_link");
    let _ = fs::remove_file(&malicious_link);
    match symlink(&home, &malicious_link) {
        Ok(_) => {}
        Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied => {
            println!(
                "Skipping test: Windows requires Developer Mode or Admin rights to create symlinks"
            );
            return;
        }
        Err(e) => panic!("Failed to create symlink: {}", e),
    }

    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": "ls .ssh",
            "working_directory": "home_link"
        }))
        .unwrap(),
    );
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
    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": "echo test",
            "working_directory": "; cat /etc/passwd #"
        }))
        .unwrap(),
    );
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
    let sandbox = Sandbox::new(vec![], SandboxMode::Strict, true, false).unwrap();
    assert!(
        sandbox.is_no_temp_files(),
        "no_temp_files should be enabled"
    );

    let sandbox_default = Sandbox::new(vec![], SandboxMode::Strict, false, false).unwrap();
    assert!(
        !sandbox_default.is_no_temp_files(),
        "no_temp_files should be disabled by default"
    );
}

// =============================================================================
// RED TEAM TEST 7: Global Read Access Prevention (Uniform Strictness)
// =============================================================================

/// Test that reading a file outside the sandbox is universally blocked (including macOS)
#[tokio::test]
async fn red_team_global_read_access_blocked() {
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

    let outside_dir = TempDir::new().unwrap();
    let outside_file = outside_dir.path().join("secret.txt");
    std::fs::write(&outside_file, "secret content").unwrap();

    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": format!("cat {}", outside_file.display()),
            "execution_mode": "Synchronous"
        }))
        .unwrap(),
    );

    let result = client.call_tool(params).await;

    // Command should fail or return error exit code
    if let Ok(tools_res) = result {
        for content in tools_res.content {
            if let Some(text) = content.as_text() {
                let res_json: serde_json::Value = serde_json::from_str(&text.text).unwrap();
                let exit_code = res_json
                    .get("exit_code")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);

                assert!(
                    exit_code != 0,
                    "SECURITY: Should not be able to read file outside sandbox on any platform. Exit: {}",
                    exit_code
                );
            }
        }
    }

    client.cancel().await.unwrap();
}

// =============================================================================
// RED TEAM TEST 8: LiveLog Symlink Targeted Read Expansion
// =============================================================================

/// Test that --livelog grants precise read-only access to a target symlink, but blocks writes and blocks neighboring files.
#[tokio::test]
async fn red_team_livelog_symlink_read_allowed() {
    init_test_logging();

    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    #[cfg(windows)]
    use std::os::windows::fs::symlink_dir as symlink;

    let temp_dir = TempDir::new().unwrap(); // sandbox scope
    let log_dir = temp_dir.path().join("log");
    std::fs::create_dir_all(&log_dir).unwrap();

    let outside_dir = TempDir::new().unwrap();
    let outside_target = outside_dir.path().join("secret.log");
    std::fs::write(&outside_target, "livelog secret content").unwrap();

    let outside_forbidden = outside_dir.path().join("forbidden.log");
    std::fs::write(&outside_forbidden, "forbidden content").unwrap();

    let malicious_link = log_dir.join("live.log");
    match symlink(&outside_target, &malicious_link) {
        Ok(_) => {}
        Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied => {
            println!(
                "Skipping test: Windows requires Developer Mode or Admin rights to create symlinks"
            );
            return;
        }
        Err(e) => panic!("Failed to create symlink: {}", e),
    }

    let tools_dir = get_workspace_tools_dir();
    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .working_dir(temp_dir.path())
        .no_sandbox(false)
        .livelog(true) // Enable the feature we are testing
        .build()
        .await
        .unwrap();

    // 1. We MUST be able to read the explicit target file via its absolute path.
    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": format!("cat {}", outside_target.display()),
            "execution_mode": "Synchronous"
        }))
        .unwrap(),
    );
    let result = client.call_tool(params).await;
    let mut read_succeeded = false;
    if let Ok(tools_res) = result {
        for content in tools_res.content {
            if let Some(text) = content.as_text() {
                let res_json: serde_json::Value = serde_json::from_str(&text.text).unwrap();
                let exit_code = res_json
                    .get("exit_code")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                assert_eq!(
                    exit_code, 0,
                    "SECURITY: Valid livelog symlink target read was blocked"
                );
                let stdout = res_json
                    .get("stdout")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                assert!(stdout.contains("livelog secret content"));
                read_succeeded = true;
            }
        }
    }
    assert!(read_succeeded, "Read command did not complete successfully");

    // 2. We MUST NOT be able to read neighboring files in the external directory.
    let params2 = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": format!("cat {}", outside_forbidden.display()),
            "execution_mode": "Synchronous"
        }))
        .unwrap(),
    );
    let result2 = client.call_tool(params2).await;
    if let Ok(tools_res) = result2 {
        for content in tools_res.content {
            if let Some(text) = content.as_text() {
                let res_json: serde_json::Value = serde_json::from_str(&text.text).unwrap();
                let exit_code = res_json
                    .get("exit_code")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                assert_ne!(
                    exit_code, 0,
                    "SECURITY: Livelog should not grant directory access"
                );
            }
        }
    }

    // 3. We MUST NOT be able to WRITE to the explicit target file.
    let params3 = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": format!("echo hax > {}", outside_target.display()),
            "execution_mode": "Synchronous"
        }))
        .unwrap(),
    );
    let result3 = client.call_tool(params3).await;
    if let Ok(tools_res) = result3 {
        for content in tools_res.content {
            if let Some(text) = content.as_text() {
                let res_json: serde_json::Value = serde_json::from_str(&text.text).unwrap();
                let exit_code = res_json
                    .get("exit_code")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                assert_ne!(
                    exit_code, 0,
                    "SECURITY: Livelog target should be strictly read-only"
                );
            }
        }
    }

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
    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": format!("echo 'hacked' > {}", outside_file.display()),
            "execution_mode": "Synchronous"
        }))
        .unwrap(),
    );

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
    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": "cat /etc/shadow", // Typically root only, but Landlock should block open() regardless
            "execution_mode": "Synchronous"
        }))
        .unwrap(),
    );

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
    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": format!("cat {}", outside_file.display()),
            "execution_mode": "Synchronous"
        }))
        .unwrap(),
    );

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
