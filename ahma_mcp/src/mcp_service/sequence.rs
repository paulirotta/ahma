//! Sequence execution for MCP tools.
//!
//! Contains handlers for executing sequence tools that invoke multiple
//! other tools in order, both synchronously and asynchronously.

use rmcp::model::{CallToolRequestParams, CallToolResult, Content, ErrorData as McpError};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::adapter::Adapter;
use crate::callback_system::CallbackSender;
use crate::client_type::McpClientType;
use crate::config::{SequenceStep, SubcommandConfig, ToolConfig};
use crate::constants::SEQUENCE_STEP_DELAY_MS;
use crate::mcp_callback::McpCallbackSender;
use crate::operation_monitor::OperationMonitor;

use super::handlers::common;
use super::subcommand::find_subcommand_config_from_args;
use super::types::{META_PARAMS, SequenceKind};

static SEQUENCE_ID: AtomicU64 = AtomicU64::new(1);

// ============= Helper Functions =============

/// Extracts working directory from params, falling back to sandbox scope or "."
fn extract_working_directory(adapter: &Adapter, params: &CallToolRequestParams) -> String {
    params
        .arguments
        .as_ref()
        .and_then(|args| args.get("working_directory"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            if adapter.sandbox().is_test_mode() {
                None
            } else {
                adapter
                    .sandbox()
                    .scopes()
                    .first()
                    .map(|p: &std::path::PathBuf| p.to_string_lossy().to_string())
            }
        })
        .unwrap_or_else(|| ".".to_string())
}

/// Merges parent arguments with step arguments, excluding meta-parameters.
fn merge_step_arguments(
    parent_args: &Map<String, Value>,
    step_args: &Map<String, Value>,
) -> Map<String, Value> {
    let mut merged = Map::new();
    for (key, value) in parent_args.iter() {
        if !META_PARAMS.contains(&key.as_str()) {
            merged.insert(key.clone(), value.clone());
        }
    }
    merged.extend(step_args.clone());
    merged
}

/// Looks up a tool config from the configs map, returning an error if not found.
fn get_tool_config(
    configs: &Arc<RwLock<HashMap<String, ToolConfig>>>,
    tool_name: &str,
) -> Result<ToolConfig, McpError> {
    let configs_lock = configs.read().unwrap();
    configs_lock.get(tool_name).cloned().ok_or_else(|| {
        common::mcp_internal(format!(
            "Tool '{}' referenced in sequence step is not configured.",
            tool_name
        ))
    })
}

/// Finds subcommand config with proper error handling for sequence steps.
fn find_step_subcommand<'a>(
    config: &'a ToolConfig,
    subcommand: &str,
    tool_name: &str,
) -> Result<(&'a SubcommandConfig, Vec<String>), McpError> {
    find_subcommand_config_from_args(config, Some(subcommand.to_string())).ok_or_else(|| {
        common::mcp_internal(format!(
            "Subcommand '{}' for tool '{}' not found in sequence step.",
            subcommand, tool_name
        ))
    })
}

/// Generates a new unique operation ID.
fn next_id() -> String {
    format!("op_{}", SEQUENCE_ID.fetch_add(1, Ordering::SeqCst))
}

/// Creates a callback sender if a progress token is available.
fn create_callback(
    context: &RequestContext<RoleServer>,
    id: &str,
) -> Option<Box<dyn CallbackSender>> {
    let progress_token = context.meta.get_progress_token()?;
    let client_type = McpClientType::from_peer(&context.peer);
    Some(Box::new(McpCallbackSender::new(
        context.peer.clone(),
        id.to_string(),
        Some(progress_token),
        client_type,
    )) as Box<dyn CallbackSender>)
}

/// Applies inter-step delay if configured and not the last step.
async fn apply_step_delay(step_delay_ms: u64, current_index: usize, total_steps: usize) {
    if step_delay_ms > 0 && current_index + 1 < total_steps {
        tokio::time::sleep(Duration::from_millis(step_delay_ms)).await;
    }
}

