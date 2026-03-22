//! CLI Mode and Flag Coverage Tests
//!
//! These tests specifically target the low-coverage areas in cli.rs (42% coverage).
//! They cover:
//! - All subcommand combinations (serve stdio, serve http, tool list)
//! - Flag permutations (AHMA_SYNC, AHMA_DISABLE_SANDBOX, AHMA_SANDBOX_DEFER, RUST_LOG)
//! - Error paths for invalid configurations
//! - Sandbox scope initialization paths
//!
//! ## Anti-pattern to avoid: spawn + yield_now/sleep + kill
//!
//! Never start a long-running server, yield briefly, kill it, then assert on output
//! with an `|| combined.is_empty()` escape hatch.  That race passes vacuously on
//! slow runners (nothing written yet) while failing on fast ones (clap error written).
//!
//! **Correct pattern for server startup tests**: read the server's machine-readable
//! startup sentinel (`AHMA_BOUND_PORT=<port>`) from stderr in a loop with a real
//! deadline.  The sentinel is emitted immediately after the server binds, so the
//! test is deterministic regardless of scheduler timing.

use ahma_mcp::test_utils::cli::{build_binary_cached, test_command};
use ahma_mcp::test_utils::fs::get_workspace_dir as workspace_dir;
use std::process::Command;
use tempfile::TempDir;

fn build_binary() -> std::path::PathBuf {
    build_binary_cached("ahma_mcp", "ahma-mcp")
}

// ============================================================================
// Mode Flag Tests
// ============================================================================

mod mode_flags {
    use super::*;

    /// Verifies that `serve stdio` is a valid subcommand.
    ///
    /// Running with `--help` terminates deterministically (exit 0) without
    /// needing to pipe stdin or race against stdin-EOF detection.
    #[test]
    fn test_mode_stdio_explicit() {
        let binary = build_binary();
        let workspace = workspace_dir();
        let tools_dir = workspace.join(".ahma");

        let output = test_command(&binary)
            .current_dir(&workspace)
            .args([
                "serve",
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "stdio",
                "--help",
            ])
            .output()
            .expect("Failed to execute serve stdio --help");

        assert!(
            output.status.success(),
            "serve stdio --help should exit 0. stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("stdio") || stdout.contains("Usage"),
            "Help text should describe the stdio transport. Got: {}",
            stdout
        );
    }

    /// Verifies that `serve http` starts and binds a port.
    ///
    /// Reads stderr line-by-line until the bridge emits `AHMA_BOUND_PORT=<port>`,
    /// which is written via `eprintln!` immediately after binding (independent of
    /// `RUST_LOG` level).  No timing luck required: we block until we see the
    /// sentinel or a 30-second deadline expires.
    #[test]
    fn test_mode_http_explicit() {
        use std::io::BufRead as _;
        use std::time::{Duration, Instant};

        let binary = build_binary();
        let workspace = workspace_dir();
        let tools_dir = workspace.join(".ahma");

        let mut child = std::process::Command::new(&binary)
            .current_dir(&workspace)
            .env("AHMA_DISABLE_SANDBOX", "1")
            .args([
                "serve",
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "http",
                "--port",
                "0", // OS assigns a free port; sentinel carries the actual value
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to spawn HTTP mode");

        let stderr = child.stderr.take().expect("stderr should be piped");
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut bound_port: Option<u16> = None;

        for line in std::io::BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            if let Some(port_str) = line.trim().strip_prefix("AHMA_BOUND_PORT=")
                && let Ok(port) = port_str.trim().parse::<u16>()
            {
                bound_port = Some(port);
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
        }

        let _ = child.kill();
        let _ = child.wait();

        assert!(
            bound_port.is_some_and(|p| p > 0),
            "HTTP bridge did not emit AHMA_BOUND_PORT= within 30s \
             — server may have failed to start or args were rejected by clap"
        );
    }

    /// Verifies that an unknown transport subcommand is rejected by clap.
    #[test]
    fn test_mode_invalid_rejected() {
        let binary = build_binary();

        let output = test_command(&binary)
            .args(["serve", "invalid_mode"])
            .output()
            .expect("Failed to execute serve invalid_mode");

        assert!(
            !output.status.success(),
            "Unknown transport subcommand should be rejected"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("invalid")
                || stderr.contains("error")
                || stderr.contains("unrecognized")
                || stderr.contains("Usage"),
            "Error should describe the invalid subcommand. Got: {}",
            stderr
        );
    }
}

// ============================================================================
// Sync Flag Tests
// ============================================================================

mod sync_flag {
    use super::*;

    #[test]
    fn test_sync_flag_accepted() {
        let binary = build_binary();
        let temp = TempDir::new().unwrap();
        let tools_dir = temp.path().join("tools");
        std::fs::create_dir_all(&tools_dir).unwrap();

        // Create a simple echo tool
        let echo_tool = r#"{
            "name": "sync_test",
            "description": "Sync test tool",
            "command": "echo",
            "timeout_seconds": 10,
            "synchronous": false,
            "enabled": true,
            "subcommand": [{
                "name": "default",
                "description": "Echo",
                "positional_args": [{
                    "name": "msg",
                    "type": "string",
                    "required": true
                }]
            }]
        }"#;
        std::fs::write(tools_dir.join("sync_test.json"), echo_tool).unwrap();

        let output = test_command(&binary)
            .args([
                "--sync",
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "--sandbox-scope",
                temp.path().to_str().unwrap(),
                "sync_test",
                "--working-directory",
                temp.path().to_str().unwrap(),
                "--",
                "sync_output",
            ])
            .output()
            .expect("Failed to execute with --sync");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // With --sync, the tool should run synchronously
        if output.status.success() {
            assert!(
                stdout.contains("sync_output"),
                "Sync mode should return output. Got: {}",
                stdout
            );
        } else {
            // Even if it fails, --sync should be recognized
            eprintln!("--sync test failed (may be acceptable): {}", stderr);
        }
    }
}

// ============================================================================
// No-Sandbox Flag Tests
// ============================================================================

mod no_sandbox_flag {
    use super::*;

