//! In-process MCP client/server pairs for fast unit-level testing.
//!
//! Bypasses subprocess spawning by connecting `AhmaMcpService` directly to a
//! `RunningService<RoleClient, ()>` via a `tokio::io::duplex` channel.  The full
//! MCP handshake (initialize / initialized) still runs, so the behaviour is
//! identical to the stdio subprocess path – but without the forking overhead.

use crate::adapter::Adapter;
use crate::config::{ToolConfig, load_tool_configs};
use crate::mcp_service::{AhmaMcpService, GuidanceConfig};
use crate::operation_monitor::{MonitorConfig, OperationMonitor};
use crate::sandbox::{Sandbox, SandboxMode};
use crate::shell::cli::AppConfig;
use crate::shell_pool::{ShellPoolConfig, ShellPoolManager};
use anyhow::Result;
use rmcp::{
    ServiceExt,
    service::{RoleClient, RoleServer, RunningService},
    transport::async_rw::AsyncRwTransport,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Holds both sides of an in-process MCP connection.
///
/// The server handle must remain alive for the duration of the test so that
/// the background task loop can respond to client requests.  Drop this value
/// (or let it go out of scope) to cleanly shut down the connection.
pub struct InProcessMcp {
    /// The MCP client – use this to call `list_all_tools`, `call_tool`, etc.
    pub client: RunningService<RoleClient, ()>,
    // Keeps the server background tasks alive.
    _server: RunningService<RoleServer, AhmaMcpService>,
}

/// Create an in-process MCP pair using an empty tool config map.
///
/// Suitable for error-handling tests where the specific tool names don't matter.
pub async fn create_in_process_mcp_empty() -> Result<InProcessMcp> {
    create_in_process_mcp(HashMap::new()).await
}

/// Create an in-process MCP pair whose tool list is loaded from `tools_dir`.
pub async fn create_in_process_mcp_from_dir(tools_dir: &Path) -> Result<InProcessMcp> {
    let configs = load_tool_configs(&AppConfig::default(), Some(tools_dir))
        .await
        .unwrap_or_default();
    create_in_process_mcp(configs).await
}

/// Core constructor: wire `AhmaMcpService` to a client over a duplex channel.
///
/// Both the MCP initialize/initialized handshake and any subsequent requests
/// go through the in-memory channel – no subprocess is spawned.
pub async fn create_in_process_mcp(configs: HashMap<String, ToolConfig>) -> Result<InProcessMcp> {
    wire_in_process_mcp(configs, Sandbox::new_test()).await
}

/// Create an in-process MCP pair with a strict sandbox scoped to `scopes`.
///
/// Unlike [`create_in_process_mcp_from_dir`], this constructor creates a
/// `SandboxMode::Strict` sandbox, so path-security tests that assert sandbox
/// enforcement still work correctly in-process.
pub async fn create_in_process_mcp_with_scope(
    tools_dir: &Path,
    scopes: Vec<PathBuf>,
) -> Result<InProcessMcp> {
    let configs = load_tool_configs(&AppConfig::default(), Some(tools_dir))
        .await
        .unwrap_or_default();
    let sandbox = Sandbox::new(scopes, SandboxMode::Strict, false, false, false)?;
    wire_in_process_mcp(configs, sandbox).await
}

/// Internal: wire a pre-built `Sandbox` and tool configs into an in-process pair.
async fn wire_in_process_mcp(
    configs: HashMap<String, ToolConfig>,
    sandbox: Sandbox,
) -> Result<InProcessMcp> {
    let monitor_config = MonitorConfig::with_timeout(std::time::Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let adapter = Arc::new(Adapter::new(
        Arc::clone(&operation_monitor),
        shell_pool,
        Arc::new(sandbox),
    )?);

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        Arc::new(configs),
        Arc::new(None::<GuidanceConfig>),
        false, // force_synchronous
        false, // defer_sandbox
        false, // progressive_disclosure
    )
    .await?;

    // Wire client and server through an in-memory duplex channel.
    let (client_stream, server_stream) = tokio::io::duplex(65536);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let (server_read, server_write) = tokio::io::split(server_stream);

    let client_transport = AsyncRwTransport::new_client(client_read, client_write);
    let server_transport = AsyncRwTransport::new_server(server_read, server_write);

    // Run both handshakes concurrently; both futures complete once the
    // initialize / initialized exchange is done and both sides are ready.
    let (client_result, server_result) =
        tokio::join!(().serve(client_transport), service.serve(server_transport),);

    Ok(InProcessMcp {
        client: client_result?,
        _server: server_result?,
    })
}
