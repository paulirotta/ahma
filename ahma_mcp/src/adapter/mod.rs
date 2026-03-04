//! # CLI Tool Adapter
//!
//! This module provides the core execution engine for running external command-line tools.
//! Its primary responsibility is to execute commands, either synchronously or asynchronously,
//! and manage a pool of reusable shell processes for performance.
//!
//! ## Core Components
//!
//! - **`Adapter`**: The central struct that manages command execution. It holds the
//!   `ShellPoolManager` for efficient asynchronous command execution and configuration
//!   for timeout and synchronous mode.
//!
//! ## Execution Flow
//!
//! 1. **Initialization**:
//!    - The `Adapter` is created with settings for synchronous mode and command timeouts.
//!    - It initializes a `ShellPoolManager` to handle a pool of warm shell processes,
//!      which reduces the overhead of spawning new processes for each command.
//!
//! 2. **Execution (`execute_tool_in_dir`)**:
//!    - A tool execution request is received with a base command, arguments, and a working directory.
//!    - The `Adapter` determines whether to run the command synchronously or asynchronously based
//!      on its `synchronous_mode` setting.
//!    - **Async Path**: If not in synchronous mode and a working directory is provided, a
//!      pre-warmed shell is requested from the `ShellPoolManager`. The command is sent to the
//!      shell for execution, and the shell is returned to the pool afterward. This is the
//!      preferred path for performance.
//!    - **Sync Path**: If in synchronous mode, or if no working directory is given (making the
//!      shell pool less effective), the command is executed as a standard `tokio::process::Command`.
//!
//! ## Key Design Decisions
//!
//! - **Stateless Execution**: The `Adapter` is stateless regarding tool definitions. It simply
//!   receives a command and arguments and executes them. All logic for tool discovery and
//!   argument parsing is handled by the `mcp_service` and `main` modules.
//! - **Performance-Oriented Async**: The integration with `ShellPoolManager` ensures that
//!   asynchronous operations are executed with minimal overhead, which is critical for a
//!   responsive user experience in a server context.
//!
//! ## Security Note
//! Kernel-enforced sandboxing is the only security boundary (R7). The adapter does not
//! attempt to parse command strings for security.

mod preparer;
mod types;

pub use preparer::{escape_shell_argument, format_option_flag, needs_file_handling};
pub use types::{AsyncExecOptions, ExecutionMode};