/// Handles execution of sequence tools - tools that invoke multiple other tools in order.
pub async fn handle_sequence_tool(
    adapter: &Adapter,
    _operation_monitor: &OperationMonitor,
    configs: &Arc<RwLock<HashMap<String, ToolConfig>>>,
    config: &ToolConfig,
    params: CallToolRequestParams,
    context: RequestContext<RoleServer>,
) -> Result<CallToolResult, McpError> {
    let sequence = config.sequence.as_ref().unwrap(); // Safe due to prior check
    let step_delay_ms = config.step_delay_ms.unwrap_or(SEQUENCE_STEP_DELAY_MS);

    // Determine if sequence should run synchronously
    // - If synchronous is true, run sync
    // - If synchronous is false or None, default is async for sequence tools
    let run_synchronously = config.synchronous.unwrap_or(false);

    if run_synchronously {
        handle_sequence_tool_sync(adapter, configs, config, params, sequence, step_delay_ms).await
    } else {
        handle_sequence_tool_async(adapter, configs, params, context, sequence, step_delay_ms).await
    }
}

/// Handles synchronous sequence execution - blocks until all steps complete
async fn handle_sequence_tool_sync(
    adapter: &Adapter,
    configs: &Arc<RwLock<HashMap<String, ToolConfig>>>,
    config: &ToolConfig,
    params: CallToolRequestParams,
    sequence: &[SequenceStep],
    step_delay_ms: u64,
) -> Result<CallToolResult, McpError> {
    let mut final_result = CallToolResult::success(vec![]);
    let mut all_outputs = Vec::new();
    let kind = SequenceKind::TopLevel;
    let parent_args = params.arguments.clone().unwrap_or_default();
    let working_directory = extract_working_directory(adapter, &params);

    for (index, step) in sequence.iter().enumerate() {
        if should_skip_step_with_context(&kind, step, &working_directory) {
            final_result
                .content
                .push(Content::text(format_step_skipped_message(&kind, step)));
            continue;
        }

        let merged_args = merge_step_arguments(&parent_args, &step.args);
        let step_tool_config = get_tool_config(configs, &step.tool)?;
        let (subcommand_config, command_parts) =
            find_step_subcommand(&step_tool_config, &step.subcommand, &step.tool)?;

        let step_result = adapter
            .execute_sync_in_dir(
                &command_parts.join(" "),
                Some(merged_args),
                &working_directory,
                config.timeout_seconds,
                Some(subcommand_config),
            )
            .await;

        match step_result {
            Ok(output) => {
                let output_text = if output.is_empty() {
                    "(no output)"
                } else {
                    &output
                };
                let message = format!(
                    "OK Step {} completed: {} {}\n{}",
                    index + 1,
                    step.tool,
                    step.subcommand,
                    output_text
                );
                all_outputs.push(message);
                tracing::info!(
                    "Sequence step {} succeeded: {} {}",
                    index + 1,
                    step.tool,
                    step.subcommand
                );
            }
            Err(e) => {
                all_outputs.push(format!(
                    "✗ Step {} FAILED: {} {}\nError: {}",
                    index + 1,
                    step.tool,
                    step.subcommand,
                    e
                ));
                tracing::error!(
                    "Sequence step {} failed: {} {}: {}",
                    index + 1,
                    step.tool,
                    step.subcommand,
                    e
                );
                final_result.content.push(Content::text(format!(
                    "Sequence failed at step {}:\n\n{}",
                    index + 1,
                    all_outputs.join("\n\n")
                )));
                final_result.is_error = Some(true);
                return Ok(final_result);
            }
        }

        apply_step_delay(step_delay_ms, index, sequence.len()).await;
    }

    final_result.content.push(Content::text(format!(
        "All {} sequence steps completed successfully:\n\n{}",
        sequence.len(),
        all_outputs.join("\n\n")
    )));
    Ok(final_result)
}

