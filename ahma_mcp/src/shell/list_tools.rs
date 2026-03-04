//! # List Tools Module
//!
//! Provides functionality to list all MCP tools from an MCP server.
//! This is useful for tests, development, and verifying MCP server tool configurations.
//!
//! ## Usage
//!
//! Connect via command-line arguments:
//! ```bash
//! ahma_mcp --list-tools -- /path/to/ahma_mcp --tools-dir ./tools
//! ```
//!
//! Connect via mcp.json:
//! ```bash
//! ahma_mcp --list-tools --mcp-config /path/to/mcp.json --server Ahma
//! ```
//!
//! Connect to HTTP server:
//! ```bash
//! ahma_mcp --list-tools --http http://localhost:3000
//! ```

use ahma_http_mcp_client::client::HttpMcpTransport;
use anyhow::{Context, Result, anyhow};
use rmcp::{
    ServiceExt,
    model::Tool,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf};
use tokio::process::Command;
use tracing::info;
use url::Url;

/// Output format for tool listing
#[derive(Debug, Clone, clap::ValueEnum, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

/// MCP server configuration from mcp.json
#[derive(Debug, Deserialize, Serialize)]
pub struct McpConfig {
    #[serde(alias = "mcpServers")]
    pub servers: HashMap<String, ServerConfig>,
}

/// Minimal server config entry from mcp.json used by the CLI tool lister.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    /// Transport type ("child_process", "stdio" or "http").
    #[serde(rename = "type", default = "default_server_type")]
    pub server_type: String,
    /// Command to launch a child-process server (if applicable).
    pub command: Option<String>,
    /// Arguments for the child-process server.
    pub args: Option<Vec<String>>,
    /// Working directory for launching the server.
    pub cwd: Option<String>,
    /// Environment variables for the server.
    pub env: Option<HashMap<String, String>>,
    /// HTTP URL for the server (if type is "http").
    pub url: Option<String>,
}

fn default_server_type() -> String {
    "stdio".to_string()
}

/// Tool listing result for JSON output
#[derive(Debug, Serialize)]
pub struct ToolListResult {
    pub server_info: Option<ServerInfoOutput>,
    pub tools: Vec<ToolOutput>,
}

/// Summary information about the connected MCP server.
#[derive(Debug, Serialize)]
pub struct ServerInfoOutput {
    /// Server name (if reported by the MCP handshake).
    pub name: String,
    /// Optional server version string.
    pub version: Option<String>,
}

/// Tool description output for JSON listing.
#[derive(Debug, Serialize)]
pub struct ToolOutput {
    /// Tool name.
    pub name: String,
    /// Optional tool description.
    pub description: Option<String>,
    /// Parameter schema (flattened for display).
    pub parameters: Vec<ParameterOutput>,
}

/// Flattened parameter schema for JSON output.
#[derive(Debug, Serialize)]
pub struct ParameterOutput {
    /// Parameter name.
    pub name: String,
    /// Parameter type (string/number/array/etc).
    pub param_type: String,
    /// Whether the parameter is required.
    pub required: bool,
    /// Optional parameter description.
    pub description: Option<String>,
}

/// Parse mcp.json and get server configuration
pub fn parse_mcp_config_full(
    path: &PathBuf,
    server_name: Option<&str>,
) -> Result<(String, ServerConfig)> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read mcp.json from {}", path.display()))?;

    let config: McpConfig =
        serde_json::from_str(&content).with_context(|| "Failed to parse mcp.json")?;

    let (name, server_config): (String, ServerConfig) = if let Some(name) = server_name {
        let server = config
            .servers
            .get(name)
            .ok_or_else(|| anyhow!("Server '{}' not found in mcp.json", name))?;
        (name.to_string(), server.clone())
    } else {
        config
            .servers
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("No servers defined in mcp.json"))?
    };

    Ok((name, server_config))
}

type LegacyMcpConfig = (String, String, Vec<String>, HashMap<String, String>);

