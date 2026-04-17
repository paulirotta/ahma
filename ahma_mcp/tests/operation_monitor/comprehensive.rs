//! Comprehensive operation monitor testing for Phase 7 requirements.
//!
//! This test module targets:
//! - Concurrent operation tracking edge cases
//! - Memory cleanup validation for completed operations
//! - Status query performance under high load
//! - Advanced await functionality edge cases
//! - Operation lifecycle and state transition validation

use anyhow::Result;
use serde_json::json;
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{sync::Barrier, time::Instant};

use ahma_mcp::operation_monitor::{MonitorConfig, Operation, OperationMonitor, OperationStatus};

/// Test concurrent operation tracking with multiple threads
#[tokio::test]
async fn test_concurrent_operation_tracking() -> Result<()> {
    let monitor = OperationMonitor::new(MonitorConfig::with_timeout(Duration::from_secs(30)));
    let monitor = Arc::new(monitor);

    let num_concurrent = 10;
    let barrier = Arc::new(Barrier::new(num_concurrent));
    let mut handles = Vec::new();

    // Start multiple concurrent operations
    for i in 0..num_concurrent {
        let monitor_clone = monitor.clone();
        let barrier_clone = barrier.clone();

        let handle = tokio::spawn(async move {
            let op_id = format!("concurrent_op_{}", i);
            let operation = Operation::new(
                op_id.clone(),
                "test_tool".to_string(),
                format!("Concurrent operation {}", i),
                Some(json!({"test": i})),
            );

            // Add operation
            monitor_clone.add_operation(operation).await;

            // Wait for all tasks to be ready
            barrier_clone.wait().await;

            // Update to in-progress
            monitor_clone
                .update_status(&op_id, OperationStatus::InProgress, None)
                .await;

            // Simulate some work without time-based sleeps
            tokio::task::yield_now().await;

            // Complete the operation
            monitor_clone
                .update_status(
                    &op_id,
                    OperationStatus::Completed,
                    Some(json!({"result": format!("completed_{}", i)})),
                )
                .await;

            op_id
        });

        handles.push(handle);
    }

    // Wait for all operations to complete
    let completed_ops: Vec<String> = futures::future::join_all(handles)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    // Verify all operations completed
    assert_eq!(completed_ops.len(), num_concurrent);

    // Check that all operations are in completion history
    let completed = monitor.get_completed_operations().await;
    assert_eq!(completed.len(), num_concurrent);

    // Verify no active operations remain
    let active = monitor.get_active_operations().await;
    assert_eq!(active.len(), 0);

    // Verify each operation can be retrieved
    for op_id in completed_ops {
        let op = monitor.wait_for_operation(&op_id).await;
        assert!(op.is_some());
        assert_eq!(op.unwrap().state, OperationStatus::Completed);
    }

    Ok(())
}

/// Test memory cleanup for completed operations over time
#[tokio::test]
async fn test_memory_cleanup_validation() -> Result<()> {
    let monitor = OperationMonitor::new(MonitorConfig::with_timeout(Duration::from_secs(30)));

    let num_operations = 100;
    let mut ids = Vec::new();

    // Create and complete many operations
    for i in 0..num_operations {
        let op_id = format!("cleanup_test_op_{}", i);
        let operation = Operation::new(
            op_id.clone(),
            "cleanup_tool".to_string(),
            format!("Cleanup test operation {}", i),
            Some(json!({"batch": i})),
        );

        ids.push(op_id.clone());

        // Add and immediately complete operations
        monitor.add_operation(operation).await;
        monitor
            .update_status(
                &op_id,
                OperationStatus::Completed,
                Some(json!({"result": i})),
            )
            .await;
    }

    // Verify all operations are in completion history
    let completed = monitor.get_completed_operations().await;
    assert_eq!(completed.len(), num_operations);

    // Verify no operations remain in active tracking
    let active = monitor.get_active_operations().await;
    assert_eq!(active.len(), 0);

    // Test that all operations can be retrieved via wait_for_operation
    let mut retrieved_count = 0;
    for op_id in &ids {
        if let Some(op) = monitor.wait_for_operation(op_id).await {
            assert_eq!(op.state, OperationStatus::Completed);
            retrieved_count += 1;
        }
    }
    assert_eq!(retrieved_count, num_operations);

    // Test get_all_operations returns only active operations (should be 0 after completion)
    let all_ops = monitor.get_all_active_operations().await;
    assert_eq!(all_ops.len(), 0); // Should be 0 active operations

    Ok(())
}

