//! Stress Test for AHMA HTTP Bridge
//!
//! This example spawns an AHMA HTTP server and runs multiple concurrent clients
//! to stress test the server's capabilities. It exercises both synchronous and
//! asynchronous command execution patterns.
//!
//! # Usage
//!
//! ```bash
//! # Run with defaults (port 7634, 60 second duration)
//! cargo run --example stress_test
//!
//! # Custom port and duration
//! cargo run --example stress_test -- --port 8080 --duration 120
//!
//! # Use existing binary instead of cargo run
//! cargo run --example stress_test -- --binary ./target/release/ahma-mcp
//! ```
//!
//! # Architecture
//!
//! - Spawns 1 HTTP server on the specified port
//! - Spawns 4 concurrent clients:
//!   - 1 synchronous client (waits for each command to complete)
//!   - 3 asynchronous clients (fire-and-forget pattern)
//! - Each client runs a randomized sequence of safe shell commands
//! - Server stderr is monitored; any error output stops the test immediately
//!
//! # Command Pool
//!
//! The test uses safe, read-only commands that don't modify the repository:
//! - `echo`: Tests basic command execution
//! - `ls -la`: Tests directory listing
//! - `whoami`: Tests user context
//! - `date`: Tests system commands
//! - `pwd`: Tests working directory
//! - `uname -a`: Tests system info
//! - `env | grep PATH`: Tests environment access
//! - `cat /etc/hosts | head -5`: Tests file reading (non-repo)

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinSet;

const ROOTS_LIST_TIMEOUT_SECS: u64 = 10;
const SANDBOX_CONFIG_TIMEOUT_SECS: u64 = 45;
const TOOL_CALL_RETRY_WINDOW_SECS: u64 = 10;
const TOOL_CALL_RETRY_BACKOFF_MS: u64 = 100;

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

/// Command-line arguments for the stress test
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port to run the server on
    #[arg(long, default_value_t = 7634)]
    port: u16,

    /// Duration of the test in seconds
    #[arg(long, default_value_t = 60)]
    duration: u64,

    /// Path to ahma_mcp binary (if not provided, uses cargo run)
    #[arg(long)]
    binary: Option<String>,

    /// Number of async clients (in addition to 1 sync client)
    #[arg(long, default_value_t = 3)]
    async_clients: usize,
}

// ---------------------------------------------------------------------------
// Deterministic RNG (avoids `rand` dependency)
// ---------------------------------------------------------------------------

/// Simple Linear Congruential Generator for deterministic randomization
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    fn next_range(&mut self, max: usize) -> usize {
        (self.next() as usize) % max
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.next_range(i + 1);
            items.swap(i, j);
        }
    }
}

// ---------------------------------------------------------------------------
// Async progress tracking (for non-sync clients)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AsyncProgressState {
    pending_tokens: Arc<Mutex<HashSet<String>>>,
    completed_success: Arc<AtomicU64>,
    completed_error: Arc<AtomicU64>,
}

impl AsyncProgressState {
    fn new(completed_success: Arc<AtomicU64>, completed_error: Arc<AtomicU64>) -> Self {
        Self {
            pending_tokens: Arc::new(Mutex::new(HashSet::new())),
            completed_success,
            completed_error,
        }
    }

