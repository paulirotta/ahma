//! # Ahma HTTP MCP Client
//!
//! This crate provides an HTTP transport implementation for the Model Context Protocol (MCP).
//! It is designed to communicate with MCP servers over HTTP/HTTPS, including support for
//! OAuth2 authentication workflows (currently optimized for Atlassian services).
//!
//! ## Key Features
//!
//! - **HTTP Transport**: Implements `rmcp::transport::Transport` for sending and receiving JSON-RPC messages.
//! - **OAuth2 Support**: Built-in flow to authenticate with providers (e.g., Atlassian) using code grant with PKCE.
//! - **Token Management**: Automatically handles token storage and retrieval from a local file.
//!
//! ## Usage
//!
//! The main entry point is [`client::HttpMcpTransport`]. You construct it with the target URL
//! and optional OAuth2 credentials.
//!
//! ```no_run
//! use ahma_http_mcp_client::client::HttpMcpTransport;
//! use url::Url;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let url = Url::parse("https://api.atlassian.com/mcp")?;
//! let client = HttpMcpTransport::new(
//!     url,
//!     Some("client_id".to_string()),
//!     Some("client_secret".to_string())
//! )?;
//!
//! // Ensure we have a valid token before making requests
//! client.ensure_authenticated().await?;
//! # Ok(())
//! # }
//! ```

/// HTTP transport implementation for MCP clients.
pub mod client;
/// Error types for HTTP MCP client operations.
pub mod error;
/// Unix domain socket MCP transport (Unix only).
#[cfg(unix)]
pub mod unix_client;