/// Populate a monitor with `num_ops` operations in rotating Completed / InProgress / Pending states.
async fn create_mixed_state_operations(monitor: &OperationMonitor, num_ops: usize) {
    for i in 0..num_ops {
        let op_id = format!("perf_test_op_{}", i);
        let operation = Operation::new(
            op_id.clone(),
            "perf_tool".to_string(),
            format!("Performance test operation {}", i),
            Some(json!({"test_id": i})),
        );
        monitor.add_operation(operation).await;

        if i % 3 == 0 {
            monitor
                .update_status(
                    &op_id,
                    OperationStatus::Completed,
                    Some(json!({"performance_result": i})),
                )
                .await;
        } else if i % 3 == 1 {
            monitor
                .update_status(&op_id, OperationStatus::InProgress, None)
                .await;
        }
        // Every third operation stays Pending
    }
}

/// Run a single query-task: synchronise at the barrier, then cycle through all four query types
/// for `duration_ms` milliseconds, returning `(task_id, query_count, elapsed)`.
async fn run_query_task(
    task_id: usize,
    monitor: Arc<OperationMonitor>,
    barrier: Arc<Barrier>,
    duration_ms: u64,
) -> (usize, usize, Duration) {
    barrier.wait().await;
    let start_time = Instant::now();
    let mut query_count = 0;

    while start_time.elapsed() < Duration::from_millis(duration_ms) {
        match query_count % 4 {
            0 => { let _ = monitor.get_active_operations().await; }
            1 => { let _ = monitor.get_completed_operations().await; }
            2 => { let _ = monitor.get_all_active_operations().await; }
            _ => { let _ = monitor.get_shutdown_summary().await; }
        }
        query_count += 1;
    }

    (task_id, query_count, start_time.elapsed())
}

/// Assert that performance results satisfy throughput and latency bounds.
fn assert_query_performance(results: &[(usize, usize, Duration)], num_tasks: usize) {
    let total_queries: usize = results.iter().map(|(_, count, _)| count).sum();
    let avg_queries_per_task = total_queries / num_tasks;

    assert!(
        avg_queries_per_task > 10,
        "Expected high query throughput, got avg {} queries per task",
        avg_queries_per_task
    );

    let max_duration = Duration::from_millis(1000);
    for (task_id, query_count, duration) in results {
        assert!(
            duration < &max_duration,
            "Task {} took too long: {:?} for {} queries (exceeded {:?})",
            task_id,
            duration,
            query_count,
            max_duration
        );
    }

    println!(
        "Performance test completed: {} total queries across {} tasks",
        total_queries, num_tasks
    );
}

/// Test status query performance under high load
#[tokio::test]
async fn test_status_query_performance_under_load() -> Result<()> {
    let monitor = OperationMonitor::new(MonitorConfig::with_timeout(Duration::from_secs(60)));
    let monitor = Arc::new(monitor);

    let num_operations = 50;
    let num_query_tasks = 20;

    create_mixed_state_operations(&monitor, num_operations).await;

    let barrier = Arc::new(Barrier::new(num_query_tasks));
    let handles: Vec<_> = (0..num_query_tasks)
        .map(|task_id| {
            tokio::spawn(run_query_task(
                task_id,
                monitor.clone(),
                barrier.clone(),
                100,
            ))
        })
        .collect();

    let results: Vec<(usize, usize, Duration)> = futures::future::join_all(handles)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    assert_query_performance(&results, num_query_tasks);

    Ok(())
}

/// Test advanced await functionality edge cases
#[tokio::test]
async fn test_advanced_await_functionality_edge_cases() -> Result<()> {
    let monitor = OperationMonitor::new(MonitorConfig::with_timeout(Duration::from_secs(30)));

    // Test with very short timeout
    let short_timeout_result = monitor.wait_for_operations_advanced(None, Some(1)).await;
    // Should complete quickly even with no operations
    assert!(short_timeout_result.is_empty());

    // Test with tool filter that matches nothing
    let no_match_result = monitor
        .wait_for_operations_advanced(Some("nonexistent_tool"), Some(2))
        .await;
    assert!(no_match_result.is_empty());

    // Create operations with different tool names
    let tool_names = ["cargo", "git", "test", "other"];
    for (i, &tool) in tool_names.iter().enumerate() {
        let op_id = format!("await_test_{}_{}", tool, i);
        let operation = Operation::new(
            op_id.clone(),
            tool.to_string(),
            format!("Await test for {}", tool),
            Some(json!({"tool": tool, "index": i})),
        );

        monitor.add_operation(operation).await;

        // Complete operations immediately
        monitor
            .update_status(
                &op_id,
                OperationStatus::Completed,
                Some(json!({"tool_result": tool})),
            )
            .await;
    }

    // Test tool filter for specific tools
    let cargo_results = monitor
        .wait_for_operations_advanced(Some("cargo"), Some(5))
        .await;
    assert_eq!(cargo_results.len(), 1);
    assert!(cargo_results[0].tool_name.starts_with("cargo"));

    // Test multiple tool filter
    let multi_results = monitor
        .wait_for_operations_advanced(Some("cargo,git"), Some(5))
        .await;
    assert_eq!(multi_results.len(), 2);

    // Test filter with no timeout (default timeout)
    let default_results = monitor
        .wait_for_operations_advanced(Some("test"), None)
        .await;
    assert_eq!(default_results.len(), 1);

    Ok(())
}

