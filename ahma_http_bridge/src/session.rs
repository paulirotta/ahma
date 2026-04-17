//! Session management for HTTP bridge session isolation mode.
//!
//! Per R8D, session isolation allows multiple IDE instances to share a single
//! HTTP server with per-session sandbox scopes. Each session spawns a separate
//! `ahma_mcp` subprocess with its own sandbox scope derived from the client's
//! workspace roots.
//!
//! ## Overview
//!
//! In HTTP mode, the server spawns a separate `ahma_mcp` subprocess per MCP session.
//! Each subprocess has its own sandbox scope derived from the client's workspace roots,
//! providing complete isolation between concurrent sessions.
//!
//! ## How It Works
//!
//! ### Protocol Flow
//!
//! 1. **Receive initialize request**: Generate session ID (UUID).
//! 2. **Spawn ahma_mcp subprocess**: Start the MCP engine for this session.
//! 3. **Forward initialize**: Hand over the initialization to the subprocess.
//! 4. **Subprocess requests roots/list**: The engine asks for workspace context.
//! 5. **Bridge intercept**: Capture the roots to define the sandbox boundary.
//!
//! ### Sandbox Scope Binding
//!
//! The sandbox scope is determined lazily via the MCP `roots/list` protocol:
//! 1. Client sends `initialize` with `capabilities.roots: { listChanged: true }`.
//! 2. Server spawns subprocess without sandbox restriction initially.
//! 3. Subprocess sends `roots/list` request to get workspace folders.
//! 4. Bridge intercepts and caches the first root as sandbox scope.
//! 5. Subsequent file operations are validated against this scope.
//!
//! **Security Invariant**: The sandbox scope is set **once** when the first `roots/list`
//! response is received and cannot be changed for that session.
//!
//! ### Handling Roots Changes
//!
//! For security, session isolation mode rejects roots changes after the sandbox is locked.
//! If `notifications/roots/list_changed` is received after locking, the subprocess is
//! immediately terminated to prevent sandbox escape.

use crate::error::{BridgeError, Result};
use ahma_common::sandbox_state::{SandboxState, SandboxStateMachine};
use chrono::Local;
use dashmap::DashMap;
use owo_colors::OwoColorize;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::VecDeque,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, Notify, broadcast, mpsc, oneshot},
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Maximum number of SSE events retained in the per-session replay buffer.
/// Older events are evicted when the buffer exceeds this size.
const EVENT_HISTORY_CAPACITY: usize = 1000;

/// Handshake state machine for MCP session coordination.
///
/// The MCP Streamable HTTP protocol requires a specific sequence:
/// 1. Client sends `initialize` request → server creates session
/// 2. Client opens SSE stream (GET /mcp with session header)
/// 3. Client sends `notifications/initialized` notification
/// 4. Server sends `notifications/roots/list_changed` to subprocess
/// 5. Subprocess sends `roots/list` request back through SSE
/// 6. Client responds with roots → sandbox is locked
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeState {
    /// Initial state: session created, waiting for SSE and MCP initialized
    AwaitingBoth,
    /// SSE connected, waiting for MCP initialized notification
    AwaitingSseOnly,
    /// MCP initialized received, waiting for SSE connection
    AwaitingMcpOnly,
    /// Both SSE and MCP initialized, roots/list_changed sent, awaiting sandbox lock
    RootsRequested,
    /// Sandbox locked, handshake complete
    Complete,
}

/// Action to perform after a state transition
#[derive(Debug)]
enum HandshakeAction {
    None,
    SendRootsListChanged,
}

/// Represents a workspace root provided by the client's `roots/list` capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRoot {
    /// URI of the root (must be file://)
    pub uri: String,
    /// Optional human-readable name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Default handshake timeout in seconds.
pub const DEFAULT_HANDSHAKE_TIMEOUT_SECS: u64 = 45;

