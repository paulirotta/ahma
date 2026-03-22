//! # List Tools Mode
//!
//! Runs the ahma_mcp server in list-tools mode, which connects to an MCP server
//! and lists all available tools.

use crate::shell::{cli::AppConfig, list_tools};
use anyhow::{Result, anyhow};
use std::collections::HashMap;

/// Run in list-tools mode: connect to an MCP server and list all available tools.
///
/// # Arguments
/// * `config` - Immutable application configuration.
///
/// # Errors
/// Returns an error if the connection or listing fails.
pub async fn run_list_tools_mode(config: &AppConfig) -> Result<()> {
    // Determine connection mode
    let result = if let Some(ref http_url) = config.list_http {
        list_tools::list_tools_http(http_url).await?
    } else if config.run_tool.is_some() || !config.run_tool_args.is_empty() {
        // Build command args from run_tool (first positional) and run_tool_args (after --)
        let mut command_args: Vec<String> = Vec::new();
        if let Some(ref cmd) = config.run_tool {
            command_args.push(cmd.clone());
        }
        command_args.extend(config.run_tool_args.clone());

        if command_args.is_empty() {
            return Err(anyhow!(
                "No command specified for tool list. Provide server command after --"
            ));
        }

        list_tools::list_tools_stdio_with_env(&command_args, HashMap::new()).await?
    } else if config.mcp_config.exists() {
        list_tools::list_tools_from_config(&config.mcp_config, config.list_server.as_deref())
            .await?
    } else {
        return Err(anyhow!(
            "No connection method specified for tool list. Use --http, --mcp-config with --server, or provide command after --"
        ));
    };

    // Output result
    match config.list_format {
        list_tools::OutputFormat::Text => list_tools::print_text_output(&result),
        list_tools::OutputFormat::Json => list_tools::print_json_output(&result)?,
    }

    Ok(())
}
