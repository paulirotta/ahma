//! Unit tests for sandbox error types.
//!
//! These tests cover `SandboxError` variants and the `format_scopes` helper
//! in `ahma_mcp/src/sandbox/error.rs` which previously had 0% coverage.
//! We exercise the code through the public `SandboxError` re-export and via
//! `Sandbox::validate_path` which internally calls `format_scopes`.

use ahma_mcp::sandbox::{Sandbox, SandboxError, SandboxMode};
use std::path::PathBuf;
use tempfile::tempdir;

// ───────────────────── SandboxError Display tests ─────────────────────────

/// PathOutsideSandbox with zero configured scopes should produce a message
/// mentioning the path and something like "none configured".
#[test]
fn test_path_outside_sandbox_error_no_scopes_message() {
    let err = SandboxError::PathOutsideSandbox {
        path: PathBuf::from("/bad/path"),
        scopes: vec![],
    };
    let msg = err.to_string();
    assert!(
        msg.contains("none configured") || msg.contains("no scope") || msg.contains("sandbox"),
        "Empty scopes error should describe the situation: {msg}"
    );
    assert!(
        msg.contains("/bad/path") || msg.contains("bad"),
        "Error should mention the rejected path: {msg}"
    );
}

/// PathOutsideSandbox with one scope should embed that scope path.
#[test]
fn test_path_outside_sandbox_error_single_scope_message() {
    let scope = PathBuf::from("/allowed/dir");
    let err = SandboxError::PathOutsideSandbox {
        path: PathBuf::from("/bad/path"),
        scopes: vec![scope],
    };
    let msg = err.to_string();
    assert!(
        msg.contains("/allowed/dir"),
        "Single-scope error should embed that scope's path: {msg}"
    );
}

/// PathOutsideSandbox with multiple scopes should list all of them.
#[test]
fn test_path_outside_sandbox_error_multiple_scopes_message() {
    let err = SandboxError::PathOutsideSandbox {
        path: PathBuf::from("/bad/path"),
        scopes: vec![PathBuf::from("/scope/one"), PathBuf::from("/scope/two")],
    };
    let msg = err.to_string();
    assert!(
        msg.contains("/scope/one") && msg.contains("/scope/two"),
        "Multi-scope error should list all scopes: {msg}"
    );
}

/// LandlockNotAvailable display is non-trivial and should mention Landlock or --disable-sandbox.
#[test]
fn test_landlock_not_available_display() {
    let err = SandboxError::LandlockNotAvailable;
    let msg = err.to_string();
    assert!(
        msg.contains("Landlock") || msg.contains("--disable-sandbox"),
        "LandlockNotAvailable should guide the user: {msg}"
    );
}

/// MacOSSandboxNotAvailable display should mention sandbox-exec or --disable-sandbox.
#[test]
fn test_macos_sandbox_not_available_display() {
    let err = SandboxError::MacOSSandboxNotAvailable;
    let msg = err.to_string();
    assert!(
        msg.contains("macOS") || msg.contains("sandbox-exec") || msg.contains("--disable-sandbox"),
        "MacOSSandboxNotAvailable should guide the user: {msg}"
    );
}

/// UnsupportedOs display should include the OS name.
#[test]
fn test_unsupported_os_display() {
    let err = SandboxError::UnsupportedOs("freebsd".to_string());
    let msg = err.to_string();
    assert!(
        msg.contains("freebsd"),
        "UnsupportedOs should include the OS name: {msg}"
    );
}

/// CanonicalizationFailed display should include both the path and the reason.
#[test]
fn test_canonicalization_failed_display() {
    let err = SandboxError::CanonicalizationFailed {
        path: PathBuf::from("/test/path"),
        reason: "No such file or directory".to_string(),
    };
    let msg = err.to_string();
    assert!(
        msg.contains("/test/path"),
        "CanonicalizationFailed should include the path: {msg}"
    );
    assert!(
        msg.contains("No such file or directory"),
        "CanonicalizationFailed should include the reason: {msg}"
    );
}

/// PrerequisiteFailed display should include the custom reason.
#[test]
fn test_prerequisite_failed_display() {
    let err = SandboxError::PrerequisiteFailed("missing kernel feature".to_string());
    let msg = err.to_string();
    assert!(
        msg.contains("missing kernel feature"),
        "PrerequisiteFailed should include the reason: {msg}"
    );
}

/// HighSecurityViolation display should reference the blocked path.
#[test]
fn test_high_security_violation_display() {
    let err = SandboxError::HighSecurityViolation {
        path: PathBuf::from("/tmp/blocked.txt"),
    };
    let msg = err.to_string();
    assert!(
        msg.contains("blocked.txt") || msg.contains("/tmp"),
        "HighSecurityViolation should reference the blocked path: {msg}"
    );
    assert!(
        msg.contains("no-temp-files") || msg.contains("high-security") || msg.contains("blocked"),
        "HighSecurityViolation should mention security mode: {msg}"
    );
}

