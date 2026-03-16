use super::common;
use crate::AhmaMcpService;
use crate::mcp_service::bundle_registry;
use crate::mcp_service::schema;
use rmcp::model::{CallToolResult, ErrorData as McpError};
use serde_json::{Map, Value};
use std::sync::Arc;

impl AhmaMcpService {
    /// Generates the specific input schema for the `activate_tools` tool.
    pub fn generate_input_schema_for_discover_tools(&self) -> Arc<Map<String, Value>> {
        let mut properties = Map::new();
        properties.insert(
            "action".to_string(),
            schema::enum_string_property(
                "Action to perform: 'list' shows available bundles, 'reveal' activates a bundle's tools",
                &["list", "reveal"],
            ),
        );
        properties.insert(
            "bundle".to_string(),
            schema::string_property(
                "Bundle name to reveal (required for 'reveal' action). Use comma-separated names to reveal multiple bundles at once.",
            ),
        );
        schema::object_input_schema(properties, &["action"])
    }

    /// Handles the `activate_tools` tool call.
    ///
    /// **list**: Returns a compact summary of available tool bundles.
    /// **reveal**: Activates one or more bundles so their tools appear in `tools/list`.
    pub async fn handle_discover_tools(
        &self,
        args: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        let action = common::opt_str(&args, "action").unwrap_or_else(|| "list".to_string());

        match action.as_str() {
            "list" => self.discover_tools_list().await,
            "reveal" => {
                let bundle_arg = common::require_str(
                    &args,
                    "bundle",
                    "The 'bundle' parameter is required for the 'reveal' action",
                )?;
                self.discover_tools_reveal(bundle_arg).await
            }
            other => Err(common::mcp_invalid_params(format!(
                "Unknown action '{}'. Valid actions: 'list', 'reveal'",
                other
            ))),
        }
    }

    /// Lists all available bundles with their disclosure status.
    async fn discover_tools_list(&self) -> Result<CallToolResult, McpError> {
        let config_keys: std::collections::HashSet<String> = {
            let configs_lock = self.configs.read().unwrap();
            configs_lock.keys().cloned().collect()
        };
        let disclosed = self.disclosed_bundles.read().unwrap().clone();

        let loaded = bundle_registry::loaded_bundle_names(&config_keys);

        if loaded.is_empty() {
            return Ok(common::text_result(
                "No tool bundles are loaded. Use CLI flags like --rust, --git, --python to enable bundles.",
            ));
        }

        // Count subcommands per bundle to give a sense of scope
        let mut entries = Vec::new();
        for bundle in &loaded {
            let tool_count = {
                let configs_lock = self.configs.read().unwrap();
                configs_lock
                    .get(bundle.config_tool_name)
                    .map(|tc| tc.subcommand.as_ref().map(|s| s.len()).unwrap_or(1))
                    .unwrap_or(0)
            };
            let revealed = disclosed.contains(bundle.name);
            entries.push(serde_json::json!({
                "bundle": bundle.name,
                "description": bundle.description,
                "tools": tool_count,
                "revealed": revealed,
            }));
        }

        let summary = serde_json::to_string_pretty(&entries).unwrap_or_default();
        Ok(common::text_result(format!(
            "Available tool bundles (use `activate_tools reveal <bundle>` to activate):\n{}",
            summary
        )))
    }

    /// Reveals one or more bundles, making their tools visible in `tools/list`.
    async fn discover_tools_reveal(&self, bundle_arg: String) -> Result<CallToolResult, McpError> {
        let bundle_names: Vec<&str> = bundle_arg.split(',').map(|s| s.trim()).collect();
        let mut revealed = Vec::new();
        let mut already = Vec::new();
        let mut unknown = Vec::new();

        let config_keys: std::collections::HashSet<String> = {
            let configs_lock = self.configs.read().unwrap();
            configs_lock.keys().cloned().collect()
        };

        for name in &bundle_names {
            match bundle_registry::find_bundle(name) {
                Some(bundle) if config_keys.contains(bundle.config_tool_name) => {
                    let mut disclosed = self.disclosed_bundles.write().unwrap();
                    if disclosed.insert(name.to_string()) {
                        revealed.push(*name);
                    } else {
                        already.push(*name);
                    }
                }
                Some(bundle) => {
                    unknown.push(format!(
                        "{} (not loaded — enable with --{})",
                        name, bundle.name
                    ));
                }
                None => {
                    unknown.push(format!("{} (unknown bundle)", name));
                }
            }
        }

        // If we revealed anything, notify the client to re-fetch tools/list
        if !revealed.is_empty() {
            self.notify_tools_changed().await;
        }

        let mut parts = Vec::new();
        if !revealed.is_empty() {
            parts.push(format!("Revealed: {}", revealed.join(", ")));
        }
        if !already.is_empty() {
            parts.push(format!("Already revealed: {}", already.join(", ")));
        }
        if !unknown.is_empty() {
            parts.push(format!("Not available: {}", unknown.join(", ")));
        }

        let message = parts.join("\n");
        Ok(common::text_result(message))
    }
}
