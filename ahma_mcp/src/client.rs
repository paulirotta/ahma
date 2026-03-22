//! # MCP Client for Integration Testing
//!
//! This module provides a simple client wrapper for testing the MCP service.
//! It is designed to spawn the `ahma_mcp` binary in a subprocess and interact
//! with it via the MCP protocol.
//!
//! **Note**: This client is primarily for integration tests and isn't intended
//! for general purpose MCP client usage.

use anyhow::Result;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, Content},
    service::{RoleClient, RunningService},
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::json;
use std::path::PathBuf;
use tokio::process::Command;

/// A test client that wraps a running `ahma_mcp` server process.
#[derive(Debug)]
pub struct Client {
    service: Option<RunningService<RoleClient, ()>>,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// Creates a new, uninitialized client. call `start_process` to launch the server.
    pub fn new() -> Self {
        Self { service: None }
    }

    /// Spawns the `ahma_mcp` server process and connects to it.
    ///
    /// # Arguments
    ///
    /// * `tools_dir` - Optional path to a directory containing tool definitions/configurations.
    ///   If provided, this is passed as the `--tools-dir` argument to the server.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ahma_mcp::client::Client;
    ///
    /// # async fn run() -> anyhow::Result<()> {
    /// let mut client = Client::new();
    /// // Start server with tools from "test-data/tools"
    /// client.start_process(Some("test-data/tools")).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start_process(&mut self, tools_dir: Option<&str>) -> Result<()> {
        self.start_process_with_args(tools_dir, &[]).await
    }

    /// Spawns the server with additional command-line arguments.
    pub async fn start_process_with_args(
        &mut self,
        tools_dir: Option<&str>,
        extra_args: &[&str],
    ) -> Result<()> {
        let command = if let Some(binary) = resolve_prebuilt_ahma_mcp_binary().await {
            Command::new(binary)
        } else {
            eprintln!(
                "Warning: Using slow 'cargo run' path. Run 'cargo build -p ahma_mcp --bin ahma-mcp' first for faster tests."
            );
            let mut cmd = Command::new("cargo");
            cmd.arg("run")
                .arg("--package")
                .arg("ahma_mcp")
                .arg("--bin")
                .arg("ahma-mcp")
                .arg("--");
            cmd
        };

        let client = ()
            .serve(TokioChildProcess::new(command.configure(|cmd| {
                // New CLI: ahma-mcp serve stdio [--tools-dir PATH]
                // Behaviour flags (--disable-sandbox, etc.) are now env vars.
                cmd.args(["serve", "stdio"]);
                cmd.env("AHMA_DISABLE_SANDBOX", "1");
                cmd.env("AHMA_SKIP_PROBES", "1");
                if let Some(dir) = tools_dir {
                    cmd.env("AHMA_TOOLS_DIR", dir);
                }
                for arg in extra_args {
                    cmd.arg(arg);
                }
            }))?)
            .await?;
        self.service = Some(client);
        Ok(())
    }

    pub fn service(&self) -> Option<&RunningService<RoleClient, ()>> {
        self.service.as_ref()
    }

    pub fn service_mut(&mut self) -> Option<&mut RunningService<RoleClient, ()>> {
        self.service.as_mut()
    }

    fn get_service(&self) -> Result<&RunningService<RoleClient, ()>> {
        self.service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Client not initialized"))
    }

    pub async fn shell_async_sleep(&mut self, duration: &str) -> Result<ToolCallResult> {
        let service = self.get_service()?;

        let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
            json!({
                "subcommand": "default",
                "args": [format!("sleep {}", duration)]
            })
            .as_object()
            .unwrap()
            .clone(),
        );

        let result = service.call_tool(params).await?;
        if let Some(content) = result.content.first() {
            if let Some(text_content) = content.as_text() {
                let text = &text_content.text;
                let job_id = extract_id(text)?;
                Ok(ToolCallResult {
                    status: "started".to_string(),
                    job_id,
                    message: text.clone(),
                })
            } else {
                Err(anyhow::anyhow!("No text content in response"))
            }
        } else {
            Err(anyhow::anyhow!("No text content in response"))
        }
    }

    pub async fn await_op(&mut self, op_id: &str) -> Result<String> {
        let service = self.get_service()?;

        let params = CallToolRequestParams::new("await").with_arguments(
            json!({
                "id": op_id
            })
            .as_object()
            .unwrap()
            .clone(),
        );

        let result = service.call_tool(params).await?;
        let full_text = join_text_contents(&result.content)?;
        Ok(full_text)
    }

    pub async fn status(&mut self, op_id: &str) -> Result<String> {
        let service = self.get_service()?;

        let params = CallToolRequestParams::new("status").with_arguments(
            json!({
                "id": op_id
            })
            .as_object()
            .unwrap()
            .clone(),
        );

        let result = service.call_tool(params).await?;
        first_text_content(&result.content)
    }
}

