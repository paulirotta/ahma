//! # Tool Configuration Management
//!
//! This module defines the data structures and logic for managing the configuration of
//! command-line tools. All tool configurations are loaded from `.json` files
//! located in the `.ahma/` directory. This approach allows for easy extension and
//! modification of supported tools without altering the core server code.
//!
//! ## Core Data Structures
//!
//! - **`Config`**: The main struct representing the complete configuration for a single
//!   tool. It includes the tool's name, the actual command to execute, and whether the
//!   tool is enabled. It also contains nested structures for more granular control.
//!
//! - **`ToolHints`**: A collection of strings intended to provide guidance to an AI agent
//!   on how to use the tool effectively. It includes hints for specific operations like
//!   `build` and `test`, as well as custom hints for any subcommand.
//!
//! - **`CommandOverride`**: Allows for overriding default behaviors for specific subcommands.
//!   For example, a `test` subcommand could be given a longer timeout or be forced to run
//!   synchronously, even if the CLI --async flag is set.
//!
//! ## Configuration Loading
//!
//! - The `load_from_file` function reads a specified JSON file and deserializes it
//!   into a `Config` struct.
//! - The `load_tool_config` helper function simplifies loading by constructing the path
//!   to a tool's configuration file within the `.ahma/` directory.
//!
//! ## Key Features
//!
//! - **Declarative Tool Definition**: Tools are defined entirely through JSON,
//!   making the system highly modular and easy to maintain.
//! - **Hierarchical Configuration**: Settings can be applied globally (in `Config`), per
//!   operation type (in `ToolHints`), or per specific subcommand (in `CommandOverride`),
//!   providing a flexible and powerful configuration cascade.
//! - **AI Guidance**: The `ToolHints` system is a key feature for improving the performance
//!   of AI agents using the tools, providing them with contextual advice.
//! - **Dynamic Behavior**: The server's behavior, such as whether a command runs
//!   synchronously or asynchronously, can be controlled directly from the configuration files.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{collections::HashMap, path::Path};

/// Represents the complete configuration for a command-line tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subcommand: Option<Vec<SubcommandConfig>>,
    /// Generated input schema (optional - auto-generated from subcommands)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// Default timeout for operations in seconds
    pub timeout_seconds: Option<u64>,
    /// Override the default execution mode for this tool.
    /// - `true`: Always run synchronously (blocking, returns result immediately)
    /// - `false`: Always run asynchronously (non-blocking, returns operation ID)
    /// - `null`/omitted: Use server default (async unless --sync CLI flag)
    ///
    /// Inheritance: Subcommand-level settings override tool-level settings.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "force_synchronous"
    )]
    pub synchronous: Option<bool>,
    #[serde(default)]
    pub hints: ToolHints,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Key to look up hardcoded guidance
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guidance_key: Option<String>,
    /// Optional sequence of tools to execute in order (for composite tools)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<Vec<SequenceStep>>,
    /// Delay in milliseconds between sequence steps (default: SEQUENCE_STEP_DELAY_MS)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_delay_ms: Option<u64>,
    /// Runtime availability probe executed at server startup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability_check: Option<AvailabilityCheck>,
    /// Installation guidance displayed when the tool is unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_instructions: Option<String>,
    /// Default log monitor level for this tool. When set, all invocations of this tool
    /// will have live log monitoring enabled at this severity.
    /// Values: "error", "warn", "info", "debug", "trace"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_level: Option<String>,
    /// Which stream to monitor for log patterns (default: "stderr").
    /// Values: "stderr", "stdout", "both"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_stream: Option<String>,
}