/// Parse mcp.json and get server configuration (legacy helper)
pub fn parse_mcp_config(path: &PathBuf, server_name: Option<&str>) -> Result<LegacyMcpConfig> {
    let (name, config) = parse_mcp_config_full(path, server_name)?;
    let command = config
        .command
        .ok_or_else(|| anyhow!("Server '{}' has no command defined", name))?;
    let command = expand_home(&command);
    let args = config
        .args
        .unwrap_or_default()
        .iter()
        .map(|a| expand_home(a))
        .collect();
    let env = config.env.unwrap_or_default();
    Ok((name, command, args, env))
}

/// Expand `~` prefix to the home directory.
///
/// Handles both Unix (`~/`) and Windows (`~\`) separator styles so that
/// tool configs authored on one platform work correctly on the other.
/// Bare `~` without a following separator is left unchanged.
pub fn expand_home(path: &str) -> String {
    // Accept `~/` (Unix) or `~\` (Windows) — but NOT bare `~` alone.
    let is_tilde_prefix = path.starts_with("~/") || path.starts_with("~\\");
    if is_tilde_prefix && let Some(home) = dirs::home_dir() {
        return path.replacen("~", home.to_str().unwrap_or("~"), 1);
    }
    path.to_string()
}

/// List tools from mcp.json configuration
pub async fn list_tools_from_config(
    config_path: &PathBuf,
    server_name: Option<&str>,
) -> Result<ToolListResult> {
    let (name, server_config) = parse_mcp_config_full(config_path, server_name)?;

    if server_config.server_type == "http" {
        let url_str = server_config
            .url
            .ok_or_else(|| anyhow!("Server '{}' (http) has no url defined", name))?;
        info!("Connecting to server '{}' via HTTP: {}", name, url_str);
        list_tools_http(&url_str).await
    } else {
        let command = server_config
            .command
            .ok_or_else(|| anyhow!("Server '{}' has no command defined", name))?;
        let args = server_config.args.unwrap_or_default();
        let env = server_config.env.unwrap_or_default();

        info!(
            "Connecting to server '{}' via stdio: {} {:?} (env: {:?})",
            name, command, args, env
        );

        let mut all_args = vec![expand_home(&command)];
        all_args.extend(args.iter().map(|a| expand_home(a)));

        list_tools_stdio_with_env(&all_args, env).await
    }
}

/// List tools from stdio MCP server with environment variables
pub async fn list_tools_stdio_with_env(
    command_args: &[String],
    env: HashMap<String, String>,
) -> Result<ToolListResult> {
    if command_args.is_empty() {
        return Err(anyhow!("No command specified for stdio connection"));
    }

    let command = &command_args[0];
    let args = &command_args[1..];

    info!(
        "Starting MCP server: {} {:?} (env: {:?})",
        command, args, env
    );

    // Build command using rmcp's ConfigureCommandExt
    let args_clone: Vec<String> = args.to_vec();
    let env_clone = env.clone();
    let transport = TokioChildProcess::new(Command::new(command).configure(move |cmd| {
        for arg in &args_clone {
            cmd.arg(arg);
        }
        for (key, value) in &env_clone {
            cmd.env(key, value);
        }
    }))
    .with_context(|| format!("Failed to start MCP server process: {} {:?}", command, args))?;

    let client = ()
        .serve(transport)
        .await
        .with_context(|| "Failed to connect to MCP server via stdio")?;

    // Get server info from peer_info
    let server_info_output = client.peer_info().map(|info| ServerInfoOutput {
        name: info.server_info.name.clone(),
        version: Some(info.server_info.version.clone()),
    });

    // List tools
    let tools = client.list_tools(None).await?;

    let tools: Vec<ToolOutput> = tools
        .tools
        .into_iter()
        .map(convert_tool_to_output)
        .collect();

    Ok(ToolListResult {
        server_info: server_info_output,
        tools,
    })
}

/// List tools from HTTP MCP server
pub async fn list_tools_http(url: &str) -> Result<ToolListResult> {
    let url = Url::parse(url).context("Invalid URL")?;

    // Initialize HTTP transport
    let transport =
        HttpMcpTransport::new(url, None, None).context("Failed to create HTTP transport")?;

    // Connect via rmcp client
    let client = ().serve(transport).await.with_context(|| "Failed to connect to HTTP MCP server")?;

    // Get server info
    let server_info_output = client.peer_info().map(|info| ServerInfoOutput {
        name: info.server_info.name.clone(),
        version: Some(info.server_info.version.clone()),
    });

    // List tools
    let tools_result = client.list_tools(None).await?;

    let tools: Vec<ToolOutput> = tools_result
        .tools
        .into_iter()
        .map(convert_tool_to_output)
        .collect();

    Ok(ToolListResult {
        server_info: server_info_output,
        tools,
    })
}