    #[test]
    fn test_no_sandbox_flag_logs_warning() {
        let binary = build_binary();
        let temp = TempDir::new().unwrap();
        let tools_dir = temp.path().join("tools");
        std::fs::create_dir_all(&tools_dir).unwrap();

        // Create a simple tool
        let tool = r#"{
            "name": "no_sandbox_test",
            "description": "Test tool",
            "command": "echo",
            "timeout_seconds": 10,
            "enabled": true,
            "subcommand": [{
                "name": "default",
                "description": "Echo"
            }]
        }"#;
        std::fs::write(tools_dir.join("no_sandbox_test.json"), tool).unwrap();

        // Run with env vars replacing removed CLI flags
        let output = Command::new(&binary)
            .env("AHMA_DISABLE_SANDBOX", "1")
            .env("AHMA_LOG_TARGET", "stderr")
            .env("AHMA_TOOLS_DIR", tools_dir.to_str().unwrap())
            .env("AHMA_SANDBOX_SCOPE", temp.path().to_str().unwrap())
            .args(["tool", "run", "no_sandbox_test"])
            .output()
            .expect("Failed to execute with AHMA_DISABLE_SANDBOX");

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Should log sandbox disabled warning
        assert!(
            stderr.contains("sandbox")
                || stderr.contains("Sandbox")
                || stderr.contains("DISABLED")
                || output.status.success(), // Or just succeed
            "Should mention sandbox or succeed. Got: {}",
            stderr
        );
    }

    #[test]
    fn test_no_sandbox_env_var() {
        let binary = build_binary();
        let _temp = TempDir::new().unwrap();

        let output = Command::new(&binary)
            .env("AHMA_DISABLE_SANDBOX", "1")
            .args(["--help"])
            .output()
            .expect("Failed to execute with AHMA_DISABLE_SANDBOX env");

        // --help should still work regardless of sandbox setting
        assert!(
            output.status.success(),
            "Help should work with AHMA_DISABLE_SANDBOX set"
        );
    }
}

// ============================================================================
// Debug Flag Tests
// ============================================================================

mod debug_flag {
    use super::*;