/// Get the request timeout in seconds for bridge → subprocess calls.
pub fn request_timeout_secs() -> u64 {
    std::env::var("AHMA_HTTP_BRIDGE_REQUEST_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60)
}

/// Get the tools/call request timeout in seconds.
pub fn tool_call_timeout_secs() -> u64 {
    std::env::var("AHMA_HTTP_BRIDGE_TOOL_CALL_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60)
}

/// Session termination reason
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTerminationReason {
    /// Client requested termination (HTTP DELETE)
    ClientRequested,
    /// Roots change attempted after sandbox lock (security violation)
    RootsChangeRejected,
    /// Subprocess crashed
    ProcessCrashed,
    /// Session timed out
    Timeout,
}

/// Represents an active client session.
pub struct Session {
    /// Unique session identifier
    pub id: String,
    /// Channel to send messages to the subprocess (wrapped in Mutex for restart support)
    sender: Mutex<mpsc::Sender<String>>,
    /// Map of pending request IDs to response channels
    pending_requests: Arc<DashMap<String, oneshot::Sender<Value>>>,
    /// Broadcast channel for SSE events from this session.
    /// Each message is `(event_id, json_string)`.
    broadcast_tx: broadcast::Sender<(u64, String)>,
    /// Sandbox scopes (set on first roots/list response) - supports multiple roots
    sandbox_scopes: Mutex<Option<Vec<PathBuf>>>,
    /// Whether the session has been terminated
    terminated: AtomicBool,
    /// Termination reason (if terminated)
    termination_reason: Mutex<Option<SessionTerminationReason>>,
    /// Handle to the subprocess (for cleanup)
    child_handle: Mutex<Option<Child>>,

    /// Handshake state machine protected by a sync Mutex for atomic transitions
    handshake_state: std::sync::Mutex<HandshakeState>,

    /// Notify for waiting on MCP initialization
    mcp_initialized_notify: Notify,

    /// Shared state machine for sandbox lifecycle (R18, R20)
    sandbox_state_machine: Arc<SandboxStateMachine>,

    /// When the session was created (for handshake timeout tracking)
    created_at: Instant,
    /// Per-session handshake timeout duration
    handshake_timeout: Duration,

    /// Cumulative count of SSE events lost due to broadcast channel lag.
    /// Incremented in the SSE stream handler when `BroadcastStreamRecvError::Lagged(n)` is received.
    lagged_events: AtomicU64,

    /// Monotonically increasing per-session SSE event ID counter.
    event_id_counter: AtomicU64,
    /// Bounded ring buffer of recent SSE events for `Last-Event-Id` replay.
    event_history: std::sync::Mutex<VecDeque<(u64, String)>>,
}

impl Session {
    /// Check if the session is terminated
    pub fn is_terminated(&self) -> bool {
        self.terminated.load(Ordering::SeqCst)
    }

    /// Check if the sandbox is fully configured and active.
    ///
    /// Returns `true` only in `Active` state (subprocess confirmed sandbox configuration).
    /// During `Configuring` state the subprocess may still be processing the roots response,
    /// so tool calls must be held until the subprocess sends `notifications/sandbox/configured`.
    pub fn is_sandbox_locked(&self) -> bool {
        self.sandbox_state_machine.is_active()
    }

    /// Wait until the sandbox reaches `Active` state (subprocess confirmed configuration).
    ///
    /// Returns the configured scopes on success, or an error message if the sandbox
    /// enters a terminal state (Failed/Terminated).
    pub async fn wait_for_sandbox_active(&self) -> std::result::Result<Vec<PathBuf>, String> {
        self.sandbox_state_machine.wait_for_active().await
    }

    /// Get the current sandbox state
    pub fn current_sandbox_state(&self) -> ahma_common::sandbox_state::SandboxState {
        self.sandbox_state_machine.current()
    }

    /// Get the current handshake state
    pub fn handshake_state(&self) -> HandshakeState {
        *self.handshake_state.lock().unwrap()
    }

    /// Check if the SSE stream is connected (client opened GET /mcp)
    pub fn is_sse_connected(&self) -> bool {
        matches!(
            self.handshake_state(),
            HandshakeState::AwaitingSseOnly
                | HandshakeState::RootsRequested
                | HandshakeState::Complete
        )
    }

    /// Check if MCP initialized notification was received
    pub fn is_mcp_initialized(&self) -> bool {
        matches!(
            self.handshake_state(),
            HandshakeState::AwaitingMcpOnly
                | HandshakeState::RootsRequested
                | HandshakeState::Complete
        )
    }

    /// Wait for MCP initialization.
    pub async fn wait_for_mcp_initialized(&self) {
        if self.is_mcp_initialized() {
            return;
        }
        self.mcp_initialized_notify.notified().await;

        // Double check in case of race/spurious wakeup
        if !self.is_mcp_initialized() {
            // This is rare but possible; the caller might want to loop
            // For now, simpler to just return as the notify implies state change
        }
    }

    /// Check if subprocess has applied sandbox scopes (Active state)
    pub fn is_sandbox_applied(&self) -> bool {
        self.sandbox_state_machine.is_active()
    }

    /// Wait for sandbox application
    pub async fn wait_for_sandbox_applied(&self) {
        if self.is_sandbox_applied() {
            return;
        }
        let _ = self.sandbox_state_machine.wait_for_active().await;
    }

    /// Check if the handshake has timed out
    pub fn is_handshake_timed_out(&self) -> Option<u64> {
        if self.is_sandbox_locked() {
            return None;
        }
        let elapsed = self.created_at.elapsed();
        if elapsed >= self.handshake_timeout {
            Some(elapsed.as_secs())
        } else {
            None
        }
    }

    /// Get the first sandbox scope
    pub async fn get_sandbox_scope(&self) -> Option<PathBuf> {
        self.sandbox_scopes
            .lock()
            .await
            .as_ref()
            .and_then(|v| v.first().cloned())
    }

    /// Get all sandbox scopes
    pub async fn get_sandbox_scopes(&self) -> Option<Vec<PathBuf>> {
        self.sandbox_scopes.lock().await.clone()
    }

    /// Subscribe to SSE events. Each item is `(event_id, json_string)`.
    pub fn subscribe(&self) -> broadcast::Receiver<(u64, String)> {
        self.broadcast_tx.subscribe()
    }

    /// Record `n` events lost due to broadcast receiver lag.
    pub fn record_lagged_events(&self, n: u64) {
        self.lagged_events.fetch_add(n, Ordering::Relaxed);
    }

    /// Total number of SSE events lost due to broadcast receiver lag (cumulative).
    pub fn total_lagged_events(&self) -> u64 {
        self.lagged_events.load(Ordering::Relaxed)
    }

    /// Assign a unique event ID to a message and store it in the replay buffer.
    ///
    /// Returns the assigned event ID.
    pub fn assign_event_id(&self, msg: &str) -> u64 {
        let id = self.event_id_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let mut history = self.event_history.lock().unwrap();
        history.push_back((id, msg.to_string()));
        if history.len() > EVENT_HISTORY_CAPACITY {
            history.pop_front();
        }
        id
    }

    /// Return events with ID > `last_id` from the replay buffer.
    pub fn replay_events_after(&self, last_id: u64) -> Vec<(u64, String)> {
        let history = self.event_history.lock().unwrap();
        history
            .iter()
            .filter(|(id, _)| *id > last_id)
            .cloned()
            .collect()
    }

    /// Broadcast a message to all SSE subscribers with an assigned event ID.
    /// Primarily for testing.
    pub fn broadcast(
        &self,
        message: String,
    ) -> std::result::Result<usize, broadcast::error::SendError<(u64, String)>> {
        let id = self.assign_event_id(&message);
        self.broadcast_tx.send((id, message))
    }

    /// Helper to transitions state and return necessary action
    fn transition_sse_connected(&self) -> HandshakeAction {
        let mut state = self.handshake_state.lock().unwrap();
        match *state {
            HandshakeState::AwaitingBoth => {
                *state = HandshakeState::AwaitingSseOnly;
                info!(session_id = %self.id, from = ?HandshakeState::AwaitingBoth, to = ?HandshakeState::AwaitingSseOnly, "SSE connected");
                HandshakeAction::None
            }
            HandshakeState::AwaitingMcpOnly => {
                *state = HandshakeState::RootsRequested;
                info!(session_id = %self.id, from = ?HandshakeState::AwaitingMcpOnly, to = ?HandshakeState::RootsRequested, "SSE connected (completing handshake)");
                HandshakeAction::SendRootsListChanged
            }
            _ => {
                debug!(session_id = %self.id, state = ?*state, "SSE connected but already handled/advanced");
                HandshakeAction::None
            }
        }
    }

    /// Helper to transition state for MCP initialization
    fn transition_mcp_initialized(&self) -> HandshakeAction {
        let mut state = self.handshake_state.lock().unwrap();
        match *state {
            HandshakeState::AwaitingBoth => {
                *state = HandshakeState::AwaitingMcpOnly;
                info!(session_id = %self.id, from = ?HandshakeState::AwaitingBoth, to = ?HandshakeState::AwaitingMcpOnly, "MCP initialized");
                self.mcp_initialized_notify.notify_waiters();
                HandshakeAction::None
            }
            HandshakeState::AwaitingSseOnly => {
                *state = HandshakeState::RootsRequested;
                info!(session_id = %self.id, from = ?HandshakeState::AwaitingSseOnly, to = ?HandshakeState::RootsRequested, "MCP initialized (completing handshake)");
                self.mcp_initialized_notify.notify_waiters();
                HandshakeAction::SendRootsListChanged
            }
            _ => {
                debug!(session_id = %self.id, state = ?*state, "MCP initialized but already handled/advanced");
                // Ensure waiters are notified even if state was already advanced
                self.mcp_initialized_notify.notify_waiters();
                HandshakeAction::None
            }
        }
    }

    /// Mark SSE as connected and trigger action if needed
    pub async fn mark_sse_connected(&self) -> Result<bool> {
        match self.transition_sse_connected() {
            HandshakeAction::SendRootsListChanged => {
                self.send_roots_list_changed().await?;
                Ok(true)
            }
            HandshakeAction::None => Ok(false),
        }
    }

    /// Mark MCP as initialized and trigger action if needed
    pub async fn mark_mcp_initialized(&self) -> Result<bool> {
        match self.transition_mcp_initialized() {
            HandshakeAction::SendRootsListChanged => {
                self.send_roots_list_changed().await?;
                Ok(true)
            }
            HandshakeAction::None => Ok(false),
        }
    }

    /// Mark handshake as complete (sandbox locked).
    pub fn mark_handshake_complete(&self) {
        let mut state = self.handshake_state.lock().unwrap();
        if *state == HandshakeState::RootsRequested {
            *state = HandshakeState::Complete;
            info!(session_id = %self.id, "Handshake complete");
        }
    }

    /// Send roots/list_changed notification to subprocess.
    pub async fn send_roots_list_changed(&self) -> Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/roots/list_changed"
        });
        let json_str = serde_json::to_string(&notification)?;
        self.sender.lock().await.send(json_str).await.map_err(|e| {
            BridgeError::Communication(format!("Failed to send roots/list_changed: {}", e))
        })?;
        let sse_receivers = self.broadcast_tx.receiver_count();
        info!(session_id = %self.id, sse_receivers = sse_receivers, "Sent roots/list_changed to subprocess; waiting for roots/list response via broadcast");
        Ok(())
    }
}