/// Test operation lifecycle and state transitions
#[tokio::test]
async fn test_operation_lifecycle_and_state_transitions() -> Result<()> {
    let monitor = OperationMonitor::new(MonitorConfig::with_timeout(Duration::from_secs(30)));

    let op_id = "lifecycle_test_op";
    let operation = Operation::new(
        op_id.to_string(),
        "lifecycle_tool".to_string(),
        "Lifecycle test operation".to_string(),
        None,
    );

    // Verify initial state
    assert_eq!(operation.state, OperationStatus::Pending);
    assert!(operation.end_time.is_none());
    assert!(operation.first_wait_time.is_none());

    // Add to monitor
    monitor.add_operation(operation).await;

    // Verify operation can be retrieved
    let retrieved = monitor.get_operation(op_id).await;
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().state, OperationStatus::Pending);

    // Test state transitions
    let states_to_test = [OperationStatus::InProgress, OperationStatus::Completed];

    for state in &states_to_test {
        monitor
            .update_status(
                op_id,
                *state,
                Some(json!({"state": format!("{:?}", state)})),
            )
            .await;

        // For terminal states, operation should move to completion history
        if state.is_terminal() {
            // Should not be in active operations
            let active = monitor.get_operation(op_id).await;
            assert!(active.is_none());

            // Should be retrievable via wait_for_operation
            let completed = monitor.wait_for_operation(op_id).await;
            assert!(completed.is_some());
            assert_eq!(completed.unwrap().state, *state);
        }
    }

    Ok(())
}

/// Test cancellation functionality and edge cases
#[tokio::test]
async fn test_cancellation_functionality_edge_cases() -> Result<()> {
    let monitor = OperationMonitor::new(MonitorConfig::with_timeout(Duration::from_secs(30)));

    // Test cancelling non-existent operation
    let cancel_nonexistent = monitor.cancel_operation("nonexistent_op").await;
    assert!(!cancel_nonexistent);

    // Test cancelling with reason
    let op_id = "cancel_test_op";
    let operation = Operation::new(
        op_id.to_string(),
        "cancel_tool".to_string(),
        "Cancellation test operation".to_string(),
        None,
    );

    monitor.add_operation(operation).await;

    // Cancel with specific reason
    let cancel_result = monitor
        .cancel_operation_with_reason(op_id, Some("Test cancellation".to_string()))
        .await;
    assert!(cancel_result);

    // Verify operation is cancelled and moved to history
    let cancelled_op = monitor.wait_for_operation(op_id).await;
    assert!(cancelled_op.is_some());
    let op = cancelled_op.unwrap();
    assert_eq!(op.state, OperationStatus::Cancelled);
    assert!(op.end_time.is_some());

    // Verify cancellation reason is stored
    if let Some(result) = op.result {
        assert_eq!(result["cancelled"], json!(true));
        assert_eq!(result["reason"], json!("Test cancellation"));
    }

    // Test cancelling already terminal operation
    let cancel_again = monitor.cancel_operation(op_id).await;
    assert!(!cancel_again); // Should return false for already terminal operation

    Ok(())
}