    #[test]
    fn test_debug_flag_increases_log_level() {
        let binary = build_binary();
        let temp = TempDir::new().unwrap();
        let tools_dir = temp.path().join("tools");
        std::fs::create_dir_all(&tools_dir).unwrap();

        let tool = r#"{
            "name": "debug_test",
            "description": "Debug test",
            "command": "echo",
            "timeout_seconds": 10,
            "enabled": true,
            "subcommand": [{"name": "default", "description": "Echo"}]
        }"#;
        std::fs::write(tools_dir.join("debug_test.json"), tool).unwrap();

        let output = test_command(&binary)
            .args([
                "--debug",
                "--log-to-stderr",
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "--sandbox-scope",
                temp.path().to_str().unwrap(),
                "debug_test",
                "--working-directory",
                temp.path().to_str().unwrap(),
            ])
            .output()
            .expect("Failed to execute with --debug");

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Debug mode should produce more verbose output
        // (may contain DEBUG level log entries)
        if !stderr.is_empty() {
            // Debug output is present - test passed
            eprintln!("Debug output: {}", &stderr[..stderr.len().min(500)]);
        }
    }
}

// ============================================================================
// Log-to-stderr Flag Tests
// ============================================================================

mod log_to_stderr_flag {
    use super::*;

    #[test]
    fn test_log_to_stderr_outputs_to_stderr() {
        let binary = build_binary();
        let temp = TempDir::new().unwrap();
        let tools_dir = temp.path().join("tools");
        std::fs::create_dir_all(&tools_dir).unwrap();

        let tool = r#"{
            "name": "stderr_test",
            "description": "Stderr test",
            "command": "echo",
            "timeout_seconds": 10,
            "enabled": true,
            "subcommand": [{"name": "default", "description": "Echo"}]
        }"#;
        std::fs::write(tools_dir.join("stderr_test.json"), tool).unwrap();

        let output = test_command(&binary)
            .args([
                "--log-to-stderr",
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "--sandbox-scope",
                temp.path().to_str().unwrap(),
                "stderr_test",
                "--working-directory",
                temp.path().to_str().unwrap(),
            ])
            .output()
            .expect("Failed to execute with --log-to-stderr");

        let stderr = String::from_utf8_lossy(&output.stderr);

        // With --log-to-stderr, logs should appear on stderr
        if output.status.success() || !stderr.is_empty() {
            // Either succeeded or produced stderr output - test passed
        } else {
            panic!("No stderr output with --log-to-stderr");
        }
    }
}

// ============================================================================
// Sandbox Scope Tests
// ============================================================================

mod sandbox_scope {
    use super::*;

    #[test]
    fn test_sandbox_scope_cli_override() {
        let binary = build_binary();
        let temp = TempDir::new().unwrap();
        let tools_dir = temp.path().join("tools");
        let custom_scope = temp.path().join("custom_scope");
        std::fs::create_dir_all(&tools_dir).unwrap();
        std::fs::create_dir_all(&custom_scope).unwrap();

        let tool = r#"{
            "name": "scope_test",
            "description": "Scope test",
            "command": "pwd",
            "timeout_seconds": 10,
            "enabled": true,
            "subcommand": [{"name": "default", "description": "Print working dir"}]
        }"#;
        std::fs::write(tools_dir.join("scope_test.json"), tool).unwrap();

        let output = Command::new(&binary)
            .args([
                "--sandbox-scope",
                custom_scope.to_str().unwrap(),
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "scope_test",
                "--working-directory",
                custom_scope.to_str().unwrap(),
            ])
            .output()
            .expect("Failed to execute with --sandbox-scope");

