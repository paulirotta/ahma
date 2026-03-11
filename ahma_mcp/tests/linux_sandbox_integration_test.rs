//! Linux Sandbox Integration Tests (Landlock)
//!
//! These tests verify that the Linux Landlock sandbox works correctly
//! for kernel-level file system access control. This is critical because
//! tests might run with permissive settings that bypass strict sandbox enforcement.
//!
//! These tests MUST run WITHOUT test mode to catch sandbox configuration errors.
//!
//! NOTE: These tests require Linux kernel 5.13+ with Landlock LSM enabled.
//! They will be skipped if Landlock is not available.

// This entire test file is Linux-specific - skip compilation on other platforms
#![cfg(target_os = "linux")]

use landlock::{
    ABI, Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use tempfile::TempDir;

/// Check if Landlock is available on this Linux system.
/// Returns true only when Landlock enforcement is actually effective.
fn is_landlock_available() -> bool {
    static LANDLOCK_EFFECTIVE: OnceLock<bool> = OnceLock::new();
    *LANDLOCK_EFFECTIVE.get_or_init(detect_landlock_enforcement)
}

/// Detect whether Landlock can be enforced effectively on this host.
fn detect_landlock_enforcement() -> bool {
    if !kernel_version_supports_landlock() {
        return false;
    }
    if !check_lsm_list_for_landlock() {
        return false;
    }
    probe_landlock_in_child()
}

/// Returns false if we can read the LSM list and landlock is absent.
fn check_lsm_list_for_landlock() -> bool {
    if let Ok(content) = fs::read_to_string("/sys/kernel/security/lsm")
        && !content.contains("landlock")
    {
        return false;
    }
    true
}

/// Spawn a child that enforces Landlock and verifies writes outside scope are blocked.
fn probe_landlock_in_child() -> bool {
    let temp = match TempDir::new() {
        Ok(temp) => temp,
        Err(_) => return false,
    };
    let sandbox_subdir = temp.path().join("probe_scope");
    if fs::create_dir(&sandbox_subdir).is_err() {
        return false;
    }
    let blocked_file = temp.path().join("probe_blocked.txt");
    let sandbox_scope_str = sandbox_subdir.to_string_lossy().to_string();

    let output = unsafe {
        Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("echo probe > {}", blocked_file.display()))
            .current_dir(&sandbox_subdir)
            .pre_exec(move || {
                apply_landlock_rules_strict(&sandbox_scope_str)?;
                Ok(())
            })
            .output()
    };
    match output {
        Ok(result) => !result.status.success() && !blocked_file.exists(),
        Err(_) => false,
    }
}

/// Kernel version guard (Landlock requires Linux 5.13+).
fn kernel_version_supports_landlock() -> bool {
    parse_kernel_version()
        .map(|(major, minor)| major > 5 || (major == 5 && minor >= 13))
        .unwrap_or(false)
}

/// Parse the running kernel's (major, minor) version numbers.
fn parse_kernel_version() -> Option<(u32, u32)> {
    let output = Command::new("uname").arg("-r").output().ok()?;
    let version_str = String::from_utf8_lossy(&output.stdout);
    let mut parts = version_str.trim().split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts
        .next()?
        .split('-')
        .next()?
        .parse()
        .unwrap_or(0);
    Some((major, minor))
}

/// Macro to skip test if Landlock is not available
macro_rules! skip_if_landlock_unavailable {
    () => {
        if !is_landlock_available() {
            eprintln!(
                "Skipping test: Landlock not available (requires Linux kernel 5.13+ with Landlock LSM)"
            );
            return;
        }
    };
}

/// Apply Landlock restrictions to restrict file access to sandbox scope only.
/// This creates a child process with Landlock rules applied.
///
/// Returns the exit status and output of the sandboxed command.
fn run_sandboxed_command(
    sandbox_scope: &Path,
    shell: &str,
    script: &str,
    working_dir: &Path,
) -> std::io::Result<std::process::Output> {
    // We need to fork a child process and apply Landlock there,
    // since Landlock restrictions are inherited but cannot be removed.
    // Use a simple wrapper approach: spawn a process that applies Landlock then execs.

    // For testing purposes, we'll use the pre_exec hook to apply Landlock
    // before the child process executes the shell command.
    let sandbox_scope_str = sandbox_scope.to_string_lossy().to_string();

    unsafe {
        Command::new(shell)
            .arg("-c")
            .arg(script)
            .current_dir(working_dir)
            .pre_exec(move || {
                apply_landlock_rules(&sandbox_scope_str)?;
                Ok(())
            })
            .output()
    }
}

