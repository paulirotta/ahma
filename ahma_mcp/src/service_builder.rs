//! # Service Builder
//!
//! Provides a builder for initializing [`AhmaMcpService`] with all required
//! infrastructure components, eliminating the duplicated startup code that
//! previously existed across transport modes (stdio server, CLI, etc.).
//!
//! ## The shared initialization sequence
//!
//! All transport modes share the same init chain:
//! ```text
//! MonitorConfig → OperationMonitor
//!   → ShellPoolConfig → ShellPoolManager (+ start_background_tasks)
//!     → Adapter
//!       → load_tool_configs
//!         → evaluate_tool_availability
//!           → AhmaMcpService
//! ```
//!
//! `ServiceBuilder` encapsulates that chain so each mode only handles what is
//! unique to it — transport binding, signal handling, output formatting, etc.

use crate::{
    adapter::Adapter,
    config::{ToolConfig, load_tool_configs},
    mcp_service::{AhmaMcpService, GuidanceConfig},
    operation_monitor::{MonitorConfig, OperationMonitor},
    sandbox::Sandbox,
    shell::cli::AppConfig,
    shell_pool::{ShellPoolConfig, ShellPoolManager},
    tool_availability::{AvailabilitySummary, evaluate_tool_availability, format_install_guidance},
};
use anyhow::{Context, Result};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// The result of a successful [`ServiceBuilder::build`] call.
///
/// Contains the fully initialised MCP service together with the infrastructure
/// components that the caller may need for lifecycle management (e.g. signal
/// handlers, graceful shutdown).
pub struct BuiltService {
    /// The ready-to-serve MCP service.
    pub service: AhmaMcpService,
    /// Shared adapter for execution and resource cleanup.
    pub adapter: Arc<Adapter>,
    /// Shared operation monitor for in-flight task tracking.
    pub operation_monitor: Arc<OperationMonitor>,
    /// How long to wait for in-flight operations during graceful shutdown.
    pub shutdown_timeout: Duration,
    /// Number of tool configurations that passed availability checks.
    pub loaded_tools_count: usize,
    /// The final tool configurations after availability filtering.
    ///
    /// Provided for callers (e.g. CLI mode) that need direct config access
    /// before going through the MCP service layer.
    pub configs: Arc<HashMap<String, ToolConfig>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Builder
// ─────────────────────────────────────────────────────────────────────────────

/// Builds an [`AhmaMcpService`] from the shared initialization sequence.
///
/// Defaults are pre-populated from the provided [`AppConfig`] so callers only
/// need to override values that differ from the config (e.g. forcing
/// `force_synchronous = true` for CLI single-shot mode).
pub struct ServiceBuilder<'a> {
    config: &'a AppConfig,
    sandbox: Arc<Sandbox>,
    guidance: Option<GuidanceConfig>,
    skip_availability_probes: bool,
    force_synchronous: bool,
    defer_sandbox: bool,
    progressive_disclosure: bool,
    monitor_rate_limit: u64,
}

impl<'a> ServiceBuilder<'a> {
    /// Create a new builder, pre-populating options from `config`.
    pub fn new(config: &'a AppConfig, sandbox: Arc<Sandbox>) -> Self {
        Self {
            config,
            sandbox,
            guidance: Some(GuidanceConfig::default()),
            skip_availability_probes: config.skip_availability_probes,
            force_synchronous: config.force_sync,
            defer_sandbox: config.defer_sandbox,
            progressive_disclosure: config.progressive_disclosure,
            monitor_rate_limit: config.monitor_rate_limit_secs,
        }
    }

    /// Override the guidance configuration.
    pub fn with_guidance(mut self, guidance: GuidanceConfig) -> Self {
        self.guidance = Some(guidance);
        self
    }

    /// Override whether to skip tool availability probes.
    pub fn skip_availability_probes(mut self, skip: bool) -> Self {
        self.skip_availability_probes = skip;
        self
    }

    /// Override whether to force synchronous execution for all tools.
    pub fn force_synchronous(mut self, force: bool) -> Self {
        self.force_synchronous = force;
        self
    }

    /// Override whether to defer sandbox initialisation until roots arrive.
    pub fn defer_sandbox(mut self, defer: bool) -> Self {
        self.defer_sandbox = defer;
        self
    }

    /// Override progressive disclosure behaviour.
    pub fn progressive_disclosure(mut self, pd: bool) -> Self {
        self.progressive_disclosure = pd;
        self
    }

    /// Override the rate-limit (in seconds) for log-monitor alert suppression.
    pub fn monitor_rate_limit(mut self, secs: u64) -> Self {
        self.monitor_rate_limit = secs;
        self
    }

