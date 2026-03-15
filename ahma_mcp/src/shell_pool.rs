//! # Pre-warmed Shell Pool for High-Performance Command Execution
//!
//! This module provides a sophisticated pooling mechanism for shell processes, designed to
//! minimize latency when executing external commands, particularly `cargo` operations.
//!
//! ## Core Concepts
//!
//! * **`PrewarmedShell`**: A single, long-lived shell process (e.g., `bash`) that is kept
//!   alive and ready to accept commands. It communicates over its `stdin` and `stdout`
//!   using a simple JSON-based protocol.
//!
//! * **`ShellPool`**: A collection of `PrewarmedShell` instances, all configured for the
//!   same working directory. This allows multiple commands to be executed in parallel
//!   for a given directory.
//!
//! * **`ShellPoolManager`**: The central manager that oversees all `ShellPool`s. It creates,
//!   manages, and garbage-collects pools as needed, ensuring that resources are used
//!   efficiently. It also enforces global limits on the total number of active shells.
//!
//! ## Features
//!
//! * **Low Latency**: By reusing existing shell processes, the overhead of process creation
//!   is eliminated, leading to significantly faster command execution.
//! * **Working Directory Isolation**: Shells are pooled on a per-directory basis, ensuring
//!   that commands run in the correct context without interfering with each other.
//! * **Resource Management**: The manager enforces a cap on the total number of concurrent
//!   shells and automatically cleans up idle pools and shells to prevent resource leaks.
//! * **Health Checking**: Shells are periodically checked for responsiveness, and unhealthy
//!   processes are automatically culled and replaced.
//! * **Asynchronous API**: The entire system is built on `tokio`, providing a fully
//!   non-blocking interface suitable for high-concurrency applications.
//!
//! ## Usage
//!
//! 1. Create a `ShellPoolManager` with a desired `ShellPoolConfig`.
//! 2. Start the background monitoring tasks using `manager.start_background_tasks()`.
//! 3. When a command needs to be executed, request a shell from the manager using
//!    `manager.get_shell(working_dir).await`.
//! 4. If a shell is available, execute the command using `shell.execute_command(...)`.
//! 5. Return the shell to the manager using `manager.return_shell(shell).await` so it can
//!    be reused.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, RwLock, Semaphore},
    time::timeout,
};
use tracing;

static SHELL_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Configuration for shell pool behavior
#[derive(Debug, Clone)]
pub struct ShellPoolConfig {
    pub enabled: bool,
    pub shells_per_directory: usize,
    pub max_total_shells: usize,
    pub shell_idle_timeout: Duration,
    pub pool_cleanup_interval: Duration,
    pub shell_spawn_timeout: Duration,
    pub command_timeout: Duration,
    pub health_check_interval: Duration,
}

impl Default for ShellPoolConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            shells_per_directory: 2,
            max_total_shells: 20,
            shell_idle_timeout: Duration::from_secs(60),
            pool_cleanup_interval: Duration::from_secs(60),
            shell_spawn_timeout: Duration::from_secs(5),
            command_timeout: Duration::from_secs(300),
            health_check_interval: Duration::from_secs(60),
        }
    }
}

/// Serializable command payload sent to a pre-warmed shell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellCommand {
    /// Unique command identifier for correlation.
    pub id: String,
    /// Command and arguments (argv style).
    pub command: Vec<String>,
    /// Working directory for execution.
    pub working_dir: String,
    /// Timeout for execution in milliseconds.
    pub timeout_ms: u64,
}

/// Response payload returned by a pre-warmed shell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellResponse {
    /// Command identifier matching the request.
    pub id: String,
    /// Process exit code.
    pub exit_code: i32,
    /// Captured stdout output.
    pub stdout: String,
    /// Captured stderr output.
    pub stderr: String,
    /// Total execution time in milliseconds.
    pub duration_ms: u64,
}

/// Error types for shell operations
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("Failed to spawn shell process: {0}")]
    SpawnError(#[from] std::io::Error),

    #[error("Shell communication timeout")]
    Timeout,

    #[error("Shell process died unexpectedly")]
    ProcessDied,

    #[error("Failed to serialize command: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Shell pool is at capacity")]
    PoolFull,

    #[error("Working directory access error: {0}")]
    WorkingDirectoryError(String),
}

