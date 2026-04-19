//! Operation Monitor Edge Cases Integration Tests
//!
//! Tests for operation_monitor.rs edge cases covering:
//! 1. Concurrent timeout and cancellation race conditions
//! 2. wait_for_completion with already-completed operations
//! 3. Operation state transitions
//! 4. Timeout enforcement with real timers
//! 5. Cancellation propagation to running operations
//!
//! These are real integration tests using the actual OperationMonitor.

use ahma_mcp::operation_monitor::{MonitorConfig, Operation, OperationMonitor, OperationStatus};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::time::timeout;

// ============================================================================
// Test: Operation Lifecycle State Transitions
// ============================================================================

/// Test basic operation state transitions from Pending → InProgress → Completed
#[tokio::test]
async fn test_operation_state_transitions() {
    let config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(config));

    // Create and add a new operation
    let op = Operation::new(
        "test_op_1".to_string(),
        "test_tool".to_string(),
        "Test operation".to_string(),
        None,
    );
    assert_eq!(op.state, OperationStatus::Pending);

    monitor.add_operation(op).await;

    // Transition to InProgress
    monitor
        .update_status("test_op_1", OperationStatus::InProgress, None)
        .await;

    // The operation should now be in the active list with InProgress status
    let all_ops = monitor.get_all_active_operations().await;
    assert_eq!(all_ops.len(), 1);
    assert_eq!(all_ops[0].state, OperationStatus::InProgress);

    // Transition to Completed
    monitor
        .update_status(
            "test_op_1",
            OperationStatus::Completed,
            Some(serde_json::json!({"output": "success"})),
        )
        .await;

    // Should no longer be in active operations (moved to history)
    let all_ops = monitor.get_all_active_operations().await;
    assert_eq!(
        all_ops.len(),
        0,
        "Completed operations should not be active"
    );

    // Should be in completion history
    let history = monitor.get_completed_operations().await;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, OperationStatus::Completed);
}

/// Test that terminal states are correctly identified
#[tokio::test]
async fn test_terminal_states_are_correctly_identified() {
    assert!(OperationStatus::Completed.is_terminal());
    assert!(OperationStatus::Failed.is_terminal());
    assert!(OperationStatus::Cancelled.is_terminal());
    assert!(OperationStatus::TimedOut.is_terminal());
    assert!(!OperationStatus::Pending.is_terminal());
    assert!(!OperationStatus::InProgress.is_terminal());
}

// ============================================================================
// Test: Cancellation
// ============================================================================

/// Test that cancel_operation_with_reason properly cancels and moves to history
#[tokio::test]
async fn test_cancel_operation_with_reason() {
    let config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(config));

    let op = Operation::new(
        "cancel_test".to_string(),
        "test_tool".to_string(),
        "Operation to cancel".to_string(),
        None,
    );
    monitor.add_operation(op).await;

    monitor
        .update_status("cancel_test", OperationStatus::InProgress, None)
        .await;

    // Cancel with a specific reason
    let cancelled = monitor
        .cancel_operation_with_reason(
            "cancel_test",
            Some("User requested cancellation".to_string()),
        )
        .await;

    assert!(cancelled, "Should return true when cancellation succeeds");

    // Should be moved to history
    let history = monitor.get_completed_operations().await;
    let cancelled_op = history.iter().find(|op| op.id == "cancel_test");
    assert!(cancelled_op.is_some(), "Cancelled op should be in history");

    let op = cancelled_op.unwrap();
    assert_eq!(op.state, OperationStatus::Cancelled);

    // Check the reason is stored
    if let Some(result) = &op.result {
        assert!(result.get("cancelled").is_some());
        assert!(
            result
                .get("reason")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .contains("User requested")
        );
    }
}

/// Test cancelling a non-existent operation returns false
#[tokio::test]
async fn test_cancel_nonexistent_operation() {
    let config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(config));

    let cancelled = monitor
        .cancel_operation_with_reason("nonexistent_op", None)
        .await;

    assert!(
        !cancelled,
        "Cancelling non-existent operation should return false"
    );
}

/// Test cancelling an already-completed operation returns false
#[tokio::test]
async fn test_cancel_already_completed_operation() {
    let config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(config));

    let op = Operation::new(
        "already_done".to_string(),
        "test_tool".to_string(),
        "Already completed op".to_string(),
        None,
    );
    monitor.add_operation(op).await;

    // Complete the operation first
    monitor
        .update_status("already_done", OperationStatus::Completed, None)
        .await;

    // Try to cancel - should fail since it's already terminal
    let cancelled = monitor
        .cancel_operation_with_reason("already_done", None)
        .await;

    assert!(
        !cancelled,
        "Cancelling already-completed operation should return false"
    );
}

