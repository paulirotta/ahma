//! # Ahma MCP Service: The Protocol Layer
//!
//! This module implements the "brain" of the Ahma server. The [`AhmaMcpService`]
//! is responsible for managing the full lifecycle of the Model Context Protocol (MCP),
//! from initial handshake and tool discovery to execution routing and session isolation.
//!
//! ## Protocol Lifecycle
//!
//! The service coordinates several critical phases of an MCP session:
//!
//! 1. **Handshake (`initialize`)**: Establishes the connection and identifies client
//!    capabilities.
//! 2. **Sandbox Anchoring (`roots/list`)**: In HTTP bridge or session-isolated modes,
//!    the server queries the client for workspace roots to dynamically configure the
//!    security sandbox for that specific session.
//! 3. **Tool Discovery (`list_tools`)**: Dynamically transforms MTDF JSON configurations
//!    and bundled capability flags (like `--rust` or `--git`) into a rich set of
//!    tools that the AI can understand and call.
//! 4. **Execution Routing (`call_tool`)**: Validates incoming arguments against the
//!    tool's JSON schema and routes the execution request to the [`Adapter`].
//!
//! ## Async-First Philosophy
//!
//! Ahma is designed for agents performing complex, multi-threaded work. Most tool calls
//! follow an **Async-Result-Push** pattern:
//! - **Immediate Response**: The server returns an operation ID (e.g., `op_123`)
//!   instantly, allowing the agent to continue working or call other tools.
//! - **Background Execution**: The task runs in the background, governed by the
//!   [`OperationMonitor`](crate::operation_monitor::OperationMonitor).
//! - **Progressive Feedback**: Status updates and final results are pushed back to the
//!   client via standard MCP notifications as they happen.
//!
//! ## Built-in Core Tools
//!
//! The service always exposes a set of "Internal Tools" (`await`, `status`,
//! `cancel`, and `sandboxed_shell`) that provide essential primitives for managing
//! background tasks and executing arbitrary logic within the sandbox.

pub mod bundle_registry;
mod config_watcher;
mod handlers;
mod schema;
mod sequence;
mod subcommand;
mod types;
mod utils;

pub use types::{GuidanceConfig, LegacyGuidanceConfig, META_PARAMS, SequenceKind};

use rmcp::{
    handler::server::ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, CancelledNotificationParam, ErrorData as McpError,
        Implementation, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
        ServerCapabilities, ServerInfo, Tool, ToolsCapability,
    },
    service::{NotificationContext, Peer, RequestContext, RoleServer},
};
use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicU64, Ordering},
};
use tracing;

use crate::{
    adapter::Adapter, callback_system::CallbackSender, client_type::McpClientType,
    config::ToolConfig, mcp_callback::McpCallbackSender,
};

pub(crate) static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// `AhmaMcpService` is the server handler for the MCP service.
#[derive(Clone)]
pub struct AhmaMcpService {
    pub adapter: Arc<Adapter>,
    pub operation_monitor: Arc<crate::operation_monitor::OperationMonitor>,
    pub configs: Arc<RwLock<HashMap<String, ToolConfig>>>,
    pub guidance: Arc<Option<GuidanceConfig>>,
    /// When true, forces all operations to run synchronously (overrides async-by-default).
    /// This is set when the --sync CLI flag is used.
    pub force_synchronous: bool,
    /// When true, sandbox initialization is deferred until roots/list_changed notification.
    /// This is used in HTTP bridge mode where SSE must connect before server→client requests.
    pub defer_sandbox: bool,
    /// The peer handle for sending notifications to the client.
    /// This is populated by capturing it from the first request context.
    pub peer: Arc<RwLock<Option<Peer<RoleServer>>>>,
    /// Minimum seconds between successive log monitoring alerts (default: 60).
    pub monitor_rate_limit_seconds: u64,
    /// When true, only built-in tools and `activate_tools` are shown initially.
    /// Bundled tools are revealed on demand via `activate_tools reveal <bundle>`.
    pub progressive_disclosure: bool,
    /// Set of bundle names whose tools have been disclosed to the client.
    pub disclosed_bundles: Arc<RwLock<HashSet<String>>>,
}

impl AhmaMcpService {
    /// Creates a new `AhmaMcpService` instance.
    ///
    /// This service implements the `rmcp::ServerHandler` trait and manages tool execution
    /// via the provided `Adapter`.
    ///
    /// # Arguments
    ///
    /// * `adapter` - The tool execution engine.
    /// * `operation_monitor` - Monitor for tracking background task progress.
    /// * `configs` - Map of loaded tool configurations.
    /// * `guidance` - Optional guidance configuration for AI usage hints.
    /// * `force_synchronous` - If true, overrides async defaults (e.g., for debugging).
    /// * `defer_sandbox` - If true, delays sandbox initialization (for HTTP bridge scenarios).
    /// * `progressive_disclosure` - If true, only built-in + activate_tools shown initially.
    pub async fn new(
        adapter: Arc<Adapter>,
        operation_monitor: Arc<crate::operation_monitor::OperationMonitor>,
        configs: Arc<HashMap<String, ToolConfig>>,
        guidance: Arc<Option<GuidanceConfig>>,
        force_synchronous: bool,
        defer_sandbox: bool,
        progressive_disclosure: bool,
    ) -> Result<Self, anyhow::Error> {
        // Start the background monitor for operation timeouts
        crate::operation_monitor::OperationMonitor::start_background_monitor(
            operation_monitor.clone(),
        );

        Ok(Self {
            adapter,
            operation_monitor,
            configs: Arc::new(RwLock::new((*configs).clone())),
            guidance,
            force_synchronous,
            defer_sandbox,
            peer: Arc::new(RwLock::new(None)),
            monitor_rate_limit_seconds: crate::log_monitor::DEFAULT_RATE_LIMIT_SECONDS,
            progressive_disclosure,
            disclosed_bundles: Arc::new(RwLock::new(HashSet::new())),
        })
    }

    /// Pre-discloses the given bundle names so their tools appear in the first
    /// `tools/list` response without requiring an `activate_tools reveal` call.
    ///
    /// Used for bundles explicitly requested via CLI flags (e.g. `--rust`).
    pub fn pre_disclose(&self, bundles: &std::collections::HashSet<String>) {
        if bundles.is_empty() {
            return;
        }
        let mut disclosed = self.disclosed_bundles.write().unwrap();
        for name in bundles {
            disclosed.insert(name.clone());
        }
        tracing::info!(
            "Auto-revealed CLI-flagged bundles: {}",
            bundles.iter().cloned().collect::<Vec<_>>().join(", ")
        );
    }