    /// Run all initialization steps and return a [`BuiltService`].
    ///
    /// # Errors
    ///
    /// Returns an error if any step fails: sandbox creation, pool startup, tool
    /// config loading, availability probing, or `AhmaMcpService` construction.
    pub async fn build(self) -> Result<BuiltService> {
        let config = self.config;
        let sandbox = &self.sandbox;

        let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(config.timeout_secs));
        let shutdown_timeout = monitor_config.shutdown_timeout;
        let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));

        let shell_pool_config = ShellPoolConfig {
            command_timeout: Duration::from_secs(config.timeout_secs),
            ..Default::default()
        };
        let shell_pool_manager = Arc::new(ShellPoolManager::new(shell_pool_config));
        shell_pool_manager.clone().start_background_tasks();

        let adapter = Arc::new(Adapter::new(
            operation_monitor.clone(),
            shell_pool_manager.clone(),
            sandbox.clone(),
        )?);

        let raw_configs = load_tool_configs(config, config.tools_dir.as_deref())
            .await
            .context("Failed to load tool configurations")?;

        let configs = if self.skip_availability_probes {
            tracing::info!("Skipping tool availability probes (AHMA_SKIP_PROBES)");
            Arc::new(raw_configs)
        } else {
            let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let availability_summary = evaluate_tool_availability(
                shell_pool_manager,
                raw_configs,
                working_dir.as_path(),
                sandbox.as_ref(),
            )
            .await?;

            log_availability_warnings(&availability_summary);
            Arc::new(availability_summary.filtered_configs)
        };

        log_loaded_tools(&configs, config.tools_dir.as_deref());

        let loaded_tools_count = configs.len();
        let configs_for_output = configs.clone();

        let mut service = AhmaMcpService::new(
            adapter.clone(),
            operation_monitor.clone(),
            configs,
            Arc::new(self.guidance),
            self.force_synchronous,
            self.defer_sandbox,
            self.progressive_disclosure,
        )
        .await?;

        service.monitor_rate_limit_seconds = self.monitor_rate_limit;

        // Pre-disclose bundles explicitly requested via --tools CLI flags so
        // their tools appear immediately in tools/list without an activate_tools
        // call.
        let cli_bundles = crate::config::cli_flagged_bundle_names(config);
        service.pre_disclose(&cli_bundles);

        Ok(BuiltService {
            service,
            adapter,
            operation_monitor,
            shutdown_timeout,
            loaded_tools_count,
            configs: configs_for_output,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Private helpers
// ─────────────────────────────────────────────────────────────────────────────

fn log_availability_warnings(summary: &AvailabilitySummary) {
    if !summary.disabled_tools.is_empty() {
        let current_path = std::env::var("PATH").unwrap_or_else(|_| "<not set>".to_string());
        tracing::warn!(
            "{} tool(s) disabled by availability probes. PATH={}",
            summary.disabled_tools.len(),
            current_path
        );
        for disabled in &summary.disabled_tools {
            tracing::warn!(
                "Tool '{}' disabled at startup. {}",
                disabled.name,
                disabled.message
            );
            if let Some(instructions) = &disabled.install_instructions {
                tracing::info!(
                    "Install instructions for '{}': {}",
                    disabled.name,
                    instructions
                );
            }
        }
    }

    if !summary.disabled_subcommands.is_empty() {
        for disabled in &summary.disabled_subcommands {
            tracing::warn!(
                "Tool subcommand '{}::{}' disabled at startup. {}",
                disabled.tool,
                disabled.subcommand_path,
                disabled.message
            );
            if let Some(instructions) = &disabled.install_instructions {
                tracing::info!(
                    "Install instructions for '{}::{}': {}",
                    disabled.tool,
                    disabled.subcommand_path,
                    instructions
                );
            }
        }
    }

    if !summary.disabled_tools.is_empty() || !summary.disabled_subcommands.is_empty() {
        let install_guidance = format_install_guidance(summary);
        tracing::warn!(
            "Startup tool guidance (share with users who need to install prerequisites):\n{}",
            install_guidance
        );
    }
}

fn log_loaded_tools(configs: &HashMap<String, ToolConfig>, tools_dir: Option<&std::path::Path>) {
    if configs.is_empty() {
        tracing::error!("No valid tool configurations available after availability checks");
        if let Some(dir) = tools_dir {
            tracing::error!("Tools directory: {:?}", dir);
        } else {
            tracing::error!("No tools directory specified (using built-in internal tools only)");
        }
        // Not fatal – the service can still serve built-in (hard-wired) tools.
    } else {
        let tool_names: Vec<String> = configs.keys().cloned().collect();
        tracing::info!(
            "Loaded {} tool configurations: {}",
            configs.len(),
            tool_names.join(", ")
        );
    }
}
