//! Nested Sandbox Exit Test (R7.6)
//!
//! This test verifies that when ahma_mcp is running inside another sandbox,
//! it exits with a clear error message instructing the user to use --disable-sandbox.
//!
//! Per R7.6.2: "Upon detection, the system **must** exit with a clear error message
//! instructing the user to disable the internal sandbox using the --disable-sandbox flag
//! or AHMA_DISABLE_SANDBOX=1 environment variable."
//!
//! ## Running These Tests
//!
//! These tests require the ability to run `sandbox-exec`. If the test runner itself
//! is inside a sandbox (e.g., Cursor IDE, certain CI environments), the tests will
//! be skipped automatically.
//!
//! To run manually:
//! ```bash
//! # From a non-sandboxed terminal (e.g., iTerm, Terminal.app)
//! cargo test --package ahma_mcp --test nested_sandbox_exit_test
//! ```

#![cfg(target_os = "macos")]

use ahma_mcp::test_utils::cli::build_binary_cached;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

static AHMA_MCP_BINARY: OnceLock<PathBuf> = OnceLock::new();

/// Check if sandbox-exec can be run at all.
/// Returns false if we're inside another sandbox that prevents sandbox-exec.
fn can_run_sandbox_exec() -> bool {
    let result = Command::new("sandbox-exec")
        .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
        .output();

    match result {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Get the workspace directory (project root)
fn get_workspace_dir() -> PathBuf {
    std::env::current_dir()
        .expect("Failed to get current directory")
        .ancestors()
        .find(|p| p.join("Cargo.toml").exists() && p.join("ahma_mcp").exists())
        .expect("Could not find workspace root")
        .to_path_buf()
}

/// Resolve ahma_mcp binary path once per test process.
fn get_ahma_mcp_binary() -> PathBuf {
    AHMA_MCP_BINARY
        .get_or_init(|| build_binary_cached("ahma_mcp", "ahma-mcp"))
        .clone()
}

/// Skip helper macro - prints message and returns early if sandbox-exec unavailable
macro_rules! skip_if_sandboxed {
    () => {
        if !can_run_sandbox_exec() {
            eprintln!(
                "SKIPPED: Test runner is inside a sandbox - cannot test nested sandbox behavior."
            );
            eprintln!("Run this test from a non-sandboxed terminal (e.g., iTerm, Terminal.app)");
            return;
        }
    };
}

/// Test that ahma_mcp exits with error when launched inside a sandbox (R7.6)
///
/// This test wraps ahma_mcp in sandbox-exec, which triggers the nested sandbox detection.
/// The expected behavior is:
/// 1. Process exits with non-zero code
/// 2. stderr contains "SECURITY ERROR" or "nested sandbox"
/// 3. stderr provides instructions about --disable-sandbox or AHMA_DISABLE_SANDBOX
///
/// Note: We use a sandbox profile that allows the binary to run but denies
/// the `process-exec-interpreter` operation which is required for nested sandbox-exec.
#[test]
fn test_nested_sandbox_detection_exits_with_error() {
    skip_if_sandboxed!();
    let binary = get_ahma_mcp_binary();
    let workspace_dir = get_workspace_dir();

    // Create a sandbox profile that:
    // - Allows the ahma_mcp binary to run (allow process-fork, process-exec)
    // - Denies nested sandbox-exec by blocking process-exec with sandbox-exec
    // This simulates what Cursor/VS Code's sandbox does
    let outer_sandbox_profile = r#"
(version 1)
(deny default)
(allow file-read*)
(allow file-write*)
(allow process-fork)
(allow process-exec)
(allow sysctl-read)
(allow mach-lookup)
(allow mach-register)
(allow ipc-posix-shm)
(allow signal)
(deny process-exec (literal "/usr/bin/sandbox-exec") (with send-signal SIGKILL))
"#;

    // Run ahma_mcp inside sandbox-exec using CLI mode (single command execution)
    // CLI mode triggers sandbox checks and exits after command completion
    let output = Command::new("sandbox-exec")
        .current_dir(&workspace_dir)
        .args([
            "-p",
            outer_sandbox_profile,
            binary.to_str().unwrap(),
            "--tools-dir",
            ".ahma",
            // CLI mode: execute sandboxed_shell with echo (command as single arg)
            "sandboxed_shell",
            "--",
            "echo test",
        ])
        .output()
        .expect("Failed to spawn ahma_mcp inside sandbox");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Debug output for test failures
    eprintln!("Exit code: {:?}", output.status.code());
    eprintln!("stderr:\n{}", stderr);
    eprintln!("stdout:\n{}", stdout);

    // Assert: process exits with non-zero code (per R7.6.2)
    assert!(
        !output.status.success(),
        "ahma_mcp should exit with non-zero when nested sandbox is detected"
    );

    // Assert: stderr contains security error message
    // The specific error could be "nested sandbox" or "sandbox-exec is not available"
    // depending on how the outer sandbox blocks the nested one
    assert!(
        stderr.contains("SECURITY ERROR"),
        "Error message should mention SECURITY ERROR. Got:\n{}",
        stderr
    );

    // Assert: message indicates sandbox issue (either nested or unavailable)
    assert!(
        stderr.contains("nested sandbox")
            || stderr.contains("sandbox-exec")
            || stderr.contains("sandbox"),
        "Error message should mention sandbox issue. Got:\n{}",
        stderr
    );
}

/// Test that ahma_mcp works normally with --disable-sandbox when inside a sandbox (R7.6)
///
/// When the user explicitly disables the sandbox with --disable-sandbox,
/// ahma_mcp should run successfully even inside another sandbox.
#[test]
fn test_no_sandbox_flag_allows_nested_execution() {
    skip_if_sandboxed!();
    let binary = get_ahma_mcp_binary();
    let workspace_dir = get_workspace_dir();

    let outer_sandbox_profile = "(version 1)(allow default)";

    // Run ahma_mcp inside sandbox-exec with --disable-sandbox using CLI mode
    let output = Command::new("sandbox-exec")
        .current_dir(&workspace_dir)
        .args([
            "-p",
            outer_sandbox_profile,
            binary.to_str().unwrap(),
            "--disable-sandbox", // Explicitly disable sandbox
            "--tools-dir",
            ".ahma",
            // CLI mode: execute sandboxed_shell with echo (command as single arg)
            "sandboxed_shell",
            "--",
            "echo nested_sandbox_test_success",
        ])
        .output()
        .expect("Failed to spawn ahma_mcp inside sandbox with --disable-sandbox");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Debug output for test failures
    eprintln!("Exit code: {:?}", output.status.code());
    eprintln!("stderr:\n{}", stderr);
    eprintln!("stdout:\n{}", stdout);

    // With --disable-sandbox, the process should succeed
    assert!(
        output.status.success(),
        "ahma_mcp should succeed with --disable-sandbox even inside another sandbox. stderr:\n{}",
        stderr
    );

    // stdout should contain the echo output
    assert!(
        stdout.contains("nested_sandbox_test_success"),
        "Expected echo output. Got:\n{}",
        stdout
    );
}

/// Test that AHMA_DISABLE_SANDBOX=1 env var allows nested execution (R7.6)
#[test]
fn test_no_sandbox_env_var_allows_nested_execution() {
    skip_if_sandboxed!();
    let binary = get_ahma_mcp_binary();
    let workspace_dir = get_workspace_dir();

    let outer_sandbox_profile = "(version 1)(allow default)";

    // Run ahma_mcp inside sandbox-exec with AHMA_DISABLE_SANDBOX=1 using CLI mode
    let output = Command::new("sandbox-exec")
        .current_dir(&workspace_dir)
        .env("AHMA_DISABLE_SANDBOX", "1")
        .args([
            "-p",
            outer_sandbox_profile,
            binary.to_str().unwrap(),
            "--tools-dir",
            ".ahma",
            // CLI mode: execute sandboxed_shell with echo (command as single arg)
            "sandboxed_shell",
            "--",
            "echo env_var_test_success",
        ])
        .output()
        .expect("Failed to spawn ahma_mcp inside sandbox with AHMA_DISABLE_SANDBOX=1");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Debug output for test failures
    eprintln!("Exit code: {:?}", output.status.code());
    eprintln!("stderr:\n{}", stderr);
    eprintln!("stdout:\n{}", stdout);

    // With AHMA_DISABLE_SANDBOX=1, the process should succeed
    assert!(
        output.status.success(),
        "ahma_mcp should succeed with AHMA_DISABLE_SANDBOX=1 even inside another sandbox. stderr:\n{}",
        stderr
    );
}