/// Test shutdown summary functionality
#[tokio::test]
async fn test_shutdown_summary_functionality() -> Result<()> {
    let monitor = OperationMonitor::new(MonitorConfig::with_timeout(Duration::from_secs(30)));

    // Create mix of active and completed operations
    let active_count = 3;
    let completed_count = 2;

    for i in 0..active_count {
        let op_id = format!("active_shutdown_op_{}", i);
        let operation = Operation::new(
            op_id.clone(),
            "active_tool".to_string(),
            format!("Active operation {}", i),
            None,
        );

        monitor.add_operation(operation).await;
        monitor
            .update_status(&op_id, OperationStatus::InProgress, None)
            .await;
    }

    for i in 0..completed_count {
        let op_id = format!("completed_shutdown_op_{}", i);
        let operation = Operation::new(
            op_id.clone(),
            "completed_tool".to_string(),
            format!("Completed operation {}", i),
            None,
        );

        monitor.add_operation(operation).await;
        monitor
            .update_status(&op_id, OperationStatus::Completed, Some(json!({"done": i})))
            .await;
    }

    // Get shutdown summary
    let summary = monitor.get_shutdown_summary().await;

    // Should only include active operations
    assert_eq!(summary.total_active, active_count);
    assert_eq!(summary.operations.len(), active_count);

    // Verify all operations in summary are non-terminal
    for op in &summary.operations {
        assert!(!op.state.is_terminal());
        assert_eq!(op.state, OperationStatus::InProgress);
    }

    Ok(())
}

/// Test timeout and timing edge cases
#[tokio::test]
async fn test_timeout_and_timing_edge_cases() -> Result<()> {
    // Use very short timeout to avoid long test duration
    let short_timeout = Duration::from_millis(50); // Reduced from 100ms
    let config = MonitorConfig::with_timeout(short_timeout);
    let monitor = OperationMonitor::new(config);

    let op_id = "timing_test_op";
    let operation = Operation::new(
        op_id.to_string(),
        "timing_tool".to_string(),
        "Timing test operation".to_string(),
        None,
    );

    let start_time = SystemTime::now();
    monitor.add_operation(operation).await;

    // Test first_wait_time is recorded
    // Use tokio::time::timeout to ensure this doesn't hang the test
    let wait_result = tokio::time::timeout(
        Duration::from_millis(200), // Max 200ms for the entire wait
        monitor.wait_for_operation(op_id),
    )
    .await;

    // The timeout should fire and return Err, or the wait should return None
    let waited_op: Option<ahma_mcp::operation_monitor::Operation> = wait_result.unwrap_or_default();

    // Since operation never completes, this should be None
    assert!(waited_op.is_none());

    // Check operation is still active and first_wait_time was set
    let active_op = monitor.get_operation(op_id).await;
    assert!(active_op.is_some());
    let op = active_op.unwrap();
    assert!(op.first_wait_time.is_some());

    // Verify timing relationships
    let first_wait = op.first_wait_time.unwrap();
    assert!(first_wait >= start_time);
    assert!(first_wait <= SystemTime::now());

    Ok(())
}

/// Test error conditions and resilience
#[tokio::test]
async fn test_error_conditions_and_resilience() -> Result<()> {
    let monitor = OperationMonitor::new(MonitorConfig::with_timeout(Duration::from_secs(30)));

    // Test operations with various error states
    let error_states = [
        OperationStatus::Failed,
        OperationStatus::TimedOut,
        OperationStatus::Cancelled,
    ];

    for (i, &error_state) in error_states.iter().enumerate() {
        let op_id = format!("error_test_op_{}", i);
        let operation = Operation::new(
            op_id.clone(),
            "error_tool".to_string(),
            format!("Error test operation {}", i),
            None,
        );

        monitor.add_operation(operation).await;

        // Transition to error state
        monitor
            .update_status(
                &op_id,
                error_state,
                Some(json!({"error_type": format!("{:?}", error_state)})),
            )
            .await;

        // Verify operation is moved to completion history
        let error_op = monitor.wait_for_operation(&op_id).await;
        assert!(error_op.is_some());
        let op = error_op.unwrap();
        assert_eq!(op.state, error_state);
        assert!(op.state.is_terminal());
        assert!(op.end_time.is_some());
    }

    // Verify monitor continues to function normally after errors
    let normal_op_id = "normal_after_errors";
    let normal_operation = Operation::new(
        normal_op_id.to_string(),
        "normal_tool".to_string(),
        "Normal operation after errors".to_string(),
        None,
    );

    monitor.add_operation(normal_operation).await;
    monitor
        .update_status(
            normal_op_id,
            OperationStatus::Completed,
            Some(json!({"success": true})),
        )
        .await;

    let normal_result = monitor.wait_for_operation(normal_op_id).await;
    assert!(normal_result.is_some());
    assert_eq!(normal_result.unwrap().state, OperationStatus::Completed);

    Ok(())
}
