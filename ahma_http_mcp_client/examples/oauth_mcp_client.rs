use ahma_http_mcp_client::client::HttpMcpTransport;
use clap::Parser;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use url::Url;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// MCP server SSE endpoint (defaults to Atlassian)
    #[arg(long, default_value = "https://mcp.atlassian.com/v1/sse")]
    server_url: Url,

    /// Atlassian Client ID
    #[arg(long)]
    atlassian_client_id: String,

    /// Atlassian Client Secret
    #[arg(long)]
    atlassian_client_secret: String,

    /// Query string to send to the Confluence search tool
    #[arg(long, default_value = "MCP")]
    query: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    println!("Connecting to Atlassian MCP server at {}", args.server_url);

    let transport = HttpMcpTransport::new(
        args.server_url.clone(),
        Some(args.atlassian_client_id),
        Some(args.atlassian_client_secret),
    )?;

    println!("Authenticating...");
    transport.ensure_authenticated().await?;
    println!("Authentication successful!");

    // Create the client service
    let service = ().serve(transport).await?;

    println!("Listing tools...");
    let tools_result = service.list_tools(None).await?;

    println!("Found {} tools:", tools_result.tools.len());
    for tool in &tools_result.tools {
        println!(
            "- {}: {}",
            tool.name,
            tool.description.as_deref().unwrap_or("No description")
        );
    }

    // Search for "MCP"
    let search_tool = tools_result
        .tools
        .iter()
        .find(|t| t.name.contains("search"));

    if let Some(tool) = search_tool {
        println!("\nFound search tool: {}", tool.name);
        println!("Searching for '{}'...", args.query);

        let params = CallToolRequestParams::new(tool.name.clone()).with_arguments(
            serde_json::json!({ "query": args.query })
                .as_object()
                .unwrap()
                .clone(),
        );

        match service.call_tool(params).await {
            Ok(result) => {
                println!("Search result:");
                for content in result.content {
                    if let Some(text_content) = content.as_text() {
                        println!("{}", text_content.text);
                    } else {
                        println!("[Non-text content]");
                    }
                }
            }
            Err(e) => {
                println!("Error calling search tool: {}", e);
            }
        }
    } else {
        println!("\nNo tool with 'search' in its name found.");
    }

    Ok(())
}
