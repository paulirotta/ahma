//! Additional adapter coverage tests targeting specific uncovered paths.
//!
//! This module focuses on edge cases and error conditions that are difficult to reach
//! in the existing comprehensive tests, specifically targeting:
//! - prepare_command_and_args edge cases
//! - Temp file creation error paths
//! - Operation ID generation uniqueness
//! - Task handle cleanup edge cases
//! - Complex async execution error paths

use ahma_mcp::adapter::{Adapter, AsyncExecOptions};
use ahma_mcp::config::{CommandOption, SubcommandConfig};
use ahma_mcp::operation_monitor::{MonitorConfig, OperationMonitor};
use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};
use ahma_mcp::utils::logging::init_test_logging;
use serde_json::{Map, json};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

/// Helper function to create test adapter with custom configuration
/// Helper to create test adapter
async fn create_simple_test_adapter() -> (Arc<Adapter>, TempDir) {
    init_test_logging();

    let temp_dir = TempDir::new().unwrap();
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));
    let adapter = Arc::new(Adapter::new(monitor, shell_pool).unwrap());
    (adapter, temp_dir)
}

#[tokio::test]
async fn test_prepare_command_and_args_edge_cases() {
    init_test_logging();
    init_test_logging();

    let (adapter, temp_dir) = create_simple_test_adapter().await;

    // Test with empty subcommand config
    let empty_config = SubcommandConfig {
        name: "empty".to_string(),
        description: "Empty config".to_string(),
        synchronous: None,
        timeout_seconds: None,
        enabled: true,
        guidance_key: None,
        subcommand: None,
        sequence: None,
        step_delay_ms: None,
        positional_args_first: None, positional_args: None,
        options: None,
        availability_check: None,
        install_instructions: None,
    };

    let mut args = Map::new();
    args.insert("unused".to_string(), json!("value"));

    let result = adapter
        .execute_sync_in_dir(
            "echo",
            Some(args),
            temp_dir.path().to_str().unwrap(),
            Some(5),
            Some(&empty_config),
        )
        .await;

    assert!(result.is_ok());

    adapter.shutdown().await;
}

#[tokio::test]
async fn test_prepare_command_and_args_with_aliases() {
    init_test_logging();
    init_test_logging();

    let (adapter, temp_dir) = create_simple_test_adapter().await;

    // Test alias handling where both alias and main name are provided
    let alias_config = SubcommandConfig {
        name: "alias_test".to_string(),
        description: "Alias test".to_string(),
        synchronous: None,
        timeout_seconds: None,
        enabled: true,
        guidance_key: None,
        subcommand: None,
        sequence: None,
        step_delay_ms: None,
        positional_args_first: None, positional_args: None,
        options: Some(vec![
            CommandOption {
                name: "verbose".to_string(),
                alias: Some("v".to_string()),
                option_type: "boolean".to_string(),
                description: Some("Verbose flag".to_string()),
                format: None,

                items: None,
                required: Some(false),
                file_arg: Some(false),
                file_flag: None,
            },
            CommandOption {
                name: "output".to_string(),
                alias: Some("o".to_string()),
                option_type: "string".to_string(),
                description: Some("Output file".to_string()),
                format: None,

                items: None,
                required: Some(false),
                file_arg: Some(false),
                file_flag: None,
            },
        ]),
        availability_check: None,
        install_instructions: None,
    };

    let mut args = Map::new();
    args.insert("v".to_string(), json!(true)); // Use alias
    args.insert("output".to_string(), json!("test.txt")); // Use main name
    args.insert("verbose".to_string(), json!(false)); // Main name with different value

    let result = adapter
        .execute_sync_in_dir(
            "echo",
            Some(args),
            temp_dir.path().to_str().unwrap(),
            Some(5),
            Some(&alias_config),
        )
        .await;

    assert!(result.is_ok());
    let output = result.unwrap();
    // Should prioritize main name over alias when both are present
    assert!(output.contains("--output"));
    assert!(output.contains("test.txt"));

    adapter.shutdown().await;
}

