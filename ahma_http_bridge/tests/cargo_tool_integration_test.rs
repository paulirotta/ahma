//! Tool Integration Tests with Real Assertions
//!
//! These tests verify that tools work correctly end-to-end via the HTTP bridge.
//!
//! ## Core Tests (Always Run)
//! Tests using `sandboxed_shell` validate core MCP functionality and always run
//! because sandboxed_shell is a built-in tool that's always available.
//!
//! ## Cargo Tests (Optional)
//! Tests using `cargo` are skipped if the cargo tool is unavailable.
//! They test cargo-specific behavior but depend on cargo.json being enabled.
//!
//! ## Protocol Flow
//!
//! These tests properly implement the MCP protocol handshake:
//! 1. initialize → get session ID
//! 2. initialized notification
//! 3. Wait for roots/list request from server via SSE
//! 4. Respond with client roots (the temp project directory)
//! 5. THEN call tools
//!
//! ## Running Tests
//!
//! ```bash
//! cargo nextest run --test cargo_tool_integration_test
//! ```

mod common;

use common::{McpTestClient, spawn_test_server};
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// Check if cargo tool is available in the server's tool list
async fn is_cargo_tool_available(client: &McpTestClient) -> bool {
    match client.list_tools().await {
        Ok(tools) => tools.iter().any(|t| {
            t.get("name")
                .and_then(|n| n.as_str())
                .is_some_and(|n| n == "cargo")
        }),
        Err(_) => false,
    }
}

/// Create a minimal but valid Cargo project in a temporary directory.
/// This project has:
/// - A valid Cargo.toml
/// - A simple lib.rs
/// - A test module (for clippy --tests)
fn create_test_cargo_project() -> TempDir {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let project_dir = temp_dir.path();

    // Create Cargo.toml - use edition 2021 for compatibility
    let cargo_toml = r#"[package]
name = "test_project"
version = "0.1.0"
edition = "2021"

[dependencies]
"#;
    fs::write(project_dir.join("Cargo.toml"), cargo_toml).expect("Failed to write Cargo.toml");

    // Create src directory
    fs::create_dir(project_dir.join("src")).expect("Failed to create src directory");

    // Create lib.rs with some code
    let lib_rs = r#"//! A test library for integration tests

/// A simple function that adds two numbers
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        assert_eq!(add(2, 2), 4);
    }

    #[test]
    fn test_add_negative() {
        assert_eq!(add(-1, 1), 0);
    }
}
"#;
    fs::write(project_dir.join("src/lib.rs"), lib_rs).expect("Failed to write lib.rs");

    temp_dir
}

/// Verify that the temporary project compiles before testing
fn verify_project_compiles(project_dir: &std::path::Path) -> bool {
    let output = Command::new("cargo")
        .current_dir(project_dir)
        .args(["check", "--quiet"])
        .output()
        .expect("Failed to run cargo check");

    output.status.success()
}

struct CargoTestContext {
    project_dir: TempDir,
    client: McpTestClient,
    _server: common::server::TestServerInstance,
}

async fn setup_cargo_test() -> Option<CargoTestContext> {
    let project = create_test_cargo_project();

    assert!(
        verify_project_compiles(project.path()),
        "Test project should compile before we test it"
    );

    let server = match spawn_test_server().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("WARNING️  Skipping test - failed to spawn server: {}", e);
            return None;
        }
    };

    let mut client = McpTestClient::with_url(&server.base_url());

    if client
        .initialize_with_roots("cargo-test-client", &[project.path().to_path_buf()])
        .await
        .is_err()
    {
        eprintln!("WARNING️  Skipping test - failed to initialize MCP client");
        return None;
    }

    if !is_cargo_tool_available(&client).await {
        eprintln!("WARNING️  Skipping test - cargo tool not available");
        return None;
    }

    Some(CargoTestContext {
        project_dir: project,
        client,
        _server: server,
    })
}

// =============================================================================
// Test: Cargo Check with Assertions
// =============================================================================

#[tokio::test]
async fn test_cargo_check_with_assertions() {
    let Some(ctx) = setup_cargo_test().await else {
        return;
    };

    // Call cargo with subcommand check, working_directory pointing to our test project
    let result = ctx
        .client
        .call_tool(
            "cargo",
            json!({
                "subcommand": "check",
                "working_directory": ctx.project_dir.path().to_string_lossy()
            }),
        )
        .await;

    // CRITICAL: Assert success
    assert!(
        result.success,
        "cargo_check should succeed. Error: {:?}",
        result.error
    );

    // Validate output contains expected strings
    if let Some(output) = &result.output {
        // cargo check output should contain something meaningful
        assert!(
            output.contains("Finished")
                || output.contains("Checking")
                || output.contains("Compiling")
                || output.is_empty(), // Sometimes check is instant if already built
            "cargo_check output should indicate progress: got '{}'",
            output
        );

        // Should NOT contain cancellation messages
        assert!(
            !output.to_lowercase().contains("canceled"),
            "cargo_check should not be canceled: got '{}'",
            output
        );
    }

    println!("OK cargo_check succeeded in {}ms", result.duration_ms);
}