    async fn track_completion(&self, token: &str, message: &str) {
        let removed = self.pending_tokens.lock().await.remove(token);
        if removed {
            let is_failure = message.starts_with("Failed:") || message.contains("OPERATION FAILED");
            if is_failure {
                self.completed_error.fetch_add(1, Ordering::Relaxed);
            } else {
                self.completed_success.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared counters used by all clients
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct SharedCounters {
    success: Arc<AtomicU64>,
    error: Arc<AtomicU64>,
    async_success: Arc<AtomicU64>,
    async_error: Arc<AtomicU64>,
}

impl SharedCounters {
    fn new() -> Self {
        Self {
            success: Arc::new(AtomicU64::new(0)),
            error: Arc::new(AtomicU64::new(0)),
            async_success: Arc::new(AtomicU64::new(0)),
            async_error: Arc::new(AtomicU64::new(0)),
        }
    }
}

// ---------------------------------------------------------------------------
// StressClient
// ---------------------------------------------------------------------------

struct StressClient {
    client: Client,
    base_url: String,
    is_sync: bool,
    session_id: Option<String>,
    request_id: AtomicU64,
    success_count: Arc<AtomicU64>,
    error_count: Arc<AtomicU64>,
    async_progress_state: Option<AsyncProgressState>,
    sse_task: Option<tokio::task::JoinHandle<()>>,
}

impl StressClient {
    fn new(base_url: String, is_sync: bool, counters: &SharedCounters) -> Self {
        let async_progress_state = (!is_sync).then(|| {
            AsyncProgressState::new(counters.async_success.clone(), counters.async_error.clone())
        });
        Self {
            client: Client::new(),
            base_url,
            is_sync,
            session_id: None,
            request_id: AtomicU64::new(0),
            success_count: counters.success.clone(),
            error_count: counters.error.clone(),
            async_progress_state,
            sse_task: None,
        }
    }

    fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url)
    }

    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::SeqCst)
    }

    fn session_id(&self) -> Result<&str> {
        self.session_id
            .as_deref()
            .ok_or_else(|| anyhow!("No session ID"))
    }

    // -- Initialization (MCP handshake) ------------------------------------

    async fn initialize(&mut self) -> Result<()> {
        let session_id = self.send_initialize().await?;
        self.session_id = Some(session_id.clone());

        let sse_response = self.open_sse_stream(&session_id).await?;
        self.send_initialized_notification(&session_id).await?;

        let ready_rx = self.spawn_sse_task(sse_response, &session_id);
        ready_rx.await.map_err(|_| {
            anyhow!("SSE setup channel closed before sandbox handshake completed")
        })??;
        Ok(())
    }