impl ShellError {
    /// Check if this error represents a potentially recoverable condition
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            ShellError::Timeout | ShellError::PoolFull | ShellError::ProcessDied
        )
    }

    /// Check if this error indicates resource exhaustion
    pub fn is_resource_exhaustion(&self) -> bool {
        matches!(self, ShellError::PoolFull | ShellError::Timeout)
    }

    /// Check if this error is related to I/O operations
    pub fn is_io_error(&self) -> bool {
        matches!(
            self,
            ShellError::SpawnError(_) | ShellError::WorkingDirectoryError(_)
        )
    }

    /// Get error category for programmatic handling
    pub fn error_category(&self) -> &'static str {
        match self {
            ShellError::SpawnError(_) | ShellError::WorkingDirectoryError(_) => "IO",
            ShellError::Timeout => "TIMEOUT",
            ShellError::ProcessDied => "PROCESS",
            ShellError::SerializationError(_) => "SERIALIZATION",
            ShellError::PoolFull => "RESOURCE",
        }
    }

    /// Get severity level for logging
    pub fn severity_level(&self) -> &'static str {
        match self {
            ShellError::SpawnError(_)
            | ShellError::ProcessDied
            | ShellError::SerializationError(_)
            | ShellError::WorkingDirectoryError(_) => "ERROR",
            ShellError::Timeout | ShellError::PoolFull => "WARN", // Might be temporary
        }
    }
}

// ---------------------------------------------------------------------------
// Platform-specific shell helpers
// ---------------------------------------------------------------------------

/// The shell binary for the pre-warmed protocol process.
/// On Windows we use the built-in PowerShell (`powershell.exe`) for the
/// health-check loop; on all other platforms we use `bash`.
///
/// Also used by `mcp_service` handlers to build progress descriptions.
pub(crate) fn platform_shell_program() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "powershell"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "bash"
    }
}

/// Extra arguments passed to the shell binary before it reads the protocol
/// script from stdin.
fn shell_args() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        // -Command - : read a script from stdin and execute it
        &["-NoProfile", "-NonInteractive", "-Command", "-"]
    }
    #[cfg(not(target_os = "windows"))]
    {
        &[]
    }
}

/// The protocol setup script injected into the shell on startup.
/// The script must:
///
/// 1. Write a single "SHELL_READY" line to stdout.
/// 2. Enter a loop: "HEALTH_CHECK" -> "HEALTHY", "SHUTDOWN" -> exit.
///
/// Actual command execution bypasses this shell entirely (see `execute_command`).
fn initialize_protocol_script() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        // PowerShell (`powershell -Command -`) reads the script from stdin;
        // the while-loop then keeps reading further stdin lines for health checks.
        r#"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::InputEncoding  = [System.Text.UTF8Encoding]::new($false)