/// Create a Landlock ruleset allowing full access to `sandbox_scope`.
fn create_ruleset_with_scope(
    sandbox_path: &Path,
    abi: ABI,
) -> std::io::Result<landlock::RulesetCreated> {
    let access_all = AccessFs::from_all(abi);
    let mut ruleset = Ruleset::default()
        .handle_access(access_all)
        .map_err(|e| std::io::Error::other(e.to_string()))?
        .create()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    ruleset = ruleset
        .add_rule(PathBeneath::new(
            PathFd::new(sandbox_path).map_err(|e| std::io::Error::other(e.to_string()))?,
            access_all,
        ))
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(ruleset)
}

const SYSTEM_PATHS: &[&str] = &["/usr", "/bin", "/etc", "/lib", "/lib64", "/proc", "/dev"];

/// Add read-only access rules for standard system directories.
fn add_system_read_rules(ruleset: &mut landlock::RulesetCreated, abi: ABI) {
    let access_read = AccessFs::from_read(abi);
    for path in SYSTEM_PATHS {
        let path_obj = Path::new(path);
        if path_obj.exists()
            && let Ok(fd) = PathFd::new(path_obj)
        {
            let _ = ruleset.add_rule(PathBeneath::new(fd, access_read));
        }
    }
}

/// Apply Landlock rules to the current process.
/// This restricts file system access to the sandbox scope.
fn apply_landlock_rules(sandbox_scope: &str) -> std::io::Result<()> {
    let sandbox_path = Path::new(sandbox_scope);
    let abi = ABI::V3;

    let mut ruleset = create_ruleset_with_scope(sandbox_path, abi)?;
    add_system_read_rules(&mut ruleset, abi);

    // Also allow /tmp for temporary files during execution
    if let Ok(fd) = PathFd::new(Path::new("/tmp")) {
        ruleset = ruleset
            .add_rule(PathBeneath::new(fd, AccessFs::from_all(abi)))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
    }

    ruleset
        .restrict_self()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(())
}

/// Apply strict Landlock rules to the current process.
/// This version does NOT allow /tmp globally - only the exact sandbox scope.
/// Used for testing that writes outside the sandbox are blocked.
fn apply_landlock_rules_strict(sandbox_scope: &str) -> std::io::Result<()> {
    let sandbox_path = Path::new(sandbox_scope);
    let abi = ABI::V3;

    let mut ruleset = create_ruleset_with_scope(sandbox_path, abi)?;
    add_system_read_rules(&mut ruleset, abi);
    // NOTE: Intentionally NOT allowing /tmp here - this is the strict version

    ruleset
        .restrict_self()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(())
}

/// Test that Landlock can execute basic shell commands (echo)
#[test]
fn test_landlock_executes_echo() {
    skip_if_landlock_unavailable!();
    let temp = TempDir::new().expect("Failed to create temp dir");

    let output = run_sandboxed_command(temp.path(), "/bin/sh", "echo 'hello world'", temp.path())
        .expect("Failed to execute sandboxed command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Landlock sandbox should allow echo. Exit: {:?}, stdout: {}, stderr: {}",
        output.status.code(),
        stdout,
        stderr
    );
    assert!(
        stdout.contains("hello world"),
        "Output should contain 'hello world'. Got: {}",
        stdout
    );
}

