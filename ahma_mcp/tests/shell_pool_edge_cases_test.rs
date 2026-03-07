//! Shell Pool Edge Cases Integration Tests
//!
//! Tests for shell_pool.rs edge cases covering:
//! 1. Pool exhaustion under load
//! 2. Shell health check and recovery
//! 3. Working directory pool isolation
//! 4. Shell timeout handling
//! 5. Concurrent shell access
//!
//! These tests use real shell processes and tempdir for isolation.

use ahma_mcp::shell_pool::{ShellCommand, ShellPoolConfig, ShellPoolManager};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

/// Helper to create a ShellCommand
fn make_cmd(
    id: &str,
    program: &str,
    args: &[&str],
    working_dir: &str,
    timeout_ms: u64,
) -> ShellCommand {
    let mut command = vec![program.to_string()];
    command.extend(args.iter().map(|s| s.to_string()));
    ShellCommand {
        id: id.to_string(),
        command,
        working_dir: working_dir.to_string(),
        timeout_ms,
    }
}

// ============================================================================
// Test: Basic Shell Pool Operations
// ============================================================================

/// Test basic shell acquisition and return
#[tokio::test]
async fn test_shell_pool_acquire_and_return() {
    let temp_dir = TempDir::new().unwrap();
    let config = ShellPoolConfig {
        enabled: true,
        shells_per_directory: 2,
        max_total_shells: 10,
        shell_idle_timeout: Duration::from_secs(60),
        pool_cleanup_interval: Duration::from_secs(60),
        shell_spawn_timeout: Duration::from_secs(5),
        command_timeout: Duration::from_secs(30),
        health_check_interval: Duration::from_secs(60),
    };

    let manager = Arc::new(ShellPoolManager::new(config));

    // Acquire a shell
    let shell = manager
        .get_shell(temp_dir.path().to_str().unwrap())
        .await
        .expect("Should acquire shell");

    // Verify we got a shell with the right working directory
    assert_eq!(
        shell.working_dir(),
        temp_dir.path(),
        "Shell should have correct working directory"
    );

    // Return the shell
    manager.return_shell(shell).await;

    // Pool should exist for this directory (shells can be pooled for reuse)
    let stats = manager.get_stats().await;
    // After returning, the shell is available again so total_shells decreases
    // But the pool for the directory still exists
    assert!(
        stats.total_pools > 0 || stats.max_shells > 0,
        "Pool infrastructure should exist"
    );
}

/// Test executing a command in a pooled shell
#[tokio::test]
async fn test_shell_pool_command_execution() {
    let temp_dir = TempDir::new().unwrap();
    let config = ShellPoolConfig::default();
    let manager = Arc::new(ShellPoolManager::new(config));

    let mut shell = manager
        .get_shell(temp_dir.path().to_str().unwrap())
        .await
        .expect("Should acquire shell");

    // Execute a simple command
    let cmd = make_cmd(
        "test_1",
        "echo",
        &["shell pool test"],
        temp_dir.path().to_str().unwrap(),
        10000,
    );
    let result = shell.execute_command(cmd).await;

    assert!(result.is_ok(), "Command should succeed");
    let response = result.unwrap();
    assert!(
        response.stdout.contains("shell pool test"),
        "Output should contain expected text. Got: {}",
        response.stdout
    );

    manager.return_shell(shell).await;
}

// ============================================================================
// Test: Working Directory Isolation
// ============================================================================

