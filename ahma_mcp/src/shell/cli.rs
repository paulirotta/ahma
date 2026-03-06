//! # Ahma Server CLI
//!
//! This module contains the command-line interface definition and main entry point.

use super::{list_tools, modes, resolution};

use crate::{sandbox, utils::logging::init_logging};
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use dunce;
use std::{io::IsTerminal, path::PathBuf, sync::Arc};

/// Ahma Server: A generic, config-driven adapter for CLI tools.
#[derive(Parser, Debug, Clone)]
#[command(
    author,
    version,
    about,
    long_about = "ahma_mcp runs in four modes:

1. STDIO Mode (default): MCP server over stdio for direct integration.
   Example: ahma_mcp --mode stdio

2. HTTP Mode: HTTP bridge server that proxies to stdio MCP server.
   Example: ahma_mcp --mode http --http-port 3000

3. CLI Mode: Execute a single command and print result to stdout.
   Example: ahma_mcp cargo_build --working-directory . -- --release

4. List Tools Mode: List all tools from an MCP server.
   Example: ahma_mcp --list-tools -- /path/to/server
   Example: ahma_mcp --list-tools --http http://localhost:3000"
)]
pub struct Cli {
    /// List all tools from an MCP server and exit
    #[arg(long)]
    pub list_tools: bool,

    /// Name of the server in mcp.json to connect to (for --list-tools mode)
    #[arg(long)]
    pub server: Option<String>,

    /// Path to mcp.json configuration file (for --list-tools mode)
    #[arg(long, default_value = "mcp.json")]
    pub mcp_config: PathBuf,

    /// HTTP URL for --list-tools mode (e.g., http://localhost:3000)
    #[arg(long)]
    pub http: Option<String>,

    /// Output format for --list-tools mode
    #[arg(long, value_enum, default_value_t = list_tools::OutputFormat::Text)]
    pub format: list_tools::OutputFormat,

    /// Server mode: 'stdio' (default) or 'http'
    #[arg(long, default_value = "stdio")]
    pub mode: String,

    /// Path to the tools directory containing JSON configurations
    #[arg(long)]
    pub tools_dir: Option<PathBuf>,

    /// Whether --tools-dir was explicitly provided on the command line
    /// (as opposed to auto-detected via .ahma/ directory).
    /// Set automatically during CLI initialization; not a user-facing flag.
    #[arg(skip)]
    pub explicit_tools_dir: bool,

    /// Bundle and enable the rust toolset (rust.json)
    #[arg(long)]
    pub rust: bool,

    /// Bundle and enable the file tools (file-tools.json)
    #[arg(long)]
    pub fileutils: bool,

    /// Bundle and enable the github toolset (gh.json)
    #[arg(long)]
    pub github: bool,

    /// Bundle and enable the git toolset (git.json)
    #[arg(long)]
    pub git: bool,

    /// Bundle and enable the gradle toolset (gradlew.json)
    #[arg(long)]
    pub gradle: bool,

    /// Bundle and enable the python toolset (python.json)
    #[arg(long)]
    pub python: bool,

    /// Bundle and enable the simplify AI tool (simplify.json)
    #[arg(long)]
    pub simplify: bool,

    /// Default timeout for tool execution in seconds
    #[arg(long, default_value_t = 360)]
    pub timeout: u64,

    /// Force all tools to run synchronously (disable async execution)
    #[arg(long)]
    pub sync: bool,

    /// Enable debug logging
    #[arg(long)]
    pub debug: bool,

    /// Log to stderr instead of file
    #[arg(long)]
    pub log_to_stderr: bool,

    /// Disable sandbox (for testing only - UNSAFE)
    #[arg(long)]
    pub no_sandbox: bool,

    /// Skip tool availability probes at startup (faster startup for testing)
    #[arg(long)]
    pub skip_availability_probes: bool,

    /// Block writes to /tmp and other temp directories (higher security, breaks tools needing temp access)
    #[arg(long)]
    pub no_temp_files: bool,

    /// Sandbox scope directories (multiple allowed)
    #[arg(long = "sandbox-scope")]
    pub sandbox_scope: Vec<PathBuf>,

    /// Defer sandbox initialization until client provides roots
    #[arg(long)]
    pub defer_sandbox: bool,

    /// Minimum seconds between successive log monitoring alerts (default: 60)
    #[arg(long, default_value_t = 60)]
    pub monitor_rate_limit: u64,

    /// Disable progressive disclosure (expose all tools immediately instead of on demand)
    #[arg(long)]
    pub no_progressive_disclosure: bool,

    /// Safe live log monitoring: evaluate symlinks in /log to configure read-only scopes
    #[arg(long)]
    pub livelog: bool,

    /// Working directories for sandbox scope when using --defer-sandbox.
    /// Required when MCP client may not provide workspace roots.
    /// Example: --working-directories "/path/to/project1,/path/to/project2"
    #[arg(long, value_delimiter = ',')]
    pub working_directories: Option<Vec<PathBuf>>,