fn workspace_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to locate workspace root")
        .to_path_buf()
}

async fn resolve_prebuilt_ahma_mcp_binary() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(path) = std::env::var("AHMA_TEST_BINARY") {
        candidates.push(PathBuf::from(path));
    }

    let workspace = workspace_dir();
    let bin_name = format!("ahma-mcp{}", std::env::consts::EXE_SUFFIX);

    if let Ok(target_dir_raw) = std::env::var("CARGO_TARGET_DIR") {
        let target_dir = PathBuf::from(target_dir_raw);
        let target_dir = if target_dir.is_absolute() {
            target_dir
        } else {
            workspace.join(target_dir)
        };
        candidates.push(target_dir.join("debug").join(&bin_name));
        candidates.push(target_dir.join("release").join(&bin_name));
    }

    candidates.push(workspace.join("target").join("debug").join(&bin_name));
    candidates.push(workspace.join("target").join("release").join(&bin_name));

    for candidate in candidates {
        if tokio::fs::metadata(&candidate).await.is_ok() {
            return Some(candidate);
        }
    }

    None
}

fn extract_id(text: &str) -> Result<String> {
    if let Some(id_start) = text.find("ID: ") {
        let id_text = &text[id_start + 4..];
        if let Some(job_id) = id_text.split_whitespace().next()
            && !job_id.is_empty()
        {
            return Ok(job_id.to_string());
        }
    }

    Err(anyhow::anyhow!(
        "Could not parse operation ID from response: {}",
        text
    ))
}

fn join_text_contents(contents: &[Content]) -> Result<String> {
    let mut combined = String::new();
    for text_content in contents.iter().filter_map(|c| c.as_text()) {
        if !combined.is_empty() {
            combined.push_str("\n\n");
        }
        combined.push_str(&text_content.text);
    }

    if combined.is_empty() {
        Err(anyhow::anyhow!("No text content in response"))
    } else {
        Ok(combined)
    }
}

fn first_text_content(contents: &[Content]) -> Result<String> {
    contents
        .iter()
        .find_map(|c| c.as_text().map(|t| t.text.clone()))
        .ok_or_else(|| anyhow::anyhow!("No text content in response"))
}

/// Parsed response payload for async tool calls.
///
/// The MCP server returns a JSON payload when a tool call is queued. This
/// struct captures the standardized fields used by the CLI helper.
#[derive(serde::Deserialize, Debug)]
pub struct ToolCallResult {
    /// Status string (e.g. "queued" or "running").
    pub status: String,
    /// Unique operation/job identifier.
    pub job_id: String,
    /// Human-readable summary message.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_id_parses_identifier() {
        let text = "AHMA ID: job_123 Follow-up";
        let job_id = extract_id(text).unwrap();
        assert_eq!(job_id, "job_123");
    }

    #[test]
    fn extract_id_errors_without_marker() {
        let err = extract_id("No identifier present").unwrap_err();
        assert!(
            err.to_string()
                .contains("Could not parse operation ID from response")
        );
    }

    #[test]
    fn join_text_contents_merges_segments() {
        let contents = vec![
            Content::text("first chunk".to_string()),
            Content::text("second chunk".to_string()),
        ];
        let combined = join_text_contents(&contents).unwrap();
        assert_eq!(combined, "first chunk\n\nsecond chunk");
    }

    #[test]
    fn join_text_contents_errors_when_empty() {
        let contents: Vec<Content> = Vec::new();
        assert!(join_text_contents(&contents).is_err());
    }

    #[test]
    fn first_text_content_returns_first_available_segment() {
        let contents = vec![
            Content::text("alpha".to_string()),
            Content::text("beta".to_string()),
        ];
        let first = first_text_content(&contents).unwrap();
        assert_eq!(first, "alpha");
    }

    #[test]
    fn first_text_content_errors_when_absent() {
        let contents: Vec<Content> = Vec::new();
        assert!(first_text_content(&contents).is_err());
    }
}
