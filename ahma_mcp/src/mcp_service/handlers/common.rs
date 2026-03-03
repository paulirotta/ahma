use crate::operation_monitor::Operation;
use rmcp::model::Content;
use serde_json::{Map, Value};

/// Parses a comma-separated string value from JSON args into a list of trimmed, non-empty strings.
pub fn parse_comma_separated_filter(args: &Map<String, Value>, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| {
            s.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Serializes operations to Content text entries, logging errors.
pub fn serialize_operations_to_content(operations: &[Operation]) -> Vec<Content> {
    operations
        .iter()
        .filter_map(|op| match serde_json::to_string_pretty(op) {
            Ok(s) => Some(Content::text(s)),
            Err(e) => {
                tracing::error!("Serialization error: {}", e);
                None
            }
        })
        .collect()
}

/// Checks whether an operation matches the given tool name prefixes and optional operation ID.
pub fn operation_matches_filters(
    op: &Operation,
    tool_filters: &[String],
    id: Option<&str>,
) -> bool {
    let matches_filter =
        tool_filters.is_empty() || tool_filters.iter().any(|tn| op.tool_name.starts_with(tn));
    let matches_id = id.is_none_or(|id| op.id == id);
    matches_filter && matches_id
}

pub fn parse_tool_filters(args: &Map<String, Value>) -> Vec<String> {
    parse_comma_separated_filter(args, "tools")
}

pub fn parse_id(args: &Map<String, Value>) -> Option<String> {
    args.get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Attempts to wait for an async operation to complete within the automatic async
/// timeout window. If the operation finishes in time, returns a `CallToolResult` with
/// the output inline. Otherwise returns `None` to signal normal async behavior.
///
/// This reduces context chatter for fast commands by eliminating the need for an
/// extra `await` round-trip.
pub async fn try_automatic_async_completion(
    monitor: &crate::operation_monitor::OperationMonitor,
    op_id: &str,
) -> Option<rmcp::model::CallToolResult> {
    use crate::constants::AUTOMATIC_ASYNC_TIMEOUT_SECS;
    use std::time::Duration;

    // First check if already completed (race: task finished before we got here)
    if let Some(op) = monitor.check_completion_history_pub(op_id).await {
        return Some(format_completed_operation(&op));
    }

    // Get the completion notifier for this operation
    let notifier = match monitor.get_notifier_or_terminal_pub(op_id).await {
        Err(terminal_op) => return Some(format_completed_operation(&terminal_op)),
        Ok(None) => return None, // Operation not found
        Ok(Some(n)) => n,
    };

    // Wait up to AUTOMATIC_ASYNC_TIMEOUT_SECS for completion
    let timeout = Duration::from_secs(AUTOMATIC_ASYNC_TIMEOUT_SECS);
    match tokio::time::timeout(timeout, notifier.notified()).await {
        Ok(_) => {
            // Operation completed — retrieve from history
            monitor
                .wait_for_history_propagation_pub(op_id)
                .await
                .map(|op| format_completed_operation(&op))
        }
        Err(_) => {
            // Timeout elapsed — fall back to normal async behavior
            tracing::debug!(
                "Automatic async timeout ({}s) elapsed for {}, returning async ID",
                AUTOMATIC_ASYNC_TIMEOUT_SECS,
                op_id
            );
            None
        }
    }
}

/// Formats a completed operation into a `CallToolResult`.
fn format_completed_operation(op: &Operation) -> rmcp::model::CallToolResult {
    use crate::operation_monitor::OperationStatus;
    use rmcp::model::{CallToolResult, Content};

    match op.state {
        OperationStatus::Completed => {
            let output = extract_output_from_result(&op.result);
            CallToolResult::success(vec![Content::text(output)])
        }
        OperationStatus::Failed => {
            let output = extract_output_from_result(&op.result);
            // Return as success with error content (same as sync path behavior)
            CallToolResult::success(vec![Content::text(format!("Command failed: {}", output))])
        }
        OperationStatus::Cancelled | OperationStatus::TimedOut => {
            let reason = op
                .result
                .as_ref()
                .and_then(|v| v.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("Operation was cancelled or timed out");
            CallToolResult::success(vec![Content::text(reason.to_string())])
        }
        _ => CallToolResult::success(vec![Content::text("Operation completed".to_string())]),
    }
}

/// Extracts human-readable output from an operation result JSON value.
fn extract_output_from_result(result: &Option<Value>) -> String {
    let Some(val) = result else {
        return String::new();
    };

    // Handle structured result with stdout/stderr/exit_code
    if let (Some(stdout), Some(stderr)) = (val.get("stdout"), val.get("stderr")) {
        let stdout_str = stdout.as_str().unwrap_or_default();
        let stderr_str = stderr.as_str().unwrap_or_default();
        let exit_code = val.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);

        if exit_code == 0 {
            if stdout_str.is_empty() && !stderr_str.is_empty() {
                return stderr_str.to_string();
            } else if !stdout_str.is_empty() && !stderr_str.is_empty() {
                return format!("{}\n{}", stdout_str, stderr_str);
            } else {
                return stdout_str.to_string();
            }
        } else {
            return format!(
                "Exit code: {}\nStdout:\n{}\nStderr:\n{}",
                exit_code, stdout_str, stderr_str
            );
        }
    }

    // Handle string result (error message)
    if let Some(s) = val.as_str() {
        return s.to_string();
    }

    // Fallback: serialize the value
    serde_json::to_string_pretty(val).unwrap_or_default()
}
