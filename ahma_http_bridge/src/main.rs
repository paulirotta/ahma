use ahma_http_bridge::{BridgeConfig, start_bridge};
use clap::Parser;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// HTTP-to-stdio bridge for MCP servers with session isolation support.
///
/// Enables multiple IDE instances to share a single HTTP endpoint while maintaining
/// separate sandbox scopes based on each client's workspace roots.
#[derive(Parser, Debug)]
#[command(name = "ahma-http-bridge")]
#[command(version, about)]
struct Args {
    /// Address to bind the HTTP server.
    #[arg(long, default_value = "127.0.0.1:3000")]
    bind_addr: SocketAddr,

    /// Explicit fallback sandbox scope if client provides no workspace roots.
    /// If omitted, clients must provide roots/list.
    #[arg(long)]
    default_sandbox_scope: Option<PathBuf>,

    /// Command to run the MCP server subprocess.
    /// If not specified, auto-detects local debug binary or uses 'ahma_mcp'.
    #[arg(long)]
    server_command: Option<String>,

    /// Additional arguments to pass to the MCP server subprocess.
    #[arg(long)]
    server_args: Vec<String>,

    /// Enable colored terminal output for subprocess I/O (debug mode).
    #[arg(long)]
    colored_output: bool,

    /// Block writes to temp directories (/tmp, /var/folders) for higher security.
    /// This prevents data exfiltration via temp files but breaks tools that require temp access.
    /// When enabled, passes --disable-temp-files to all spawned MCP subprocesses.
    #[arg(long = "disable-temp-files")]
    no_temp_files: bool,

    /// Disable HTTP/1.1 support and require HTTP/2+ over TCP.
    #[arg(long = "disable-http1-1")]
    disable_http1_1: bool,

    /// Timeout in seconds for the MCP handshake to complete.
    /// If the handshake (SSE connection + roots/list response) doesn't complete
    /// within this time, tool calls will return a timeout error.
    #[arg(long, default_value = "45")]
    handshake_timeout_secs: u64,

    /// OTLP endpoint for distributed tracing export.
    /// Providing this flag enables tracing. Equivalent to OTEL_EXPORTER_OTLP_ENDPOINT.
    #[arg(long, global = true)]
    opentelemetry: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse args early so tracing flags are available before logging init.
    let args = Args::parse();

    // Initialize logging subscriber.
    let env_filter = tracing_subscriber::EnvFilter::new(
        std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
    );

    let (otel_layer, _telemetry_guard) = ahma_common::observability::create_otel_layer(
        &ahma_common::observability::ObservabilityConfig::from_env("ahma_http_bridge")
            .with_endpoint(args.opentelemetry.as_deref()),
    );

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(otel_layer);

    subscriber.init();

    // Determine server command: explicit arg > local debug binary > default
    let cwd = std::env::current_dir()?;
    let (server_command, enable_colored_output) = match args.server_command {
        Some(cmd) => (cmd, args.colored_output),
        None => {
            if let Some(local_binary) = detect_local_debug_binary(&cwd) {
                tracing::info!("Debug mode detected - using local binary, colored output enabled");
                (local_binary, true)
            } else {
                ("ahma-mcp".to_string(), args.colored_output)
            }
        }
    };

    // Build server args, adding --disable-temp-files if enabled
    let mut server_args = args.server_args;
    if args.no_temp_files {
        server_args.push("--disable-temp-files".to_string());
        tracing::info!(
            "SECURE High-security mode: temp file writes will be blocked in subprocesses"
        );
    }

    let config = BridgeConfig {
        bind_addr: args.bind_addr,
        server_command,
        server_args,
        enable_colored_output,
        default_sandbox_scope: args.default_sandbox_scope,
        handshake_timeout_secs: args.handshake_timeout_secs,
        enable_quic: true,
        disable_http1_1: args.disable_http1_1,
        listener_kind: ahma_http_bridge::ListenerKind::Tcp(args.bind_addr),
    };

    tracing::info!("Starting Ahma HTTP Bridge on {}", config.bind_addr);
    tracing::info!("Proxying to command: {}", config.server_command);
    tracing::info!("Session isolation: ENABLED (always-on)");

    start_bridge(config).await?;
    Ok(())
}

fn detect_local_debug_binary(base_dir: &Path) -> Option<String> {
    let binary_path = base_dir.join("target").join("debug").join("ahma-mcp");
    if binary_path.exists() {
        Some(binary_path.to_str()?.to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn detect_local_debug_binary_finds_existing_path() {
        let tmp = tempdir().unwrap();
        let binary_path = tmp.path().join("target").join("debug").join("ahma-mcp");
        fs::create_dir_all(binary_path.parent().unwrap()).unwrap();
        fs::write(&binary_path, b"test").unwrap();

        let detected = detect_local_debug_binary(tmp.path());
        assert_eq!(detected.as_deref(), binary_path.to_str());
    }

    #[test]
    fn detect_local_debug_binary_returns_none_when_missing() {
        let tmp = tempdir().unwrap();
        assert!(detect_local_debug_binary(tmp.path()).is_none());
    }
}