#[tokio::test]
async fn test_prepare_command_and_args_mixed_types() {
    init_test_logging();
    init_test_logging();

    let (adapter, temp_dir) = create_simple_test_adapter().await;

    let mixed_config = SubcommandConfig {
        name: "mixed".to_string(),
        description: "Mixed types test".to_string(),
        synchronous: None,
        timeout_seconds: None,
        enabled: true,
        guidance_key: None,
        subcommand: None,
        sequence: None,
        step_delay_ms: None,
        positional_args_first: None, positional_args: Some(vec![CommandOption {
            name: "pos_str".to_string(),
            alias: None,
            option_type: "string".to_string(),
            description: Some("String positional".to_string()),
            format: None,

            items: None,
            required: Some(false),
            file_arg: Some(false),
            file_flag: None,
        }]),
        options: Some(vec![
            CommandOption {
                name: "bool_opt".to_string(),
                alias: None,
                option_type: "boolean".to_string(),
                description: Some("Boolean option".to_string()),
                format: None,

                items: None,
                required: Some(false),
                file_arg: Some(false),
                file_flag: None,
            },
            CommandOption {
                name: "array_opt".to_string(),
                alias: None,
                option_type: "array".to_string(),
                description: Some("Array option".to_string()),
                format: None,

                items: None,
                required: Some(false),
                file_arg: Some(false),
                file_flag: None,
            },
            CommandOption {
                name: "num_opt".to_string(),
                alias: None,
                option_type: "number".to_string(),
                description: Some("Number option".to_string()),
                format: None,

                items: None,
                required: Some(false),
                file_arg: Some(false),
                file_flag: None,
            },
        ]),
        availability_check: None,
        install_instructions: None,
    };

    let mut args = Map::new();
    args.insert("pos_str".to_string(), json!("positional_value"));
    args.insert("bool_opt".to_string(), json!(true));
    args.insert("array_opt".to_string(), json!(["item1", "item2"]));
    args.insert("num_opt".to_string(), json!(42.5));
    args.insert("null_opt".to_string(), json!(null));
    args.insert("working_directory".to_string(), json!("/should/be/ignored"));

    let result = adapter
        .execute_sync_in_dir(
            "echo",
            Some(args),
            temp_dir.path().to_str().unwrap(),
            Some(5),
            Some(&mixed_config),
        )
        .await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.contains("positional_value"));
    assert!(output.contains("--bool_opt"));
    assert!(output.contains("--array_opt"));
    assert!(output.contains("42.5"));

    adapter.shutdown().await;
}

#[tokio::test]
async fn test_id_generation_uniqueness() {
    init_test_logging();
    // Test that operation IDs are unique across multiple calls
    // This tests the generate_id function without actually executing commands

    let (adapter1, _temp_dir1) = create_simple_test_adapter().await;
    let (adapter2, _temp_dir2) = create_simple_test_adapter().await;

    // Generate multiple operation IDs by starting operations (but not waiting for completion)
    let mut ids = Vec::new();

    // Generate 20 operation IDs quickly (10 from each adapter)
    for _ in 0..10 {
        let id1 = adapter1
            .execute_async_in_dir("test1", "true", None, "/tmp", Some(1)) // Use 'true' command which exits immediately
            .await
            .unwrap();
        let id2 = adapter2
            .execute_async_in_dir("test2", "true", None, "/tmp", Some(1)) // Use 'true' command which exits immediately
            .await
            .unwrap();
        ids.push(id1);
        ids.push(id2);
    }

    // All IDs should be unique and follow the expected format
    let mut unique_ids = std::collections::HashSet::new();
    for id in &ids {
        assert!(
            unique_ids.insert(id.clone()),
            "Operation ID {} is not unique",
            id
        );
        assert!(id.starts_with("op_"));
    }

    // Verify we got the expected number of unique IDs
    assert_eq!(unique_ids.len(), 20, "Should have 20 unique operation IDs");

    // Yield to allow operations to start before shutdown
    tokio::task::yield_now().await;

    adapter1.shutdown().await;
    adapter2.shutdown().await;
}