    /// Creates MCP Tools from a ToolConfig.
    ///
    /// If the tool has subcommands, returns one flattened Tool per leaf subcommand
    /// (e.g., `"file-tools_hello"`, `"file-tools_world"`). If there are no
    /// subcommands (or only a single `"default"` one), returns a single Tool
    /// with the original config name.
    fn create_tools_from_config(&self, tool_config: &ToolConfig) -> Vec<Tool> {
        let base_name = &tool_config.name;

        let mut leaf_subcommands = Vec::new();
        if let Some(subcommands) = &tool_config.subcommand {
            schema::collect_leaf_subcommands(subcommands, "", &mut leaf_subcommands);
        }

        // Single tool without subcommands, or a single "default" subcommand
        let is_single_default = match leaf_subcommands.as_slice() {
            [] => true,
            [(name, _)] if name == "default" => true,
            _ => false,
        };

        if is_single_default {
            let description = self.tool_description(tool_config, base_name);
            let input_schema =
                schema::generate_schema_for_tool_config(tool_config, self.guidance.as_ref());
            return vec![
                Tool::new(base_name.clone(), description, input_schema)
                    .with_title(base_name.clone()),
            ];
        }

        // Multiple subcommands → flatten into one Tool per leaf
        leaf_subcommands
            .iter()
            .map(|(sub_path, sub_config)| {
                let flat_name = format!("{}_{}", base_name, sub_path);
                let sub_description = if sub_config.description.is_empty() {
                    tool_config.description.clone()
                } else {
                    sub_config.description.clone()
                };
                let description =
                    self.tool_description_text(tool_config, &flat_name, &sub_description);
                let input_schema = Arc::new(schema::generate_single_command_schema_pub(
                    tool_config,
                    &(sub_path.clone(), *sub_config),
                ));
                Tool::new(flat_name.clone(), description, input_schema).with_title(flat_name)
            })
            .collect()
    }

    /// Resolves guidance-augmented description for a tool config by key.
    fn tool_description(&self, tool_config: &ToolConfig, key: &str) -> String {
        let mut description = tool_config.description.clone();
        if let Some(guidance_config) = self.guidance.as_ref() {
            let default_key = key.to_string();
            let gk = tool_config.guidance_key.as_ref().unwrap_or(&default_key);
            if let Some(guidance_text) = guidance_config.guidance_blocks.get(gk) {
                description = format!("{}\n\n{}", guidance_text, description);
            }
        }
        description
    }

    /// Builds a guidance-augmented description from explicit text.
    fn tool_description_text(&self, tool_config: &ToolConfig, key: &str, base: &str) -> String {
        let mut description = base.to_string();
        if let Some(guidance_config) = self.guidance.as_ref() {
            let default_key = key.to_string();
            let gk = tool_config.guidance_key.as_ref().unwrap_or(&default_key);
            if let Some(guidance_text) = guidance_config.guidance_blocks.get(gk) {
                description = format!("{}\n\n{}", guidance_text, description);
            }
        }
        description
    }

    /// Resolves a flattened tool name (e.g., `"file-tools_hello"`) to a parent
    /// config and the subcommand path portion. Tries every possible split position
    /// of `_` from left to right so that tool names containing underscores still
    /// work correctly (the config key match is authoritative).
    fn resolve_flattened_tool<'a>(
        tool_name: &str,
        configs: &'a HashMap<String, ToolConfig>,
    ) -> Option<(&'a ToolConfig, String)> {
        // Try splitting at each '_' from left to right
        for (idx, _) in tool_name.match_indices('_') {
            let parent = &tool_name[..idx];
            let sub_path = &tool_name[idx + 1..];
            if !sub_path.is_empty()
                && let Some(config) = configs.get(parent).filter(|c| c.subcommand.is_some())
            {
                return Some((config, sub_path.to_string()));
            }
        }
        None
    }

    /// Sends a `notifications/tools/list_changed` notification to the connected client.
    ///
    /// Called after bundle disclosure state changes (e.g., via `activate_tools reveal`).
    pub async fn notify_tools_changed(&self) {
        let peer_opt = {
            let peer_lock = self.peer.read().unwrap();
            peer_lock.clone()
        };

        if let Some(peer) = peer_opt {
            if let Err(e) = peer.notify_tool_list_changed().await {
                tracing::error!("Failed to send tools/list_changed notification: {}", e);
            } else {
                tracing::info!("Sent tools/list_changed notification after bundle reveal");
            }
        } else {
            tracing::debug!("No peer connected, skipping tools/list_changed notification");
        }
    }

    /// Returns true if the given tool config name belongs to a known bundle.
    fn is_bundle_tool(&self, config_name: &str) -> bool {
        bundle_registry::BUNDLES
            .iter()
            .any(|b| b.config_tool_name == config_name)
    }

    /// Returns true if the tool's parent bundle has been disclosed.
    fn is_tool_disclosed(&self, config_name: &str, disclosed: &HashSet<String>) -> bool {
        bundle_registry::BUNDLES
            .iter()
            .find(|b| b.config_tool_name == config_name)
            .is_some_and(|b| disclosed.contains(b.name))
    }

    /// Generates a rich, action-oriented description for the `activate_tools` meta-tool.
    ///
    /// The description dynamically lists all loaded bundles with their `ai_hint` text,
    /// giving the AI immediate awareness of what capabilities are available and when
    /// to activate each one.
    fn generate_activate_tools_description(&self) -> String {
        let config_keys: std::collections::HashSet<String> = {
            let configs_lock = self.configs.read().unwrap();
            configs_lock.keys().cloned().collect()
        };

        let loaded = bundle_registry::loaded_bundle_names(&config_keys);

        if loaded.is_empty() {
            return "Discover and activate tool bundles. Call with action 'list' to see available bundles and their status, or 'reveal' with a bundle name to activate its tools. Bundles are revealed progressively to minimize context usage.".to_string();
        }

        let mut parts = Vec::new();
        parts.push("Activate additional tool bundles to extend your capabilities. You MUST call this to use specialized tools. Available bundles:".to_string());

        for bundle in &loaded {
            parts.push(format!("- '{}': {}", bundle.name, bundle.ai_hint));
        }

        parts.push(
            "Call with action 'list' for details, or 'reveal' with a bundle name to activate."
                .to_string(),
        );

        parts.join("\n")
    }
}

#[async_trait::async_trait]
#[allow(clippy::manual_async_fn)] // Required by rmcp ServerHandler trait
impl ServerHandler for AhmaMcpService {
    fn get_info(&self) -> ServerInfo {
        let instructions = if self.progressive_disclosure {
            Some("MANDATORY EXECUTION DIRECTIVE: This server provides the strictly required execution environment for ALL terminal, shell, and command-line operations. \
                  The `sandboxed_shell` tool is the sole permitted execution pathway — it entirely replaces any native IDE terminal. \
                  Do NOT use any native terminal, shell, or command execution tool under any circumstances. \
                  Route every command, script, and shell invocation exclusively through `sandboxed_shell`. \
                  Tool bundles are progressively disclosed to conserve context. \
                  Call `activate_tools` with action 'list' to see available bundles, then 'reveal' to activate what you need.".to_string())
        } else {
            Some("MANDATORY EXECUTION DIRECTIVE: This server provides the strictly required execution environment for ALL terminal, shell, and command-line operations. \
                  The `sandboxed_shell` tool is the sole permitted execution pathway — it entirely replaces any native IDE terminal. \
                  Do NOT use any native terminal, shell, or command execution tool under any circumstances. \
                  Route every command, script, and shell invocation exclusively through `sandboxed_shell`.".to_string())
        };

        let capabilities = ServerCapabilities::builder()
            .enable_tools_with(ToolsCapability {
                list_changed: Some(true),
            })
            .build();

        let server_info = Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
            .with_title(env!("CARGO_PKG_NAME"));

        let info = ServerInfo::new(capabilities)
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_server_info(server_info);

        if let Some(instructions) = instructions {
            info.with_instructions(instructions)
        } else {
            info
        }
    }