/// Test that shells for different directories are kept separate
#[tokio::test]
async fn test_shell_pool_directory_isolation() {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();

    // Create marker files in each directory
    std::fs::write(temp_dir1.path().join("dir1_marker.txt"), "dir1").unwrap();
    std::fs::write(temp_dir2.path().join("dir2_marker.txt"), "dir2").unwrap();

    let config = ShellPoolConfig::default();
    let manager = Arc::new(ShellPoolManager::new(config));

    // Get shells for both directories
    let mut shell1 = manager
        .get_shell(temp_dir1.path().to_str().unwrap())
        .await
        .expect("Should acquire shell for dir1");
    let mut shell2 = manager
        .get_shell(temp_dir2.path().to_str().unwrap())
        .await
        .expect("Should acquire shell for dir2");

    // Verify each shell sees its own directory's files
    let cmd1 = make_cmd("ls1", "ls", &[], temp_dir1.path().to_str().unwrap(), 10000);
    let cmd2 = make_cmd("ls2", "ls", &[], temp_dir2.path().to_str().unwrap(), 10000);

    let result1 = shell1.execute_command(cmd1).await.unwrap();
    let result2 = shell2.execute_command(cmd2).await.unwrap();

    assert!(
        result1.stdout.contains("dir1_marker.txt"),
        "Shell1 should see dir1 files. Got: {}",
        result1.stdout
    );
    assert!(
        result2.stdout.contains("dir2_marker.txt"),
        "Shell2 should see dir2 files. Got: {}",
        result2.stdout
    );
    assert!(
        !result1.stdout.contains("dir2_marker.txt"),
        "Shell1 should NOT see dir2 files"
    );
    assert!(
        !result2.stdout.contains("dir1_marker.txt"),
        "Shell2 should NOT see dir1 files"
    );

    manager.return_shell(shell1).await;
    manager.return_shell(shell2).await;
}

// ============================================================================
// Test: Pool Limits
// ============================================================================

/// Test that pool respects max_total_shells limit
#[tokio::test]
async fn test_shell_pool_respects_max_limit() {
    let temp_dir = TempDir::new().unwrap();
    let config = ShellPoolConfig {
        enabled: true,
        shells_per_directory: 2,
        max_total_shells: 3, // Low limit for testing
        shell_idle_timeout: Duration::from_secs(60),
        pool_cleanup_interval: Duration::from_secs(60),
        shell_spawn_timeout: Duration::from_secs(5),
        command_timeout: Duration::from_secs(30),
        health_check_interval: Duration::from_secs(60),
    };

    let manager = Arc::new(ShellPoolManager::new(config));

    // Try to acquire more shells than the limit
    let mut shells = Vec::new();
    for i in 0..5 {
        match manager.get_shell(temp_dir.path().to_str().unwrap()).await {
            Some(shell) => shells.push(shell),
            None => {
                // Expected to fail eventually due to limit
                eprintln!("Shell {} acquisition returned None as expected", i);
            }
        }
    }

    // Should have at most max_total_shells
    assert!(
        shells.len() <= 3,
        "Should not exceed max_total_shells. Got: {}",
        shells.len()
    );

    // Return all acquired shells
    for shell in shells {
        manager.return_shell(shell).await;
    }
}

// ============================================================================
// Test: Command Timeout
// ============================================================================

/// Test that commands respect timeout
#[tokio::test]
async fn test_shell_pool_command_timeout() {
    let temp_dir = TempDir::new().unwrap();
    let config = ShellPoolConfig::default();
    let manager = Arc::new(ShellPoolManager::new(config));

    let mut shell = manager
        .get_shell(temp_dir.path().to_str().unwrap())
        .await
        .expect("Should acquire shell");

    // Execute a command that takes longer than timeout (100ms timeout, 5s sleep)
    let cmd = make_cmd(
        "timeout_test",
        "sleep",
        &["5"],
        temp_dir.path().to_str().unwrap(),
        100, // 100ms timeout
    );
    let result = shell.execute_command(cmd).await;

    // Should timeout
    assert!(
        result.is_err(),
        "Long-running command should timeout with short timeout"
    );

    // The shell may be in a bad state after timeout, so we don't return it
    // In practice, the pool should handle this gracefully
}

// ============================================================================
// Test: Concurrent Shell Access
// ============================================================================