/// Test that pwd works within the Landlock sandbox
#[test]
fn test_landlock_executes_pwd() {
    skip_if_landlock_unavailable!();
    let temp = TempDir::new().expect("Failed to create temp dir");

    let output = run_sandboxed_command(temp.path(), "/bin/sh", "pwd", temp.path())
        .expect("Failed to execute sandboxed command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Landlock sandbox should allow pwd. Exit: {:?}, stdout: {}, stderr: {}",
        output.status.code(),
        stdout,
        stderr
    );

    // pwd should return the temp directory path
    let stdout_trimmed = stdout.trim();
    assert!(
        stdout_trimmed.starts_with("/tmp") || stdout_trimmed.starts_with("/var"),
        "pwd should return temp directory. Got: {}",
        stdout
    );
}

/// Test that ls works within the Landlock sandbox
#[test]
fn test_landlock_executes_ls() {
    skip_if_landlock_unavailable!();
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create a test file
    fs::write(temp.path().join("testfile.txt"), "test content")
        .expect("Failed to create test file");

    let output = run_sandboxed_command(temp.path(), "/bin/sh", "ls -la", temp.path())
        .expect("Failed to execute sandboxed command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Landlock sandbox should allow ls. Exit: {:?}, stdout: {}, stderr: {}",
        output.status.code(),
        stdout,
        stderr
    );
    assert!(
        stdout.contains("testfile.txt"),
        "ls output should contain our test file. Got: {}",
        stdout
    );
}

/// Test that file writing works within the sandbox scope
#[test]
fn test_landlock_allows_writes_in_scope() {
    skip_if_landlock_unavailable!();
    let temp = TempDir::new().expect("Failed to create temp dir");

    let output = run_sandboxed_command(
        temp.path(),
        "/bin/sh",
        "echo 'test content' > output.txt && cat output.txt",
        temp.path(),
    )
    .expect("Failed to execute sandboxed command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Landlock should allow writes in sandbox scope. Exit: {:?}, stdout: {}, stderr: {}",
        output.status.code(),
        stdout,
        stderr
    );
    assert!(
        stdout.contains("test content"),
        "Should be able to read written file. Got: {}",
        stdout
    );

    // Verify the file was actually created
    assert!(
        temp.path().join("output.txt").exists(),
        "output.txt should exist"
    );
}

/// Test that file writing is blocked outside the sandbox scope
#[test]
fn test_landlock_blocks_writes_outside_scope() {
    skip_if_landlock_unavailable!();

    // Create sandbox scope inside a subdirectory of /tmp
    // We use a custom apply_landlock_rules_strict that does NOT allow /tmp globally
    let temp = TempDir::new().expect("Failed to create temp dir");
    let sandbox_subdir = temp.path().join("sandbox");
    fs::create_dir(&sandbox_subdir).expect("Failed to create sandbox subdir");

    // The blocked path is in the parent temp dir, outside the sandbox scope
    let blocked_file = temp.path().join("blocked.txt");

    // Use a stricter sandbox that only allows the sandbox_subdir, not all of /tmp
    let sandbox_scope_str = sandbox_subdir.to_string_lossy().to_string();

    let output = unsafe {
        Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("echo 'test' > {}", blocked_file.display()))
            .current_dir(&sandbox_subdir)
            .pre_exec(move || {
                apply_landlock_rules_strict(&sandbox_scope_str)?;
                Ok(())
            })
            .output()
    }
    .expect("Failed to execute sandboxed command");

    // The command should fail because blocked_file is outside sandbox scope
    // Landlock returns EACCES (Permission denied) for blocked operations
    assert!(
        !output.status.success(),
        "Landlock should block writes outside sandbox scope. Exit: {:?}",
        output.status.code()
    );

    // Verify the file was NOT created
    assert!(
        !blocked_file.exists(),
        "blocked.txt should NOT exist outside sandbox"
    );
}

/// Test that complex shell commands work (pipes, redirects, etc.)
#[test]
fn test_landlock_handles_complex_shell_commands() {
    skip_if_landlock_unavailable!();
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create a test file with known content
    fs::write(temp.path().join("input.txt"), "line1\nline2\nline3")
        .expect("Failed to create input file");

    // Test pipes and command substitution
    let output = run_sandboxed_command(
        temp.path(),
        "/bin/sh",
        "cat input.txt | grep line | wc -l | tr -d ' '",
        temp.path(),
    )
    .expect("Failed to execute sandboxed command");

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Landlock sandbox should handle pipes. Exit: {:?}, stdout: {}, stderr: {}",
        output.status.code(),
        stdout,
        stderr
    );
    // Should find 3 lines containing "line"
    assert_eq!(
        stdout, "3",
        "Should find 3 lines with 'line'. Got: {}",
        stdout
    );
}

