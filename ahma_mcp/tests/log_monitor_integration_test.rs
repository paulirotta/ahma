//! Integration tests for the live log monitoring feature.

use ahma_mcp::adapter::{Adapter, AsyncExecOptions};
use ahma_mcp::callback_system::{CallbackError, CallbackSender, ProgressUpdate};
use ahma_mcp::log_monitor::{LogLevel, LogMonitorConfig, MonitorStream};
use ahma_mcp::operation_monitor::{MonitorConfig, OperationMonitor};
use ahma_mcp::sandbox::Sandbox;
use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};
use ahma_mcp::test_utils::concurrency::wait_for_condition;
use async_trait::async_trait;
use serde_json::Map;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::sync::Mutex;

#[derive(Clone)]
struct TestCallback {
    updates: Arc<Mutex<Vec<ProgressUpdate>>>,
}

impl TestCallback {
    fn new() -> (Self, Arc<Mutex<Vec<ProgressUpdate>>>) {
        let updates = Arc::new(Mutex::new(Vec::new()));
        let cb = Self {
            updates: updates.clone(),
        };
        (cb, updates)
    }
}

#[async_trait]
impl CallbackSender for TestCallback {
    async fn send_progress(&self, update: ProgressUpdate) -> Result<(), CallbackError> {
        self.updates.lock().await.push(update);
        Ok(())
    }
    async fn should_cancel(&self) -> bool {
        false
    }
}

async fn create_test_adapter() -> Adapter {
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(30));
    let monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_pool_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_pool_config));
    let sandbox = Arc::new(Sandbox::new_test());
    Adapter::new(monitor, shell_pool, sandbox).unwrap()
}

fn monitor_config(level: LogLevel, stream: MonitorStream) -> Option<LogMonitorConfig> {
    Some(LogMonitorConfig {
        monitor_level: level,
        monitor_stream: stream,
        rate_limit_seconds: 0,
    })
}

async fn wait_for_completion(updates: &Arc<Mutex<Vec<ProgressUpdate>>>) {
    let completed = wait_for_condition(Duration::from_secs(10), Duration::from_millis(50), || {
        let u = updates.clone();
        async move {
            let guard = u.lock().await;
            guard
                .iter()
                .any(|up| matches!(up, ProgressUpdate::FinalResult { .. }))
        }
    })
    .await;
    assert!(completed, "Timed out waiting for operation to complete");
}

fn count_log_alerts(updates: &[ProgressUpdate]) -> usize {
    updates
        .iter()
        .filter(|up| matches!(up, ProgressUpdate::LogAlert { .. }))
        .count()
}

fn write_script(temp_dir: &std::path::Path, name: &str, content: &str) -> String {
    let script_path = temp_dir.join(name);
    std::fs::write(&script_path, content).unwrap();
    #[cfg(windows)]
    {
        let path_str = script_path.to_string_lossy().replace("\\", "/");
        format!("bash {}", path_str)
    }
    #[cfg(not(windows))]
    {
        format!("bash {}", script_path.display())
    }
}

fn final_result_output(updates: &[ProgressUpdate]) -> Option<String> {
    updates.iter().find_map(|update| {
        if let ProgressUpdate::FinalResult { full_output, .. } = update {
            Some(full_output.clone())
        } else {
            None
        }
    })
}

#[tokio::test]
async fn streaming_stderr_error_triggers_log_alert() {
    let adapter = create_test_adapter().await;
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path().to_str().unwrap();
    let (callback, updates) = TestCallback::new();
    let cmd = write_script(
        temp_dir.path(),
        "emit_error.sh",
        "#!/bin/bash\necho 'error[E0308]: mismatched types' >&2\n",
    );
    let result = adapter
        .execute_async_in_dir_with_options(
            "test_stderr_error",
            &cmd,
            working_dir,
            AsyncExecOptions {
                id: Some("test_op_1".to_string()),
                args: Some(Map::new()),
                timeout: Some(10),
                callback: Some(Box::new(callback)),
                subcommand_config: None,
                log_monitor_config: monitor_config(LogLevel::Error, MonitorStream::Stderr),
            },
        )
        .await;
    assert!(result.is_ok());
    wait_for_completion(&updates).await;
    let guard = updates.lock().await;
    let alerts: Vec<_> = guard
        .iter()
        .filter(|up| matches!(up, ProgressUpdate::LogAlert { .. }))
        .collect();
    assert!(!alerts.is_empty(), "Expected LogAlert, got: {:?}", *guard);
    if let ProgressUpdate::LogAlert {
        trigger_level,
        context_snapshot,
        ..
    } = &alerts[0]
    {
        assert_eq!(trigger_level, "error");
        assert!(
            context_snapshot.contains("mismatched types"),
            "snapshot: {}",
            context_snapshot
        );
    }
}

