use ahma_mcp::{
    adapter::Adapter,
    operation_monitor::{MonitorConfig, OperationMonitor, OperationStatus},
    sandbox::Sandbox,
    shell_pool::{ShellPoolConfig, ShellPoolManager},
    test_utils::concurrency::wait_for_condition,
};
use serde_json::{Map, Value};
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::time::timeout;

/// Test that operations can be cancelled successfully
#[tokio::test]
async fn test_operation_cancellation_functionality() {
    // Initialize logging for the test
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();

    // Create a temporary directory for testing
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let temp_path = temp_dir.path().to_string_lossy().to_string();

    // Set up operation monitor
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));

    // Set up shell pool
    let shell_config = ShellPoolConfig {
        enabled: true,
        command_timeout: Duration::from_secs(30),
        ..Default::default()
    };
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));

    // Create sandbox with temp_dir and /tmp
    let scopes = vec![temp_dir.path().to_path_buf(), std::env::temp_dir()];
    let sandbox = Arc::new(
        Sandbox::new(
            scopes,
            ahma_mcp::sandbox::SandboxMode::Test,
            false,
            false,
            false,
        )
        .unwrap(),
    );

    // Create adapter
    let adapter = Arc::new(
        Adapter::new(operation_monitor.clone(), shell_pool, sandbox)
            .expect("Failed to create adapter"),
    );

    println!("🧪 Starting operation cancellation test...");

    // Start a long-running command using echo in a loop (more reliable than sleep)
    let id = adapter
        .execute_async_in_dir(
            "test_slow_command",
            "sh",
            Some({
                let mut args = Map::new();
                args.insert("command".to_string(), Value::String("-c".to_string()));
                args.insert(
                    "script".to_string(),
                    Value::String(
                        "for i in $(seq 1 20); do echo \"Step $i\"; sleep 0.5; done".to_string(),
                    ),
                );
                args
            }),
            &temp_path,
            Some(30), // 30 second timeout
            None,
        )
        .await
        .expect("Failed to start async operation");

    println!("🚀 Started long-running operation: {}", id);

    // Wait until the operation is at least InProgress
    let _ = wait_for_condition(Duration::from_secs(5), Duration::from_millis(50), || {
        let operation_monitor = operation_monitor.clone();
        let id = id.clone();
        async move {
            operation_monitor
                .get_operation(&id)
                .await
                .map(|op| op.state != OperationStatus::Pending)
                .unwrap_or(false)
        }
    })
    .await;

    // Check if the operation is still running, or if it completed/failed immediately
    let operation = operation_monitor.get_operation(&id).await;

    if let Some(op) = operation {
        if op.state == OperationStatus::InProgress {
            println!("OK Confirmed operation is in progress");

            // Cancel the operation
            let cancelled = operation_monitor.cancel_operation(&id).await;
            assert!(cancelled, "Operation cancellation should succeed");
            println!("OK Operation cancellation succeeded");

            // Wait for cancellation to be processed using the reliable watch channel.
            // `wait_for_operation` returns once the op reaches a terminal state and is
            // visible in completion history — no polling needed.
            let cancelled_op = timeout(
                Duration::from_secs(5),
                operation_monitor.wait_for_operation(&id),
            )
            .await
            .expect("Timed out waiting for cancellation")
            .expect("Operation should be in completion history after cancellation");

            assert_eq!(
                cancelled_op.state,
                OperationStatus::Cancelled,
                "Operation should be in Cancelled state"
            );
            println!("OK Operation state is correctly set to Cancelled");
        } else {
            println!(
                "WARNING Operation completed/failed too quickly to test cancellation, state: {:?}",
                op.state
            );
            if let Some(result) = &op.result {
                println!("Operation result: {:?}", result);
            }
            // We can still test the cancellation logic, even if the operation completed
            let cancelled = operation_monitor.cancel_operation(&id).await;
            assert!(
                !cancelled,
                "Should not be able to cancel already completed operation"
            );
            println!("OK Correctly prevented cancellation of completed operation");
        }
    } else {
        // Operation might have moved to completion history already
        let completed_ops = operation_monitor.get_completed_operations().await;
        let completed_op = completed_ops.iter().find(|op| op.id == id);
        if let Some(op) = completed_op {
            println!(
                "WARNING Operation completed too quickly, final state: {:?}",
                op.state
            );
            if let Some(result) = &op.result {
                println!("Operation result: {:?}", result);
            }
        } else {
            panic!("Operation {} disappeared completely", id);
        }
    }

    // Verify that attempting to cancel an already cancelled operation returns false
    let second_cancel = operation_monitor.cancel_operation(&id).await;
    assert!(
        !second_cancel,
        "Second cancellation attempt should return false"
    );
    println!("OK Second cancellation attempt correctly returned false");

    // Test cancelling a non-existent operation
    let fake_cancel = operation_monitor.cancel_operation("nonexistent_id").await;
    assert!(
        !fake_cancel,
        "Cancelling non-existent operation should return false"
    );
    println!("OK Cancelling non-existent operation correctly returned false");

    println!("🎉 All operation cancellation tests passed!");
}

