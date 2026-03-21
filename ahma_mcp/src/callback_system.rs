//! Asynchronous callback system for monitoring cargo operation progress
//!
//! This module provides a flexible callback architecture for tracking the progress of
//! long-running cargo operations. It enables real-time progress updates, output streaming,
//! and completion notifications through various callback mechanisms.
//!
//! ## Key Components
//!
//! - [`crate::callback_system::ProgressUpdate`]: Enumeration of different progress event types
//! - [`crate::callback_system::CallbackSender`]: Trait for implementing custom progress callback handlers
//! - [`crate::callback_system::ChannelCallbackSender`]: Channel-based callback implementation for async communication
//! - [`crate::callback_system::LoggingCallbackSender`]: Simple logging-based callback for debugging and monitoring
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! use ahma_mcp::callback_system::{CallbackSender, LoggingCallbackSender, ProgressUpdate};
//!
//! #[tokio::main]
//! async fn main() {
//!     let callback: Box<dyn CallbackSender> = Box::new(
//!         LoggingCallbackSender::new("cargo_build_001".to_string())
//!     );
//!     
//!     // Send progress updates during a cargo operation
//!     let update = ProgressUpdate::Started {
//!         id: "cargo_build_001".to_string(),
//!         command: "cargo build".to_string(),
//!         description: "Building project dependencies".to_string(),
//!     };
//!     
//!     callback.send_progress(update).await;
//! }
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;
use tokio::sync::mpsc;

const CANCELLATION_SUGGESTIONS: &str = "Suggestions: retry the command; capture server logs with `2>&1`; if this happened immediately, verify MCP roots/list handshake completed before tools/call";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancellationKind {
    UnknownSource,
    McpCancellation,
    Timeout,
    UserInitiated,
    Generic,
}

impl CancellationKind {
    fn as_message(self) -> &'static str {
        match self {
            CancellationKind::UnknownSource => "Operation was cancelled (source: unknown)",
            CancellationKind::McpCancellation => "MCP cancellation received",
            CancellationKind::Timeout => "Operation timed out",
            CancellationKind::UserInitiated => "User-initiated cancellation",
            CancellationKind::Generic => "Operation was cancelled",
        }
    }
}

fn detect_cancellation_kind(raw_message: &str) -> Option<CancellationKind> {
    let lower = raw_message.to_lowercase();

    if lower == "canceled" {
        return Some(CancellationKind::UnknownSource);
    }

    // Order matters: more specific patterns first, then broader patterns
    const ORDERED_PATTERNS: &[(&[&str], CancellationKind)] = &[
        (&["canceled: canceled"], CancellationKind::UnknownSource),
        (
            &["task cancelled for reason"],
            CancellationKind::McpCancellation,
        ),
        (&["timeout"], CancellationKind::Timeout),
        (&["user", "request"], CancellationKind::UserInitiated),
        (&["cancel"], CancellationKind::Generic),
    ];

    ORDERED_PATTERNS
        .iter()
        .find_map(|(patterns, kind)| patterns.iter().any(|p| lower.contains(p)).then_some(*kind))
}

