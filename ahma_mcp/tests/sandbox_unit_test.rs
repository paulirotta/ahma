//! Unit tests for sandbox.rs - Kernel-level sandboxing
//!
//! These tests cover:
//! - Path validation and normalization
//! - Sandbox scope initialization
//! - SandboxError formatting
//! - Platform-specific sandbox checks

use ahma_mcp::sandbox::{Sandbox, SandboxMode, normalize_path_lexically};
use std::path::{Path, PathBuf};
use tempfile::tempdir;

#[test]
fn test_normalize_path_removes_single_dot() {
    let path = Path::new("/home/user/./project");
    let normalized = normalize_path_lexically(path);
    assert_eq!(normalized, PathBuf::from("/home/user/project"));
}

#[test]
fn test_normalize_path_resolves_parent_dir() {
    let path = Path::new("/home/user/project/../other");
    let normalized = normalize_path_lexically(path);
    assert_eq!(normalized, PathBuf::from("/home/user/other"));
}

#[test]
fn test_normalize_path_multiple_parent_dirs() {
    let path = Path::new("/home/user/a/b/c/../../d");
    let normalized = normalize_path_lexically(path);
    assert_eq!(normalized, PathBuf::from("/home/user/a/d"));
}

// ============= Test Mode Detection Tests =============

#[test]
fn test_is_test_mode_on_sandbox_instance() {
    let sandbox = Sandbox::new_test();
    assert!(sandbox.is_test_mode());

    let temp = tempdir().unwrap();
    let sandbox_strict = Sandbox::new(
        vec![temp.path().to_path_buf()],
        SandboxMode::Strict,
        false,
        false,
    )
    .unwrap();
    assert!(!sandbox_strict.is_test_mode());
}

#[test]
fn test_is_no_temp_files_on_sandbox_instance() {
    let temp = tempdir().unwrap();
    let sandbox = Sandbox::new(
        vec![temp.path().to_path_buf()],
        SandboxMode::Strict,
        true,
        false,
    )
    .unwrap();
    assert!(sandbox.is_no_temp_files());

    let temp2 = tempdir().unwrap();
    let sandbox_default = Sandbox::new(
        vec![temp2.path().to_path_buf()],
        SandboxMode::Strict,
        false,
        false,
    )
    .unwrap();
    assert!(!sandbox_default.is_no_temp_files());
}

// ============= Path Validation Logic Tests =============

#[test]
fn test_path_validation_in_scope() {
    let temp = tempdir().unwrap();
    let scope = temp.path().to_path_buf();
    let sandbox = Sandbox::new(vec![scope.clone()], SandboxMode::Strict, false, false).unwrap();

    let valid_path = scope.join("test.txt");
    assert!(sandbox.validate_path(&valid_path).is_ok());
}

#[test]
fn test_path_validation_outside_scope() {
    let temp = tempdir().unwrap();
    let scope = temp.path().to_path_buf();
    let sandbox = Sandbox::new(vec![scope], SandboxMode::Strict, false, false).unwrap();

    let outside_path = PathBuf::from("/etc/passwd");
    assert!(sandbox.validate_path(&outside_path).is_err());
}

#[cfg(unix)]
#[test]
fn test_path_validation_accepts_symlink_alias_scope_for_nonexistent_nested_path() {
    let temp = tempdir().unwrap();
    let real_root = temp.path().join("real_root");
    std::fs::create_dir_all(&real_root).unwrap();

    let alias_root = temp.path().join("alias_root");
    std::os::unix::fs::symlink(&real_root, &alias_root).unwrap();

    let sandbox =
        Sandbox::new(vec![alias_root.clone()], SandboxMode::Strict, false, false).unwrap();

    // `nested` does not exist, so path resolution falls back to lexical normalization.
    // The alias scope must still be accepted.
    let alias_nested_target = alias_root.join("nested/new_file.txt");
    assert!(sandbox.validate_path(&alias_nested_target).is_ok());
}
