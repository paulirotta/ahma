//! # Ahma Server CLI
//!
//! This module contains the command-line interface definition and main entry point.

use super::{list_tools, modes, resolution};

use crate::{sandbox, utils::logging::init_logging};
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use dunce;
use std::{io::IsTerminal, path::PathBuf, sync::Arc};

// ─────────────────────────────────────────────────────────────────────────────
// Helper types and functions to reduce run() complexity
// ─────────────────────────────────────────────────────────────────────────────

struct SandboxPolicy {
    no_sandbox: bool,
    tmp_access: bool,
    mode: sandbox::SandboxMode,
}

fn resolve_sandbox_policy(cli: &Cli) -> SandboxPolicy {
    let no_sandbox = cli.no_sandbox || env_flag_enabled("AHMA_NO_SANDBOX");
    let tmp_access = cli.tmp || env_flag_enabled("AHMA_TMP_ACCESS");

    let mode = if no_sandbox {
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

fn resolve_sandbox_scopes(cli: &Cli) -> Result<Option<Vec<PathBuf>>> {
    if cli.defer_sandbox {
        return resolve_deferred_scopes(cli);
    }

    if !cli.sandbox_scope.is_empty() {
        let scopes = canonicalize_paths(&cli.sandbox_scope, "sandbox scope")?;
        return Ok(Some(scopes));
    }

    if let Ok(env_scope) = std::env::var("AHMA_SANDBOX_SCOPE") {
        let env_path = PathBuf::from(&env_scope);
        let canonical = dunce::canonicalize(&env_path).with_context(|| {
            format!(
                "Failed to canonicalize AHMA_SANDBOX_SCOPE environment variable: {:?}",
                env_scope
            )
        })?;
        return Ok(Some(vec![canonical]));
    }

    let cwd = std::env::current_dir()
        .context("Failed to get current working directory for sandbox scope")?;
    Ok(Some(vec![cwd]))
}

fn resolve_deferred_scopes(cli: &Cli) -> Result<Option<Vec<PathBuf>>> {
    if let Some(ref dirs) = cli.working_directories {
        let scopes = canonicalize_paths(dirs, "working directory")?;
        tracing::info!(
            "Sandbox initialized from --working-directories: {:?}",
            scopes
        );
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
                "Adding temp directory to sandbox scopes via --tmp: {:?}",
                canonical_temp
            );
            scopes.push(canonical_temp);
        }
        Ok(_) => {}
        Err(_) => {
            tracing::warn!(
                "Could not canonicalize temp directory {:?}, skipping --tmp scope addition",
                temp_dir
            );
        }
    }

    Some(scopes)
}

fn create_sandbox_instance(
    scopes: Option<Vec<PathBuf>>,
    policy: &SandboxPolicy,
    cli: &Cli,
) -> Result<Option<Arc<sandbox::Sandbox>>> {
    let Some(scopes) = scopes else {
        return Ok(None);
    };

    let s = sandbox::Sandbox::new(scopes.clone(), policy.mode, cli.no_temp_files, cli.livelog)
        .context("Failed to initialize sandbox")?;

    tracing::info!("Sandbox scopes initialized: {:?}", scopes);

    apply_platform_sandbox_enforcement(&s, policy, cli)?;

    Ok(Some(Arc::new(s)))
}

