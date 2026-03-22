//! # Ahma Server CLI
//!
//! This module contains the command-line interface definition and main entry point.
//!
//! ## CLI Design
//!
//! `ahma-mcp` uses a subcommand model (git/docker style):
//!
//! ```text
//! ahma-mcp serve stdio [--tools rust,python,git]
//! ahma-mcp serve http  [--port 3000] [--host 127.0.0.1] [--disable-quic] [--disable-http1-1]
//! ahma-mcp tool run <TOOL> [-- <TOOL_ARGS>...]
//! ahma-mcp tool validate [TARGET]
//! ahma-mcp tool list [--server NAME] [--http URL] [--format json|text] [--mcp-config PATH]
//! ahma-mcp tool info [--tools rust,git] [--format json|text] [TOOL]
//! ```
//!
//! Niche options that rarely need changing are controlled via environment variables.
//! See `docs/environment-variables.md` for the full reference.

use super::{list_tools, modes, resolution};

use crate::{sandbox, utils::logging::init_logging};
use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use dunce;
use std::{io::IsTerminal, path::PathBuf, sync::Arc};

// ─────────────────────────────────────────────────────────────────────────────
// AppConfig — single immutable application configuration
//
// Built once from CLI args + env vars, then passed as a shared reference.
// Never mutated after startup.
// ─────────────────────────────────────────────────────────────────────────────

/// Unified, immutable application configuration.
///
/// Constructed once at startup from CLI flags and environment variables.
/// All subsystems receive `Arc<AppConfig>` or `&AppConfig`; nothing reads
/// the CLI or env vars again after this point.
#[derive(Debug, Clone)]
pub struct AppConfig {
    // ── Tool loading ────────────────────────────────────────────────────────
    /// Path to the `.ahma/` tools directory (auto-detected or from AHMA_TOOLS_DIR).
    pub tools_dir: Option<PathBuf>,
    /// Whether `tools_dir` was explicitly set (vs auto-detected).
    pub explicit_tools_dir: bool,
    /// Tool bundles to activate (e.g. ["rust", "python"]).
    pub tool_bundles: Vec<String>,

    // ── Execution ───────────────────────────────────────────────────────────
    /// Default command timeout in seconds (AHMA_TIMEOUT, default 360).
    pub timeout_secs: u64,
    /// Run all tools synchronously (AHMA_SYNC=1).
    pub force_sync: bool,
    /// Reload tools from disk when `.ahma/` changes (AHMA_HOT_RELOAD=1).
    pub hot_reload_tools: bool,
    /// Skip tool availability probes at startup (AHMA_SKIP_PROBES=1).
    pub skip_availability_probes: bool,
    /// Show all tools without progressive disclosure (AHMA_PROGRESSIVE_DISCLOSURE=0).
    pub progressive_disclosure: bool,

    // ── Sandbox ─────────────────────────────────────────────────────────────
    /// Disable the kernel sandbox entirely (AHMA_DISABLE_SANDBOX=1).
    pub no_sandbox: bool,
    /// Explicit sandbox scope directories (from AHMA_SANDBOX_SCOPE).
    pub sandbox_scopes: Vec<PathBuf>,
    /// Defer sandbox lock until client provides roots/list (AHMA_SANDBOX_DEFER=1).
    pub defer_sandbox: bool,
    /// Working directories seeded when defer mode lacks client roots (AHMA_WORKING_DIRS).
    pub working_dirs: Vec<PathBuf>,
    /// Add system temp dir to sandbox scopes (AHMA_TMP_ACCESS=1).
    pub tmp_access: bool,
    /// Block writes to temp directories (AHMA_DISABLE_TEMP=1).
    pub no_temp_files: bool,
    /// Enable live-log monitoring mode (AHMA_LOG_MONITOR=1).
    pub log_monitor: bool,
    /// Minimum seconds between log-monitor alerts (AHMA_MONITOR_RATE_LIMIT, default 60).
    pub monitor_rate_limit_secs: u64,

    // ── HTTP serve mode ─────────────────────────────────────────────────────
    /// Bind host for HTTP mode (default 127.0.0.1).
    pub http_host: String,
    /// Bind port for HTTP mode (default 3000).
    pub http_port: u16,
    /// Disable HTTP/3 QUIC (AHMA_DISABLE_QUIC=1).
    pub no_quic: bool,
    /// Require HTTP/2+ only (AHMA_DISABLE_HTTP1_1=1).
    pub disable_http1_1: bool,
    /// Handshake timeout for HTTP mode in seconds (AHMA_HANDSHAKE_TIMEOUT, default 45).
    pub handshake_timeout_secs: u64,

    // ── tool list subcommand ─────────────────────────────────────────────────
    /// Server name from mcp.json (for `tool list`).
    pub list_server: Option<String>,
    /// Path to mcp.json (for `tool list`).
    pub mcp_config: PathBuf,
    /// HTTP URL to query (for `tool list`).
    pub list_http: Option<String>,
    /// Output format (for `tool list`).
    pub list_format: list_tools::OutputFormat,

    // ── run subcommand ───────────────────────────────────────────────────────
    /// Tool name for `run` subcommand (also used for positional args in that context).
    pub run_tool: Option<String>,
    /// Arguments forwarded to the tool after `--`.
    pub run_tool_args: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tools_dir: None,
            explicit_tools_dir: false,
            tool_bundles: vec![],
            timeout_secs: 360,
            force_sync: false,
            hot_reload_tools: false,
            skip_availability_probes: false,
            progressive_disclosure: true,
            no_sandbox: false,
            sandbox_scopes: vec![],
            defer_sandbox: false,
            working_dirs: vec![],
            tmp_access: false,
            no_temp_files: false,
            log_monitor: false,
            monitor_rate_limit_secs: 60,
            http_host: "127.0.0.1".to_string(),
            http_port: 3000,
            no_quic: false,
            disable_http1_1: false,
            handshake_timeout_secs: 45,
            list_server: None,
            mcp_config: PathBuf::from("mcp.json"),
            list_http: None,
            list_format: list_tools::OutputFormat::Text,
            run_tool: None,
            run_tool_args: vec![],
        }
    }
}

impl AppConfig {
    /// Read a boolean env var ("1","true","yes","on" → true; anything else → false).
    pub fn env_flag(name: &str) -> bool {
        std::env::var(name)
            .map(|v| {
                let t = v.trim().to_ascii_lowercase();
                matches!(t.as_str(), "1" | "true" | "yes" | "on")
            })
            .unwrap_or(false)
    }

    /// Read a u64 env var, returning `default` if absent or unparseable.
    fn env_u64(name: &str, default: u64) -> u64 {
        std::env::var(name)
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(default)
    }