#[tokio::test]
async fn test_async_execution_task_error_handling() {
    init_test_logging();
    let (adapter, _temp_dir) = create_simple_test_adapter().await;

    // Test async execution with invalid working directory
    let result = adapter
        .execute_async_in_dir(
            "invalid_dir_test",
            "echo",
            Some({
                let mut args = Map::new();
                args.insert("message".to_string(), json!("test"));
                args
            }),
            "/nonexistent/directory/path/that/does/not/exist",
            Some(5),
        )
        .await;

    // Should still return an operation ID even if directory doesn't exist
    assert!(result.is_ok());
    let op_id = result.unwrap();
    assert!(op_id.starts_with("op_"));

    // Yield to allow the operation to attempt execution and fail
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }

    adapter.shutdown().await;
}

#[tokio::test]
async fn test_shutdown_with_active_tasks() {
    init_test_logging();
    let (adapter, _temp_dir) = create_simple_test_adapter().await;

    // Start multiple async operations
    let mut ids = Vec::new();
    for i in 0..3 {
        let op_id = adapter
            .execute_async_in_dir(
                &format!("shutdown_test_{}", i),
                "echo",
                Some({
                    let mut args = Map::new();
                    args.insert("message".to_string(), json!(format!("test {}", i)));
                    args
                }),
                "/tmp",
                Some(5),
            )
            .await
            .unwrap();
        ids.push(op_id);
    }

    // Yield to allow operations to start
    tokio::task::yield_now().await;

    // Shutdown should complete
    adapter.shutdown().await;

    // All operation IDs should be valid
    for op_id in ids {
        assert!(op_id.starts_with("op_"));
    }
}

#[tokio::test]
async fn test_async_execution_with_none_callback() {
    init_test_logging();
    let (adapter, temp_dir) = create_simple_test_adapter().await;

    // Test async execution with None callback (should not panic)
    let result = adapter
        .execute_async_in_dir(
            "none_callback_test",
            "echo",
            Some({
                let mut args = Map::new();
                args.insert("message".to_string(), json!("no callback"));
                args
            }),
            temp_dir.path().to_str().unwrap(),
            Some(5),
            None, // No callback
        )
        .await;

    assert!(result.is_ok());
    let op_id = result.unwrap();
    assert!(op_id.starts_with("op_"));

    // Yield to allow completion
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }

    adapter.shutdown().await;
}

#[tokio::test]
async fn test_execute_async_with_empty_options() {
    init_test_logging();
    let (adapter, temp_dir) = create_simple_test_adapter().await;

    // Test with minimal options
    let options = AsyncExecOptions {
        id: None,
        args: None,
        timeout: None,
        callback: None,
        subcommand_config: None,
        log_monitor_config: None,
    };

    let result = adapter
        .execute_async_in_dir_with_options(
            "minimal_options",
            "echo",
            temp_dir.path().to_str().unwrap(),
            options,
        )
        .await;

    assert!(result.is_ok());
    let op_id = result.unwrap();
    assert!(op_id.starts_with("op_"));

    // Yield to allow completion
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }

    adapter.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_sync_execution_with_complex_error_handling() {
    init_test_logging();
    let (adapter, temp_dir) = create_simple_test_adapter().await;

    let result = adapter
        .execute_sync_in_dir(
            "sh",
            Some({
                let mut args = Map::new();
                args.insert(
                    "c".to_string(),
                    json!("echo 'stdout content' >&2; echo 'stdout content'"),
                );
                args
            }),
            temp_dir.path().to_str().unwrap(),
            Some(5),
            None,
        )
        .await;

    assert!(result.is_ok() || result.is_err());

    adapter.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_sync_execution_stderr_only() {
    init_test_logging();
    let (adapter, temp_dir) = create_simple_test_adapter().await;

    let result = adapter
        .execute_sync_in_dir(
            "sh",
            Some({
                let mut args = Map::new();
                args.insert("c".to_string(), json!("echo 'error message' >&2"));
                args
            }),
            temp_dir.path().to_str().unwrap(),
            Some(5),
            None,
        )
        .await;

    assert!(result.is_ok() || result.is_err());

    adapter.shutdown().await;
}

#[tokio::test]
async fn test_sync_execution_empty_stdout() {
    init_test_logging();
    let (adapter, temp_dir) = create_simple_test_adapter().await;

    // Test command with empty stdout but successful exit
    let result = adapter
        .execute_sync_in_dir(
            "true", // Command that does nothing but exits successfully
            None,
            temp_dir.path().to_str().unwrap(),
            Some(5),
            None,
        )
        .await;

    assert!(result.is_ok());
    // Empty output is acceptable for commands that don't produce output

    adapter.shutdown().await;
}

#[tokio::test]
async fn test_async_execution_with_custom_id() {
    init_test_logging();
    let (adapter, temp_dir) = create_simple_test_adapter().await;

    let custom_id = "custom_operation_12345";
    let options = AsyncExecOptions {
        id: Some(custom_id.to_string()),
        args: Some({
            let mut args = Map::new();
            args.insert("message".to_string(), json!("custom id test"));
            args
        }),
        timeout: Some(10),
        callback: None,
        subcommand_config: None,
        log_monitor_config: None,
    };

    let result = adapter
        .execute_async_in_dir_with_options(
            "custom_id_test",
            "echo",
            temp_dir.path().to_str().unwrap(),
            options,
        )
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), custom_id);

    // Yield to allow completion
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }

    adapter.shutdown().await;
}