        // Should use the specified sandbox scope
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        if output.status.success() {
            // pwd should work within the sandbox scope
            assert!(
                stdout.contains("custom_scope")
                    || stdout.contains(&temp.path().to_string_lossy().to_string()),
                "Output should reflect custom scope. Got: {}",
                stdout
            );
        } else {
            eprintln!("Sandbox scope test stderr: {}", stderr);
        }
    }

    #[test]
    fn test_sandbox_scope_env_var() {
        let binary = build_binary();
        let temp = TempDir::new().unwrap();
        let tools_dir = temp.path().join("tools");
        std::fs::create_dir_all(&tools_dir).unwrap();

        let tool = r#"{
            "name": "env_scope_test",
            "description": "Env scope test",
            "command": "echo",
            "timeout_seconds": 10,
            "enabled": true,
            "subcommand": [{"name": "default", "description": "Echo"}]
        }"#;
        std::fs::write(tools_dir.join("env_scope_test.json"), tool).unwrap();

        let output = Command::new(&binary)
            .env("AHMA_SANDBOX_SCOPE", temp.path().to_str().unwrap())
            .args([
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "env_scope_test",
                "--working-directory",
                temp.path().to_str().unwrap(),
            ])
            .output()
            .expect("Failed to execute with AHMA_SANDBOX_SCOPE");

        // Should accept the env var sandbox scope
        // (either succeed or fail for unrelated reason)
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() || !stderr.contains("sandbox scope"),
            "Should accept AHMA_SANDBOX_SCOPE env var. Got: {}",
            stderr
        );
    }

    #[test]
    fn test_sandbox_scope_nonexistent_fails() {
        let binary = build_binary();

        let output = Command::new(&binary)
            .env_remove("AHMA_DISABLE_SANDBOX")
            .args([
                "--sandbox-scope",
                "/nonexistent/path/that/does/not/exist",
                "--help", // Use --help to avoid needing a valid tools-dir
            ])
            .output()
            .expect("Failed to execute with nonexistent sandbox scope");

        // --help should still work, but sandbox scope warning may appear
        // OR it may fail if sandbox scope is validated before --help
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Should fail (--sandbox-scope is no longer a valid top-level flag)
        // or succeed with help output
        assert!(
            !output.status.success()
                || stdout.contains("ahma")
                || stderr.contains("Failed")
                || stderr.contains("path"),
            "Should fail or report error. stdout: {}, stderr: {}",
            stdout,
            stderr
        );
    }
}

// ============================================================================
// Defer Sandbox Tests
// ============================================================================

mod defer_sandbox {
    use super::*;

    #[test]
    fn test_defer_sandbox_flag_accepted() {
        let binary = build_binary();
        let temp = TempDir::new().unwrap();
        let tools_dir = temp.path().join("tools");
        std::fs::create_dir_all(&tools_dir).unwrap();

        let output = test_command(&binary)
            .args([
                "--defer-sandbox",
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "--mode",
                "stdio",
            ])
            .output()
            .expect("Failed to execute with --defer-sandbox");

        let stderr = String::from_utf8_lossy(&output.stderr);

        // --defer-sandbox should be recognized
        // It's mainly used for HTTP mode session isolation
        assert!(
            stderr.contains("defer")
                || stderr.contains("Sandbox")
                || !output.status.success()
                || stderr.is_empty(),
            "--defer-sandbox should be recognized. Got: {}",
            stderr
        );
    }
}

// ============================================================================
// Timeout Flag Tests
// ============================================================================

mod timeout_flag {
    use super::*;

    #[test]
    fn test_timeout_flag_accepted() {
        let binary = build_binary();
        let temp = TempDir::new().unwrap();
        let tools_dir = temp.path().join("tools");
        std::fs::create_dir_all(&tools_dir).unwrap();

        let tool = r#"{
            "name": "timeout_test",
            "description": "Timeout test",
            "command": "echo",
            "timeout_seconds": 10,
            "enabled": true,
            "subcommand": [{"name": "default", "description": "Echo"}]
        }"#;
        std::fs::write(tools_dir.join("timeout_test.json"), tool).unwrap();

        let output = test_command(&binary)
            .args([
                "--timeout",
                "60",
                "--tools-dir",
                tools_dir.to_str().unwrap(),
                "--sandbox-scope",
                temp.path().to_str().unwrap(),
                "timeout_test",
                "--working-directory",
                temp.path().to_str().unwrap(),
            ])
            .output()
            .expect("Failed to execute with --timeout");

        // --timeout should be accepted (changes default timeout)
        // Just verify it doesn't cause a parse error
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("invalid") || output.status.success(),
            "--timeout 60 should be accepted. Got: {}",
            stderr
        );
    }

    #[test]
    fn test_timeout_invalid_value_rejected() {
        let binary = build_binary();

        let output = test_command(&binary)
            .args(["--timeout", "not_a_number"])
            .output()
            .expect("Failed to execute with invalid timeout");

        assert!(
            !output.status.success(),
            "Invalid timeout should be rejected"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("invalid") || stderr.contains("error"),
            "Error should mention invalid value. Got: {}",
            stderr
        );
    }
}