    /// Parse `AHMA_SANDBOX_SCOPE` using the platform path-list separator.
    fn env_sandbox_scopes() -> Vec<PathBuf> {
        std::env::var_os("AHMA_SANDBOX_SCOPE")
            .map(|paths| {
                std::env::split_paths(&paths)
                    .filter(|path| !path.as_os_str().is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Parse `AHMA_WORKING_DIRS` using the platform path-list separator.
    fn env_working_dirs() -> Vec<PathBuf> {
        std::env::var_os("AHMA_WORKING_DIRS")
            .map(|paths| {
                std::env::split_paths(&paths)
                    .filter(|path| !path.as_os_str().is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sandbox policy helpers
// ─────────────────────────────────────────────────────────────────────────────

struct SandboxPolicy {
    no_sandbox: bool,
    tmp_access: bool,
    mode: sandbox::SandboxMode,
}

fn resolve_sandbox_policy(cfg: &AppConfig) -> SandboxPolicy {
    let no_sandbox = cfg.no_sandbox;
    let tmp_access = cfg.tmp_access;

    let mode = if no_sandbox {
        tracing::warn!("Ahma sandbox disabled via AHMA_DISABLE_SANDBOX or --serve flag");
        #[cfg(target_os = "linux")]
        if let Err(error) = sandbox::check_sandbox_prerequisites() {
            tracing::warn!(
                "Continuing without Ahma sandbox because Linux sandbox prerequisites are unavailable: {}. \
                 Update Linux kernel to 5.13+ to enable Landlock.",
                error
            );
        }
        sandbox::SandboxMode::Test
    } else {
        sandbox::SandboxMode::Strict
    };

    SandboxPolicy {
        no_sandbox,
        tmp_access,
        mode,
    }
}

fn check_sandbox_availability(no_sandbox: bool) -> Result<()> {
    if no_sandbox {
        return Ok(());
    }

    if let Err(e) = sandbox::check_sandbox_prerequisites() {
        sandbox::exit_with_sandbox_error(&e);
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(e) = sandbox::test_sandbox_exec_available() {
            sandbox::exit_with_sandbox_error(&e);
        }
    }

    Ok(())
}

fn canonicalize_paths(paths: &[PathBuf], context: &str) -> Result<Vec<PathBuf>> {
    paths
        .iter()
        .map(|p| {
            dunce::canonicalize(p)
                .with_context(|| format!("Failed to canonicalize {}: {:?}", context, p))
        })
        .collect()
}

fn resolve_sandbox_scopes(cfg: &AppConfig) -> Result<Option<Vec<PathBuf>>> {
    if cfg.defer_sandbox {
        return resolve_deferred_scopes(cfg);
    }

    if !cfg.sandbox_scopes.is_empty() {
        let scopes = canonicalize_paths(&cfg.sandbox_scopes, "sandbox scope")?;
        return Ok(Some(scopes));
    }

    let cwd = std::env::current_dir()
        .context("Failed to get current working directory for sandbox scope")?;
    Ok(Some(vec![cwd]))
}

fn resolve_deferred_scopes(cfg: &AppConfig) -> Result<Option<Vec<PathBuf>>> {
    if !cfg.working_dirs.is_empty() {
        let scopes = canonicalize_paths(&cfg.working_dirs, "working directory")?;
        tracing::info!("Sandbox initialized from AHMA_WORKING_DIRS: {:?}", scopes);
        Ok(Some(scopes))
    } else {
        tracing::info!("Sandbox initialization deferred - will be set from client roots/list");
        Ok(Some(Vec::new()))
    }
}

fn add_temp_scope_if_requested(
    scopes: Option<Vec<PathBuf>>,
    tmp_access: bool,
) -> Option<Vec<PathBuf>> {
    if !tmp_access {
        return scopes;
    }

    let mut scopes = scopes?;

    let temp_dir = std::env::temp_dir();
    match dunce::canonicalize(&temp_dir) {
        Ok(canonical_temp) if !scopes.contains(&canonical_temp) => {
            tracing::info!(
                "Adding temp directory to sandbox scopes via AHMA_TMP_ACCESS: {:?}",
                canonical_temp
            );
            scopes.push(canonical_temp);
        }
        Ok(_) => {}
        Err(_) => {
            tracing::warn!(
                "Could not canonicalize temp directory {:?}, skipping AHMA_TMP_ACCESS scope addition",
                temp_dir
            );
        }
    }

    Some(scopes)
}

fn create_sandbox_instance(
    scopes: Option<Vec<PathBuf>>,
    policy: &SandboxPolicy,
    cfg: &AppConfig,
) -> Result<Option<Arc<sandbox::Sandbox>>> {
    let Some(scopes) = scopes else {
        return Ok(None);
    };

    let s = sandbox::Sandbox::new(
        scopes.clone(),
        policy.mode,
        cfg.no_temp_files,
        cfg.log_monitor,
        policy.tmp_access,
    )
    .context("Failed to initialize sandbox")?;

    tracing::info!("Sandbox scopes initialized: {:?}", scopes);

    apply_platform_sandbox_enforcement(&s, policy, cfg)?;

    Ok(Some(Arc::new(s)))
}

fn apply_platform_sandbox_enforcement(
    sandbox: &sandbox::Sandbox,
    policy: &SandboxPolicy,
    cfg: &AppConfig,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if policy.mode == sandbox::SandboxMode::Strict
            && !cfg.defer_sandbox
            && let Err(e) = sandbox::enforce_landlock_sandbox(
                &sandbox.scopes(),
                sandbox.read_scopes(),
                sandbox.is_no_temp_files(),
            )
        {
            tracing::error!("Failed to enforce Landlock sandbox: {}", e);
            return Err(e);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Err(e) = sandbox::enforce_windows_sandbox(&sandbox.scopes()) {
            tracing::warn!("Windows Job Object enforcement failed: {}", e);
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let _ = (sandbox, policy, cfg);

    Ok(())
}

fn log_sandbox_mode(no_sandbox: bool) {
    if no_sandbox {
        tracing::info!("🔓 Sandbox mode: DISABLED (commands run without Ahma sandboxing)");
        return;
    }

    #[cfg(target_os = "linux")]
    tracing::info!("SECURE Sandbox mode: LANDLOCK (Linux kernel-level file system restrictions)");

    #[cfg(target_os = "macos")]
    tracing::info!("SECURE Sandbox mode: SEATBELT (macOS sandbox-exec per-command restrictions)");

    #[cfg(target_os = "windows")]
    tracing::info!(
        "SECURE Sandbox mode: APPCONTAINER (per-command path security) + \
         JOB OBJECT (kill-on-close process tracking)"
    );

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    tracing::info!(
        "SECURE Sandbox mode: UNSUPPORTED ON THIS OS (startup fails closed in strict mode)"
    );
}

#[cfg(target_os = "windows")]
fn check_powershell_available() {
    let ps_check = std::process::Command::new("powershell")
        .arg("-NoProfile")
        .arg("-Command")
        .arg("$PSVersionTable.PSVersion.ToString()")
        .output();
    match ps_check {
        Ok(out) if out.status.success() => {
            let ver = String::from_utf8_lossy(&out.stdout);
            tracing::info!("PowerShell detected: {}", ver.trim());
        }
        _ => {
            eprintln!(
                "\nFAIL Error: PowerShell was not found.\n\n\
                 ahma_mcp requires PowerShell (built into Windows 10/11) as its runtime shell.\n"
            );
            std::process::exit(1);
        }
    }
}

async fn dispatch_subcommand(cmd: Subcommands, cfg: AppConfig) -> Result<()> {
    match cmd {
        Subcommands::Serve(serve_args) => match serve_args.transport {
            ServeTransport::Stdio => {
                let sandbox = initialize_sandbox(&cfg)?;
                let sandbox = sandbox
                    .ok_or_else(|| anyhow!("Sandbox failed to initialize for stdio mode"))?;
                check_stdio_not_interactive()?;
                tracing::info!("Running in STDIO server mode");
                modes::run_server_mode(cfg, sandbox).await
            }
            ServeTransport::Http(_) => {
                tracing::info!("Running in HTTP bridge mode");
                modes::run_http_bridge_mode(cfg).await
            }
        },
        Subcommands::Tool(tool_cmd) => match tool_cmd.command {
            ToolCommand::Validate(v) => {
                tracing::info!("Running in validate mode");
                run_validation_mode(&v.target.unwrap_or_else(|| ".ahma".to_string()))
            }
            ToolCommand::List(_) => {
                tracing::info!("Running in list-tools mode");
                modes::run_list_tools_mode(&cfg).await
            }
            ToolCommand::Run(run_args) => {
                let cfg = AppConfig {
                    run_tool: Some(run_args.tool),
                    run_tool_args: run_args.tool_args,
                    ..cfg
                };
                let sandbox = initialize_sandbox(&cfg)?;
                let sandbox = sandbox
                    .ok_or_else(|| anyhow!("Sandbox scopes must be initialized for run mode"))?;
                tracing::info!("Running in CLI mode");
                modes::run_cli_mode(cfg, sandbox).await
            }
            ToolCommand::Info(info_args) => {
                tracing::info!("Running in tool-info mode");
                run_tool_info_mode(info_args).await
            }
        },
    }
}

fn check_stdio_not_interactive() -> Result<()> {
    if !std::io::stdin().is_terminal() {
        return Ok(());
    }

    eprintln!(
        "\nFAIL Error: ahma_mcp is an MCP server designed for JSON-RPC communication over stdio.\n"
    );
    eprintln!("It cannot be run directly from an interactive terminal.\n");
    eprintln!("Usage options:");
    eprintln!("  1. Run as stdio MCP server (requires MCP client):");
    eprintln!("     ahma-mcp serve stdio\n");
    eprintln!("  2. Run as HTTP bridge server:");
    eprintln!("     ahma-mcp serve http --port 3000\n");
    eprintln!("  3. Execute a single tool command:");
    eprintln!("     ahma-mcp tool run <tool_name> [-- tool_arguments...]\n");
    eprintln!("For more information, run: ahma-mcp --help\n");
    std::process::exit(1);
}

// ─────────────────────────────────────────────────────────────────────────────
// CLI argument types (clap)
// ─────────────────────────────────────────────────────────────────────────────

/// Ahma MCP: A secure, config-driven adapter for CLI tools.
///
/// Environment variables control all non-essential options.
/// See docs/environment-variables.md for the full reference.
#[derive(Parser, Debug)]
#[command(
    name = "ahma-mcp",
    author,
    version,
    about = "Ahma MCP: secure, config-driven adapter for CLI tools"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Subcommands,
}

#[derive(Subcommand, Debug)]
pub enum Subcommands {
    /// Start an MCP server (stdio or http).
    Serve(ServeArgs),
    /// Tool management and execution utilities.
    Tool(ToolArgs),
}

// ── serve ────────────────────────────────────────────────────────────────────

/// Start the ahma-mcp MCP server.
///
/// Choose a transport that fits your integration:
///
/// * **stdio** — spawned as a subprocess by an MCP client (Cursor, VS Code,
///   Claude Desktop).  The client manages the process lifetime; no network
///   port is opened.  This is the most common mode.
///
/// * **http** — a persistent, multi-session bridge that listens on a TCP
///   port and supports several MCP clients concurrently.  Useful for CI,
///   shared developer machines, or any situation where clients connect over
///   a network rather than spawning a process.
///
/// Tools are loaded from the directory specified by `--tools-dir`, the
/// `AHMA_TOOLS_DIR` environment variable, or the `.ahma/` folder detected
/// in the current working directory (in that order of precedence).
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
  # Serve over stdio (typical mcp.json entry for Cursor / VS Code)
  ahma-mcp serve stdio

  # Serve over stdio and enable the rust + git tool bundles
  ahma-mcp serve stdio --tools rust,git

  # Serve over HTTP on the default address (127.0.0.1:3000)
  ahma-mcp serve http

  # Serve over HTTP on a custom port with HTTP/3 disabled
  ahma-mcp serve http --port 8080 --disable-quic")]
pub struct ServeArgs {
    #[command(subcommand)]
    pub transport: ServeTransport,

    /// Tool bundles to enable (e.g. --tools rust --tools python,git).
    /// Repeat or comma-separate. Available: rust, python, git, kotlin, fileutils, github, simplify.
    #[arg(
        long = "tools",
        value_name = "NAME",
        value_delimiter = ',',
        global = true
    )]
    pub tool_bundles: Vec<String>,

    /// Path to the tools directory containing JSON tool definitions.
    /// Defaults to the auto-detected .ahma/ in the current directory.
    /// Override with AHMA_TOOLS_DIR env var.
    #[arg(long)]
    pub tools_dir: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum ServeTransport {
    /// Serve over stdio — the standard transport for MCP clients.
    ///
    /// The MCP client (Cursor, VS Code, Claude Desktop, …) spawns
    /// ahma-mcp as a child process and communicates over stdin/stdout.
    /// No network port is opened; sandboxing is applied per-session.
    ///
    /// To wire ahma-mcp into an MCP client add an entry like this to
    /// your `mcp.json` (exact key names vary by client):
    ///
    ///   "ahma": {
    ///     "command": "ahma-mcp",
    ///     "args": ["serve", "stdio", "--tool", "rust,git"]
    ///   }
    #[command(after_help = "EXAMPLES:
  # Minimal stdio server
  ahma-mcp serve stdio

  # Enable specific tool bundles
  ahma-mcp serve stdio --tool rust --tool python,git

  # Use a custom tools directory
  ahma-mcp serve stdio --tools-dir /path/to/.ahma")]
    Stdio,
    /// Serve over HTTP — a persistent multi-session bridge.
    ///
    /// Listens on a TCP port and routes MCP sessions over HTTP/2 and
    /// (optionally) HTTP/3/QUIC.  Multiple MCP clients can connect
    /// concurrently.  Suitable for CI runners, shared machines, or
    /// remote integrations.
    Http(HttpArgs),
}

/// Start a persistent HTTP-based MCP bridge.
///
/// Binds a TCP listener and serves the MCP Streamable HTTP transport
/// (2025-03-26 spec).  Each connecting client gets an isolated session
/// with its own sandbox scope.
///
/// HTTP/3 over QUIC is enabled by default when the platform supports it
/// (requires a valid TLS certificate).  Use `--disable-quic` to fall back
/// to HTTP/2 over TCP only.  HTTP/1.1 is accepted by default; use
/// `--disable-http1-1` to require HTTP/2 or better.
///
/// Security: the server binds to `127.0.0.1` by default.  Bind to
/// `0.0.0.0` only in trusted network environments and consider placing
/// a reverse proxy in front for production use.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
  # Default: 127.0.0.1:3000, HTTP/2 + HTTP/3
  ahma-mcp serve http

  # Custom port, localhost only
  ahma-mcp serve http --port 8080

  # Bind on all interfaces (use with care)
  ahma-mcp serve http --host 0.0.0.0 --port 3000

  # HTTP/2 over TCP only (disable QUIC/HTTP3)
  ahma-mcp serve http --disable-quic

  # Require at least HTTP/2 — reject HTTP/1.1 clients
  ahma-mcp serve http --disable-http1-1")]
pub struct HttpArgs {
    /// Host to bind the HTTP server on.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Port to bind the HTTP server on.
    #[arg(long, default_value_t = 3000)]
    pub port: u16,

    /// Disable HTTP/3 (QUIC). Serve HTTP/2 over TCP only.
    #[arg(long = "disable-quic")]
    pub no_quic: bool,

    /// Require HTTP/2+; reject HTTP/1.1 connections.
    #[arg(long = "disable-http1-1")]
    pub disable_http1_1: bool,
}

// ── run ──────────────────────────────────────────────────────────────────────

/// Arguments for `ahma-mcp run <TOOL> [-- <TOOL_ARGS>...]`.
#[derive(Parser, Debug)]
pub struct RunArgs {
    /// Name of the tool to execute.
    #[arg(value_name = "TOOL")]
    pub tool: String,

    /// Arguments forwarded to the tool (after --).
    #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
    pub tool_args: Vec<String>,
}

// ── tool ─────────────────────────────────────────────────────────────────────

/// Arguments for `ahma-mcp tool`.
#[derive(Parser, Debug)]
pub struct ToolArgs {
    #[command(subcommand)]
    pub command: ToolCommand,
}

#[derive(Subcommand, Debug)]
pub enum ToolCommand {
    /// Validate tool JSON configurations against the MTDF schema.
    Validate(ValidateArgs),
    /// List all tools available from an MCP server.
    List(ListArgs),
    /// Execute a single tool command and print the result.
    ///
    /// Loads tool configurations, applies sandboxing, runs the named tool
    /// with the supplied arguments, and prints the output to stdout.
    /// Useful for scripting, CI pipelines, and debugging tool behaviour
    /// outside the MCP protocol.
    #[command(after_help = "EXAMPLES:
  # Run a cargo build in release mode
  ahma-mcp tool run cargo_build -- --release

  # Run git status
  ahma-mcp tool run git_status

  # Run with a custom tools directory
  AHMA_TOOLS_DIR=/path/to/.ahma ahma-mcp tool run my_tool -- --flag value")]
    Run(RunArgs),
    /// Show locally configured tools with descriptions and parameters.
    ///
    /// Loads tool definitions from the `.ahma/` directory (or `--tools-dir`)
    /// and built-in bundles (activated with `--tools`), then prints a summary
    /// of each tool including its subcommands, parameters, and hints.
    #[command(after_help = "EXAMPLES:
  # Show all tools from the local .ahma/ directory
  ahma-mcp tool info

  # Include built-in bundles
  ahma-mcp tool info --tools rust,git

  # JSON output for scripting
  ahma-mcp tool info --tools rust --format json

  # Show details for a specific tool
  ahma-mcp tool info cargo")]
    Info(InfoArgs),
}

/// Arguments for `ahma-mcp tool validate [TARGET]`.
#[derive(Parser, Debug)]
pub struct ValidateArgs {
    /// File, directory, or comma-separated list of paths to validate.
    /// Defaults to `.ahma` in the current directory.
    #[arg(value_name = "TARGET")]
    pub target: Option<String>,
}

/// Arguments for `ahma-mcp tool list`.
#[derive(Parser, Debug)]
pub struct ListArgs {
    /// Name of the server in mcp.json to connect to.
    #[arg(long)]
    pub server: Option<String>,

    /// Path to mcp.json configuration file.
    #[arg(long, default_value = "mcp.json")]
    pub mcp_config: PathBuf,

    /// HTTP URL for the MCP server (e.g. http://localhost:3000).
    #[arg(long)]
    pub http: Option<String>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = list_tools::OutputFormat::Text)]
    pub format: list_tools::OutputFormat,
}

/// Arguments for `ahma-mcp tool info`.
#[derive(Parser, Debug)]
pub struct InfoArgs {
    /// Tool bundles to include (e.g. --tools rust --tools python,git).
    /// Repeat or comma-separate. Available: rust, python, git, kotlin, fileutils, github, simplify.
    #[arg(long = "tools", value_name = "NAME", value_delimiter = ',')]
    pub tool_bundles: Vec<String>,

    /// Path to the tools directory containing JSON tool definitions.
    /// Defaults to the auto-detected `.ahma/` in the current directory.
    #[arg(long)]
    pub tools_dir: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = list_tools::OutputFormat::Text)]
    pub format: list_tools::OutputFormat,

    /// Show only a specific tool by name.
    #[arg(value_name = "TOOL")]
    pub filter: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// AppConfig construction from CLI + env vars
// ─────────────────────────────────────────────────────────────────────────────

fn build_app_config(cli: Cli) -> AppConfig {
    // Gather serve-level fields if present
    let (tool_bundles, cli_tools_dir, http_host, http_port, no_quic, disable_http1_1) = match &cli
        .command
    {
        Subcommands::Serve(s) => {
            let (host, port, no_quic, disable_http1_1) = match &s.transport {
                ServeTransport::Http(h) => (h.host.clone(), h.port, h.no_quic, h.disable_http1_1),
                ServeTransport::Stdio => ("127.0.0.1".to_string(), 3000u16, false, false),
            };
            (
                s.tool_bundles.clone(),
                s.tools_dir.clone(),
                host,
                port,
                no_quic,
                disable_http1_1,
            )
        }
        _ => (vec![], None, "127.0.0.1".to_string(), 3000u16, false, false),
    };

    // Tool list args
    let (list_server, mcp_config, list_http, list_format) = if let Subcommands::Tool(ToolArgs {
        command: ToolCommand::List(la),
    }) = &cli.command
    {
        (
            la.server.clone(),
            la.mcp_config.clone(),
            la.http.clone(),
            la.format.clone(),
        )
    } else {
        (
            None,
            PathBuf::from("mcp.json"),
            None,
            list_tools::OutputFormat::Text,
        )
    };

    // Run args (now under tool run)
    let (run_tool, run_tool_args) = if let Subcommands::Tool(ToolArgs {
        command: ToolCommand::Run(r),
    }) = &cli.command
    {
        (Some(r.tool.clone()), r.tool_args.clone())
    } else {
        (None, vec![])
    };

    // Env-var overrides for tools_dir
    let env_tools_dir = std::env::var("AHMA_TOOLS_DIR").ok().map(PathBuf::from);
    let explicit_tools_dir = cli_tools_dir.is_some();
    let raw_tools_dir = cli_tools_dir.or(env_tools_dir);
    let tools_dir = resolution::normalize_tools_dir(raw_tools_dir);

    // Flatten and deduplicate tool bundles (support comma-separation already handled by clap delimiter)
    let tool_bundles = {
        let mut seen = std::collections::HashSet::new();
        tool_bundles
            .into_iter()
            .filter(|b| seen.insert(b.clone()))
            .collect()
    };

    // Sandbox scopes: --sandbox-scope CLI flag is gone; use AHMA_SANDBOX_SCOPE env var
    let sandbox_scopes = AppConfig::env_sandbox_scopes();
    let working_dirs = AppConfig::env_working_dirs();

    // HTTP quic override from env
    let no_quic = no_quic || AppConfig::env_flag("AHMA_DISABLE_QUIC");
    let disable_http1_1 = disable_http1_1 || AppConfig::env_flag("AHMA_DISABLE_HTTP1_1");

    AppConfig {
        tools_dir,
        explicit_tools_dir,
        tool_bundles,
        timeout_secs: AppConfig::env_u64("AHMA_TIMEOUT", 360),
        force_sync: AppConfig::env_flag("AHMA_SYNC"),
        hot_reload_tools: AppConfig::env_flag("AHMA_HOT_RELOAD"),
        skip_availability_probes: AppConfig::env_flag("AHMA_SKIP_PROBES"),
        progressive_disclosure: !AppConfig::env_flag("AHMA_PROGRESSIVE_DISCLOSURE_OFF"),
        no_sandbox: AppConfig::env_flag("AHMA_DISABLE_SANDBOX"),
        sandbox_scopes,
        defer_sandbox: AppConfig::env_flag("AHMA_SANDBOX_DEFER"),
        working_dirs,
        tmp_access: AppConfig::env_flag("AHMA_TMP_ACCESS"),
        no_temp_files: AppConfig::env_flag("AHMA_DISABLE_TEMP"),
        log_monitor: AppConfig::env_flag("AHMA_LOG_MONITOR"),
        monitor_rate_limit_secs: AppConfig::env_u64("AHMA_MONITOR_RATE_LIMIT", 60),
        http_host,
        http_port,
        no_quic,
        disable_http1_1,
        handshake_timeout_secs: AppConfig::env_u64("AHMA_HANDSHAKE_TIMEOUT", 45),
        list_server,
        mcp_config,
        list_http,
        list_format,
        run_tool,
        run_tool_args,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

pub async fn run() -> Result<()> {
    // Determine log target before parsing full CLI so logging works for all subcommands.
    // RUST_LOG controls verbosity; AHMA_LOG_TARGET=stderr routes to stderr (default: file).
    let log_to_stderr = std::env::var("AHMA_LOG_TARGET")
        .map(|v| v.trim().eq_ignore_ascii_case("stderr"))
        .unwrap_or(false);
    init_logging("info", !log_to_stderr)?;

    let cli = Cli::parse();
    let subcommand = cli.command; // move out before consuming cli

    // We need the Cli to build AppConfig, but we moved it. Re-parse just for config.
    // Actually we already extracted the subcommand. Let's rebuild cli fresh.
    // Better: parse into a new CLI struct only for config.
    let cli2 = Cli::parse(); // second parse is cheap; both are from argv
    let cfg = build_app_config(cli2);

    #[cfg(target_os = "windows")]
    check_powershell_available();

    dispatch_subcommand(subcommand, cfg).await
}

fn initialize_sandbox(cfg: &AppConfig) -> Result<Option<Arc<sandbox::Sandbox>>> {
    let policy = resolve_sandbox_policy(cfg);

    check_sandbox_availability(policy.no_sandbox)?;

    let scopes = resolve_sandbox_scopes(cfg)?;
    let scopes = add_temp_scope_if_requested(scopes, policy.tmp_access);
    let sandbox = create_sandbox_instance(scopes, &policy, cfg)?;

    log_sandbox_mode(policy.no_sandbox);
    Ok(sandbox)
}

fn run_validation_mode(target: &str) -> Result<()> {
    let result = crate::validation::run_validation(target)?;
    if result.all_valid {
        println!("All configurations are valid.");
        Ok(())
    } else {
        anyhow::bail!(
            "Validation failed: {}/{} files invalid.",
            result.files_failed,
            result.files_checked
        )
    }
}

async fn run_tool_info_mode(args: InfoArgs) -> Result<()> {
    use crate::config;

    // Build a minimal AppConfig with the requested bundles + tools_dir
    let env_tools_dir = std::env::var("AHMA_TOOLS_DIR").ok().map(PathBuf::from);
    let raw_tools_dir = args.tools_dir.or(env_tools_dir);
    let tools_dir = resolution::normalize_tools_dir(raw_tools_dir);

    let mini_cfg = AppConfig {
        tool_bundles: args.tool_bundles,
        tools_dir: tools_dir.clone(),
        ..AppConfig::default()
    };

    let configs = config::load_tool_configs(&mini_cfg, tools_dir.as_deref()).await?;

    // Optionally filter to a single tool
    let mut tools: Vec<(&String, &config::ToolConfig)> = configs.iter().collect();
    if let Some(ref filter) = args.filter {
        tools.retain(|(name, _)| name.as_str() == filter.as_str());
        if tools.is_empty() {
            anyhow::bail!(
                "Tool '{}' not found. Run without a filter to see all available tools.",
                filter
            );
        }
    }
    tools.sort_by_key(|(name, _)| (*name).clone());

    match args.format {
        list_tools::OutputFormat::Text => print_tool_info_text(&tools),
        list_tools::OutputFormat::Json => print_tool_info_json(&tools)?,
    }

    Ok(())
}

fn print_tool_info_text(tools: &[(&String, &crate::config::ToolConfig)]) {
    println!("Local Tool Configurations");
    println!("=========================");
    println!();
    println!("Total tools: {}", tools.len());
    println!();

    for (name, config) in tools {
        println!("Tool: {}", name);
        println!("  Description: {}", config.description);
        println!("  Command:     {}", config.command);
        println!("  Enabled:     {}", config.enabled);
        if let Some(timeout) = config.timeout_seconds {
            println!("  Timeout:     {}s", timeout);
        }
        if let Some(sync) = config.synchronous {
            println!("  Synchronous: {}", sync);
        }

        // Subcommands
        if let Some(ref subs) = config.subcommand {
            println!("  Subcommands:");
            for sub in subs {
                let status = if sub.enabled { "" } else { " (disabled)" };
                println!("    - {}{}: {}", sub.name, status, sub.description);

                if let Some(ref opts) = sub.options {
                    for opt in opts {
                        let req = if opt.required.unwrap_or(false) {
                            "required"
                        } else {
                            "optional"
                        };
                        print!("        --{} ({}, {})", opt.name, opt.option_type, req);
                        if let Some(ref desc) = opt.description {
                            print!(": {}", desc);
                        }
                        println!();
                    }
                }
                if let Some(ref pos) = sub.positional_args {
                    for arg in pos {
                        let req = if arg.required.unwrap_or(false) {
                            "required"
                        } else {
                            "optional"
                        };
                        print!("        <{}> ({}, {})", arg.name, arg.option_type, req);
                        if let Some(ref desc) = arg.description {
                            print!(": {}", desc);
                        }
                        println!();
                    }
                }
            }
        }

        // Hints
        let h = &config.hints;
        let has_hints = h.build.is_some()
            || h.test.is_some()
            || h.dependencies.is_some()
            || h.clean.is_some()
            || h.run.is_some()
            || h.custom.as_ref().is_some_and(|c| !c.is_empty());
        if has_hints {
            println!("  Hints:");
            if let Some(ref v) = h.build {
                println!("    build: {}", v);
            }
            if let Some(ref v) = h.test {
                println!("    test: {}", v);
            }
            if let Some(ref v) = h.dependencies {
                println!("    dependencies: {}", v);
            }
            if let Some(ref v) = h.clean {
                println!("    clean: {}", v);
            }
            if let Some(ref v) = h.run {
                println!("    run: {}", v);
            }
            if let Some(ref custom) = h.custom {
                for (k, v) in custom {
                    println!("    {}: {}", k, v);
                }
            }
        }

        if let Some(ref ac) = config.availability_check {
            print!("  Availability check:");
            if let Some(ref cmd) = ac.command {
                print!(" {}", cmd);
            }
            if !ac.args.is_empty() {
                print!(" {}", ac.args.join(" "));
            }
            println!();
        }

        if let Some(ref inst) = config.install_instructions {
            println!("  Install: {}", inst);
        }

        println!();
    }
}

fn print_tool_info_json(tools: &[(&String, &crate::config::ToolConfig)]) -> Result<()> {
    let output: Vec<_> = tools
        .iter()
        .map(|(name, config)| {
            serde_json::json!({
                "name": name,
                "description": config.description,
                "command": config.command,
                "enabled": config.enabled,
                "timeout_seconds": config.timeout_seconds,
                "synchronous": config.synchronous,
                "subcommands": config.subcommand.as_ref().map(|subs| {
                    subs.iter().map(|s| {
                        serde_json::json!({
                            "name": s.name,
                            "description": s.description,
                            "enabled": s.enabled,
                            "options": s.options.as_ref().map(|opts| {
                                opts.iter().map(|o| serde_json::json!({
                                    "name": o.name,
                                    "type": o.option_type,
                                    "required": o.required.unwrap_or(false),
                                    "description": o.description,
                                })).collect::<Vec<_>>()
                            }),
                            "positional_args": s.positional_args.as_ref().map(|args| {
                                args.iter().map(|a| serde_json::json!({
                                    "name": a.name,
                                    "type": a.option_type,
                                    "required": a.required.unwrap_or(false),
                                    "description": a.description,
                                })).collect::<Vec<_>>()
                            }),
                        })
                    }).collect::<Vec<_>>()
                }),
                "install_instructions": config.install_instructions,
            })
        })
        .collect();

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Read a boolean env var ("1","true","yes","on" → true; anything else → false).
///
/// Public for use in tests.
pub fn env_flag_enabled(name: &str) -> bool {
    AppConfig::env_flag(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn init_test() {
        crate::utils::logging::init_test_logging();
    }

    // ─── env_flag_enabled ───────────────────────────────────────────────────

    #[test]
    fn test_env_flag_enabled_unset() {
        unsafe { std::env::remove_var("AHMA_TEST_FLAG_UNSET") };
        assert!(!env_flag_enabled("AHMA_TEST_FLAG_UNSET"));
    }

    #[test]
    fn test_env_flag_enabled_empty() {
        unsafe { std::env::set_var("AHMA_TEST_FLAG_EMPTY", "") };
        let result = env_flag_enabled("AHMA_TEST_FLAG_EMPTY");
        unsafe { std::env::remove_var("AHMA_TEST_FLAG_EMPTY") };
        assert!(!result);
    }

    #[test]
    fn test_env_flag_enabled_whitespace_only() {
        unsafe { std::env::set_var("AHMA_TEST_FLAG_WS", "   ") };
        let result = env_flag_enabled("AHMA_TEST_FLAG_WS");
        unsafe { std::env::remove_var("AHMA_TEST_FLAG_WS") };
        assert!(!result);
    }

    #[test]
    fn test_env_flag_enabled_true() {
        for val in ["1", "true", "True", "TRUE", "yes", "Yes", "on", "ON"] {
            unsafe { std::env::set_var("AHMA_TEST_FLAG_VAL", val) };
            let result = env_flag_enabled("AHMA_TEST_FLAG_VAL");
            unsafe { std::env::remove_var("AHMA_TEST_FLAG_VAL") };
            assert!(result, "env_flag_enabled({:?}) should be true", val);
        }
    }

    #[test]
    fn test_env_flag_enabled_false() {
        for val in ["0", "false", "no", "off", "x", ""] {
            if val.is_empty() {
                continue;
            }
            unsafe { std::env::set_var("AHMA_TEST_FLAG_FALSE", val) };
            let result = env_flag_enabled("AHMA_TEST_FLAG_FALSE");
            unsafe { std::env::remove_var("AHMA_TEST_FLAG_FALSE") };
            assert!(!result, "env_flag_enabled({:?}) should be false", val);
        }
    }

    #[test]
    fn test_env_sandbox_scopes_single_path() {
        let temp = tempdir().expect("Failed to create temp dir");
        unsafe { std::env::set_var("AHMA_SANDBOX_SCOPE", temp.path()) };
        let scopes = AppConfig::env_sandbox_scopes();
        unsafe { std::env::remove_var("AHMA_SANDBOX_SCOPE") };

        assert_eq!(scopes, vec![temp.path().to_path_buf()]);
    }

    #[test]
    fn test_env_working_dirs_multiple_paths() {
        let temp_a = tempdir().expect("Failed to create first temp dir");
        let temp_b = tempdir().expect("Failed to create second temp dir");
        let joined =
            std::env::join_paths([temp_a.path(), temp_b.path()]).expect("Failed to join path list");

        unsafe { std::env::set_var("AHMA_WORKING_DIRS", joined) };
        let dirs = AppConfig::env_working_dirs();
        unsafe { std::env::remove_var("AHMA_WORKING_DIRS") };

        assert_eq!(
            dirs,
            vec![temp_a.path().to_path_buf(), temp_b.path().to_path_buf()]
        );
    }

    // ─── resolve_sandbox_policy ──────────────────────────────────────────────

    fn make_cfg() -> AppConfig {
        AppConfig {
            tools_dir: None,
            explicit_tools_dir: false,
            tool_bundles: vec![],
            timeout_secs: 360,
            force_sync: false,
            hot_reload_tools: false,
            skip_availability_probes: false,
            progressive_disclosure: true,
            no_sandbox: false,
            sandbox_scopes: vec![],
            defer_sandbox: false,
            working_dirs: vec![],
            tmp_access: false,
            no_temp_files: false,
            log_monitor: false,
            monitor_rate_limit_secs: 60,
            http_host: "127.0.0.1".to_string(),
            http_port: 3000,
            no_quic: false,
            disable_http1_1: false,
            handshake_timeout_secs: 45,
            list_server: None,
            mcp_config: PathBuf::from("mcp.json"),
            list_http: None,
            list_format: list_tools::OutputFormat::Text,
            run_tool: None,
            run_tool_args: vec![],
        }
    }

    #[test]
    fn test_resolve_sandbox_policy_no_sandbox_flag() {
        init_test();
        let cfg = AppConfig {
            no_sandbox: true,
            ..make_cfg()
        };
        let policy = resolve_sandbox_policy(&cfg);
        assert!(policy.no_sandbox);
        assert_eq!(policy.mode, sandbox::SandboxMode::Test);
    }

    #[test]
    fn test_resolve_sandbox_policy_strict_by_default() {
        init_test();
        unsafe { std::env::remove_var("AHMA_DISABLE_SANDBOX") };
        let cfg = make_cfg();
        let policy = resolve_sandbox_policy(&cfg);
        assert!(!policy.no_sandbox);
        assert_eq!(policy.mode, sandbox::SandboxMode::Strict);
    }

    #[test]
    fn test_resolve_sandbox_policy_tmp_flag() {
        init_test();
        let cfg = AppConfig {
            tmp_access: true,
            ..make_cfg()
        };
        let policy = resolve_sandbox_policy(&cfg);
        assert!(policy.tmp_access);
    }

    #[test]
    fn test_resolve_sandbox_policy_ahma_tmp_access_env() {
        init_test();
        unsafe { std::env::set_var("AHMA_TMP_ACCESS", "1") };
        let cfg = AppConfig {
            tmp_access: AppConfig::env_flag("AHMA_TMP_ACCESS"),
            ..make_cfg()
        };
        unsafe { std::env::remove_var("AHMA_TMP_ACCESS") };
        let policy = resolve_sandbox_policy(&cfg);
        assert!(policy.tmp_access);
    }

    // ─── canonicalize_paths (via resolve_sandbox_scopes) ─────────────────────

    #[test]
    fn test_canonicalize_paths_via_sandbox_scope() {
        init_test();
        let tmp = tempdir().unwrap();
        let path = tmp.path().to_path_buf();
        let cfg = AppConfig {
            no_sandbox: true,
            sandbox_scopes: vec![path.clone()],
            ..make_cfg()
        };
        let scopes = resolve_sandbox_scopes(&cfg).unwrap();
        assert!(scopes.is_some());
        let scopes = scopes.unwrap();
        assert_eq!(scopes.len(), 1);
        assert_eq!(dunce::canonicalize(&path).unwrap(), scopes[0]);
    }

    #[test]
    fn test_canonicalize_paths_invalid_fails() {
        init_test();
        let cfg = AppConfig {
            no_sandbox: true,
            sandbox_scopes: vec![PathBuf::from("/nonexistent/path/that/does/not/exist")],
            ..make_cfg()
        };
        let result = resolve_sandbox_scopes(&cfg);
        assert!(result.is_err());
    }

    // ─── resolve_sandbox_scopes ──────────────────────────────────────────────

    #[test]
    fn test_resolve_sandbox_scopes_explicit() {
        init_test();
        let tmp = tempdir().unwrap();
        let cfg = AppConfig {
            no_sandbox: true,
            sandbox_scopes: vec![tmp.path().to_path_buf()],
            ..make_cfg()
        };
        let scopes = resolve_sandbox_scopes(&cfg).unwrap();
        assert!(scopes.is_some());
        assert_eq!(scopes.unwrap().len(), 1);
    }

    #[test]
    fn test_resolve_sandbox_scopes_ahma_sandbox_scope_env() {
        init_test();
        let tmp = tempdir().unwrap();
        let path = tmp.path().to_path_buf();
        // Simulate what build_app_config does: read env at config-build time
        let cfg = AppConfig {
            no_sandbox: true,
            sandbox_scopes: vec![path],
            ..make_cfg()
        };
        let result = resolve_sandbox_scopes(&cfg);
        assert!(result.is_ok());
        let scopes = result.unwrap();
        assert!(scopes.is_some());
        assert_eq!(scopes.unwrap().len(), 1);
    }

    #[test]
    fn test_resolve_sandbox_scopes_cwd_fallback() {
        init_test();
        let cfg = AppConfig {
            no_sandbox: true,
            sandbox_scopes: vec![],
            ..make_cfg()
        };
        let scopes = resolve_sandbox_scopes(&cfg).unwrap();
        assert!(scopes.is_some());
        assert_eq!(scopes.unwrap().len(), 1);
    }

    // ─── resolve_deferred_scopes ─────────────────────────────────────────────

    #[test]
    fn test_resolve_deferred_scopes_with_working_dirs() {
        init_test();
        let tmp = tempdir().unwrap();
        let cfg = AppConfig {
            no_sandbox: true,
            defer_sandbox: true,
            working_dirs: vec![tmp.path().to_path_buf()],
            ..make_cfg()
        };
        let scopes = resolve_deferred_scopes(&cfg).unwrap();
        assert!(scopes.is_some());
        let scopes = scopes.unwrap();
        assert_eq!(scopes.len(), 1);
        assert_eq!(dunce::canonicalize(tmp.path()).unwrap(), scopes[0]);
    }

    #[test]
    fn test_resolve_deferred_scopes_without_working_dirs() {
        init_test();
        let cfg = AppConfig {
            no_sandbox: true,
            defer_sandbox: true,
            ..make_cfg()
        };
        let scopes = resolve_deferred_scopes(&cfg).unwrap();
        assert!(scopes.is_some());
        assert!(scopes.unwrap().is_empty());
    }

    #[test]
    fn test_resolve_sandbox_scopes_defer_takes_precedence() {
        init_test();
        let tmp = tempdir().unwrap();
        let cfg = AppConfig {
            no_sandbox: true,
            defer_sandbox: true,
            working_dirs: vec![tmp.path().to_path_buf()],
            ..make_cfg()
        };
        let scopes = resolve_sandbox_scopes(&cfg).unwrap();
        assert!(scopes.is_some());
        assert_eq!(scopes.unwrap().len(), 1);
    }

    // ─── add_temp_scope_if_requested ────────────────────────────────────────

    #[test]
    fn test_add_temp_scope_no_tmp_returns_unchanged() {
        init_test();
        let tmp = tempdir().unwrap();
        let scopes = Some(vec![tmp.path().to_path_buf()]);
        let result = add_temp_scope_if_requested(scopes.clone(), false);
        assert_eq!(result, scopes);
    }

    #[test]
    fn test_add_temp_scope_with_tmp_adds_temp_dir() {
        init_test();
        let tmp = tempdir().unwrap();
        let scopes = Some(vec![tmp.path().to_path_buf()]);
        let result = add_temp_scope_if_requested(scopes, true);
        assert!(result.is_some());
        let result = result.unwrap();
        let temp_dir = std::env::temp_dir();
        let canonical_temp = dunce::canonicalize(&temp_dir).unwrap();
        assert!(
            result.contains(&canonical_temp),
            "Expected temp dir in scopes: {:?}",
            result
        );
    }

    #[test]
    fn test_add_temp_scope_none_returns_none_when_no_tmp() {
        let result = add_temp_scope_if_requested(None, false);
        assert!(result.is_none());
    }

    #[test]
    fn test_add_temp_scope_tmp_with_none_returns_none() {
        init_test();
        // When scopes is None, scopes? returns early; temp is only added to existing scopes
        let result = add_temp_scope_if_requested(None, true);
        assert!(result.is_none());
    }

    // ─── create_sandbox_instance & log_sandbox_mode ──────────────────────────

    #[test]
    fn test_create_sandbox_instance_none() {
        init_test();
        let cfg = AppConfig {
            no_sandbox: true,
            ..make_cfg()
        };
        let policy = resolve_sandbox_policy(&cfg);
        let sandbox = create_sandbox_instance(None, &policy, &cfg).unwrap();
        assert!(sandbox.is_none());
    }

    #[test]
    fn test_create_sandbox_instance_some() {
        init_test();
        let tmp = tempdir().unwrap();
        let scopes = Some(vec![tmp.path().to_path_buf()]);
        let cfg = AppConfig {
            no_sandbox: true,
            ..make_cfg()
        };
        let policy = resolve_sandbox_policy(&cfg);
        let sandbox = create_sandbox_instance(scopes, &policy, &cfg).unwrap();
        assert!(sandbox.is_some());
    }

    #[test]
    fn test_log_sandbox_mode_disabled() {
        init_test();
        log_sandbox_mode(true);
    }

    #[test]
    fn test_log_sandbox_mode_enabled() {
        init_test();
        log_sandbox_mode(false);
    }

    // ─── check_sandbox_availability ──────────────────────────────────────────

    #[test]
    fn test_check_sandbox_availability_ok_when_no_sandbox() {
        init_test();
        assert!(check_sandbox_availability(true).is_ok());
    }

    // ─── run_validation_mode ─────────────────────────────────────────────────

    #[test]
    fn test_run_validation_mode_valid_config() {
        init_test();
        let tmp = tempdir().unwrap();
        let tools_dir = tmp.path().join(".ahma");
        std::fs::create_dir_all(&tools_dir).unwrap();
        let valid_json = r#"{
            "name": "test_tool",
            "description": "Test",
            "command": "echo",
            "enabled": true,
            "subcommand": [{"name": "default", "description": "Default", "enabled": true}]
        }"#;
        let tool_file = tools_dir.join("test.json");
        std::fs::File::create(&tool_file)
            .unwrap()
            .write_all(valid_json.as_bytes())
            .unwrap();
        let target = tools_dir.to_str().unwrap();
        let result = run_validation_mode(target);
        assert!(result.is_ok(), "run_validation_mode failed: {:?}", result);
    }

    #[test]
    fn test_run_validation_mode_invalid_fails() {
        init_test();
        let tmp = tempdir().unwrap();
        let invalid_dir = tmp.path().join("nonexistent_validation_target");
        let result = run_validation_mode(invalid_dir.to_str().unwrap());
        assert!(result.is_err());
    }

    // ─── initialize_sandbox ───────────────────────────────────────────────────

    #[test]
    fn test_initialize_sandbox_no_sandbox() {
        init_test();
        let tmp = tempdir().unwrap();
        let cfg = AppConfig {
            no_sandbox: true,
            sandbox_scopes: vec![tmp.path().to_path_buf()],
            ..make_cfg()
        };
        let sandbox = initialize_sandbox(&cfg).unwrap();
        assert!(sandbox.is_some());
    }

    #[test]
    fn test_initialize_sandbox_defer_with_working_dirs() {
        init_test();
        let tmp = tempdir().unwrap();
        let cfg = AppConfig {
            no_sandbox: true,
            defer_sandbox: true,
            working_dirs: vec![tmp.path().to_path_buf()],
            ..make_cfg()
        };
        let sandbox = initialize_sandbox(&cfg).unwrap();
        assert!(sandbox.is_some());
    }

    // ─── check_stdio_not_interactive ─────────────────────────────────────────

    #[test]
    fn test_check_stdio_not_interactive() {
        init_test();
        // When tests are launched from an interactive terminal, this helper
        // intentionally exits the process. Only call it in the non-TTY case.
        if std::io::stdin().is_terminal() {
            return;
        }
        let result = check_stdio_not_interactive();
        assert!(result.is_ok());
    }

    // ─── CLI subcommand parsing ───────────────────────────────────────────────

    #[test]
    fn test_cli_parse_serve_stdio() {
        let cli = Cli::try_parse_from(["ahma-mcp", "serve", "stdio"]).unwrap();
        assert!(matches!(
            cli.command,
            Subcommands::Serve(ServeArgs {
                transport: ServeTransport::Stdio,
                ..
            })
        ));
    }

    #[test]
    fn test_cli_parse_serve_http_defaults() {
        let cli = Cli::try_parse_from(["ahma-mcp", "serve", "http"]).unwrap();
        if let Subcommands::Serve(ServeArgs {
            transport: ServeTransport::Http(h),
            ..
        }) = cli.command
        {
            assert_eq!(h.host, "127.0.0.1");
            assert_eq!(h.port, 3000);
            assert!(!h.no_quic);
        } else {
            panic!("expected serve http");
        }
    }

    #[test]
    fn test_cli_parse_serve_http_custom_port() {
        let cli = Cli::try_parse_from(["ahma-mcp", "serve", "http", "--port", "8080"]).unwrap();
        if let Subcommands::Serve(ServeArgs {
            transport: ServeTransport::Http(h),
            ..
        }) = cli.command
        {
            assert_eq!(h.port, 8080);
        } else {
            panic!("expected serve http");
        }
    }

    #[test]
    fn test_cli_parse_run_tool() {
        let cli =
            Cli::try_parse_from(["ahma-mcp", "tool", "run", "cargo_build", "--", "--release"])
                .unwrap();
        if let Subcommands::Tool(ToolArgs {
            command: ToolCommand::Run(r),
        }) = cli.command
        {
            assert_eq!(r.tool, "cargo_build");
            assert_eq!(r.tool_args, vec!["--release"]);
        } else {
            panic!("expected tool run subcommand");
        }
    }

    #[test]
    fn test_cli_parse_tool_validate_default() {
        let cli = Cli::try_parse_from(["ahma-mcp", "tool", "validate"]).unwrap();
        if let Subcommands::Tool(ToolArgs {
            command: ToolCommand::Validate(v),
        }) = cli.command
        {
            assert!(v.target.is_none());
        } else {
            panic!("expected tool validate");
        }
    }

    #[test]
    fn test_cli_parse_tool_validate_with_target() {
        let cli = Cli::try_parse_from(["ahma-mcp", "tool", "validate", ".ahma"]).unwrap();
        if let Subcommands::Tool(ToolArgs {
            command: ToolCommand::Validate(v),
        }) = cli.command
        {
            assert_eq!(v.target, Some(".ahma".to_string()));
        } else {
            panic!("expected tool validate with target");
        }
    }

    #[test]
    fn test_cli_parse_tool_list() {
        let cli = Cli::try_parse_from(["ahma-mcp", "tool", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Subcommands::Tool(ToolArgs {
                command: ToolCommand::List(_)
            })
        ));
    }

    #[test]
    fn test_cli_parse_serve_with_tool_bundle() {
        let cli =
            Cli::try_parse_from(["ahma-mcp", "serve", "stdio", "--tools", "rust,python"]).unwrap();
        if let Subcommands::Serve(s) = cli.command {
            assert!(s.tool_bundles.contains(&"rust".to_string()));
            assert!(s.tool_bundles.contains(&"python".to_string()));
        } else {
            panic!("expected serve stdio");
        }
    }

    // ─── AppConfig::env_flag ─────────────────────────────────────────────────

    #[test]
    fn test_app_config_env_flag_via_helper() {
        unsafe { std::env::set_var("AHMA_TEST_CFG_FLAG", "yes") };
        assert!(AppConfig::env_flag("AHMA_TEST_CFG_FLAG"));
        unsafe { std::env::remove_var("AHMA_TEST_CFG_FLAG") };
    }
}