#[allow(dead_code)]
/// Extract parameter definitions from a JSON schema object.
///
/// # Arguments
/// * `schema` - JSON schema value with `properties` and `required` entries.
///
/// # Returns
/// A list of `ParameterOutput` entries suitable for display.
pub fn extract_parameters_from_json(schema: &serde_json::Value) -> Vec<ParameterOutput> {
    let mut params = Vec::new();

    if let Some(properties) = schema["properties"].as_object() {
        let required_fields: Vec<&str> = schema["required"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        for (name, prop) in properties {
            let param_type = prop["type"].as_str().unwrap_or("string").to_string();
            let description = prop["description"].as_str().map(|s| s.to_string());
            let required = required_fields.contains(&name.as_str());

            params.push(ParameterOutput {
                name: name.clone(),
                param_type,
                required,
                description,
            });
        }
    }

    params
}

fn convert_tool_to_output(tool: Tool) -> ToolOutput {
    // input_schema is Arc<Map<String, Value>>, we need to extract properties and required from it
    let schema = &tool.input_schema;

    let properties = schema.get("properties").and_then(|v| v.as_object());

    let required_fields: Vec<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let parameters = if let Some(props) = properties {
        props
            .iter()
            .map(|(name, prop)| {
                let param_type = prop
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("string")
                    .to_string();

                let description = prop
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let required = required_fields.contains(&name.as_str());

                ParameterOutput {
                    name: name.clone(),
                    param_type,
                    required,
                    description,
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    ToolOutput {
        name: tool.name.to_string(),
        description: tool.description.map(|s| s.to_string()),
        parameters,
    }
}

/// Print a human-readable tool list to stdout.
pub fn print_text_output(result: &ToolListResult) {
    println!("MCP Server Tools");
    println!("================");
    println!();

    if let Some(info) = &result.server_info {
        println!("Server: {}", info.name);
        if let Some(version) = &info.version {
            println!("Version: {}", version);
        }
        println!();
    }

    println!("Total tools: {}", result.tools.len());
    println!();

    for tool in &result.tools {
        println!("Tool: {}", tool.name);
        if let Some(desc) = &tool.description {
            println!("  Description: {}", desc);
        }
        if !tool.parameters.is_empty() {
            println!("  Parameters:");
            for param in &tool.parameters {
                let required_str = if param.required {
                    "required"
                } else {
                    "optional"
                };
                print!(
                    "    - {} ({}, {})",
                    param.name, param.param_type, required_str
                );
                if let Some(desc) = &param.description {
                    print!(": {}", desc);
                }
                println!();
            }
        }
        println!();
    }
}

/// Print a JSON tool list to stdout.
pub fn print_json_output(result: &ToolListResult) -> Result<()> {
    let json = serde_json::to_string_pretty(result)?;
    println!("{}", json);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_home() {
        // Non-home path should not change
        assert_eq!(expand_home("/absolute/path"), "/absolute/path");
        assert_eq!(expand_home("relative/path"), "relative/path");

        // Home path should expand (if home dir exists)
        if dirs::home_dir().is_some() {
            let expanded = expand_home("~/test");
            assert!(!expanded.starts_with("~/"));
            assert!(expanded.ends_with("/test"));
        }
    }

    #[test]
    fn test_expand_home_single_tilde() {
        // Single tilde without slash should not expand
        assert_eq!(expand_home("~"), "~");
    }

    #[test]
    fn test_expand_home_windows_backslash() {
        // ~\ (Windows-style) should expand if a home dir is available.
        if dirs::home_dir().is_some() {
            let expanded = expand_home("~\\test");
            assert!(!expanded.starts_with("~\\"));
            assert!(expanded.ends_with("\\test") || expanded.ends_with("/test"));
        }
    }

    #[test]
    fn test_expand_home_tilde_in_middle() {
        // Tilde not at start should not expand
        assert_eq!(expand_home("/path/~/somewhere"), "/path/~/somewhere");
    }

    #[test]
    fn test_expand_home_empty_string() {
        assert_eq!(expand_home(""), "");
    }

    #[test]
    fn test_parse_mcp_config() {
        use std::io::Write;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_path = temp_dir.path().join("mcp.json");

        let config_content = r#"{
            "servers": {
                "TestServer": {
                    "type": "stdio",
                    "command": "/usr/bin/test",
                    "args": ["--flag", "value"]
                }
            }
        }"#;

        let mut file = fs::File::create(&config_path).unwrap();
        file.write_all(config_content.as_bytes()).unwrap();

        let (name, command, args, _env) =
            parse_mcp_config(&config_path, Some("TestServer")).unwrap();

        assert_eq!(name, "TestServer");
        assert_eq!(command, "/usr/bin/test");
        assert_eq!(args, vec!["--flag", "value"]);
    }

    #[test]
    fn test_parse_mcp_config_first_server() {
        use std::io::Write;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_path = temp_dir.path().join("mcp.json");

        let config_content = r#"{
            "servers": {
                "FirstServer": {
                    "type": "stdio",
                    "command": "/usr/bin/first"
                }
            }
        }"#;

        let mut file = fs::File::create(&config_path).unwrap();
        file.write_all(config_content.as_bytes()).unwrap();

        let (name, command, _args, _env) = parse_mcp_config(&config_path, None).unwrap();

        assert_eq!(name, "FirstServer");
        assert_eq!(command, "/usr/bin/first");
    }

    #[test]
    fn test_parse_mcp_config_server_not_found() {
        use std::io::Write;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_path = temp_dir.path().join("mcp.json");

        let config_content = r#"{
            "servers": {
                "ExistingServer": {
                    "type": "stdio",
                    "command": "/usr/bin/test"
                }
            }
        }"#;

        let mut file = fs::File::create(&config_path).unwrap();
        file.write_all(config_content.as_bytes()).unwrap();

        let result = parse_mcp_config(&config_path, Some("NonExistentServer"));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not found in mcp.json")
        );
    }

    #[test]
    fn test_parse_mcp_config_no_servers() {
        use std::io::Write;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_path = temp_dir.path().join("mcp.json");

        let config_content = r#"{"servers": {}}"#;

        let mut file = fs::File::create(&config_path).unwrap();
        file.write_all(config_content.as_bytes()).unwrap();

        let result = parse_mcp_config(&config_path, None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No servers defined")
        );
    }

    #[test]
    fn test_parse_mcp_config_no_command() {
        use std::io::Write;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_path = temp_dir.path().join("mcp.json");

        let config_content = r#"{
            "servers": {
                "NoCommandServer": {
                    "type": "stdio"
                }
            }
        }"#;

        let mut file = fs::File::create(&config_path).unwrap();
        file.write_all(config_content.as_bytes()).unwrap();

        let result = parse_mcp_config(&config_path, Some("NoCommandServer"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no command"));
    }

    #[test]
    fn test_parse_mcp_config_invalid_json() {
        use std::io::Write;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_path = temp_dir.path().join("mcp.json");

        let mut file = fs::File::create(&config_path).unwrap();
        file.write_all(b"not valid json").unwrap();

        let result = parse_mcp_config(&config_path, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_mcp_config_file_not_found() {
        let config_path = PathBuf::from("/nonexistent/path/mcp.json");
        let result = parse_mcp_config(&config_path, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_mcp_config_with_home_expansion() {
        use std::io::Write;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_path = temp_dir.path().join("mcp.json");

        let config_content = r#"{
            "servers": {
                "HomeServer": {
                    "type": "stdio",
                    "command": "~/bin/mcp_server",
                    "args": ["~/config/settings.json"]
                }
            }
        }"#;

        let mut file = fs::File::create(&config_path).unwrap();
        file.write_all(config_content.as_bytes()).unwrap();

        let (name, command, args, _env) =
            parse_mcp_config(&config_path, Some("HomeServer")).unwrap();

        assert_eq!(name, "HomeServer");
        // Command and args should have ~ expanded
        if dirs::home_dir().is_some() {
            assert!(!command.starts_with("~/"));
            assert!(!args[0].starts_with("~/"));
        }
    }

    #[test]
    fn test_extract_parameters_from_json() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name"
                },
                "count": {
                    "type": "integer"
                }
            },
            "required": ["name"]
        });

        let params = extract_parameters_from_json(&schema);

        assert_eq!(params.len(), 2);

        let name_param = params.iter().find(|p| p.name == "name").unwrap();
        assert_eq!(name_param.param_type, "string");
        assert!(name_param.required);
        assert_eq!(name_param.description, Some("The name".to_string()));

        let count_param = params.iter().find(|p| p.name == "count").unwrap();
        assert_eq!(count_param.param_type, "integer");
        assert!(!count_param.required);
    }

    #[test]
    fn test_extract_parameters_empty_schema() {
        let schema = serde_json::json!({});
        let params = extract_parameters_from_json(&schema);
        assert!(params.is_empty());
    }

    #[test]
    fn test_extract_parameters_no_properties() {
        let schema = serde_json::json!({
            "type": "object"
        });
        let params = extract_parameters_from_json(&schema);
        assert!(params.is_empty());
    }

    #[test]
    fn test_extract_parameters_no_required_array() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "field1": {"type": "string"}
            }
        });
        let params = extract_parameters_from_json(&schema);
        assert_eq!(params.len(), 1);
        assert!(!params[0].required);
    }

    #[test]
    fn test_extract_parameters_missing_type_defaults_to_string() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "untyped_field": {}
            }
        });
        let params = extract_parameters_from_json(&schema);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].param_type, "string");
    }

    #[test]
    fn test_extract_parameters_no_description() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "no_desc": {"type": "boolean"}
            }
        });
        let params = extract_parameters_from_json(&schema);
        assert_eq!(params.len(), 1);
        assert!(params[0].description.is_none());
    }

    #[test]
    fn test_tool_output_serialization() {
        let tool = ToolOutput {
            name: "test_tool".to_string(),
            description: Some("A test tool".to_string()),
            parameters: vec![ParameterOutput {
                name: "param1".to_string(),
                param_type: "string".to_string(),
                required: true,
                description: Some("First param".to_string()),
            }],
        };

        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("test_tool"));
        assert!(json.contains("param1"));
    }

    #[test]
    fn test_tool_output_serialization_no_description() {
        let tool = ToolOutput {
            name: "minimal_tool".to_string(),
            description: None,
            parameters: vec![],
        };

        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("minimal_tool"));
        assert!(json.contains("null") || json.contains("\"description\":null"));
    }

    #[test]
    fn test_parameter_output_optional_fields() {
        let param = ParameterOutput {
            name: "optional_param".to_string(),
            param_type: "number".to_string(),
            required: false,
            description: None,
        };

        let json = serde_json::to_string(&param).unwrap();
        assert!(json.contains("optional_param"));
        assert!(json.contains("number"));
        assert!(json.contains("false"));
    }

    #[test]
    fn test_print_text_output_with_server_info() {
        let result = ToolListResult {
            server_info: Some(ServerInfoOutput {
                name: "TestMcpServer".to_string(),
                version: Some("1.0.0".to_string()),
            }),
            tools: vec![ToolOutput {
                name: "my_tool".to_string(),
                description: Some("Does something useful".to_string()),
                parameters: vec![
                    ParameterOutput {
                        name: "input".to_string(),
                        param_type: "string".to_string(),
                        required: true,
                        description: Some("The input value".to_string()),
                    },
                    ParameterOutput {
                        name: "count".to_string(),
                        param_type: "integer".to_string(),
                        required: false,
                        description: None,
                    },
                ],
            }],
        };

        // This test just ensures print_text_output doesn't panic
        print_text_output(&result);
    }

    #[test]
    fn test_print_text_output_no_server_info() {
        let result = ToolListResult {
            server_info: None,
            tools: vec![ToolOutput {
                name: "simple_tool".to_string(),
                description: None,
                parameters: vec![],
            }],
        };

        // This test just ensures print_text_output doesn't panic
        print_text_output(&result);
    }

    #[test]
    fn test_print_text_output_no_tools() {
        let result = ToolListResult {
            server_info: Some(ServerInfoOutput {
                name: "EmptyServer".to_string(),
                version: None,
            }),
            tools: vec![],
        };

        // This test just ensures print_text_output doesn't panic
        print_text_output(&result);
    }

    #[test]
    fn test_print_text_output_server_without_version() {
        let result = ToolListResult {
            server_info: Some(ServerInfoOutput {
                name: "NoVersionServer".to_string(),
                version: None,
            }),
            tools: vec![],
        };

        print_text_output(&result);
    }

    #[test]
    fn test_print_json_output_basic() {
        let result = ToolListResult {
            server_info: Some(ServerInfoOutput {
                name: "JsonServer".to_string(),
                version: Some("2.0.0".to_string()),
            }),
            tools: vec![
                ToolOutput {
                    name: "tool_a".to_string(),
                    description: Some("First tool".to_string()),
                    parameters: vec![],
                },
                ToolOutput {
                    name: "tool_b".to_string(),
                    description: None,
                    parameters: vec![ParameterOutput {
                        name: "arg".to_string(),
                        param_type: "string".to_string(),
                        required: true,
                        description: Some("An argument".to_string()),
                    }],
                },
            ],
        };

        let json_result = print_json_output(&result);
        assert!(json_result.is_ok());
    }

    #[test]
    fn test_print_json_output_empty() {
        let result = ToolListResult {
            server_info: None,
            tools: vec![],
        };

        let json_result = print_json_output(&result);
        assert!(json_result.is_ok());
    }

    #[test]
    fn test_tool_list_result_serialization() {
        let result = ToolListResult {
            server_info: Some(ServerInfoOutput {
                name: "TestServer".to_string(),
                version: Some("1.0".to_string()),
            }),
            tools: vec![],
        };

        let json = serde_json::to_string_pretty(&result).unwrap();
        assert!(json.contains("TestServer"));
        assert!(json.contains("1.0"));
    }

    #[test]
    fn test_server_info_output_serialization() {
        let info = ServerInfoOutput {
            name: "MyServer".to_string(),
            version: Some("3.2.1".to_string()),
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("MyServer"));
        assert!(json.contains("3.2.1"));
    }

    #[test]
    fn test_mcp_config_deserialization() {
        let config_str = r#"{
            "servers": {
                "server1": {
                    "type": "stdio",
                    "command": "/bin/cmd",
                    "args": ["--arg1"],
                    "cwd": "/workdir"
                }
            }
        }"#;

        let config: McpConfig = serde_json::from_str(config_str).unwrap();
        assert!(config.servers.contains_key("server1"));
        let server = &config.servers["server1"];
        assert_eq!(server.server_type, "stdio");
        assert_eq!(server.command, Some("/bin/cmd".to_string()));
        assert_eq!(server.args, Some(vec!["--arg1".to_string()]));
        assert_eq!(server.cwd, Some("/workdir".to_string()));
    }

    #[test]
    fn test_server_config_minimal() {
        let config_str = r#"{"type": "http"}"#;
        let server: ServerConfig = serde_json::from_str(config_str).unwrap();
        assert_eq!(server.server_type, "http");
        assert!(server.command.is_none());
        assert!(server.args.is_none());
        assert!(server.cwd.is_none());
    }

    #[test]
    fn test_server_config_serialization_roundtrip() {
        let server = ServerConfig {
            server_type: "stdio".to_string(),
            command: Some("/usr/local/bin/server".to_string()),
            args: Some(vec!["--port".to_string(), "8080".to_string()]),
            cwd: Some("/home/user".to_string()),
            env: Some(HashMap::new()),
            url: None,
        };

        let json = serde_json::to_string(&server).unwrap();
        let deserialized: ServerConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.server_type, server.server_type);
        assert_eq!(deserialized.command, server.command);
        assert_eq!(deserialized.args, server.args);
        assert_eq!(deserialized.cwd, server.cwd);
    }
}
