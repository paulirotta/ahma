//! Unix domain socket MCP transport.
//!
//! This module provides a convenience function to create an MCP client transport
//! that communicates with an ahma HTTP bridge over a Unix domain socket, using
//! the rmcp `StreamableHttpClientTransport` backed by `UnixSocketHttpClient`.
//!
//! This is a Unix-only module (`#[cfg(unix)]`).

#[cfg(unix)]
pub use unix_impl::*;

#[cfg(unix)]
mod unix_impl {
    use anyhow::Result;
    use rmcp::transport::{
        StreamableHttpClientTransport, UnixSocketHttpClient,
        streamable_http_client::StreamableHttpClientTransportConfig,
    };

    /// Create an MCP Streamable HTTP transport that connects via a Unix domain socket.
    ///
    /// The socket path may be a filesystem path (e.g. `/tmp/ahma-mcp.sock`) or
    /// a Linux abstract socket with the `@` prefix (e.g. `@ahma-mcp`).
    ///
    /// The `uri` is the HTTP URI used inside the socket connection, e.g.
    /// `http://localhost/mcp` (the host portion is used for HTTP `Host` headers
    /// but is not routed over TCP).
    pub fn unix_socket_transport(
        socket_path: &str,
        uri: &str,
    ) -> Result<StreamableHttpClientTransport<UnixSocketHttpClient>> {
        let client = UnixSocketHttpClient::new(socket_path, uri);
        let config = StreamableHttpClientTransportConfig::with_uri(uri);
        Ok(StreamableHttpClientTransport::with_client(client, config))
    }
}
