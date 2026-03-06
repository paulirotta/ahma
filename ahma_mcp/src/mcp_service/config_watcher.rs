use notify::{Event, RecursiveMode, Watcher};
use rmcp::service::{Peer, RoleServer};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing;

use super::AhmaMcpService;
use crate::config::{ToolConfig, load_tool_configs};

impl AhmaMcpService {
    /// Updates the tool configurations and notifies clients.
    pub async fn update_tools(&self, new_configs: HashMap<String, ToolConfig>) {
        {
            let mut configs_lock = self.configs.write().unwrap();
            *configs_lock = new_configs;
        }

        // Notify clients that the tool list has changed.
        // Clone peer outside the lock before async call to avoid holding guard across .await
        let peer_opt = {
            let peer_lock = self.peer.read().unwrap();
            peer_lock.clone()
        };

        if let Some(peer) = peer_opt {
            if let Err(e) = peer.notify_tool_list_changed().await {
                tracing::error!("Failed to send tools/list_changed notification: {}", e);
            } else {
                tracing::info!("Sent tools/list_changed notification to client");
            }
        } else {
            tracing::debug!("No peer connected, skipping tools/list_changed notification");
        }
    }

    /// Starts a background task to watch for changes in the tools directory.
    pub fn start_config_watcher(&self, tools_dir: PathBuf, cli: crate::shell::cli::Cli) {
        let service = self.clone();
        // Use a weak pointer to the operation monitor to detect when the service is dropped
        let weak_monitor = Arc::downgrade(&self.operation_monitor);

        tokio::spawn(async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel(1);

            let mut watcher =
                match notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                    if let Ok(event) = res {
                        // Only react to relevant events on JSON files or directory changes
                        let relevant = event
                            .paths
                            .iter()
                            .any(|p| p.extension().is_some_and(|ext| ext == "json") || p.is_dir());

                        if relevant
                            && (event.kind.is_modify()
                                || event.kind.is_create()
                                || event.kind.is_remove())
                        {
                            let _ = tx.blocking_send(());
                        }
                    }
                }) {
                    Ok(w) => w,
                    Err(e) => {
                        tracing::error!("Failed to create config watcher: {}", e);
                        return;
                    }
                };

            if let Err(e) = watcher.watch(&tools_dir, RecursiveMode::Recursive) {
                tracing::error!("Failed to watch tools directory: {}", e);
                return;
            }

            tracing::info!("Started watching tools directory: {:?}", tools_dir);