/// Test that bash specifically works (not just /bin/sh)
#[test]
fn test_landlock_works_with_bash() {
    skip_if_landlock_unavailable!();
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Check if bash exists
    if !Path::new("/bin/bash").exists() {
        eprintln!("Skipping test: /bin/bash not found");
        return;
    }

    let output = run_sandboxed_command(
        temp.path(),
        "/bin/bash",
        "VAR='hello'; echo $VAR",
        temp.path(),
    )
    .expect("Failed to execute sandboxed command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Landlock sandbox should work with bash. Exit: {:?}, stdout: {}, stderr: {}",
        output.status.code(),
        stdout,
        stderr
    );
    assert!(
        stdout.contains("hello"),
        "Bash variable expansion should work. Got: {}",
        stdout
    );
}

/// Verify that Landlock is available on this system (informational test)
#[test]
fn test_landlock_availability_check() {
    // This test always runs to provide diagnostic information
    if is_landlock_available() {
        eprintln!("OK Landlock is available on this system");

        // Additional diagnostics
        if let Ok(content) = fs::read_to_string("/sys/kernel/security/lsm") {
            eprintln!("  LSMs enabled: {}", content.trim());
        }

        if let Ok(output) = Command::new("uname").arg("-r").output() {
            let version = String::from_utf8_lossy(&output.stdout);
            eprintln!("  Kernel version: {}", version.trim());
        }
    } else {
        eprintln!("✗ Landlock is NOT available on this system");
        eprintln!("  Requirements: Linux kernel 5.13+ with Landlock LSM enabled");
    }
}

/// Test reading files inside vs outside the sandbox
#[test]
fn test_landlock_read_restrictions() {
    skip_if_landlock_unavailable!();

    // Create a single temp dir with subdirectories for controlled scope
    let temp = TempDir::new().expect("Failed to create temp dir");
    let sandbox_scope = temp.path().join("sandbox");
    let outside_scope = temp.path().join("outside");
    fs::create_dir(&sandbox_scope).expect("Failed to create sandbox dir");
    fs::create_dir(&outside_scope).expect("Failed to create outside dir");

    // Create a file inside the sandbox scope
    fs::write(sandbox_scope.join("readable.txt"), "sandbox content")
        .expect("Failed to create test file");

    // Create a file outside the sandbox scope (but still in temp)
    fs::write(outside_scope.join("outside.txt"), "outside content")
        .expect("Failed to create outside file");

    // Reading inside sandbox should work - use strict rules
    let sandbox_scope_str = sandbox_scope.to_string_lossy().to_string();
    let output = unsafe {
        Command::new("/bin/sh")
            .arg("-c")
            .arg("cat readable.txt")
            .current_dir(&sandbox_scope)
            .pre_exec(move || {
                apply_landlock_rules_strict(&sandbox_scope_str)?;
                Ok(())
            })
            .output()
    }
    .expect("Failed to execute sandboxed command");

    assert!(
        output.status.success(),
        "Should be able to read files inside sandbox. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("sandbox content"));

    // Reading outside sandbox should fail - need fresh sandbox scope string
    let sandbox_scope_str2 = sandbox_scope.to_string_lossy().to_string();
    let outside_file = outside_scope.join("outside.txt");
    let script = format!("cat {}", outside_file.display());
    let output = unsafe {
        Command::new("/bin/sh")
            .arg("-c")
            .arg(&script)
            .current_dir(&sandbox_scope)
            .pre_exec(move || {
                apply_landlock_rules_strict(&sandbox_scope_str2)?;
                Ok(())
            })
            .output()
    }
    .expect("Failed to execute sandboxed command");

    assert!(
        !output.status.success(),
        "Should NOT be able to read files outside sandbox. Exit: {:?}",
        output.status.code()
    );
}
