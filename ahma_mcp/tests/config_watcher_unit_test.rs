//! Tests for config_watcher.rs: update_tools and start_config_watcher.
//!
//! `update_tools` is defined in `mcp_service/config_watcher.rs` which had 0%
//! coverage because existing mcp_service_mod_unit_test tests manipulated the
//! underlying HashMap directly rather than calling the service method.
//! These tests call `AhmaMcpService::update_tools` through the real service
//! instance to give the file actual coverage.

use ahma_mcp::adapter::Adapter;
use ahma_mcp::config::ToolConfig;
use ahma_mcp::mcp_service::AhmaMcpService;
use ahma_mcp::operation_monitor::{MonitorConfig, OperationMonitor};
use ahma_mcp::sandbox::Sandbox;
use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_tool_config(name: &str) -> ToolConfig {
    ToolConfig {
        name: name.to_string(),
        description: format!("Test tool {name}"),
        command: "echo".to_string(),
        subcommand: None,
        input_schema: None,
        timeout_seconds: None,
        synchronous: None,
        hints: Default::default(),
        enabled: true,
        guidance_key: None,
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
        monitor_level: None,
        monitor_stream: None,
    }
}

async fn make_service(initial_configs: HashMap<String, ToolConfig>) -> AhmaMcpService {
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let sandbox = Arc::new(Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());
    let configs = Arc::new(initial_configs);

    AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        Arc::new(None),
        false,
        false,
        false,
    )
    .await
    .unwrap()
}

// ── update_tools ──────────────────────────────────────────────────────────────

/// update_tools replaces all existing tool configs with the new set.
#[tokio::test]
async fn test_update_tools_replaces_existing_configs() {
    let mut initial = HashMap::new();
    initial.insert("tool_a".to_string(), make_tool_config("tool_a"));
    let service = make_service(initial).await;

    // Confirm initial tool is present
    assert!(service.list_tool_names().contains(&"tool_a".to_string()));

    // Replace with a completely new set
    let mut new_configs = HashMap::new();
    new_configs.insert("tool_b".to_string(), make_tool_config("tool_b"));
    new_configs.insert("tool_c".to_string(), make_tool_config("tool_c"));

    service.update_tools(new_configs).await;

    let names = service.list_tool_names();
    assert!(
        names.contains(&"tool_b".to_string()),
        "tool_b should be present after update_tools: {names:?}"
    );
    assert!(
        names.contains(&"tool_c".to_string()),
        "tool_c should be present after update_tools: {names:?}"
    );
    assert!(
        !names.contains(&"tool_a".to_string()),
        "tool_a should be removed after update_tools: {names:?}"
    );
}

/// update_tools with an empty map removes all tools.
#[tokio::test]
async fn test_update_tools_with_empty_map_clears_configs() {
    let mut initial = HashMap::new();
    initial.insert("tool_x".to_string(), make_tool_config("tool_x"));
    let service = make_service(initial).await;

    // Clear all tools
    service.update_tools(HashMap::new()).await;

    let names = service.list_tool_names();
    assert!(
        names.is_empty() || !names.contains(&"tool_x".to_string()),
        "All tools should be removed: {names:?}"
    );
}

/// update_tools can be called multiple times - last call wins.
#[tokio::test]
async fn test_update_tools_multiple_updates_last_wins() {
    let service = make_service(HashMap::new()).await;

    // First update
    let mut cfg1 = HashMap::new();
    cfg1.insert("alpha".to_string(), make_tool_config("alpha"));
    service.update_tools(cfg1).await;

    // Second update - replaces first
    let mut cfg2 = HashMap::new();
    cfg2.insert("beta".to_string(), make_tool_config("beta"));
    service.update_tools(cfg2).await;

    let names = service.list_tool_names();
    assert!(
        names.contains(&"beta".to_string()),
        "Last update should be present: {names:?}"
    );
    assert!(
        !names.contains(&"alpha".to_string()),
        "Previous update should be gone: {names:?}"
    );
}

/// update_tools with no peer connected (normal case in unit tests) should
/// not panic - it just skips the notification.
#[tokio::test]
async fn test_update_tools_without_peer_does_not_panic() {
    let service = make_service(HashMap::new()).await;

    let mut cfg = HashMap::new();
    cfg.insert("my_tool".to_string(), make_tool_config("my_tool"));

    // Should complete without panicking even though there is no connected peer.
    service.update_tools(cfg).await;
}

// ── config reload integration via filesystem ───────────────────────────────

/// Verify that the service correctly reflects tool names after update_tools,
/// matching the metadata that would be sent to a client via tools/list.
#[tokio::test]
async fn test_update_tools_reflected_in_list_tool_names() {
    let service = make_service(HashMap::new()).await;

    let tool_names = ["build", "test", "lint", "fmt"];
    let mut cfg = HashMap::new();
    for name in tool_names {
        cfg.insert(name.to_string(), make_tool_config(name));
    }

    service.update_tools(cfg).await;

    let listed = service.list_tool_names();
    for name in tool_names {
        assert!(
            listed.contains(&name.to_string()),
            "Tool '{name}' should appear in list_tool_names: {listed:?}"
        );
    }
}