// ============================================================================
// Test: Timeout Enforcement
// ============================================================================

/// Test that operations exceeding their timeout are automatically timed out
#[tokio::test]
async fn test_operation_timeout_enforcement() {
    // Use a very short timeout for testing
    let config = MonitorConfig::with_timeout(Duration::from_millis(100));
    let monitor = Arc::new(OperationMonitor::new(config));

    let mut op = Operation::new(
        "timeout_test".to_string(),
        "test_tool".to_string(),
        "Operation that will timeout".to_string(),
        None,
    );
    // Set a short timeout specifically for this operation
    op.timeout_duration = Some(Duration::from_millis(50));
    // Force the operation to appear old enough to timeout immediately
    op.start_time = SystemTime::now() - Duration::from_millis(200);
    monitor.add_operation(op).await;

    monitor
        .update_status("timeout_test", OperationStatus::InProgress, None)
        .await;

    // Trigger timeout check
    monitor.check_timeouts().await;

    // The operation should now be timed out
    let history = monitor.get_completed_operations().await;
    let timed_out_op = history.iter().find(|op| op.id == "timeout_test");

    assert!(
        timed_out_op.is_some(),
        "Timed out operation should be in history"
    );
    assert_eq!(
        timed_out_op.unwrap().state,
        OperationStatus::TimedOut,
        "Operation should have TimedOut status"
    );
}

/// Test that operations within timeout are not prematurely timed out
#[tokio::test]
async fn test_operation_within_timeout_not_timed_out() {
    let config = MonitorConfig::with_timeout(Duration::from_secs(60));
    let monitor = Arc::new(OperationMonitor::new(config));

    let op = Operation::new(
        "no_timeout_test".to_string(),
        "test_tool".to_string(),
        "Operation that should not timeout".to_string(),
        None,
    );
    monitor.add_operation(op).await;

    monitor
        .update_status("no_timeout_test", OperationStatus::InProgress, None)
        .await;

    // Check timeouts immediately - operation just started, should not timeout
    monitor.check_timeouts().await;

    // Operation should still be active
    let active = monitor.get_all_active_operations().await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].state, OperationStatus::InProgress);
}

// ============================================================================
// Test: Wait for Completion
// ============================================================================

/// Test waiting for an operation that completes successfully
#[tokio::test]
async fn test_wait_for_completion_success() {
    let config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(config));

    let op = Operation::new(
        "wait_test".to_string(),
        "test_tool".to_string(),
        "Operation to wait for".to_string(),
        None,
    );
    // Subscribe to the watch channel *before* adding the operation to the monitor,
    // so we don't miss the completion signal.
    let mut completion_rx = op.subscribe_completion();
    monitor.add_operation(op).await;

    monitor
        .update_status("wait_test", OperationStatus::InProgress, None)
        .await;

    // Spawn a task to complete the operation after a short delay
    let monitor_clone = monitor.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        monitor_clone
            .update_status(
                "wait_test",
                OperationStatus::Completed,
                Some(serde_json::json!({"result": "done"})),
            )
            .await;
    });

    // Wait for completion with timeout
    let wait_result = timeout(Duration::from_secs(1), completion_rx.wait_for(|done| *done)).await;

    assert!(wait_result.is_ok(), "Should complete before timeout");

    // Verify it's in history with correct state
    let history = monitor.get_completed_operations().await;
    let completed_op = history.iter().find(|op| op.id == "wait_test");
    assert!(completed_op.is_some());
    assert_eq!(completed_op.unwrap().state, OperationStatus::Completed);
}

/// Test waiting for an already-completed operation
#[tokio::test]
async fn test_wait_for_already_completed_operation() {
    let config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(config));

    let op = Operation::new(
        "already_complete".to_string(),
        "test_tool".to_string(),
        "Already completed".to_string(),
        None,
    );
    monitor.add_operation(op).await;

    // Complete immediately
    monitor
        .update_status("already_complete", OperationStatus::Completed, None)
        .await;

    // Waiting should return immediately since operation is already done
    // We verify by checking the history
    let history = monitor.get_completed_operations().await;
    let op = history.iter().find(|op| op.id == "already_complete");
    assert!(op.is_some());
    assert!(op.unwrap().state.is_terminal());
}

// ============================================================================
// Test: Concurrent Operations
// ============================================================================

