use crate::operation_monitor::Operation;
use rmcp::model::{CallToolResult, Content, ErrorData as McpError};
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

/// Returns a successful MCP result with a single text content block.
pub fn text_result(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text.into())])
}

/// Builds an internal MCP error with no extra data payload.
pub fn mcp_internal(message: impl Into<String>) -> McpError {
    McpError::internal_error(message.into(), None)
}

/// Builds an invalid-params MCP error with no extra data payload.
pub fn mcp_invalid_params(message: impl Into<String>) -> McpError {
    McpError::invalid_params(message.into(), None)
}

/// Reads an optional string argument from JSON args.
pub fn opt_str(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

/// Reads a required string argument from JSON args.
pub fn require_str(
    args: &Map<String, Value>,
    key: &str,
    error_message: &str,
) -> Result<String, McpError> {
    opt_str(args, key).ok_or_else(|| mcp_invalid_params(error_message))
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

    match op.state {
        OperationStatus::Completed => {
            let output = extract_output_from_result(&op.result);
            text_result(output)
        }
        OperationStatus::Failed => {
            let output = extract_output_from_result(&op.result);
            // Return as success with error content (same as sync path behavior)
            text_result(format!("Command failed: {}", output))
        }
        OperationStatus::Cancelled | OperationStatus::TimedOut => {
            let reason = op
                .result
                .as_ref()
                .and_then(|v| v.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("Operation was cancelled or timed out");
            text_result(reason.to_string())
        }
        _ => text_result("Operation completed"),
    }
}

fn format_structured_output(stdout_str: &str, stderr_str: &str, exit_code: i64) -> String {
    if exit_code != 0 {
        return format!(
            "Exit code: {}\nStdout:\n{}\nStderr:\n{}",
            exit_code, stdout_str, stderr_str
        );
    }

    match (stdout_str.is_empty(), stderr_str.is_empty()) {
        (true, false) => stderr_str.to_string(),
        (false, false) => format!("{}\n{}", stdout_str, stderr_str),
        _ => stdout_str.to_string(),
    }
}

/// Extracts human-readable output from an operation result JSON value.
fn extract_output_from_result(result: &Option<Value>) -> String {
    let Some(val) = result else {
        return String::new();
    };

    if let (Some(stdout), Some(stderr)) = (val.get("stdout"), val.get("stderr")) {
        let stdout_str = stdout.as_str().unwrap_or_default();
        let stderr_str = stderr.as_str().unwrap_or_default();
        let exit_code = val.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
        return format_structured_output(stdout_str, stderr_str, exit_code);
    }

    val.as_str()
        .map(String::from)
        .unwrap_or_else(|| serde_json::to_string_pretty(val).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation_monitor::{Operation, OperationStatus};
    use serde_json::json;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_map(pairs: &[(&str, &str)]) -> Map<String, Value> {
        let mut m = Map::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), json!(*v));
        }
        m
    }

    fn make_op(id: &str, tool: &str, status: OperationStatus) -> Operation {
        let mut op = Operation::new(id.to_string(), tool.to_string(), String::new(), None);
        op.state = status;
        op
    }

    // ── parse_comma_separated_filter ─────────────────────────────────────────

    #[test]
    fn test_parse_comma_separated_filter_basic() {
        let args = make_map(&[("tools", "cargo,clippy,nextest")]);
        let result = parse_comma_separated_filter(&args, "tools");
        assert_eq!(result, vec!["cargo", "clippy", "nextest"]);
    }

    #[test]
    fn test_parse_comma_separated_filter_trims_whitespace() {
        let args = make_map(&[("tools", "  cargo , clippy , nextest  ")]);
        let result = parse_comma_separated_filter(&args, "tools");
        assert_eq!(result, vec!["cargo", "clippy", "nextest"]);
    }

    #[test]
    fn test_parse_comma_separated_filter_filters_empty_segments() {
        let args = make_map(&[("tools", "cargo,,nextest,")]);
        let result = parse_comma_separated_filter(&args, "tools");
        assert_eq!(result, vec!["cargo", "nextest"]);
    }

    #[test]
    fn test_parse_comma_separated_filter_missing_key() {
        let args = make_map(&[]);
        let result = parse_comma_separated_filter(&args, "tools");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_comma_separated_filter_single_value() {
        let args = make_map(&[("tools", "cargo")]);
        let result = parse_comma_separated_filter(&args, "tools");
        assert_eq!(result, vec!["cargo"]);
    }

    #[test]
    fn test_parse_comma_separated_filter_only_commas() {
        let args = make_map(&[("tools", ",,,")]);
        let result = parse_comma_separated_filter(&args, "tools");
        assert!(result.is_empty());
    }

    // ── parse_tool_filters ───────────────────────────────────────────────────

    #[test]
    fn test_parse_tool_filters_delegates_to_tools_key() {
        let args = make_map(&[("tools", "cargo,clippy")]);
        let result = parse_tool_filters(&args);
        assert_eq!(result, vec!["cargo", "clippy"]);
    }

    #[test]
    fn test_parse_tool_filters_empty_args() {
        let args = make_map(&[]);
        assert!(parse_tool_filters(&args).is_empty());
    }

    // ── parse_id ─────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_id_present() {
        let args = make_map(&[("id", "op-1234")]);
        assert_eq!(parse_id(&args), Some("op-1234".to_string()));
    }

    #[test]
    fn test_parse_id_absent() {
        let args = make_map(&[]);
        assert_eq!(parse_id(&args), None);
    }

    // ── operation_matches_filters ────────────────────────────────────────────

    #[test]
    fn test_operation_matches_filters_no_filters_no_id() {
        let op = make_op("op-1", "cargo_build", OperationStatus::Completed);
        // Empty filters + no id → everything matches
        assert!(operation_matches_filters(&op, &[], None));
    }

    #[test]
    fn test_operation_matches_filters_matching_tool_prefix() {
        let op = make_op("op-1", "cargo_build", OperationStatus::Completed);
        let filters = vec!["cargo".to_string()];
        assert!(operation_matches_filters(&op, &filters, None));
    }

    #[test]
    fn test_operation_matches_filters_non_matching_tool_prefix() {
        let op = make_op("op-1", "cargo_build", OperationStatus::Completed);
        let filters = vec!["npm".to_string()];
        assert!(!operation_matches_filters(&op, &filters, None));
    }

    #[test]
    fn test_operation_matches_filters_matching_id() {
        let op = make_op("op-42", "cargo_build", OperationStatus::Completed);
        assert!(operation_matches_filters(&op, &[], Some("op-42")));
    }

    #[test]
    fn test_operation_matches_filters_non_matching_id() {
        let op = make_op("op-42", "cargo_build", OperationStatus::Completed);
        assert!(!operation_matches_filters(&op, &[], Some("op-99")));
    }

    #[test]
    fn test_operation_matches_filters_tool_and_id_both_match() {
        let op = make_op("op-42", "cargo_build", OperationStatus::Completed);
        let filters = vec!["cargo".to_string()];
        assert!(operation_matches_filters(&op, &filters, Some("op-42")));
    }

    #[test]
    fn test_operation_matches_filters_tool_matches_but_id_mismatch() {
        let op = make_op("op-42", "cargo_build", OperationStatus::Completed);
        let filters = vec!["cargo".to_string()];
        assert!(!operation_matches_filters(&op, &filters, Some("op-99")));
    }

    // ── serialize_operations_to_content ──────────────────────────────────────

    #[test]
    fn test_serialize_operations_to_content_empty() {
        let ops: Vec<Operation> = vec![];
        let result = serialize_operations_to_content(&ops);
        assert!(result.is_empty());
    }

    #[test]
    fn test_serialize_operations_to_content_single_op() {
        let op = make_op("op-1", "cargo_build", OperationStatus::Completed);
        let result = serialize_operations_to_content(&[op]);
        assert_eq!(result.len(), 1);
        // The content should be valid JSON containing op id
        let text = result[0].as_text().map(|t| t.text.as_str()).unwrap_or("");
        assert!(
            text.contains("op-1"),
            "Serialized content should contain op id: {text}"
        );
    }

    #[test]
    fn test_serialize_operations_to_content_multiple_ops() {
        let ops = vec![
            make_op("op-1", "cargo_build", OperationStatus::Completed),
            make_op("op-2", "cargo_test", OperationStatus::Failed),
        ];
        let result = serialize_operations_to_content(&ops);
        assert_eq!(result.len(), 2);
    }

    // ── extract_output_from_result (private, tested via inline) ──────────────

    #[test]
    fn test_extract_output_none_result() {
        assert_eq!(extract_output_from_result(&None), "");
    }

    #[test]
    fn test_extract_output_string_result() {
        let result = Some(json!("error: compilation failed"));
        assert_eq!(
            extract_output_from_result(&result),
            "error: compilation failed"
        );
    }

    #[test]
    fn test_extract_output_stdout_only_exit_zero() {
        let result = Some(json!({
            "stdout": "hello world",
            "stderr": "",
            "exit_code": 0
        }));
        assert_eq!(extract_output_from_result(&result), "hello world");
    }

    #[test]
    fn test_extract_output_stderr_only_exit_zero() {
        let result = Some(json!({
            "stdout": "",
            "stderr": "warning: unused variable",
            "exit_code": 0
        }));
        // When only stderr, return stderr
        assert_eq!(
            extract_output_from_result(&result),
            "warning: unused variable"
        );
    }

    #[test]
    fn test_extract_output_both_stdout_and_stderr_exit_zero() {
        let result = Some(json!({
            "stdout": "output",
            "stderr": "warning",
            "exit_code": 0
        }));
        let out = extract_output_from_result(&result);
        assert!(out.contains("output"));
        assert!(out.contains("warning"));
    }

    #[test]
    fn test_extract_output_nonzero_exit_code() {
        let result = Some(json!({
            "stdout": "some output",
            "stderr": "error text",
            "exit_code": 1
        }));
        let out = extract_output_from_result(&result);
        assert!(
            out.contains("Exit code: 1"),
            "Non-zero exit should show exit code: {out}"
        );
        assert!(out.contains("some output"));
        assert!(out.contains("error text"));
    }

    #[test]
    fn test_extract_output_arbitrary_json_fallback() {
        let result = Some(json!({"nested": {"key": "value"}}));
        let out = extract_output_from_result(&result);
        // Should be serialized JSON
        assert!(
            out.contains("nested"),
            "Fallback should serialize JSON: {out}"
        );
    }
}