fn format_cancellation_context(tool_name: Option<&str>, id: Option<&str>) -> Vec<String> {
    [
        tool_name.map(|t| format!("Tool: {t}")),
        id.map(|o| format!("Operation: {o}")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn cancellation_parts(
    kind: CancellationKind,
    raw_message: &str,
    tool_name: Option<&str>,
    id: Option<&str>,
) -> Vec<String> {
    let mut parts = vec![kind.as_message().to_string()];
    parts.extend(format_cancellation_context(tool_name, id));
    parts.push(format!("Raw: {raw_message}"));
    parts.push(CANCELLATION_SUGGESTIONS.to_string());
    parts
}

/// Represents different types of progress updates during cargo operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ProgressUpdate {
    /// Operation has started
    Started {
        id: String,
        command: String,
        description: String,
    },
    /// Progress update with optional percentage and message
    Progress {
        id: String,
        message: String,
        percentage: Option<f64>,
        current_step: Option<String>,
    },
    /// Output line from the cargo command
    Output {
        id: String,
        line: String,
        is_stderr: bool,
    },
    /// Operation completed successfully
    Completed {
        id: String,
        message: String,
        duration_ms: u64,
    },
    /// Operation failed with error
    Failed {
        id: String,
        error: String,
        duration_ms: u64,
    },
    /// Operation was cancelled
    Cancelled {
        id: String,
        message: String,
        duration_ms: u64,
    },
    /// Final comprehensive result with all details (like await command output)
    FinalResult {
        id: String,
        command: String,
        description: String,
        working_directory: String,
        success: bool,
        duration_ms: u64,
        full_output: String,
    },
    /// Log alert triggered by live monitoring when an error/warning pattern is detected.
    /// Contains the trigger line plus recent stdout/stderr context for AI analysis.
    LogAlert {
        id: String,
        /// The detected severity of the trigger line.
        trigger_level: String,
        /// Pre-formatted context snapshot (trigger + recent stdout + stderr).
        context_snapshot: String,
        /// Plain-English summary from the LLM when AI-powered detection flags an issue.
        llm_summary: Option<String>,
        /// The raw log lines that were sent to the LLM (or that triggered the regex pattern).
        trigger_lines: Option<Vec<String>>,
    },
}

impl ProgressUpdate {
    fn format_progress(
        message: &str,
        percentage: Option<f64>,
        current_step: Option<&str>,
    ) -> String {
        let progress_str = percentage.map_or(String::new(), |p| format!(" ({p:.1}%)"));
        let step_str = current_step.map_or(String::new(), |step| format!(" [{step}]"));
        format!("Progress{progress_str}: {message}{step_str}")
    }

    fn format_output(line: &str, is_stderr: bool) -> String {
        let stream = if is_stderr { "stderr" } else { "stdout" };
        format!("{stream}: {line}")
    }

    fn format_terminal(status: &str, duration_ms: u64, message: &str) -> String {
        format!("{status} after {duration_ms}ms: {message}")
    }

    fn format_completion(duration_ms: u64, message: &str) -> String {
        format!("Completed in {duration_ms}ms: {message}")
    }

    fn format_final_result(command: &str, success: bool, full_output: &str) -> String {
        let status = if success { "COMPLETED" } else { "FAILED" };
        format!("{status}: {command}\n{full_output}")
    }

    fn format_log_alert(
        trigger_level: &str,
        context_snapshot: &str,
        llm_summary: Option<&str>,
    ) -> String {
        match llm_summary {
            Some(summary) => {
                format!("LOG_ALERT ({trigger_level}):\n{summary}\n---\n{context_snapshot}")
            }
            None => format!("LOG_ALERT ({trigger_level}):\n{context_snapshot}"),
        }
    }
}

impl fmt::Display for ProgressUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProgressUpdate::Started {
                id,
                command,
                description,
            } => {
                write!(f, "[{id}] Started: {command} - {description}")
            }
            ProgressUpdate::Progress {
                id,
                message,
                percentage,
                current_step,
            } => write!(
                f,
                "[{id}] {}",
                Self::format_progress(message, *percentage, current_step.as_deref())
            ),
            ProgressUpdate::Output {
                id,
                line,
                is_stderr,
            } => write!(f, "[{id}] {}", Self::format_output(line, *is_stderr)),
            ProgressUpdate::Completed {
                id,
                message,
                duration_ms,
            } => write!(
                f,
                "[{id}] {}",
                Self::format_completion(*duration_ms, message)
            ),
            ProgressUpdate::Failed {
                id,
                error,
                duration_ms,
            } => write!(
                f,
                "[{id}] {}",
                Self::format_terminal("Failed", *duration_ms, error)
            ),
            ProgressUpdate::Cancelled {
                id,
                message,
                duration_ms,
            } => write!(
                f,
                "[{id}] {}",
                Self::format_terminal("CANCELLED", *duration_ms, message)
            ),
            ProgressUpdate::FinalResult {
                id,
                command,
                success,
                full_output,
                ..
            } => write!(
                f,
                "[{id}] {}",
                Self::format_final_result(command, *success, full_output)
            ),
            ProgressUpdate::LogAlert {
                id,
                trigger_level,
                context_snapshot,
                llm_summary,
                trigger_lines: _,
            } => write!(
                f,
                "[{id}] {}",
                Self::format_log_alert(trigger_level, context_snapshot, llm_summary.as_deref())
            ),
        }
    }
}