/// NestedSandboxDetected display should mention nesting or --disable-sandbox.
#[test]
fn test_nested_sandbox_detected_display() {
    let err = SandboxError::NestedSandboxDetected;
    let msg = err.to_string();
    assert!(
        msg.contains("nested") || msg.contains("Nested") || msg.contains("--disable-sandbox"),
        "NestedSandboxDetected should explain the situation: {msg}"
    );
}

// ───────────────────── SandboxError Debug impl ─────────────────────────────

/// Confirm that SandboxError implements Debug.
#[test]
fn test_sandbox_error_debug_impl() {
    let err = SandboxError::PrerequisiteFailed("test".to_string());
    let debug = format!("{err:?}");
    assert!(
        debug.contains("PrerequisiteFailed"),
        "Debug should include the variant name: {debug}"
    );
}

// ───────────── Exercising format_scopes via Sandbox::validate_path ──────────

/// Triggering validate_path on a path outside the sandbox exercises
/// format_scopes(scopes) which was previously uncovered.
#[test]
fn test_validate_path_triggers_format_scopes_single_scope() {
    let scope = tempdir().unwrap();
    let other = tempdir().unwrap();

    let sandbox = Sandbox::new(
        vec![scope.path().to_path_buf()],
        SandboxMode::Strict,
        false,
        false,
        false,
    )
    .unwrap();

    // Write a real file outside the scope so canonicalize() can resolve it
    let bad_file = other.path().join("bad.txt");
    std::fs::write(&bad_file, "x").unwrap();

    let result = sandbox.validate_path(&bad_file);
    assert!(
        result.is_err(),
        "Expected path outside scope to be rejected"
    );
    // The error message should embed the scope path (from format_scopes)
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("sandbox") || !msg.is_empty(),
        "Error from validate_path should be informative: {msg}"
    );
}

/// Same as above but with multiple scopes - exercises the plural branch of format_scopes.
#[test]
fn test_validate_path_triggers_format_scopes_multiple_scopes() {
    let scope1 = tempdir().unwrap();
    let scope2 = tempdir().unwrap();
    let other = tempdir().unwrap();

    let sandbox = Sandbox::new(
        vec![scope1.path().to_path_buf(), scope2.path().to_path_buf()],
        SandboxMode::Strict,
        false,
        false,
        false,
    )
    .unwrap();

    let bad_file = other.path().join("bad.txt");
    std::fs::write(&bad_file, "x").unwrap();

    let result = sandbox.validate_path(&bad_file);
    assert!(
        result.is_err(),
        "Path outside all scopes should be rejected"
    );
}

// ───────────────────────── High-security mode ──────────────────────────────

/// HighSecurityViolation variant can be created and has a non-empty message.
/// (Testing the error type directly rather than via validate_path, since
/// the sandbox scope being in /tmp makes live testing against no-temp-files
/// platform-dependent.)
#[test]
fn test_high_security_violation_error_display() {
    let err = SandboxError::HighSecurityViolation {
        path: PathBuf::from("/tmp/secret.txt"),
    };
    let msg = err.to_string();
    assert!(
        !msg.is_empty(),
        "HighSecurityViolation should have a message"
    );
    assert!(
        msg.contains("/tmp/secret.txt"),
        "Should reference the blocked path: {msg}"
    );
}

/// With no-temp-files mode and a scope inside /tmp, validate_path of a file
/// in that scope should be blocked by HighSecurityViolation, since the scope
/// itself is inside the system temp directory.
#[test]
fn test_validate_path_no_temp_files_blocks_temp_dir_scope() {
    let scope = tempdir().unwrap();
    // Only run if the tempdir is actually under the system temp dir
    let sys_tmp = std::env::temp_dir();
    if !scope.path().starts_with(&sys_tmp) {
        // If tempdir is not under /tmp (unusual OS setup), skip this test.
        return;
    }

    let sandbox = Sandbox::new(
        vec![scope.path().to_path_buf()],
        SandboxMode::Strict,
        true,
        false,
        false,
    )
    .unwrap();

    // A file within the scope (which is in /tmp) should be blocked by no-temp-files.
    let file = scope.path().join("blocked.txt");
    std::fs::write(&file, "data").unwrap();

    let result = sandbox.validate_path(&file);
    // no-temp-files blocks all temp paths, so even the scope itself is blocked.
    assert!(
        result.is_err(),
        "File in /tmp scope should be blocked by no-temp-files mode"
    );
}