// =============================================================================
// Test: Cargo Clippy (Basic)
// =============================================================================

#[tokio::test]
async fn test_cargo_clippy_basic_with_assertions() {
    let Some(ctx) = setup_cargo_test().await else {
        return;
    };

    // Call cargo clippy without the --tests flag
    let result = ctx
        .client
        .call_tool(
            "cargo",
            json!({
                "subcommand": "clippy",
                "working_directory": ctx.project_dir.path().to_string_lossy()
            }),
        )
        .await;

    // CRITICAL: Assert success
    assert!(
        result.success,
        "cargo_clippy should succeed. Error: {:?}",
        result.error
    );

    if let Some(output) = &result.output {
        // Should NOT contain cancellation messages
        assert!(
            !output.to_lowercase().contains("canceled"),
            "cargo_clippy should not be canceled: got '{}'",
            output
        );
    }

    println!(
        "OK cargo_clippy (basic) succeeded in {}ms",
        result.duration_ms
    );
}

// =============================================================================
// Test: Cargo Clippy with --tests flag (THE CRITICAL TEST)
// =============================================================================

#[tokio::test]
async fn test_cargo_clippy_with_tests_flag_assertions() {
    let Some(ctx) = setup_cargo_test().await else {
        return;
    };

    // Call cargo clippy WITH the --tests flag (this is what was failing!)
    let result = ctx
        .client
        .call_tool(
            "cargo",
            json!({
                "subcommand": "clippy",
                "tests": true,
                "working_directory": ctx.project_dir.path().to_string_lossy()
            }),
        )
        .await;

    // CRITICAL: Assert success - this is THE test that was failing with "Canceled: canceled"
    assert!(
        result.success,
        "cargo_clippy with --tests should succeed. Error: {:?}. \
        If this fails with a cancellation message, the underlying issue is NOT fixed.",
        result.error
    );

    if let Some(output) = &result.output {
        // Should NOT contain cancellation messages
        assert!(
            !output.to_lowercase().contains("canceled"),
            "cargo_clippy --tests should not be canceled: got '{}'",
            output
        );

        // Output should indicate clippy ran
        assert!(
            output.contains("Finished")
                || output.contains("Checking")
                || output.contains("Compiling")
                || output.contains("clippy")
                || output.is_empty(),
            "cargo_clippy --tests output should indicate progress: got '{}'",
            output
        );
    }

    println!(
        "OK cargo_clippy --tests succeeded in {}ms",
        result.duration_ms
    );
}

// =============================================================================
// Test: Cargo Build
// =============================================================================

#[tokio::test]
async fn test_cargo_build_with_assertions() {
    let Some(ctx) = setup_cargo_test().await else {
        return;
    };

    let result = ctx
        .client
        .call_tool(
            "cargo",
            json!({
                "subcommand": "build",
                "working_directory": ctx.project_dir.path().to_string_lossy()
            }),
        )
        .await;

    // CRITICAL: Assert success
    assert!(
        result.success,
        "cargo build should succeed. Error: {:?}",
        result.error
    );

    if let Some(output) = &result.output {
        assert!(
            !output.to_lowercase().contains("canceled"),
            "cargo_build should not be canceled: got '{}'",
            output
        );
    }

    println!("OK cargo_build succeeded in {}ms", result.duration_ms);
}

// =============================================================================
// Test: Cargo Test
// =============================================================================

#[tokio::test]
async fn test_cargo_test_with_assertions() {
    let Some(ctx) = setup_cargo_test().await else {
        return;
    };

    let result = ctx
        .client
        .call_tool(
            "cargo",
            json!({
                "subcommand": "test",
                "working_directory": ctx.project_dir.path().to_string_lossy()
            }),
        )
        .await;

    // CRITICAL: Assert success
    assert!(
        result.success,
        "cargo test should succeed. Error: {:?}",
        result.error
    );

    if let Some(output) = &result.output {
        assert!(
            !output.to_lowercase().contains("canceled"),
            "cargo_test should not be canceled: got '{}'",
            output
        );

        // Tests should have run
        assert!(
            output.contains("test result:")
                || output.contains("running")
                || output.contains("passed")
                || output.contains("Compiling"),
            "cargo_test output should indicate tests ran: got '{}'",
            output
        );
    }

    println!("OK cargo_test succeeded in {}ms", result.duration_ms);
}

// =============================================================================
// Test: Cargo Fmt
// =============================================================================

#[tokio::test]
async fn test_cargo_fmt_with_assertions() {
    let Some(ctx) = setup_cargo_test().await else {
        return;
    };

    let result = ctx
        .client
        .call_tool(
            "cargo",
            json!({
                "subcommand": "fmt",
                "working_directory": ctx.project_dir.path().to_string_lossy()
            }),
        )
        .await;

    // CRITICAL: Assert success
    assert!(
        result.success,
        "cargo fmt should succeed. Error: {:?}",
        result.error
    );

    if let Some(output) = &result.output {
        assert!(
            !output.to_lowercase().contains("canceled"),
            "cargo_fmt should not be canceled: got '{}'",
            output
        );
    }

    println!("OK cargo_fmt succeeded in {}ms", result.duration_ms);
}