/// Configuration for the `SessionManager`.
#[derive(Debug, Clone)]
pub struct SessionManagerConfig {
    /// The executable command to start the MCP server (e.g., "ahma_mcp").
    pub server_command: String,
    /// Arguments to pass to the server executable.
    pub server_args: Vec<String>,
    /// Explicit fallback directory for clients that do not provide roots.
    ///
    /// If `None`, clients must provide at least one valid `file://` root during
    /// handshake before tool calls are allowed.
    pub default_scope: Option<PathBuf>,
    /// Whether to preserve ANSI colors in the server's output streams.
    pub enable_colored_output: bool,
    /// Timeout in seconds for the MCP handshake to complete.
    /// If the handshake (SSE connection + roots/list response) doesn't complete
    /// within this time, tool calls will return a timeout error.
    /// Defaults to 45 seconds if not specified.
    pub handshake_timeout_secs: u64,
}

/// Manages the lifecycle of concurrent MCP sessions.
///
/// This component is responsible for:
/// - creating new sessions with unique IDs.
/// - spawning isolated subprocesses for each session.
/// - tracking active sessions in a thread-safe map.
/// - handling session termination.
///
/// Each session operates in its own `ahma_mcp` subprocess, ensuring that file access
/// rights and state are strictly isolated between different clients.
///
/// Use `create_session` to start a new handshake, and `lock_sandbox` to finalize security.
pub struct SessionManager {
    /// Active sessions indexed by session ID
    sessions: DashMap<String, Arc<Session>>,
    /// Configuration for spawning new sessions
    config: SessionManagerConfig,
}

