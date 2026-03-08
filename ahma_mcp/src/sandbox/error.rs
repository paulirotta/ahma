use std::path::PathBuf;

/// Errors specific to sandbox operations
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error(
        "Path '{path:?}' is outside the sandbox root{} (this usually means your MCP session is scoped to a different workspace root; reconnect from the intended workspace or use a multi-root workspace)",
        format_scopes(.scopes)
    )]
    PathOutsideSandbox { path: PathBuf, scopes: Vec<PathBuf> },

    #[error("Landlock is not available on this system (requires Linux kernel 5.13+ with Landlock LSM enabled). To run without sandboxing, add --no-sandbox to your mcp.json tool definition. Example: \"args\": [\"--mode\", \"stdio\", \"--no-sandbox\"]")]
    LandlockNotAvailable,

    #[error("macOS sandbox-exec is not available. To run without sandboxing, add --no-sandbox to your mcp.json tool definition. Example: \"args\": [\"--mode\", \"stdio\", \"--no-sandbox\"]")]
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