/// Configuration for a subcommand, allowing for nested commands.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct SubcommandConfig {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subcommand: Option<Vec<SubcommandConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<CommandOption>>,
    /// Optional arguments that are not flags, but positional values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positional_args: Option<Vec<CommandOption>>,
    /// When true, positional args are placed BEFORE options in the command line.
    /// Required for commands like `find` where path must precede expressions.
    /// Default: false (options come first, then positional args)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positional_args_first: Option<bool>,
    /// Override timeout for this specific subcommand
    pub timeout_seconds: Option<u64>,
    /// Override the default execution mode for this subcommand.
    /// - `true`: Always run synchronously (blocking, returns result immediately)
    /// - `false`: Always run asynchronously (non-blocking, returns operation ID)
    /// - `null`/omitted: Inherit from tool level, or use server default if tool doesn't specify
    ///
    /// Inheritance: Subcommand-level settings override tool-level settings.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "force_synchronous"
    )]
    pub synchronous: Option<bool>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Key to look up hardcoded guidance
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guidance_key: Option<String>,
    /// Optional sequence of tools to execute in order (for composite tools)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<Vec<SequenceStep>>,
    /// Delay in milliseconds between sequence steps (default: SEQUENCE_STEP_DELAY_MS)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_delay_ms: Option<u64>,
    /// Runtime availability probe executed at server startup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability_check: Option<AvailabilityCheck>,
    /// Installation guidance displayed when the subcommand is unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_instructions: Option<String>,
}

/// Configuration for a single command-line option.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandOption {
    pub name: String,
    #[serde(rename = "type")]
    pub option_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<ItemsSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_arg: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_flag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// Schema details for array items.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemsSpec {
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Provides hints to an AI agent on how to use a tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolHints {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clean: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<HashMap<String, String>>,
}

/// Defines how to probe for tool or subcommand availability at runtime.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AvailabilityCheck {
    /// Optional override for the executable to invoke during the check. Defaults to the tool command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments passed to the availability probe.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Working directory used for the probe (defaults to project root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    /// Exit codes considered successful (defaults to `[0]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_exit_codes: Option<Vec<i32>>,
    /// If true, do not append derived subcommand arguments when constructing the probe command.
    #[serde(default, skip_serializing_if = "is_false")]
    pub skip_subcommand_args: bool,
}

/// Represents a single step in a tool sequence
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SequenceStep {
    /// Name of the tool to invoke
    pub tool: String,
    /// Subcommand within that tool
    pub subcommand: String,
    /// Arguments to pass to the tool
    #[serde(default)]
    pub args: Map<String, Value>,
    /// Optional description for logging/display
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Skip this step if the specified file exists
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_if_file_exists: Option<String>,
    /// Skip this step if the specified file is missing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_if_file_missing: Option<String>,
}

/// Top-level MCP client configuration (mcp.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    /// Server entries keyed by logical name.
    pub servers: HashMap<String, ServerConfig>,
}

/// Configuration for a single MCP server entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerConfig {
    /// Spawn an MCP server as a child process over stdio.
    #[serde(rename = "child_process")]
    ChildProcess(ChildProcessConfig),
    /// Connect to an MCP server over HTTP/SSE.
    #[serde(rename = "http")]
    Http(HttpServerConfig),
}

/// Child-process transport configuration for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildProcessConfig {
    /// Executable path or command name.
    pub command: String,
    /// Arguments passed to the MCP server process.
    pub args: Vec<String>,
}

/// HTTP transport configuration for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpServerConfig {
    /// Base URL of the MCP HTTP endpoint.
    pub url: String,
    /// Optional OAuth client id for Atlassian flows.
    pub atlassian_client_id: Option<String>,
    /// Optional OAuth client secret for Atlassian flows.
    pub atlassian_client_secret: Option<String>,
}

// Helper functions for serde defaults
fn default_enabled() -> bool {
    true
}

fn is_false(b: &bool) -> bool {
    !*b
}

pub async fn load_mcp_config(config_path: &Path) -> anyhow::Result<McpConfig> {
    if !tokio::fs::try_exists(config_path).await.unwrap_or(false) {
        return Ok(McpConfig {
            servers: HashMap::new(),
        });
    }

    let contents = tokio::fs::read_to_string(config_path).await?;
    let config: McpConfig = serde_json::from_str(&contents)?;
    Ok(config)
}