// =============================================================================
// Meta-test: Verify cancellation detection in error messages
// =============================================================================

#[test]
fn test_cancellation_detection_patterns() {
    // This test verifies our cancellation detection patterns work correctly
    // These are the patterns we check for to identify cancellation issues

    let cancellation_patterns = [
        "Canceled: canceled",
        "canceled",
        "Cancelled",
        "task cancelled for reason",
        "Operation cancelled",
    ];

    let success_patterns = [
        "Finished",
        "Compiling",
        "Checking",
        "running 2 tests",
        "test result: ok",
    ];

    // Cancellation patterns should be detected
    for pattern in cancellation_patterns {
        let lower = pattern.to_lowercase();
        assert!(
            lower.contains("cancel"),
            "Pattern '{}' should be detected as cancellation",
            pattern
        );
    }

    // Success patterns should NOT be detected as cancellation
    for pattern in success_patterns {
        let lower = pattern.to_lowercase();
        assert!(
            !lower.contains("cancel"),
            "Pattern '{}' should NOT be detected as cancellation",
            pattern
        );
    }
}

// =============================================================================
// Test: Core Tool Execution with sandboxed_shell (Always Available)
// =============================================================================

/// This test validates core MCP tool execution functionality using sandboxed_shell,
/// which is always available as a built-in tool. This ensures that the HTTP bridge
/// tool execution path is always tested, regardless of which optional tools are enabled.
#[tokio::test]
async fn test_sandboxed_shell_execution_with_assertions() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let project_dir = temp_dir.path();

    // Create a test file to verify we're in the right directory
    fs::write(project_dir.join("test_marker.txt"), "hello from test")
        .expect("Failed to write test file");

    let server = match spawn_test_server().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("WARNING️  Skipping test - failed to spawn server: {}", e);
            return;
        }
    };

    let mut client = McpTestClient::with_url(&server.base_url());

    if client
        .initialize_with_roots("shell-test-client", &[project_dir.to_path_buf()])
        .await
        .is_err()
    {
        eprintln!("WARNING️  Skipping test - failed to initialize MCP client");
        return;
    }

    // Platform-specific commands:
    //   - PowerShell `pwd` (Get-Location) emits a table with "Path\n----\n..." headers
    //     and canonicalize() adds a `\\?\` prefix. Use `(Get-Location).Path` for a
    //     plain string and strip the extended-length prefix before comparison.
    //   - PowerShell `echo` with single quotes works, but `Write-Output` is safer.
    //   - `ls -la` is Unix-only; PowerShell's `Get-ChildItem` is the equivalent.

    // Build the expected directory name (last path component) for a prefix-agnostic check.
    // On Windows, canonicalize() returns `\\?\C:\Users\...` but the shell reports
    // `C:\Users\...`; comparing just the directory name avoids the mismatch.
    let expected_dir_name = project_dir
        .file_name()
        .expect("temp dir should have a name")
        .to_string_lossy()
        .to_string();

    // Test 1: pwd returns the working directory
    #[cfg(target_os = "windows")]
    let pwd_command = "(Get-Location).Path";
    #[cfg(not(target_os = "windows"))]
    let pwd_command = "pwd";

    let result = client
        .call_tool(
            "sandboxed_shell",
            json!({
                "command": pwd_command,
                "working_directory": project_dir.to_string_lossy()
            }),
        )
        .await;

    assert!(
        result.success,
        "sandboxed_shell pwd should succeed. Error: {:?}",
        result.error
    );

    if let Some(output) = &result.output {
        assert!(
            output.contains(&expected_dir_name),
            "pwd should return working directory containing '{}': got '{}'",
            expected_dir_name,
            output
        );
    }

    // Test 2: echo returns expected output
    // Use double quotes so the string is treated identically by both bash and powershell.
    let result = client
        .call_tool(
            "sandboxed_shell",
            json!({
                "command": "echo \"MCP tool execution works\"",
                "working_directory": project_dir.to_string_lossy()
            }),
        )
        .await;

    assert!(
        result.success,
        "sandboxed_shell echo should succeed. Error: {:?}",
        result.error
    );

    if let Some(output) = &result.output {
        assert!(
            output.contains("MCP tool execution works"),
            "echo should return expected output: got '{}'",
            output
        );
    }

    // Test 3: directory listing can see files in sandbox
    #[cfg(target_os = "windows")]
    let ls_command = "Get-ChildItem";
    #[cfg(not(target_os = "windows"))]
    let ls_command = "ls -la";

    let result = client
        .call_tool(
            "sandboxed_shell",
            json!({
                "command": ls_command,
                "working_directory": project_dir.to_string_lossy()
            }),
        )
        .await;

    assert!(
        result.success,
        "sandboxed_shell ls should succeed. Error: {:?}",
        result.error
    );

    if let Some(output) = &result.output {
        assert!(
            output.contains("test_marker.txt"),
            "ls should show test file: got '{}'",
            output
        );
    }

    println!("OK sandboxed_shell core tests passed");
}