use crate::operation_monitor::{Operation, OperationMonitor, OperationStatus};
use crate::retry::{self, RetryConfig};
use crate::sandbox;
use crate::shell_pool::ShellPoolManager;
use anyhow::Result;
use serde_json::{Map, Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{sync::Mutex, task::JoinHandle};

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn generate_id() -> String {
    let id = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("op_{}", id)
}

const MAX_STREAM_COLLECTED_LINES: usize = 5_000;
const MAX_STREAM_COLLECTED_BYTES: usize = 1_000_000;

#[derive(Debug, Default)]
struct BoundedLineCollector {
    lines: VecDeque<String>,
    bytes: usize,
    dropped_lines: usize,
    dropped_bytes: usize,
}

impl BoundedLineCollector {
    fn push(&mut self, line: String) {
        self.bytes = self.bytes.saturating_add(line.len());
        self.lines.push_back(line);

        while self.lines.len() > MAX_STREAM_COLLECTED_LINES
            || self.bytes > MAX_STREAM_COLLECTED_BYTES
        {
            let Some(evicted) = self.lines.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(evicted.len());
            self.dropped_lines = self.dropped_lines.saturating_add(1);
            self.dropped_bytes = self.dropped_bytes.saturating_add(evicted.len());
        }
    }

    fn dropped_lines(&self) -> usize {
        self.dropped_lines
    }

    fn dropped_bytes(&self) -> usize {
        self.dropped_bytes
    }

    fn rendered_output(&self) -> String {
        let mut output = String::new();
        if self.dropped_lines > 0 {
            output.push_str(&format!(
                "[output truncated: dropped {} earlier lines ({} bytes)]\n",
                self.dropped_lines, self.dropped_bytes
            ));
        }

        output.push_str(
            &self
                .lines
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        );

        output
    }
}

/// The core execution engine for external tools.
///
/// The `Adapter` coordinates the execution of CLI commands, managing the lifecycle of
/// shell processes, operation tracking, and resource cleanup. It serves as the bridge
/// between incoming tool requests and the underlying system shell.
///
/// # Responsibilities
///
/// *   **Command Execution**: Executes tools using either a pre-warmed shell pool (for async
///     performance) or standard process spawning.
/// *   **Resource Management**: Manages temporary files created for complex arguments and
///     ensures they are cleaned up.
/// *   **Shell Pooling**: Integrates with `ShellPoolManager` to reuse shell processes,
///     reducing latency for frequent commands.
/// *   **Operation Tracking**: Uses `OperationMonitor` to track the status (running, completed,
///     failed) of asynchronous operations.
/// *   **Sandboxing**: Enforces path security by validating operations against a root directory.
/// *   **Retry Logic**: Applies configured retry policies for transient failures.
///
/// # Thread Safety
///
/// The `Adapter` is designed to be shared across threads (`Arc<Adapter>`) and uses internal
/// synchronization to manage state safely.
#[derive(Debug)]
pub struct Adapter {
    /// Operation monitor for async tasks.
    monitor: Arc<OperationMonitor>,
    /// Pre-warmed shell pool manager for async execution.
    shell_pool: Arc<ShellPoolManager>,
    /// Security sandbox context.
    sandbox: Arc<sandbox::Sandbox>,
    /// Handles to spawned tasks for graceful shutdown.
    task_handles: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    /// Temporary file manager for multi-line arguments - cleaned up automatically when dropped
    temp_file_manager: preparer::TempFileManager,
    /// Optional retry configuration for transient error handling.
    retry_config: Option<RetryConfig>,
}

impl Adapter {
    /// Creates a new `Adapter` instance.
    ///
    /// The adapter requires an `OperationMonitor` for tracking async tasks, a `ShellPoolManager`
    /// for efficient shell execution, and a `Sandbox` for security context.
    ///
    /// # Arguments
    ///
    /// * `monitor` - Shared reference to the operation monitor
    /// * `shell_pool` - Shared reference to the shell pool manager
    /// * `sandbox` - Shared reference to the security sandbox
    pub fn new(
        monitor: Arc<OperationMonitor>,
        shell_pool: Arc<ShellPoolManager>,
        sandbox: Arc<sandbox::Sandbox>,
    ) -> Result<Self> {
        Ok(Self {
            monitor,
            shell_pool,
            sandbox,
            task_handles: Arc::new(Mutex::new(HashMap::new())),
            temp_file_manager: preparer::TempFileManager::new(),
            retry_config: None,
        })
    }

    /// Sets retry configuration for transient error handling.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = Some(config);
        self
    }

    /// Returns the retry configuration, if set.
    pub fn retry_config(&self) -> Option<&RetryConfig> {
        self.retry_config.as_ref()
    }

    /// Gracefully shuts down the adapter by cancelling active operations, aborting tasks,
    /// and shutting down shell pools. Uses timeouts to avoid hanging indefinitely.
    pub async fn shutdown(&self) {
        tracing::info!("Adapter shutdown initiated: cancelling operations and aborting tasks");

        // 1) Cancel all known operations tracked by this adapter (best-effort)
        // We use the task handle keys which are the operation IDs
        {
            let handles = self.task_handles.lock().await;
            for op_id in handles.keys() {
                // Provide a clear reason for downstream logs
                let reason = Some("Adapter shutdown".to_string());
                let _ = self
                    .monitor
                    .cancel_operation_with_reason(op_id, reason)
                    .await;
            }
        }

        // 2) Abort all running tasks and wait briefly for them to finish
        // Drain handles to avoid races with task completion removing them concurrently
        let mut drained: Vec<(String, JoinHandle<()>)> = Vec::new();
        {
            let mut handles = self.task_handles.lock().await;
            for (id, handle) in handles.drain() {
                drained.push((id, handle));
            }
        }

        // First give a small grace period for tasks to finish naturally
        for (id, handle) in drained.iter_mut() {
            tracing::debug!("Waiting briefly for task {} to complete...", id);
            match tokio::time::timeout(Duration::from_millis(250), handle).await {
                Ok(res) => {
                    if let Err(e) = res {
                        tracing::debug!(
                            "Task {} finished with join error before abort: {:?}",
                            id,
                            e
                        );
                    }
                }
                Err(_) => {
                    tracing::debug!("Task {} did not complete in grace period", id);
                }
            }
        }

        // Abort any remaining tasks and await their termination with a bounded timeout
        for (id, mut handle) in drained {
            if !handle.is_finished() {
                tracing::info!("Aborting task {} during shutdown", id);
                handle.abort();
                // Await aborted join to ensure cleanup
                match tokio::time::timeout(Duration::from_secs(2), &mut handle).await {
                    Ok(join_res) => {
                        if let Err(e) = join_res {
                            tracing::debug!("Task {} aborted with: {:?}", id, e);
                        }
                    }
                    Err(_) => {
                        tracing::warn!("Timed out waiting for aborted task {} to finish", id);
                    }
                }
            }
        }

        // 3) Shut down all shell pools (kills any lingering shell processes)
        tracing::info!("Shutting down shell pools");
        self.shell_pool.shutdown_all().await;

        tracing::info!("Adapter shutdown complete");
    }

    /// Get a reference to the sandbox.
    pub fn sandbox(&self) -> &crate::sandbox::Sandbox {
        &self.sandbox
    }

    /// Synchronously executes a command and returns the result directly.
    ///
    /// This method bypasses the async operation queue and runs the command directly, waiting for it to complete.
    /// It captures and returns the combined stdout and stderr.
    ///
    /// # Arguments
    ///
    /// * `command` - The command to execute (e.g., "ls", "grep").
    /// * `args` - Optional map of arguments (see `prepare_command_and_args`).
    /// * `working_dir` - Directory to execute the command in. Must be within allowed sandbox scopes.
    /// * `timeout_seconds` - Optional timeout in seconds (overrides default).
    /// * `subcommand_config` - Optional configuration for dealing with subcommands and aliases.
    ///
    /// # Returns
    ///
    /// Returns the command output (stdout + stderr) as a `String` if successful.
    /// Returns an error if execution fails or times out.
    pub async fn execute_sync_in_dir(
        &self,
        command: &str,
        args: Option<Map<String, serde_json::Value>>,
        working_dir: &str,
        timeout_seconds: Option<u64>,
        subcommand_config: Option<&crate::config::SubcommandConfig>,
    ) -> Result<String, anyhow::Error> {
        tracing::error!(
            "execute_sync_in_dir START: command='{}', working_dir='{}', args={:?}",
            command,
            working_dir,
            args
        );

        // Validate working directory against sandbox scope.
        let safe_wd = self
            .sandbox
            .validate_path(std::path::Path::new(working_dir))?;

        let (program, args_vec) = self
            .prepare_command_and_args(command, args.as_ref(), subcommand_config, &safe_wd)
            .await?;

        tracing::info!(
            "Prepared command: program='{}', args={:?}",
            program,
            args_vec
        );

        let timeout = timeout_seconds
            .map(Duration::from_secs)
            .unwrap_or_else(|| self.shell_pool.config().command_timeout);

        // Create sandboxed command.
        // `create_shell_command` is only needed for raw /bin/sh invocations where
        // the caller has NOT already added the -c flag via a subcommand config.
        // For bash/pwsh the preparer already embeds -c/-Command; always use
        // create_command so the sandbox wrapper is applied without double-wrapping.
        let mut cmd = if program == "/bin/sh" {
            let full_command = args_vec.join(" ");
            self.sandbox
                .create_shell_command(&program, &full_command, &safe_wd)?
        } else {
            self.sandbox.create_command(&program, &args_vec, &safe_wd)?
        };

        let output_res = tokio::time::timeout(timeout, cmd.output()).await;

        match output_res {
            Err(_) => Err(anyhow::anyhow!(
                "Operation timed out (exceeded timeout limit): {} seconds",
                timeout.as_secs()
            )),
            Ok(Err(e)) => Err(anyhow::anyhow!("Command execution failed: {}", e)),
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                if output.status.success() {
                    let result_content = if stdout.is_empty() && !stderr.is_empty() {
                        stderr
                    } else if !stdout.is_empty() && !stderr.is_empty() {
                        format!("{}\n{}", stdout, stderr)
                    } else {
                        stdout
                    };
                    Ok(result_content)
                } else {
                    Err(anyhow::anyhow!(
                        "Command failed with exit code {}: stderr: {}, stdout: {}",
                        output.status.code().unwrap_or(-1),
                        stderr,
                        stdout
                    ))
                }
            }
        }
    }

    /// Synchronously executes a command with optional retry logic for transient errors.
    ///
    /// If a `RetryConfig` is set on the adapter, transient errors (timeouts, resource
    /// exhaustion, network issues) will be retried with exponential backoff.
    /// Permanent errors (command not found, permission denied) fail immediately.
    ///
    /// If no retry config is set, this behaves identically to `execute_sync_in_dir`.
    pub async fn execute_sync_with_retry(
        &self,
        command: &str,
        args: Option<Map<String, serde_json::Value>>,
        working_dir: &str,
        timeout_seconds: Option<u64>,
        subcommand_config: Option<&crate::config::SubcommandConfig>,
    ) -> Result<String, anyhow::Error> {
        match &self.retry_config {
            Some(config) => {
                // Clone args for each retry attempt since the closure needs ownership
                let args_clone = args.clone();
                retry::execute_with_retry(config, || {
                    let args_inner = args_clone.clone();
                    async move {
                        self.execute_sync_in_dir(
                            command,
                            args_inner,
                            working_dir,
                            timeout_seconds,
                            subcommand_config,
                        )
                        .await
                    }
                })
                .await
            }
            None => {
                // No retry config, execute directly
                self.execute_sync_in_dir(
                    command,
                    args,
                    working_dir,
                    timeout_seconds,
                    subcommand_config,
                )
                .await
            }
        }
    }

    /// Asynchronously starts a command, returning an operation ID immediately.
    ///
    /// This method queues the command for execution in a background task, potentially using a
    /// pre-warmed shell from the pool. The result will be reported via the `OperationMonitor`
    /// and any registered callbacks.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - The logical name of the tool (for logging/monitoring).
    /// * `command` - The base command to run.
    /// * `args` - Optional arguments for the command.
    /// * `working_directory` - Directory to execute in.
    /// * `timeout` - Optional timeout in seconds.
    ///
    /// # Returns
    ///
    /// Returns a `String` containing the `id` (e.g., "op_123").
    /// The actual command result is not returned here.
    pub async fn execute_async_in_dir(
        &self,
        tool_name: &str,
        command: &str,
        args: Option<Map<String, serde_json::Value>>,
        working_directory: &str,
        timeout: Option<u64>,
        callback: Option<Box<dyn crate::callback_system::CallbackSender>>,
    ) -> Result<String> {
        self.execute_async_in_dir_with_options(
            tool_name,
            command,
            working_directory,
            AsyncExecOptions {
                id: None,
                args,
                timeout,
                callback,
                subcommand_config: None,
                log_monitor_config: None,
            },
        )
        .await
    }

    /// Asynchronously starts a command using structured options to avoid long arg lists
    pub async fn execute_async_in_dir_with_options(
        &self,
        tool_name: &str,
        command: &str,
        working_dir: &str,
        options: AsyncExecOptions<'_>,
    ) -> Result<String> {
        let AsyncExecOptions {
            id,
            args,
            timeout,
            callback,
            subcommand_config,
            log_monitor_config,
        } = options;

        // Validate working directory against sandbox scope.
        let safe_wd = self
            .sandbox
            .validate_path(std::path::Path::new(working_dir))?;
        let safe_wd_str = safe_wd.to_string_lossy().to_string();

        // Validate command arguments
        let (program_with_subcommand, args_vec) = self
            .prepare_command_and_args(command, args.as_ref(), subcommand_config, &safe_wd)
            .await?;

        let op_id = id.unwrap_or_else(generate_id);
        let op_id_clone = op_id.clone();
        let wd = safe_wd_str.clone();

        let timeout_duration = timeout.map(Duration::from_secs);

        let operation = Operation::new_with_timeout(
            op_id.clone(),
            tool_name.to_string(),
            format!("{} {:?}", command, args),
            None,
            timeout_duration,
        );
        self.monitor.add_operation(operation).await;

        let monitor = self.monitor.clone();
        let shell_pool = self.shell_pool.clone();
        let sandbox = self.sandbox.clone(); // Clone ARC to pass to task
        let command = command.to_string();
        let wd_clone = wd.clone();

        let task_handles = self.task_handles.clone();

        let handle = tokio::spawn(async move {
            // Get the cancellation token from the operation
            let cancellation_token = {
                if let Some(operation) = monitor.get_operation(&op_id).await {
                    operation.cancellation_token.clone()
                } else {
                    tracing::error!("Could not find operation {} for cancellation token", op_id);
                    return;
                }
            };

            monitor
                .update_status(&op_id, OperationStatus::InProgress, None)
                .await;

            // Send an immediate 'Started' notification if a callback is provided
            if let Some(callback) = &callback {
                let _ = callback
                    .send_progress(crate::callback_system::ProgressUpdate::Started {
                        id: op_id.clone(),
                        command: command.clone(),
                        description: format!("Execute {} in {}", command, wd_clone),
                    })
                    .await;
            }

            // Check for cancellation before starting
            if cancellation_token.is_cancelled() {
                tracing::info!("Operation {} was cancelled before execution started", op_id);
                monitor
                    .update_status(
                        &op_id,
                        OperationStatus::Cancelled,
                        Some(Value::String("Operation was cancelled".to_string())),
                    )
                    .await;
                // Notify LLM about cancellation with reason if available
                if let Some(callback) = &callback {
                    // Best effort: retrieve reason string from current operation state
                    let reason_owned = match monitor.get_operation(&op_id).await {
                        Some(op) => {
                            let val = op.result.clone();
                            val.and_then(|v| v.get("reason").cloned())
                                .and_then(|v| v.as_str().map(|s| s.to_string()))
                                .unwrap_or_else(|| "Operation was cancelled".to_string())
                        }
                        None => "Operation was cancelled".to_string(),
                    };
                    let _ = callback
                        .send_progress(crate::callback_system::ProgressUpdate::Cancelled {
                            id: op_id.clone(),
                            message: reason_owned,
                            duration_ms: 0,
                        })
                        .await;
                }
                return;
            }

            let start_time = Instant::now();

            // Build sandboxed process command.
            // `create_shell_command` injects -c (Unix) or -NoProfile … -Command (Windows).
            // Only use it for raw /bin/sh invocations where no -c flag was pre-added.
            // For bash/pwsh the preparer already adds the flag; create_command applies
            // the sandbox wrapper without double-wrapping.
            let wd_path = std::path::PathBuf::from(&wd_clone);
            let proc_cmd_result = if program_with_subcommand == "/bin/sh" {
                let full_command = args_vec.join(" ");
                sandbox.create_shell_command(&program_with_subcommand, &full_command, &wd_path)
            } else {
                sandbox.create_command(&program_with_subcommand, &args_vec, &wd_path)
            };

            let mut proc_cmd: tokio::process::Command = match proc_cmd_result {
                Ok(cmd) => cmd,
                Err(e) => {
                    let error_message = format!("Failed to create sandboxed command: {}", e);
                    tracing::error!("{}", error_message);
                    monitor
                        .update_status(
                            &op_id,
                            OperationStatus::Failed,
                            Some(Value::String(error_message.clone())),
                        )
                        .await;
                    if let Some(callback) = &callback {
                        let failure_update = crate::callback_system::ProgressUpdate::FinalResult {
                            id: op_id.clone(),
                            command: program_with_subcommand.clone(),
                            description: format!(
                                "Execute {} in {}",
                                program_with_subcommand, wd_clone
                            ),
                            working_directory: wd_clone.clone(),
                            success: false,
                            duration_ms: 0,
                            full_output: format!("Error: {}", error_message),
                        };
                        let _ = callback.send_progress(failure_update).await;
                    }
                    return;
                }
            };

            // Resolve timeout in milliseconds
            let timeout_ms: u64 = timeout
                .map(|t| t * 1000)
                .unwrap_or_else(|| shell_pool.config().command_timeout.as_millis() as u64);

            // Branch: streaming execution for log monitoring vs. batch execution
            if let Some(monitor_config) = log_monitor_config {
                // === STREAMING PATH: line-by-line output with log monitoring ===
                execute_with_streaming(
                    &mut proc_cmd,
                    timeout_ms,
                    monitor_config,
                    &cancellation_token,
                    &callback,
                    &op_id,
                    &program_with_subcommand,
                    &wd_clone,
                    start_time,
                    &monitor,
                )
                .await;
            } else {
                // === BATCH PATH: existing behavior, collect all output at once ===
                execute_batch(
                    &mut proc_cmd,
                    timeout_ms,
                    &cancellation_token,
                    &callback,
                    &op_id,
                    &program_with_subcommand,
                    &wd_clone,
                    start_time,
                    &monitor,
                )
                .await;
            }

            // Remove the task handle from the map once it's complete
            task_handles.lock().await.remove(&op_id);
        });

        // Store the handle for graceful shutdown
        self.task_handles
            .lock()
            .await
            .insert(op_id_clone.clone(), handle);

        Ok(op_id_clone)
    }

    /// Parses the command string and arguments into a program and argument list.
    ///
    /// This helper handles complications such as:
    /// - Splitting the base command string (e.g., "python script.py" -> program: "python", args: ["script.py"])
    /// - Converting structured JSON arguments into CLI flags and positional arguments
    /// - Applying subcommand configurations (aliases, hardcoded args)
    /// - Creating temporary files for multi-line string arguments
    async fn prepare_command_and_args(
        &self,
        command: &str,
        args: Option<&Map<String, Value>>,
        subcommand_config: Option<&crate::config::SubcommandConfig>,
        working_dir: &std::path::Path,
    ) -> Result<(String, Vec<String>)> {
        preparer::prepare_command_and_args(
            command,
            args,
            subcommand_config,
            working_dir,
            &self.temp_file_manager,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Extracted execution helpers (batch vs streaming)
// ---------------------------------------------------------------------------

/// Execute a command in batch mode (existing behavior): collect all output at once.
#[allow(clippy::too_many_arguments)]
async fn execute_batch(
    proc_cmd: &mut tokio::process::Command,
    timeout_ms: u64,
    cancellation_token: &tokio_util::sync::CancellationToken,
    callback: &Option<Box<dyn crate::callback_system::CallbackSender>>,
    op_id: &str,
    program: &str,
    working_dir: &str,
    start_time: Instant,
    monitor: &Arc<OperationMonitor>,
) {
    let proc_result =
        tokio::time::timeout(Duration::from_millis(timeout_ms), proc_cmd.output()).await;

    let duration_ms = start_time.elapsed().as_millis() as u64;

    // Check for cancellation after command execution
    if cancellation_token.is_cancelled() {
        tracing::info!("Operation {} was cancelled after shell execution", op_id);
        handle_cancellation(monitor, callback, op_id, duration_ms).await;
        return;
    }

    match proc_result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let final_output = json!({
                "stdout": stdout,
                "stderr": stderr,
                "exit_code": output.status.code().unwrap_or(-1),
            });
            let success = output.status.success();
            let status = if success {
                OperationStatus::Completed
            } else {
                OperationStatus::Failed
            };
            monitor
                .update_status(op_id, status, Some(final_output.clone()))
                .await;

            if let Some(callback) = callback {
                let completion_update = crate::callback_system::ProgressUpdate::FinalResult {
                    id: op_id.to_string(),
                    command: program.to_string(),
                    description: format!("Execute {} in {}", program, working_dir),
                    working_directory: working_dir.to_string(),
                    success,
                    duration_ms,
                    full_output: format!(
                        "Exit code: {}\nStdout:\n{}\nStderr:\n{}",
                        final_output["exit_code"], final_output["stdout"], final_output["stderr"]
                    ),
                };
                if let Err(e) = callback.send_progress(completion_update).await {
                    tracing::error!("Failed to send completion notification: {:?}", e);
                } else {
                    tracing::info!("Sent completion notification for operation: {}", op_id);
                }
            }
        }
        Ok(Err(e)) => {
            let error_message = e.to_string();
            monitor
                .update_status(
                    op_id,
                    OperationStatus::Failed,
                    Some(Value::String(error_message.clone())),
                )
                .await;

            if let Some(callback) = callback {
                let failure_update = crate::callback_system::ProgressUpdate::FinalResult {
                    id: op_id.to_string(),
                    command: program.to_string(),
                    description: format!("Execute {} in {}", program, working_dir),
                    working_directory: working_dir.to_string(),
                    success: false,
                    duration_ms,
                    full_output: format!("Error: {}", error_message),
                };
                let _ = callback.send_progress(failure_update).await;
            }
        }
        Err(_) => {
            let timeout_reason = format!(
                "Operation timed out after {}ms (exceeded timeout limit)",
                duration_ms
            );
            monitor
                .update_status(
                    op_id,
                    OperationStatus::Cancelled,
                    Some(Value::String(timeout_reason.clone())),
                )
                .await;

            if let Some(callback) = callback {
                let _ = callback
                    .send_progress(crate::callback_system::ProgressUpdate::Cancelled {
                        id: op_id.to_string(),
                        message: timeout_reason,
                        duration_ms,
                    })
                    .await;
            }
        }
    }
}