Write-Output 'SHELL_READY'
[Console]::Out.Flush()
while ($true) {
    $line = [Console]::In.ReadLine()
    if ($null -eq $line) { break }
    if ($line -eq 'HEALTH_CHECK') {
        Write-Output 'HEALTHY'
        [Console]::Out.Flush()
    } elseif ($line -eq 'SHUTDOWN') {
        break
    }
}
"#
    }
    #[cfg(not(target_os = "windows"))]
    {
        r#"
# Portable minimal shell setup for async_cargo_mcp (macOS bash 3.2 compatible)
set +e

if ! command -v jq >/dev/null 2>&1; then
    echo 'MCP_DIAG: jq not found in PATH' >&2
fi

# JSON string encoder: reads from stdin, outputs a JSON string value
json_escape_stream() {
    if command -v jq >/dev/null 2>&1; then
        jq -Rs . 2>/dev/null
        return
    fi
    # Fallback: emit empty JSON string if no input
    if [ -t 0 ]; then
        printf '""'
        return
    fi
    tmp_in=$(mktemp)
    cat >"$tmp_in"
    if [ ! -s "$tmp_in" ]; then
        printf '""'
    else
        # Basic escaping: backslash and quote, join lines with \n
        sed 's/\\/\\\\/g; s/"/\"/g; s/$/\\n/' "${tmp_in}" | tr -d '\n' | sed 's/\\n$//' | sed 's/^"/;s/$/"/' 
    fi
    rm -f "$tmp_in"
}

# Extract simple fields from JSON when jq is unavailable
json_get_string() {
    key="$1"
    input="$2"
    if command -v jq >/dev/null 2>&1; then
        echo "$input" | jq -r ".${key}"
    else
        echo "$input" | sed -n 's/.*"'"$key"'"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1
    fi
}

json_get_command_array() {
    input="$1"
    if command -v jq >/dev/null 2>&1; then
        echo "$input" | jq -r '.command[]'
    else
        # Extract content inside command array and split by ","
        echo "$input" | sed -n 's/.*"command"[[:space:]]*:[[:space:]]*\[\(.*\)\].*/\1/p' | head -n1 | awk -v RS=',' '{gsub(/^\"|\"$/,"",$0); print}'
    fi
}

execute_command() {
    cmd_json="$1"
    id=$(json_get_string id "$cmd_json")
    working_dir=$(json_get_string working_dir "$cmd_json")

    # Safely read command and arguments into a bash array
    temp_cmd_file=$(mktemp)
    json_get_command_array "$cmd_json" > "$temp_cmd_file"
    cmd_array=()
    while IFS= read -r cmd_part; do
        [ -z "$cmd_part" ] && continue
        cmd_array[${#cmd_array[@]}]="$cmd_part"
    done < "$temp_cmd_file"
    rm -f "$temp_cmd_file"

    cd "$working_dir" 2>/dev/null || {
        echo '{"id":"'"$id"'","exit_code":1,"stdout":"","stderr":"Failed to change directory","duration_ms":0}'
        return
    }

    start_time=$(date +%s)
    temp_stdout=$(mktemp)
    temp_stderr=$(mktemp)

    # Execute command directly, with each part as a separate argument
    "${cmd_array[@]}" >"$temp_stdout" 2>"$temp_stderr"
    exit_code=$?
    end_time=$(date +%s)
    duration=$(((end_time - start_time)*1000))

    stdout_json=$(json_escape_stream < "$temp_stdout")
    stderr_json=$(json_escape_stream < "$temp_stderr")
    rm -f "$temp_stdout" "$temp_stderr"
    echo '{"id":"'"$id"'","exit_code":"'$exit_code'","stdout":'"$stdout_json"',"stderr":'"$stderr_json"',"duration_ms":'"$duration"'}'
}

echo "SHELL_READY"

while IFS= read -r line; do
    [ -z "$line" ] && continue
    if [ "$line" = "HEALTH_CHECK" ]; then
        echo "HEALTHY"
    elif [ "$line" = "SHUTDOWN" ]; then
        break
    else
        execute_command "$line"
    fi

done
"#
    }
}

/// A single prewarmed shell process
pub struct PrewarmedShell {
    /// Unique identifier for this shell
    pub id: String,
    /// The shell process
    process: Child,
    /// Writer for sending commands via stdin
    stdin: tokio::process::ChildStdin,
    /// Reader for receiving responses via stdout
    stdout_reader: BufReader<tokio::process::ChildStdout>,
    /// Working directory for this shell
    working_dir: PathBuf,
    /// Configuration for this shell
    config: ShellPoolConfig,
    /// Last time this shell was used
    last_used: Instant,
    /// Whether this shell is currently healthy
    is_healthy: bool,
    /// Lock to ensure only one command runs at a time
    command_lock: Mutex<()>,
}

impl std::fmt::Debug for PrewarmedShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrewarmedShell")
            .field("id", &self.id)
            .field("working_dir", &self.working_dir)
            .field("config", &self.config)
            .field("last_used", &self.last_used)
            .field("is_healthy", &self.is_healthy)
            .field("process_id", &self.process.id())
            .finish_non_exhaustive()
    }
}

impl PrewarmedShell {
    /// Create a new prewarmed shell for the specified working directory
    pub async fn new(
        working_dir: impl AsRef<Path>,
        config: &ShellPoolConfig,
    ) -> Result<Self, ShellError> {
        let working_dir = working_dir.as_ref().to_path_buf();
        let shell_id = format!("sh_{}", SHELL_ID_COUNTER.fetch_add(1, Ordering::Relaxed));

        tracing::debug!(
            "Spawning new shell {} for directory: {:?}",
            shell_id,
            &working_dir
        );

        // Wrap initialization in timeout
        timeout(config.shell_spawn_timeout, async {
            // Spawn a pre-warmed protocol shell.
            // On Windows: `powershell -NoProfile -NonInteractive -Command -`
            // On Unix:    `bash`
            let mut process = Command::new(platform_shell_program())
                .args(shell_args())
                .kill_on_drop(true)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()) // capture stderr for diagnostics
                .current_dir(&working_dir)
                .spawn()?;

            let stdin = process.stdin.take().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Failed to get stdin")
            })?;

