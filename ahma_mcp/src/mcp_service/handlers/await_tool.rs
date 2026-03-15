use super::common;
use crate::AhmaMcpService;
use crate::operation_monitor::Operation;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content, ErrorData as McpError};
use serde_json::{Map, Value};
use std::sync::Arc;
use tokio::time::Instant;
use tracing;

impl AhmaMcpService {
    /// Generates the specific input schema for the `await` tool.
    pub fn generate_input_schema_for_wait(&self) -> Arc<Map<String, Value>> {
        let mut properties = Map::new();
        properties.insert(
            "tools".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Comma-separated tool name prefixes to await for (optional; waits for all if omitted)"
            }),
        );
        properties.insert(
            "id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Specific operation ID to await for (optional)"
            }),
        );

        let mut schema = Map::new();
        schema.insert("type".to_string(), Value::String("object".to_string()));
        schema.insert("properties".to_string(), Value::Object(properties));
        Arc::new(schema)
    }

    /// Handles the 'await' tool call.
    pub async fn handle_await(
        &self,
        params: CallToolRequestParams,
    ) -> Result<CallToolResult, McpError> {
        let args = params.arguments.unwrap_or_default();

        let id_filter = common::parse_id(&args);
        let tool_filters = common::parse_tool_filters(&args);

        // If id is specified, wait for that specific operation
        if let Some(op_id) = id_filter {
            return self.handle_await_specific_operation(op_id).await;
        }

        // Original behavior: wait for operations by tool filter
        // Always use intelligent timeout calculation (no user-provided timeout parameter)
        let timeout_seconds = self.calculate_intelligent_timeout(&tool_filters).await;
        let timeout_duration = std::time::Duration::from_secs(timeout_seconds as u64);

        let pending_ops: Vec<Operation> = self
            .operation_monitor
            .get_all_active_operations()
            .await
            .into_iter()
            .filter(|op| {
                !op.state.is_terminal()
                    && common::operation_matches_filters(op, &tool_filters, None)
            })
            .collect();

        if pending_ops.is_empty() {
            return self.handle_await_no_pending_ops(&tool_filters).await;
        }

        tracing::info!(
            "Waiting for {} pending operations (timeout: {}s): {:?}",
            pending_ops.len(),
            timeout_seconds,
            pending_ops.iter().map(|op| &op.id).collect::<Vec<_>>()
        );

        let wait_start = Instant::now();
        let (warning_task, mut warning_rx) = spawn_progress_warnings(timeout_seconds);

        let wait_result = tokio::time::timeout(timeout_duration, async {
            let futures: Vec<_> = pending_ops
                .iter()
                .map(|op| self.operation_monitor.wait_for_operation(&op.id))
                .collect();
            let completed: Vec<Operation> = futures::future::join_all(futures)
                .await
                .into_iter()
                .flatten()
                .collect();
            common::serialize_operations_to_content(&completed)
        })
        .await;

        warning_task.abort();
        while let Ok(warning) = warning_rx.try_recv() {
            tracing::info!("Wait progress: {}", warning);
        }

        match wait_result {
            Ok(contents) => Ok(build_completion_result(contents, wait_start)),
            Err(_) => {
                self.handle_await_timeout(wait_start, timeout_seconds, &pending_ops)
                    .await
            }
        }
    }

    async fn handle_await_timeout(
        &self,
        wait_start: Instant,
        timeout_seconds: f64,
        pending_ops: &[Operation],
    ) -> Result<CallToolResult, McpError> {
        let elapsed = wait_start.elapsed();
        let still_running: Vec<Operation> = self
            .operation_monitor
            .get_all_active_operations()
            .await
            .into_iter()
            .filter(|op| !op.state.is_terminal())
            .collect();
        let completed_during_wait = pending_ops.len() - still_running.len();
        let remediation_steps = self.generate_remediation_suggestions(&still_running).await;

        let mut error_message = format!(
            "Wait operation timed out after {:.2}s (configured timeout: {:.0}s).\n\n\
            Progress: {}/{} operations completed during await.\n\
            Still running: {} operations.\n\nSuggestions:",
            elapsed.as_secs_f64(),
            timeout_seconds,
            completed_during_wait,
            pending_ops.len(),
            still_running.len()
        );
        for step in &remediation_steps {
            error_message.push_str(&format!("\n{}", step));
        }
        if !still_running.is_empty() {
            error_message.push_str("\n\nStill running operations:");
            for op in &still_running {
                error_message.push_str(&format!("\n• {} ({})", op.id, op.tool_name));
            }
        }
        Ok(CallToolResult::success(vec![Content::text(error_message)]))
    }

    /// Calculate intelligent timeout based on operation timeouts and default await timeout
    pub async fn calculate_intelligent_timeout(&self, tool_filters: &[String]) -> f64 {
        const DEFAULT_AWAIT_TIMEOUT: f64 = 600.0;

        let pending_ops = self.operation_monitor.get_all_active_operations().await;

        let max_op_timeout = pending_ops
            .iter()
            .filter(|op| {
                tool_filters.is_empty() || tool_filters.iter().any(|f| op.tool_name.starts_with(f))
            })
            .filter_map(|op| op.timeout_duration)
            .map(|t| t.as_secs_f64())
            .fold(0.0, f64::max);

        DEFAULT_AWAIT_TIMEOUT.max(max_op_timeout)
    }

    async fn handle_await_no_pending_ops(
        &self,
        tool_filters: &[String],
    ) -> Result<CallToolResult, McpError> {
        let completed_ops = self.operation_monitor.get_completed_operations().await;
        let relevant_completed: Vec<Operation> = completed_ops
            .into_iter()
            .filter(|op| {
                !tool_filters.is_empty()
                    && tool_filters.iter().any(|tn| op.tool_name.starts_with(tn))
            })
            .collect();

        if !relevant_completed.is_empty() {
            let mut contents = vec![Content::text(format!(
                "No pending operations for tools: {}. However, these operations recently completed:",
                tool_filters.join(", ")
            ))];
            contents.extend(common::serialize_operations_to_content(&relevant_completed));
            return Ok(CallToolResult::success(contents));
        }

        Ok(CallToolResult::success(vec![Content::text(
            if tool_filters.is_empty() {
                "No pending operations to await for.".to_string()
            } else {
                format!(
                    "No pending operations for tools: {}",
                    tool_filters.join(", ")
                )
            },
        )]))
    }

    async fn handle_await_specific_operation(
        &self,
        op_id: String,
    ) -> Result<CallToolResult, McpError> {
        if self.operation_monitor.get_operation(&op_id).await.is_none() {
            return Ok(self.format_already_completed_or_not_found(&op_id).await);
        }

        tracing::info!("Waiting for operation: {}", op_id);
        let timeout_duration = std::time::Duration::from_secs(300);
        let wait_start = Instant::now();

        let wait_result = tokio::time::timeout(
            timeout_duration,
            self.operation_monitor.wait_for_operation(&op_id),
        )
        .await;

        match wait_result {
            Ok(Some(completed_op)) => {
                let contents =
                    common::serialize_operations_to_content(std::slice::from_ref(&completed_op));
                Ok(build_completion_result(contents, wait_start))
            }
            Ok(None) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Operation {} completed but no result available",
                op_id
            ))])),
            Err(_) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Timeout waiting for operation {}",
                op_id
            ))])),
        }
    }

    async fn format_already_completed_or_not_found(&self, op_id: &str) -> CallToolResult {
        let completed_ops = self.operation_monitor.get_completed_operations().await;
        let Some(completed_op) = completed_ops.iter().find(|op| op.id == op_id) else {
            return CallToolResult::success(vec![Content::text(format!(
                "Operation {} not found",
                op_id
            ))]);
        };
        let mut contents = vec![Content::text(format!(
            "Operation {} already completed",
            op_id
        ))];
        contents.extend(common::serialize_operations_to_content(
            std::slice::from_ref(completed_op),
        ));
        CallToolResult::success(contents)
    }

    async fn generate_remediation_suggestions(&self, still_running: &[Operation]) -> Vec<String> {
        let mut steps = Vec::new();
        self.collect_lock_file_suggestions(&mut steps).await;
        collect_process_suggestions(still_running, &mut steps);
        collect_network_suggestions(still_running, &mut steps);
        collect_build_suggestions(still_running, &mut steps);
        if steps.is_empty() {
            steps.push("• Use the 'status' tool to check remaining operations".to_string());
            steps.push(
                "• Operations continue running in background - they may complete shortly"
                    .to_string(),
            );
            steps.push(
                "• Consider increasing timeout_seconds if operations legitimately need more time"
                    .to_string(),
            );
        }
        steps
    }

    async fn collect_lock_file_suggestions(&self, steps: &mut Vec<String>) {
        for dir in &["target", "node_modules", ".cargo", "tmp", "temp"] {
            scan_dir_for_lock_files(dir, steps).await;
        }
        if tokio::fs::metadata(".").await.is_ok() {
            steps.push("• Check available disk space: df -h .".to_string());
        }
    }
}