/// Test cancellation of an operation before it starts executing
#[tokio::test]
async fn test_operation_cancellation_before_execution() {
    // Initialize logging for the test
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();

    // Create operation monitor
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(10));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));

    // Create a simple operation and immediately cancel it
    let operation = ahma_mcp::operation_monitor::Operation::new(
        "test_op_1".to_string(),
        "test_tool".to_string(),
        "Test operation".to_string(),
        None,
    );

    // Add operation to monitor
    operation_monitor.add_operation(operation).await;

    // Immediately cancel the operation
    let cancelled = operation_monitor.cancel_operation("test_op_1").await;
    assert!(cancelled, "Operation should be successfully cancelled");

    // Verify the cancellation token is triggered
    let operation = operation_monitor.get_operation("test_op_1").await;
    if let Some(op) = operation {
        assert!(
            op.cancellation_token.is_cancelled(),
            "Cancellation token should be cancelled"
        );
        assert_eq!(op.state, OperationStatus::Cancelled);
    } else {
        // Check completion history if not in active operations
        let completed_ops = operation_monitor.get_completed_operations().await;
        let cancelled_op = completed_ops.iter().find(|op| op.id == "test_op_1");
        assert!(cancelled_op.is_some(), "Cancelled operation should exist");
        assert_eq!(cancelled_op.unwrap().state, OperationStatus::Cancelled);
    }

    println!("OK Pre-execution cancellation test passed");
}

/// Test the basic cancellation token functionality
#[tokio::test]
async fn test_cancellation_token_integration() {
    // Initialize logging for the test
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();

    // Create operation monitor
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(10));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));

    // Create an operation
    let operation = ahma_mcp::operation_monitor::Operation::new(
        "token_test_op".to_string(),
        "test_tool".to_string(),
        "Test cancellation token".to_string(),
        None,
    );

    // Verify the cancellation token is initially not cancelled
    assert!(
        !operation.cancellation_token.is_cancelled(),
        "Initial token should not be cancelled"
    );

    // Add to monitor
    operation_monitor.add_operation(operation).await;

    // Cancel the operation
    let cancelled = operation_monitor.cancel_operation("token_test_op").await;
    assert!(cancelled, "Operation should be successfully cancelled");

    // Verify the cancellation token is now cancelled
    let cancelled_operation = operation_monitor.get_operation("token_test_op").await;
    if let Some(op) = cancelled_operation {
        assert!(
            op.cancellation_token.is_cancelled(),
            "Token should be cancelled after cancel_operation"
        );
    } else {
        // Check completion history
        let completed_ops = operation_monitor.get_completed_operations().await;
        let cancelled_op = completed_ops.iter().find(|op| op.id == "token_test_op");
        assert!(
            cancelled_op.is_some(),
            "Cancelled operation should exist in completion history"
        );
        assert!(
            cancelled_op.unwrap().cancellation_token.is_cancelled(),
            "Token should be cancelled"
        );
    }

    println!("OK Cancellation token integration test passed");
}