            let stdout = process.stdout.take().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Failed to get stdout")
            })?;

            let stdout_reader = BufReader::new(stdout);
            // Spawn a background task to read and log stderr lines for diagnostics
            if let Some(stderr) = process.stderr.take() {
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stderr);
                    loop {
                        let mut line = String::new();
                        match reader.read_line(&mut line).await {
                            Ok(0) => break, // EOF
                            Ok(_) => {
                                if !line.trim().is_empty() {
                                    tracing::warn!(target: "shell_stderr", "shell stderr: {}", line.trim_end());
                                }
                            }
                            Err(e) => {
                                tracing::warn!(target: "shell_stderr", "error reading shell stderr: {}", e);
                                break;
                            }
                        }
                    }
                });
            }

            let mut shell = Self {
                id: shell_id.clone(),
                process,
                stdin,
                stdout_reader,
                working_dir: working_dir.clone(),
                config: config.clone(),
                last_used: Instant::now(),
                is_healthy: true,
                command_lock: Mutex::new(()),
            };

            // Initialize the shell with our command protocol handler
            shell.initialize_protocol().await?;

            tracing::info!(
                "Successfully spawned shell {} for directory: {:?}",
                shell_id,
                &working_dir
            );
            Ok(shell)
        })
        .await
        .map_err(|_| ShellError::Timeout)?
    }

    /// Initialize the shell with our JSON command protocol
    async fn initialize_protocol(&mut self) -> Result<(), ShellError> {
        // Send the platform-specific protocol setup script to the shell.
        // NOTE: `execute_command` bypasses this shell and spawns programs directly;
        // this script only drives the SHELL_READY handshake and health checks.
        let setup_script = initialize_protocol_script();

        // Send setup script to shell
        self.stdin.write_all(setup_script.as_bytes()).await?;
        self.stdin.flush().await?;

        // Wait for ready signal
        let mut ready_line = String::new();
        self.stdout_reader.read_line(&mut ready_line).await?;

        if ready_line.trim() != "SHELL_READY" {
            tracing::error!(
                "Shell {} failed to emit SHELL_READY, got: '{}'",
                self.id,
                ready_line.trim()
            );
            return Err(ShellError::ProcessDied);
        }

        tracing::debug!("Shell {} initialized and ready", self.id);
        Ok(())
    }

    /// Execute a command in this shell
    pub async fn execute_command(
        &mut self,
        command: ShellCommand,
    ) -> Result<ShellResponse, ShellError> {
        let _lock = self.command_lock.lock().await;
        self.last_used = Instant::now();

        tracing::info!(
            "Executing command {} in shell {}: {:?}",
            command.id,
            self.id,
            command.command
        );

        // SAFETY & ROBUSTNESS: Execute directly via a subprocess to avoid
        // protocol flakiness under heavy concurrency while preserving pooling semantics.
        if command.command.is_empty() {
            return Err(ShellError::WorkingDirectoryError(
                "empty command".to_string(),
            ));
        }

        let program = &command.command[0];
        let args: Vec<&str> = command.command.iter().skip(1).map(|s| s.as_str()).collect();

        let child_spawn = Command::new(program)
            .args(&args)
            .current_dir(&command.working_dir)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let child = match child_spawn {
            Ok(c) => c,
            Err(e) => {
                // If the binary is not found, emulate shell behavior with exit code 127
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(ShellResponse {
                        id: command.id,
                        exit_code: 127,
                        stdout: String::new(),
                        stderr: format!("{}: command not found", program),
                        duration_ms: 0,
                    });
                }
                return Err(ShellError::SpawnError(e));
            }
        };

        let start = Instant::now();
        let timeout_duration = Duration::from_millis(command.timeout_ms);

        let output = timeout(timeout_duration, child.wait_with_output())
            .await
            .map_err(|_| ShellError::Timeout)?
            .map_err(ShellError::SpawnError)?;

        let duration_ms = start.elapsed().as_millis() as u64;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok(ShellResponse {
            id: command.id,
            exit_code: output.status.code().unwrap_or(-1),
            stdout,
            stderr,
            duration_ms,
        })
    }

    /// Check if this shell is healthy
    pub async fn health_check(&mut self) -> bool {
        let _lock = self.command_lock.lock().await;

        tracing::debug!("Performing health check on shell {}", self.id);

        // Send health check command
        if let Err(e) = self.stdin.write_all(b"HEALTH_CHECK\n").await {
            tracing::warn!("Health check failed for shell {}: {}", self.id, e);
            self.is_healthy = false;
            return false;
        }

        if let Err(e) = self.stdin.flush().await {
            tracing::warn!("Health check failed for shell {}: {}", self.id, e);
            self.is_healthy = false;
            return false;
        }

        // Read health response with short timeout
        let health_future = async {
            let mut response = String::new();
            self.stdout_reader.read_line(&mut response).await?;
            Ok::<String, std::io::Error>(response)
        };

        match timeout(Duration::from_secs(2), health_future).await {
            Ok(Ok(response)) if response.trim() == "HEALTHY" => {
                tracing::debug!("Shell {} is healthy", self.id);
                self.is_healthy = true;
                true
            }
            _ => {
                tracing::warn!("Shell {} failed health check", self.id);
                self.is_healthy = false;
                false
            }
        }
    }

    /// Get the working directory this shell is configured for
    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }

    /// Get when this shell was last used
    pub fn last_used(&self) -> Instant {
        self.last_used
    }

    /// Check if this shell is healthy
    pub fn is_healthy(&self) -> bool {
        self.is_healthy
    }

    /// Get the shell ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Gracefully shutdown this shell
    pub async fn shutdown(&mut self) {
        tracing::debug!("Shutting down shell {}", self.id);

        // Try to send shutdown signal
        if (self.stdin.write_all(b"SHUTDOWN\n").await).is_ok() {
            let _ = self.stdin.flush().await;
        }

        // Kill the process
        if let Err(e) = self.process.kill().await {
            tracing::warn!("Failed to kill shell process {}: {}", self.id, e);
        }

        // Wait for process to exit
        if let Err(e) = self.process.wait().await {
            tracing::warn!("Error waiting for shell {} to exit: {}", self.id, e);
        }

        tracing::info!("Shell {} has been shut down", self.id);
    }
}

