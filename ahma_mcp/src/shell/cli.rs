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

    // ─── resolve_sandbox_policy ──────────────────────────────────────────────

    #[test]
    fn test_resolve_sandbox_policy_no_sandbox_flag() {
        init_test();
        let cli = Cli::try_parse_from(["ahma_mcp", "--no-sandbox"]).unwrap();
        let policy = resolve_sandbox_policy(&cli);
        assert!(policy.no_sandbox);
        assert_eq!(policy.mode, sandbox::SandboxMode::Test);
    }

    #[test]
    fn test_resolve_sandbox_policy_strict_by_default() {
        init_test();
        unsafe { std::env::remove_var("AHMA_NO_SANDBOX") };
        let cli = Cli::try_parse_from(["ahma_mcp"]).unwrap();
        let policy = resolve_sandbox_policy(&cli);
        assert!(!policy.no_sandbox);
        assert_eq!(policy.mode, sandbox::SandboxMode::Strict);
    }

    #[test]
    fn test_resolve_sandbox_policy_tmp_flag() {
        init_test();
        let cli = Cli::try_parse_from(["ahma_mcp", "--tmp"]).unwrap();
        let policy = resolve_sandbox_policy(&cli);
        assert!(policy.tmp_access);
    }

    #[test]
    fn test_resolve_sandbox_policy_ahma_tmp_access_env() {
        init_test();
        unsafe { std::env::set_var("AHMA_TMP_ACCESS", "1") };
        let cli = Cli::try_parse_from(["ahma_mcp"]).unwrap();
        let policy = resolve_sandbox_policy(&cli);
        unsafe { std::env::remove_var("AHMA_TMP_ACCESS") };
        assert!(policy.tmp_access);
    }

    // ─── canonicalize_paths (via resolve_sandbox_scopes) ─────────────────────

    #[test]
    fn test_canonicalize_paths_via_sandbox_scope() {
        init_test();
        let tmp = tempdir().unwrap();
        let path = tmp.path().to_path_buf();
        let cli = Cli::try_parse_from([
            "ahma_mcp",
            "--no-sandbox",
            "--sandbox-scope",
            path.to_str().unwrap(),
        ])
        .unwrap();
        let scopes = resolve_sandbox_scopes(&cli).unwrap();
        assert!(scopes.is_some());
        let scopes = scopes.unwrap();
        assert_eq!(scopes.len(), 1);
        assert_eq!(dunce::canonicalize(&path).unwrap(), scopes[0]);
    }

    #[test]
    fn test_canonicalize_paths_invalid_fails() {
        init_test();
        let cli = Cli::try_parse_from([
            "ahma_mcp",
            "--no-sandbox",
            "--sandbox-scope",
            "/nonexistent/path/that/does/not/exist",
        ])
        .unwrap();
        let result = resolve_sandbox_scopes(&cli);
        assert!(result.is_err());
    }

    // ─── resolve_sandbox_scopes ──────────────────────────────────────────────

    #[test]
    fn test_resolve_sandbox_scopes_explicit() {
        init_test();
        let tmp = tempdir().unwrap();
        let cli = Cli::try_parse_from([
            "ahma_mcp",
            "--no-sandbox",
            "--sandbox-scope",
            tmp.path().to_str().unwrap(),
        ])
        .unwrap();
        let scopes = resolve_sandbox_scopes(&cli).unwrap();
        assert!(scopes.is_some());
        assert_eq!(scopes.unwrap().len(), 1);
    }

    #[test]
    fn test_resolve_sandbox_scopes_ahma_sandbox_scope_env() {
        init_test();
        let tmp = tempdir().unwrap();
        let path = tmp.path().to_path_buf();
        unsafe { std::env::set_var("AHMA_SANDBOX_SCOPE", path.as_os_str()) };
        let cli = Cli::try_parse_from(["ahma_mcp", "--no-sandbox"]).unwrap();
        let result = resolve_sandbox_scopes(&cli);
        unsafe { std::env::remove_var("AHMA_SANDBOX_SCOPE") };
        assert!(result.is_ok());
        let scopes = result.unwrap();
        assert!(scopes.is_some());
        assert_eq!(scopes.unwrap().len(), 1);
    }

    #[test]
    fn test_resolve_sandbox_scopes_cwd_fallback() {
        init_test();
        unsafe { std::env::remove_var("AHMA_SANDBOX_SCOPE") };
        let cli = Cli::try_parse_from(["ahma_mcp", "--no-sandbox"]).unwrap();
        let scopes = resolve_sandbox_scopes(&cli).unwrap();
        assert!(scopes.is_some());
        assert_eq!(scopes.unwrap().len(), 1);
    }

    // ─── resolve_deferred_scopes ─────────────────────────────────────────────

    #[test]
    fn test_resolve_deferred_scopes_with_working_dirs() {
        init_test();
        let tmp = tempdir().unwrap();
        let cli = Cli::try_parse_from([
            "ahma_mcp",
            "--no-sandbox",
            "--defer-sandbox",
            "--working-directories",
            tmp.path().to_str().unwrap(),
        ])
        .unwrap();
        let scopes = resolve_deferred_scopes(&cli).unwrap();
        assert!(scopes.is_some());
        let scopes = scopes.unwrap();
        assert_eq!(scopes.len(), 1);
        assert_eq!(dunce::canonicalize(tmp.path()).unwrap(), scopes[0]);
    }

    #[test]
    fn test_resolve_deferred_scopes_without_working_dirs() {
        init_test();
        let cli = Cli::try_parse_from(["ahma_mcp", "--no-sandbox", "--defer-sandbox"]).unwrap();
        let scopes = resolve_deferred_scopes(&cli).unwrap();
        assert!(scopes.is_some());
        assert!(scopes.unwrap().is_empty());
    }

    #[test]
    fn test_resolve_sandbox_scopes_defer_takes_precedence() {
        init_test();
        let tmp = tempdir().unwrap();
        let cli = Cli::try_parse_from([
            "ahma_mcp",
            "--no-sandbox",
            "--defer-sandbox",
            "--working-directories",
            tmp.path().to_str().unwrap(),
        ])
        .unwrap();
        let scopes = resolve_sandbox_scopes(&cli).unwrap();
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
        let cli = Cli::try_parse_from(["ahma_mcp", "--no-sandbox"]).unwrap();
        let policy = resolve_sandbox_policy(&cli);
        let sandbox = create_sandbox_instance(None, &policy, &cli).unwrap();
        assert!(sandbox.is_none());
    }

    #[test]
    fn test_create_sandbox_instance_some() {
        init_test();
        let tmp = tempdir().unwrap();
        let scopes = Some(vec![tmp.path().to_path_buf()]);
        let cli = Cli::try_parse_from(["ahma_mcp", "--no-sandbox"]).unwrap();
        let policy = resolve_sandbox_policy(&cli);
        let sandbox = create_sandbox_instance(scopes, &policy, &cli).unwrap();
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
        let mut cli = Cli::try_parse_from([
            "ahma_mcp",
            "--no-sandbox",
            "--sandbox-scope",
            tmp.path().to_str().unwrap(),
        ])
        .unwrap();
        let sandbox = initialize_sandbox(&mut cli).unwrap();
        assert!(sandbox.is_some());
    }

    #[test]
    fn test_initialize_sandbox_defer_with_working_dirs() {
        init_test();
        let tmp = tempdir().unwrap();
        let mut cli = Cli::try_parse_from([
            "ahma_mcp",
            "--no-sandbox",
            "--defer-sandbox",
            "--working-directories",
            tmp.path().to_str().unwrap(),
        ])
        .unwrap();
        let sandbox = initialize_sandbox(&mut cli).unwrap();
        assert!(sandbox.is_some());
    }

    // ─── check_stdio_not_interactive ─────────────────────────────────────────

    #[test]
    fn test_check_stdio_not_interactive() {
        init_test();
        // When run from cargo test, stdin is typically not a TTY
        let result = check_stdio_not_interactive();
        assert!(result.is_ok());
    }

    // ─── Cli struct / parsing ────────────────────────────────────────────────

    #[test]
    fn test_cli_parse_defaults() {
        let cli = Cli::try_parse_from(["ahma_mcp"]).unwrap();
        assert_eq!(cli.mode, "stdio");
        assert_eq!(cli.http_port, 3000);
        assert_eq!(cli.timeout, 360);
        assert!(cli.tool_name.is_none());
    }

    #[test]
    fn test_cli_parse_validate_option() {
        let cli = Cli::try_parse_from(["ahma_mcp", "--validate"]).unwrap();
        assert_eq!(cli.validate, Some(".ahma".to_string()));
    }

    #[test]
    fn test_cli_parse_tool_name() {
        let cli = Cli::try_parse_from(["ahma_mcp", "cargo_build", "--", "--release"]).unwrap();
        assert_eq!(cli.tool_name, Some("cargo_build".to_string()));
        assert_eq!(cli.tool_args, vec!["--release"]);
    }
}
