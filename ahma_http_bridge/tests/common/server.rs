use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};
use reqwest::Client;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::time::sleep;

use super::sandbox_env::SandboxTestEnv;

/// A running test server instance with dynamic port.
pub struct TestServerInstance {
    child: Child,
    port: u16,
    _temp_dir: TempDir,
}

impl TestServerInstance {
    /// Get the base URL for this server.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Get the port this server is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for TestServerInstance {
    fn drop(&mut self) {
        eprintln!(
            "[TestServer] Shutting down test server on port {}",
            self.port
        );
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// RAII guard for a raw `Child` server process.
///
/// Kills the child and reaps it on drop to prevent leaked (zombie) processes
/// when test assertions panic before manual cleanup.
pub struct ServerGuard {
    child: Option<Child>,
    port: u16,
}

impl ServerGuard {
    /// Wrap a raw child process and its port in an RAII guard.
    pub fn new(child: Child, port: u16) -> Self {
        Self {
            child: Some(child),
            port,
        }
    }

    /// Get the port this server is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get the base URL for this server.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn workspace_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get workspace dir")
        .to_path_buf()
}

pub fn resolve_binary_path() -> PathBuf {
    static BINARY_LOG_ONCE: std::sync::Once = std::sync::Once::new();

    let debug_bin = ahma_mcp::test_utils::cli::get_binary_path("ahma-mcp", "ahma-mcp");
    // Construct sibling binary paths with the correct platform executable extension.
    let exe_ext = if cfg!(windows) { ".exe" } else { "" };
    let bin_name = format!("ahma-mcp{exe_ext}");
    let release_bin = debug_bin
        .parent()
        .and_then(|p| p.parent())
        .map(|target| target.join("release").join(&bin_name));
    let llvm_cov_debug_bin = debug_bin
        .parent()
        .and_then(|p| p.parent())
        .map(|target| target.join("llvm-cov-target/debug").join(&bin_name));
    let llvm_cov_release_bin = debug_bin
        .parent()
        .and_then(|p| p.parent())
        .map(|target| target.join("llvm-cov-target/release").join(&bin_name));

    let mut candidates = vec![debug_bin];
    if let Some(path) = release_bin {
        candidates.push(path);
    }
    if let Some(path) = llvm_cov_debug_bin {
        candidates.push(path);
    }
    if let Some(path) = llvm_cov_release_bin {
        candidates.push(path);
    }

    let binary_path = candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| {
            panic!(
                "\n\
                 FAIL ahma-mcp binary NOT FOUND in target directory.\n\n\
                 The integration tests require the server binary to be built first.\n\
                 Please run: cargo build --package ahma-mcp --bin ahma-mcp\n\n\
                 Looked in: {:?}\n",
                ahma_mcp::test_utils::cli::get_binary_path("ahma-mcp", "ahma-mcp")
                    .parent()
                    .and_then(|p| p.parent())
            )
        });

    BINARY_LOG_ONCE.call_once(|| {
        eprintln!(
            "[TestServer] Using ahma-mcp binary: {}",
            binary_path.display()
        );
    });

    binary_path
}

fn build_server_args(handshake_timeout_secs: Option<u64>) -> Vec<String> {
    let workspace = workspace_dir();
    let tools_dir = workspace.join(".ahma");

    let mut args = vec![
        "--mode".to_string(),
        "http".to_string(),
        "--http-port".to_string(),
        "0".to_string(),
        "--sync".to_string(),
        "--tools-dir".to_string(),
        tools_dir.to_string_lossy().to_string(),
        "--sandbox-scope".to_string(),
        String::new(),
        "--log-to-stderr".to_string(),
    ];

    if let Some(timeout) = handshake_timeout_secs {
        args.push("--handshake-timeout-secs".to_string());
        args.push(timeout.to_string());
    }

    if should_force_no_sandbox_for_test_server() {
        args.push("--no-sandbox".to_string());
    }

    args
}

#[cfg(target_os = "linux")]
fn should_force_no_sandbox_for_test_server() -> bool {
    use ahma_mcp::sandbox::SandboxError;

    matches!(
        ahma_mcp::sandbox::check_sandbox_prerequisites(),
        Err(SandboxError::LandlockNotAvailable) | Err(SandboxError::PrerequisiteFailed(_))
    )
}

/// On macOS, spawn the test server without `--no-sandbox` only when
/// `sandbox-exec` is known to work in the current environment.  When running
/// inside a nested sandbox (Cursor, VS Code, Docker) `sandbox-exec` returns
/// exit 71 / "Operation not permitted", so we fall back to `--no-sandbox` so
/// the integration tests can still exercise the server code.
#[cfg(target_os = "macos")]
fn should_force_no_sandbox_for_test_server() -> bool {
    ahma_mcp::sandbox::test_sandbox_exec_available().is_err()
}

/// On Windows, only skip sandbox if the OS version is too old (Windows 7 or older)
/// or if sandbox creation is otherwise unavailable.
#[cfg(target_os = "windows")]
fn should_force_no_sandbox_for_test_server() -> bool {
    ahma_mcp::sandbox::check_windows_sandbox_available().is_err()
}

/// On any other unsupported platform, skip sandbox to let tests run.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn should_force_no_sandbox_for_test_server() -> bool {
    true
}

