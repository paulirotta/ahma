use std::process::Command;

pub const SANDBOX_BYPASS_ENV_VARS: &[&str] = &[
    "NEXTEST",
    "NEXTEST_EXECUTION_MODE",
    "CARGO_TARGET_DIR",
    "RUST_TEST_THREADS",
];

pub struct SandboxTestEnv;

impl SandboxTestEnv {
    pub fn configure(cmd: &mut Command) -> &mut Command {
        for var in SANDBOX_BYPASS_ENV_VARS {
            cmd.env_remove(var);
        }
        cmd
    }

    pub fn configure_tokio(cmd: &mut tokio::process::Command) -> &mut tokio::process::Command {
        for var in SANDBOX_BYPASS_ENV_VARS {
            cmd.env_remove(var);
        }
        cmd
    }

    /// When the *current* test process is running inside a nested sandbox
    /// (e.g., `mcp_ahma_sandboxed_shell`, Cursor, VS Code, Docker), the child
    /// `ahma_mcp` binary would detect the nesting and exit before serving any
    /// requests.  This helper adds `AHMA_DISABLE_SANDBOX=1` to the command so the
    /// binary can start; application-level path security (path_security.rs) is
    /// still active in that mode.
    ///
    /// Call this **after** `configure()` on every direct binary spawn.
    pub fn apply_nested_sandbox_override(cmd: &mut Command) -> &mut Command {
        if Self::is_nested_sandbox() {
            cmd.env("AHMA_DISABLE_SANDBOX", "1");
        }
        cmd
    }

    /// Tokio-command variant of `apply_nested_sandbox_override`.
    pub fn apply_nested_sandbox_override_tokio(
        cmd: &mut tokio::process::Command,
    ) -> &mut tokio::process::Command {
        if Self::is_nested_sandbox() {
            cmd.env("AHMA_DISABLE_SANDBOX", "1");
        }
        cmd
    }

    /// Returns `true` when the current process is running inside a sandbox
    /// that would prevent child processes from applying their own OS-level
    /// sandbox (sandbox-exec / Landlock / AppContainer).
    pub fn is_nested_sandbox() -> bool {
        #[cfg(target_os = "macos")]
        {
            ahma_mcp::sandbox::test_sandbox_exec_available().is_err()
        }
        #[cfg(target_os = "linux")]
        {
            use ahma_mcp::sandbox::SandboxError;
            matches!(
                ahma_mcp::sandbox::check_sandbox_prerequisites(),
                Err(SandboxError::LandlockNotAvailable) | Err(SandboxError::PrerequisiteFailed(_))
            )
        }
        // Windows: AppContainer backend not yet implemented; always allow bypass.
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            true
        }
    }

    pub fn current_bypass_vars() -> Vec<String> {
        SANDBOX_BYPASS_ENV_VARS
            .iter()
            .filter_map(|var| {
                std::env::var(var)
                    .ok()
                    .map(|val| format!("{}={}", var, val))
            })
            .collect()
    }

    pub fn is_bypass_active() -> bool {
        SANDBOX_BYPASS_ENV_VARS
            .iter()
            .any(|var| std::env::var(var).is_ok())
    }
}
