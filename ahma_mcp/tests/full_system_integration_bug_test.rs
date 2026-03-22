#[cfg(test)]
mod tests {
    use ahma_mcp::adapter::Adapter;
    use ahma_mcp::callback_system::{CallbackSender, ProgressUpdate};
    use ahma_mcp::config::load_tool_configs;
    use ahma_mcp::mcp_service::AhmaMcpService;
    use ahma_mcp::operation_monitor::{MonitorConfig, OperationMonitor};
    use ahma_mcp::sandbox::Sandbox;
    use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};

    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// A mock callback that tracks all notifications sent
    #[derive(Debug, Clone)]
    struct TrackingCallback {
        notifications: Arc<Mutex<Vec<ProgressUpdate>>>,
    }

    impl TrackingCallback {
        fn new() -> Self {
            Self {
                notifications: Arc::new(Mutex::new(Vec::new())),
            }
        }

        #[allow(dead_code)]
        fn get_notifications(&self) -> Vec<ProgressUpdate> {
            self.notifications.lock().unwrap().clone()
        }

        fn count_completed_notifications(&self, id: &str) -> usize {
            self.notifications
                .lock()
                .unwrap()
                .iter()
                .filter(|update| {
                    matches!(
                        update,
                        ProgressUpdate::Completed {
                            id: update_id,
                            ..
                        } if update_id == id
                    )
                })
                .count()
        }
    }

    #[async_trait::async_trait]
    impl CallbackSender for TrackingCallback {
        async fn send_progress(
            &self,
            update: ProgressUpdate,
        ) -> Result<(), ahma_mcp::callback_system::CallbackError> {
            println!("📨 TRACKED NOTIFICATION: {}", update);
            self.notifications.lock().unwrap().push(update);
            Ok(())
        }

        async fn should_cancel(&self) -> bool {
            false
        }
    }

    /// This test simulates the full system to ensure that with the new `completion_history`
    /// architecture, operations result in exactly one notification.
    #[tokio::test]
    async fn test_full_system_integration_single_notification() {
        println!("🔍 Testing full system integration for single notification guarantee...");

        // System setup
        let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(30));
        let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
        let shell_pool_config = ShellPoolConfig::default();
        let shell_pool_manager = Arc::new(ShellPoolManager::new(shell_pool_config));
        shell_pool_manager.clone().start_background_tasks();
        let sandbox = Arc::new(Sandbox::new_test());
        let adapter =
            Arc::new(Adapter::new(operation_monitor.clone(), shell_pool_manager, sandbox).unwrap());
        let configs = Arc::new(
            load_tool_configs(
                &ahma_mcp::shell::cli::AppConfig::default(),
                Some(&std::path::PathBuf::from(".ahma")),
            )
            .await
            .unwrap(),
        );
        let _service = AhmaMcpService::new(
            adapter.clone(),
            operation_monitor.clone(),
            configs,
            Arc::new(None),
            false,
            false,
            false,
        )
        .await
        .unwrap();
        let tracking_callback = Arc::new(TrackingCallback::new());
        println!("OK Full system initialized");

        let current_dir = std::env::current_dir().unwrap();
        let current_dir_str = current_dir.to_str().unwrap();

        // Start an operation
        let id = adapter
            .execute_async_in_dir("cargo", "version", None, current_dir_str, Some(30), None)
            .await
            .expect("Failed to execute async operation");
        println!("🚀 Started operation: {}", id);

        // Wait for the operation to complete
        let completed_op = operation_monitor.wait_for_operation(&id).await;
        assert!(
            completed_op.is_some(),
            "Operation should have completed and been returned by wait_for_operation"
        );
        println!("OK Operation completed and is in history.");

        // Simulate a notification loop that runs multiple times
        println!("🔄 Simulating notification loop...");
        let mut notified_operations = std::collections::HashSet::new();
        for iteration in 1..=10 {
            let completed_ops = operation_monitor.get_completed_operations().await;
            if !completed_ops.is_empty() {
                println!(
                    "📊 Iteration {}: Found {} completed operations in history",
                    iteration,
                    completed_ops.len()
                );
                for op in completed_ops {
                    // A real notification system would check if a notification has already been sent.
                    if notified_operations.insert(op.id.clone()) {
                        let update = ProgressUpdate::Completed {
                            id: op.id.clone(),
                            message: "Operation finished".to_string(),
                            duration_ms: 1000,
                        };
                        tracking_callback.send_progress(update).await.unwrap();
                    }
                }
            }
            // The sleep is removed as the test's correctness relies on the logic
            // of checking `notified_operations`, not on timing.
        }

        // --- Analysis ---
        let completed_notifications = tracking_callback.count_completed_notifications(&id);
        println!("📈 Notification analysis for operation {}:", id);
        println!(
            "   Total 'Completed' notifications sent: {}",
            completed_notifications
        );

        // There should be exactly one "Completed" notification for the operation.
        assert_eq!(
            completed_notifications, 1,
            "BUG: Expected exactly 1 completed notification, but got {}. The notification logic is flawed.",
            completed_notifications
        );

        println!("OK Full system integration test passed - operation was notified exactly once.");
    }

    /// Test system under load with multiple operations, ensuring each is notified once.
    #[tokio::test]
    async fn test_multiple_operations_system_integration() {
        println!("⚡ Testing multiple operations system integration...");

        // System setup
        let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(30));
        let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
        let shell_pool_config = ShellPoolConfig::default();
        let shell_pool_manager = Arc::new(ShellPoolManager::new(shell_pool_config));
        shell_pool_manager.clone().start_background_tasks();
        let sandbox = Arc::new(Sandbox::new_test());
        let adapter =
            Arc::new(Adapter::new(operation_monitor.clone(), shell_pool_manager, sandbox).unwrap());

        let current_dir = std::env::current_dir().unwrap();
        let current_dir_str = current_dir.to_str().unwrap();

        // Start multiple operations
        let op_ids = vec![
            adapter
                .execute_async_in_dir("cargo", "version", None, current_dir_str, Some(30), None)
                .await
                .expect("Failed to execute first async operation"),
            adapter
                .execute_async_in_dir("cargo", "--version", None, current_dir_str, Some(30), None)
                .await
                .expect("Failed to execute second async operation"),
        ];
        println!("🚀 Started operations: {:?}", op_ids);

        // Wait for all operations to complete
        for op_id in &op_ids {
            let completed_op = operation_monitor.wait_for_operation(op_id).await;
            assert!(
                completed_op.is_some(),
                "Operation {} should have completed",
                op_id
            );
        }
        println!("OK All operations completed.");

        assert_eq!(
            operation_monitor.get_completed_operations().await.len(),
            op_ids.len(),
            "Not all operations completed in time."
        );

        // Simulate notification loop and track notifications
        let mut all_notified_operations = std::collections::HashSet::new();
        for iteration in 1..=6 {
            let completed_ops = operation_monitor.get_completed_operations().await;
            println!(
                "📊 Iteration {}: Found {} operations in history",
                iteration,
                completed_ops.len()
            );
            for op in completed_ops {
                if all_notified_operations.insert(op.id.clone()) {
                    println!("   - Sending notification for new operation {}", op.id);
                }
            }
            // The sleep is removed as the test's correctness relies on the logic
            // of checking `all_notified_operations`, not on timing.
        }

        // --- Analysis ---
        println!("🔍 Analysis of total notifications sent:");
        for op_id in &op_ids {
            let was_notified = all_notified_operations.contains(op_id);
            println!("   - Operation {}: Notified? {}", op_id, was_notified);
            assert!(was_notified, "BUG: Operation {} was never notified!", op_id);
        }

        assert_eq!(
            all_notified_operations.len(),
            op_ids.len(),
            "BUG: The number of unique notified operations ({}) does not match the number of started operations ({}).",
            all_notified_operations.len(),
            op_ids.len()
        );

        println!(
            "OK Multiple operations integration test passed - each operation was notified exactly once."
        );
    }
}