const LOCK_PATTERNS: &[&str] = &[
    ".cargo-lock",
    ".lock",
    "package-lock.json",
    "yarn.lock",
    ".npm-lock",
    "composer.lock",
    "Pipfile.lock",
    ".bundle-lock",
];

fn is_lock_file(name: &str) -> bool {
    LOCK_PATTERNS.iter().any(|p| name.contains(p))
}

async fn scan_dir_for_lock_files(dir: &str, steps: &mut Vec<String>) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        if is_lock_file(&name) {
            steps.push(format!(
                "• Remove potential stale lock file: rm {}/{}",
                dir, name
            ));
        }
    }
}

fn spawn_progress_warnings(
    timeout_secs: f64,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::UnboundedReceiver<String>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = tokio::spawn(async move {
        for (pct, remaining_factor) in [(50, 0.5), (75, 0.25), (90, 0.1)] {
            tokio::time::sleep(std::time::Duration::from_secs_f64(
                timeout_secs * remaining_factor,
            ))
            .await;
            let _ = tx.send(format!(
                "Wait operation {}% complete ({:.0}s remaining)",
                pct,
                timeout_secs * remaining_factor
            ));
        }
    });
    (handle, rx)
}

fn build_completion_result(contents: Vec<Content>, wait_start: Instant) -> CallToolResult {
    let elapsed = wait_start.elapsed();
    if contents.is_empty() {
        return CallToolResult::success(vec![Content::text(
            "No operations completed within timeout period".to_string(),
        )]);
    }
    let mut result_contents = vec![Content::text(format!(
        "Completed {} operations in {:.2}s",
        contents.len(),
        elapsed.as_secs_f64()
    ))];
    result_contents.extend(contents);
    CallToolResult::success(result_contents)
}

