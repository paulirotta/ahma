//! # Unix Bridge Mode
//!
//! Runs the ahma_mcp server in HTTP-over-Unix-Socket bridge mode.
//! Clients connect via a Unix domain socket (UDS) using standard
//! MCP Streamable HTTP framing; no TCP port is opened.
//!
//! This mode is Unix-only (`#[cfg(unix)]`).

use crate::shell::cli::AppConfig;
use ahma_http_bridge::{BridgeConfig, ListenerKind, start_bridge};
use anyhow::{Context, Result};
use dunce;
use std::env;

/// Run in Unix domain socket bridge mode.
///
/// # Arguments
/// * `config` - Immutable application configuration.
///
/// # Errors
/// Returns an error if the bridge fails to start.
pub async fn run_unix_bridge_mode(config: AppConfig) -> Result<()> {
    let socket_path = if config.unix_socket_path.is_empty() {
        "/tmp/ahma-mcp.sock".to_string()
    } else {
        config.unix_socket_path.clone()
    };

    tracing::info!("Starting Unix socket bridge on {}", socket_path);
    tracing::info!("Session isolation: ENABLED (always-on)");

    let server_command = env::current_exe()
        .context("Failed to get current executable path")?
        .to_string_lossy()
        .to_string();

    let explicit_fallback_scope = if !config.sandbox_scopes.is_empty() {
        Some(
            dunce::canonicalize(&config.sandbox_scopes[0])
                .unwrap_or_else(|_| config.sandbox_scopes[0].clone()),
        )
    } else {
        None
    };

    let mut server_args = vec!["serve".to_string()];

    if config.explicit_tools_dir
        && let Some(ref tools_dir) = config.tools_dir
    {
        server_args.push("--tools-dir".to_string());
        server_args.push(tools_dir.to_string_lossy().to_string());
    }

    server_args.push("stdio".to_string());

    for bundle in &config.tool_bundles {
        server_args.push("--tool".to_string());
        server_args.push(bundle.clone());
    }

    let enable_colored_output = true;

    match &explicit_fallback_scope {
        Some(scope) => tracing::info!(
            "Unix socket bridge mode - explicit fallback sandbox scope: {}",
            scope.display()
        ),
        None => tracing::info!(
            "Unix socket bridge mode - strict roots mode: no fallback scope configured"
        ),
    }

    // Derive a dummy bind_addr (unused when ListenerKind::Unix is set).
    let bind_addr = "127.0.0.1:0".parse().unwrap();

    let bridge_config = BridgeConfig {
        bind_addr,
        server_command,
        server_args,
        enable_colored_output,
        default_sandbox_scope: explicit_fallback_scope,
        handshake_timeout_secs: config.handshake_timeout_secs,
        // QUIC is UDP-based and incompatible with Unix sockets.
        enable_quic: false,
        disable_http1_1: false,
        listener_kind: ListenerKind::Unix(socket_path),
    };

    start_bridge(bridge_config).await?;

    Ok(())
}