/// Execute a command with line-by-line streaming and log monitoring.
///
/// Instead of buffering all output, this spawns the process and reads stdout/stderr
/// concurrently via `BufReader::lines()`. Each line is fed through a `LogMonitor`
/// which checks for error/warning patterns and fires alerts via the callback.
#[allow(clippy::too_many_arguments)]
async fn execute_with_streaming(
    proc_cmd: &mut tokio::process::Command,
    timeout_ms: u64,
    monitor_config: crate::log_monitor::LogMonitorConfig,
    cancellation_token: &tokio_util::sync::CancellationToken,
    callback: &Option<Box<dyn crate::callback_system::CallbackSender>>,
    op_id: &str,
    program: &str,
    working_dir: &str,
    start_time: Instant,
    op_monitor: &Arc<OperationMonitor>,
) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    // Ensure stdout/stderr are piped (should already be set by sandbox)
    proc_cmd.stdout(std::process::Stdio::piped());
    proc_cmd.stderr(std::process::Stdio::piped());

    let spawn_result = proc_cmd.spawn();
    let mut child = match spawn_result {
        Ok(child) => child,
        Err(e) => {
            let error_message = format!("Failed to spawn process: {}", e);
            tracing::error!("{}", error_message);
            op_monitor
                .update_status(
                    op_id,
                    OperationStatus::Failed,
                    Some(Value::String(error_message.clone())),
                )
                .await;
            if let Some(callback) = callback {
                let _ = callback
                    .send_progress(crate::callback_system::ProgressUpdate::FinalResult {
                        id: op_id.to_string(),
                        command: program.to_string(),
                        description: format!("Execute {} in {}", program, working_dir),
                        working_directory: working_dir.to_string(),
                        success: false,
                        duration_ms: 0,
                        full_output: format!("Error: {}", error_message),
                    })
                    .await;
            }
            return;
        }
    };

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();

    let mut log_monitor = crate::log_monitor::LogMonitor::new(monitor_config);

    // Collected output for the final result (bounded to prevent unbounded memory growth).
    let mut collected_stdout = BoundedLineCollector::default();
    let mut collected_stderr = BoundedLineCollector::default();

    let timeout_deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        tokio::select! {
            // Bias stderr to prioritize error-related output
            biased;

            // Check cancellation
            _ = cancellation_token.cancelled() => {
                tracing::info!("Operation {} cancelled during streaming", op_id);
                let _ = child.kill().await;
                let duration_ms = start_time.elapsed().as_millis() as u64;
                handle_cancellation(op_monitor, callback, op_id, duration_ms).await;
                return;
            }

            // Timeout
            _ = tokio::time::sleep_until(timeout_deadline) => {
                tracing::warn!("Operation {} timed out during streaming", op_id);
                let _ = child.kill().await;
                let duration_ms = start_time.elapsed().as_millis() as u64;
                let timeout_reason = format!(
                    "Operation timed out after {}ms (exceeded timeout limit)", duration_ms
                );
                op_monitor
                    .update_status(
                        op_id,
                        OperationStatus::Cancelled,
                        Some(Value::String(timeout_reason.clone())),
                    )
                    .await;
                if let Some(callback) = callback {
                    let _ = callback
                        .send_progress(crate::callback_system::ProgressUpdate::Cancelled {
                            id: op_id.to_string(),
                            message: timeout_reason,
                            duration_ms,
                        })
                        .await;
                }
                return;
            }

            // Read stderr line
            result = stderr_reader.next_line() => {
                match result {
                    Ok(Some(line)) => {
                        let safe_line = crate::log_monitor::redact_sensitive_line(&line);
                        collected_stderr.push(safe_line);
                        if let Some(snapshot) = log_monitor.process_line(&line, true)
                            && let Some(callback) = callback {
                                let alert = crate::callback_system::ProgressUpdate::LogAlert {
                                    id: op_id.to_string(),
                                    trigger_level: snapshot.trigger_level.to_string(),
                                    context_snapshot: snapshot.format_for_notification(),
                                };
                                let _ = callback.send_progress(alert).await;
                            }
                    }
                    Ok(None) => {
                        // stderr stream closed — wait for process to exit
                        // stdout may still be open, continue the loop to drain it
                    }
                    Err(e) => {
                        tracing::warn!("Error reading stderr for {}: {}", op_id, e);
                    }
                }
            }

            // Read stdout line
            result = stdout_reader.next_line() => {
                match result {
                    Ok(Some(line)) => {
                        let safe_line = crate::log_monitor::redact_sensitive_line(&line);
                        collected_stdout.push(safe_line);
                        if let Some(snapshot) = log_monitor.process_line(&line, false)
                            && let Some(callback) = callback {
                                let alert = crate::callback_system::ProgressUpdate::LogAlert {
                                    id: op_id.to_string(),
                                    trigger_level: snapshot.trigger_level.to_string(),
                                    context_snapshot: snapshot.format_for_notification(),
                                };
                                let _ = callback.send_progress(alert).await;
                            }
                    }
                    Ok(None) => {
                        // stdout stream closed
                    }
                    Err(e) => {
                        tracing::warn!("Error reading stdout for {}: {}", op_id, e);
                    }
                }
            }
        }

        // Check if the child process has exited
        // We use try_wait() to avoid blocking — if streams are closed the process may
        // have already exited.
        match child.try_wait() {
            Ok(Some(_status)) => {
                // Process exited. Drain remaining lines from both streams.
                while let Ok(Some(line)) = stderr_reader.next_line().await {
                    let safe_line = crate::log_monitor::redact_sensitive_line(&line);
                    collected_stderr.push(safe_line);
                    if let Some(snapshot) = log_monitor.process_line(&line, true)
                        && let Some(callback) = callback
                    {
                        let alert = crate::callback_system::ProgressUpdate::LogAlert {
                            id: op_id.to_string(),
                            trigger_level: snapshot.trigger_level.to_string(),
                            context_snapshot: snapshot.format_for_notification(),
                        };
                        let _ = callback.send_progress(alert).await;
                    }
                }
                while let Ok(Some(line)) = stdout_reader.next_line().await {
                    let safe_line = crate::log_monitor::redact_sensitive_line(&line);
                    collected_stdout.push(safe_line);
                    if let Some(snapshot) = log_monitor.process_line(&line, false)
                        && let Some(callback) = callback
                    {
                        let alert = crate::callback_system::ProgressUpdate::LogAlert {
                            id: op_id.to_string(),
                            trigger_level: snapshot.trigger_level.to_string(),
                            context_snapshot: snapshot.format_for_notification(),
                        };
                        let _ = callback.send_progress(alert).await;
                    }
                }
                break;
            }
            Ok(None) => {
                // Process still running, continue loop
            }
            Err(e) => {
                tracing::warn!("Error checking child status for {}: {}", op_id, e);
                break;
            }
        }
    }

    // Wait for the process to finish and get the exit status
    let exit_status = child.wait().await;
    let duration_ms = start_time.elapsed().as_millis() as u64;

    let exit_code = match &exit_status {
        Ok(status) => status.code().unwrap_or(-1),
        Err(_) => -1,
    };
    let success = exit_status.as_ref().map(|s| s.success()).unwrap_or(false);

    let stdout_str = collected_stdout.rendered_output();
    let stderr_str = collected_stderr.rendered_output();

    let final_output = json!({
        "stdout": stdout_str,
        "stderr": stderr_str,
        "exit_code": exit_code,
        "stdout_truncated_lines": collected_stdout.dropped_lines(),
        "stderr_truncated_lines": collected_stderr.dropped_lines(),
        "stdout_truncated_bytes": collected_stdout.dropped_bytes(),
        "stderr_truncated_bytes": collected_stderr.dropped_bytes(),
    });

    let status = if success {
        OperationStatus::Completed
    } else {
        OperationStatus::Failed
    };
    op_monitor
        .update_status(op_id, status, Some(final_output.clone()))
        .await;

    if let Some(callback) = callback {
        let completion_update = crate::callback_system::ProgressUpdate::FinalResult {
            id: op_id.to_string(),
            command: program.to_string(),
            description: format!("Execute {} in {}", program, working_dir),
            working_directory: working_dir.to_string(),
            success,
            duration_ms,
            full_output: format!(
                "Exit code: {}\nStdout:\n{}\nStderr:\n{}",
                exit_code, stdout_str, stderr_str
            ),
        };
        if let Err(e) = callback.send_progress(completion_update).await {
            tracing::error!("Failed to send completion notification: {:?}", e);
        } else {
            tracing::info!("Sent completion notification for operation: {}", op_id);
        }
    }
}

/// Handle cancellation of an operation — shared logic for both execution paths.
async fn handle_cancellation(
    monitor: &Arc<OperationMonitor>,
    callback: &Option<Box<dyn crate::callback_system::CallbackSender>>,
    op_id: &str,
    duration_ms: u64,
) {
    monitor
        .update_status(
            op_id,
            OperationStatus::Cancelled,
            Some(Value::String("Operation was cancelled".to_string())),
        )
        .await;
    if let Some(callback) = callback {
        let reason_owned = match monitor.get_operation(op_id).await {
            Some(op) => {
                let val = op.result.clone();
                val.and_then(|v| v.get("reason").cloned())
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "Operation was cancelled".to_string())
            }
            None => "Operation was cancelled".to_string(),
        };
        let _ = callback
            .send_progress(crate::callback_system::ProgressUpdate::Cancelled {
                id: op_id.to_string(),
                message: reason_owned,
                duration_ms,
            })
            .await;
    }
}