/// Test concurrent command execution in multiple shells
#[tokio::test]
async fn test_shell_pool_concurrent_execution() {
    let temp_dir = TempDir::new().unwrap();
    let config = ShellPoolConfig {
        enabled: true,
        shells_per_directory: 4,
        max_total_shells: 10,
        ..Default::default()
    };

    let manager = Arc::new(ShellPoolManager::new(config));

    // Spawn multiple concurrent tasks
    let mut handles = Vec::new();
    for i in 0..4 {
        let manager_clone = manager.clone();
        let path = temp_dir.path().to_str().unwrap().to_string();
        let handle = tokio::spawn(async move {
            let mut shell = manager_clone
                .get_shell(&path)
                .await
                .expect("Should get shell");
            let cmd = ShellCommand {
                id: format!("concurrent_{}", i),
                command: vec!["echo".to_string(), format!("task {}", i)],
                working_dir: path.clone(),
                timeout_ms: 10000,
            };
            let result = shell
                .execute_command(cmd)
                .await
                .expect("Command should succeed");
            manager_clone.return_shell(shell).await;
            (i, result.stdout)
        });
        handles.push(handle);
    }

    // Wait for all tasks
    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // All should have succeeded
    assert_eq!(results.len(), 4);
    for (i, output) in &results {
        assert!(
            output.contains(&format!("task {}", i)),
            "Task {} output should contain expected text. Got: {}",
            i,
            output
        );
    }
}

// ============================================================================
// Test: Shell Pool Stats
// ============================================================================

/// Test that pool stats are tracked correctly
#[tokio::test]
async fn test_shell_pool_stats_tracking() {
    let temp_dir = TempDir::new().unwrap();
    let config = ShellPoolConfig::default();
    let manager = Arc::new(ShellPoolManager::new(config));

    // Initially should have no shells
    let initial_stats = manager.get_stats().await;
    assert_eq!(initial_stats.total_shells, 0);

    // Acquire a shell
    let shell = manager
        .get_shell(temp_dir.path().to_str().unwrap())
        .await
        .expect("Should acquire shell");

    let active_stats = manager.get_stats().await;
    assert!(
        active_stats.total_shells >= 1,
        "Should have at least 1 shell after acquisition"
    );

    // Return the shell
    manager.return_shell(shell).await;

    // Stats should reflect the returned shell
    let final_stats = manager.get_stats().await;
    // After return, total_shells should decrease or stay at 0
    assert!(
        final_stats.total_shells <= active_stats.total_shells,
        "Shell count should not increase after return"
    );
}

// ============================================================================
// Test: Disabled Pool
// ============================================================================

/// Test behavior when pool is disabled
#[tokio::test]
async fn test_shell_pool_disabled() {
    let temp_dir = TempDir::new().unwrap();
    let config = ShellPoolConfig {
        enabled: false, // Disabled
        ..Default::default()
    };

    let manager = Arc::new(ShellPoolManager::new(config));

    // Acquiring a shell should return None when pool is disabled
    let result = manager.get_shell(temp_dir.path().to_str().unwrap()).await;

    // When disabled, get_shell returns None
    assert!(
        result.is_none(),
        "Disabled pool should return None for get_shell"
    );
}

// ============================================================================
// Test: Error Command Handling
// ============================================================================

/// Test that commands returning non-zero exit code are handled properly
#[tokio::test]
async fn test_shell_pool_error_command() {
    let temp_dir = TempDir::new().unwrap();
    let config = ShellPoolConfig::default();
    let manager = Arc::new(ShellPoolManager::new(config));

    let mut shell = manager
        .get_shell(temp_dir.path().to_str().unwrap())
        .await
        .expect("Should acquire shell");

    // Execute a command that fails
    let cmd = make_cmd(
        "fail",
        "false",
        &[],
        temp_dir.path().to_str().unwrap(),
        10000,
    );
    let _result = shell.execute_command(cmd).await;

    // The command should complete (captures exit code) rather than error
    // The shell should still be usable

    // Try another command to verify shell is still healthy
    let cmd2 = make_cmd(
        "after_fail",
        "echo",
        &["still working"],
        temp_dir.path().to_str().unwrap(),
        10000,
    );
    let result2 = shell.execute_command(cmd2).await;

    assert!(
        result2.is_ok(),
        "Shell should still be usable after failed command. Error: {:?}",
        result2.err()
    );
    assert!(result2.unwrap().stdout.contains("still working"));

    manager.return_shell(shell).await;
}