/// Trait for sending progress updates during cargo operations
/// This allows for different callback implementations (MCP notifications, logging, etc.)
#[async_trait]
pub trait CallbackSender: Send + Sync {
    /// Send a progress update
    async fn send_progress(&self, update: ProgressUpdate) -> Result<(), CallbackError>;

    /// Check if the operation should be cancelled
    async fn should_cancel(&self) -> bool;

    /// Send multiple progress updates in sequence
    async fn send_batch(&self, updates: Vec<ProgressUpdate>) -> Result<(), CallbackError> {
        for update in updates {
            self.send_progress(update).await?;
        }
        Ok(())
    }
}

/// Errors that can occur when sending callbacks
#[derive(Debug, thiserror::Error)]
pub enum CallbackError {
    #[error("Failed to send progress update: {0}")]
    SendFailed(String),
    #[error("Callback receiver disconnected")]
    Disconnected,
    #[error("Operation was cancelled")]
    Cancelled,
    #[error("Callback timeout: {0}")]
    Timeout(String),
}

/// Format a cancellation error message to be more informative.
///
/// This converts cryptic messages like "Canceled: canceled" into actionable messages
/// that explain what happened and what the user can do next.
///
/// # Arguments
/// * `raw_message` - The raw error/cancellation message from rmcp or the MCP client
/// * `tool_name` - Optional tool name that was being executed
/// * `id` - Optional operation ID for reference
///
/// # Returns
/// A user-friendly cancellation message with context
pub fn format_cancellation_message(
    raw_message: &str,
    tool_name: Option<&str>,
    id: Option<&str>,
) -> String {
    let Some(kind) = detect_cancellation_kind(raw_message) else {
        return raw_message.to_string();
    };

    cancellation_parts(kind, raw_message, tool_name, id).join(". ")
}

/// Channel-based callback sender for async communication
pub struct ChannelCallbackSender {
    sender: mpsc::UnboundedSender<ProgressUpdate>,
    cancellation_token: tokio_util::sync::CancellationToken,
}

impl ChannelCallbackSender {
    /// Create a channel-based callback sender.
    ///
    /// # Arguments
    /// * `sender` - Channel used to push `ProgressUpdate` events.
    /// * `cancellation_token` - Token used to check for cancellation.
    pub fn new(
        sender: mpsc::UnboundedSender<ProgressUpdate>,
        cancellation_token: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            sender,
            cancellation_token,
        }
    }
}

#[async_trait]
impl CallbackSender for ChannelCallbackSender {
    async fn send_progress(&self, update: ProgressUpdate) -> Result<(), CallbackError> {
        self.sender
            .send(update)
            .map_err(|_| CallbackError::Disconnected)?;
        Ok(())
    }

    async fn should_cancel(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }
}

/// No-op callback sender for when progress updates are not needed
pub struct NoOpCallbackSender;

#[async_trait]
impl CallbackSender for NoOpCallbackSender {
    async fn send_progress(&self, _update: ProgressUpdate) -> Result<(), CallbackError> {
        Ok(())
    }

    async fn should_cancel(&self) -> bool {
        false
    }
}

/// Logging callback sender that writes progress to the log
pub struct LoggingCallbackSender {
    operation_name: String,
}

impl LoggingCallbackSender {
    /// Create a logging callback sender for an operation.
    ///
    /// # Arguments
    /// * `operation_name` - Prefix used in log messages.
    pub fn new(operation_name: String) -> Self {
        Self { operation_name }
    }
}