/// Handles asynchronous sequence execution - starts all steps and returns immediately
async fn handle_sequence_tool_async(
    adapter: &Adapter,
    configs: &Arc<RwLock<HashMap<String, ToolConfig>>>,
    params: CallToolRequestParams,
    context: RequestContext<RoleServer>,
    sequence: &[SequenceStep],
    step_delay_ms: u64,
) -> Result<CallToolResult, McpError> {
    let mut final_result = CallToolResult::success(vec![]);
    let kind = SequenceKind::TopLevel;
    let parent_args = params.arguments.clone().unwrap_or_default();
    let working_directory = extract_working_directory(adapter, &params);

    for (index, step) in sequence.iter().enumerate() {
        if should_skip_step_with_context(&kind, step, &working_directory) {
            final_result
                .content
                .push(Content::text(format_step_skipped_message(&kind, step)));
            continue;
        }

        let mut merged_args = merge_step_arguments(&parent_args, &step.args);
        merged_args.insert(
            "working_directory".to_string(),
            Value::String(working_directory.clone()),
        );

        let step_tool_config = get_tool_config(configs, &step.tool)?;
        let (subcommand_config, command_parts) =
            find_step_subcommand(&step_tool_config, &step.subcommand, &step.tool)?;

        let id = next_id();
        let callback = create_callback(&context, &id);

        let step_result = adapter
            .execute_async_in_dir_with_options(
                &step.tool,
                &command_parts.join(" "),
                ".",
                crate::adapter::AsyncExecOptions {
                    id: Some(id),
                    args: Some(merged_args),
                    timeout: None,
                    callback,
                    subcommand_config: Some(subcommand_config),
                    log_monitor_config: None,
                },
            )
            .await;

        match step_result {
            Ok(id) => {
                final_result
                    .content
                    .push(Content::text(format_step_started_message(&kind, step, &id)));
            }
            Err(e) => {
                let error_message = format!(
                    "Sequence step '{}' failed to start: {}. Halting sequence.",
                    step.tool, e
                );
                tracing::error!("{}", error_message);
                return Err(common::mcp_internal(error_message));
            }
        }

        apply_step_delay(step_delay_ms, index, sequence.len()).await;
    }

    Ok(final_result)
}

/// Handles execution of subcommand sequences - subcommands that invoke multiple cargo commands in order.
pub async fn handle_subcommand_sequence(
    adapter: &Adapter,
    config: &ToolConfig,
    subcommand_config: &SubcommandConfig,
    params: CallToolRequestParams,
    context: RequestContext<RoleServer>,
) -> Result<CallToolResult, McpError> {
    let sequence = subcommand_config.sequence.as_ref().unwrap(); // Safe due to prior check
    let step_delay_ms = subcommand_config
        .step_delay_ms
        .or(config.step_delay_ms)
        .unwrap_or(SEQUENCE_STEP_DELAY_MS);
    let mut final_result = CallToolResult::success(vec![]);
    let kind = SequenceKind::Subcommand {
        base_config: config,
    };

    for (index, step) in sequence.iter().enumerate() {
        let (step_config, command_parts) =
            find_subcommand_config_from_args(config, Some(step.subcommand.clone())).ok_or_else(
                || {
                    let msg = format!(
                        "Subcommand sequence step '{}' not found in tool config. Halting sequence.",
                        step.subcommand
                    );
                    tracing::error!("{}", msg);
                    common::mcp_internal(msg)
                },
            )?;

        let id = next_id();
        let callback = create_callback(&context, &id);

        let step_result = adapter
            .execute_async_in_dir_with_options(
                &config.name,
                &command_parts.join(" "),
                ".",
                crate::adapter::AsyncExecOptions {
                    id: Some(id),
                    args: params.arguments.clone(),
                    timeout: None,
                    callback,
                    subcommand_config: Some(step_config),
                    log_monitor_config: None,
                },
            )
            .await;

        match step_result {
            Ok(id) => {
                final_result
                    .content
                    .push(Content::text(format_step_started_message(&kind, step, &id)));
            }
            Err(e) => {
                let msg = format!(
                    "Subcommand sequence step '{}' failed to start: {}. Halting sequence.",
                    step.subcommand, e
                );
                tracing::error!("{}", msg);
                return Err(common::mcp_internal(msg));
            }
        }

        apply_step_delay(step_delay_ms, index, sequence.len()).await;
    }

    Ok(final_result)
}