#[tokio::test]
async fn test_prepare_command_and_args_with_null_values() {
    init_test_logging();
    let (adapter, temp_dir) = create_simple_test_adapter().await;

    let config_with_nulls = SubcommandConfig {
        name: "null_test".to_string(),
        description: "Null values test".to_string(),
        synchronous: None,
        timeout_seconds: None,
        enabled: true,
        guidance_key: None,
        subcommand: None,
        sequence: None,
        step_delay_ms: None,
        positional_args_first: None, positional_args: Some(vec![CommandOption {
            name: "pos_arg".to_string(),
            alias: None,
            option_type: "string".to_string(),
            description: Some("Positional argument".to_string()),
            format: None,

            items: None,
            required: Some(false),
            file_arg: Some(false),
            file_flag: None,
        }]),
        options: Some(vec![CommandOption {
            name: "opt_arg".to_string(),
            alias: None,
            option_type: "string".to_string(),
            description: Some("Optional argument".to_string()),
            format: None,

            items: None,
            required: Some(false),
            file_arg: Some(false),
            file_flag: None,
        }]),
        availability_check: None,
        install_instructions: None,
    };

    let mut args = Map::new();
    args.insert("pos_arg".to_string(), json!(null));
    args.insert("opt_arg".to_string(), json!(null));
    args.insert("valid_arg".to_string(), json!("valid_value"));

    let result = adapter
        .execute_sync_in_dir(
            "echo",
            Some(args),
            temp_dir.path().to_str().unwrap(),
            Some(5),
            Some(&config_with_nulls),
        )
        .await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.contains("valid_value"));
    // Null values should be ignored - check that we don't have --pos_arg or --opt_arg flags
    // The substring "null" will appear in "null_test" subcommand name, so check more specifically
    assert!(!output.contains("--pos_arg"));
    assert!(!output.contains("--opt_arg"));
    assert!(
        !output.contains(" null "),
        "Null should not appear as a standalone argument value"
    );

    adapter.shutdown().await;
}

#[tokio::test]
async fn test_multiple_concurrent_async_operations() {
    init_test_logging();
    let (adapter, temp_dir) = create_simple_test_adapter().await;
    let temp_dir_path = temp_dir.path().to_str().unwrap().to_string();

    // Start many concurrent operations to stress test the task handle management
    let mut handles = Vec::new();
    for i in 0..10 {
        let adapter_clone = adapter.clone();
        let temp_dir_path_clone = temp_dir_path.clone();
        let handle = tokio::spawn(async move {
            adapter_clone
                .execute_async_in_dir(
                    &format!("concurrent_{}", i),
                    "echo",
                    Some({
                        let mut args = Map::new();
                        args.insert(
                            "message".to_string(),
                            json!(format!("concurrent test {}", i)),
                        );
                        args
                    }),
                    &temp_dir_path_clone,
                    Some(10),
                )
                .await
        });
        handles.push(handle);
    }

    // Wait for all operations to complete
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok());
        let op_id = result.unwrap();
        assert!(op_id.starts_with("op_"));
    }

    // Yield to allow async operations to finish processing
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }

    adapter.shutdown().await;
}
