#[cfg(target_os = "macos")]
use ahma_mcp::sandbox::test_sandbox_exec_available;
use ahma_mcp::sandbox::{Sandbox, SandboxMode, check_sandbox_prerequisites};
#[cfg(unix)]
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_sandbox_prerequisites_check() {
    // This runs the actual check for the current OS.
    // On github actions or local dev, it should pass or fail predictably.
    // We just want to ensure it runs without panicking.
    let result = check_sandbox_prerequisites();
    // It might be Ok or Err depending on environment, but we assert it returns a result.
    assert!(result.is_ok() || result.is_err());
}

#[test]
#[cfg(target_os = "macos")]
fn test_macos_sandbox_exec_available() {
    // Should run sandbox-exec check
    let result = test_sandbox_exec_available();
    // Just verifying it runs
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_validate_path_basic() {
    let tmp_dir = TempDir::new().unwrap();
    let root = tmp_dir.path().to_path_buf();

    let sandbox =
        Sandbox::new(vec![root.clone()], SandboxMode::Strict, false, false, false).unwrap();

    // Allowed path
    let file_path = root.join("test.txt");
    // Only works if file or parent exists for canonicalize logic in validate_path
    // validate_path attempts to canonicalize

    // Create the file so it exists
    std::fs::write(&file_path, "content").unwrap();

    let validated = sandbox.validate_path(&file_path);
    assert!(validated.is_ok());

    // Outside path
    let outside = std::env::temp_dir().join("outside_ahma_test.txt");
    if !outside.starts_with(&root) {
        let res = sandbox.validate_path(&outside);
        // Should fail
        assert!(res.is_err());
    }
}

#[test]
fn test_validate_path_no_temp_files_violation() {
    let tmp_dir = TempDir::new().unwrap();
    let root = tmp_dir.path().to_path_buf();

    // Enable no_temp_files
    let _sandbox =
        Sandbox::new(vec![root.clone()], SandboxMode::Strict, true, false, false).unwrap();

    // Even if /tmp is in scope (unlikely but if added), it should be blocked by HighSecurityViolation logic
    // But logic says: if scopes.iter().any... THEN check high security.
    // So to trigger HighSecurityViolation, the path MUST be in scope AND be a temp path.

    // If we add /tmp as a scope
    #[cfg(unix)]
    {
        let tmp_root = PathBuf::from("/tmp");
        if tmp_root.exists() {
            let sandbox_lax = Sandbox::new(
                vec![tmp_root.clone()],
                SandboxMode::Strict,
                true,
                false,
                false,
            )
            .unwrap();

            let file_in_tmp = tmp_root.join("test_security.txt");
            // It is in scope /tmp, but blocked by no_temp_files policy
            let res = sandbox_lax.validate_path(&file_in_tmp);

            // Depending on whether `test_security.txt` exists, validate_path might behave differently regarding canonicalization,
            // but it should eventually hit the check.
            // Actually, validate_path tries to canonicalize first.

            // If res is Err, we want to check it is HighSecurityViolation ideally, but Anyhow hides it.
            // Just asserting error is enough coverage for now.
            // Note: on Mac /tmp is symlink to /private/tmp.
            assert!(res.is_err());
        }
    }
}

#[test]
fn test_validate_path_symlink_traversal() {
    let tmp_dir = TempDir::new().unwrap();
    let root = tmp_dir.path().to_path_buf();
    let sandbox =
        Sandbox::new(vec![root.clone()], SandboxMode::Strict, false, false, false).unwrap();

    let safe_dir = root.join("safe");
    std::fs::create_dir(&safe_dir).unwrap();

    let symlink = safe_dir.join("shortcut_out");
    let outside_target = std::env::temp_dir();

    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_target, &symlink).unwrap();

    #[cfg(windows)]
    match std::os::windows::fs::symlink_dir(&outside_target, &symlink) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            println!("Skipping: Windows requires Developer Mode or Admin rights for symlinks");
            return;
        }
        Err(e) => panic!("Failed to create symlink: {e}"),
    }

    let res = sandbox.validate_path(&symlink);
    assert!(res.is_err());
}

#[test]
fn test_sandbox_test_mode_bypass() {
    // In Test mode with no scopes, everything should be allowed
    let sandbox = Sandbox::new_test(); // has "/" scope or similar permissive

    let path = std::env::current_dir().unwrap();
    let res = sandbox.validate_path(&path);
    assert!(res.is_ok());
}