    async fn send_initialize(&self) -> Result<String> {
        let id = self.next_id();
        let response = self
            .client
            .post(self.mcp_url())
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "roots": { "listChanged": true } },
                    "clientInfo": { "name": "stress-test", "version": "0.1.0" }
                },
                "id": id
            }))
            .send()
            .await
            .context("Failed to send initialize request")?;

        if !response.status().is_success() {
            return Err(anyhow!("Initialize failed: {}", response.status()));
        }

        response
            .headers()
            .get("mcp-session-id")
            .ok_or_else(|| anyhow!("No mcp-session-id header in initialize response"))?
            .to_str()
            .map(|s| s.to_string())
            .map_err(|e| anyhow!("Invalid mcp-session-id header: {}", e))
    }

    async fn open_sse_stream(&self, session_id: &str) -> Result<reqwest::Response> {
        let response = self
            .client
            .get(self.mcp_url())
            .header("mcp-session-id", session_id)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .context("Failed to open SSE stream")?;

        if !response.status().is_success() {
            return Err(anyhow!("SSE connection failed: {}", response.status()));
        }
        Ok(response)
    }

    async fn send_initialized_notification(&self, session_id: &str) -> Result<()> {
        self.client
            .post(self.mcp_url())
            .header("mcp-session-id", session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))
            .send()
            .await
            .context("Failed to send initialized notification")?;
        Ok(())
    }

    fn spawn_sse_task(
        &mut self,
        sse_response: reqwest::Response,
        session_id: &str,
    ) -> oneshot::Receiver<Result<()>> {
        let post_client = self.client.clone();
        let post_url = self.mcp_url();
        let post_session_id = session_id.to_string();
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let progress_state = self.async_progress_state.clone();
        let (ready_tx, ready_rx) = oneshot::channel::<Result<()>>();

        self.sse_task = Some(tokio::spawn(async move {
            if let Err(e) = run_sse_loop(
                &post_client,
                &post_url,
                &post_session_id,
                &cwd,
                sse_response,
                progress_state,
                ready_tx,
            )
            .await
            {
                eprintln!("SSE task error for session {}: {}", post_session_id, e);
            }
        }));

        ready_rx
    }

    // -- Command execution loop --------------------------------------------

    async fn run_commands(
        &mut self,
        mut commands: Vec<Vec<String>>,
        mut lcg: Lcg,
        stop_flag: Arc<AtomicBool>,
    ) -> Result<()> {
        lcg.shuffle(&mut commands);

        let mut command_idx = 0;
        while !stop_flag.load(Ordering::Relaxed) {
            let cmd = &commands[command_idx % commands.len()];
            command_idx += 1;

            match self.send_tool_call(cmd).await {
                Ok(_) => {
                    self.success_count.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    eprintln!("Request error: {}", e);
                    self.error_count.fetch_add(1, Ordering::Relaxed);
                }
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        if let Some(handle) = self.sse_task.take() {
            handle.abort();
        }
        Ok(())
    }

    // -- Single tool call with retry ---------------------------------------

    async fn send_tool_call(&mut self, command: &[String]) -> Result<()> {
        let id = self.next_id();
        let cmd_str = command.join(" ");
        let session_id = self.session_id()?.to_string();

        let progress_token = self.prepare_progress_token(&session_id, id).await;
        let execution_mode = if self.is_sync {
            "Synchronous"
        } else {
            "AsyncResultPush"
        };

        let body = self
            .retry_tool_call(
                &session_id,
                id,
                &cmd_str,
                execution_mode,
                progress_token.as_deref(),
            )
            .await?;

        if let Some(error) = body.get("error") {
            self.clear_pending_token(progress_token.as_deref()).await;
            return Err(anyhow!("JSON-RPC error: {}", error));
        }
        Ok(())
    }

    async fn prepare_progress_token(&self, session_id: &str, id: u64) -> Option<String> {
        if self.is_sync {
            return None;
        }
        let token = format!("stress-{}-{}", session_id, id);
        if let Some(state) = &self.async_progress_state {
            state.pending_tokens.lock().await.insert(token.clone());
        }
        Some(token)
    }

    async fn retry_tool_call(
        &self,
        session_id: &str,
        id: u64,
        cmd_str: &str,
        execution_mode: &str,
        progress_token: Option<&str>,
    ) -> Result<Value> {
        let mut attempt = 0_u32;
        let retry_deadline = Instant::now() + Duration::from_secs(TOOL_CALL_RETRY_WINDOW_SECS);

        loop {
            attempt += 1;
            let body = self
                .post_tool_call(session_id, id, cmd_str, execution_mode, progress_token)
                .await?;

            match body {
                ToolCallResult::Retry if Instant::now() < retry_deadline => {
                    tokio::time::sleep(Duration::from_millis(TOOL_CALL_RETRY_BACKOFF_MS)).await;
                    continue;
                }
                ToolCallResult::Retry => {
                    self.clear_pending_token(progress_token).await;
                    return Err(anyhow!("Request failed: sandbox initialization timeout"));
                }
                ToolCallResult::Failure(msg) => {
                    self.clear_pending_token(progress_token).await;
                    return Err(anyhow!("{}", msg));
                }
                ToolCallResult::Success(value) => {
                    if attempt > 1 {
                        println!(
                            "Recovered tools/call after {} retry attempt(s) due to sandbox initialization race",
                            attempt - 1
                        );
                    }
                    return Ok(value);
                }
            }
        }
    }

    async fn post_tool_call(
        &self,
        session_id: &str,
        id: u64,
        cmd_str: &str,
        execution_mode: &str,
        progress_token: Option<&str>,
    ) -> Result<ToolCallResult> {
        let mut params = json!({
            "name": "sandboxed_shell",
            "arguments": {
                "command": cmd_str,
                "working_directory": ".",
                "synchronous": self.is_sync,
                "execution_mode": execution_mode
            }
        });
        if let Some(token) = progress_token {
            params["_meta"] = json!({ "progressToken": token });
        }

        let response = self
            .client
            .post(self.mcp_url())
            .header("mcp-session-id", session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "tools/call",
                "params": params,
                "id": id
            }))
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send request: {}", e))?;

        if response.status() == reqwest::StatusCode::CONFLICT {
            let body_text = response.text().await.unwrap_or_default();
            if body_text.contains("Sandbox initializing from client roots") {
                return Ok(ToolCallResult::Retry);
            }
            return Ok(ToolCallResult::Failure(format!(
                "Request failed (409): {}",
                body_text
            )));
        }

        if !response.status().is_success() {
            return Ok(ToolCallResult::Failure(format!(
                "Request failed: {}",
                response.status()
            )));
        }

        let body: Value = response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse response body: {}", e))?;

        if is_retryable_sandbox_error(&body) {
            return Ok(ToolCallResult::Retry);
        }

        Ok(ToolCallResult::Success(body))
    }

    async fn clear_pending_token(&self, token: Option<&str>) {
        if let Some(token) = token
            && let Some(state) = &self.async_progress_state
        {
            state.pending_tokens.lock().await.remove(token);
        }
    }
}