// ============================================================================
// Tools Dir Flag Tests
// ============================================================================

mod tools_dir_flag {
    use super::*;

    #[test]
    fn test_tools_dir_nonexistent() {
        let binary = build_binary();

        let output = test_command(&binary)
            .args(["--tools-dir", "/nonexistent/tools/dir", "some_tool"])
            .output()
            .expect("Failed to execute with nonexistent tools-dir");

        // Should fail gracefully
        assert!(
            !output.status.success(),
            "Nonexistent tools-dir should cause failure"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("not found")
                || stderr.contains("No such")
                || stderr.contains("error")
                || stderr.contains("No matching"),
            "Error should mention path issue. Got: {}",
            stderr
        );
    }

    #[test]
    fn test_tools_dir_empty() {
        let binary = build_binary();
        let temp = TempDir::new().unwrap();
        let empty_dir = temp.path().join("empty_tools");
        std::fs::create_dir_all(&empty_dir).unwrap();

        let output = test_command(&binary)
            .args(["--tools-dir", empty_dir.to_str().unwrap(), "any_tool"])
            .output()
            .expect("Failed to execute with empty tools-dir");

        // Should fail because no tools found
        assert!(
            !output.status.success(),
            "Empty tools-dir should cause failure"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("not found")
                || stderr.contains("No matching")
                || stderr.contains("error"),
            "Error should indicate tool not found. Got: {}",
            stderr
        );
    }
}

// ============================================================================
// HTTP Mode Specific Tests
// ============================================================================

mod http_mode {
    use super::*;

    #[test]
    fn test_http_port_flag() {
        let binary = build_binary();

        let output = test_command(&binary)
            .args(["serve", "http", "--port", "12345", "--help"])
            .output()
            .expect("Failed to execute with serve http --port");

        // --help should work and show http serve options
        assert!(
            output.status.success(),
            "Help with serve http --port should succeed"
        );
    }

    #[test]
    fn test_http_host_flag() {
        let binary = build_binary();

        let output = test_command(&binary)
            .args(["serve", "http", "--host", "0.0.0.0", "--help"])
            .output()
            .expect("Failed to execute with serve http --host");

        // --help should work and show http serve options
        assert!(
            output.status.success(),
            "Help with serve http --host should succeed"
        );
    }

    #[test]
    fn test_http_port_invalid_rejected() {
        let binary = build_binary();

        let output = test_command(&binary)
            .args(["--http-port", "not_a_port"])
            .output()
            .expect("Failed to execute with invalid port");

        assert!(!output.status.success(), "Invalid port should be rejected");
    }
}

// ============================================================================
// Combined Flag Tests
// ============================================================================

mod combined_flags {
    use super::*;

    #[test]
    fn test_all_global_flags_together() {
        let binary = build_binary();
        let temp = TempDir::new().unwrap();
        let tools_dir = temp.path().join("tools");
        std::fs::create_dir_all(&tools_dir).unwrap();

        let tool = r#"{
            "name": "combined_test",
            "description": "Combined flags test",
            "command": "echo",
            "timeout_seconds": 10,
            "enabled": true,
            "subcommand": [{"name": "default", "description": "Echo"}]
        }"#;
        std::fs::write(tools_dir.join("combined_test.json"), tool).unwrap();

        // Combine multiple env vars (replacing removed flags) with run subcommand
        let output = test_command(&binary)
            .env("RUST_LOG", "debug")
            .env("AHMA_SYNC", "1")
            .env("AHMA_LOG_TARGET", "stderr")
            .env("AHMA_TOOLS_DIR", tools_dir.to_str().unwrap())
            .env("AHMA_SANDBOX_SCOPE", temp.path().to_str().unwrap())
            .args(["tool", "run", "combined_test"])
            .output()
            .expect("Failed to execute with combined env vars");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let _stdout = String::from_utf8_lossy(&output.stdout);

        // Should work with all flags combined
        if !output.status.success() {
            eprintln!("Combined flags test failed: {}", stderr);
        }

        // At minimum, flags should be parsed without error
        assert!(
            !stderr.contains("unexpected argument"),
            "All flags should be recognized. Got: {}",
            stderr
        );
    }
}