impl SessionManager {
    #[cfg(target_os = "windows")]
    fn is_windows_drive_path(path: &str) -> bool {
        let path = path.strip_prefix(r"\\?\").unwrap_or(path);
        let path = path.strip_prefix(r"\\?\/").unwrap_or(path);
        let bytes = path.as_bytes();
        bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'/' || bytes[2] == b'\\')
    }

    fn parse_file_uri_to_path(uri: &str) -> Option<PathBuf> {
        // RFC 8089-ish minimal parsing: accept file:///abs/path and file://localhost/abs/path.
        // Percent-decoding is required for common IDE roots that contain spaces/unicode.
        const PREFIX: &str = "file://";
        if !uri.starts_with(PREFIX) {
            return None;
        }

        // Remove scheme.
        let mut rest = &uri[PREFIX.len()..];

        // Strip any query/fragment.
        if let Some(idx) = rest.find(['?', '#']) {
            rest = &rest[..idx];
        }

        // Handle host form: file://localhost/...
        if let Some(after_localhost) = rest.strip_prefix("localhost") {
            rest = after_localhost;
        }

        #[cfg(target_os = "windows")]
        {
            // Accept drive-letter forms:
            // - file:///C:/Users/name
            // - file://localhost/C:/Users/name
            // - file://C:/Users/name
            // And UNC host form:
            // - file://server/share/path
            let decoded = Self::percent_decode_utf8(rest)?;

            if let Some(without_leading_slash) = decoded.strip_prefix('/')
                && Self::is_windows_drive_path(without_leading_slash)
            {
                return Some(PathBuf::from(without_leading_slash));
            }

            if Self::is_windows_drive_path(&decoded) {
                return Some(PathBuf::from(decoded));
            }

            if !decoded.starts_with('/') && !decoded.starts_with("//") {
                return Some(PathBuf::from(format!("//{}", decoded)));
            }

            return None;
        }

        #[cfg(not(target_os = "windows"))]
        {
            // For unix-like paths, we only accept absolute paths.
            if !rest.starts_with('/') {
                return None;
            }

            let decoded = Self::percent_decode_utf8(rest)?;
            Some(PathBuf::from(decoded))
        }
    }

    fn percent_decode_utf8(input: &str) -> Option<String> {
        // Decode %XX sequences into bytes, then UTF-8.
        // If decoding fails, return None so the roots entry is treated as invalid.
        let bytes = input.as_bytes();
        let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'%' => {
                    if i + 2 >= bytes.len() {
                        return None;
                    }
                    let hi = bytes[i + 1];
                    let lo = bytes[i + 2];
                    let hex = |b: u8| -> Option<u8> {
                        match b {
                            b'0'..=b'9' => Some(b - b'0'),
                            b'a'..=b'f' => Some(b - b'a' + 10),
                            b'A'..=b'F' => Some(b - b'A' + 10),
                            _ => None,
                        }
                    };
                    let hi = hex(hi)?;
                    let lo = hex(lo)?;
                    out.push((hi << 4) | lo);
                    i += 3;
                }
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        String::from_utf8(out).ok()
    }

    /// Creates a new `SessionManager` with the given configuration.
    pub fn new(config: SessionManagerConfig) -> Self {
        Self {
            sessions: DashMap::new(),
            config,
        }
    }