/// Outcome of a single tool-call HTTP round-trip
enum ToolCallResult {
    Success(Value),
    Retry,
    Failure(String),
}

fn is_retryable_sandbox_error(body: &Value) -> bool {
    let Some(error) = body.get("error") else {
        return false;
    };
    let code_match = error.get("code").and_then(|c| c.as_i64()) == Some(-32001);
    let msg_match = error
        .get("message")
        .and_then(|m| m.as_str())
        .is_some_and(|m| m.contains("Sandbox initializing from client roots"));
    code_match || msg_match
}

// ---------------------------------------------------------------------------
// SSE event loop
// ---------------------------------------------------------------------------

async fn run_sse_loop(
    post_client: &Client,
    post_url: &str,
    session_id: &str,
    cwd: &str,
    sse_response: reqwest::Response,
    progress_state: Option<AsyncProgressState>,
    ready_tx: oneshot::Sender<Result<()>>,
) -> Result<()> {
    let mut ctx = SseLoopContext {
        post_client,
        post_url,
        session_id,
        cwd,
        progress_state,
        ready_tx: Some(ready_tx),
        answered_roots: false,
        saw_sandbox_configured: false,
        handshake_completed: false,
    };

    let mut buffer = String::new();
    let mut stream = sse_response.bytes_stream();
    let roots_deadline = Instant::now() + Duration::from_secs(ROOTS_LIST_TIMEOUT_SECS);
    let sandbox_deadline = Instant::now() + Duration::from_secs(SANDBOX_CONFIG_TIMEOUT_SECS);

    loop {
        ctx.check_timeouts(roots_deadline, sandbox_deadline)?;

        match tokio::time::timeout(Duration::from_millis(500), stream.next()).await {
            Ok(Some(Ok(chunk))) => buffer.push_str(&String::from_utf8_lossy(&chunk)),
            Ok(Some(Err(e))) => {
                return ctx.signal_error(anyhow!("Failed to read SSE chunk: {}", e));
            }
            Ok(None) => return ctx.signal_error(anyhow!("SSE stream ended")),
            Err(_) => continue,
        }

        while let Some(event_end) = buffer.find("\n\n") {
            let event_block = buffer[..event_end].to_string();
            buffer = buffer[event_end + 2..].to_string();

            if let Some(msg) = parse_sse_event(&event_block) {
                ctx.handle_message(&msg).await?;
            }
        }
    }
}

struct SseLoopContext<'a> {
    post_client: &'a Client,
    post_url: &'a str,
    session_id: &'a str,
    cwd: &'a str,
    progress_state: Option<AsyncProgressState>,
    ready_tx: Option<oneshot::Sender<Result<()>>>,
    answered_roots: bool,
    saw_sandbox_configured: bool,
    handshake_completed: bool,
}