#[tokio::test]
async fn streaming_stdout_error_triggers_when_monitoring_both() {
    let adapter = create_test_adapter().await;
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path().to_str().unwrap();
    let (callback, updates) = TestCallback::new();
    let cmd = write_script(
        temp_dir.path(),
        "emit_stdout_error.sh",
        "#!/bin/bash\necho 'ERROR: something failed'\n",
    );
    let result = adapter
        .execute_async_in_dir_with_options(
            "test_stdout_error",
            &cmd,
            working_dir,
            AsyncExecOptions {
                id: Some("test_op_2".to_string()),
                args: Some(Map::new()),
                timeout: Some(10),
                callback: Some(Box::new(callback)),
                subcommand_config: None,
                log_monitor_config: monitor_config(LogLevel::Error, MonitorStream::Both),
            },
        )
        .await;
    assert!(result.is_ok());
    wait_for_completion(&updates).await;
    let guard = updates.lock().await;
    assert!(
        count_log_alerts(&guard) > 0,
        "Expected LogAlert: {:?}",
        *guard
    );
}

#[tokio::test]
async fn streaming_no_alert_when_output_is_clean() {
    let adapter = create_test_adapter().await;
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path().to_str().unwrap();
    let (callback, updates) = TestCallback::new();
    let cmd = write_script(
        temp_dir.path(),
        "clean.sh",
        "#!/bin/bash\necho 'hello world'\n",
    );
    let result = adapter
        .execute_async_in_dir_with_options(
            "test_clean",
            &cmd,
            working_dir,
            AsyncExecOptions {
                id: Some("test_op_3".to_string()),
                args: Some(Map::new()),
                timeout: Some(10),
                callback: Some(Box::new(callback)),
                subcommand_config: None,
                log_monitor_config: monitor_config(LogLevel::Error, MonitorStream::Both),
            },
        )
        .await;
    assert!(result.is_ok());
    wait_for_completion(&updates).await;
    let guard = updates.lock().await;
    assert_eq!(
        count_log_alerts(&guard),
        0,
        "Clean output should not trigger: {:?}",
        *guard
    );
}

#[tokio::test]
async fn streaming_warn_level_triggers_on_warning() {
    let adapter = create_test_adapter().await;
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path().to_str().unwrap();
    let (callback, updates) = TestCallback::new();
    let cmd = write_script(
        temp_dir.path(),
        "emit_warn.sh",
        "#!/bin/bash\necho 'warning: unused variable' >&2\n",
    );
    let result = adapter
        .execute_async_in_dir_with_options(
            "test_warn",
            &cmd,
            working_dir,
            AsyncExecOptions {
                id: Some("test_op_4".to_string()),
                args: Some(Map::new()),
                timeout: Some(10),
                callback: Some(Box::new(callback)),
                subcommand_config: None,
                log_monitor_config: monitor_config(LogLevel::Warn, MonitorStream::Stderr),
            },
        )
        .await;
    assert!(result.is_ok());
    wait_for_completion(&updates).await;
    let guard = updates.lock().await;
    let alerts: Vec<_> = guard
        .iter()
        .filter(|up| matches!(up, ProgressUpdate::LogAlert { .. }))
        .collect();
    assert!(
        !alerts.is_empty(),
        "Expected LogAlert for warning: {:?}",
        *guard
    );
    if let ProgressUpdate::LogAlert { trigger_level, .. } = &alerts[0] {
        assert_eq!(trigger_level, "warn");
    }
}