fn apply_platform_sandbox_enforcement(
    sandbox: &sandbox::Sandbox,
    policy: &SandboxPolicy,
    cli: &Cli,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if policy.mode == sandbox::SandboxMode::Strict && !cli.defer_sandbox {
            if let Err(e) = sandbox::enforce_landlock_sandbox(
                &sandbox.scopes(),
                sandbox.read_scopes(),
                sandbox.is_no_temp_files(),
            ) {
                tracing::error!("Failed to enforce Landlock sandbox: {}", e);
                return Err(e);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Err(e) = sandbox::enforce_windows_sandbox(&sandbox.scopes()) {
            tracing::warn!("Windows Job Object enforcement failed: {}", e);
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let _ = (sandbox, policy, cli);

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

async fn dispatch_mode(cli: Cli, sandbox: Option<Arc<sandbox::Sandbox>>) -> Result<()> {
    let is_server_mode = cli.tool_name.is_none();

    if !is_server_mode {
        let sandbox =
            sandbox.ok_or_else(|| anyhow!("Sandbox scopes must be initialized for CLI mode"))?;
        tracing::info!("Running in CLI mode");
        return modes::run_cli_mode(cli, sandbox).await;
    }

    match cli.mode.as_str() {
        "http" => {
            tracing::info!("Running in HTTP bridge mode");
            modes::run_http_bridge_mode(cli).await
        }
        "stdio" => {
            let sandbox = sandbox
                .ok_or_else(|| anyhow!("Sandbox scopes must be initialized for stdio mode"))?;
            check_stdio_not_interactive()?;
            tracing::info!("Running in STDIO server mode");
            modes::run_server_mode(cli, sandbox).await
        }
        _ => {
            eprintln!("Invalid mode: {}. Use 'stdio' or 'http'", cli.mode);
            std::process::exit(1);
        }
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
    eprintln!("     ahma_mcp --mode stdio\n");
    eprintln!("  2. Run as HTTP bridge server:");
    eprintln!("     ahma_mcp --mode http --http-port 3000\n");
    eprintln!("  3. Execute a single tool command:");
    eprintln!("     ahma_mcp <tool_name> [tool_arguments...]\n");
    eprintln!("For more information, run: ahma_mcp --help\n");
    std::process::exit(1);
}

/// Ahma Server: A generic, config-driven adapter for CLI tools.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "ahma-mcp",
    author,
    version,
    about,
    long_about = "ahma_mcp runs in five modes:

1. STDIO Mode (default): MCP server over stdio for direct integration.
   Example: ahma_mcp --mode stdio

2. HTTP Mode: HTTP bridge server that proxies to stdio MCP server.
   Example: ahma_mcp --mode http --http-port 3000

3. CLI Mode: Execute a single command and print result to stdout.
   Example: ahma_mcp cargo_build --working-directory . -- --release

4. List Tools Mode: List all tools from an MCP server.
   Example: ahma_mcp --list-tools -- /path/to/server
   Example: ahma_mcp --list-tools --http http://localhost:3000

5. Validate Mode: Validate tool configurations against MTDF schema.
   Example: ahma_mcp --validate
   Example: ahma_mcp --validate .ahma
   Example: ahma_mcp --validate tool.json"
)]
pub struct Cli {
    /// List all tools from an MCP server and exit
    #[arg(long)]
    pub list_tools: bool,

    /// Validate tool configurations against the MTDF schema and exit.
    /// Optionally specify a target path (file, directory, or comma-separated list).
    /// Defaults to '.ahma' if no target is given.
    #[arg(long, value_name = "TARGET", default_missing_value = ".ahma", num_args = 0..=1)]
    pub validate: Option<String>,

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

    /// Add system temp directory to sandbox scopes (explicit temp access for testing/dynamic workflows)
    #[arg(long)]
    pub tmp: bool,

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

    let log_level = if cli.debug { "debug" } else { "info" };
    init_logging(log_level, !cli.log_to_stderr)?;

    if let Some(early_result) = handle_early_exit_modes(&cli).await {
        return early_result;
    }

    let sandbox = initialize_sandbox(&mut cli)?;

    #[cfg(target_os = "windows")]
    check_powershell_available();

    dispatch_mode(cli, sandbox).await
}

async fn handle_early_exit_modes(cli: &Cli) -> Option<Result<()>> {
    if cli.list_tools {
        tracing::info!("Running in list-tools mode");
        return Some(modes::run_list_tools_mode(cli).await);
    }
    if let Some(ref target) = cli.validate {
        tracing::info!("Running in validate mode");
        return Some(run_validation_mode(target));
    }
    None
}

fn initialize_sandbox(cli: &mut Cli) -> Result<Option<Arc<sandbox::Sandbox>>> {
    let policy = resolve_sandbox_policy(cli);
    cli.no_sandbox = policy.no_sandbox;

    check_sandbox_availability(policy.no_sandbox)?;

    let scopes = resolve_sandbox_scopes(cli)?;
    let scopes = add_temp_scope_if_requested(scopes, policy.tmp_access);
    let sandbox = create_sandbox_instance(scopes, &policy, cli)?;

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