/// Load all tool configurations from a directory (async version)
///
/// This function scans the specified directory for JSON files and attempts to
/// deserialize each one into a `ToolConfig`. If the directory doesn't exist or
/// is empty, an empty HashMap is returned. It also loads built-in tools based on CLI flags.
///
/// # Arguments
/// * `cli` - Current CLI arguments to determine which tools are built-in
/// * `tools_dir` - Optional path to the directory containing tool configuration files.
///   When `None`, only bundled (CLI-flag) tools and the synthetic `sandboxed_shell` config
///   are loaded. This ensures `--rust`, `--simplify`, etc. work even without a `.ahma/` dir.
///
/// # Returns
/// * `Result<HashMap<String, ToolConfig>>` - Map of tool name to configuration or error
pub async fn load_tool_configs(
    cli: &crate::shell::cli::Cli,
    tools_dir: Option<&Path>,
) -> anyhow::Result<HashMap<String, ToolConfig>> {
    use std::time::Duration;
    use tokio::fs;
    use tokio::time;

    // Core tool names implemented directly in Rust code (mcp_service handlers).
    // These must not be overridden by user or bundled JSON configurations.
    const RESERVED_TOOL_NAMES: &[&str] = &["await", "status", "sandboxed_shell", "cancel"];

    let all_dirs: Vec<std::path::PathBuf> =
        tools_dir.map(|p| vec![p.to_path_buf()]).unwrap_or_default();

    let mut configs = HashMap::new();

    async fn read_tool_config_with_retry(path: &Path) -> Option<ToolConfig> {
        // Filesystem watchers can fire before a newly-written JSON file is fully durable.
        // A short bounded retry makes dynamic reload reliable without slowing normal startup.
        const MAX_ATTEMPTS: usize = 8;
        const BACKOFF_MS: u64 = 40;

        for attempt in 1..=MAX_ATTEMPTS {
            match fs::read_to_string(path).await {
                Ok(contents) => match serde_json::from_str::<ToolConfig>(&contents) {
                    Ok(config) => return Some(config),
                    Err(e) => {
                        if attempt == MAX_ATTEMPTS {
                            tracing::warn!("Failed to parse {}: {}", path.display(), e);
                            return None;
                        }
                    }
                },
                Err(e) => {
                    if attempt == MAX_ATTEMPTS {
                        tracing::warn!("Failed to read {}: {}", path.display(), e);
                        return None;
                    }
                }
            }

            time::sleep(Duration::from_millis(BACKOFF_MS)).await;
        }

        None
    }

    // When --tools-dir is explicitly provided, load all tools from it regardless
    // of bundle flags. When .ahma/ is auto-detected, also load ALL local tools
    // regardless of bundle flags. Bundle flags only control which built-in
    // (compiled-in) fallback definitions are activated; local .ahma/ definitions
    // always take precedence and are always fully loaded.

    for dir in all_dirs {
        if !fs::try_exists(&dir).await.unwrap_or(false) {
            continue;
        }

        // Read directory entries
        let mut entries = match fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    "Skipping inaccessible tools directory '{}': {}",
                    dir.display(),
                    e
                );
                continue;
            }
        };

        loop {
            match entries.next_entry().await {
                Ok(Some(entry)) => {
                    let path = entry.path();

                    // Only process .json files
                    if path.extension().and_then(|s| s.to_str()) == Some("json")
                        && let Some(config) = read_tool_config_with_retry(&path).await
                    {
                        // Guard rail: Check for conflicts with hardcoded tool names
                        if RESERVED_TOOL_NAMES.contains(&config.name.as_str()) {
                            anyhow::bail!(
                                "Tool name '{}' conflicts with a hardcoded system tool. Reserved tool names: {:?}. Please rename your tool in {}",
                                config.name,
                                RESERVED_TOOL_NAMES,
                                path.display()
                            );
                        }

                        // Include all tools (enabled or disabled)
                        // Disabled tools will be rejected at execution time with a helpful message
                        configs.insert(config.name.clone(), config);
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!("Error reading entry in '{}': {}", dir.display(), e);
                    break;
                }
            }
        }
    }

    // Load built-in tools based on CLI flags
    let mut builtin_tools: Vec<(&str, &str)> = Vec::new();

    if cli.rust {
        builtin_tools.push(("rust", include_str!("../../.ahma/rust.json")));
    }
    if cli.fileutils {
        builtin_tools.push(("file-tools", include_str!("../../.ahma/file-tools.json")));
    }
    if cli.github {
        builtin_tools.push(("gh", include_str!("../../.ahma/gh.json")));
    }
    if cli.git {
        builtin_tools.push(("git", include_str!("../../.ahma/git.json")));
    }
    if cli.gradle {
        builtin_tools.push(("gradlew", include_str!("../../.ahma/gradlew.json")));
    }
    if cli.python {
        builtin_tools.push(("python", include_str!("../../.ahma/python.json")));
    }
    if cli.simplify {
        builtin_tools.push(("simplify", include_str!("../../.ahma/simplify.json")));
    }

    for (bundle_name, json_str) in builtin_tools {
        match serde_json::from_str::<ToolConfig>(json_str) {
            Ok(config) => {
                if RESERVED_TOOL_NAMES.contains(&config.name.as_str()) {
                    anyhow::bail!(
                        "Built-in tool '{}' conflicts with a hardcoded system tool.",
                        config.name
                    );
                }
                // Do not override if already loaded from tools_dir / .ahma
                if configs.contains_key(&config.name) {
                    tracing::info!(
                        "Bundled tool '{}' overridden by local .ahma/ definition",
                        config.name
                    );
                } else {
                    configs.insert(config.name.clone(), config);
                }
            }
            Err(e) => {
                tracing::error!(
                    "Failed to parse built-in tool configuration for {}: {}",
                    bundle_name,
                    e
                );
            }
        }
    }

    // Inject synthetic config for `sandboxed_shell` so sequences can reference it.
    // This is the single source of truth for sandboxed_shell's ToolConfig shape.
    // The MCP service handler still intercepts `sandboxed_shell` calls directly,
    // but sequences need a ToolConfig to resolve the command and subcommand.
    configs.insert(
        "sandboxed_shell".to_string(),
        ToolConfig {
            name: "sandboxed_shell".to_string(),
            description: "Execute shell commands within a secure sandbox".to_string(),
            command: if cfg!(target_os = "windows") {
                "pwsh -Command".to_string()
            } else {
                "bash -c".to_string()
            },
            enabled: true,
            subcommand: Some(vec![SubcommandConfig {
                name: "default".to_string(),
                description: "Run a shell command".to_string(),
                enabled: true,
                positional_args: Some(vec![CommandOption {
                    name: "command".to_string(),
                    option_type: "string".to_string(),
                    description: Some("The shell command to execute".to_string()),
                    required: Some(true),
                    format: None,
                    items: None,
                    file_arg: None,
                    file_flag: None,
                    alias: None,
                }]),
                ..Default::default()
            }]),
            ..Default::default()
        },
    );

    Ok(configs)
}

/// Synchronous wrapper around `load_tool_configs` for test use only.
///
/// Creates a one-shot Tokio runtime and delegates to the async version.
/// Production code should use `load_tool_configs` directly.
pub fn load_tool_configs_sync(
    cli: &crate::shell::cli::Cli,
    tools_dir: Option<&Path>,
) -> anyhow::Result<HashMap<String, ToolConfig>> {
    tokio::runtime::Runtime::new()?.block_on(load_tool_configs(cli, tools_dir))
}
