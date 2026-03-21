//! Tests for sandbox/command.rs create_command and create_shell_command.
//!
//! The integration test file exercises these via the public Sandbox API
//! from external test perspective, complementing the inline unit tests
//! already added to command.rs.

use ahma_mcp::sandbox::{Sandbox, SandboxMode};
use tempfile::tempdir;

// ── Sandbox::create_command via public API ────────────────────────────────────

/// Test mode: create_command for a simple program returns a valid Command.
#[test]
fn test_sandbox_create_command_test_mode_echo() {
    let sandbox = Sandbox::new_test();
    let td = tempdir().unwrap();
    let result = sandbox.create_command("echo", &["world".to_string()], td.path());
    assert!(
        result.is_ok(),
        "Test-mode create_command should succeed: {result:?}"
    );
}

/// Test mode: create_command with zero args succeeds.
#[test]
fn test_sandbox_create_command_no_args() {
    let sandbox = Sandbox::new_test();
    let td = tempdir().unwrap();
    let result = sandbox.create_command("true", &[], td.path());
    assert!(result.is_ok());
}

/// Test mode: create_command with "cargo" sets CARGO_TARGET_DIR.
/// We can't directly inspect the env on a tokio::process::Command, but
/// the call should at minimum not panic and return Ok.
#[test]
fn test_sandbox_create_command_cargo_program() {
    let sandbox = Sandbox::new_test();
    let td = tempdir().unwrap();
    let result = sandbox.create_command("cargo", &["check".to_string()], td.path());
    assert!(result.is_ok(), "Cargo command should succeed in test mode");
}

/// Test mode: create_command with a path-like cargo program still triggers env.
#[test]
fn test_sandbox_create_command_path_to_cargo() {
    let sandbox = Sandbox::new_test();
    let td = tempdir().unwrap();
    // A full path like /usr/local/bin/cargo has file_name() == "cargo"
    let result = sandbox.create_command("/usr/local/bin/cargo", &[], td.path());
    assert!(result.is_ok());
}

// ── Sandbox::create_shell_command via public API ──────────────────────────────

/// Test mode: create_shell_command returns a valid Command.
#[test]
fn test_sandbox_create_shell_command_test_mode() {
    let sandbox = Sandbox::new_test();
    let td = tempdir().unwrap();

    #[cfg(not(target_os = "windows"))]
    let shell = "sh";
    #[cfg(target_os = "windows")]
    let shell = "powershell";

    let result = sandbox.create_shell_command(shell, "echo hello", td.path());
    assert!(
        result.is_ok(),
        "create_shell_command in test mode should succeed"
    );
}

/// Strict mode: create_shell_command should succeed on all supported platforms.
#[test]
fn test_sandbox_create_shell_command_strict_mode() {
    let td = tempdir().unwrap();
    let sandbox = Sandbox::new(
        vec![td.path().to_path_buf()],
        SandboxMode::Strict,
        false,
        false,
        false,
    )
    .unwrap();

    #[cfg(not(target_os = "windows"))]
    let shell = "sh";
    #[cfg(target_os = "windows")]
    let shell = "powershell";

    let result = sandbox.create_shell_command(shell, "echo hello", td.path());
    assert!(
        result.is_ok(),
        "create_shell_command in strict mode should succeed: {result:?}"
    );
}

// ── Strict mode create_command ─────────────────────────────────────────────────

/// Strict mode create_command for a basic program should succeed.
#[test]
fn test_sandbox_create_command_strict_mode_basic() {
    let td = tempdir().unwrap();
    let sandbox = Sandbox::new(
        vec![td.path().to_path_buf()],
        SandboxMode::Strict,
        false,
        false,
        false,
    )
    .unwrap();

    let result = sandbox.create_command("echo", &["hi".to_string()], td.path());
    assert!(
        result.is_ok(),
        "Strict mode create_command should succeed: {result:?}"
    );
}

/// Verify that create_command can actually be spawned in test mode.
/// This exercises the fully constructed Command end-to-end.
#[tokio::test]
async fn test_sandbox_create_command_spawns_successfully() {
    let sandbox = Sandbox::new_test();
    let td = tempdir().unwrap();

    #[cfg(not(target_os = "windows"))]
    let (program, args) = ("echo", vec!["spawn_test".to_string()]);
    #[cfg(target_os = "windows")]
    let (program, args) = ("cmd", vec!["/C".to_string(), "echo spawn_test".to_string()]);

    let mut cmd = sandbox.create_command(program, &args, td.path()).unwrap();
    let output = cmd.output().await.unwrap();
    assert!(output.status.success(), "Spawned command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("spawn_test"),
        "Command output should contain 'spawn_test': {stdout}"
    );
}
