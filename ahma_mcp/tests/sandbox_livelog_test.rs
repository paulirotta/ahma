//! Tests for livelog symlink resolution in Sandbox::new().
//!
//! When `livelog = true`, the sandbox looks for `<scope>/log/*.log` symlinks
//! and resolves their targets as read-only scopes to allow live log monitoring
//! of tracing-appender rotated logs.

#[cfg(unix)]
mod unix_tests {
    use std::os::unix::fs::symlink;

    use ahma_mcp::sandbox::{Sandbox, SandboxMode};
    use tempfile::tempdir;

    #[test]
    fn test_livelog_resolves_log_symlink() {
        let temp = tempdir().unwrap();
        let scope = temp.path().to_path_buf();

        // Create log/ dir with a symlink to a .log file
        let log_dir = scope.join("log");
        std::fs::create_dir_all(&log_dir).unwrap();

        let actual_log = temp.path().join("real.log");
        std::fs::write(&actual_log, "log content").unwrap();

        symlink(&actual_log, log_dir.join("current.log")).unwrap();

        let sandbox = Sandbox::new(vec![scope], SandboxMode::Strict, false, true).unwrap();
        let read_scopes = sandbox.read_scopes();
        assert!(
            !read_scopes.is_empty(),
            "should resolve symlink to .log file"
        );

        let resolved = dunce::canonicalize(&actual_log).unwrap();
        assert!(
            read_scopes.contains(&resolved),
            "read_scopes should contain resolved symlink target"
        );
    }

    #[test]
    fn test_livelog_ignores_non_log_symlinks() {
        let temp = tempdir().unwrap();
        let scope = temp.path().to_path_buf();
        let log_dir = scope.join("log");
        std::fs::create_dir_all(&log_dir).unwrap();

        let actual_file = temp.path().join("data.txt");
        std::fs::write(&actual_file, "not a log").unwrap();

        symlink(&actual_file, log_dir.join("data.txt")).unwrap();

        let sandbox = Sandbox::new(vec![scope], SandboxMode::Strict, false, true).unwrap();
        assert!(
            sandbox.read_scopes().is_empty(),
            "should ignore non-.log symlinks"
        );
    }

    #[test]
    fn test_livelog_ignores_regular_log_files() {
        let temp = tempdir().unwrap();
        let scope = temp.path().to_path_buf();
        let log_dir = scope.join("log");
        std::fs::create_dir_all(&log_dir).unwrap();

        // Regular file (not a symlink) — should be ignored
        std::fs::write(log_dir.join("app.log"), "log line").unwrap();

        let sandbox = Sandbox::new(vec![scope], SandboxMode::Strict, false, true).unwrap();
        assert!(
            sandbox.read_scopes().is_empty(),
            "should ignore regular .log files (non-symlink)"
        );
    }

    #[test]
    fn test_livelog_ignores_broken_symlinks() {
        let temp = tempdir().unwrap();
        let scope = temp.path().to_path_buf();
        let log_dir = scope.join("log");
        std::fs::create_dir_all(&log_dir).unwrap();

        // Symlink to non-existent file
        symlink("/nonexistent/path.log", log_dir.join("broken.log")).unwrap();

        let sandbox = Sandbox::new(vec![scope], SandboxMode::Strict, false, true).unwrap();
        assert!(
            sandbox.read_scopes().is_empty(),
            "should ignore broken symlinks"
        );
    }

    #[test]
    fn test_livelog_ignores_symlink_to_directory() {
        let temp = tempdir().unwrap();
        let scope = temp.path().to_path_buf();
        let log_dir = scope.join("log");
        std::fs::create_dir_all(&log_dir).unwrap();

        let dir_target = temp.path().join("some_dir");
        std::fs::create_dir_all(&dir_target).unwrap();

        symlink(&dir_target, log_dir.join("dir.log")).unwrap();

        let sandbox = Sandbox::new(vec![scope], SandboxMode::Strict, false, true).unwrap();
        assert!(
            sandbox.read_scopes().is_empty(),
            "should ignore symlinks to directories"
        );
    }

    #[test]
    fn test_livelog_no_log_directory() {
        let temp = tempdir().unwrap();
        let scope = temp.path().to_path_buf();

        // No log/ directory at all
        let sandbox = Sandbox::new(vec![scope], SandboxMode::Strict, false, true).unwrap();
        assert!(
            sandbox.read_scopes().is_empty(),
            "should return empty when no log/ directory exists"
        );
    }

    #[test]
    fn test_livelog_empty_log_directory() {
        let temp = tempdir().unwrap();
        let scope = temp.path().to_path_buf();
        let log_dir = scope.join("log");
        std::fs::create_dir_all(&log_dir).unwrap();

        // log/ exists but has no symlinks
        let sandbox = Sandbox::new(vec![scope], SandboxMode::Strict, false, true).unwrap();
        assert!(
            sandbox.read_scopes().is_empty(),
            "should return empty when log/ has no symlinks"
        );
    }

    #[test]
    fn test_livelog_disabled_produces_no_read_scopes() {
        let temp = tempdir().unwrap();
        let scope = temp.path().to_path_buf();
        let log_dir = scope.join("log");
        std::fs::create_dir_all(&log_dir).unwrap();

        let actual_log = temp.path().join("real.log");
        std::fs::write(&actual_log, "content").unwrap();
        symlink(&actual_log, log_dir.join("current.log")).unwrap();

        // livelog = false
        let sandbox = Sandbox::new(vec![scope], SandboxMode::Strict, false, false).unwrap();
        assert!(
            sandbox.read_scopes().is_empty(),
            "livelog=false should not resolve any read scopes"
        );
    }
}