fn collect_process_suggestions(still_running: &[Operation], steps: &mut Vec<String>) {
    let running_commands: std::collections::HashSet<String> = still_running
        .iter()
        .map(|op| {
            op.tool_name
                .split('_')
                .next()
                .unwrap_or(&op.tool_name)
                .to_string()
        })
        .collect();
    for cmd in &running_commands {
        steps.push(format!(
            "• Check for competing {} processes: ps aux | grep {}",
            cmd, cmd
        ));
    }
}

fn collect_network_suggestions(still_running: &[Operation], steps: &mut Vec<String>) {
    const NETWORK_KEYWORDS: &[&str] = &[
        "network", "http", "https", "tcp", "udp", "socket", "curl", "wget", "git", "api", "rest",
        "graphql", "rpc", "ssh", "ftp", "scp", "rsync", "net", "audit", "update", "search", "add",
        "install", "fetch", "clone", "pull", "push", "download", "upload", "sync",
    ];
    let has_network_ops = still_running
        .iter()
        .any(|op| NETWORK_KEYWORDS.iter().any(|kw| op.tool_name.contains(kw)));
    if has_network_ops {
        steps.push(
            "• Network operations detected - check internet connection: ping 8.8.8.8".to_string(),
        );
        steps.push("• Try running with offline flags if tool supports them".to_string());
    }
}

