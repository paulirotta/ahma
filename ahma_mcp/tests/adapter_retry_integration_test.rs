//! TDD tests for Adapter retry integration
//!
//! These tests verify that the Adapter can be configured with retry logic
//! and that transient errors are retried appropriately.

use ahma_mcp::operation_monitor::{MonitorConfig, OperationMonitor};
use ahma_mcp::retry::RetryConfig;
use ahma_mcp::sandbox::Sandbox;
use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Helper to create a test adapter with retry configuration and custom root
fn create_test_adapter_with_retry_and_root(
    retry_config: Option<RetryConfig>,
    root: PathBuf,
) -> ahma_mcp::adapter::Adapter {
    let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
        Duration::from_secs(30),
    )));
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));

    // Create a sandbox with the specific root as a scope + /tmp for tests
    let scopes = vec![root.clone(), std::env::temp_dir()];
    let sandbox =
        Arc::new(Sandbox::new(scopes, ahma_mcp::sandbox::SandboxMode::Test, false, false).unwrap());

    let mut adapter = ahma_mcp::adapter::Adapter::new(monitor, shell_pool, sandbox)
        .expect("Failed to create adapter");

    if let Some(config) = retry_config {
        adapter = adapter.with_retry_config(config);
    }
    adapter
}

/// Helper to create a test adapter with retry configuration
fn create_test_adapter_with_retry(retry_config: Option<RetryConfig>) -> ahma_mcp::adapter::Adapter {
    let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
        Duration::from_secs(30),
    )));
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let sandbox = Arc::new(Sandbox::new_test());

    let mut adapter = ahma_mcp::adapter::Adapter::new(monitor, shell_pool, sandbox)
        .expect("Failed to create adapter");

    if let Some(config) = retry_config {
        adapter = adapter.with_retry_config(config);
    }
    adapter
}

/// Helper to create a test adapter without retry
fn create_test_adapter() -> ahma_mcp::adapter::Adapter {
    create_test_adapter_with_retry(None)
}

#[tokio::test]
async fn test_adapter_has_retry_config_builder() {
    // Test that Adapter has with_retry_config builder method
    let config = RetryConfig::new().with_max_retries(5);
    let adapter = create_test_adapter_with_retry(Some(config));

    // Verify the adapter was created successfully with retry config
    assert!(adapter.retry_config().is_some());
    assert_eq!(adapter.retry_config().unwrap().max_retries, 5);
}

#[tokio::test]
async fn test_adapter_without_retry_config_has_none() {
    // Test that Adapter without retry config returns None
    let adapter = create_test_adapter();
    assert!(adapter.retry_config().is_none());
}

#[tokio::test]
async fn test_adapter_retry_config_default_values() {
    // Test that default RetryConfig has sensible values
    let config = RetryConfig::default();
    let adapter = create_test_adapter_with_retry(Some(config));

    let retry = adapter.retry_config().unwrap();
    assert_eq!(retry.max_retries, 3);
    assert_eq!(retry.initial_delay, Duration::from_millis(100));
    assert_eq!(retry.max_delay, Duration::from_secs(5));
    assert_eq!(retry.backoff_factor, 2.0);
}

#[tokio::test]
async fn test_execute_sync_with_retry_succeeds_on_first_try() {
    // Test that successful commands don't trigger retries
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let working_dir = temp_dir.path().to_path_buf();

    let config = RetryConfig::new().with_max_retries(3);
    let adapter = create_test_adapter_with_retry_and_root(Some(config), working_dir.clone());

    // Use the retry-aware execution method
    let result = adapter
        .execute_sync_with_retry("echo", None, &working_dir.to_string_lossy(), None, None)
        .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_execute_sync_with_retry_fails_on_permanent_error() {
    // Test that permanent errors (command not found) fail immediately without retry
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let working_dir = temp_dir.path().to_path_buf();

    let config = RetryConfig::new().with_max_retries(3);
    let adapter = create_test_adapter_with_retry_and_root(Some(config), working_dir.clone());

    let result = adapter
        .execute_sync_with_retry(
            "nonexistent_command_xyz",
            None,
            &working_dir.to_string_lossy(),
            None,
            None,
        )
        .await;

    // Should fail immediately without retrying
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string().to_lowercase();
    assert!(
        err_msg.contains("not found") || err_msg.contains("no such file"),
        "Expected 'not found' error, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_execute_sync_without_retry_config_works_normally() {
    // Test that adapter without retry config still executes normally
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let working_dir = temp_dir.path().to_path_buf();

    let adapter = create_test_adapter_with_retry_and_root(None, working_dir.clone());

    // Should fall back to normal execution when no retry config
    let result = adapter
        .execute_sync_with_retry("echo", None, &working_dir.to_string_lossy(), None, None)
        .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_retry_disabled_when_max_retries_zero() {
    // Test that setting max_retries to 0 effectively disables retry
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let working_dir = temp_dir.path().to_path_buf();

    let config = RetryConfig::new().with_max_retries(0);
    let adapter = create_test_adapter_with_retry_and_root(Some(config), working_dir.clone());

    // Should execute without any retries
    let result = adapter
        .execute_sync_with_retry("echo", None, &working_dir.to_string_lossy(), None, None)
        .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_adapter_retry_config_is_cloneable() {
    // Test that the adapter's retry config can be inspected and is Clone
    let config = RetryConfig::new()
        .with_max_retries(5)
        .with_initial_delay(Duration::from_millis(50))
        .with_jitter(true);

    let adapter = create_test_adapter_with_retry(Some(config));

    let retry = adapter.retry_config().cloned().unwrap();
    assert_eq!(retry.max_retries, 5);
    assert_eq!(retry.initial_delay, Duration::from_millis(50));
    assert!(retry.jitter_enabled);
}

#[tokio::test]
async fn test_execute_sync_preserves_output_on_success() {
    // Test that successful output is preserved through retry wrapper
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let working_dir = temp_dir.path().to_path_buf();

    let config = RetryConfig::new().with_max_retries(3);
    let adapter = create_test_adapter_with_retry_and_root(Some(config), working_dir.clone());

    let result = adapter
        .execute_sync_with_retry(
            "echo hello",
            None,
            &working_dir.to_string_lossy(),
            None,
            None,
        )
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap().contains("hello"));
}