    /// HTTP server host (for HTTP mode)
    #[arg(long, default_value = "127.0.0.1")]
    pub http_host: String,

    /// HTTP server port (for HTTP mode)
    #[arg(long, default_value_t = 3000)]
    pub http_port: u16,

    /// Handshake timeout in seconds (for HTTP mode)
    #[arg(long, default_value_t = 10)]
    pub handshake_timeout_secs: u64,

    /// Tool name (for CLI mode)
    #[arg(value_name = "TOOL")]
    pub tool_name: Option<String>,

    /// Tool arguments (for CLI mode, after --)
    #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
    pub tool_args: Vec<String>,
}

pub async fn run() -> Result<()> {
    let mut cli = Cli::parse();
    cli.explicit_tools_dir = cli.tools_dir.is_some();
    cli.tools_dir = resolution::normalize_tools_dir(cli.tools_dir);

    // Initialize logging
    let log_level = if cli.debug { "debug" } else { "info" };
    let log_to_file = !cli.log_to_stderr;

    init_logging(log_level, log_to_file)?;

    // Handle --list-tools mode early (before sandbox initialization)
    // This mode doesn't execute tools locally, so it doesn't need sandbox
    if cli.list_tools {
        tracing::info!("Running in list-tools mode");
        return modes::run_list_tools_mode(&cli).await;
    }

    // Resolve sandbox policy from flags/env.
    let no_sandbox_requested = cli.no_sandbox || env_flag_enabled("AHMA_NO_SANDBOX");

    #[allow(unused_mut)] // mut needed for macOS nested sandbox detection
    let mut no_sandbox = no_sandbox_requested;
    cli.no_sandbox = no_sandbox;

    // Determine Sandbox Mode
    let sandbox_mode = if no_sandbox {
        tracing::warn!("Ahma sandbox disabled via --no-sandbox flag or environment variable");
        #[cfg(target_os = "linux")]
        {
            if let Err(error) = sandbox::check_sandbox_prerequisites() {
                tracing::warn!(
                    "Continuing without Ahma sandbox because Linux sandbox prerequisites are unavailable: {}. \
                     Update Linux kernel to 5.13+ to enable Landlock.",
                    error
                );
            }
        }
        sandbox::SandboxMode::Test
    } else {
        sandbox::SandboxMode::Strict
    };

    if !no_sandbox {
        // Check sandbox prerequisites before anything else
        if let Err(e) = sandbox::check_sandbox_prerequisites() {
            sandbox::exit_with_sandbox_error(&e);
        }

        // On macOS, test if sandbox-exec can actually be applied
        // This detects when running inside another sandbox (Cursor, VS Code, Docker)
        #[cfg(target_os = "macos")]
        {
            if let Err(e) = sandbox::test_sandbox_exec_available() {
                match e {
                    crate::sandbox::SandboxError::NestedSandboxDetected => {
                        // Per R7.6.2: Refuse to start in nested environment without explicit override
                        sandbox::exit_with_sandbox_error(&e);
                    }
                    _ => {
                        // Other sandbox errors should be fatal
                        sandbox::exit_with_sandbox_error(&e);
                    }
                }
            }
        }
    }

    // Initialize sandbox scopes unless deferred.
    // Priority:
    // 1. CLI --sandbox-scope (multiple supported)
    // 2. CLI --working-directories (for deferred mode)
    // 3. AHMA_SANDBOX_SCOPE env var (legacy single path)
    // 4. Current working directory
    let sandbox_scopes = if cli.defer_sandbox {
        // When using --defer-sandbox, check if --working-directories was provided
        if let Some(ref dirs) = cli.working_directories {
            // Use provided working-directories as initial scope
            let mut canonical_scopes = Vec::new();
            for scope in dirs {
                let canonical = dunce::canonicalize(scope).with_context(|| {
                    format!("Failed to canonicalize working directory: {:?}", scope)
                })?;
                canonical_scopes.push(canonical);
            }
            tracing::info!(
                "Sandbox initialized from --working-directories: {:?}",
                canonical_scopes
            );
            Some(canonical_scopes)
        } else {
            // No working-directories provided - start with empty scope
            // Sandbox will be configured from client roots/list
            tracing::info!("Sandbox initialization deferred - will be set from client roots/list");
            Some(Vec::new())
        }
    } else if !cli.sandbox_scope.is_empty() {
        // CLI override takes precedence
        let mut canonical_scopes = Vec::new();
        for scope in &cli.sandbox_scope {
            let canonical = dunce::canonicalize(scope)
                .with_context(|| format!("Failed to canonicalize sandbox scope: {:?}", scope))?;
            canonical_scopes.push(canonical);
        }
        Some(canonical_scopes)
    } else if let Ok(env_scope) = std::env::var("AHMA_SANDBOX_SCOPE") {
        // Environment variable is second priority
        let env_path = PathBuf::from(&env_scope);
        let canonical = dunce::canonicalize(&env_path).with_context(|| {
            format!(
                "Failed to canonicalize AHMA_SANDBOX_SCOPE environment variable: {:?}",
                env_scope
            )
        })?;
        Some(vec![canonical])
    } else {
        // Default to current working directory
        let cwd = std::env::current_dir()
            .context("Failed to get current working directory for sandbox scope")?;
        Some(vec![cwd])
    };

    // Create the Sandbox instance
    let sandbox = if let Some(scopes) = sandbox_scopes {
        let s = sandbox::Sandbox::new(scopes.clone(), sandbox_mode, cli.no_temp_files, cli.livelog)
            .context("Failed to initialize sandbox")?;

        tracing::info!("Sandbox scopes initialized: {:?}", scopes);

        // Apply kernel-level restrictions on Linux
        #[cfg(target_os = "linux")]
        {
            if sandbox_mode == sandbox::SandboxMode::Strict
                && !cli.defer_sandbox
                && let Err(e) = sandbox::enforce_landlock_sandbox(
                    &s.scopes(),
                    s.read_scopes(),
                    s.is_no_temp_files(),
                )
            {
                tracing::error!("Failed to enforce Landlock sandbox: {}", e);
                return Err(e);
            }
        }

        // Apply Windows Job Object enforcement as defense-in-depth.
        // This ensures all child processes are killed when the server exits.
        // Non-fatal: if the process is already inside an outer job (CI,
        // Task Scheduler) we log a warning and continue.
        #[cfg(target_os = "windows")]
        {
            match sandbox::enforce_windows_sandbox(&s.scopes()) {
                Ok(()) => {}
                Err(e) => tracing::warn!("Windows Job Object enforcement failed: {}", e),
            }
        }

        Some(Arc::new(s))
    } else {
        None
    };

    // Log the active sandbox mode for clarity
    if no_sandbox {
        tracing::info!("🔓 Sandbox mode: DISABLED (commands run without Ahma sandboxing)");
    } else {
        #[cfg(target_os = "linux")]
        tracing::info!(
            "SECURE Sandbox mode: LANDLOCK (Linux kernel-level file system restrictions)"
        );
        #[cfg(target_os = "macos")]
        tracing::info!(
            "SECURE Sandbox mode: SEATBELT (macOS sandbox-exec per-command restrictions)"
        );
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

    // R6.3.6: On Windows, PowerShell Core (pwsh) is the required runtime shell.
    // Probe for it early so the user gets an actionable error instead of a
    // generic "file not found" later when the shell pool tries to spawn it.
    #[cfg(target_os = "windows")]
    {
        let pwsh_check = std::process::Command::new("pwsh").arg("--version").output();
        match pwsh_check {
            Ok(out) if out.status.success() => {
                let ver = String::from_utf8_lossy(&out.stdout);
                tracing::info!("PowerShell Core detected: {}", ver.trim());
            }
            _ => {
                eprintln!(
                    "\nFAIL Error: PowerShell Core (pwsh) was not found.\n\n\
                     ahma_mcp requires PowerShell 7+ as its runtime shell on Windows.\n\n\
                     Install it with:\n\
                     \n  winget install Microsoft.PowerShell\n\
                     \nOr download from: https://github.com/PowerShell/PowerShell/releases\n"
                );
                std::process::exit(1);
            }
        }
    }

    // Determine mode based on CLI arguments
    let is_server_mode = cli.tool_name.is_none();

    if is_server_mode {
        match cli.mode.as_str() {
            "http" => {
                tracing::info!("Running in HTTP bridge mode");
                modes::run_http_bridge_mode(cli).await
            }
            "stdio" => {
                let sandbox = sandbox
                    .ok_or_else(|| anyhow!("Sandbox scopes must be initialized for stdio mode"))?;
                // Check if stdin is a terminal (interactive mode)
                if std::io::stdin().is_terminal() {
                    eprintln!(
                        "\nFAIL Error: ahma_mcp is an MCP server designed for JSON-RPC communication over stdio.\n"
                    );
                    eprintln!("It cannot be run directly from an interactive terminal.\n");
                    eprintln!("Usage options:");
                    eprintln!("  1. Run as stdio MCP server (requires MCP client):");
                    eprintln!("     ahma_mcp --mode stdio\n");
                    eprintln!("  2. Run as HTTP bridge server:");
                    eprintln!("     ahma_mcp --mode http --http-port 3000\n");
                    eprintln!("  3. Execute a single tool command:");
                    eprintln!("     ahma_mcp <tool_name> [tool_arguments...]\n");
                    eprintln!("For more information, run: ahma_mcp --help\n");
                    std::process::exit(1);
                }

                tracing::info!("Running in STDIO server mode");
                modes::run_server_mode(cli, sandbox).await
            }
            _ => {
                eprintln!("Invalid mode: {}. Use 'stdio' or 'http'", cli.mode);
                std::process::exit(1);
            }
        }
    } else {
        let sandbox =
            sandbox.ok_or_else(|| anyhow!("Sandbox scopes must be initialized for CLI mode"))?;
        tracing::info!("Running in CLI mode");
        modes::run_cli_mode(cli, sandbox).await
    }
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return false;
            }

            matches!(
                trimmed.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}