    fn on_initialized(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        async move {
            tracing::info!("Client connected: {context:?}");

            // Detect and log client type for debugging
            let client_type = McpClientType::from_peer(&context.peer);
            tracing::info!(
                "Detected MCP client type: {} (progress notifications: {})",
                client_type.display_name(),
                if client_type.supports_progress() {
                    "enabled"
                } else {
                    "disabled"
                }
            );

            // Get the peer from the context
            let peer = &context.peer;
            if self.peer.read().unwrap().is_none() {
                let mut peer_guard = self.peer.write().unwrap();
                if peer_guard.is_none() {
                    *peer_guard = Some(peer.clone());
                    tracing::info!(
                        "Successfully captured MCP peer handle for async notifications."
                    );
                }
            }

            // Query client for workspace roots and configure sandbox
            // Per MCP spec, server sends roots/list request to client
            // IMPORTANT: Only do this if sandbox is NOT deferred.
            // In HTTP bridge mode with --defer-sandbox, we wait for roots/list_changed
            // notification which is sent by the bridge when SSE connects.
            if !self.defer_sandbox {
                // IF scopes are already configured (e.g. via CLI --sandbox-scope), respect them
                // and do not ask client for roots (which would overwrite CLI scopes).
                // This also prevents hangs when testing with clients that don't support roots/list.
                if !self.adapter.sandbox().scopes().is_empty() {
                    tracing::info!(
                        "Sandbox scopes already configured via CLI/Env ({:?}), skipping roots/list request",
                        self.adapter.sandbox().scopes()
                    );
                } else {
                    // Run synchronously per R19.3 - sandbox configuration is a lifecycle
                    // operation that should complete before we're "ready"
                    self.configure_sandbox_from_roots(peer).await;
                }
            } else {
                tracing::info!("Sandbox deferred - waiting for roots/list_changed notification");
            }
        }
    }

    fn on_roots_list_changed(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        async move {
            tracing::info!("Received roots/list_changed notification");

            // This notification is sent by the HTTP bridge when SSE connects.
            // It signals that we can now safely call roots/list.
            let peer = &context.peer;

            // Run synchronously per R19.3 - sandbox configuration must complete
            // before we can safely process tools/call requests. Initial handshake
            // timing is not super critical, but correctness is.
            self.configure_sandbox_from_roots(peer).await;
        }
    }

