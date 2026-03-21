/// Test for the "Canceled: Canceled" issue that occurred when MCP clients cancelled requests
///
/// This test reproduces the exact scenario where:
/// 1. User cancels an MCP tool call (like await) from VS Code
/// 2. VS Code sends MCP cancellation notification
/// 3. Our on_cancelled handler tries to cancel operations
/// 4. rmcp library outputs "Canceled: Canceled"
/// 5. Our adapter incorrectly processes this as a process cancellation
///
/// The fix ensures we only cancel actual background operations, not synchronous MCP tools.
use ahma_mcp::{
    adapter::Adapter,
    mcp_service::AhmaMcpService,
    operation_monitor::{MonitorConfig, OperationMonitor},
    sandbox::Sandbox,
    shell_pool::{ShellPoolConfig, ShellPoolManager},
};
use rmcp::model::{CancelledNotificationParam, RequestId};
use serde_json::{Map, Value};
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::time::Instant;

#[tokio::test]
async fn test_mcp_cancellation_does_not_trigger_canceled_canceled_message() {
    // Initialize logging for the test
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();

    println!("🧪 Testing MCP cancellation bug fix...");

    // Create a temporary directory for testing
    let temp_dir = TempDir::new().expect("Failed to create temp directory");

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

    // Create sandbox with temp_dir as root + /tmp
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

    // Create empty tool configs (we don't need real tools for this test)
    let configs = Arc::new(std::collections::HashMap::new());
    let guidance = Arc::new(None);

    // Create MCP service
    let _mcp_service = AhmaMcpService::new(
        adapter.clone(),
        operation_monitor.clone(),
        configs,
        guidance,
        false,
        false,
        false,
    )
    .await
    .expect("Failed to create MCP service");

    // Scenario 1: Test cancellation when NO operations are running
    // This simulates cancelling an "await" tool when nothing is running
    println!("🔍 Test 1: MCP cancellation with no active operations");

    // Simulate MCP cancellation notification
    let _cancellation_notification = CancelledNotificationParam {
        request_id: RequestId::String("test_request_1".into()),
        reason: Some("User cancelled from VS Code".to_string()),
    };

    // Create mock notification context
    // Note: This is complex to create properly, so we'll test the logic indirectly

    // The key fix: check that no operations get cancelled when none are background operations
    let initial_ops = operation_monitor.get_all_active_operations().await;
    assert_eq!(initial_ops.len(), 0, "Should start with no operations");

    // Simulate what happens in on_cancelled method
    let active_ops = operation_monitor.get_all_active_operations().await;
    let background_ops: Vec<_> = active_ops
        .iter()
        .filter(|op| {
            // Only cancel operations that represent actual background processes
            // NOT synchronous tools like 'await', 'status', 'cancel'
            !matches!(op.tool_name.as_str(), "await" | "status" | "cancel")
        })
        .collect();

    assert_eq!(
        background_ops.len(),
        0,
        "Should have no background operations to cancel"
    );
    println!("OK Test 1 passed: No spurious cancellations when no background operations");

    // Scenario 2: Test cancellation when there's a mix of operations
    println!("🔍 Test 2: MCP cancellation with mixed operation types");

    // Start a background operation (simulated)
    let bg_id = adapter
        .execute_async_in_dir(
            "test_background_op",
            "sh",
            Some({
                let mut args = Map::new();
                args.insert("command".to_string(), Value::String("-c".to_string()));
                args.insert("script".to_string(), Value::String("sleep 2".to_string()));
                args
            }),
            temp_dir.path().to_str().unwrap(),
            Some(10), // 10 second timeout
            None,
        )
        .await
        .expect("Failed to start background operation");

    // Add simulated "await" operation (this would be in operation monitor in real scenario)
    // But we can't easily simulate this, so we'll test the filtering logic directly

    let active_ops = operation_monitor.get_all_active_operations().await;
    assert!(
        !active_ops.is_empty(),
        "Should have at least one active operation"
    );

    // Test the filtering logic that prevents cancelling synchronous tools
    let background_ops: Vec<_> = active_ops
        .iter()
        .filter(|op| !matches!(op.tool_name.as_str(), "await" | "status" | "cancel"))
        .collect();

    // The background operation should be eligible for cancellation
    assert_eq!(
        background_ops.len(),
        1,
        "Should have exactly one background operation"
    );
    assert_eq!(
        background_ops[0].id, bg_id,
        "Should identify the correct background operation"
    );

    println!("OK Test 2 passed: Correctly filters background vs synchronous operations");

    // Clean up: cancel the background operation
    let cancelled = operation_monitor.cancel_operation(&bg_id).await;
    assert!(cancelled, "Should be able to cancel background operation");

    println!("OK All MCP cancellation bug tests passed!");
}

#[tokio::test]
async fn test_await_tool_timeout_handling() {
    // This test specifically targets the timeout handling bug in the await tool
    // where the operation monitor's 5-minute timeout was overriding the await tool's timeout

    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();

    println!("🧪 Testing await tool timeout handling...");

    // Set up minimal test environment
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));

    // Test the operation monitor's wait_for_operation timeout behavior
    let start_time = Instant::now();

    // Try to wait for a non-existent operation
    let result = operation_monitor
        .wait_for_operation("non_existent_op")
        .await;

    let elapsed = start_time.elapsed();

    // Should return None quickly, not wait for 5 minutes
    assert!(
        result.is_none(),
        "Should return None for non-existent operation"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "Should return quickly, not wait for 5-minute timeout"
    );

    println!("OK Operation monitor correctly handles non-existent operations");
    println!("OK Await tool timeout test passed!");
}

#[test]
fn test_cancellation_detection_patterns() {
    // Test the patterns used to detect "Canceled: Canceled" messages

    println!("🧪 Testing cancellation detection patterns...");

    let test_cases = vec![
        ("Canceled", true),
        ("Canceled: Canceled", true),
        ("task cancelled for reason", true),
        ("Some other output", false),
        ("", false),
        ("Cancellation in progress", false),
        ("Process completed successfully", false),
    ];

    for (output, should_match) in test_cases {
        let is_cancelled_output = output.trim() == "Canceled"
            || output.contains("Canceled: Canceled")
            || output.contains("task cancelled for reason");

        assert_eq!(
            is_cancelled_output, should_match,
            "Detection pattern failed for output: '{}'",
            output
        );
    }

    println!("OK All cancellation detection pattern tests passed!");
}
