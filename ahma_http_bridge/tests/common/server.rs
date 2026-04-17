use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};
use base64::Engine as _;
use reqwest::Client;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
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
    quic_port: Option<u16>,
    quic_cert_der: Option<Vec<u8>>,
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

    /// Get the QUIC base URL (UDP), or `None` if QUIC is not running.
    pub fn quic_base_url(&self) -> Option<String> {
        self.quic_port.map(|p| format!("https://127.0.0.1:{}", p))
    }

    /// Get the QUIC port, or `None` if QUIC is not running.
    pub fn quic_port(&self) -> Option<u16> {
        self.quic_port
    }

    /// Get the DER-encoded self-signed certificate for the QUIC endpoint,
    /// or `None` if QUIC is not running.
    pub fn quic_cert_der(&self) -> Option<&[u8]> {
        self.quic_cert_der.as_deref()
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

fn target_dir_from_binary(binary: &Path) -> Option<PathBuf> {
    binary
        .parent()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
}

fn candidate_in_target(base_target: &Path, subdir: &str, bin_name: &str) -> PathBuf {
    base_target.join(subdir).join(bin_name)
}

pub fn resolve_binary_path() -> PathBuf {
    static BINARY_LOG_ONCE: std::sync::Once = std::sync::Once::new();

    let debug_bin = ahma_mcp::test_utils::cli::get_binary_path("ahma-mcp", "ahma-mcp");
    // Construct sibling binary paths with the correct platform executable extension.
    let exe_ext = if cfg!(windows) { ".exe" } else { "" };
    let bin_name = format!("ahma-mcp{exe_ext}");
    let mut candidates = vec![debug_bin.clone()];
    if let Some(base_target) = target_dir_from_binary(&debug_bin) {
        for subdir in [
            "release",
            "llvm-cov-target/debug",
            "llvm-cov-target/release",
        ] {
            candidates.push(candidate_in_target(&base_target, subdir, &bin_name));
        }
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

struct ServerSpec {
    args: Vec<String>,
    env: Vec<(String, String)>,
}

fn build_server_spec(
    tools_dir: &Path,
    sandbox_scope: &Path,
    handshake_timeout_secs: Option<u64>,
) -> ServerSpec {
    let mut env = vec![
        ("AHMA_SYNC".to_string(), "1".to_string()),
        ("AHMA_LOG_TARGET".to_string(), "stderr".to_string()),
        (
            "AHMA_TOOLS_DIR".to_string(),
            tools_dir.to_string_lossy().to_string(),
        ),
        (
            "AHMA_SANDBOX_SCOPE".to_string(),
            sandbox_scope.to_string_lossy().to_string(),
        ),
    ];

    if let Some(timeout) = handshake_timeout_secs {
        env.push(("AHMA_HANDSHAKE_TIMEOUT".to_string(), timeout.to_string()));
        env.push((
            "AHMA_HTTP_BRIDGE_REQUEST_TIMEOUT_SECS".to_string(),
            timeout.to_string(),
        ));
    }

    ServerSpec {
        args: vec![
            "serve".to_string(),
            "http".to_string(),
            "--port".to_string(),
            "0".to_string(),
        ],
        env,
    }
}

#[cfg(target_os = "linux")]
fn should_force_no_sandbox_for_test_server() -> bool {
    use ahma_mcp::sandbox::SandboxError;

    matches!(
        ahma_mcp::sandbox::check_sandbox_prerequisites(),
        Err(SandboxError::LandlockNotAvailable) | Err(SandboxError::PrerequisiteFailed(_))
    )
}

/// On macOS, spawn the test server without `--disable-sandbox` only when
/// `sandbox-exec` is known to work in the current environment.  When running
/// inside a nested sandbox (Cursor, VS Code, Docker) `sandbox-exec` returns
/// exit 71 / "Operation not permitted", so we fall back to `--disable-sandbox` so
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

struct ServerStartupInfo {
    bound_port: u16,
    quic_port: Option<u16>,
    quic_cert_der: Option<Vec<u8>>,
}

enum StartupMarker {
    BoundPort(u16),
    QuicPort(u16),
    QuicCert(Vec<u8>),
}

fn parse_port_marker(line: &str, marker: &str) -> Option<u16> {
    let index = line.find(marker)?;
    line[index + marker.len()..].trim().parse::<u16>().ok()
}

fn parse_quic_cert_marker(line: &str) -> Option<Vec<u8>> {
    let index = line.find("AHMA_QUIC_CERT=")?;
    let encoded = line[index + "AHMA_QUIC_CERT=".len()..].trim();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    (!decoded.is_empty()).then_some(decoded)
}

fn parse_startup_marker(line: &str) -> Option<StartupMarker> {
    parse_port_marker(line, "AHMA_BOUND_PORT=")
        .map(StartupMarker::BoundPort)
        .or_else(|| parse_port_marker(line, "AHMA_QUIC_PORT=").map(StartupMarker::QuicPort))
        .or_else(|| parse_quic_cert_marker(line).map(StartupMarker::QuicCert))
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn read_startup_line(
    line: &str,
    quic_port: &mut Option<u16>,
    quic_cert_der: &mut Option<Vec<u8>>,
) -> Option<u16> {
    match parse_startup_marker(line) {
        Some(StartupMarker::QuicPort(port)) => {
            *quic_port = Some(port);
            None
        }
        Some(StartupMarker::QuicCert(cert)) => {
            *quic_cert_der = Some(cert);
            None
        }
        Some(StartupMarker::BoundPort(port)) => Some(port),
        None => None,
    }
}

fn configure_server_command(
    cmd: &mut Command,
    workspace: &Path,
    args: &[String],
    env: &[(String, String)],
    no_sandbox_message: &str,
) {
    cmd.args(args)
        .current_dir(workspace)
        .env_remove("AHMA_HANDSHAKE_TIMEOUT_SECS")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (k, v) in env {
        cmd.env(k, v);
    }

    if should_force_no_sandbox_for_test_server() {
        eprintln!("{no_sandbox_message}");
        cmd.env("AHMA_DISABLE_SANDBOX", "1");
    }

    SandboxTestEnv::configure(cmd);
}

fn attach_output_readers(child: &mut Child) -> mpsc::Receiver<String> {
    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let stderr = child.stderr.take().expect("Failed to capture stderr");
    let (line_tx, line_rx) = mpsc::channel::<String>();
    wire_output_reader(stdout, line_tx.clone());
    wire_output_reader(stderr, line_tx);
    line_rx
}

fn spawn_server_child(
    binary: &Path,
    workspace: &Path,
    spec: &ServerSpec,
    spawn_error: &str,
    no_sandbox_message: &str,
) -> Result<(Child, mpsc::Receiver<String>), String> {
    let mut cmd = Command::new(binary);
    configure_server_command(
        &mut cmd,
        workspace,
        &spec.args,
        &spec.env,
        no_sandbox_message,
    );

    let mut child = cmd.spawn().map_err(|e| format!("{spawn_error}: {e}"))?;
    let line_rx = attach_output_readers(&mut child);
    Ok((child, line_rx))
}

fn wait_for_startup_info(
    receiver: &mpsc::Receiver<String>,
    timeout: Duration,
) -> Option<ServerStartupInfo> {
    let start = Instant::now();
    let mut quic_port: Option<u16> = None;
    let mut quic_cert_der: Option<Vec<u8>> = None;

    while start.elapsed() <= timeout {
        match receiver.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                eprintln!("{}", line);
                if let Some(bound_port) =
                    read_startup_line(&line, &mut quic_port, &mut quic_cert_der)
                {
                    return Some(ServerStartupInfo {
                        bound_port,
                        quic_port,
                        quic_cert_der,
                    });
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    None
}

async fn wait_for_health(port: u16) -> bool {
    let client = Client::builder()
        .http2_prior_knowledge()
        .build()
        .expect("Failed to build HTTP/2 health-check client");
    let health_url = format!("http://127.0.0.1:{}/health", port);
    let timeout = TestTimeouts::get(TimeoutCategory::HealthCheck);
    let poll_interval = TestTimeouts::poll_interval();
    let start = Instant::now();

    while start.elapsed() < timeout {
        sleep(poll_interval).await;
        match client.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => return true,
            Ok(_) | Err(_) => {}
        }
    }
    false
}

fn wait_for_startup_or_cleanup(
    child: &mut Child,
    line_rx: &mpsc::Receiver<String>,
    timeout: Duration,
    timeout_message: &'static str,
) -> Result<ServerStartupInfo, String> {
    match wait_for_startup_info(line_rx, timeout) {
        Some(info) => Ok(info),
        None => {
            stop_child(child);
            Err(timeout_message.to_string())
        }
    }
}

async fn wait_for_health_or_cleanup(
    child: &mut Child,
    bound_port: u16,
    failure_message: &'static str,
) -> Result<(), String> {
    if wait_for_health(bound_port).await {
        return Ok(());
    }
    stop_child(child);
    Err(failure_message.to_string())
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
    let tools_dir = workspace.join(".ahma");
    let temp_dir = TempDir::new().map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let sandbox_scope = temp_dir.path().to_path_buf();
    let spec = build_server_spec(&tools_dir, &sandbox_scope, handshake_timeout_secs);

    eprintln!("[TestServer] Starting test server with dynamic port");
    let (mut child, line_rx) = spawn_server_child(
        &binary,
        &workspace,
        &spec,
        "Failed to spawn test server",
        "[TestServer] Sandbox unavailable on this platform/kernel; running test server with --disable-sandbox",
    )?;

    let startup_info = wait_for_startup_or_cleanup(
        &mut child,
        &line_rx,
        TestTimeouts::get(TimeoutCategory::ProcessSpawn),
        "Timeout waiting for server to start",
    )?;
    let bound_port = startup_info.bound_port;

    eprintln!("[TestServer] Server bound to port {}", bound_port);

    wait_for_health_or_cleanup(
        &mut child,
        bound_port,
        "Test server failed to respond to health check within 5 seconds",
    )
    .await?;

    Ok(TestServerInstance {
        child,
        port: bound_port,
        quic_port: startup_info.quic_port,
        quic_cert_der: startup_info.quic_cert_der,
        _temp_dir: temp_dir,
    })
}

/// Spawn a raw server guard using explicit tools + sandbox scope paths.
///
/// This is useful for integration tests that need custom roots but still want
/// shared startup/health-check behavior from `tests/common/server.rs`.
pub async fn spawn_server_guard_with_config(
    tools_dir: &Path,
    sandbox_scope: &Path,
    handshake_timeout_secs: Option<u64>,
) -> Result<ServerGuard, String> {
    let binary = resolve_binary_path();
    let workspace = workspace_dir();
    let spec = build_server_spec(tools_dir, sandbox_scope, handshake_timeout_secs);

    eprintln!(
        "[TestServer] Starting custom server with scope {}",
        sandbox_scope.display()
    );
    let (mut child, line_rx) = spawn_server_child(
        &binary,
        &workspace,
        &spec,
        "Failed to spawn custom test server",
        "[TestServer] Sandbox unavailable on this platform/kernel; forcing custom server no-sandbox",
    )?;

    let startup_info = wait_for_startup_or_cleanup(
        &mut child,
        &line_rx,
        TestTimeouts::get(TimeoutCategory::ProcessSpawn),
        "Timeout waiting for custom server to start",
    )?;

    wait_for_health_or_cleanup(
        &mut child,
        startup_info.bound_port,
        "Custom test server failed to respond to health check within timeout",
    )
    .await?;

    Ok(ServerGuard::new(child, startup_info.bound_port))
}