fn collect_build_suggestions(still_running: &[Operation], steps: &mut Vec<String>) {
    const BUILD_KEYWORDS: &[&str] = &[
        "build", "compile", "test", "lint", "clippy", "format", "check", "verify", "validate",
        "analyze",
    ];
    let has_build_ops = still_running
        .iter()
        .any(|op| BUILD_KEYWORDS.iter().any(|kw| op.tool_name.contains(kw)));
    if has_build_ops {
        steps.push(
            "• Build/compile operations can take time - consider increasing timeout_seconds"
                .to_string(),
        );
        steps.push("• Check system resources: top or htop".to_string());
        steps.push("• Consider running operations with verbose flags to see progress".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation_monitor::{Operation, OperationStatus};

    fn make_op(id: &str, tool: &str, status: OperationStatus) -> Operation {
        let mut op = Operation::new(id.to_string(), tool.to_string(), String::new(), None);
        op.state = status;
        op
    }

    #[test]
    fn test_is_lock_file_matches() {
        assert!(is_lock_file("Cargo.lock"));
        assert!(is_lock_file("package-lock.json"));
        assert!(is_lock_file("yarn.lock"));
        assert!(is_lock_file(".cargo-lock"));
    }

    #[test]
    fn test_is_lock_file_non_matches() {
        assert!(!is_lock_file("Cargo.toml"));
        assert!(!is_lock_file("src.rs"));
    }

    #[tokio::test]
    async fn test_spawn_progress_warnings_returns_handle_and_rx() {
        let (handle, mut rx) = spawn_progress_warnings(1.0);
        handle.abort();
        let _ = handle.await;
        let _ = rx.try_recv();
    }

    #[test]
    fn test_build_completion_result_empty_contents() {
        let start = Instant::now();
        let result = build_completion_result(vec![], start);
        assert!(!result.content.is_empty());
        let text = result.content.first().unwrap().as_text().unwrap();
        assert!(text.text.contains("No operations completed"));
    }

    #[test]
    fn test_build_completion_result_with_contents() {
        use rmcp::model::Content;
        let start = Instant::now();
        let contents = vec![Content::text("op output".to_string())];
        let result = build_completion_result(contents, start);
        assert_eq!(result.content.len(), 2);
        let first = result.content.first().unwrap().as_text().unwrap();
        assert!(first.text.contains("Completed"));
    }

    #[test]
    fn test_collect_process_suggestions() {
        let ops = vec![
            make_op("op1", "cargo_build", OperationStatus::InProgress),
            make_op("op2", "cargo_test", OperationStatus::InProgress),
        ];
        let mut steps = Vec::new();
        collect_process_suggestions(&ops, &mut steps);
        assert!(!steps.is_empty());
        assert!(steps.iter().any(|s| s.contains("cargo")));
    }

    #[test]
    fn test_collect_network_suggestions_with_network_op() {
        let ops = vec![make_op("op1", "git_clone", OperationStatus::InProgress)];
        let mut steps = Vec::new();
        collect_network_suggestions(&ops, &mut steps);
        assert!(steps.iter().any(|s| s.contains("Network")));
    }

    #[test]
    fn test_collect_network_suggestions_no_network_op() {
        let ops = vec![make_op("op1", "cargo_build", OperationStatus::InProgress)];
        let mut steps = Vec::new();
        collect_network_suggestions(&ops, &mut steps);
        assert!(steps.is_empty());
    }

    #[test]
    fn test_collect_build_suggestions_with_build_op() {
        let ops = vec![make_op("op1", "cargo_build", OperationStatus::InProgress)];
        let mut steps = Vec::new();
        collect_build_suggestions(&ops, &mut steps);
        assert!(steps.iter().any(|s| s.contains("Build")));
    }

    #[test]
    fn test_collect_build_suggestions_no_build_op() {
        let ops = vec![make_op("op1", "echo_tool", OperationStatus::InProgress)];
        let mut steps = Vec::new();
        collect_build_suggestions(&ops, &mut steps);
        assert!(steps.is_empty());
    }

    #[tokio::test]
    async fn test_scan_dir_for_lock_files_finds_lock() {
        let temp = tempfile::tempdir().unwrap();
        let target_dir = temp.path().join("target");
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(target_dir.join("Cargo.lock"), "").unwrap();

        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp.path()).unwrap();

        let mut steps = Vec::new();
        scan_dir_for_lock_files("target", &mut steps).await;

        std::env::set_current_dir(&original_cwd).unwrap();

        assert!(!steps.is_empty(), "Should find Cargo.lock");
    }

    #[tokio::test]
    async fn test_scan_dir_for_lock_files_nonexistent_dir() {
        let mut steps = Vec::new();
        scan_dir_for_lock_files("nonexistent_dir_12345", &mut steps).await;
        assert!(steps.is_empty());
    }

    #[tokio::test]
    async fn test_calculate_intelligent_timeout() {
        let (service, _tmp) = crate::test_utils::client::setup_test_environment().await;
        let timeout = service.calculate_intelligent_timeout(&[]).await;
        assert!(timeout >= 600.0);
    }
}
