use super::fs::get_workspace_dir;
pub use super::fs::get_workspace_tools_dir;

use crate::adapter::Adapter;
use crate::client::Client;
use crate::mcp_service::AhmaMcpService;
use crate::operation_monitor::{MonitorConfig, OperationMonitor};
use crate::shell_pool::{ShellPoolConfig, ShellPoolManager};
use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};
use anyhow::{Context, Result};
use rmcp::{
    ServiceExt,
    service::{RoleClient, RunningService},
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tempfile::{TempDir, tempdir};
use tokio::process::Command;
use tokio::sync::mpsc::{Receiver, Sender};

/// Cached path to the pre-built ahma_mcp binary.
static BINARY_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Get the path to the ahma_mcp binary.
fn get_test_binary_path() -> PathBuf {
    BINARY_PATH
        .get_or_init(|| {
            // Check env var first
            if let Ok(path) = std::env::var("AHMA_TEST_BINARY") {
                let p = PathBuf::from(&path);
                if p.exists() {
                    return p;
                }
                eprintln!(
                    "Warning: AHMA_TEST_BINARY={} does not exist, falling back",
                    path
                );
            }

            // Check for debug binary
            let workspace = get_workspace_dir();
            let bin_name = format!("ahma-mcp{}", std::env::consts::EXE_SUFFIX);

            // Check CARGO_TARGET_DIR
            if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
                let p = PathBuf::from(target_dir).join("debug").join(&bin_name);
                if p.exists() {
                    return p;
                }
            }

            let debug_binary = workspace.join("target/debug").join(&bin_name);
            if debug_binary.exists() {
                return debug_binary;
            }

            // Check for release binary
            let release_binary = workspace.join("target/release").join(&bin_name);
            if release_binary.exists() {
                return release_binary;
            }

            // No pre-built binary found - return empty path to signal fallback
            PathBuf::new()
        })
        .clone()
}

fn use_prebuilt_binary() -> bool {
    let path = get_test_binary_path();
    !path.as_os_str().is_empty() && path.exists()
}

/// Builder for creating MCP clients in tests.
#[derive(Default)]
pub struct ClientBuilder {
    tools_dir: Option<PathBuf>,
    extra_args: Vec<String>,
    extra_env: Vec<(String, String)>,
    working_dir: Option<PathBuf>,
    no_sandbox: bool,
    livelog: bool,
    skip_availability_probes: bool,
}

impl ClientBuilder {
    pub fn new() -> Self {
        Self {
            no_sandbox: true,               // Default to permissive for tests (legacy behavior)
            skip_availability_probes: true, // Skip slow availability probes in tests by default
            livelog: false,
            ..Default::default()
        }
    }

    pub fn tools_dir<P: AsRef<Path>>(mut self, path: P) -> Self {
        let path = path.as_ref();
        if path.is_absolute() {
            self.tools_dir = Some(path.to_path_buf());
        } else {
            // Resolve relative to workspace or working dir if set?
            // Existing logic resolved relative to workspace if not absolute.
            // Let's resolve it here to avoid ambiguity.
            self.tools_dir = Some(path.to_path_buf());
        }
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for arg in args {
            self.extra_args.push(arg.into());
        }
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_env.push((key.into(), value.into()));
        self
    }

    pub fn working_dir<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.working_dir = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn no_sandbox(mut self, enabled: bool) -> Self {
        self.no_sandbox = enabled;
        self
    }

    pub fn livelog(mut self, enabled: bool) -> Self {
        self.livelog = enabled;
        self
    }

    /// Control whether availability probes are skipped during server startup.
    /// Default: true (skipped) for fast test startup.
    #[allow(dead_code)]
    pub fn skip_availability_probes(mut self, skip: bool) -> Self {
        self.skip_availability_probes = skip;
        self
    }

