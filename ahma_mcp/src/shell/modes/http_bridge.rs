//! # HTTP Bridge Mode
//!
//! Runs the ahma_mcp server in HTTP bridge mode, which provides an HTTP interface
//! to the MCP server.

use crate::shell::cli::Cli;
use anyhow::{Context, Result};
use dunce;
use std::{env, path::PathBuf};

/// Run in HTTP bridge mode.
///
/// # Arguments
/// * `cli` - Command-line arguments.
///
/// # Errors
/// Returns an error if the bridge fails to start.
pub async fn run_http_bridge_mode(cli: Cli) -> Result<()> {
    use ahma_http_bridge::{BridgeConfig, start_bridge};

    // We need to re-derive fallback scope because we don't have global state anymore.
    // Ideally we pass `sandbox` to this function, but `sandbox` might be None if deferred.
    // So logic inside `run_http_bridge_mode` should just default to CWD if not provided,
    // OR we pass the calculated `sandbox_scopes` (if any) to it.

    let bind_addr = format!("{}:{}", cli.http_host, cli.http_port)
        .parse()
        .context("Invalid HTTP host/port")?;

    tracing::info!("Starting HTTP bridge on {}", bind_addr);
    tracing::info!("Session isolation: ENABLED (always-on)");

    // Build the command to run the stdio MCP server
    let server_command = env::current_exe()
        .context("Failed to get current executable path")?
        .to_string_lossy()
        .to_string();

    // Determine explicit fallback scope for no-roots clients.
    // SECURITY: only treat CLI/env as explicit fallback; do not silently use CWD.
    let explicit_fallback_scope = if !cli.sandbox_scope.is_empty() {
        Some(
            dunce::canonicalize(&cli.sandbox_scope[0])
                .unwrap_or_else(|_| cli.sandbox_scope[0].clone()),
        )
    } else if let Ok(env_scope) = std::env::var("AHMA_SANDBOX_SCOPE") {
        Some(PathBuf::from(env_scope))
    } else {
        None
    };

    // Session isolation is always enabled in HTTP mode.
    let mut server_args = vec!["--mode".to_string(), "stdio".to_string()];

    // Only pass --tools-dir if explicitly provided
    if let Some(ref tools_dir) = cli.tools_dir {
        server_args.push("--tools-dir".to_string());
        server_args.push(tools_dir.to_string_lossy().to_string());
    }

    if cli.hot_reload_tools {
        server_args.push("--hot-reload-tools".to_string());
    }

    server_args.push("--timeout".to_string());
    server_args.push(cli.timeout.to_string());

    if cli.debug {
        server_args.push("--debug".to_string());
    }

    if cli.sync {
        server_args.push("--sync".to_string());
    }

    if cli.no_sandbox {
        server_args.push("--disable-sandbox".to_string());
    }

    if cli.no_temp_files {
        server_args.push("--disable-temp-files".to_string());
    }

    if cli.livelog {
        server_args.push("--livelog".to_string());
    }

    if let Some(scope) = &explicit_fallback_scope {
        server_args.push("--working-directories".to_string());
        server_args.push(scope.to_string_lossy().to_string());
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

    let config = BridgeConfig {
        bind_addr,
        server_command,
        server_args,
        enable_colored_output,
        default_sandbox_scope: explicit_fallback_scope,
        handshake_timeout_secs: cli.handshake_timeout_secs,
        enable_quic: !cli.no_quic,
        disable_http1_1: cli.disable_http1_1,
    };

    start_bridge(config).await?;

    Ok(())
}