    fn on_cancelled(
        &self,
        notification: CancelledNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        async move {
            let request_id = format!("{:?}", notification.request_id);
            let reason = notification
                .reason
                .as_deref()
                .unwrap_or("Client-initiated cancellation");

            tracing::info!(
                "MCP protocol cancellation received: request_id={}, reason='{}'",
                request_id,
                reason
            );

            // CRITICAL FIX: Only cancel background operations, not synchronous MCP calls
            // This prevents the rmcp library from generating "Canceled: Canceled" messages
            // that get incorrectly processed as process cancellations.

            let active_ops = self.operation_monitor.get_all_active_operations().await;
            let active_count = active_ops.len();

            if active_count > 0 {
                // Filter for operations that are actually background processes
                // vs. synchronous MCP tools like 'await' that don't have processes
                let background_ops: Vec<_> = active_ops
                    .iter()
                    .filter(|op| {
                        // Only cancel operations that represent actual background processes
                        // NOT synchronous tools like 'await', 'status', 'cancel'
                        !matches!(op.tool_name.as_str(), "await" | "status" | "cancel")
                    })
                    .collect();

                if !background_ops.is_empty() {
                    tracing::info!(
                        "Found {} background operations during MCP cancellation. Cancelling most recent background operation...",
                        background_ops.len()
                    );

                    if let Some(most_recent_bg_op) = background_ops.last() {
                        let enhanced_reason = format!(
                            "MCP protocol cancellation (request_id: {}, reason: '{}')",
                            request_id, reason
                        );

                        let cancelled = self
                            .operation_monitor
                            .cancel_operation_with_reason(
                                &most_recent_bg_op.id,
                                Some(enhanced_reason.clone()),
                            )
                            .await;

                        if cancelled {
                            tracing::info!(
                                "Successfully cancelled background operation '{}' due to MCP protocol cancellation: {}",
                                most_recent_bg_op.id,
                                enhanced_reason
                            );
                        } else {
                            tracing::warn!(
                                "Failed to cancel background operation '{}' for MCP protocol cancellation",
                                most_recent_bg_op.id
                            );
                        }
                    }
                } else {
                    tracing::info!(
                        "Found {} operations during MCP cancellation, but none are background processes. No cancellation needed.",
                        active_count
                    );
                }
            } else {
                tracing::info!(
                    "No active operations found during MCP protocol cancellation (request_id: {})",
                    request_id
                );
            }
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        async move {
            let mut tools = Vec::new();

            // Hard-wired await command - always available
            tools.push(
                Tool::new(
                    "await",
                    "Wait for previously started asynchronous operations to complete. **WARNING:** This is a blocking tool and makes you inefficient. **ONLY** use this if you have NO other tasks and cannot proceed until completion. It is **ALWAYS** better to perform other work and let results be pushed to you. **IMPORTANT:** Operations automatically notify you when complete - you do NOT need to check status repeatedly. Use this tool only when you genuinely cannot make progress without the results.",
                    self.generate_input_schema_for_wait(),
                )
                .with_title("await"),
            );

            // Hard-wired status command - always available
            tools.push(
                Tool::new(
                    "status",
                    "Query the status of operations without blocking. Shows active and completed operations. **IMPORTANT:** Results are automatically pushed to you when operations complete - you do NOT need to poll this tool repeatedly! If you find yourself calling 'status' multiple times for the same operation, you should use 'await' instead. Repeated status checks are an anti-pattern that wastes resources.",
                    self.generate_input_schema_for_status(),
                )
                .with_title("status"),
            );

            // Hard-wired sandboxed_shell command - always available
            tools.push(
                Tool::new(
                    "sandboxed_shell",
                    "The default, required execution environment for ALL shell, terminal, and command-line operations. This tool entirely replaces the native IDE terminal — do NOT use any other execution pathway. Supports pipes, redirects, environment variables, and full shell syntax. Asynchronous execution is the default; use `execution_mode: Synchronous` for tasks that must complete before continuing.",
                    self.generate_input_schema_for_sandboxed_shell(),
                )
                .with_title("sandboxed_shell"),
            );

            // When progressive disclosure is enabled, expose the activate_tools meta-tool
            // with a dynamically generated description listing all loaded bundles
            if self.progressive_disclosure {
                let description = self.generate_activate_tools_description();
                tools.push(
                    Tool::new(
                        "activate_tools",
                        description,
                        self.generate_input_schema_for_discover_tools(),
                    )
                    .with_title("activate_tools"),
                );
            }

            {
                // Reserved names are already hard-wired above; skip them from
                // user/bundled configs to avoid duplicates in `tools/list`.
                const HARDCODED_TOOLS: &[&str] = &[
                    "await",
                    "status",
                    "sandboxed_shell",
                    "cancel",
                    "activate_tools",
                ];
                let disclosed = if self.progressive_disclosure {
                    Some(self.disclosed_bundles.read().unwrap().clone())
                } else {
                    None
                };
                let configs_lock = self.configs.read().unwrap();
                for config in configs_lock.values() {
                    if HARDCODED_TOOLS.contains(&config.name.as_str()) {
                        continue;
                    }
                    if !config.enabled {
                        tracing::debug!(
                            "Skipping disabled tool '{}' during list_tools",
                            config.name
                        );
                        continue;
                    }

                    // Progressive disclosure: only show tools whose bundle has been revealed
                    if let Some(ref disclosed_set) = disclosed
                        && self.is_bundle_tool(&config.name)
                        && !self.is_tool_disclosed(&config.name, disclosed_set)
                    {
                        tracing::debug!(
                            "Skipping undisclosed bundle tool '{}' during list_tools",
                            config.name
                        );
                        continue;
                    }

                    let tools_from_config = self.create_tools_from_config(config);
                    tools.extend(tools_from_config);
                }
            }

            Ok(ListToolsResult {
                meta: None,
                tools,
                next_cursor: None,
            })
        }
    }

    fn call_tool(
        &self,
        params: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        async move {
            let tool_name = params.name.as_ref();

            if tool_name == "status" {
                return self
                    .handle_status(params.arguments.unwrap_or_default())
                    .await;
            }

            if tool_name == "await" {
                return self.handle_await(params).await;
            }

            if tool_name == "sandboxed_shell" {
                return self.handle_sandboxed_shell(params, context).await;
            }

            if tool_name == "cancel" {
                return self
                    .handle_cancel(params.arguments.unwrap_or_default())
                    .await;
            }

            if tool_name == "activate_tools" {
                return self
                    .handle_discover_tools(params.arguments.unwrap_or_default())
                    .await;
            }

            // Delay tool execution until sandbox is initialized from roots/list.
            // This is critical in HTTP bridge mode with deferred sandbox initialization.
            if !self.adapter.sandbox().is_ready_for_tool_calls() {
                let error_message = "Sandbox initializing from client roots - retry tools/call after roots/list completes".to_string();
                tracing::warn!("{}", error_message);
                return Err(handlers::common::mcp_internal(error_message));
            }

            // Find tool configuration
            // Acquire read lock for configs, clone the config, and drop the lock immediately.
            // If the tool name contains '_', it may be a flattened subcommand name
            // (e.g. "file-tools_hello" → parent "file-tools", subcommand "hello").
            let (config, flattened_subcommand) = {
                let configs_lock = self.configs.read().unwrap();
                if let Some(config) = configs_lock.get(tool_name) {
                    (config.clone(), None)
                } else if let Some((parent, sub_path)) =
                    Self::resolve_flattened_tool(tool_name, &configs_lock)
                {
                    (parent.clone(), Some(sub_path))
                } else {
                    let error_message = format!("Tool '{}' not found.", tool_name);
                    tracing::error!("{}", error_message);
                    return Err(McpError::invalid_params(
                        error_message,
                        Some(serde_json::json!({ "tool_name": tool_name })),
                    ));
                }
            };

            if !config.enabled {
                let error_message = format!(
                    "Tool '{}' is unavailable because its runtime availability probe failed",
                    tool_name
                );
                tracing::error!("{}", error_message);
                return Err(McpError::invalid_request(error_message, None));
            }

            // Check if this is a sequence tool
            if config.sequence.is_some() {
                return sequence::handle_sequence_tool(
                    &self.adapter,
                    &self.operation_monitor,
                    &self.configs,
                    &config,
                    params,
                    context,
                )
                .await;
            }

            // Check if this is a livelog tool (long-running LLM-monitored log source)
            if config.tool_type == Some(crate::config::ToolType::Livelog) {
                let params_map = params.arguments.clone().unwrap_or_default();
                let op_id = format!("livelog_{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
                let progress_token = context.meta.get_progress_token();
                let client_type = McpClientType::from_peer(&context.peer);
                let callback: Option<Box<dyn CallbackSender>> = progress_token.map(|token| {
                    Box::new(McpCallbackSender::new(
                        context.peer.clone(),
                        op_id.clone(),
                        Some(token),
                        client_type,
                    )) as Box<dyn CallbackSender>
                });
                return match handlers::livelog_tool::handle_livelog_start(
                    op_id.clone(),
                    &config,
                    &params_map,
                    self.operation_monitor.clone(),
                    self.adapter.sandbox_arc(),
                    callback,
                )
                .await
                {
                    Ok(started_id) => Ok(handlers::common::text_result(format!(
                        "Live log monitoring started. Operation ID: {started_id}\n\
                         Use `status` or `await` to check progress, `cancel` to stop."
                    ))),
                    Err(e) => {
                        let msg = format!("Failed to start livelog '{}': {}", config.name, e);
                        tracing::error!("{}", msg);
                        Err(handlers::common::mcp_internal(msg))
                    }
                };
            }

            let mut arguments = params.arguments.clone().unwrap_or_default();
            // Use flattened subcommand path if resolved, otherwise use the "subcommand" argument
            let subcommand_name = flattened_subcommand.or_else(|| {
                arguments
                    .remove("subcommand")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            });

            // Find the subcommand config and construct the command parts
            let (subcommand_config, command_parts) =
                match subcommand::find_subcommand_config_from_args(&config, subcommand_name.clone())
                {
                    Some(result) => result,
                    None => {
                        let has_subcommands = config.subcommand.is_some();
                        let num_subcommands =
                            config.subcommand.as_ref().map(|s| s.len()).unwrap_or(0);
                        let subcommand_names: Vec<String> = config
                            .subcommand
                            .as_ref()
                            .map(|subs| {
                                subs.iter()
                                    .map(|s| format!("{} (enabled={})", s.name, s.enabled))
                                    .collect()
                            })
                            .unwrap_or_default();

                        let error_message = format!(
                            "Subcommand '{:?}' for tool '{}' not found or invalid. Tool enabled={}, has_subcommands={}, num_subcommands={}, available_subcommands={:?}",
                            subcommand_name,
                            tool_name,
                            config.enabled,
                            has_subcommands,
                            num_subcommands,
                            subcommand_names
                        );
                        tracing::error!("{}", error_message);
                        return Err(McpError::invalid_params(
                            error_message,
                            Some(
                                serde_json::json!({ "tool_name": tool_name, "subcommand": subcommand_name }),
                            ),
                        ));
                    }
                };

            // Check if the subcommand itself is a sequence
            if subcommand_config.sequence.is_some() {
                return sequence::handle_subcommand_sequence(
                    &self.adapter,
                    &config,
                    subcommand_config,
                    params,
                    context,
                )
                .await;
            }

            let base_command = command_parts.join(" ");

            let working_directory = arguments
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
                            .map(|p| p.to_string_lossy().to_string())
                    }
                })
                .unwrap_or_else(|| ".".to_string());

            let timeout = arguments.get("timeout_seconds").and_then(|v| v.as_u64());

            // Determine execution mode (default is ASYNCHRONOUS):
            // 1. If synchronous=true in config (subcommand or inherited from tool), ALWAYS use sync
            // 2. If synchronous=false in config, ALWAYS use async (explicit async override)
            // 3. If --sync CLI flag was used (force_synchronous=true), use sync mode
            // 4. Check explicit execution_mode argument (for advanced use)
            // 5. Default to ASYNCHRONOUS
            //
            // Inheritance: subcommand.synchronous overrides tool.synchronous
            // If subcommand doesn't specify, inherit from tool level
            let sync_override = subcommand_config.synchronous.or(config.synchronous);
            let execution_mode = if sync_override == Some(true) {
                // Config explicitly requires synchronous: FORCE sync mode
                crate::adapter::ExecutionMode::Synchronous
            } else if sync_override == Some(false) {
                // Config explicitly requires async: FORCE async mode (ignores --sync flag)
                crate::adapter::ExecutionMode::AsyncResultPush
            } else if self.force_synchronous {
                // --sync flag was used and not overridden by config: use sync mode
                crate::adapter::ExecutionMode::Synchronous
            } else if let Some(mode_str) = arguments.get("execution_mode").and_then(|v| v.as_str())
            {
                match mode_str {
                    "Synchronous" => crate::adapter::ExecutionMode::Synchronous,
                    "AsyncResultPush" => crate::adapter::ExecutionMode::AsyncResultPush,
                    _ => crate::adapter::ExecutionMode::AsyncResultPush, // Default to async
                }
            } else {
                // Default to ASYNCHRONOUS mode
                crate::adapter::ExecutionMode::AsyncResultPush
            };

            match execution_mode {
                crate::adapter::ExecutionMode::Synchronous => {
                    let id = format!("op_{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
                    let progress_token = context.meta.get_progress_token();
                    let client_type = McpClientType::from_peer(&context.peer);

                    // Send 'Started' notification if progress token is present
                    if let Some(token) = progress_token.clone() {
                        let callback = McpCallbackSender::new(
                            context.peer.clone(),
                            id.clone(),
                            Some(token),
                            client_type,
                        );
                        let _ = callback
                            .send_progress(crate::callback_system::ProgressUpdate::Started {
                                id: id.clone(),
                                command: base_command.clone(),
                                description: format!(
                                    "Execute {} in {}",
                                    base_command, working_directory
                                ),
                            })
                            .await;
                    }

                    let result = self
                        .adapter
                        .execute_sync_in_dir(
                            &base_command,
                            Some(arguments),
                            &working_directory,
                            timeout,
                            Some(subcommand_config),
                        )
                        .await;

                    // Send completion notification if progress token is present
                    if let Some(token) = progress_token {
                        let callback = McpCallbackSender::new(
                            context.peer.clone(),
                            id.clone(),
                            Some(token),
                            client_type,
                        );
                        match &result {
                            Ok(output) => {
                                let _ = callback
                                    .send_progress(
                                        crate::callback_system::ProgressUpdate::FinalResult {
                                            id: id.clone(),
                                            command: base_command.clone(),
                                            description: format!(
                                                "Execute {} in {}",
                                                base_command, working_directory
                                            ),
                                            working_directory: working_directory.clone(),
                                            success: true,
                                            duration_ms: 0,
                                            full_output: output.clone(),
                                        },
                                    )
                                    .await;
                            }
                            Err(e) => {
                                let _ = callback
                                    .send_progress(
                                        crate::callback_system::ProgressUpdate::FinalResult {
                                            id: id.clone(),
                                            command: base_command.clone(),
                                            description: format!(
                                                "Execute {} in {}",
                                                base_command, working_directory
                                            ),
                                            working_directory: working_directory.clone(),
                                            success: false,
                                            duration_ms: 0,
                                            full_output: format!("Error: {}", e),
                                        },
                                    )
                                    .await;
                            }
                        }
                    }

                    match result {
                        Ok(output) => Ok(handlers::common::text_result(output)),
                        Err(e) => {
                            let error_message = format!("Synchronous execution failed: {}", e);
                            tracing::error!("{}", error_message);
                            Err(handlers::common::mcp_internal(error_message))
                        }
                    }
                }
                crate::adapter::ExecutionMode::AsyncResultPush => {
                    let id = format!("op_{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
                    // Only send progress notifications when the client provided a progressToken
                    // in request `_meta`. Additionally, skip progress for clients that don't
                    // handle them well (e.g., Cursor logs errors for valid tokens).
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

                    // Build log monitor config from MTDF tool-level settings
                    let log_monitor_config = config.monitor_level.as_deref().map(|level_str| {
                        let level = level_str
                            .parse::<crate::log_monitor::LogLevel>()
                            .unwrap_or(crate::log_monitor::LogLevel::Error);
                        let stream = config
                            .monitor_stream
                            .as_deref()
                            .and_then(|s| s.parse::<crate::log_monitor::MonitorStream>().ok())
                            .unwrap_or(crate::log_monitor::MonitorStream::Stderr);
                        crate::log_monitor::LogMonitorConfig {
                            monitor_level: level,
                            monitor_stream: stream,
                            rate_limit_seconds: self.monitor_rate_limit_seconds,
                        }
                    });

                    let job_id = self
                        .adapter
                        .execute_async_in_dir_with_options(
                            tool_name,
                            &base_command,
                            &working_directory,
                            crate::adapter::AsyncExecOptions {
                                id: Some(id),
                                args: Some(arguments),
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
                            if let Some(result) = handlers::common::try_automatic_async_completion(
                                &self.operation_monitor,
                                &id,
                            )
                            .await
                            {
                                return Ok(result);
                            }

                            // Include tool hints to guide AI on handling async operations
                            let hint = crate::tool_hints::preview(&id, tool_name);
                            let message = format!("AHMA ID: {}{}", id, hint);
                            Ok(handlers::common::text_result(message))
                        }
                        Err(e) => {
                            let error_message =
                                format!("Failed to start asynchronous operation: {}", e);
                            tracing::error!("{}", error_message);
                            Err(handlers::common::mcp_internal(error_message))
                        }
                    }
                }
            }
        }
    }
}

impl AhmaMcpService {
    /// Returns the list of tool names that `list_tools()` would return,
    /// without requiring a `RequestContext`.
    ///
    /// This is useful for testing and introspection.
    pub fn list_tool_names(&self) -> Vec<String> {
        const HARDCODED_TOOLS: &[&str] = &[
            "await",
            "status",
            "sandboxed_shell",
            "cancel",
            "activate_tools",
        ];

        let mut names: Vec<String> =
            vec!["await".into(), "status".into(), "sandboxed_shell".into()];

        if self.progressive_disclosure {
            names.push("activate_tools".into());
        }

        let disclosed = if self.progressive_disclosure {
            Some(self.disclosed_bundles.read().unwrap().clone())
        } else {
            None
        };

        let configs_lock = self.configs.read().unwrap();
        for config in configs_lock.values() {
            if HARDCODED_TOOLS.contains(&config.name.as_str()) {
                continue;
            }
            if !config.enabled {
                continue;
            }
            if let Some(ref disclosed_set) = disclosed
                && self.is_bundle_tool(&config.name)
                && !self.is_tool_disclosed(&config.name, disclosed_set)
            {
                continue;
            }
            names.push(config.name.clone());
        }
        names
    }
}

#[cfg(test)]
mod tests {
    // ==================== force_synchronous inheritance tests ====================