    pub async fn build(self) -> Result<RunningService<RoleClient, ()>> {
        let workspace_dir = get_workspace_dir();
        let working_dir = self
            .working_dir
            .clone()
            .unwrap_or_else(|| workspace_dir.clone());

        let fut = if use_prebuilt_binary() {
            let binary_path = get_test_binary_path();
            self.run_command(Command::new(&binary_path), &working_dir)
        } else {
            eprintln!(
                "Warning: Using slow 'cargo run' path. Run 'cargo build' first for faster tests."
            );
            let mut cmd = Command::new("cargo");
            cmd.arg("run")
                .arg("--manifest-path")
                .arg(workspace_dir.join("Cargo.toml"))
                .arg("--package")
                .arg("ahma_mcp")
                .arg("--bin")
                .arg("ahma-mcp")
                .arg("--");
            self.run_command(cmd, &working_dir)
        };

        // Safety-net timeout: fail fast with a clear message instead of being
        // killed by nextest after 60s with no context.
        let startup_timeout = TestTimeouts::get(TimeoutCategory::ProcessSpawn);
        tokio::time::timeout(startup_timeout, fut)
            .await
            .with_context(|| {
                format!(
                    "Server failed to start within {:?} (child process startup timed out)",
                    startup_timeout
                )
            })?
    }

    async fn run_command(
        self,
        command: Command,
        working_dir: &Path,
    ) -> Result<RunningService<RoleClient, ()>> {
        // When the caller explicitly requests sandbox (`no_sandbox == false`) but the current
        // environment is a nested sandbox (Cursor, VS Code, Docker), `sandbox-exec` would fail
        // inside the child process and cause an immediate exit.  Force `--no-sandbox` so the
        // child can start; application-level path checks (path_security.rs) still enforce bounds.
        let force_no_sandbox = self.no_sandbox || is_nested_sandbox_environment();

        ().serve(TokioChildProcess::new(command.configure(|cmd| {
            if force_no_sandbox {
                cmd.arg("--no-sandbox");
            } else {
                cmd.arg("--sandbox-scope").arg(working_dir);
                if self.livelog {
                    cmd.arg("--livelog");
                }
            }
            if self.skip_availability_probes {
                cmd.arg("--skip-availability-probes");
            }
            cmd.current_dir(working_dir).kill_on_drop(true);

            for (k, v) in self.extra_env {
                cmd.env(k, v);
            }

            if let Some(dir) = self.tools_dir {
                let tools_path = if dir.is_absolute() {
                    dir
                } else {
                    // Resolve relative to working_dir
                    working_dir.join(dir)
                };
                cmd.arg("--tools-dir").arg(tools_path);
            }
            for arg in self.extra_args {
                cmd.arg(arg);
            }
        }))?)
        .await
        .context("Failed to start client service")
    }
}

/// Convenience test fixture for MCP integration tests that need:
/// 1) an isolated temporary working directory
/// 2) a tools directory under that temp root
/// 3) a connected MCP client service
pub struct McpClientFixture {
    pub temp_dir: TempDir,
    pub client: RunningService<RoleClient, ()>,
}

impl McpClientFixture {
    /// Create a fixture with a tools directory under the temp root.
    pub async fn with_tools_dir(tools_dir_name: &str) -> Result<Self> {
        let temp_dir = tempdir().context("Failed to create temp dir for MCP fixture")?;
        let tools_dir = temp_dir.path().join(tools_dir_name);
        tokio::fs::create_dir_all(&tools_dir)
            .await
            .with_context(|| {
                format!(
                    "Failed to create MCP fixture tools directory: {}",
                    tools_dir.display()
                )
            })?;

        let client = ClientBuilder::new()
            .tools_dir(tools_dir_name)
            .working_dir(temp_dir.path())
            .build()
            .await?;

        Ok(Self { temp_dir, client })
    }

    pub fn working_dir(&self) -> &Path {
        self.temp_dir.path()
    }
}

/// Returns `true` when the current process is running inside a sandbox environment
/// (e.g., Cursor, VS Code, Docker, or ahma's own `sandboxed_shell`) that would
/// prevent the child MCP server from applying its own OS-level sandbox.
///
/// On macOS this probes `sandbox-exec` directly; on other platforms we check for
/// the `AHMA_NO_SANDBOX` env var as a convention for nested callers.
#[cfg(target_os = "macos")]
fn is_nested_sandbox_environment() -> bool {
    ahma_mcp_internal_sandbox_probe()
}