#[async_trait]
impl CallbackSender for LoggingCallbackSender {
    async fn send_progress(&self, update: ProgressUpdate) -> Result<(), CallbackError> {
        tracing::debug!("{}: {update}", self.operation_name);
        Ok(())
    }

    async fn should_cancel(&self) -> bool {
        false
    }
}

/// Utility function to create a no-op callback sender
pub fn no_callback() -> Box<dyn CallbackSender> {
    Box::new(NoOpCallbackSender)
}

/// Utility function to create a logging callback sender
pub fn logging_callback(operation_name: String) -> Box<dyn CallbackSender> {
    Box::new(LoggingCallbackSender::new(operation_name))
}

/// Utility function to create a channel-based callback sender with receiver
pub fn channel_callback(
    cancellation_token: tokio_util::sync::CancellationToken,
) -> (
    Box<dyn CallbackSender>,
    mpsc::UnboundedReceiver<ProgressUpdate>,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let callback = Box::new(ChannelCallbackSender::new(sender, cancellation_token));
    (callback, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::logging::init_test_logging;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn test_no_op_callback() {
        init_test_logging();
        let callback = no_callback();
        let update = ProgressUpdate::Started {
            id: "test".to_string(),
            command: "cargo build".to_string(),
            description: "Building project".to_string(),
        };

        assert!(callback.send_progress(update).await.is_ok());
        assert!(!callback.should_cancel().await);
    }

    #[tokio::test]
    async fn test_channel_callback() {
        init_test_logging();
        let token = tokio_util::sync::CancellationToken::new();
        let (callback, mut receiver) = channel_callback(token.clone());

        let update = ProgressUpdate::Progress {
            id: "test".to_string(),
            message: "Building...".to_string(),
            percentage: Some(50.0),
            current_step: Some("Compiling".to_string()),
        };

        callback.send_progress(update.clone()).await.unwrap();

        let received = timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        match (&update, &received) {
            (
                ProgressUpdate::Progress { id: id1, .. },
                ProgressUpdate::Progress { id: id2, .. },
            ) => {
                assert_eq!(id1, id2);
            }
            _ => panic!("Unexpected update type"),
        }
    }

    #[tokio::test]
    async fn test_cancellation() {
        init_test_logging();
        let token = tokio_util::sync::CancellationToken::new();
        let (callback, _receiver) = channel_callback(token.clone());

        assert!(!callback.should_cancel().await);

        token.cancel();

        assert!(callback.should_cancel().await);
    }

    #[test]
    fn test_detect_cancellation_kind_variants() {
        assert_eq!(
            detect_cancellation_kind("Canceled: canceled"),
            Some(CancellationKind::UnknownSource)
        );
        assert_eq!(
            detect_cancellation_kind("task cancelled for reason: client disconnected"),
            Some(CancellationKind::McpCancellation)
        );
        assert_eq!(
            detect_cancellation_kind("operation timeout waiting for subprocess"),
            Some(CancellationKind::Timeout)
        );
        assert_eq!(
            detect_cancellation_kind("cancelled by user request"),
            Some(CancellationKind::UserInitiated)
        );
        assert_eq!(
            detect_cancellation_kind("cancelled"),
            Some(CancellationKind::Generic)
        );
        assert_eq!(detect_cancellation_kind("some unrelated error"), None);
    }

    #[test]
    fn test_format_cancellation_message_passthrough_for_non_cancellation() {
        let raw = "failed to parse json";
        assert_eq!(format_cancellation_message(raw, None, None), raw);
    }

    #[test]
    fn test_format_cancellation_message_includes_context_and_suggestions() {
        let message =
            format_cancellation_message("Canceled: canceled", Some("cargo_build"), Some("op-123"));

        assert!(message.contains("Operation was cancelled (source: unknown)"));
        assert!(message.contains("Tool: cargo_build"));
        assert!(message.contains("Operation: op-123"));
        assert!(message.contains("Raw: Canceled: canceled"));
        assert!(message.contains("Suggestions: retry the command"));
    }
}