impl Drop for PrewarmedShell {
    fn drop(&mut self) {
        // Attempt to kill the process on drop
        let _ = self.process.start_kill();
    }
}

/// Pool of shells for a specific working directory
#[derive(Debug)]
pub struct ShellPool {
    working_dir: PathBuf,
    shells: Mutex<Vec<PrewarmedShell>>,
    config: ShellPoolConfig,
    last_accessed: Mutex<Instant>,
}

impl ShellPool {
    /// Create a new shell pool for the specified working directory
    pub fn new(working_dir: impl AsRef<Path>, config: ShellPoolConfig) -> Self {
        let working_dir = working_dir.as_ref().to_path_buf();
        tracing::info!("Creating shell pool for directory: {:?}", working_dir);

        Self {
            working_dir,
            shells: Mutex::new(Vec::new()),
            config,
            last_accessed: Mutex::new(Instant::now()),
        }
    }

    /// Get a shell from the pool, creating one if necessary
    pub async fn get_shell(&self) -> Result<PrewarmedShell, ShellError> {
        let mut last_accessed = self.last_accessed.lock().await;
        *last_accessed = Instant::now();
        drop(last_accessed);

        let mut shells = self.shells.lock().await;

        // Try to find a healthy shell
        while let Some(shell) = shells.pop() {
            if shell.is_healthy() {
                tracing::debug!("Reusing healthy shell {} from pool", shell.id());
                return Ok(shell);
            } else {
                tracing::debug!("Discarding unhealthy shell {} from pool", shell.id());
                // Shell is unhealthy, let it drop and try next
            }
        }

        drop(shells);

        // No healthy shells available, create a new one
        tracing::debug!(
            "Creating new shell for pool (directory: {:?})",
            self.working_dir
        );
        PrewarmedShell::new(&self.working_dir, &self.config).await
    }

    /// Return a shell to the pool
    pub async fn return_shell(&self, shell: PrewarmedShell) {
        let mut shells = self.shells.lock().await;

        // Only return healthy shells and respect pool size limits
        if shell.is_healthy() && shells.len() < self.config.shells_per_directory {
            tracing::debug!("Returning shell {} to pool", shell.id());
            shells.push(shell);
        } else {
            tracing::debug!("Discarding shell {} (unhealthy or pool full)", shell.id());
            // Shell will be dropped and process killed
        }
    }

