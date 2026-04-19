//! # Server Mode
//!
//! Runs the ahma_mcp server in stdio mode, which is the default mode for MCP integration.

use crate::shell::cli::AppConfig;
use crate::{
    config::ServerConfig as MpcServerConfig,
    sandbox,
    service_builder::{BuiltService, ServiceBuilder},
    utils::stdio::emit_stdout_notification,
};
use ahma_http_mcp_client::client::HttpMcpTransport;
use anyhow::{Context, Result};
use rmcp::ServiceExt;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{fs, signal};
use tracing::info;

/// Run in server mode (stdio MCP server).
///
/// # Arguments
/// * `config` - Immutable application configuration.
/// * `sandbox` - Sandbox configuration.
///
/// # Errors
/// Returns an error if the server fails to start or encounters a fatal error.
pub async fn run_server_mode(config: AppConfig, sandbox: Arc<sandbox::Sandbox>) -> Result<()> {
    tracing::info!("Starting ahma_mcp v{}", env!("CARGO_PKG_VERSION"));
    if let Some(ref tools_dir) = config.tools_dir {
        tracing::info!("Tools directory: {:?}", tools_dir);
    } else {
        tracing::info!("No tools directory (using built-in internal tools only)");
    }
    tracing::info!("Command timeout: {}s", config.timeout_secs);

    // --- MCP Client Mode ---
    if fs::try_exists(&config.mcp_config).await.unwrap_or(false) {
        // Try to load the MCP config, but ignore if it's not a valid ahma_mcp config
        // (e.g., if it's a Cursor/VSCode MCP server config with "type": "stdio")
        match crate::config::load_mcp_config(&config.mcp_config).await {
            Ok(mcp_config) => {
                if let Some(server_config) = mcp_config.servers.values().next()
                    && let MpcServerConfig::Http(http_config) = server_config
                {
                    tracing::info!("Initializing HTTP MCP Client for: {}", http_config.url);

                    let url = url::Url::parse(&http_config.url)
                        .context("Failed to parse MCP server URL")?;

                    let transport = HttpMcpTransport::new(
                        url,
                        http_config.atlassian_client_id.clone(),
                        http_config.atlassian_client_secret.clone(),
                    )?;

                    // Authenticate if needed
                    transport.ensure_authenticated().await?;

                    tracing::info!("Successfully connected to HTTP MCP server");
                    tracing::warn!(
                        "Remote tools are not yet proxied to the client - this is a partial integration"
                    );

                    // Keep the transport alive for the duration of the process
                    // This ensures the background SSE listener continues to run
                    Box::leak(Box::new(transport));
                }
            }
            Err(e) => {
                // Ignore config parse errors - the file might be a Cursor/VSCode MCP config
                tracing::debug!(
                    "Could not parse mcp.json as ahma_mcp config (this is OK if it's a Cursor/VSCode MCP config): {}",
                    e
                );
            }
        }
    }

    // Build the MCP service: monitor → pool → adapter → configs → service.
    let BuiltService {
        service,
        adapter,
        operation_monitor,
        shutdown_timeout,
        loaded_tools_count,
        configs: _configs,
    } = ServiceBuilder::new(&config, sandbox.clone())
        .build()
        .await?;
    let service_handler = service;

    // Hot-reload is opt-in because runtime writes can change tool behavior mid-session.
    if config.hot_reload_tools
        && let Some(tools_dir) = config.tools_dir.clone()
    {
        service_handler.start_config_watcher(tools_dir, config.clone());
    }

    let sandbox_mode_label = if sandbox.is_test_mode() {
        "DISABLED/TEST"
    } else {
        #[cfg(target_os = "linux")]
        {
            "LANDLOCK"
        }
        #[cfg(target_os = "macos")]
        {
            "SEATBELT"
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            "UNSUPPORTED"
        }
    };

    let sandbox_scopes = sandbox
        .scopes()
        .iter()
        .map(|scope| scope.display().to_string())
        .collect::<Vec<_>>();

    tracing::info!(
        "Startup summary: sandbox_mode={}, sandbox_scopes={:?}, disable_temp_files={}, tools_dir={}, loaded_tools={}",
        sandbox_mode_label,
        sandbox_scopes,
        sandbox.is_no_temp_files(),
        config
            .tools_dir
            .as_ref()
            .map_or_else(|| "<none>".to_string(), |dir| dir.display().to_string()),
        loaded_tools_count,
    );

    // Use PatchedStdioTransport to fix rmcp 0.13.0 deserialization issues with VS Code
    use crate::transport_patch::PatchedStdioTransport;
    let service = service_handler
        .serve(PatchedStdioTransport::new_stdio())
        .await?;

    // ============================================================================
    // CRITICAL: Graceful Shutdown Implementation for Development Workflow
    // ============================================================================
    //
    // PURPOSE: Solves "Does the ahma_mcp server shut down gracefully when
    //          .vscode/mcp.json watch triggers a restart?"
    //
    // LESSON LEARNED: cargo watch sends SIGTERM during file changes, causing
    // abrupt termination of ongoing operations. This implementation provides:
    // 1. Signal handling for SIGTERM (cargo watch) and SIGINT (Ctrl+C)
    // 2. 360-second grace period for operations to complete naturally
    // 3. Progress monitoring with user feedback during shutdown
    // 4. Forced exit if service doesn't shutdown within 5 additional seconds
    //
    // DO NOT REMOVE: This is essential for development workflow integration
    // ============================================================================

    // Set up signal handling for graceful shutdown
    let adapter_for_signal = adapter.clone();
    let operation_monitor_for_signal = operation_monitor.clone();
    tokio::spawn(async move {
        let shutdown_reason = tokio::select! {
            _ = signal::ctrl_c() => {
                info!("Received SIGINT, initiating graceful shutdown...");
                "Cancelled due to SIGINT (Ctrl+C) - user interrupt"
            }
            _ = async {
                #[cfg(unix)]
                {
                    let mut term_signal = signal::unix::signal(signal::unix::SignalKind::terminate())
                        .expect("Failed to setup SIGTERM handler");
                    term_signal.recv().await;
                }
                #[cfg(not(unix))]
                {
                    // On non-Unix systems, just await indefinitely
                    // The ctrl_c signal above will handle shutdown
                    std::future::pending::<()>().await;
                }
            } => {
                info!("Received SIGTERM (likely from cargo watch), initiating graceful shutdown...");
                "Cancelled due to SIGTERM from cargo watch - source code reload"
            }
        };

        // Check for active operations and provide progress feedback
        info!("🛑 Shutdown initiated - checking for active operations...");

        let shutdown_summary = operation_monitor_for_signal.get_shutdown_summary().await;

        if shutdown_summary.total_active > 0 {
            info!(
                "⏳ Waiting up to 15 seconds for {} active operation(s) to complete...",
                shutdown_summary.total_active
            );

            // Wait up to configured timeout for operations to complete with priority-based progress updates
            let shutdown_start = Instant::now();
            let shutdown_timeout = shutdown_timeout;

            while shutdown_start.elapsed() < shutdown_timeout {
                let current_summary = operation_monitor_for_signal.get_shutdown_summary().await;

                if current_summary.total_active == 0 {
                    info!("OK All operations completed successfully");
                    break;
                } else if current_summary.total_active != shutdown_summary.total_active {
                    info!(
                        "📈 Progress: {} operations remaining",
                        current_summary.total_active
                    );
                }

                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            let final_summary = operation_monitor_for_signal.get_shutdown_summary().await;

            if final_summary.total_active > 0 {
                info!(
                    "⏱️  Shutdown timeout reached - cancelling {} remaining operation(s) with reason: {}",
                    final_summary.total_active, shutdown_reason
                );

                // Cancel remaining operations with descriptive reason
                for op in final_summary.operations.iter() {
                    tracing::debug!(
                        "Attempting to cancel operation '{}' ({}) with reason: '{}'",
                        op.id,
                        op.tool_name,
                        shutdown_reason
                    );

                    let cancelled = operation_monitor_for_signal
                        .cancel_operation_with_reason(&op.id, Some(shutdown_reason.to_string()))
                        .await;

                    if cancelled {
                        info!("   OK Cancelled operation '{}' ({})", op.id, op.tool_name);
                        tracing::debug!("Successfully cancelled operation '{}' with reason", op.id);
                    } else {
                        tracing::warn!(
                            "   WARNING Failed to cancel operation '{}' ({})",
                            op.id,
                            op.tool_name
                        );
                        tracing::debug!(
                            "Failed to cancel operation '{}' - it may have already completed",
                            op.id
                        );
                    }
                }
            }
        } else {
            info!("OK No active operations - proceeding with immediate shutdown");
        }

        info!("🔄 Shutting down adapter and shell pools...");

        // Emit sandbox terminated notification
        if let Ok(notification) = serde_json::to_string(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/sandbox/terminated",
            "params": { "reason": shutdown_reason }
        })) {
            let _ = emit_stdout_notification(&notification);
        }

        adapter_for_signal.shutdown().await;

        // Force process exit if service doesn't stop naturally
        tokio::time::sleep(Duration::from_secs(5)).await;
        info!("Service did not stop gracefully, forcing exit");
        std::process::exit(0);
    });

    let result = service.waiting().await;

    // Emit sandbox terminated notification
    let reason = match &result {
        Ok(_) => "session_ended".to_string(),
        Err(e) => format!("session_error: {:#}", e),
    };

    if let Ok(notification) = serde_json::to_string(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/sandbox/terminated",
        "params": { "reason": reason }
    })) {
        let _ = emit_stdout_notification(&notification);
    }

    // Gracefully shutdown the adapter
    adapter.shutdown().await;

    result?;

    Ok(())
}
