//! Operation monitoring and management system
//!
//! This module provides comprehensive monitoring, timeout handling, and cancellation
//! support for long-running cargo operations. It enables tracking of operation state,
//! automatic cleanup, and detailed logging for debugging.

use crate::utils::time;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::{
    RwLock,
    watch::{self, Receiver as WatchReceiver},
};
use tokio_util::sync::CancellationToken;
use tracing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
/// Represents the current state of an operation
pub enum OperationStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

impl OperationStatus {
    /// Check if this state represents a terminal (completed) operation
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            OperationStatus::Completed
                | OperationStatus::Failed
                | OperationStatus::Cancelled
                | OperationStatus::TimedOut
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Information about a running operation
pub struct Operation {
    pub id: String,
    pub tool_name: String,
    pub description: String,
    pub state: OperationStatus,
    pub result: Option<Value>,
    /// When the operation was created
    #[serde(with = "time")]
    pub start_time: SystemTime,
    /// When the operation completed (None if still running)
    #[serde(with = "time::option", default)]
    pub end_time: Option<SystemTime>,
    /// When wait_for_operation was first called for this operation (None if never waited for)
    #[serde(with = "time::option", default)]
    pub first_wait_time: Option<SystemTime>,
    /// Timeout duration for this specific operation (None means use default)
    pub timeout_duration: Option<Duration>,
    /// Cancellation token for this operation (not serialized)
    #[serde(skip)]
    pub cancellation_token: CancellationToken,
    /// Completion watch channel sender: false = in-progress, true = done.
    /// All subscribers observe completion even if they subscribe after the signal fires.
    /// Not serialised — only meaningful for live operations held in OperationMonitor.
    #[serde(skip, default = "default_completion_watch")]
    pub completion_watch: Arc<watch::Sender<bool>>,
}

/// Default factory for `completion_watch` used during serde deserialisation.
fn default_completion_watch() -> Arc<watch::Sender<bool>> {
    Arc::new(watch::channel(false).0)
}

impl Operation {
    /// Create a new operation info
    pub fn new(id: String, tool_name: String, description: String, result: Option<Value>) -> Self {
        Self {
            id,
            tool_name,
            description,
            state: OperationStatus::Pending,
            result,
            start_time: SystemTime::now(),
            end_time: None,
            first_wait_time: None,
            timeout_duration: None,
            cancellation_token: CancellationToken::new(),
            completion_watch: Arc::new(watch::channel(false).0),
        }
    }

    /// Create a new operation info with timeout
    pub fn new_with_timeout(
        id: String,
        tool_name: String,
        description: String,
        result: Option<Value>,
        timeout: Option<Duration>,
    ) -> Self {
        Self {
            id,
            tool_name,
            description,
            state: OperationStatus::Pending,
            result,
            start_time: SystemTime::now(),
            end_time: None,
            first_wait_time: None,
            timeout_duration: timeout,
            cancellation_token: CancellationToken::new(),
            completion_watch: Arc::new(watch::channel(false).0),
        }
    }

    /// Subscribe to the completion channel.
    /// Returns a receiver that yields `true` when the operation reaches a terminal state.
    /// Safe to call at any point — if the operation already completed the receiver
    /// immediately observes `true` on the first `borrow()` / `wait_for` poll.
    pub fn subscribe_completion(&self) -> WatchReceiver<bool> {
        self.completion_watch.subscribe()
    }
}

/// Configuration for operation monitoring
#[derive(Debug, Clone)]
pub struct MonitorConfig {
    /// Default timeout for operations (reduced from 5 minutes to 30 seconds)
    pub default_timeout: Duration,
    /// Maximum time to await for graceful shutdown
    pub shutdown_timeout: Duration,
}

#[derive(Debug, Clone)]
/// Summary of active operations for shutdown coordination
pub struct ShutdownSummary {
    pub total_active: usize,
    pub operations: Vec<Operation>,
}

impl MonitorConfig {
    /// Create a MonitorConfig with a custom timeout
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            default_timeout: timeout,
            shutdown_timeout: Duration::from_secs(30), // Reduced from 360s to 30s
        }
    }

    /// Create a MonitorConfig with custom timeouts
    pub fn with_timeouts(operation_timeout: Duration, shutdown_timeout: Duration) -> Self {
        Self {
            default_timeout: operation_timeout,
            shutdown_timeout,
        }
    }
}