    /// Check if this pool has been idle for too long
    pub async fn is_idle(&self) -> bool {
        let last_accessed = self.last_accessed.lock().await;
        last_accessed.elapsed() > self.config.shell_idle_timeout
    }

    /// Get the working directory for this pool
    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }

    /// Perform health checks on all shells in the pool
    pub async fn health_check(&self) {
        let mut shells = self.shells.lock().await;
        let mut healthy_shells = Vec::new();

        for mut shell in shells.drain(..) {
            if shell.health_check().await {
                healthy_shells.push(shell);
            } else {
                tracing::debug!("Removing unhealthy shell {} from pool", shell.id());
                // Unhealthy shell will be dropped
            }
        }

        *shells = healthy_shells;
    }

    /// Shutdown all shells in this pool
    pub async fn shutdown(&self) {
        let mut shells = self.shells.lock().await;
        for mut shell in shells.drain(..) {
            shell.shutdown().await;
        }
        tracing::info!("Shut down shell pool for directory: {:?}", self.working_dir);
    }

    /// Get the current number of shells in the pool
    pub async fn shell_count(&self) -> usize {
        let shells = self.shells.lock().await;
        shells.len()
    }
}

/// Manager for multiple shell pools across different working directories
#[derive(Debug)]
pub struct ShellPoolManager {
    pools: RwLock<HashMap<PathBuf, Arc<ShellPool>>>,
    config: ShellPoolConfig,
    shell_semaphore: Semaphore,
}

impl ShellPoolManager {
    /// Create a new shell pool manager
    pub fn new(config: ShellPoolConfig) -> Self {
        tracing::info!("Creating shell pool manager with config: {:#?}", config);

        Self {
            pools: RwLock::new(HashMap::new()),
            shell_semaphore: Semaphore::new(config.max_total_shells),
            config,
        }
    }

    /// Start background monitoring tasks (call this after creating the manager)
    pub fn start_background_tasks(self: Arc<Self>) {
        // Simplified: No background tasks for now to avoid polling issues
        // Background cleanup will be handled on-demand during get_shell/return_shell
        tracing::info!("Shell pool background tasks disabled for performance");
    }

    /// Get a shell for the specified working directory
    pub async fn get_shell(&self, working_dir: impl AsRef<Path>) -> Option<PrewarmedShell> {
        if !self.config.enabled {
            tracing::debug!("Shell pooling is disabled");
            return None;
        }

        let working_dir = working_dir.as_ref().to_path_buf();

        // Check if we're at capacity first
        if self.shell_semaphore.available_permits() == 0 {
            tracing::debug!(
                "Shell pool at capacity ({} shells), skipping",
                self.config.max_total_shells
            );
            return None;
        }

        // Acquire a permit from the semaphore
        let _permit = match self.shell_semaphore.try_acquire() {
            Ok(permit) => permit,
            Err(_) => {
                tracing::debug!(
                    "Shell pool at capacity ({} shells), skipping",
                    self.config.max_total_shells
                );
                return None;
            }
        };

        let pool = {
            let pools = self.pools.read().await;
            if let Some(pool) = pools.get(&working_dir) {
                Arc::clone(pool)
            } else {
                drop(pools);
                self.create_pool_for_dir(&working_dir).await
            }
        };

        // Get shell from pool
        match pool.get_shell().await {
            Ok(shell) => {
                tracing::debug!(
                    "Got shell from pool, available permits: {}",
                    self.shell_semaphore.available_permits()
                );
                // Keep the permit active until the shell is returned
                std::mem::forget(_permit);
                Some(shell)
            }
            Err(e) => {
                tracing::warn!("Failed to get shell from pool for {:?}: {}", working_dir, e);
                // Release the permit since we didn't use it
                drop(_permit);
                None
            }
        }
    }

    /// Return a shell to its appropriate pool
    pub async fn return_shell(&self, shell: PrewarmedShell) {
        let working_dir = shell.working_dir().to_path_buf();

        // Find the pool for this working directory
        let pools = self.pools.read().await;
        if let Some(pool) = pools.get(&working_dir) {
            let pool = Arc::clone(pool);
            drop(pools);

            pool.return_shell(shell).await;

            // Release one permit back to the semaphore
            self.shell_semaphore.add_permits(1);
            tracing::debug!(
                "Returned shell to pool, available permits: {}",
                self.shell_semaphore.available_permits()
            );
        } else {
            tracing::warn!("No pool found for working directory: {:?}", working_dir);
            // Shell will be dropped, also release permit
            self.shell_semaphore.add_permits(1);
        }
    }

