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

fn find_first_existing_path<I>(candidates: I) -> PathBuf
where
    I: IntoIterator<Item = PathBuf>,
{
    candidates
        .into_iter()
        .find(|candidate| candidate.exists())
        .unwrap_or_default()
}

fn check_env_binary() -> Option<PathBuf> {
    let path_str = std::env::var("AHMA_TEST_BINARY").ok()?;
    let path = PathBuf::from(&path_str);
    if path.exists() {
        return Some(path);
    }
    eprintln!(
        "Warning: AHMA_TEST_BINARY={} does not exist, falling back",
        path_str
    );
    None
}

fn collect_binary_candidates(workspace: &Path, bin_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::with_capacity(3);
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        candidates.push(PathBuf::from(target_dir).join("debug").join(bin_name));
    }
    candidates.push(workspace.join("target/debug").join(bin_name));
    candidates.push(workspace.join("target/release").join(bin_name));
    candidates
}

/// Get the path to the ahma_mcp binary.
fn get_test_binary_path() -> PathBuf {
    BINARY_PATH
        .get_or_init(|| {
            if let Some(path) = check_env_binary() {
                return path;
            }
            let workspace = get_workspace_dir();
            let bin_name = format!("ahma-mcp{}", std::env::consts::EXE_SUFFIX);
            find_first_existing_path(collect_binary_candidates(&workspace, &bin_name))
        })
        .clone()
}

fn build_cargo_run_command(workspace_dir: &Path) -> Command {
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("--manifest-path")
        .arg(workspace_dir.join("Cargo.toml"))
        .arg("--package")
        .arg("ahma_mcp")
        .arg("--bin")
        .arg("ahma-mcp")
        .arg("--");
    cmd
}

fn use_prebuilt_binary() -> bool {
    let path = get_test_binary_path();
    !path.as_os_str().is_empty() && path.exists()
}

fn configure_sandbox_env(
    cmd: &mut Command,
    force_no_sandbox: bool,
    working_dir: &Path,
    livelog: bool,
) {
    if force_no_sandbox {
        cmd.env("AHMA_DISABLE_SANDBOX", "1");
    } else {
        if let Some(scope) = working_dir.to_str() {
            cmd.env("AHMA_SANDBOX_SCOPE", scope);
        }
        if livelog {
            cmd.env("AHMA_LOG_MONITOR", "1");
        }
    }
}

fn configure_tools_dir_env(
    cmd: &mut Command,
    tools_dir: Option<PathBuf>,
    working_dir: &Path,
) {
    let Some(dir) = tools_dir else { return };
    let tools_path = if dir.is_absolute() {
        dir
    } else {
        working_dir.join(dir)
    };
    cmd.env("AHMA_TOOLS_DIR", tools_path);
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

        let command = if use_prebuilt_binary() {
            Command::new(get_test_binary_path())
        } else {
            eprintln!(
                "Warning: Using slow 'cargo run' path. Run 'cargo build' first for faster tests."
            );
            build_cargo_run_command(&workspace_dir)
        };

        let fut = self.run_command(command, &working_dir);

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
        // inside the child process and cause an immediate exit.  Force no-sandbox so the
        // child can start; application-level path checks (path_security.rs) still enforce bounds.
        let ClientBuilder {
            tools_dir,
            extra_args,
            extra_env,
            no_sandbox,
            livelog,
            skip_availability_probes,
            ..
        } = self;
        let force_no_sandbox = no_sandbox || is_nested_sandbox_environment();

        ().serve(TokioChildProcess::new(command.configure(|cmd| {
            cmd.args(["serve", "stdio"]);

            // Clear sandbox-related env vars inherited from a parent ahma process to avoid
            // bypassing path validation (security invariant violation).
            cmd.env_remove("AHMA_SANDBOX_DEFER");
            cmd.env_remove("AHMA_SANDBOX_SCOPE");
            cmd.env_remove("AHMA_WORKING_DIRS");

            configure_sandbox_env(cmd, force_no_sandbox, working_dir, livelog);

            if skip_availability_probes {
                cmd.env("AHMA_SKIP_PROBES", "1");
            }

            cmd.current_dir(working_dir).kill_on_drop(true);

            for (k, v) in extra_env {
                cmd.env(k, v);
            }

            configure_tools_dir_env(cmd, tools_dir, working_dir);

            cmd.args(extra_args);
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
/// the `AHMA_DISABLE_SANDBOX` env var as a convention for nested callers.
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
        .start_process_with_args(Some(tools_dir.to_str().unwrap()), &[])
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
        max_total_shells: config.max_concurrent_tasks,
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