#[tokio::test]
async fn streaming_error_level_ignores_warnings() {
    let adapter = create_test_adapter().await;
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path().to_str().unwrap();
    let (callback, updates) = TestCallback::new();
    let cmd = write_script(
        temp_dir.path(),
        "warn_only.sh",
        "#!/bin/bash\necho 'warning: unused variable' >&2\n",
    );
    let result = adapter
        .execute_async_in_dir_with_options(
            "test_warn_at_error",
            &cmd,
            working_dir,
            AsyncExecOptions {
                id: Some("test_op_5".to_string()),
                args: Some(Map::new()),
                timeout: Some(10),
                callback: Some(Box::new(callback)),
                subcommand_config: None,
                log_monitor_config: monitor_config(LogLevel::Error, MonitorStream::Stderr),
            },
        )
        .await;
    assert!(result.is_ok());
    wait_for_completion(&updates).await;
    let guard = updates.lock().await;
    assert_eq!(
        count_log_alerts(&guard),
        0,
        "Warning should not trigger at Error level: {:?}",
        *guard
    );
}

#[tokio::test]
async fn streaming_no_monitor_uses_batch_path() {
    let adapter = create_test_adapter().await;
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path().to_str().unwrap();
    let (callback, updates) = TestCallback::new();
    let cmd = write_script(
        temp_dir.path(),
        "batch.sh",
        "#!/bin/bash\necho 'error: something' >&2\necho done\n",
    );
    let result = adapter
        .execute_async_in_dir_with_options(
            "test_batch",
            &cmd,
            working_dir,
            AsyncExecOptions {
                id: Some("test_op_6".to_string()),
                args: Some(Map::new()),
                timeout: Some(10),
                callback: Some(Box::new(callback)),
                subcommand_config: None,
                log_monitor_config: None,
            },
        )
        .await;
    assert!(result.is_ok());
    wait_for_completion(&updates).await;
    let guard = updates.lock().await;
    assert_eq!(
        count_log_alerts(&guard),
        0,
        "Batch path should not produce LogAlert: {:?}",
        *guard
    );
    assert!(
        guard
            .iter()
            .any(|up| matches!(up, ProgressUpdate::FinalResult { .. }))
    );
}

#[tokio::test]
async fn streaming_alert_includes_context_lines() {
    let adapter = create_test_adapter().await;
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path().to_str().unwrap();
    let (callback, updates) = TestCallback::new();
    let script = "#!/bin/bash\nfor i in $(seq 1 5); do echo \"info: compiling module $i\" >&2; done\necho 'error[E0277]: the trait bound is not satisfied' >&2\n";
    let cmd = write_script(temp_dir.path(), "context.sh", script);
    let result = adapter
        .execute_async_in_dir_with_options(
            "test_context",
            &cmd,
            working_dir,
            AsyncExecOptions {
                id: Some("test_op_7".to_string()),
                args: Some(Map::new()),
                timeout: Some(10),
                callback: Some(Box::new(callback)),
                subcommand_config: None,
                log_monitor_config: monitor_config(LogLevel::Error, MonitorStream::Stderr),
            },
        )
        .await;
    assert!(result.is_ok());
    wait_for_completion(&updates).await;
    let guard = updates.lock().await;
    let alerts: Vec<_> = guard
        .iter()
        .filter(|up| matches!(up, ProgressUpdate::LogAlert { .. }))
        .collect();
    assert!(!alerts.is_empty(), "Expected LogAlert: {:?}", *guard);
    if let ProgressUpdate::LogAlert {
        context_snapshot, ..
    } = &alerts[0]
    {
        assert!(
            context_snapshot.contains("compiling module"),
            "Missing context: {}",
            context_snapshot
        );
        assert!(
            context_snapshot.contains("E0277"),
            "Missing trigger: {}",
            context_snapshot
        );
    }
}

#[tokio::test]
async fn streaming_multiline_errors_with_rate_limit() {
    let adapter = create_test_adapter().await;
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path().to_str().unwrap();
    let (callback, updates) = TestCallback::new();
    let script = "#!/bin/bash\necho 'error[E0308]: mismatched types' >&2\necho 'error[E0277]: trait bound' >&2\necho 'error[E0599]: no method' >&2\n";
    let cmd = write_script(temp_dir.path(), "multi_error.sh", script);
    let result = adapter
        .execute_async_in_dir_with_options(
            "test_rate_limit",
            &cmd,
            working_dir,
            AsyncExecOptions {
                id: Some("test_op_8".to_string()),
                args: Some(Map::new()),
                timeout: Some(10),
                callback: Some(Box::new(callback)),
                subcommand_config: None,
                log_monitor_config: Some(LogMonitorConfig {
                    monitor_level: LogLevel::Error,
                    monitor_stream: MonitorStream::Stderr,
                    rate_limit_seconds: 60,
                }),
            },
        )
        .await;
    assert!(result.is_ok());
    wait_for_completion(&updates).await;
    let guard = updates.lock().await;
    assert_eq!(
        count_log_alerts(&guard),
        1,
        "Rate limit should suppress: {:?}",
        *guard
    );
}

