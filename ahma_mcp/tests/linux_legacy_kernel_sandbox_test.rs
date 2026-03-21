#![cfg(target_os = "linux")]

use ahma_mcp::sandbox::SandboxError;
use ahma_mcp::test_utils::cli::build_binary_cached;
use ahma_mcp::test_utils::fs::get_workspace_dir as workspace_dir;
use std::process::Command;

fn landlock_unavailable() -> bool {
    matches!(
        ahma_mcp::sandbox::check_sandbox_prerequisites(),
        Err(SandboxError::LandlockNotAvailable) | Err(SandboxError::PrerequisiteFailed(_))
    )
}

fn build_binary() -> std::path::PathBuf {
    build_binary_cached("ahma_mcp", "ahma-mcp")
}

#[test]
fn test_no_sandbox_warns_and_runs_on_legacy_kernel() {
    if !landlock_unavailable() {
        eprintln!("SKIPPED: Landlock is available on this kernel");
        return;
    }

    let binary = build_binary();
    let output = Command::new(&binary)
        .current_dir(workspace_dir())
        .env("AHMA_DISABLE_SANDBOX", "1")
        .args([
            "--log-to-stderr",
            "sandboxed_shell",
            "--",
            "echo legacy-kernel-fallback",
        ])
        .output()
        .expect("Failed to run ahma_mcp with AHMA_DISABLE_SANDBOX");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Expected explicit no-sandbox mode to run on legacy kernel. stderr:\n{}",
        stderr
    );
    assert!(
        stderr.contains("Continuing without Ahma sandbox")
            && stderr.contains("Update Linux kernel to 5.13+"),
        "Expected legacy-kernel warning with upgrade guidance. stderr:\n{}",
        stderr
    );
    assert!(
        stdout.contains("legacy-kernel-fallback"),
        "Expected command output. stdout:\n{}",
        stdout
    );
}
