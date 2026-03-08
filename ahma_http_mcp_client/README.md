# Ahma HTTP MCP Client

HTTP client library for communicating with MCP servers over HTTP.

## Overview

This crate provides a transport for interacting with MCP servers via HTTP/S, supporting both the Ahma HTTP bridge and native HTTP MCP servers that expose an SSE stream.

## Features

- HTTP/HTTPS support
- JSON-RPC message handling
- Async/await API
- Error handling and retries

## Usage

```rust
use ahma_http_mcp_client::client::HttpMcpTransport;
use rmcp::ServiceExt;
use url::Url;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
        // Note: External servers may use /sse (legacy), local Ahma servers use /mcp
        let mcp_url = Url::parse("https://mcp.atlassian.com/v1/sse")?;
        let transport = HttpMcpTransport::new(
                mcp_url,
                Some("ATLAS_CLIENT_ID".to_string()),
                Some("ATLAS_CLIENT_SECRET".to_string()),
        )?;

        transport.ensure_authenticated().await?;

        let service = ().serve(transport).await?;
        let tool_list = service.list_tools(None).await?;
        println!("Loaded {} tools", tool_list.tools.len());

        Ok(())
}
```

## Examples

### OAuth MCP Client (Atlassian)

An example client that connects to the Atlassian MCP server using OAuth2 is provided in `examples/oauth_mcp_client.rs`.

1. Add the server to your MCP config (for example `~/Library/Application Support/Code/User/mcp.json`):

```jsonc
{
    "servers": {
        "atlassian-mcp-server": {
            "type": "http",
            "url": "https://mcp.atlassian.com/v1/sse"
        }
    }
}
```

1. Run the example:

```bash
cargo run -p ahma-http-mcp-client --example oauth_mcp_client -- \
    --atlassian-client-id YOUR_CLIENT_ID \
    --atlassian-client-secret YOUR_CLIENT_SECRET \
    --query "MCP"
```

The example opens your browser to complete the OAuth consent flow, listens for the redirect on `http://localhost:8080`, and then issues a Confluence search for the provided query.

## License

MIT OR Apache-2.0

## Examples

### MCP Client

cargo run -p ahma-http-mcp-client --example oauth_mcp_client -- --atlassian-client-id … --atlassian-client-secret …