/// Check if an operation's tool name matches any of the given filter prefixes.
/// Returns true if no filters are provided (i.e., all operations match).
fn matches_tool_filter(op: &Operation, filters: &Option<Vec<String>>) -> bool {
    filters.as_ref().is_none_or(|f| {
        let name = op.tool_name.to_lowercase();
        f.iter().any(|filter| name.starts_with(filter))
    })
}

/// Build a structured JSON cancellation result for debugging/LLM visibility.
fn build_cancellation_result(reason: Option<String>) -> Value {
    let reason_str = reason.unwrap_or_else(|| "Cancelled by user".to_string());
    serde_json::json!({
        "cancelled": true,
        "reason": reason_str
    })
}

/// Parse a comma-separated tool filter string into lowercase prefixes.
fn parse_tool_filters(tool_filter: Option<&str>) -> Option<Vec<String>> {
    tool_filter.map(|filters| {
        filters
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .collect()
    })
}

/// Log progressive timeout warnings at 50%, 75%, and 90% thresholds.
fn log_progress_warnings(progress_percent: u8, remaining_secs: i64, warnings_sent: &mut [bool; 3]) {
    const THRESHOLDS: [u8; 3] = [50, 75, 90];
    const MESSAGES: [&str; 3] = [
        "Wait operation 50% complete. Current active operations being monitored.",
        "Wait operation 75% complete. Consider checking operation status.",
        "Wait operation 90% complete. Operations may timeout soon!",
    ];

    for (i, &threshold) in THRESHOLDS.iter().enumerate() {
        if progress_percent >= threshold && !warnings_sent[i] {
            warnings_sent[i] = true;
            tracing::warn!("{} - {}s remaining", MESSAGES[i], remaining_secs.max(0));
        }
    }
}

/// Operation monitor that tracks and manages cargo operations
#[derive(Debug, Clone)]
pub struct OperationMonitor {
    operations: Arc<RwLock<HashMap<String, Operation>>>,
    completion_history: Arc<RwLock<HashMap<String, Operation>>>,
    #[allow(dead_code)]
    config: MonitorConfig,
}