    /// Create a new pool for the specified working directory
    async fn create_pool_for_dir(&self, working_dir: &Path) -> Arc<ShellPool> {
        let mut pools = self.pools.write().await;

        // Double-check that pool wasn't created while we were waiting for write lock
        if let Some(existing_pool) = pools.get(working_dir) {
            return Arc::clone(existing_pool);
        }

        let pool = Arc::new(ShellPool::new(working_dir, self.config.clone()));
        pools.insert(working_dir.to_path_buf(), Arc::clone(&pool));

        tracing::info!("Created new shell pool for directory: {:?}", working_dir);
        pool
    }

    /// Clean up idle pools and perform health checks
    pub async fn cleanup_idle_pools(&self) {
        tracing::debug!("Starting cleanup of idle pools");

        let mut pools = self.pools.write().await;
        let mut pools_to_remove = Vec::new();

        // Check each pool for idleness and health
        for (working_dir, pool) in pools.iter() {
            if pool.is_idle().await {
                tracing::debug!("Pool for {:?} is idle, marking for removal", working_dir);
                pools_to_remove.push(working_dir.clone());
            } else {
                // Perform health check on active pools
                pool.health_check().await;
            }
        }

        // Remove idle pools
        for working_dir in pools_to_remove {
            if let Some(pool) = pools.remove(&working_dir) {
                pool.shutdown().await;
            }
        }

        tracing::debug!("Completed cleanup, {} pools remaining", pools.len());
    }

    /// Shutdown all pools and shells
    pub async fn shutdown_all(&self) {
        tracing::info!("Shutting down all shell pools");

        let mut pools = self.pools.write().await;
        let pool_count = pools.len();

        for (_, pool) in pools.drain() {
            pool.shutdown().await;
        }

        self.shell_semaphore.add_permits(
            self.config
                .max_total_shells
                .saturating_sub(self.shell_semaphore.available_permits()),
        );

        tracing::info!("Shut down {} shell pools", pool_count);
    }

    /// Get configuration
    pub fn config(&self) -> &ShellPoolConfig {
        &self.config
    }

    /// Get current statistics
    pub async fn get_stats(&self) -> ShellPoolStats {
        let pools = self.pools.read().await;
        let available_permits = self.shell_semaphore.available_permits();
        let total_shells = self
            .config
            .max_total_shells
            .saturating_sub(available_permits);

        ShellPoolStats {
            total_pools: pools.len(),
            total_shells,
            max_shells: self.config.max_total_shells,
        }
    }
}

/// Statistics about shell pool usage
#[derive(Debug, Clone)]
pub struct ShellPoolStats {
    pub total_pools: usize,
    pub total_shells: usize,
    pub max_shells: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::logging::init_test_logging;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_shell_pool_config_defaults() {
        init_test_logging();
        let config = ShellPoolConfig::default();
        assert!(config.enabled); // Should be enabled by default for production use
        assert_eq!(config.shells_per_directory, 2);
        assert_eq!(config.max_total_shells, 20);
    }

    #[tokio::test]
    async fn test_shell_command_serialization() {
        init_test_logging();
        let command = ShellCommand {
            id: "test123".to_string(),
            command: vec!["cargo".to_string(), "build".to_string()],
            working_dir: "/tmp".to_string(),
            timeout_ms: 30000,
        };

        let json = serde_json::to_string(&command).unwrap();
        let deserialized: ShellCommand = serde_json::from_str(&json).unwrap();

        assert_eq!(command.id, deserialized.id);
        assert_eq!(command.command, deserialized.command);
    }

    #[tokio::test]
    async fn test_shell_pool_manager_disabled() {
        init_test_logging();
        let config = ShellPoolConfig {
            enabled: false,
            ..Default::default()
        };

        let manager = ShellPoolManager::new(config);
        let shell = manager.get_shell("/tmp").await;
        assert!(shell.is_none());
    }

    #[tokio::test]
    async fn test_shell_pool_creation() {
        init_test_logging();
        let temp_dir = TempDir::new().unwrap();
        let config = ShellPoolConfig::default();

        let pool = ShellPool::new(temp_dir.path(), config);
        assert_eq!(pool.working_dir(), temp_dir.path());
    }
}