impl<'a> SseLoopContext<'a> {
    fn check_timeouts(&mut self, roots_deadline: Instant, sandbox_deadline: Instant) -> Result<()> {
        if !self.answered_roots && Instant::now() > roots_deadline {
            return self.signal_error(anyhow!(
                "Timeout waiting for roots/list request from server ({}s)",
                ROOTS_LIST_TIMEOUT_SECS
            ));
        }
        if !self.handshake_completed && self.answered_roots && Instant::now() > sandbox_deadline {
            return self.signal_error(anyhow!(
                "Timeout waiting for sandbox/configured notification ({}s)",
                SANDBOX_CONFIG_TIMEOUT_SECS
            ));
        }
        Ok(())
    }

    fn signal_error(&mut self, err: anyhow::Error) -> Result<()> {
        if let Some(tx) = self.ready_tx.take() {
            let _ = tx.send(Err(anyhow!(err.to_string())));
        }
        Err(err)
    }

    fn signal_ready(&mut self) {
        if !self.handshake_completed && self.answered_roots && self.saw_sandbox_configured {
            self.handshake_completed = true;
            if let Some(tx) = self.ready_tx.take() {
                let _ = tx.send(Ok(()));
            }
        }
    }

    async fn handle_message(&mut self, msg: &Value) -> Result<()> {
        let method = msg.get("method").and_then(|m| m.as_str());
        match method {
            Some("notifications/sandbox/failed") => self.handle_sandbox_failed(msg),
            Some("notifications/sandbox/configured") => {
                self.saw_sandbox_configured = true;
                self.signal_ready();
                Ok(())
            }
            Some("roots/list") if !self.answered_roots => self.handle_roots_list(msg).await,
            Some("notifications/progress") => {
                self.handle_progress(msg).await;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn handle_sandbox_failed(&mut self, msg: &Value) -> Result<()> {
        let error = msg
            .get("params")
            .and_then(|p| p.get("error"))
            .and_then(|e| e.as_str())
            .unwrap_or("unknown");
        self.signal_error(anyhow!("Sandbox configuration failed: {}", error))
    }

    async fn handle_roots_list(&mut self, msg: &Value) -> Result<()> {
        let req_id = msg
            .get("id")
            .ok_or_else(|| anyhow!("roots/list request missing id"))?;

        let response = self
            .post_client
            .post(self.post_url)
            .header("mcp-session-id", self.session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "roots": [{
                        "uri": format!("file://{}", self.cwd),
                        "name": "workspace"
                    }]
                }
            }))
            .send()
            .await
            .context("Failed to respond to roots/list")?;

        if !response.status().is_success() {
            return self.signal_error(anyhow!("roots/list response failed: {}", response.status()));
        }

        self.answered_roots = true;
        self.signal_ready();
        Ok(())
    }

    async fn handle_progress(&self, msg: &Value) {
        let Some(state) = &self.progress_state else {
            return;
        };
        let params = msg.get("params");
        let token = params
            .and_then(|p| p.get("progressToken"))
            .and_then(|t| t.as_str());
        let progress = params
            .and_then(|p| p.get("progress"))
            .and_then(|p| p.as_f64());
        let message = params
            .and_then(|p| p.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("");

        if progress.unwrap_or(0.0) >= 100.0
            && let Some(token) = token
        {
            state.track_completion(token, message).await;
        }
    }
}

fn parse_sse_event(event_block: &str) -> Option<Value> {
    let data: String = event_block
        .lines()
        .filter_map(|line| line.trim_end_matches('\r').strip_prefix("data:"))
        .map(|content| content.trim_start())
        .collect::<Vec<_>>()
        .join("\n");

    if data.is_empty() {
        return None;
    }
    serde_json::from_str(&data).ok()
}

// ---------------------------------------------------------------------------
// Server manager
// ---------------------------------------------------------------------------

struct ServerManager {
    child: Child,
    error_flag: Arc<AtomicBool>,
    error_context: Arc<Mutex<Vec<String>>>,
}

impl ServerManager {
    async fn spawn(port: u16, binary: Option<String>) -> Result<Self> {
        let error_flag = Arc::new(AtomicBool::new(false));
        let error_context: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let mut cmd = build_server_command(port, &binary);
        let mut child = cmd.spawn().context("Failed to spawn server")?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("Failed to capture stderr"))?;