    /// Returns true when this server requires client roots to complete sandbox lock.
    pub fn requires_client_roots(&self) -> bool {
        self.config.default_scope.is_none()
    }

    /// Initializes a new session and spawns a specific `ahma_mcp` subprocess for it.
    ///
    /// This initiates the "deferred sandbox" flow:
    /// 1. A new session ID is generated.
    /// 2. The subprocess is started with `--defer-sandbox`.
    /// 3. The subprocess waits for the bridge to provide the sandbox scope (derived from client roots).
    ///
    /// # Returns
    ///
    /// * `Ok(String)`: The new session ID (UUID v4). This ID must be included in the `Mcp-Session-Id` header for all subsequent requests.
    /// * `Err(BridgeError)`: If the subprocess could not be spawned.
    pub async fn create_session(&self) -> Result<String> {
        let session_id = Uuid::new_v4().to_string();

        info!(session_id = %session_id, "Creating new session");

        // Spawn subprocess WITHOUT sandbox restriction initially
        // Sandbox will be applied when roots/list is received
        let stderr_mode = if self.config.enable_colored_output {
            Stdio::piped()
        } else {
            Stdio::inherit()
        };

        // Set AHMA_SANDBOX_DEFER so the subprocess waits for roots/list to set sandbox scope.
        // Previously this was passed as --defer-sandbox CLI flag; now it uses an env var so
        // we don't need to care where on the command line it would go.
        let args = self.config.server_args.clone();

        let mut child = Command::new(&self.config.server_command)
            .args(&args)
            .env("AHMA_SANDBOX_DEFER", "1")
            // SECURITY:
            // Avoid inheriting env vars that can auto-enable permissive test mode in ahma_mcp,
            // which can mask real sandbox-scoping behavior.
            .env_remove("NEXTEST")
            .env_remove("NEXTEST_EXECUTION_MODE")
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("RUST_TEST_THREADS")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr_mode)
            // Ensure the subprocess does not outlive this process in the event a test exits early
            // or fails to explicitly terminate the session.
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                BridgeError::ServerProcess(format!("Failed to spawn subprocess: {}", e))
            })?;

        let stdin = child.stdin.take().expect("Failed to get stdin");
        let stdout = child.stdout.take().expect("Failed to get stdout");
        let stderr = if self.config.enable_colored_output {
            child.stderr.take()
        } else {
            None
        };

        // Create channels
        let (tx, rx) = mpsc::channel::<String>(100);
        let (broadcast_tx, _) = broadcast::channel::<(u64, String)>(256);
        let pending_requests = Arc::new(DashMap::new());

        let handshake_timeout = Duration::from_secs(self.config.handshake_timeout_secs);

        let session = Arc::new(Session {
            id: session_id.clone(),
            sender: Mutex::new(tx),
            pending_requests: pending_requests.clone(),
            broadcast_tx: broadcast_tx.clone(),
            sandbox_scopes: Mutex::new(None),
            terminated: AtomicBool::new(false),
            termination_reason: Mutex::new(None),
            child_handle: Mutex::new(Some(child)),
            handshake_state: std::sync::Mutex::new(HandshakeState::AwaitingBoth),
            mcp_initialized_notify: Notify::new(),
            sandbox_state_machine: Arc::new(SandboxStateMachine::new()),
            created_at: Instant::now(),
            handshake_timeout,
            lagged_events: AtomicU64::new(0),
            event_id_counter: AtomicU64::new(0),
            event_history: std::sync::Mutex::new(VecDeque::new()),
        });

        // Spawn the I/O handler task
        let session_clone = session.clone();
        let colored_output = self.config.enable_colored_output;
        tokio::spawn(async move {
            Self::handle_session_io(session_clone, rx, stdin, stdout, stderr, colored_output).await;
        });

        self.sessions.insert(session_id.clone(), session);

        Ok(session_id)
    }

    /// Get a session by ID
    pub fn get_session(&self, session_id: &str) -> Option<Arc<Session>> {
        self.sessions.get(session_id).map(|s| s.clone())
    }

    /// Send a message to a session's subprocess
    pub async fn send_message(&self, session_id: &str, message: &Value) -> Result<()> {
        let session = self.sessions.get(session_id).ok_or_else(|| {
            BridgeError::Communication(format!("Session not found: {}", session_id))
        })?;

        if session.is_terminated() {
            return Err(BridgeError::Communication(
                "Session has been terminated".to_string(),
            ));
        }

        let json_str = serde_json::to_string(message)?;
        session
            .sender
            .lock()
            .await
            .send(json_str)
            .await
            .map_err(|e| {
                BridgeError::Communication(format!("Failed to send to subprocess: {}", e))
            })?;

        Ok(())
    }

    /// Send a request and wait for response
    pub async fn send_request(
        &self,
        session_id: &str,
        request: &Value,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let session = self.sessions.get(session_id).ok_or_else(|| {
            BridgeError::Communication(format!("Session not found: {}", session_id))
        })?;

        if session.is_terminated() {
            return Err(BridgeError::Communication(
                "Session has been terminated".to_string(),
            ));
        }

        // Get request ID - treat null as absent (notification)
        let id_opt = request.get("id").and_then(|id| {
            if id.is_null() {
                None
            } else if id.is_string() {
                Some(id.as_str().unwrap().to_string())
            } else {
                Some(id.to_string())
            }
        });

        let (response_tx, response_rx) = if id_opt.is_some() {
            let (tx, rx) = oneshot::channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        // Register pending request
        if let Some(id) = &id_opt {
            session
                .pending_requests
                .insert(id.clone(), response_tx.unwrap());
        }

        // Send the request
        let json_str = serde_json::to_string(request)?;
        session
            .sender
            .lock()
            .await
            .send(json_str)
            .await
            .map_err(|e| {
                if let Some(id) = &id_opt {
                    session.pending_requests.remove(id);
                }
                BridgeError::Communication(format!("Failed to send to subprocess: {}", e))
            })?;

        // Wait for response if this is a request (has ID)
        if let Some(rx) = response_rx {
            let wait_timeout =
                timeout.unwrap_or_else(|| Duration::from_secs(request_timeout_secs()));
            match tokio::time::timeout(wait_timeout, rx).await {
                Ok(Ok(response)) => Ok(response),
                Ok(Err(_)) => Err(BridgeError::Communication(
                    "Response channel closed".to_string(),
                )),
                Err(_) => {
                    if let Some(id) = &id_opt {
                        session.pending_requests.remove(id);
                    }
                    Err(BridgeError::Communication("Request timed out".to_string()))
                }
            }
        } else {
            // Notification, return success immediately
            Ok(serde_json::json!({"jsonrpc": "2.0", "result": null}))
        }
    }

    /// Lock sandbox scope for a session (called when observing first roots/list response).
    ///
    /// Per R8.4.4-R8.4.5, sandbox scope is determined from the first roots/list response
    /// and cannot be changed. In the simplified design, the subprocess is spawned with
    /// `--defer-sandbox` and configures its own sandbox after roots are received.
    ///
    /// This method only records the scopes for bridge-side enforcement (e.g. rejecting
    /// roots changes after lock) and for debugging.
    ///
    /// Returns `true` if the sandbox was newly locked by this call.
    /// Returns an error if no valid lock source is available.
    ///
    /// Lock source priority:
    /// 1) Client-provided valid file:// roots
    /// 2) Explicit server fallback scope (if configured)
    pub async fn lock_sandbox(&self, session_id: &str, roots: &[McpRoot]) -> Result<bool> {
        let session = self.sessions.get(session_id).ok_or_else(|| {
            BridgeError::Communication(format!("Session not found: {}", session_id))
        })?;

        // Check if already locked using state machine
        if !matches!(
            session.sandbox_state_machine.current(),
            SandboxState::AwaitingRoots
        ) {
            return Ok(false);
        }

        // Extract all roots as sandbox scopes
        let parsed_scopes: Vec<PathBuf> = roots
            .iter()
            .filter_map(|r| Self::parse_file_uri_to_path(&r.uri))
            .collect();

        let scopes = if !parsed_scopes.is_empty() {
            parsed_scopes
        } else if roots.is_empty() {
            match &self.config.default_scope {
                Some(scope) => {
                    info!(
                        session_id = %session_id,
                        fallback_scope = %scope.display(),
                        "Client provided no roots; using explicit fallback sandbox scope"
                    );
                    vec![scope.clone()]
                }
                None => {
                    warn!(
                        session_id = %session_id,
                        "Rejecting sandbox lock: client provided no roots and no explicit fallback scope is configured"
                    );
                    return Err(BridgeError::Communication(
                        "Client did not provide roots/list entries. Configure explicit sandbox scope on server startup (e.g. --sandbox-scope /path/to/project) or use a client that supports roots/list.".to_string()
                    ));
                }
            }
        } else {
            warn!(
                session_id = %session_id,
                provided_roots = roots.len(),
                "Rejecting sandbox lock: roots/list contained no valid file:// URIs"
            );
            return Err(BridgeError::Communication(
                "No valid file:// sandbox roots were provided in roots/list response.".to_string(),
            ));
        };

        info!(
            session_id = %session_id,
            sandbox_scopes = ?scopes,
            "Locking sandbox scope(s) for session"
        );

        // Store the sandbox scopes
        *session.sandbox_scopes.lock().await = Some(scopes.clone());

        // Transition to Configuring state
        if let Err(e) = session
            .sandbox_state_machine
            .transition_to_configuring(scopes.clone())
        {
            warn!(session_id = %session_id, error = %e, "Failed to transition sandbox state to Configuring");
        }

        // Transition handshake state to Complete
        session.mark_handshake_complete();

        Ok(true)
    }

    /// Handle roots/list_changed notification
    ///
    /// Per R8D.12-R8D.13, if sandbox is locked, terminate the session immediately
    pub async fn handle_roots_changed(&self, session_id: &str) -> Result<()> {
        let session = self.sessions.get(session_id).ok_or_else(|| {
            BridgeError::Communication(format!("Session not found: {}", session_id))
        })?;

        if !matches!(
            session.sandbox_state_machine.current(),
            SandboxState::AwaitingRoots
        ) {
            // Security violation: attempt to change roots after sandbox lock
            let scopes = session.sandbox_scopes.lock().await.clone();
            error!(
                session_id = %session_id,
                sandbox_scopes = ?scopes,
                "Roots change rejected after sandbox lock - terminating session"
            );

            // Drop the session reference before terminating to avoid deadlock
            drop(session);

            self.terminate_session(session_id, SessionTerminationReason::RootsChangeRejected)
                .await?;

            return Err(BridgeError::Communication(
                "Session terminated: roots change not allowed after sandbox lock".to_string(),
            ));
        }

        // Sandbox not yet locked - this is unusual but allowed
        // (roots/list hasn't been processed yet)
        warn!(
            session_id = %session_id,
            "Roots change received before sandbox lock - allowing"
        );
        Ok(())
    }

    /// Terminate a session
    pub async fn terminate_session(
        &self,
        session_id: &str,
        reason: SessionTerminationReason,
    ) -> Result<()> {
        if let Some((_, session)) = self.sessions.remove(session_id) {
            info!(
                session_id = %session_id,
                reason = ?reason,
                "Terminating session"
            );

            session.terminated.store(true, Ordering::SeqCst);
            *session.termination_reason.lock().await = Some(reason);

            // Kill the subprocess
            if let Some(mut child) = session.child_handle.lock().await.take() {
                let _ = child.kill().await;
            }

            // Clear pending requests
            session.pending_requests.clear();

            // Transition state machine to Terminated
            let _ = session.sandbox_state_machine.transition_to_terminated();
        }

        Ok(())
    }

    /// Check if a session exists and is not terminated
    pub fn session_exists(&self, session_id: &str) -> bool {
        self.sessions
            .get(session_id)
            .map(|s| !s.is_terminated())
            .unwrap_or(false)
    }

    /// Get session count (for metrics/debugging)
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Handle I/O for a session subprocess
    async fn handle_session_io(
        session: Arc<Session>,
        mut rx: mpsc::Receiver<String>,
        mut stdin: tokio::process::ChildStdin,
        stdout: tokio::process::ChildStdout,
        stderr: Option<tokio::process::ChildStderr>,
        colored_output: bool,
    ) {
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = stderr.map(|s| BufReader::new(s).lines());

        loop {
            tokio::select! {
                // Handle outgoing messages (HTTP -> Stdio)
                Some(msg) = rx.recv() => {
                    debug!(session_id = %session.id, "Sending to subprocess: {}", msg);

                    // Echo STDIN in cyan if colored output is enabled
                    if colored_output {
                        let timestamp = format!("[{}]", Local::now().format("%H:%M:%S%.3f"));
                        if let Ok(parsed) = serde_json::from_str::<Value>(&msg) {
                            let pretty = serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| msg.clone());
                            eprintln!("{} {} {}\n{}", timestamp, format!("[{}]", &session.id[..8]).cyan(), "→ STDIN:".cyan(), pretty.cyan());
                        } else {
                            eprintln!("{} {} {}\n{}", timestamp, format!("[{}]", &session.id[..8]).cyan(), "→ STDIN:".cyan(), msg.cyan());
                        }
                    }

                    if let Err(e) = stdin.write_all(msg.as_bytes()).await {
                        error!(session_id = %session.id, "Failed to write to stdin: {}", e);
                        break;
                    }
                    if let Err(e) = stdin.write_all(b"\n").await {
                        error!(session_id = %session.id, "Failed to write newline to stdin: {}", e);
                        break;
                    }
                    if let Err(e) = stdin.flush().await {
                        error!(session_id = %session.id, "Failed to flush stdin: {}", e);
                        break;
                    }
                }

                // Handle incoming messages (Stdio -> HTTP/SSE)
                stdout_result = stdout_reader.next_line() => {
                    match stdout_result {
                        Ok(Some(line)) => {
                            if line.is_empty() { continue; }
                            debug!(session_id = %session.id, "Received from subprocess: {}", line);

                            // Echo STDOUT in green if colored output is enabled
                            if colored_output {
                                let timestamp = format!("[{}]", Local::now().format("%H:%M:%S%.3f"));
                                if let Ok(parsed) = serde_json::from_str::<Value>(&line) {
                                    let pretty = serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| line.clone());
                                    eprintln!("{} {} {}\n{}", timestamp, format!("[{}]", &session.id[..8]).green(), "← STDOUT:".green(), pretty.green());
                                } else {
                                    eprintln!("{} {} {}\n{}", timestamp, format!("[{}]", &session.id[..8]).green(), "← STDOUT:".green(), line.green());
                                }
                            }

                            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                                // Check if it's a response to a pending request
                                if let Some(id) = value.get("id") {
                                    let id_str = if id.is_string() {
                                        id.as_str().unwrap().to_string()
                                    } else {
                                        id.to_string()
                                    };

                                    if let Some((_, sender)) = session.pending_requests.remove(&id_str) {
                                        let _ = sender.send(value);
                                        continue;
                                    }
                                }

                                // If this is a notification that the subprocess has applied
                                // the sandbox scopes, mark the session and notify waiters.
                                if let Some(method_val) = value.get("method") {
                                    if let Some(method_str) = method_val.as_str() {
                                        if method_str == "notifications/sandbox/configured" {
                                            if let Err(e) = session.sandbox_state_machine.transition_to_active() {
                                                warn!(
                                                    session_id = %session.id,
                                                    error = %e,
                                                    "Failed to transition sandbox state to Active (received notifications/sandbox/configured)"
                                                );
                                            } else {
                                                info!(session_id = %session.id, "Observed notifications/sandbox/configured from subprocess - Sandbox is now ACTIVE");
                                            }
                                        } else if method_str == "notifications/sandbox/failed" {
                                            let err_msg = value
                                                .get("params")
                                                .and_then(|p| p.get("error"))
                                                .and_then(|e| e.as_str())
                                                .unwrap_or("Unknown error");

                                            if let Err(e) = session.sandbox_state_machine.transition_to_failed(err_msg.to_string()) {
                                                warn!(
                                                    session_id = %session.id,
                                                    error = %e,
                                                    "Failed to transition sandbox state to Failed (received notifications/sandbox/failed)"
                                                );
                                            } else {
                                                warn!(
                                                    session_id = %session.id,
                                                    error_msg = %err_msg,
                                                    "Subprocess reported sandbox configuration failed"
                                                );
                                            }
                                        } else {
                                            debug!(session_id = %session.id, method = %method_str, "Subprocess sent a different notification");
                                        }
                                    } else {
                                        warn!(session_id = %session.id, "Subprocess sent a method that is not a string");
                                    }
                                }

                                // Not a response to a pending request - broadcast as SSE event
                                let receiver_count = session.broadcast_tx.receiver_count();
                                if receiver_count == 0 {
                                    warn!(session_id = %session.id, method = %value.get("method").and_then(|m| m.as_str()).unwrap_or("<none>"), "Broadcasting to 0 SSE subscribers - event will be dropped (SSE stream not yet open)");
                                } else {
                                    debug!(session_id = %session.id, receiver_count = receiver_count, "Broadcasting to SSE subscribers");
                                }
                                let id = session.assign_event_id(&line);
                                let _ = session.broadcast_tx.send((id, line));
                            } else {
                                warn!(session_id = %session.id, "Failed to parse JSON from subprocess: {}", line);
                            }
                        }
                        Ok(None) => {
                            warn!(session_id = %session.id, "Subprocess stdout closed - assuming crash or exit");
                            break;
                        }
                        Err(e) => {
                            error!(session_id = %session.id, "Failed to read stdout: {}", e);
                            break;
                        }
                    }
                }

                // Handle stderr if colored output is enabled
                result = async {
                    if let Some(ref mut reader) = stderr_reader {
                        reader.next_line().await
                    } else {
                        std::future::pending().await
                    }
                } => {
                    match result {
                        Ok(Some(line)) if !line.is_empty() => {
                            let timestamp = format!("[{}]", Local::now().format("%H:%M:%S%.3f"));
                            if let Ok(parsed) = serde_json::from_str::<Value>(&line) {
                                let pretty = serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| line.clone());
                                eprintln!("{} {} {}\n{}", timestamp, format!("[{}]", &session.id[..8]).red(), "WARNING STDERR:".red(), pretty.red());
                            } else {
                                eprintln!("{} {} {}\n{}", timestamp, format!("[{}]", &session.id[..8]).red(), "WARNING STDERR:".red(), line.red());
                            }
                        }
                        Ok(Some(_)) => {} // Empty line
                        Ok(None) => {} // stderr closed
                        Err(e) => {
                            error!(session_id = %session.id, "Failed to read stderr: {}", e);
                        }
                    }
                }

                // Check for termination
                _ = async {
                    while !session.terminated.load(Ordering::SeqCst) {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                } => {
                    info!(session_id = %session.id, "Session terminated, stopping I/O handler");
                    break;
                }
            }
        }

        // Mark session as terminated if not already
        session.terminated.store(true, Ordering::SeqCst);

        // Send explicit error responses to all pending requests before clearing.
        // This prevents "Response channel closed" errors that manifest as cryptic
        // "Canceled: canceled" messages in clients.
        let pending_count = session.pending_requests.len();
        if pending_count > 0 {
            warn!(
                session_id = %session.id,
                pending_count = pending_count,
                "Session terminated with pending requests - sending error responses"
            );
            // Drain all pending requests and send error response
            let pending: Vec<_> = session
                .pending_requests
                .iter()
                .map(|entry| entry.key().clone())
                .collect();
            for id in pending {
                if let Some((_, sender)) = session.pending_requests.remove(&id) {
                    let error_response = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32603,
                            "message": "Session terminated unexpectedly - subprocess may have crashed or handshake failed"
                        }
                    });
                    let _ = sender.send(error_response);
                }
            }
        }
    }
}