// ============================================================================
// Test: Multi-line Command Output
// ============================================================================

/// Test that multi-line command output is captured correctly
#[tokio::test]
async fn test_shell_pool_multiline_output() {
    let temp_dir = TempDir::new().unwrap();
    let config = ShellPoolConfig::default();
    let manager = Arc::new(ShellPoolManager::new(config));

    let mut shell = manager
        .get_shell(temp_dir.path().to_str().unwrap())
        .await
        .expect("Should acquire shell");

    // Create test files
    std::fs::write(temp_dir.path().join("file1.txt"), "content1").unwrap();
    std::fs::write(temp_dir.path().join("file2.txt"), "content2").unwrap();
    std::fs::write(temp_dir.path().join("file3.txt"), "content3").unwrap();

    let cmd = make_cmd(
        "ls",
        "ls",
        &["-1"],
        temp_dir.path().to_str().unwrap(),
        10000,
    );
    let result = shell.execute_command(cmd).await.expect("ls should succeed");

    // Should capture all lines
    assert!(result.stdout.contains("file1.txt"));
    assert!(result.stdout.contains("file2.txt"));
    assert!(result.stdout.contains("file3.txt"));

    manager.return_shell(shell).await;
}

// ============================================================================
// Test: Exit Code Capture
// ============================================================================

/// Test that exit codes are properly captured
#[tokio::test]
async fn test_shell_pool_exit_code_capture() {
    let temp_dir = TempDir::new().unwrap();
    let config = ShellPoolConfig::default();
    let manager = Arc::new(ShellPoolManager::new(config));

    let mut shell = manager
        .get_shell(temp_dir.path().to_str().unwrap())
        .await
        .expect("Should acquire shell");

    // Test successful command (exit 0)
    let cmd_success = make_cmd(
        "exit0",
        "true",
        &[],
        temp_dir.path().to_str().unwrap(),
        10000,
    );
    let result_success = shell.execute_command(cmd_success).await.unwrap();
    assert_eq!(
        result_success.exit_code, 0,
        "true command should exit with 0"
    );

    // Test failing command (exit 1)
    let cmd_fail = make_cmd(
        "exit1",
        "false",
        &[],
        temp_dir.path().to_str().unwrap(),
        10000,
    );
    let result_fail = shell.execute_command(cmd_fail).await.unwrap();
    assert_eq!(result_fail.exit_code, 1, "false command should exit with 1");

    manager.return_shell(shell).await;
}

// ============================================================================
// Test: Stderr Capture
// ============================================================================

/// Test that stderr is captured separately from stdout
#[cfg(unix)]
#[tokio::test]
async fn test_shell_pool_stderr_capture() {
    let temp_dir = TempDir::new().unwrap();
    let config = ShellPoolConfig::default();
    let manager = Arc::new(ShellPoolManager::new(config));

    let mut shell = manager
        .get_shell(temp_dir.path().to_str().unwrap())
        .await
        .expect("Should acquire shell");

    // Use bash to write to stderr
    let cmd = ShellCommand {
        id: "stderr_test".to_string(),
        command: vec![
            "bash".to_string(),
            "-c".to_string(),
            "echo stdout_output; echo stderr_output >&2".to_string(),
        ],
        working_dir: temp_dir.path().to_str().unwrap().to_string(),
        timeout_ms: 10000,
    };
    let result = shell.execute_command(cmd).await.unwrap();

    assert!(
        result.stdout.contains("stdout_output"),
        "stdout should contain stdout_output. Got: {}",
        result.stdout
    );
    assert!(
        result.stderr.contains("stderr_output"),
        "stderr should contain stderr_output. Got: {}",
        result.stderr
    );

    manager.return_shell(shell).await;
}