impl OperationMonitor {
    /// Create a new operation monitor
    pub fn new(config: MonitorConfig) -> Self {
        Self {
            operations: Arc::new(RwLock::new(HashMap::new())),
            completion_history: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    pub async fn add_operation(&self, operation: Operation) {
        let mut ops = self.operations.write().await;
        tracing::info!(
            "Adding operation to monitor: {} (status: {:?})",
            operation.id,
            operation.state
        );
        ops.insert(operation.id.clone(), operation);
        tracing::debug!("Total operations in monitor after add: {}", ops.len());
    }

    pub async fn get_operation(&self, id: &str) -> Option<Operation> {
        let ops = self.operations.read().await;
        ops.get(id).cloned()
    }

    /// Starts a background task that periodically checks for timed-out operations.
    pub fn start_background_monitor(monitor: Arc<Self>) {
        let weak_monitor = Arc::downgrade(&monitor);
        tokio::spawn(async move {
            let check_interval = Duration::from_secs(1);
            loop {
                if let Some(monitor) = weak_monitor.upgrade() {
                    monitor.check_timeouts().await;
                } else {
                    tracing::debug!("OperationMonitor dropped, stopping background monitor task");
                    break;
                }
                tokio::time::sleep(check_interval).await;
            }
        });
    }

    /// Checks all active operations for timeouts and cancels them if necessary.
    pub async fn check_timeouts(&self) {
        let now = SystemTime::now();
        let timed_out_ops = {
            let ops = self.operations.read().await;
            ops.values()
                .filter(|op| !op.state.is_terminal())
                .filter_map(|op| {
                    let timeout = op.timeout_duration.unwrap_or(self.config.default_timeout);
                    let elapsed = now.duration_since(op.start_time).ok()?;
                    (elapsed > timeout).then_some((op.id.clone(), elapsed, timeout))
                })
                .collect::<Vec<_>>()
        };

        for (op_id, elapsed, timeout) in timed_out_ops {
            let reason = format!(
                "Operation timed out after {:.1}s (limit: {:.1}s)",
                elapsed.as_secs_f64(),
                timeout.as_secs_f64()
            );
            self.timeout_operation(&op_id, reason).await;
        }
    }

    async fn timeout_operation(&self, id: &str, reason: String) {
        let mut ops = self.operations.write().await;
        let Some(op) = ops.get_mut(id) else {
            return;
        };
        if op.state.is_terminal() {
            return;
        }

        tracing::warn!("Timing out operation: {} - {}", id, reason);

        op.state = OperationStatus::TimedOut;
        op.end_time = Some(SystemTime::now());
        op.cancellation_token.cancel();
        op.result = Some(serde_json::json!({
            "timed_out": true,
            "reason": reason
        }));

        let timed_out_op = ops.remove(id);
        drop(ops);

        self.move_to_history_and_notify(id, timed_out_op).await;
    }

    /// Move a completed operation to history and signal all completion waiters.
    ///
    /// Inserts into `completion_history` first, then broadcasts `true` through
    /// the watch channel.  Because the watch channel stores the current value,
    /// any subscriber that calls `subscribe_completion()` *after* this point
    /// will immediately observe `true` — eliminating the race that existed with
    /// the old `Arc<Notify>` approach where late subscribers missed the wakeup.
    async fn move_to_history_and_notify(&self, id: &str, operation: Option<Operation>) {
        let Some(op) = operation else { return };
        let mut history = self.completion_history.write().await;
        history.insert(id.to_string(), op.clone());
        drop(history);
        // Signal completion.  Ignore errors: a SendError means no subscribers,
        // which is fine — the result is already in completion_history.
        let _ = op.completion_watch.send(true);
    }

    /// Returns all currently active (non-terminal) operations.
    ///
    /// Note: Completed operations are accessible via `get_completed_operations`.
    pub async fn get_all_active_operations(&self) -> Vec<Operation> {
        self.get_active_operations().await
    }

    pub async fn update_status(&self, id: &str, status: OperationStatus, result: Option<Value>) {
        let mut ops = self.operations.write().await;
        let mut operation_to_move = None;

        if let Some(op) = ops.get_mut(id) {
            tracing::debug!(
                "Updating operation {} from {:?} to {:?}",
                id,
                op.state,
                status
            );
            op.state = status;
            op.result = result;

            if status.is_terminal() {
                op.end_time = Some(SystemTime::now());
                operation_to_move = ops.remove(id);
            }
        }

        drop(ops);

        if let Some(op) = operation_to_move {
            let mut history = self.completion_history.write().await;
            history.insert(id.to_string(), op.clone());
            tracing::debug!("Moved operation {} to completion history.", id);
            drop(history);
            let _ = op.completion_watch.send(true);
        }
    }

    /// Cancel an operation by ID with an optional reason string.
    /// Returns true if the operation was found and cancelled, false if not found.
    pub async fn cancel_operation_with_reason(&self, id: &str, reason: Option<String>) -> bool {
        let mut ops = self.operations.write().await;

        let Some(op) = ops.get_mut(id) else {
            tracing::warn!("Attempted to cancel non-existent operation: {}", id);
            return false;
        };

        if op.state.is_terminal() {
            tracing::warn!("Attempted to cancel already terminal operation: {}", id);
            return false;
        }

        tracing::info!("Cancelling operation: {}", id);
        tracing::debug!(
            "CANCEL_OPERATION_WITH_REASON: id='{}', reason={:?}, current_state={:?}",
            id,
            reason,
            op.state
        );

        op.state = OperationStatus::Cancelled;
        op.end_time = Some(SystemTime::now());
        op.cancellation_token.cancel();
        op.result = Some(build_cancellation_result(reason));

        let cancelled_op = op.clone();
        ops.remove(id);
        drop(ops);

        let mut history = self.completion_history.write().await;
        history.insert(id.to_string(), cancelled_op.clone());
        tracing::debug!("Moved cancelled operation {} to completion history.", id);
        drop(history);
        // Signal any concurrent wait_for_operation callers.
        let _ = cancelled_op.completion_watch.send(true);

        true
    }

    /// Backward-compatible helper without explicit reason
    pub async fn cancel_operation(&self, id: &str) -> bool {
        self.cancel_operation_with_reason(id, None).await
    }

    pub async fn get_active_operations(&self) -> Vec<Operation> {
        let ops = self.operations.read().await;
        ops.values()
            .filter(|op| !op.state.is_terminal())
            .cloned()
            .collect()
    }

    pub async fn get_completed_operations(&self) -> Vec<Operation> {
        let history = self.completion_history.read().await;
        history.values().cloned().collect()
    }

    pub async fn get_shutdown_summary(&self) -> ShutdownSummary {
        let operations = self.get_active_operations().await;
        let total_active = operations.len();
        ShutdownSummary {
            total_active,
            operations,
        }
    }

    /// Look up an operation in the completion history.
    pub async fn check_completion_history_pub(&self, id: &str) -> Option<Operation> {
        let history = self.completion_history.read().await;
        history.get(id).cloned()
    }

    /// Get a completion watch receiver for an active operation, recording first-wait time.
    ///
    /// Returns:
    /// - `Ok(Some(rx))` — operation is active; `rx` fires when it reaches a terminal state
    /// - `Ok(None)` — operation not found in active ops (may have just moved to history)
    /// - `Err(op)` — operation is already terminal in active ops (edge case)
    ///
    /// Unlike the old `get_notifier_or_terminal_pub`, the returned receiver will immediately
    /// yield `true` if the operation completed between this call and the first `wait_for` poll,
    /// eliminating the race that `wait_for_history_propagation_pub` papered over.
    pub async fn get_completion_receiver_or_terminal_pub(
        &self,
        id: &str,
    ) -> Result<Option<WatchReceiver<bool>>, Operation> {
        let mut ops = self.operations.write().await;
        let Some(op) = ops.get_mut(id) else {
            return Ok(None);
        };

        if op.first_wait_time.is_none() {
            op.first_wait_time = Some(SystemTime::now());
        }

        if op.state.is_terminal() {
            return Err(op.clone());
        }

        Ok(Some(op.subscribe_completion()))
    }

    /// Wait for an operation to reach a terminal state and return it from completion history.
    ///
    /// The implementation is race-free:
    /// 1. Check history first (fast path for already-completed ops).
    /// 2. Subscribe to the watch channel *while* the op is still in active ops, so we
    ///    cannot miss the completion signal even if it fires between steps 1 and 2.
    /// 3. If the op transitioned to history between steps 1 and 2, the channel already
    ///    holds `true` and `wait_for` returns immediately.
    /// 4. After the channel signals, the op is guaranteed to be in history (the channel
    ///    is sent *after* the history write), so a single history lookup suffices.
    pub async fn wait_for_operation(&self, id: &str) -> Option<Operation> {
        let timeout = Duration::from_secs(300);

        // Fast path: already completed.
        if let Some(op) = self.check_completion_history_pub(id).await {
            return Some(op);
        }

        let mut rx = match self.get_completion_receiver_or_terminal_pub(id).await {
            Err(terminal_op) => return Some(terminal_op),
            Ok(None) => {
                // Op is not in active ops.  It may have completed between the history
                // check above and here.  Re-check history once.
                return self.check_completion_history_pub(id).await;
            }
            Ok(Some(rx)) => rx,
        };

        // Wait until the completion flag turns true (or timeout).
        // `wait_for` is safe against missed signals: the watch channel stores its
        // current value, so even if `send(true)` fired between
        // `get_completion_receiver_or_terminal_pub` and this await, the receiver
        // will see `true` on the very first poll.
        //
        // Note: `watch::Ref` wraps an `RwLockReadGuard` which is not `Send`, so we
        // extract a plain `bool` and drop the guard before the next `await`.
        let timed_out = tokio::time::timeout(timeout, rx.wait_for(|done| *done))
            .await
            .is_err();
        if timed_out {
            tracing::warn!("Wait for operation {} timed out.", id);
            None
        } else {
            // The completion signal was sent *after* history insertion, so the
            // operation must already be in history at this point.
            self.check_completion_history_pub(id).await
        }
    }

    /// Get active operations that match the given tool filter.
    async fn get_filtered_active_operations(
        &self,
        filters: &Option<Vec<String>>,
    ) -> Vec<Operation> {
        let ops = self.operations.read().await;
        ops.values()
            .filter(|op| !op.state.is_terminal())
            .filter(|op| matches_tool_filter(op, filters))
            .cloned()
            .collect()
    }

    /// Collect completed operations from history that match the filter and finished
    /// after the given start time.
    async fn collect_completed_since(
        &self,
        filters: &Option<Vec<String>>,
        since: SystemTime,
    ) -> Vec<Operation> {
        let history = self.completion_history.read().await;
        history
            .values()
            .filter(|op| matches_tool_filter(op, filters))
            .filter(|op| op.end_time.is_some_and(|t| t >= since))
            .cloned()
            .collect()
    }

    /// Collect all completed operations from history that match the filter.
    async fn collect_all_completed(&self, filters: &Option<Vec<String>>) -> Vec<Operation> {
        let history = self.completion_history.read().await;
        history
            .values()
            .filter(|op| matches_tool_filter(op, filters))
            .cloned()
            .collect()
    }

    /// Advanced await functionality that waits for multiple operations with progressive timeout warnings.
    ///
    /// # Arguments
    /// * `tool_filter` - Optional comma-separated list of tool prefixes to await for
    /// * `timeout_seconds` - Timeout in seconds (1-1800 range, defaults to 240)
    ///
    /// # Returns
    /// A vector of completed operations that match the filter criteria
    pub async fn wait_for_operations_advanced(
        &self,
        tool_filter: Option<&str>,
        timeout_seconds: Option<u32>,
    ) -> Vec<Operation> {
        let timeout_secs = timeout_seconds.unwrap_or(240).clamp(1, 1800);
        let timeout = Duration::from_secs(timeout_secs as u64);
        let start_time = Instant::now();
        let tool_filters = parse_tool_filters(tool_filter);

        tracing::info!(
            "Starting advanced await operation: timeout={}s, tool_filter={:?}",
            timeout_secs,
            tool_filters
        );

        let mut warnings_sent = [false; 3];
        let mut completed_operations = Vec::new();

        loop {
            let elapsed = start_time.elapsed();

            if elapsed >= timeout {
                tracing::warn!("Advanced await operation timed out after {}s", timeout_secs);
                break;
            }

            let progress_percent = (elapsed.as_secs_f64() / timeout.as_secs_f64() * 100.0) as u8;
            let remaining_secs = timeout_secs as i64 - elapsed.as_secs() as i64;
            log_progress_warnings(progress_percent, remaining_secs, &mut warnings_sent);

            let active_ops = self.get_filtered_active_operations(&tool_filters).await;

            if active_ops.is_empty() {
                completed_operations = self.collect_all_completed(&tool_filters).await;
                tracing::info!(
                    "Advanced await completed: {} operations finished, no active operations remaining",
                    completed_operations.len()
                );
                break;
            }

            let wait_start_system_time = SystemTime::now() - elapsed;
            let newly_completed = self
                .collect_completed_since(&tool_filters, wait_start_system_time)
                .await;
            completed_operations.extend(newly_completed);

            tokio::time::sleep(Duration::from_millis(
                crate::constants::SEQUENCE_STEP_DELAY_MS,
            ))
            .await;
        }

        completed_operations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::logging::init_test_logging;
    use std::time::Duration;

    /// This test simulates the race condition where an operation completes
    /// so quickly that a `await` call might miss it. By using the `completion_history`
    /// map, the monitor should now correctly retrieve the status of already-completed
    /// operations.
    #[tokio::test]
    async fn test_wait_for_fast_completion_race_condition() {
        init_test_logging();
        let monitor = OperationMonitor::new(MonitorConfig::with_timeout(Duration::from_secs(5)));
        let op_id = "fast_op_1".to_string();
        let op = Operation::new(
            op_id.clone(),
            "test_tool".to_string(),
            "A test operation".to_string(),
            None,
        );

        // 1. Add the operation
        monitor.add_operation(op).await;

        // 2. Immediately update its status to Completed, which moves it to history
        monitor
            .update_status(
                &op_id,
                OperationStatus::Completed,
                Some(serde_json::json!({"result": "success"})),
            )
            .await;

        // 3. Now, try to await for it. The old system might have failed here.
        let result = monitor.wait_for_operation(&op_id).await;

        // 4. Assert that we correctly found the completed operation.
        assert!(result.is_some());
        let completed_op = result.unwrap();
        assert_eq!(completed_op.id, op_id);
        assert_eq!(completed_op.state, OperationStatus::Completed);
        assert_eq!(
            completed_op.result,
            Some(serde_json::json!({"result": "success"}))
        );

        // 5. Verify it's not in the active operations map anymore
        let active_ops = monitor.operations.read().await;
        assert!(!active_ops.contains_key(&op_id));

        // 6. Verify it IS in the completion history map
        let history = monitor.completion_history.read().await;
        assert!(history.contains_key(&op_id));
    }

    /// Tests that waiting for an operation that never existed returns `None`
    /// immediately instead of blocking indefinitely.
    #[tokio::test]
    async fn test_wait_for_nonexistent_operation() {
        init_test_logging();
        let monitor = OperationMonitor::new(MonitorConfig::with_timeout(Duration::from_secs(5)));

        // Use a timeout to ensure the test completes quickly
        let wait_result = tokio::time::timeout(
            Duration::from_millis(200),
            monitor.wait_for_operation("nonexistent-id"),
        )
        .await;

        // Should complete quickly and return None for nonexistent operation
        match wait_result {
            Ok(result) => {
                assert!(
                    result.is_none(),
                    "Should return None for nonexistent operation"
                );
            }
            Err(_) => {
                panic!(
                    "wait_for_operation should return quickly for nonexistent operation, not timeout"
                );
            }
        }
    }
}