fn wire_output_reader<R: std::io::Read + Send + 'static>(reader: R, sender: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines().map_while(Result::ok) {
            let _ = sender.send(line);
        }
    });
}

fn wait_for_bound_port(receiver: &mpsc::Receiver<String>, timeout: Duration) -> Option<u16> {
    let start = Instant::now();
    while start.elapsed() <= timeout {
        match receiver.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                eprintln!("{}", line);
                if let Some(idx) = line.find("AHMA_BOUND_PORT=") {
                    let port_str = &line[idx + "AHMA_BOUND_PORT=".len()..];
                    if let Ok(port) = port_str.trim().parse::<u16>() {
                        return Some(port);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    None
}

async fn wait_for_health(port: u16) -> bool {
    let client = Client::new();
    let health_url = format!("http://127.0.0.1:{}/health", port);
    let timeout = TestTimeouts::get(TimeoutCategory::HealthCheck);
    let poll_interval = TestTimeouts::poll_interval();
    let start = Instant::now();

    while start.elapsed() < timeout {
        sleep(poll_interval).await;
        if let Ok(resp) = client.get(&health_url).send().await
            && resp.status().is_success()
        {
            return true;
        }
    }
    false
}

/// Spawn a new test server with dynamic port allocation.
pub async fn spawn_test_server() -> Result<TestServerInstance, String> {
    spawn_test_server_with_timeout(None).await
}

/// Spawn a new test server with a custom handshake timeout.
pub async fn spawn_test_server_with_timeout(
    handshake_timeout_secs: Option<u64>,
) -> Result<TestServerInstance, String> {
    let binary = resolve_binary_path();
    let workspace = workspace_dir();
    let temp_dir = TempDir::new().map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let sandbox_scope = temp_dir.path().to_path_buf();
    let mut args = build_server_args(handshake_timeout_secs);

    // Slot was intentionally reserved in build_server_args.
    if let Some(scope_slot) = args.get_mut(8) {
        *scope_slot = sandbox_scope.to_string_lossy().to_string();
    }

    eprintln!("[TestServer] Starting test server with dynamic port");

    let mut cmd = Command::new(&binary);
    cmd.args(&args)
        .current_dir(&workspace)
        .env_remove("AHMA_HANDSHAKE_TIMEOUT_SECS")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if should_force_no_sandbox_for_test_server() {
        eprintln!(
            "[TestServer] Sandbox unavailable on this platform/kernel; running test server with --no-sandbox"
        );
        cmd.env("AHMA_NO_SANDBOX", "1");
    }

    SandboxTestEnv::configure(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn test server: {}", e))?;

    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let stderr = child.stderr.take().expect("Failed to capture stderr");
    let (line_tx, line_rx) = mpsc::channel::<String>();
    wire_output_reader(stdout, line_tx.clone());
    wire_output_reader(stderr, line_tx);

    let bound_port = match wait_for_bound_port(&line_rx, TestTimeouts::get(TimeoutCategory::ProcessSpawn)) {
        Some(port) => port,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Timeout waiting for server to start".to_string());
        }
    };

    eprintln!("[TestServer] Server bound to port {}", bound_port);

    if wait_for_health(bound_port).await {
        return Ok(TestServerInstance {
            child,
            port: bound_port,
            _temp_dir: temp_dir,
        });
    }

    let _ = child.kill();
    let _ = child.wait();
    Err("Test server failed to respond to health check within 5 seconds".to_string())
}
