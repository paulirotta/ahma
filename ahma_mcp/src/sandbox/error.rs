use std::path::PathBuf;

/// Errors specific to sandbox operations
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error(
        "Path '{path:?}' is outside the sandbox root{} (this usually means your MCP session is scoped to a different workspace root; reconnect from the intended workspace or use a multi-root workspace)",
        format_scopes(.scopes)
    )]
    PathOutsideSandbox { path: PathBuf, scopes: Vec<PathBuf> },

    #[error(
        "Landlock is not available on this system (requires Linux kernel 5.13+ with Landlock LSM enabled). To run without sandboxing, add --no-sandbox to your mcp.json tool definition. Example: \"args\": [\"--mode\", \"stdio\", \"--no-sandbox\"]"
    )]
    LandlockNotAvailable,

    #[error(
        "macOS sandbox-exec is not available. To run without sandboxing, add --no-sandbox to your mcp.json tool definition. Example: \"args\": [\"--mode\", \"stdio\", \"--no-sandbox\"]"
    )]
    MacOSSandboxNotAvailable,

    #[error("Unsupported operating system: {0}")]
    UnsupportedOs(String),

    #[error("Failed to canonicalize path '{path:?}': {reason}")]
    CanonicalizationFailed { path: PathBuf, reason: String },

    #[error("Sandbox prerequisite check failed: {0}")]
    PrerequisiteFailed(String),

    #[error("Path '{path:?}' is blocked by high-security mode (no-temp-files)")]
    HighSecurityViolation { path: PathBuf },

    #[error(
        "Nested sandbox detected - running inside another sandbox (e.g., Cursor, VS Code, Docker). To override, add --no-sandbox to your mcp.json tool definition. Example: \"args\": [\"--mode\", \"stdio\", \"--no-sandbox\"]"
    )]
    NestedSandboxDetected,
}

/// Format sandbox scopes for error messages
pub(crate) fn format_scopes(scopes: &[PathBuf]) -> String {
    if scopes.is_empty() {
        " (none configured)".to_string()
    } else if scopes.len() == 1 {
        format!(" '{}'", scopes[0].display())
    } else {
        let scope_list: Vec<String> = scopes
            .iter()
            .map(|s| format!("'{}'", s.display()))
            .collect();
        format!("s [{}]", scope_list.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── format_scopes ────────────────────────────────────────────────────────

    #[test]
    fn test_format_scopes_empty() {
        let result = format_scopes(&[]);
        assert_eq!(result, " (none configured)");
    }

    #[test]
    fn test_format_scopes_single_scope() {
        let scopes = vec![PathBuf::from("/allowed/dir")];
        let result = format_scopes(&scopes);
        assert_eq!(result, " '/allowed/dir'");
    }

    #[test]
    fn test_format_scopes_multiple_scopes() {
        let scopes = vec![PathBuf::from("/scope/one"), PathBuf::from("/scope/two")];
        let result = format_scopes(&scopes);
        // Should start with "s [" (plural form)
        assert!(
            result.starts_with("s ["),
            "Multiple scopes should use plural: {result}"
        );
        assert!(result.contains("/scope/one"));
        assert!(result.contains("/scope/two"));
    }

    #[test]
    fn test_format_scopes_three_scopes() {
        let scopes = vec![
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            PathBuf::from("/c"),
        ];
        let result = format_scopes(&scopes);
        assert!(result.contains("/a"));
        assert!(result.contains("/b"));
        assert!(result.contains("/c"));
    }

    // ── SandboxError variants ────────────────────────────────────────────────

    #[test]
    fn test_path_outside_sandbox_empty_scopes() {
        let err = SandboxError::PathOutsideSandbox {
            path: PathBuf::from("/bad/path"),
            scopes: vec![],
        };
        let msg = err.to_string();
        assert!(
            msg.contains("none configured"),
            "Should note no scopes: {msg}"
        );
        assert!(msg.contains("/bad/path"), "Should mention the path: {msg}");
    }

    #[test]
    fn test_path_outside_sandbox_single_scope() {
        let err = SandboxError::PathOutsideSandbox {
            path: PathBuf::from("/bad"),
            scopes: vec![PathBuf::from("/allowed")],
        };
        let msg = err.to_string();
        assert!(msg.contains("/allowed"), "Should list the scope: {msg}");
    }

    #[test]
    fn test_path_outside_sandbox_multiple_scopes() {
        let err = SandboxError::PathOutsideSandbox {
            path: PathBuf::from("/bad"),
            scopes: vec![PathBuf::from("/s1"), PathBuf::from("/s2")],
        };
        let msg = err.to_string();
        assert!(
            msg.contains("/s1") && msg.contains("/s2"),
            "Should list all scopes: {msg}"
        );
    }

    #[test]
    fn test_landlock_not_available_display() {
        let err = SandboxError::LandlockNotAvailable;
        let msg = err.to_string();
        assert!(
            msg.contains("Landlock") && msg.contains("--no-sandbox"),
            "Should instruct to use --no-sandbox: {msg}"
        );
    }

    #[test]
    fn test_macos_sandbox_not_available_display() {
        let err = SandboxError::MacOSSandboxNotAvailable;
        let msg = err.to_string();
        assert!(
            msg.contains("sandbox-exec") && msg.contains("--no-sandbox"),
            "Should instruct to use --no-sandbox: {msg}"
        );
    }

    #[test]
    fn test_unsupported_os_includes_os_name() {
        let err = SandboxError::UnsupportedOs("haiku".to_string());
        let msg = err.to_string();
        assert!(msg.contains("haiku"), "Should include OS name: {msg}");
    }

    #[test]
    fn test_canonicalization_failed_includes_path_and_reason() {
        let err = SandboxError::CanonicalizationFailed {
            path: PathBuf::from("/some/path"),
            reason: "no such file".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("/some/path"), "Should contain path: {msg}");
        assert!(msg.contains("no such file"), "Should contain reason: {msg}");
    }

    #[test]
    fn test_prerequisite_failed_includes_reason() {
        let err = SandboxError::PrerequisiteFailed("kernel too old".to_string());
        let msg = err.to_string();
        assert!(
            msg.contains("kernel too old"),
            "Should contain reason: {msg}"
        );
    }

    #[test]
    fn test_high_security_violation_includes_path() {
        let err = SandboxError::HighSecurityViolation {
            path: PathBuf::from("/tmp/secret.txt"),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("/tmp/secret.txt"),
            "Should reference blocked path: {msg}"
        );
        assert!(
            msg.contains("no-temp-files")
                || msg.contains("high-security")
                || msg.contains("blocked"),
            "Should mention high-security mode: {msg}"
        );
    }

    #[test]
    fn test_nested_sandbox_detected_display() {
        let err = SandboxError::NestedSandboxDetected;
        let msg = err.to_string();
        assert!(
            (msg.contains("Nested") || msg.contains("nested")) && msg.contains("--no-sandbox"),
            "Should mention nesting and --no-sandbox: {msg}"
        );
    }

    #[test]
    fn test_sandbox_error_debug_is_non_empty() {
        let err = SandboxError::LandlockNotAvailable;
        let debug = format!("{err:?}");
        assert!(debug.contains("LandlockNotAvailable"));
    }
}