    use super::*;
    use crate::config::{SubcommandConfig, ToolConfig, ToolHints};
    use crate::operation_monitor::{MonitorConfig, Operation, OperationMonitor, OperationStatus};

    use serde_json::json;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    async fn make_service_with_monitor(
        monitor: Arc<OperationMonitor>,
        guidance: Arc<Option<GuidanceConfig>>,
    ) -> AhmaMcpService {
        // Adapter is required by the service but not used by these unit tests.
        let adapter =
            crate::test_utils::client::create_test_config(Path::new(".")).expect("adapter");
        let configs: Arc<HashMap<String, ToolConfig>> = Arc::new(HashMap::new());
        AhmaMcpService::new(adapter, monitor, configs, guidance, false, false, false)
            .await
            .expect("service")
    }

    async fn make_service() -> AhmaMcpService {
        let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
            Duration::from_secs(30),
        )));
        make_service_with_monitor(monitor, Arc::new(None)).await
    }

    fn call_tool_params(name: &str, args: serde_json::Value) -> CallToolRequestParams {
        let mut params = CallToolRequestParams::new(name.to_string());
        if let Some(arguments) = args.as_object().cloned() {
            params = params.with_arguments(arguments);
        }
        params
    }

    fn first_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .find_map(|c| c.as_text().map(|t| t.text.clone()))
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn handle_status_empty_shows_zero_counts() {
        let service = make_service().await;
        let result = service
            .handle_status(serde_json::Map::new())
            .await
            .expect("status result");
        let text = first_text(&result);
        assert!(text.contains("Operations status:"));
        assert!(text.contains("0 active"));
        assert!(text.contains("0 completed"));
    }

    #[tokio::test]
    async fn handle_status_filters_by_tools_and_id() {
        let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
            Duration::from_secs(30),
        )));
        let service = make_service_with_monitor(monitor.clone(), Arc::new(None)).await;

        // Active operation
        let op_active = Operation::new(
            "op_active".to_string(),
            "alpha_tool".to_string(),
            "desc".to_string(),
            None,
        );
        monitor.add_operation(op_active).await;

        // Completed operation
        let op_completed = Operation::new(
            "op_completed".to_string(),
            "beta_tool".to_string(),
            "desc".to_string(),
            None,
        );
        monitor.add_operation(op_completed).await;
        monitor
            .update_status(
                "op_completed",
                OperationStatus::Completed,
                Some(json!({"ok": true})),
            )
            .await;

        // Filter by tool prefix
        let args = json!({"tools": "alpha"}).as_object().unwrap().clone();
        let result = service.handle_status(args).await.expect("status");
        let text = first_text(&result);
        assert!(text.contains("Operations status for 'alpha': 1 active, 0 completed"));
        assert!(
            result
                .content
                .iter()
                .filter_map(|c| c.as_text())
                .any(|t| t.text.contains("=== ACTIVE OPERATIONS ==="))
        );

        // Filter by specific operation id
        let args = json!({"id": "op_active"}).as_object().unwrap().clone();
        let result = service.handle_status(args).await.expect("status");
        let text = first_text(&result);
        assert!(text.contains("Operation 'op_active' found"));
    }

    #[tokio::test]
    async fn handle_cancel_requires_id() {
        let service = make_service().await;
        let err = service
            .handle_cancel(serde_json::Map::new())
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("id parameter is required"));
    }

    #[tokio::test]
    async fn handle_cancel_rejects_non_string_id() {
        let service = make_service().await;
        let args = json!({"id": 123}).as_object().unwrap().clone();
        let err = service.handle_cancel(args).await.unwrap_err();
        assert!(format!("{err:?}").contains("id must be a string"));
    }

    #[tokio::test]
    async fn handle_cancel_success_includes_hint_block() {
        let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
            Duration::from_secs(30),
        )));
        let service = make_service_with_monitor(monitor.clone(), Arc::new(None)).await;

        let op = Operation::new(
            "op_to_cancel".to_string(),
            "alpha_tool".to_string(),
            "desc".to_string(),
            None,
        );
        monitor.add_operation(op).await;

        let args = json!({"id": "op_to_cancel", "reason": "because"})
            .as_object()
            .unwrap()
            .clone();
        let result = service.handle_cancel(args).await.expect("cancel");
        let text = first_text(&result);
        assert!(text.contains("has been cancelled successfully"));
        assert!(text.contains("reason='because'"));
        assert!(
            result
                .content
                .iter()
                .filter_map(|c| c.as_text())
                .any(|t| t.text.contains("\"tool_hint\""))
        );
    }

    #[tokio::test]
    async fn handle_cancel_terminal_operation_reports_already_terminal() {
        let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
            Duration::from_secs(30),
        )));
        let service = make_service_with_monitor(monitor.clone(), Arc::new(None)).await;

        let mut op = Operation::new(
            "op_terminal".to_string(),
            "alpha_tool".to_string(),
            "desc".to_string(),
            None,
        );
        op.state = OperationStatus::Completed;
        monitor.add_operation(op).await;

        let args = json!({"id": "op_terminal"}).as_object().unwrap().clone();
        let result = service.handle_cancel(args).await.expect("cancel");
        let text = first_text(&result);
        assert!(text.contains("already completed"));
    }

    #[tokio::test]
    async fn handle_cancel_operation_not_found() {
        let service = make_service().await;
        let args = json!({"id": "op_nonexistent_xyz"})
            .as_object()
            .unwrap()
            .clone();
        let result = service.handle_cancel(args).await.expect("cancel");
        let text = first_text(&result);
        assert!(text.contains("not found"));
        assert!(text.contains("op_nonexistent_xyz"));
    }

    #[tokio::test]
    async fn handle_cancel_success_without_reason_uses_default_message() {
        let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
            Duration::from_secs(30),
        )));
        let service = make_service_with_monitor(monitor.clone(), Arc::new(None)).await;

        let op = Operation::new(
            "op_no_reason".to_string(),
            "test_tool".to_string(),
            "desc".to_string(),
            None,
        );
        monitor.add_operation(op).await;

        let args = json!({"id": "op_no_reason"}).as_object().unwrap().clone();
        let result = service.handle_cancel(args).await.expect("cancel");
        let text = first_text(&result);
        assert!(text.contains("has been cancelled successfully"));
        assert!(text.contains("No reason provided (default: user-initiated)"));
    }

    #[tokio::test]
    async fn handle_cancel_already_failed_reports_failed() {
        let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
            Duration::from_secs(30),
        )));
        let service = make_service_with_monitor(monitor.clone(), Arc::new(None)).await;

        let mut op = Operation::new(
            "op_failed".to_string(),
            "test_tool".to_string(),
            "desc".to_string(),
            None,
        );
        op.state = OperationStatus::Failed;
        monitor.add_operation(op).await;

        let args = json!({"id": "op_failed"}).as_object().unwrap().clone();
        let result = service.handle_cancel(args).await.expect("cancel");
        let text = first_text(&result);
        assert!(text.contains("already failed"));
    }

    #[tokio::test]
    async fn handle_cancel_already_cancelled_reports_cancelled() {
        let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
            Duration::from_secs(30),
        )));
        let service = make_service_with_monitor(monitor.clone(), Arc::new(None)).await;

        let mut op = Operation::new(
            "op_cancelled".to_string(),
            "test_tool".to_string(),
            "desc".to_string(),
            None,
        );
        op.state = OperationStatus::Cancelled;
        monitor.add_operation(op).await;

        let args = json!({"id": "op_cancelled"}).as_object().unwrap().clone();
        let result = service.handle_cancel(args).await.expect("cancel");
        let text = first_text(&result);
        assert!(text.contains("already cancelled"));
    }

    #[tokio::test]
    async fn handle_cancel_already_timed_out_reports_timed_out() {
        let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
            Duration::from_secs(30),
        )));
        let service = make_service_with_monitor(monitor.clone(), Arc::new(None)).await;

        let mut op = Operation::new(
            "op_timed_out".to_string(),
            "test_tool".to_string(),
            "desc".to_string(),
            None,
        );
        op.state = OperationStatus::TimedOut;
        monitor.add_operation(op).await;

        let args = json!({"id": "op_timed_out"}).as_object().unwrap().clone();
        let result = service.handle_cancel(args).await.expect("cancel");
        let text = first_text(&result);
        assert!(text.contains("timed out"));
    }

    #[test]
    fn parse_file_uri_to_path_accepts_localhost_and_decodes() {
        let p = AhmaMcpService::parse_file_uri_to_path(
            "file://localhost/Users/test/My%20Project/file.txt?x=1#frag",
        )
        .expect("path");
        assert_eq!(p.to_string_lossy(), "/Users/test/My Project/file.txt");
    }

    #[test]
    fn parse_file_uri_to_path_rejects_non_file_scheme_and_relative() {
        assert!(AhmaMcpService::parse_file_uri_to_path("http://example.com/a").is_none());
        assert!(AhmaMcpService::parse_file_uri_to_path("file://not-abs").is_none());
        assert!(AhmaMcpService::parse_file_uri_to_path("file://localhostnotabs").is_none());
    }

    #[test]
    fn parse_file_uri_to_path_accepts_absolute_without_localhost() {
        let p = AhmaMcpService::parse_file_uri_to_path("file:///home/user/file.txt").expect("path");
        assert_eq!(p.to_string_lossy(), "/home/user/file.txt");
    }

    #[test]
    fn parse_file_uri_to_path_strips_query_only() {
        let p =
            AhmaMcpService::parse_file_uri_to_path("file:///path/to/file?query=1").expect("path");
        assert_eq!(p.to_string_lossy(), "/path/to/file");
    }

    #[test]
    fn parse_file_uri_to_path_strips_fragment_only() {
        let p =
            AhmaMcpService::parse_file_uri_to_path("file:///path/to/file#section").expect("path");
        assert_eq!(p.to_string_lossy(), "/path/to/file");
    }

    #[test]
    fn percent_decode_utf8_rejects_invalid_hex() {
        assert!(AhmaMcpService::percent_decode_utf8("/a%ZZ").is_none());
        assert!(AhmaMcpService::percent_decode_utf8("/a%2").is_none());
    }

    #[test]
    fn percent_decode_utf8_decodes_space() {
        let decoded = AhmaMcpService::percent_decode_utf8("/path%20to%20file").expect("decode");
        assert_eq!(decoded, "/path to file");
    }

    #[test]
    fn percent_decode_utf8_preserves_plain_text() {
        let decoded = AhmaMcpService::percent_decode_utf8("/path/to/file").expect("decode");
        assert_eq!(decoded, "/path/to/file");
    }

    #[test]
    fn percent_decode_utf8_uppercase_hex() {
        let decoded = AhmaMcpService::percent_decode_utf8("path%2Ffile").expect("decode");
        assert_eq!(decoded, "path/file");
    }

    #[test]
    fn percent_decode_utf8_truncated_percent_at_end() {
        assert!(AhmaMcpService::percent_decode_utf8("/path%").is_none());
    }

    #[test]
    fn percent_decode_utf8_invalid_utf8_returns_none() {
        // %FF decodes to byte 0xFF which is invalid as standalone UTF-8
        assert!(AhmaMcpService::percent_decode_utf8("%FF").is_none());
    }

    #[test]
    fn percent_decode_utf8_empty_string() {
        let decoded = AhmaMcpService::percent_decode_utf8("").expect("decode");
        assert_eq!(decoded, "");
    }

    #[tokio::test]
    async fn handle_await_id_not_found_reports_not_found() {
        let service = make_service().await;
        let params = call_tool_params("await", json!({"id": "op_missing"}));
        let result = service.handle_await(params).await.expect("await result");
        assert!(first_text(&result).contains("Operation op_missing not found"));
    }

    #[tokio::test]
    async fn handle_await_id_in_history_reports_already_completed() {
        let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
            Duration::from_secs(30),
        )));
        let service = make_service_with_monitor(monitor.clone(), Arc::new(None)).await;

        let op_id = "op_done".to_string();
        let op = Operation::new(
            op_id.clone(),
            "demo_tool".to_string(),
            "desc".to_string(),
            None,
        );
        monitor.add_operation(op).await;
        monitor
            .update_status(
                &op_id,
                OperationStatus::Completed,
                Some(json!({"ok": true})),
            )
            .await;

        let params = call_tool_params("await", json!({"id": op_id}));
        let result = service.handle_await(params).await.expect("await result");
        assert!(first_text(&result).contains("already completed"));
        // Completed op details should be included as a JSON block in content.
        assert!(
            result
                .content
                .iter()
                .filter_map(|c| c.as_text())
                .any(|t| t.text.contains("\"tool_name\": \"demo_tool\""))
        );
    }

    #[tokio::test]
    async fn handle_await_no_pending_operations_returns_fast_message() {
        let service = make_service().await;
        let params = call_tool_params("await", json!({}));
        let result = service.handle_await(params).await.expect("await result");
        assert_eq!(first_text(&result), "No pending operations to await for.");
    }

    #[tokio::test]
    async fn handle_await_filtered_no_pending_but_recently_completed_lists_history() {
        let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
            Duration::from_secs(30),
        )));
        let service = make_service_with_monitor(monitor.clone(), Arc::new(None)).await;

        let op_id = "op_recent".to_string();
        let op = Operation::new(
            op_id.clone(),
            "alpha_tool".to_string(),
            "desc".to_string(),
            None,
        );
        monitor.add_operation(op).await;
        monitor
            .update_status(
                &op_id,
                OperationStatus::Completed,
                Some(json!({"ok": true})),
            )
            .await;

        let params = call_tool_params("await", json!({"tools": "alpha"}));
        let result = service.handle_await(params).await.expect("await result");
        let text = first_text(&result);
        assert!(text.contains("No pending operations for tools: alpha"));
        assert!(
            result
                .content
                .iter()
                .filter_map(|c| c.as_text())
                .any(|t| t.text.contains("\"id\": \"op_recent\""))
        );
    }

    #[tokio::test]
    async fn calculate_intelligent_timeout_uses_max_of_default_and_ops() {
        let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
            Duration::from_secs(30),
        )));
        let service = make_service_with_monitor(monitor.clone(), Arc::new(None)).await;

        let mut op = Operation::new(
            "op_long".to_string(),
            "beta_tool".to_string(),
            "desc".to_string(),
            None,
        );
        op.timeout_duration = Some(Duration::from_secs(600));
        monitor.add_operation(op).await;

        let t_any = service.calculate_intelligent_timeout(&[]).await;
        assert!(t_any >= 600.0);

        let t_filtered_miss = service
            .calculate_intelligent_timeout(&["nope".to_string()])
            .await;
        assert!(t_filtered_miss >= 240.0);

        let t_filtered_hit = service
            .calculate_intelligent_timeout(&["beta".to_string()])
            .await;
        assert!(t_filtered_hit >= 600.0);
    }

    #[tokio::test]
    async fn create_tool_from_config_prepends_guidance_block() {
        let mut guidance_blocks = std::collections::HashMap::new();
        guidance_blocks.insert("my_tool".to_string(), "GUIDE".to_string());
        let guidance = GuidanceConfig {
            guidance_blocks,
            templates: std::collections::HashMap::new(),
            legacy_guidance: None,
        };

        let service = make_service_with_monitor(
            Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
                Duration::from_secs(30),
            ))),
            Arc::new(Some(guidance)),
        )
        .await;

        let tool_config = ToolConfig {
            name: "my_tool".to_string(),
            description: "DESC".to_string(),
            command: "echo".to_string(),
            subcommand: Some(vec![SubcommandConfig {
                name: "default".to_string(),
                description: "d".to_string(),
                subcommand: None,
                options: None,
                positional_args: None,
                positional_args_first: None,
                timeout_seconds: None,
                synchronous: None,
                enabled: true,
                guidance_key: None,
                sequence: None,
                step_delay_ms: None,
                availability_check: None,
                install_instructions: None,
            }]),
            input_schema: None,
            timeout_seconds: None,
            synchronous: None,
            hints: ToolHints::default(),
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
        };

        let tools = service.create_tools_from_config(&tool_config);
        assert_eq!(tools.len(), 1);
        let desc = tools[0].description.clone().unwrap_or_default();
        assert!(desc.starts_with("GUIDE\n\nDESC"));
    }

    #[tokio::test]
    async fn schemas_for_await_and_status_have_expected_properties() {
        let service = make_service().await;
        let await_schema = service.generate_input_schema_for_wait();
        let status_schema = service.generate_input_schema_for_status();

        let await_props = await_schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("await properties");
        assert!(await_props.contains_key("tools"));
        assert!(await_props.contains_key("id"));

        let status_props = status_schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("status properties");
        assert!(status_props.contains_key("tools"));
        assert!(status_props.contains_key("id"));
    }

    #[test]
    fn test_force_synchronous_inheritance_subcommand_overrides_tool() {
        // When subcommand has force_synchronous set, it should override tool level
        let subcommand_sync = Some(true);
        let tool_sync = Some(false);

        // Subcommand wins
        let effective = subcommand_sync.or(tool_sync);
        assert_eq!(effective, Some(true));
    }

    #[test]
    fn test_force_synchronous_inheritance_subcommand_none_inherits_tool() {
        // When subcommand has no force_synchronous, it should inherit from tool
        let subcommand_sync: Option<bool> = None;
        let tool_sync = Some(true);

        // Tool wins when subcommand is None
        let effective = subcommand_sync.or(tool_sync);
        assert_eq!(effective, Some(true));
    }

    #[test]
    fn test_force_synchronous_inheritance_both_none() {
        // When both are None, effective is None (default behavior)
        let subcommand_sync: Option<bool> = None;
        let tool_sync: Option<bool> = None;

        let effective = subcommand_sync.or(tool_sync);
        assert_eq!(effective, None);
    }

    #[test]
    fn test_force_synchronous_subcommand_explicit_false_overrides_tool_true() {
        // Subcommand can explicitly set false to override tool's true
        let subcommand_sync = Some(false);
        let tool_sync = Some(true);

        let effective = subcommand_sync.or(tool_sync);
        assert_eq!(effective, Some(false));
    }

    // ============= resolve_flattened_tool tests =============

    #[test]
    fn test_resolve_flattened_tool_found() {
        let mut configs = HashMap::new();
        configs.insert(
            "file-tools".to_string(),
            ToolConfig {
                name: "file-tools".to_string(),
                description: "File tools".to_string(),
                command: "ls".to_string(),
                subcommand: Some(vec![SubcommandConfig {
                    name: "read".to_string(),
                    ..SubcommandConfig::default()
                }]),
                ..serde_json::from_value(json!({
                    "name": "file-tools",
                    "description": "File tools",
                    "command": "ls",
                }))
                .unwrap()
            },
        );
        let result = AhmaMcpService::resolve_flattened_tool("file-tools_read", &configs);
        assert!(result.is_some());
        let (config, sub_path) = result.unwrap();
        assert_eq!(config.name, "file-tools");
        assert_eq!(sub_path, "read");
    }

    #[test]
    fn test_resolve_flattened_tool_not_found() {
        let configs = HashMap::new();
        let result = AhmaMcpService::resolve_flattened_tool("nonexistent_tool", &configs);
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_flattened_tool_no_subcommands() {
        let mut configs = HashMap::new();
        configs.insert(
            "simple".to_string(),
            ToolConfig {
                name: "simple".to_string(),
                description: "Simple tool".to_string(),
                command: "echo".to_string(),
                subcommand: None,
                ..serde_json::from_value(json!({
                    "name": "simple",
                    "description": "Simple tool",
                    "command": "echo",
                }))
                .unwrap()
            },
        );
        // Won't resolve because config has no subcommands
        let result = AhmaMcpService::resolve_flattened_tool("simple_sub", &configs);
        assert!(result.is_none());
    }

    // ============= is_bundle_tool / is_tool_disclosed tests =============

    #[tokio::test]
    async fn test_is_bundle_tool_known() {
        let service = make_service().await;
        // "cargo" is a known bundle config_tool_name
        assert!(service.is_bundle_tool("cargo"));
    }

    #[tokio::test]
    async fn test_is_bundle_tool_unknown() {
        let service = make_service().await;
        assert!(!service.is_bundle_tool("nonexistent_tool"));
    }

    #[tokio::test]
    async fn test_is_tool_disclosed_not_disclosed() {
        let service = make_service().await;
        let disclosed = HashSet::new();
        assert!(!service.is_tool_disclosed("cargo", &disclosed));
    }

    #[tokio::test]
    async fn test_is_tool_disclosed_after_disclosure() {
        let service = make_service().await;
        let mut disclosed = HashSet::new();
        // Bundle name for "cargo" config_tool_name is "rust"
        disclosed.insert("rust".to_string());
        assert!(service.is_tool_disclosed("cargo", &disclosed));
    }

    // ============= pre_disclose tests =============

    #[tokio::test]
    async fn test_pre_disclose_empty() {
        let service = make_service().await;
        let empty = HashSet::new();
        service.pre_disclose(&empty);
        // Should not panic and disclosed set should remain empty
        let disclosed = service.disclosed_bundles.read().unwrap();
        assert!(disclosed.is_empty());
    }

    #[tokio::test]
    async fn test_pre_disclose_adds_bundles() {
        let service = make_service().await;
        let mut bundles = HashSet::new();
        bundles.insert("cargo".to_string());
        bundles.insert("git".to_string());
        service.pre_disclose(&bundles);
        let disclosed = service.disclosed_bundles.read().unwrap();
        assert!(disclosed.contains("cargo"));
        assert!(disclosed.contains("git"));
    }

    // ============= list_tool_names tests =============

    #[tokio::test]
    async fn test_list_tool_names_includes_hardcoded() {
        let service = make_service().await;
        let names = service.list_tool_names();
        assert!(names.contains(&"await".to_string()));
        assert!(names.contains(&"status".to_string()));
        assert!(names.contains(&"sandboxed_shell".to_string()));
    }

    // ============= generate_activate_tools_description tests =============

    #[tokio::test]
    async fn test_generate_activate_tools_description_no_bundles_loaded() {
        let service = make_service().await;
        let desc = service.generate_activate_tools_description();
        // With empty configs, no bundles are loaded; should return default description
        assert!(desc.contains("Discover and activate"));
    }

    // ============= get_info tests =============

    #[tokio::test]
    async fn test_get_info_returns_server_info() {
        let service = make_service().await;
        let info = service.get_info();
        assert_eq!(info.server_info.name, env!("CARGO_PKG_NAME"));
    }
}