        spawn_stderr_monitor(stderr, error_flag.clone(), error_context.clone());

        Ok(Self {
            child,
            error_flag,
            error_context,
        })
    }

    fn has_error(&self) -> bool {
        self.error_flag.load(Ordering::Relaxed)
    }

    async fn get_error_context(&self) -> Vec<String> {
        self.error_context.lock().await.clone()
    }

    async fn kill(mut self) -> Result<()> {
        self.child.kill().await.context("Failed to kill server")?;
        Ok(())
    }
}

fn build_server_command(port: u16, binary: &Option<String>) -> Command {
    let mut cmd = if let Some(binary_path) = binary {
        println!("Using binary: {}", binary_path);
        Command::new(binary_path)
    } else {
        let mut c = Command::new("cargo");
        c.args(["run", "-p", "ahma_http_bridge", "--"]);
        println!("Using: cargo run -p ahma_http_bridge");
        c
    };

    cmd.arg("--bind-addr")
        .arg(format!("127.0.0.1:{}", port))
        .arg("--default-sandbox-scope")
        .arg(".")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    cmd
}

fn spawn_stderr_monitor(
    stderr: tokio::process::ChildStderr,
    error_flag: Arc<AtomicBool>,
    error_context: Arc<Mutex<Vec<String>>>,
) {
    const CONTEXT_LINES: usize = 15;

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut recent_lines: Vec<String> = Vec::new();

        while let Ok(Some(line)) = reader.next_line().await {
            println!("[SERVER] {}", line);

            recent_lines.push(line.clone());
            if recent_lines.len() > CONTEXT_LINES {
                recent_lines.remove(0);
            }

            if !is_fatal_server_error(&line) {
                continue;
            }

            let trailing = collect_trailing_lines(&mut reader, 10).await;
            store_error_context(&error_context, &recent_lines, &trailing).await;
            error_flag.store(true, Ordering::SeqCst);
            break;
        }
    });
}

fn is_fatal_server_error(line: &str) -> bool {
    line.starts_with("error:")
        || (line.contains("ERROR") && !line.contains("0 errors"))
        || line.contains("panicked at")
        || line.contains("fatal error")
}

async fn collect_trailing_lines(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStderr>>,
    max: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    for _ in 0..max {
        if let Ok(Some(line)) = reader.next_line().await {
            println!("[SERVER] {}", line);
            lines.push(line);
        } else {
            break;
        }
    }
    lines
}

async fn store_error_context(
    error_context: &Mutex<Vec<String>>,
    recent_lines: &[String],
    trailing_lines: &[String],
) {
    let separator = "═══════════════════════════════════════════════════════════";
    let mut ctx = error_context.lock().await;

    ctx.push(format!("╔{}╗", separator));
    ctx.push("║  ERROR CONTEXT (recent server output before error)       ║".to_string());
    ctx.push(format!("╠{}╣", separator));

    for (i, line) in recent_lines.iter().enumerate() {
        if i == recent_lines.len() - 1 {
            ctx.push(format!(">>> {}", line));
        } else {
            ctx.push(format!("    {}", line));
        }
    }

    ctx.push(format!("╠{}╣", separator));
    ctx.push("║  FOLLOWING OUTPUT                                         ║".to_string());
    ctx.push(format!("╠{}╣", separator));

    for line in trailing_lines {
        ctx.push(format!("    {}", line));
    }

    ctx.push(format!("╚{}╝", separator));
}

// ---------------------------------------------------------------------------
// Health check
// ---------------------------------------------------------------------------

