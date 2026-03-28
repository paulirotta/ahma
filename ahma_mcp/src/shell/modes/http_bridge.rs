//! # HTTP Bridge Mode
//!
//! Runs the ahma_mcp server in HTTP bridge mode, which provides an HTTP interface
//! to the MCP server.

use crate::shell::cli::AppConfig;
use anyhow::{Context, Result};
use dunce;
use std::env;

/// Run in HTTP bridge mode.
///
/// # Arguments
/// * `config` - Immutable application configuration.
///
/// # Errors
/// Returns an error if the bridge fails to start.
pub async fn run_http_bridge_mode(config: AppConfig) -> Result<()> {
    use ahma_http_bridge::{BridgeConfig, start_bridge};

    let bind_addr = format!("{}:{}", config.http_host, config.http_port)
        .parse()
        .context("Invalid HTTP host/port")?;

    tracing::info!("Starting HTTP bridge on {}", bind_addr);
    tracing::info!("Session isolation: ENABLED (always-on)");

    // Build the command to run the stdio MCP server subprocess.
    // Env vars (AHMA_SYNC, AHMA_TIMEOUT, AHMA_LOG_MONITOR, AHMA_DISABLE_TEMP, etc.)
    // are automatically inherited by child processes — no need to pass them as flags.
    let server_command = env::current_exe()
        .context("Failed to get current executable path")?
        .to_string_lossy()
        .to_string();

    // Determine explicit fallback scope for no-roots clients.
    // SECURITY: only treat CLI/env as explicit fallback; do not silently use CWD.
    let explicit_fallback_scope = if !config.sandbox_scopes.is_empty() {
        Some(
            dunce::canonicalize(&config.sandbox_scopes[0])
                .unwrap_or_else(|_| config.sandbox_scopes[0].clone()),
        )
    } else {
        // AHMA_SANDBOX_SCOPE is already baked into config.sandbox_scopes — no need to recheck env
        None
    };

    // Subprocess gets the `serve stdio` subcommand.
    // --tools-dir must come before the subcommand (it's on `serve`, not `serve stdio`).
    // --tool flags propagate tool bundle choices; env vars propagate everything else.
    let mut server_args = vec!["serve".to_string()];

    // Pass --tools-dir only if explicitly provided (otherwise subprocess auto-detects)
    if config.explicit_tools_dir
        && let Some(ref tools_dir) = config.tools_dir
    {
        server_args.push("--tools-dir".to_string());
        server_args.push(tools_dir.to_string_lossy().to_string());
    }

    server_args.push("stdio".to_string());

    // Pass through tool bundle selection
    for bundle in &config.tool_bundles {
        server_args.push("--tool".to_string());
        server_args.push(bundle.clone());
    }

    if let Some(ref scope) = explicit_fallback_scope {
        // Pass scope as env AHMA_WORKING_DIRS for subprocess deferred-sandbox resolution
        // (already set in env by parent process if user configured it)
        let _ = scope; // scope used below in BridgeConfig only
    }

    let enable_colored_output = true;
    tracing::info!(
        "HTTP bridge mode - colored terminal output enabled (v{})",
        env!("CARGO_PKG_VERSION")
    );
    match &explicit_fallback_scope {
        Some(scope) => tracing::info!(
            "HTTP explicit fallback sandbox scope configured for no-roots clients: {}",
            scope.display()
        ),
        None => tracing::info!(
            "HTTP strict roots mode: no fallback scope configured; clients must provide roots/list"
        ),
    }

    let bridge_config = BridgeConfig {
        bind_addr,
        server_command,
        server_args,
        enable_colored_output,
        default_sandbox_scope: explicit_fallback_scope,
        handshake_timeout_secs: config.handshake_timeout_secs,
        enable_quic: !config.no_quic,
        disable_http1_1: config.disable_http1_1,
        listener_kind: ahma_http_bridge::ListenerKind::Tcp(bind_addr),
    };

    start_bridge(bridge_config).await?;

    Ok(())
}