/// Formats a message for a sequence step that was started.
/// Unified handler for both top-level and subcommand sequences.
pub fn format_step_started_message(kind: &SequenceKind, step: &SequenceStep, id: &str) -> String {
    let (step_name, prefix) = match kind {
        SequenceKind::TopLevel => (&step.tool, "Sequence step"),
        SequenceKind::Subcommand { .. } => (&step.subcommand, "Subcommand sequence step"),
    };
    let hint = crate::tool_hints::preview(id, step_name);
    match step.description.as_deref() {
        Some(desc) if !desc.is_empty() => {
            format!(
                "{} '{}' ({}) started with operation ID: {}{}",
                prefix, step_name, desc, id, hint
            )
        }
        _ => format!(
            "{} '{}' started with operation ID: {}{}",
            prefix, step_name, id, hint
        ),
    }
}

/// Formats a message for a sequence step that was skipped.
/// Unified handler for both top-level and subcommand sequences.
pub fn format_step_skipped_message(kind: &SequenceKind, step: &SequenceStep) -> String {
    let (step_name, prefix) = match kind {
        SequenceKind::TopLevel => (&step.tool, "Sequence step"),
        SequenceKind::Subcommand { .. } => (&step.subcommand, "Subcommand sequence step"),
    };
    match step.description.as_deref() {
        Some(desc) if !desc.is_empty() => {
            format!(
                "{} '{}' ({}) skipped due to environment override.",
                prefix, step_name, desc
            )
        }
        _ => format!(
            "{} '{}' skipped due to environment override.",
            prefix, step_name
        ),
    }
}