async fn wait_for_health(base_url: &str, timeout_secs: u64) -> Result<()> {
    let client = Client::new();
    let health_url = format!("{}/health", base_url);
    let start = Instant::now();

    loop {
        if start.elapsed().as_secs() > timeout_secs {
            return Err(anyhow!(
                "Server failed to become healthy in {}s",
                timeout_secs
            ));
        }
        match client.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                println!("OK Server is healthy");
                return Ok(());
            }
            _ => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Command pool
// ---------------------------------------------------------------------------

fn get_command_pool() -> Vec<Vec<String>> {
    [
        &["echo", "stress test ping"][..],
        &["ls", "-la"],
        &["whoami"],
        &["date"],
        &["pwd"],
        &["echo", "hello world"],
        &["uname", "-a"],
        &["printenv", "PATH"],
        &["echo", "$HOME"],
        &["ls", "-lh"],
        &["date", "+%Y-%m-%d"],
        &["echo", "test", "123"],
    ]
    .into_iter()
    .map(|parts| parts.iter().map(|s| s.to_string()).collect())
    .collect()
}

// ---------------------------------------------------------------------------
// Client spawning
// ---------------------------------------------------------------------------

struct SpawnConfig {
    is_sync: bool,
    seed: u64,
    label: String,
}

fn spawn_client(
    join_set: &mut JoinSet<Result<()>>,
    base_url: &str,
    config: SpawnConfig,
    commands: Vec<Vec<String>>,
    counters: &SharedCounters,
    stop_flag: Arc<AtomicBool>,
) {
    let base_url = base_url.to_string();
    let counters = counters.clone();

    join_set.spawn(async move {
        let mut client = StressClient::new(base_url, config.is_sync, &counters);
        let lcg = Lcg::new(config.seed);
        client.initialize().await?;
        println!("OK {} initialized", config.label);
        client.run_commands(commands, lcg, stop_flag).await
    });
}

// ---------------------------------------------------------------------------
// Main loop monitoring & reporting
// ---------------------------------------------------------------------------

enum TestOutcome {
    ServerError,
    ExcessiveClientErrors,
    DurationElapsed,
}

async fn run_monitoring_loop(
    server: &ServerManager,
    counters: &SharedCounters,
    stop_flag: &Arc<AtomicBool>,
    duration: Duration,
) -> TestOutcome {
    let start = Instant::now();

    loop {
        if server.has_error() {
            stop_flag.store(true, Ordering::SeqCst);
            return TestOutcome::ServerError;
        }

        if start.elapsed() >= duration {
            println!("\nOK Test duration completed");
            return TestOutcome::DurationElapsed;
        }

        let success = counters.success.load(Ordering::Relaxed);
        let errors = counters.error.load(Ordering::Relaxed);

        if errors >= 10 && success == 0 {
            stop_flag.store(true, Ordering::SeqCst);
            return TestOutcome::ExcessiveClientErrors;
        }

        if start.elapsed().as_secs().is_multiple_of(5) {
            let elapsed = start.elapsed().as_secs();
            let rate = if elapsed > 0 { success / elapsed } else { 0 };
            println!(
                "[{:3}s] Success: {} | Errors: {} | Rate: {}/s",
                elapsed, success, errors, rate
            );
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn print_final_report(counters: &SharedCounters, elapsed: f64) {
    let success = counters.success.load(Ordering::Relaxed);
    let errors = counters.error.load(Ordering::Relaxed);
    let async_ok = counters.async_success.load(Ordering::Relaxed);
    let async_err = counters.async_error.load(Ordering::Relaxed);
    let total = success + errors;
    let success_rate = if total > 0 {
        (success as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    println!();
    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║                     Test Results                          ║");
    println!("╚═══════════════════════════════════════════════════════════╝");
    println!("  Total requests: {}", total);
    println!("  Successful: {} ({:.1}%)", success, success_rate);
    println!("  Errors: {}", errors);
    println!(
        "  Async completions observed: {} ok / {} failed",
        async_ok, async_err
    );
    println!("  Duration: {:.1}s", elapsed);
    println!("  Average rate: {:.1} req/s", total as f64 / elapsed);
    println!();
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║       AHMA HTTP Bridge Stress Test                        ║");
    println!("╚═══════════════════════════════════════════════════════════╝");
    println!();
    println!("Configuration:");
    println!("  Port: {}", args.port);
    println!("  Duration: {}s", args.duration);
    println!("  Async clients: {}", args.async_clients);
    println!("  Sync clients: 1");
    println!();

    // Start server
    println!("Starting server...");
    let server = ServerManager::spawn(args.port, args.binary).await?;

    let base_url = format!("http://127.0.0.1:{}", args.port);
    if let Err(e) = wait_for_health(&base_url, 120).await {
        eprintln!("\nFAIL Failed to start server: {}", e);
        eprintln!("Cleaning up server process...");
        let _ = server.kill().await;
        std::process::exit(1);
    }

    println!();
    println!("Starting {} client(s)...", args.async_clients + 1);

    let counters = SharedCounters::new();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let commands = get_command_pool();
    let mut join_set = JoinSet::new();

    spawn_client(
        &mut join_set,
        &base_url,
        SpawnConfig {
            is_sync: true,
            seed: 12345,
            label: "Sync client".to_string(),
        },
        commands.clone(),
        &counters,
        stop_flag.clone(),
    );

    for i in 0..args.async_clients {
        spawn_client(
            &mut join_set,
            &base_url,
            SpawnConfig {
                is_sync: false,
                seed: 67890 + i as u64,
                label: format!("Async client {}", i + 1),
            },
            commands.clone(),
            &counters,
            stop_flag.clone(),
        );
    }

    println!();
    println!("Running stress test...");
    println!();

    let start = Instant::now();
    let duration = Duration::from_secs(args.duration);

    match run_monitoring_loop(&server, &counters, &stop_flag, duration).await {
        TestOutcome::ServerError => {
            print_server_error(&server).await;
            join_set.abort_all();
            let _ = server.kill().await;
            std::process::exit(1);
        }
        TestOutcome::ExcessiveClientErrors => {
            print_excessive_errors(&counters);
            join_set.abort_all();
            let _ = server.kill().await;
            std::process::exit(1);
        }
        TestOutcome::DurationElapsed => {}
    }

    // Graceful shutdown
    stop_flag.store(true, Ordering::Relaxed);
    println!();
    println!("Stopping clients...");

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => eprintln!("Client error: {}", e),
            Err(e) => eprintln!("Task error: {}", e),
        }
    }

    let had_server_error = server.has_error();
    println!("Stopping server...");
    server.kill().await?;

    print_final_report(&counters, start.elapsed().as_secs_f64());

    if counters.error.load(Ordering::Relaxed) > 0 || had_server_error {
        eprintln!("FAIL Test completed with errors");
        std::process::exit(1);
    }

    println!("OK Test completed successfully");
    Ok(())
}

async fn print_server_error(server: &ServerManager) {
    eprintln!();
    eprintln!("╔═══════════════════════════════════════════════════════════╗");
    eprintln!("║             FAIL SERVER ERROR DETECTED                       ║");
    eprintln!("╚═══════════════════════════════════════════════════════════╝");
    eprintln!();
    for line in server.get_error_context().await {
        eprintln!("{}", line);
    }
    eprintln!();
    eprintln!("ABORTING: Stress test terminated due to server error.");
    eprintln!();
}

fn print_excessive_errors(counters: &SharedCounters) {
    let errors = counters.error.load(Ordering::Relaxed);
    eprintln!();
    eprintln!("╔═══════════════════════════════════════════════════════════╗");
    eprintln!("║       FAIL TOO MANY CLIENT ERRORS - ABORTING                 ║");
    eprintln!("╚═══════════════════════════════════════════════════════════╝");
    eprintln!();
    eprintln!("  Errors: {} with 0 successes", errors);
    eprintln!("  This indicates a fundamental problem with the server or protocol.");
    eprintln!();
    eprintln!("  Common causes:");
    eprintln!("    - 409 Conflict: Sandbox not initialized (missing roots/list response)");
    eprintln!("    - 401/403: Authentication issues");
    eprintln!("    - 404: Wrong endpoint URL");
    eprintln!();
}
