use super::super::NEXT_ID;
use crate::AhmaMcpService;
use crate::callback_system::CallbackSender;
use crate::client_type::McpClientType;
use crate::mcp_callback::McpCallbackSender;
use crate::shell_pool::platform_shell_program;
use rmcp::{
    model::{CallToolRequestParams, CallToolResult, Content, ErrorData as McpError},
    service::{RequestContext, RoleServer},
};
use serde_json::{Map, Value};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing;

impl AhmaMcpService {
    /// Generates the specific input schema for the `sandboxed_shell` tool.
    pub fn generate_input_schema_for_sandboxed_shell(&self) -> Arc<Map<String, Value>> {
        let mut properties = Map::new();
        properties.insert(
            "command".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The shell command to execute (supports pipes, redirects, variables, etc.)"
            }),
        );
        properties.insert(
            "working_directory".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Working directory for command execution",
                "format": "path"
            }),
        );
        properties.insert(
            "monitor_level".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Enable live log monitoring at this severity level. When set, stderr/stdout is streamed line-by-line and alerts are pushed when error/warning patterns are detected. Values: error, warn, info, debug, trace",
                "enum": ["error", "warn", "info", "debug", "trace"]
            }),
        );
        properties.insert(
            "monitor_stream".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Which stream to monitor for log patterns (default: stderr). Use 'stdout' for tools like adb logcat that write logs to stdout.",
                "enum": ["stderr", "stdout", "both"],
                "default": "stderr"
            }),
        );

        let mut schema = Map::new();
        schema.insert("type".to_string(), Value::String("object".to_string()));
        schema.insert("properties".to_string(), Value::Object(properties));
        schema.insert(
            "required".to_string(),
            Value::Array(vec![Value::String("command".to_string())]),
        );
        Arc::new(schema)
    }

    /// Handles the 'sandboxed_shell' built-in tool call.
    pub async fn handle_sandboxed_shell(
        &self,
        params: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = params.arguments.unwrap_or_default();

        // Delay tool execution until sandbox is initialized from roots/list.
        // This is critical in HTTP bridge mode with deferred sandbox initialization.
        if self.adapter.sandbox().scopes().is_empty() && !self.adapter.sandbox().is_test_mode() {
            let error_message = "Sandbox initializing from client roots - retry tools/call after roots/list completes".to_string();
            tracing::warn!("{}", error_message);
            return Err(McpError::internal_error(error_message, None));
        }

        // Extract command (required)
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                McpError::invalid_params("command parameter is required".to_string(), None)
            })?
            .to_string();

        // Extract working_directory (optional)
        let working_directory = args
            .get("working_directory")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                if self.adapter.sandbox().is_test_mode() {
                    None
                } else {
                    self.adapter
                        .sandbox()
                        .scopes()
                        .first()
                        .map(|p: &std::path::PathBuf| p.to_string_lossy().to_string())
                }
            })
            .unwrap_or_else(|| ".".to_string());

        let timeout = args.get("timeout_seconds").and_then(|v| v.as_u64());

        // Extract optional log monitoring parameters
        let log_monitor_config =
            args.get("monitor_level")
                .and_then(|v| v.as_str())
                .map(|level_str| {
                    let monitor_level: crate::log_monitor::LogLevel = level_str
                        .parse()
                        .unwrap_or(crate::log_monitor::LogLevel::Error);
                    let monitor_stream: crate::log_monitor::MonitorStream = args
                        .get("monitor_stream")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_default();
                    crate::log_monitor::LogMonitorConfig {
                        monitor_level,
                        monitor_stream,
                        rate_limit_seconds: self.monitor_rate_limit_seconds,
                    }
                });

        // Determine execution mode
        let execution_mode = if self.force_synchronous {
            crate::adapter::ExecutionMode::Synchronous
        } else if let Some(mode_str) = args.get("execution_mode").and_then(|v| v.as_str()) {
            match mode_str {
                "Synchronous" => crate::adapter::ExecutionMode::Synchronous,
                "AsyncResultPush" => crate::adapter::ExecutionMode::AsyncResultPush,
                _ => crate::adapter::ExecutionMode::AsyncResultPush,
            }
        } else {
            crate::adapter::ExecutionMode::AsyncResultPush
        };

        // Build arguments map for adapter
        let mut adapter_args = Map::new();
        adapter_args.insert("command".to_string(), serde_json::Value::String(command));
        if let Some(wd) = args.get("working_directory") {
            adapter_args.insert("working_directory".to_string(), wd.clone());
        }

        let subcommand_config = Self::build_shell_subcommand_config(timeout, &execution_mode);

        adapter_args.insert("c_flag".to_string(), serde_json::Value::Bool(true));

        match execution_mode {
            crate::adapter::ExecutionMode::Synchronous => {
                self.execute_shell_sync(
                    adapter_args,
                    &working_directory,
                    timeout,
                    &subcommand_config,
                    &context,
                )
                .await
            }
            crate::adapter::ExecutionMode::AsyncResultPush => {
                self.execute_shell_async(
                    adapter_args,
                    &working_directory,
                    timeout,
                    &subcommand_config,
                    &context,
                    log_monitor_config,
                )
                .await
            }
        }
    }

    pub fn build_shell_subcommand_config(
        timeout: Option<u64>,
        execution_mode: &crate::adapter::ExecutionMode,
    ) -> crate::config::SubcommandConfig {
        crate::config::SubcommandConfig {
            name: "sandboxed_shell".to_string(),
            description: "Execute shell commands".to_string(),
            subcommand: None,
            options: Some(vec![crate::config::CommandOption {
                name: "c_flag".to_string(),
                option_type: "boolean".to_string(),
                description: Some("Execute command string".to_string()),
                required: Some(false),
                format: None,
                items: None,
                file_arg: None,
                file_flag: None,
                alias: Some("c".to_string()),
            }]),
            positional_args: Some(vec![crate::config::CommandOption {
                name: "command".to_string(),
                option_type: "string".to_string(),
                description: Some("Shell command to execute".to_string()),
                required: Some(true),
                format: None,
                items: None,
                file_arg: None,
                file_flag: None,
                alias: None,
            }]),
            positional_args_first: Some(false),
            timeout_seconds: timeout,
            synchronous: Some(matches!(
                execution_mode,
                crate::adapter::ExecutionMode::Synchronous
            )),
            enabled: true,
            guidance_key: None,
            sequence: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
        }
    }

    pub async fn execute_shell_sync(
        &self,
        adapter_args: Map<String, Value>,
        working_directory: &str,
        timeout: Option<u64>,
        subcommand_config: &crate::config::SubcommandConfig,
        context: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let id = format!("op_{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
        let progress_token = context.meta.get_progress_token();
        let client_type = McpClientType::from_peer(&context.peer);
        let description = format!(
            "Execute {} in {}",
            platform_shell_program(),
            working_directory
        );

        if let Some(token) = progress_token.clone() {
            let callback =
                McpCallbackSender::new(context.peer.clone(), id.clone(), Some(token), client_type);
            let _ = callback
                .send_progress(crate::callback_system::ProgressUpdate::Started {
                    id: id.clone(),
                    command: platform_shell_program().to_string(),
                    description: description.clone(),
                })
                .await;
        }

        let result = self
            .adapter
            .execute_sync_in_dir(
                platform_shell_program(),
                Some(adapter_args),
                working_directory,
                timeout,
                Some(subcommand_config),
            )
            .await;

        if let Some(token) = progress_token {
            let callback =
                McpCallbackSender::new(context.peer.clone(), id.clone(), Some(token), client_type);
            let (success, full_output) = match &result {
                Ok(output) => (true, output.clone()),
                Err(e) => (false, format!("Error: {}", e)),
            };
            let _ = callback
                .send_progress(crate::callback_system::ProgressUpdate::FinalResult {
                    id: id.clone(),
                    command: platform_shell_program().to_string(),
                    description,
                    working_directory: working_directory.to_string(),
                    success,
                    duration_ms: 0,
                    full_output,
                })
                .await;
        }

        match result {
            Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
            Err(e) => {
                let error_message = format!("Synchronous execution failed: {}", e);
                tracing::error!("{}", error_message);
                Err(McpError::internal_error(error_message, None))
            }
        }
    }

    pub async fn execute_shell_async(
        &self,
        adapter_args: Map<String, Value>,
        working_directory: &str,
        timeout: Option<u64>,
        subcommand_config: &crate::config::SubcommandConfig,
        context: &RequestContext<RoleServer>,
        log_monitor_config: Option<crate::log_monitor::LogMonitorConfig>,
    ) -> Result<CallToolResult, McpError> {
        let id = format!("op_{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
        let progress_token = context.meta.get_progress_token();
        let client_type = McpClientType::from_peer(&context.peer);
        let callback: Option<Box<dyn CallbackSender>> = progress_token.map(|token| {
            Box::new(McpCallbackSender::new(
                context.peer.clone(),
                id.clone(),
                Some(token),
                client_type,
            )) as Box<dyn CallbackSender>
        });

        let job_id = self
            .adapter
            .execute_async_in_dir_with_options(
                "sandboxed_shell",
                platform_shell_program(),
                working_directory,
                crate::adapter::AsyncExecOptions {
                    id: Some(id),
                    args: Some(adapter_args),
                    timeout,
                    callback,
                    subcommand_config: Some(subcommand_config),
                    log_monitor_config,
                },
            )
            .await;

        match job_id {
            Ok(id) => {
                // Automatic async: wait briefly for fast commands to complete
                if let Some(result) =
                    super::common::try_automatic_async_completion(&self.operation_monitor, &id)
                        .await
                {
                    return Ok(result);
                }

                let hint = crate::tool_hints::preview(&id, "sandboxed_shell");
                let message = format!("Asynchronous operation started with ID: {}{}", id, hint);
                Ok(CallToolResult::success(vec![Content::text(message)]))
            }
            Err(e) => {
                let error_message = format!("Async execution failed: {}", e);
                tracing::error!("{}", error_message);
                Err(McpError::internal_error(error_message, None))
            }
        }
    }
}
