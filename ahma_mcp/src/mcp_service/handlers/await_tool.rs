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
        let (warning_tx, mut warning_rx) = tokio::sync::mpsc::unbounded_channel();

        let warning_task = {
            let warning_tx = warning_tx.clone();
            let timeout_secs = timeout_seconds;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs_f64(timeout_secs * 0.5)).await;
                let _ = warning_tx.send(format!(
                    "Wait operation 50% complete ({:.0}s remaining)",
                    timeout_secs * 0.5
                ));
                tokio::time::sleep(std::time::Duration::from_secs_f64(timeout_secs * 0.25)).await;
                let _ = warning_tx.send(format!(
                    "Wait operation 75% complete ({:.0}s remaining)",
                    timeout_secs * 0.25
                ));
                tokio::time::sleep(std::time::Duration::from_secs_f64(timeout_secs * 0.15)).await;
                let _ = warning_tx.send(format!(
                    "Wait operation 90% complete ({:.0}s remaining)",
                    timeout_secs * 0.1
                ));
            })
        };

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
            Ok(contents) => {
                let elapsed = wait_start.elapsed();
                if !contents.is_empty() {
                    let mut result_contents = vec![Content::text(format!(
                        "Completed {} operations in {:.2}s",
                        contents.len(),
                        elapsed.as_secs_f64()
                    ))];
                    result_contents.extend(contents);
                    Ok(CallToolResult::success(result_contents))
                } else {
                    Ok(CallToolResult::success(vec![Content::text(
                        "No operations completed within timeout period".to_string(),
                    )]))
                }
            }
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
        const DEFAULT_AWAIT_TIMEOUT: f64 = 240.0;

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
        // Check if operation exists
        let operation = self.operation_monitor.get_operation(&op_id).await;

        if operation.is_none() {
            // Check if it's in completed operations
            let completed_ops = self.operation_monitor.get_completed_operations().await;
            if let Some(completed_op) = completed_ops.iter().find(|op| op.id == op_id) {
                let mut contents = vec![Content::text(format!(
                    "Operation {} already completed",
                    op_id
                ))];
                contents.extend(common::serialize_operations_to_content(
                    std::slice::from_ref(completed_op),
                ));
                return Ok(CallToolResult::success(contents));
            } else {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Operation {} not found",
                    op_id
                ))]));
            }
        }

        // Wait for the specific operation
        tracing::info!("Waiting for operation: {}", op_id);

        // Use a reasonable timeout (e.g., 5 minutes)
        let timeout_duration = std::time::Duration::from_secs(300);
        let wait_start = Instant::now();

        let wait_result = tokio::time::timeout(
            timeout_duration,
            self.operation_monitor.wait_for_operation(&op_id),
        )
        .await;

        match wait_result {
            Ok(Some(completed_op)) => {
                let elapsed = wait_start.elapsed();
                let mut contents = vec![Content::text(format!(
                    "Completed 1 operations in {:.2}s",
                    elapsed.as_secs_f64()
                ))];
                contents.extend(common::serialize_operations_to_content(
                    std::slice::from_ref(&completed_op),
                ));
                Ok(CallToolResult::success(contents))
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
        for dir in &["target", "node_modules", ".cargo", "tmp", "temp"] {
            if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if let Some(name) = entry.file_name().to_str()
                        && LOCK_PATTERNS.iter().any(|p| name.contains(p))
                    {
                        steps.push(format!(
                            "• Remove potential stale lock file: rm {}/{}",
                            dir, name
                        ));
                    }
                }
            }
        }
        if tokio::fs::metadata(".").await.is_ok() {
            steps.push("• Check available disk space: df -h .".to_string());
        }
    }
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