/// Checks if a sequence step should be skipped based on environment variables or file existence.
pub fn should_skip_step_with_context(
    _kind: &SequenceKind,
    step: &SequenceStep,
    working_dir: &str,
) -> bool {
    // Check environment variables first
    // AHMA_SKIP_SEQUENCE_TOOLS support removed

    // Check file existence
    if let Some(path) = &step.skip_if_file_exists {
        let full_path = std::path::Path::new(working_dir).join(path);
        if full_path.exists() {
            tracing::info!(
                "Skipping sequence step {} because file exists: {:?}",
                step.tool,
                full_path
            );
            return true;
        }
    }

    if let Some(path) = &step.skip_if_file_missing {
        let full_path = std::path::Path::new(working_dir).join(path);
        if !full_path.exists() {
            tracing::info!(
                "Skipping sequence step {} because file is missing: {:?}",
                step.tool,
                full_path
            );
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_utils::client::create_test_config;
    use rmcp::model::CallToolRequestParams;
    use serde_json::{Map, Value, json};
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, RwLock};

    fn make_test_sequence_step(
        tool: &str,
        subcommand: &str,
        description: Option<&str>,
    ) -> SequenceStep {
        SequenceStep {
            tool: tool.to_string(),
            subcommand: subcommand.to_string(),
            description: description.map(|s| s.to_string()),
            args: Default::default(),
            skip_if_file_exists: None,
            skip_if_file_missing: None,
        }
    }

    fn make_dummy_tool_config() -> ToolConfig {
        ToolConfig {
            name: "test_tool".to_string(),
            description: "Test tool".to_string(),
            command: "test".to_string(),
            subcommand: None,
            input_schema: None,
            timeout_seconds: None,
            synchronous: None,
            hints: Default::default(),
            enabled: true,
            guidance_key: None,
            sequence: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
            monitor_level: None,
            monitor_stream: None,
            tool_type: None,
            livelog: None,
        }
    }

    // ============= format_step_started_message tests =============

    #[test]
    fn test_format_step_started_message_toplevel_with_description() {
        let step = make_test_sequence_step("cargo", "build", Some("Build the project"));
        let kind = SequenceKind::TopLevel;
        let message = format_step_started_message(&kind, &step, "op_test_001");

        assert!(message.contains("cargo"));
        assert!(message.contains("Build the project"));
        assert!(message.contains("op_test_001"));
        assert!(message.contains("Sequence step"));
    }

    #[test]
    fn test_format_step_started_message_toplevel_without_description() {
        let step = make_test_sequence_step("cargo", "build", None);
        let kind = SequenceKind::TopLevel;
        let message = format_step_started_message(&kind, &step, "op_test_002");

        assert!(message.contains("cargo"));
        assert!(message.contains("op_test_002"));
        assert!(!message.contains("()"));
    }

    #[test]
    fn test_format_step_started_message_subcommand_with_description() {
        let step = make_test_sequence_step("cargo", "clippy", Some("Run linter"));
        let dummy_config = make_dummy_tool_config();
        let kind = SequenceKind::Subcommand {
            base_config: &dummy_config,
        };
        let message = format_step_started_message(&kind, &step, "op_sub_test_001");

        assert!(message.contains("clippy"));
        assert!(message.contains("Run linter"));
        assert!(message.contains("op_sub_test_001"));
        assert!(message.contains("Subcommand sequence step"));
    }

    // ============= format_step_skipped_message tests =============

    #[test]
    fn test_format_step_skipped_message_toplevel_with_description() {
        let step = make_test_sequence_step("cargo", "audit", Some("Security audit"));
        let kind = SequenceKind::TopLevel;
        let message = format_step_skipped_message(&kind, &step);

        assert!(message.contains("cargo"));
        assert!(message.contains("Security audit"));
        assert!(message.contains("skipped"));
    }

    #[test]
    fn test_format_step_skipped_message_toplevel_without_description() {
        let step = make_test_sequence_step("cargo", "audit", None);
        let kind = SequenceKind::TopLevel;
        let message = format_step_skipped_message(&kind, &step);

        assert!(message.contains("cargo"));
        assert!(message.contains("skipped"));
        assert!(!message.contains("()"));
    }

    #[test]
    fn test_format_step_skipped_message_subcommand() {
        let step = make_test_sequence_step("cargo", "nextest_run", Some("Run tests"));
        let dummy_config = make_dummy_tool_config();
        let kind = SequenceKind::Subcommand {
            base_config: &dummy_config,
        };
        let message = format_step_skipped_message(&kind, &step);

        assert!(message.contains("nextest_run"));
        assert!(message.contains("Run tests"));
        assert!(message.contains("skipped"));
        assert!(message.contains("Subcommand sequence step"));
    }

    #[test]
    fn test_should_skip_step_file_exists() {
        let temp = tempfile::tempdir().unwrap();
        let file_path = temp.path().join("skip_me.txt");
        std::fs::write(&file_path, "exists").unwrap();

        let mut step = make_test_sequence_step("cargo", "build", None);
        step.skip_if_file_exists = Some("skip_me.txt".to_string());

        let kind = SequenceKind::TopLevel;
        assert!(should_skip_step_with_context(
            &kind,
            &step,
            temp.path().to_str().unwrap()
        ));

        // Should not skip if file doesn't exist
        step.skip_if_file_exists = Some("non_existent.txt".to_string());
        assert!(!should_skip_step_with_context(
            &kind,
            &step,
            temp.path().to_str().unwrap()
        ));
    }

    #[test]
    fn test_should_skip_step_file_missing() {
        let temp = tempfile::tempdir().unwrap();

        let mut step = make_test_sequence_step("cargo", "build", None);
        step.skip_if_file_missing = Some("missing.txt".to_string());

        let kind = SequenceKind::TopLevel;
        assert!(should_skip_step_with_context(
            &kind,
            &step,
            temp.path().to_str().unwrap()
        ));

        // Should not skip if file exists
        let file_path = temp.path().join("exists.txt");
        std::fs::write(&file_path, "exists").unwrap();
        step.skip_if_file_missing = Some("exists.txt".to_string());
        assert!(!should_skip_step_with_context(
            &kind,
            &step,
            temp.path().to_str().unwrap()
        ));
    }

    // ============= merge_step_arguments tests =============

    #[test]
    fn test_merge_step_arguments_excludes_meta_params() {
        let mut parent = Map::new();
        parent.insert(
            "working_directory".to_string(),
            Value::String("/tmp".into()),
        );
        parent.insert("execution_mode".to_string(), Value::String("Sync".into()));
        parent.insert("timeout_seconds".to_string(), json!(30));
        parent.insert("custom_arg".to_string(), Value::String("parent_val".into()));

        let mut step = Map::new();
        step.insert("message".to_string(), Value::String("step_val".into()));

        let merged = merge_step_arguments(&parent, &step);

        assert!(!merged.contains_key("working_directory"));
        assert!(!merged.contains_key("execution_mode"));
        assert!(!merged.contains_key("timeout_seconds"));
        assert_eq!(
            merged.get("custom_arg"),
            Some(&Value::String("parent_val".into()))
        );
        assert_eq!(
            merged.get("message"),
            Some(&Value::String("step_val".into()))
        );
    }

    #[test]
    fn test_merge_step_arguments_step_overrides_parent() {
        let mut parent = Map::new();
        parent.insert("message".to_string(), Value::String("parent_msg".into()));

        let mut step = Map::new();
        step.insert("message".to_string(), Value::String("step_msg".into()));

        let merged = merge_step_arguments(&parent, &step);
        assert_eq!(
            merged.get("message"),
            Some(&Value::String("step_msg".into()))
        );
    }

    // ============= get_tool_config tests =============

    #[test]
    fn test_get_tool_config_found() {
        let mut configs = HashMap::new();
        let tc = make_dummy_tool_config();
        configs.insert("test_tool".to_string(), tc.clone());
        let configs = Arc::new(RwLock::new(configs));

        let result = get_tool_config(&configs, "test_tool").unwrap();
        assert_eq!(result.name, "test_tool");
    }

    #[test]
    fn test_get_tool_config_not_found() {
        let configs = Arc::new(RwLock::new(HashMap::new()));
        let err = get_tool_config(&configs, "nonexistent").unwrap_err();
        assert!(format!("{err:?}").contains("nonexistent"));
        assert!(format!("{err:?}").contains("not configured"));
    }

    // ============= extract_working_directory tests =============

    #[test]
    fn test_extract_working_directory_from_params() {
        let adapter = create_test_config(Path::new(".")).unwrap();
        let mut params = CallToolRequestParams::new("test".to_string());
        params = params.with_arguments(
            json!({"working_directory": "/custom/path"})
                .as_object()
                .unwrap()
                .clone(),
        );
        let wd = extract_working_directory(&adapter, &params);
        assert_eq!(wd, "/custom/path");
    }

    #[test]
    fn test_extract_working_directory_fallback_default() {
        let adapter = create_test_config(Path::new(".")).unwrap();
        let params = CallToolRequestParams::new("test".to_string());
        let wd = extract_working_directory(&adapter, &params);
        assert_eq!(wd, ".");
    }

    // ============= find_step_subcommand tests =============

    #[test]
    fn test_find_step_subcommand_found() {
        // Use JSON deserialization to match real config format (enabled uses default_enabled)
        let config: ToolConfig = serde_json::from_str(
            r#"{"name":"test_tool","description":"Test","command":"echo","enabled":true,"subcommand":[{"name":"default","description":"Default"}]}"#,
        )
        .unwrap();
        let (sub, parts) = find_step_subcommand(&config, "default", "test_tool").unwrap();
        assert_eq!(sub.name, "default");
        assert!(!parts.is_empty());
    }

    #[test]
    fn test_find_step_subcommand_not_found() {
        let config = make_dummy_tool_config();
        let err = find_step_subcommand(&config, "nonexistent_sub", "test_tool").unwrap_err();
        assert!(format!("{err:?}").contains("nonexistent_sub"));
        assert!(format!("{err:?}").contains("not found"));
    }

    // ============= next_id tests =============

    #[test]
    fn test_next_id_format_and_increment() {
        let id1 = next_id();
        let id2 = next_id();
        assert!(id1.starts_with("op_"));
        assert!(id2.starts_with("op_"));
        let n1: u64 = id1.trim_start_matches("op_").parse().unwrap();
        let n2: u64 = id2.trim_start_matches("op_").parse().unwrap();
        assert!(n2 > n1);
    }

    // ============= format helpers with empty description =============

    #[test]
    fn test_format_step_started_message_empty_description() {
        let mut step = make_test_sequence_step("cargo", "build", None);
        step.description = Some(String::new());
        let kind = SequenceKind::TopLevel;
        let message = format_step_started_message(&kind, &step, "op_001");
        assert!(message.contains("cargo"));
        assert!(message.contains("op_001"));
        assert!(!message.contains("()"));
    }

    #[test]
    fn test_format_step_skipped_message_subcommand_without_description() {
        let step = make_test_sequence_step("cargo", "clippy", None);
        let dummy_config = make_dummy_tool_config();
        let kind = SequenceKind::Subcommand {
            base_config: &dummy_config,
        };
        let message = format_step_skipped_message(&kind, &step);
        assert!(message.contains("clippy"));
        assert!(message.contains("skipped"));
        assert!(message.contains("Subcommand sequence step"));
    }

    // ============= apply_step_delay tests =============

    #[tokio::test]
    async fn test_apply_step_delay_skips_last_step() {
        // Last step (index == total - 1) should not delay
        let start = std::time::Instant::now();
        apply_step_delay(1000, 2, 3).await; // index 2 of 3 = last
        assert!(start.elapsed().as_millis() < 50);
    }

    #[tokio::test]
    async fn test_apply_step_delay_skips_zero_delay() {
        let start = std::time::Instant::now();
        apply_step_delay(0, 0, 3).await; // zero delay
        assert!(start.elapsed().as_millis() < 50);
    }

    #[tokio::test]
    async fn test_apply_step_delay_applies_delay() {
        let start = std::time::Instant::now();
        apply_step_delay(50, 0, 3).await; // 50ms delay, not last step
        assert!(start.elapsed().as_millis() >= 40); // allow some slack
    }

    // ============= should_skip_step tests (additional) =============

    #[test]
    fn test_should_skip_step_no_conditions() {
        let step = make_test_sequence_step("cargo", "build", None);
        let kind = SequenceKind::TopLevel;
        assert!(!should_skip_step_with_context(&kind, &step, "/nonexistent"));
    }

    #[test]
    fn test_should_skip_step_both_conditions_exists_wins() {
        let temp = tempfile::tempdir().unwrap();
        let file_path = temp.path().join("exists.txt");
        std::fs::write(&file_path, "").unwrap();

        let mut step = make_test_sequence_step("cargo", "build", None);
        step.skip_if_file_exists = Some("exists.txt".to_string());
        step.skip_if_file_missing = Some("also_exists.txt".to_string());

        let kind = SequenceKind::TopLevel;
        // skip_if_file_exists matches, so should skip
        assert!(should_skip_step_with_context(
            &kind,
            &step,
            temp.path().to_str().unwrap()
        ));
    }
}
