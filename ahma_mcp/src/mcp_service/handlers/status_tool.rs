use super::common;
use crate::AhmaMcpService;
use crate::mcp_service::schema;
use crate::operation_monitor::Operation;
use rmcp::model::{CallToolResult, Content, ErrorData as McpError};
use serde_json::{Map, Value};
use std::sync::Arc;

impl AhmaMcpService {
    /// Generates the specific input schema for the `status` tool.
    pub fn generate_input_schema_for_status(&self) -> Arc<Map<String, Value>> {
        let mut properties = Map::new();
        properties.insert(
            "tools".to_string(),
            schema::string_property(
                "Comma-separated tool name prefixes to filter by (optional; shows all if omitted)",
            ),
        );
        properties.insert(
            "id".to_string(),
            schema::string_property(
                "Specific operation ID to query (optional; shows all if omitted)",
            ),
        );
        schema::object_input_schema(properties, &[])
    }

    /// Handles the 'status' tool call.
    pub async fn handle_status(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        let tool_filters = common::parse_tool_filters(&args);
        let specific_id = common::parse_id(&args);

        let mut contents = Vec::new();

        let op_id_ref = specific_id.as_deref();

        let active_ops: Vec<Operation> = self
            .operation_monitor
            .get_all_active_operations()
            .await
            .into_iter()
            .filter(|op| common::operation_matches_filters(op, &tool_filters, op_id_ref))
            .collect();

        let completed_ops: Vec<Operation> = self
            .operation_monitor
            .get_completed_operations()
            .await
            .into_iter()
            .filter(|op| common::operation_matches_filters(op, &tool_filters, op_id_ref))
            .collect();

        // Create summary with timing information
        let active_count = active_ops.len();
        let completed_count = completed_ops.len();
        let total_count = active_count + completed_count;

        let summary = if let Some(ref id) = specific_id {
            if total_count == 0 {
                format!("Operation '{}' not found", id)
            } else {
                format!("Operation '{}' found", id)
            }
        } else if tool_filters.is_empty() {
            format!(
                "Operations status: {} active, {} completed (total: {})",
                active_count, completed_count, total_count
            )
        } else {
            format!(
                "Operations status for '{}': {} active, {} completed (total: {})",
                tool_filters.join(", "),
                active_count,
                completed_count,
                total_count
            )
        };

        contents.push(Content::text(summary));

        // Add concurrency efficiency analysis
        if !completed_ops.is_empty()
            && let Some(efficiency_analysis) = Self::run_concurrency_analysis(&completed_ops)
        {
            contents.push(Content::text(format!(
                "\nConcurrency Analysis:\n{}",
                efficiency_analysis
            )));
        }

        if !active_ops.is_empty() {
            contents.push(Content::text("\n=== ACTIVE OPERATIONS ===".to_string()));
            contents.extend(common::serialize_operations_to_content(&active_ops));
        }

        if !completed_ops.is_empty() {
            contents.push(Content::text("\n=== COMPLETED OPERATIONS ===".to_string()));
            contents.extend(common::serialize_operations_to_content(&completed_ops));
        }

        Ok(CallToolResult::success(contents))
    }

    fn run_concurrency_analysis(completed_ops: &[Operation]) -> Option<String> {
        let mut total_execution_time = 0.0;
        let mut total_wait_time = 0.0;
        let mut operations_with_waits = 0;

        for op in completed_ops {
            if let Some(end_time) = op.end_time
                && let Ok(execution_duration) = end_time.duration_since(op.start_time)
            {
                total_execution_time += execution_duration.as_secs_f64();

                if let Some(first_wait_time) = op.first_wait_time
                    && let Ok(wait_duration) = first_wait_time.duration_since(op.start_time)
                {
                    total_wait_time += wait_duration.as_secs_f64();
                    operations_with_waits += 1;
                }
            }
        }

        if total_execution_time > 0.0 {
            if operations_with_waits > 0 {
                let avg_wait_ratio = (total_wait_time / total_execution_time) * 100.0;
                if avg_wait_ratio < 10.0 {
                    Some(format!(
                        "OK Good concurrency efficiency: {:.1}% of execution time spent waiting",
                        avg_wait_ratio
                    ))
                } else if avg_wait_ratio < 50.0 {
                    Some(format!(
                        "WARNING Moderate concurrency efficiency: {:.1}% of execution time spent waiting",
                        avg_wait_ratio
                    ))
                } else {
                    Some(format!(
                        "WARNING Low concurrency efficiency: {:.1}% of execution time spent waiting. Consider using status tool instead of frequent waits.",
                        avg_wait_ratio
                    ))
                }
            } else {
                Some("OK Excellent concurrency: No blocking waits detected".to_string())
            }
        } else {
            None
        }
    }
}