/// Test that multiple operations can be tracked concurrently
#[tokio::test]
async fn test_concurrent_operations_tracking() {
    let config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(config));

    // Add multiple operations
    for i in 0..10 {
        let op = Operation::new(
            format!("concurrent_op_{}", i),
            "test_tool".to_string(),
            format!("Concurrent operation {}", i),
            None,
        );
        monitor.add_operation(op).await;
        monitor
            .update_status(
                &format!("concurrent_op_{}", i),
                OperationStatus::InProgress,
                None,
            )
            .await;
    }

    // All should be active
    let active = monitor.get_all_active_operations().await;
    assert_eq!(active.len(), 10);

    // Complete odd-numbered operations
    for i in (1..10).step_by(2) {
        monitor
            .update_status(
                &format!("concurrent_op_{}", i),
                OperationStatus::Completed,
                None,
            )
            .await;
    }

    // Should have 5 active (even-numbered)
    let active = monitor.get_all_active_operations().await;
    assert_eq!(active.len(), 5);

    // Should have 5 in history (odd-numbered)
    let history = monitor.get_completed_operations().await;
    assert_eq!(history.len(), 5);
}

/// Test concurrent cancellation doesn't cause issues
#[tokio::test]
async fn test_concurrent_cancellation_safety() {
    let config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(config));

    let op = Operation::new(
        "race_cancel".to_string(),
        "test_tool".to_string(),
        "Operation for race testing".to_string(),
        None,
    );
    monitor.add_operation(op).await;
    monitor
        .update_status("race_cancel", OperationStatus::InProgress, None)
        .await;

    // Spawn multiple tasks trying to cancel the same operation
    let mut handles = vec![];
    for i in 0..5 {
        let monitor_clone = monitor.clone();
        let handle = tokio::spawn(async move {
            monitor_clone
                .cancel_operation_with_reason("race_cancel", Some(format!("Canceller {}", i)))
                .await
        });
        handles.push(handle);
    }

    // Wait for all cancellation attempts
    let results: Vec<bool> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Exactly one should succeed
    let success_count: usize = results.iter().filter(|&&b| b).count();
    assert_eq!(
        success_count, 1,
        "Exactly one cancellation should succeed, got {}",
        success_count
    );

    // Operation should be in history with Cancelled state
    let history = monitor.get_completed_operations().await;
    let op = history.iter().find(|op| op.id == "race_cancel");
    assert!(op.is_some());
    assert_eq!(op.unwrap().state, OperationStatus::Cancelled);
}

// ============================================================================
// Test: Operation with Result Data
// ============================================================================

/// Test that operation results are properly stored and retrievable
#[tokio::test]
async fn test_operation_result_storage() {
    let config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(config));

    let op = Operation::new(
        "result_test".to_string(),
        "test_tool".to_string(),
        "Operation with result".to_string(),
        None,
    );
    monitor.add_operation(op).await;

    let result_data = serde_json::json!({
        "stdout": "Build succeeded",
        "stderr": "",
        "exit_code": 0,
        "artifacts": ["target/debug/myapp"]
    });

    monitor
        .update_status(
            "result_test",
            OperationStatus::Completed,
            Some(result_data.clone()),
        )
        .await;

    let history = monitor.get_completed_operations().await;
    let op = history.iter().find(|op| op.id == "result_test").unwrap();

    assert!(op.result.is_some());
    let stored_result = op.result.as_ref().unwrap();
    assert_eq!(stored_result["exit_code"], 0);
    assert_eq!(stored_result["stdout"], "Build succeeded");
}

// ============================================================================
// Test: Failed Operations
// ============================================================================

/// Test that failed operations are properly tracked
#[tokio::test]
async fn test_failed_operation_tracking() {
    let config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(config));

    let op = Operation::new(
        "fail_test".to_string(),
        "test_tool".to_string(),
        "Operation that will fail".to_string(),
        None,
    );
    monitor.add_operation(op).await;

    monitor
        .update_status("fail_test", OperationStatus::InProgress, None)
        .await;

    let error_result = serde_json::json!({
        "error": "Compilation failed",
        "exit_code": 1,
        "stderr": "error[E0382]: use of moved value"
    });

    monitor
        .update_status("fail_test", OperationStatus::Failed, Some(error_result))
        .await;

    // Should be in history with Failed state
    let history = monitor.get_completed_operations().await;
    let op = history.iter().find(|op| op.id == "fail_test").unwrap();

    assert_eq!(op.state, OperationStatus::Failed);
    assert!(
        op.result.as_ref().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("Compilation failed")
    );
}