#[cfg(not(target_os = "macos"))]
fn is_nested_sandbox_environment() -> bool {
    // On Linux with Landlock unavailable, check sandbox prerequisites.
    #[cfg(target_os = "linux")]
    {
        use crate::sandbox::SandboxError;
        matches!(
            crate::sandbox::check_sandbox_prerequisites(),
            Err(SandboxError::LandlockNotAvailable) | Err(SandboxError::PrerequisiteFailed(_))
        )
    }
    // On Windows (sandbox not yet implemented) or any other platform, always
    // force no-sandbox so tests can run.
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// Probe for the macOS nested sandbox condition without importing the sandbox module
/// (avoids circular dependency in test-utils).  Calls `sandbox-exec` with a
/// trivial allow-all profile; if it returns exit 71 or fails, we're nested.
#[cfg(target_os = "macos")]
fn ahma_mcp_internal_sandbox_probe() -> bool {
    use crate::sandbox::test_sandbox_exec_available;
    test_sandbox_exec_available().is_err()
}

// Backward compatibility wrappers

pub async fn setup_mcp_service_with_client() -> Result<(TempDir, Client)> {
    // Create a temporary directory for tool configs
    // sandboxed_shell is a core built-in tool, no JSON config needed
    let temp_dir = tempfile::tempdir()?;
    let tools_dir = temp_dir.path();

    let mut client = Client::new();
    client
        .start_process_with_args(Some(tools_dir.to_str().unwrap()), &["--no-sandbox"])
        .await?;

    // Give the server a moment to start
    tokio::time::sleep(Duration::from_millis(
        crate::constants::SEQUENCE_STEP_DELAY_MS,
    ))
    .await;

    Ok((temp_dir, client))
}

pub async fn setup_test_environment() -> (AhmaMcpService, TempDir) {
    let temp_dir = tempdir().unwrap();
    let tools_dir = temp_dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).unwrap();

    let config = super::config::default_config();
    let monitor_config = MonitorConfig::with_timeout(config.default_timeout);
    let monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_pool_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_pool_config));
    let sandbox = Arc::new(crate::sandbox::Sandbox::new_test());
    let adapter = Arc::new(Adapter::new(monitor.clone(), shell_pool, sandbox).unwrap());

    // Create empty configs and guidance for the new API
    let configs = Arc::new(HashMap::new());
    let guidance = Arc::new(None);

    let service = AhmaMcpService::new(adapter, monitor, configs, guidance, false, false, false)
        .await
        .unwrap();

    (service, temp_dir)
}

#[allow(dead_code)]
pub async fn setup_test_environment_with_io()
-> (AhmaMcpService, Sender<String>, Receiver<String>, TempDir) {
    let temp_dir = tempdir().unwrap();
    let tools_dir = temp_dir.path().join("tools");
    // Use Tokio's async filesystem API so we don't block the runtime
    tokio::fs::create_dir_all(&tools_dir).await.unwrap();

    let config = super::config::default_config();
    let monitor_config = MonitorConfig::with_timeout(config.default_timeout);
    let monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_pool_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_pool_config));
    let sandbox = Arc::new(crate::sandbox::Sandbox::new_test());
    let adapter = Arc::new(Adapter::new(monitor.clone(), shell_pool, sandbox).unwrap());

    // Create empty configs and guidance for the new API
    let configs = Arc::new(HashMap::new());
    let guidance = Arc::new(None);

    let service = AhmaMcpService::new(adapter, monitor, configs, guidance, false, false, false)
        .await
        .unwrap();

    let (input_tx, output_rx) = tokio::sync::mpsc::channel(100);
    (service, input_tx, output_rx, temp_dir)
}

/// Create a test config for integration tests
#[allow(dead_code)]
pub fn create_test_config(_workspace_dir: &Path) -> Result<Arc<Adapter>> {
    let config = super::config::default_config();
    // Create test monitor and shell pool configurations
    let monitor_config = MonitorConfig::with_timeout(config.default_timeout);
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));

    let shell_pool_config = ShellPoolConfig {
        enabled: true,
        shells_per_directory: 2,
        max_total_shells: config.max_concurrent_tasks as usize,
        shell_idle_timeout: Duration::from_secs(1800),
        pool_cleanup_interval: Duration::from_secs(300),
        shell_spawn_timeout: config.quick_timeout,
        command_timeout: config.default_timeout,
        health_check_interval: Duration::from_secs(60),
    };
    let shell_pool_manager = Arc::new(ShellPoolManager::new(shell_pool_config));

    // Create a test sandbox
    let sandbox = Arc::new(crate::sandbox::Sandbox::new_test());

    Adapter::new(operation_monitor, shell_pool_manager, sandbox).map(Arc::new)
}