#[tokio::test]
async fn streaming_stderr_only_ignores_stdout_patterns() {
    let adapter = create_test_adapter().await;
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path().to_str().unwrap();
    let (callback, updates) = TestCallback::new();
    let cmd = write_script(
        temp_dir.path(),
        "stdout_error.sh",
        "#!/bin/bash\necho 'error[E0308]: type mismatch'\n",
    );
    let result = adapter
        .execute_async_in_dir_with_options(
            "test_stderr_filter",
            &cmd,
            working_dir,
            AsyncExecOptions {
                id: Some("test_op_9".to_string()),
                args: Some(Map::new()),
                timeout: Some(10),
                callback: Some(Box::new(callback)),
                subcommand_config: None,
                log_monitor_config: monitor_config(LogLevel::Error, MonitorStream::Stderr),
            },
        )
        .await;
    assert!(result.is_ok());
    wait_for_completion(&updates).await;
    let guard = updates.lock().await;
    assert_eq!(
        count_log_alerts(&guard),
        0,
        "Stderr-only should ignore stdout: {:?}",
        *guard
    );
}

#[tokio::test]
async fn streaming_final_result_redacts_sensitive_output() {
    let adapter = create_test_adapter().await;
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path().to_str().unwrap();
    let (callback, updates) = TestCallback::new();
    let cmd = write_script(
        temp_dir.path(),
        "secret_output.sh",
        "#!/bin/bash\necho 'token=supersecret123'\necho 'Authorization: Bearer abcdefghijklmnop' >&2\n",
    );

    let result = adapter
        .execute_async_in_dir_with_options(
            "test_redaction",
            &cmd,
            working_dir,
            AsyncExecOptions {
                id: Some("test_op_10".to_string()),
                args: Some(Map::new()),
                timeout: Some(10),
                callback: Some(Box::new(callback)),
                subcommand_config: None,
                log_monitor_config: monitor_config(LogLevel::Error, MonitorStream::Both),
            },
        )
        .await;
    assert!(result.is_ok());

    wait_for_completion(&updates).await;
    let guard = updates.lock().await;
    let output = final_result_output(&guard).expect("missing final result output");
    assert!(!output.contains("supersecret123"), "output: {}", output);
    assert!(!output.contains("abcdefghijklmnop"), "output: {}", output);
    assert!(output.contains("[REDACTED]"), "output: {}", output);
}

#[tokio::test]
async fn streaming_final_result_is_bounded_and_marks_truncation() {
    let adapter = create_test_adapter().await;
    let temp_dir = tempdir().unwrap();
    let working_dir = temp_dir.path().to_str().unwrap();
    let (callback, updates) = TestCallback::new();
    let cmd = write_script(
        temp_dir.path(),
        "many_lines.sh",
        "#!/bin/bash\nfor i in $(seq 1 7000); do echo \"line-$i\"; done\n",
    );

    let result = adapter
        .execute_async_in_dir_with_options(
            "test_truncation",
            &cmd,
            working_dir,
            AsyncExecOptions {
                id: Some("test_op_11".to_string()),
                args: Some(Map::new()),
                timeout: Some(20),
                callback: Some(Box::new(callback)),
                subcommand_config: None,
                log_monitor_config: monitor_config(LogLevel::Error, MonitorStream::Both),
            },
        )
        .await;
    assert!(result.is_ok());

    wait_for_completion(&updates).await;
    let guard = updates.lock().await;
    let output = final_result_output(&guard).expect("missing final result output");
    assert!(
        output.contains("[output truncated: dropped"),
        "output: {}",
        output
    );
    assert!(!output.contains("line-1"), "oldest lines should be dropped");
    assert!(
        output.contains("line-7000"),
        "latest line should be retained"
    );
}