            // Debounce logic
            loop {
                tokio::select! {
                    recv = rx.recv() => {
                        if recv.is_none() {
                            break;
                        }

                        // Drain any other events that happened in quick succession
                        while rx.try_recv().is_ok() {}

                        // Wait a bit for file writes to complete
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

                        tracing::info!("Detected change in tools directory, reloading configs...");
                        match load_tool_configs(&cli, Some(&tools_dir)).await {
                            Ok(new_configs) => {
                                service.update_tools(new_configs).await;
                                tracing::info!("Successfully reloaded tool configurations");
                            }
                            Err(e) => {
                                tracing::error!("Failed to reload tool configurations: {}", e);
                            }
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                        // Check if the service (via its monitor) is still alive
                        if weak_monitor.upgrade().is_none() {
                            tracing::debug!("AhmaMcpService dropped, stopping config watcher task");
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Query the client for workspace roots and initialize the sandbox scope.
    ///
    /// This implements the MCP roots protocol where the server requests the
    /// client's workspace roots to establish sandbox boundaries.
    pub async fn configure_sandbox_from_roots(&self, peer: &Peer<RoleServer>) {
        eprintln!("DEBUG: ahma_mcp calling peer.list_roots()");
        tracing::info!("Requesting roots/list from client...");

        // Use the list_roots() method provided by Peer<RoleServer>
        match peer.list_roots().await {
            Ok(result) => {
                let roots = result.roots;
                eprintln!("DEBUG: ahma_mcp received {} roots", roots.len());
                tracing::info!("Received {} roots from client: {:?}", roots.len(), roots);

                // Convert McpRoot URIs to PathBufs
                let mut new_scopes = Vec::new();
                for root in roots.iter() {
                    #[allow(clippy::collapsible_if)]
                    if let Ok(url) = url::Url::parse(&root.uri) {
                        if url.scheme() == "file" {
                            if let Ok(path) = url.to_file_path() {
                                tracing::info!("Parsed valid file URI: {} -> {:?}", root.uri, path);
                                new_scopes.push(path);
                            } else {
                                tracing::warn!("Failed to convert file URI to path: {}", root.uri);
                            }
                        } else {
                            tracing::warn!("Ignoring non-file URI: {}", root.uri);
                        }
                    } else {
                        tracing::warn!("Failed to parse URI: {}", root.uri);
                    }
                }

                tracing::info!(
                    "Parsed {} valid scopes out of {} client roots",
                    new_scopes.len(),
                    roots.len()
                );

                if !new_scopes.is_empty() {
                    if let Err(e) = self.adapter.sandbox().update_scopes(new_scopes.clone()) {
                        tracing::error!("Failed to update sandbox from roots: {}", e);
                        if let Ok(notification) = serde_json::to_string(&serde_json::json!({
                            "jsonrpc": "2.0",
                            "method": "notifications/sandbox/failed",
                            "params": { "error": e.to_string() }
                        })) {
                            println!("\n{}", notification);
                        }
                        return;
                    }

                    tracing::info!("Sandbox scopes updated successfully");

                    // On Linux, apply Landlock kernel-level restrictions now that we have scopes.
                    // This is critical for HTTP bridge deferred sandbox mode where Landlock
                    // couldn't be applied at startup (scopes weren't known yet).
                    // SECURITY: Fail the session if Landlock enforcement fails - we cannot
                    // guarantee security without kernel-level restrictions.
                    #[cfg(target_os = "linux")]
                    {
                        if !self.adapter.sandbox().is_test_mode() {
                            if let Err(e) = crate::sandbox::enforce_landlock_sandbox(
                                &new_scopes,
                                self.adapter.sandbox().read_scopes(),
                                self.adapter.sandbox().is_no_temp_files(),
                            ) {
                                tracing::error!(
                                    "FATAL: Failed to enforce Landlock sandbox: {}. \
                                     Exiting to prevent running without kernel-level security.",
                                    e
                                );
                                if let Ok(notification) =
                                    serde_json::to_string(&serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "method": "notifications/sandbox/failed",
                                        "params": { "error": e.to_string() }
                                    }))
                                {
                                    println!("\n{}", notification);
                                }
                                std::process::exit(1);
                            }
                            tracing::info!("Landlock sandbox enforced successfully");
                        }
                    }
                } else if !self.adapter.sandbox().scopes().is_empty() {
                    // Client provided no file:// roots but we have pre-configured scopes
                    // from --working-directories. These are valid, so proceed.
                    tracing::info!(
                        "No new scopes from roots/list; using pre-configured scopes: {:?}",
                        self.adapter.sandbox().scopes()
                    );
                } else {
                    tracing::warn!("No scopes available from roots or pre-configuration");
                    return;
                }

                // Notify bridge that sandbox has been configured so it can safely
                // forward tools/call requests. We emit a JSON-RPC notification
                // on stdout which the HTTP bridge listens for on the subprocess
                // stdout stream.
                // NOTE: we intentionally write the raw JSON to stdout instead of
                // using rmcp Peer helpers here to avoid relying on generated
                // methods for a new notification name.
                if let Ok(notification) = serde_json::to_string(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/sandbox/configured"
                })) {
                    // Use println! with a leading newline to write to stdout so the bridge
                    // picks it up even if it was concatenated with a previous partial message.
                    println!("\n{}", notification);
                }
            }
            Err(e) => {
                tracing::error!("Failed to request roots/list: {}", e);
                if let Ok(notification) = serde_json::to_string(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/sandbox/failed",
                    "params": { "error": e.to_string() }
                })) {
                    println!("\n{}", notification);
                }
            }
        }
    }
}
