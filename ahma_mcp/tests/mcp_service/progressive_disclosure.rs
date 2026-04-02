//! Tests for progressive disclosure of tool bundles.
//!
//! Validates that:
//! - With progressive disclosure enabled, only built-in + activate_tools are listed initially
//! - `activate_tools list` returns available bundles
//! - `activate_tools reveal <bundle>` makes bundle tools visible
//! - With progressive disclosure disabled, all tools are listed immediately (legacy)
//! - The `instructions` field in get_info() is populated

use ahma_mcp::adapter::Adapter;
use ahma_mcp::config::load_tool_configs;
use ahma_mcp::mcp_service::bundle_registry;
use ahma_mcp::mcp_service::{AhmaMcpService, GuidanceConfig};
use ahma_mcp::operation_monitor::{MonitorConfig, OperationMonitor};
use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};
use ahma_mcp::utils::logging::init_test_logging;
use rmcp::handler::server::ServerHandler;
use std::sync::Arc;
use std::time::Duration;

/// Creates a test service with progressive disclosure ON and rust+git bundles loaded.
async fn create_pd_service() -> AhmaMcpService {
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(ahma_mcp::sandbox::Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    // Load rust + git bundles
    let config = ahma_mcp::shell::cli::AppConfig {
        tool_bundles: vec!["rust".to_string(), "git".to_string()],
        ..ahma_mcp::shell::cli::AppConfig::default()
    };
    let tool_configs = load_tool_configs(&config, None).await.unwrap_or_default();

    let configs = Arc::new(tool_configs);
    let guidance = Arc::new(None::<GuidanceConfig>);

    // progressive_disclosure = true (7th arg)
    AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        true,
    )
    .await
    .unwrap()
}

/// Creates a test service with progressive disclosure OFF (legacy behavior).
async fn create_legacy_service() -> AhmaMcpService {
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(ahma_mcp::sandbox::Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    let config = ahma_mcp::shell::cli::AppConfig {
        tool_bundles: vec!["rust".to_string(), "git".to_string()],
        ..ahma_mcp::shell::cli::AppConfig::default()
    };
    let tool_configs = load_tool_configs(&config, None).await.unwrap_or_default();

    let configs = Arc::new(tool_configs);
    let guidance = Arc::new(None::<GuidanceConfig>);

    // progressive_disclosure = false (7th arg)
    AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        false,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn test_progressive_disclosure_initial_tools() {
    init_test_logging();
    let service = create_pd_service().await;

    let tool_names = service.list_tool_names();

    // Built-in tools should always be present
    assert!(
        tool_names.contains(&"await".to_string()),
        "await should be listed"
    );
    assert!(
        tool_names.contains(&"status".to_string()),
        "status should be listed"
    );
    assert!(
        tool_names.contains(&"sandboxed_shell".to_string()),
        "sandboxed_shell should be listed"
    );
    assert!(
        tool_names.contains(&"activate_tools".to_string()),
        "activate_tools should be listed"
    );

    // Bundle tools should NOT be listed yet
    assert!(
        !tool_names.contains(&"cargo".to_string()),
        "cargo should be hidden before reveal"
    );
    assert!(
        !tool_names.contains(&"git".to_string()),
        "git should be hidden before reveal"
    );

    // Exactly 4 tools: await, status, sandboxed_shell, activate_tools
    assert_eq!(
        tool_names.len(),
        4,
        "Should have exactly 4 tools initially, got: {:?}",
        tool_names
    );
}

#[tokio::test]
async fn test_progressive_disclosure_legacy_shows_all() {
    init_test_logging();
    let service = create_legacy_service().await;

    let tool_names = service.list_tool_names();

    // Built-in tools present
    assert!(tool_names.contains(&"await".to_string()));
    assert!(tool_names.contains(&"status".to_string()));
    assert!(tool_names.contains(&"sandboxed_shell".to_string()));

    // activate_tools should NOT be present when PD is off
    assert!(
        !tool_names.contains(&"activate_tools".to_string()),
        "activate_tools should not appear when progressive disclosure is disabled"
    );

    // Bundle tools should be listed immediately
    assert!(
        tool_names.contains(&"cargo".to_string()),
        "cargo should be listed in legacy mode"
    );
    assert!(
        tool_names.contains(&"git".to_string()),
        "git should be listed in legacy mode"
    );
}

#[tokio::test]
async fn test_activate_tools_list_action() {
    init_test_logging();
    let service = create_pd_service().await;

    let args = serde_json::from_value(serde_json::json!({ "action": "list" })).unwrap();
    let result = service.handle_discover_tools(args).await.unwrap();

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or("");

    assert!(text.contains("rust"), "Should list rust bundle");
    assert!(text.contains("git"), "Should list git bundle");
    assert!(
        text.contains("\"revealed\": false"),
        "Bundles should initially be unrevealed"
    );
}

#[tokio::test]
async fn test_activate_tools_reveal_single_bundle() {
    init_test_logging();
    let service = create_pd_service().await;

    // Reveal rust
    let args = serde_json::from_value(serde_json::json!({
        "action": "reveal",
        "bundle": "rust"
    }))
    .unwrap();
    let result = service.handle_discover_tools(args).await.unwrap();

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or("");
    assert!(text.contains("Revealed: rust"), "Should confirm reveal");

    // Now list_tool_names should include cargo but not git
    let tool_names = service.list_tool_names();

    assert!(
        tool_names.contains(&"cargo".to_string()),
        "cargo should now be visible after reveal"
    );
    assert!(
        !tool_names.contains(&"git".to_string()),
        "git should still be hidden"
    );
}

#[tokio::test]
async fn test_activate_tools_reveal_multiple_bundles() {
    init_test_logging();
    let service = create_pd_service().await;

    // Reveal rust AND git
    let args = serde_json::from_value(serde_json::json!({
        "action": "reveal",
        "bundle": "rust,git"
    }))
    .unwrap();
    let result = service.handle_discover_tools(args).await.unwrap();

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or("");
    assert!(text.contains("rust"), "Should confirm rust");
    assert!(text.contains("git"), "Should confirm git");

    // Both should now be visible
    let tool_names = service.list_tool_names();
    assert!(tool_names.contains(&"cargo".to_string()));
    assert!(tool_names.contains(&"git".to_string()));
}

#[tokio::test]
async fn test_activate_tools_reveal_already_revealed() {
    init_test_logging();
    let service = create_pd_service().await;

    // Reveal rust
    let args = serde_json::from_value(serde_json::json!({
        "action": "reveal",
        "bundle": "rust"
    }))
    .unwrap();
    service.handle_discover_tools(args).await.unwrap();

    // Reveal rust again
    let args = serde_json::from_value(serde_json::json!({
        "action": "reveal",
        "bundle": "rust"
    }))
    .unwrap();
    let result = service.handle_discover_tools(args).await.unwrap();

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or("");
    assert!(
        text.contains("Already revealed: rust"),
        "Should indicate already revealed"
    );
}

#[tokio::test]
async fn test_activate_tools_reveal_unknown_bundle() {
    init_test_logging();
    let service = create_pd_service().await;

    let args = serde_json::from_value(serde_json::json!({
        "action": "reveal",
        "bundle": "nonexistent"
    }))
    .unwrap();
    let result = service.handle_discover_tools(args).await.unwrap();

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or("");
    assert!(
        text.contains("unknown bundle"),
        "Should indicate unknown bundle"
    );
}

#[tokio::test]
async fn test_activate_tools_invalid_action() {
    init_test_logging();
    let service = create_pd_service().await;

    let args = serde_json::from_value(serde_json::json!({ "action": "destroy" })).unwrap();
    let result = service.handle_discover_tools(args).await;

    assert!(result.is_err(), "Invalid action should return an error");
}

#[tokio::test]
async fn test_activate_tools_reveal_missing_bundle_param() {
    init_test_logging();
    let service = create_pd_service().await;

    let args = serde_json::from_value(serde_json::json!({ "action": "reveal" })).unwrap();
    let result = service.handle_discover_tools(args).await;

    assert!(
        result.is_err(),
        "Reveal without bundle param should return an error"
    );
}

#[tokio::test]
async fn test_instructions_populated_with_pd() {
    init_test_logging();
    let service = create_pd_service().await;
    let info = service.get_info();

    assert!(
        info.instructions.is_some(),
        "instructions should be populated"
    );
    let instructions = info.instructions.unwrap();
    assert!(
        instructions.contains("sandboxed_shell"),
        "instructions should mention sandboxed_shell"
    );
    assert!(
        instructions.contains("activate_tools"),
        "instructions should mention activate_tools when PD is enabled"
    );
}

#[tokio::test]
async fn test_instructions_populated_without_pd() {
    init_test_logging();
    let service = create_legacy_service().await;
    let info = service.get_info();

    assert!(
        info.instructions.is_some(),
        "instructions should be populated even without PD"
    );
    let instructions = info.instructions.unwrap();
    assert!(
        instructions.contains("sandboxed_shell"),
        "instructions should mention sandboxed_shell"
    );
    assert!(
        !instructions.contains("activate_tools"),
        "instructions should NOT mention activate_tools when PD is disabled"
    );
}

#[tokio::test]
async fn test_bundle_registry_find_bundle() {
    assert!(bundle_registry::find_bundle("rust").is_some());
    assert!(bundle_registry::find_bundle("git").is_some());
    assert!(bundle_registry::find_bundle("python").is_some());
    assert!(bundle_registry::find_bundle("nonexistent").is_none());
}

#[tokio::test]
async fn test_bundle_registry_loaded_bundle_names() {
    let mut keys = std::collections::HashSet::new();
    keys.insert("cargo".to_string());
    keys.insert("git".to_string());

    let loaded = bundle_registry::loaded_bundle_names(&keys);
    let names: Vec<&str> = loaded.iter().map(|b| b.name).collect();

    assert!(names.contains(&"rust")); // cargo -> rust bundle
    assert!(names.contains(&"git")); // git -> git bundle
    assert!(!names.contains(&"python")); // python not loaded
}

// ==================== Auto-reveal CLI-flagged bundles ====================

/// Creates a service with PD ON and CLI-flagged bundles pre-disclosed.
async fn create_pd_service_with_auto_reveal() -> AhmaMcpService {
    let monitor_config =
        ahma_mcp::operation_monitor::MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(ahma_mcp::operation_monitor::OperationMonitor::new(
        monitor_config,
    ));
    let shell_config = ahma_mcp::shell_pool::ShellPoolConfig::default();
    let shell_pool = Arc::new(ahma_mcp::shell_pool::ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(ahma_mcp::sandbox::Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    let config = ahma_mcp::shell::cli::AppConfig {
        tool_bundles: vec!["rust".to_string(), "git".to_string()],
        ..ahma_mcp::shell::cli::AppConfig::default()
    };
    let tool_configs = load_tool_configs(&config, None).await.unwrap_or_default();
    let configs = Arc::new(tool_configs);
    let guidance = Arc::new(None::<GuidanceConfig>);

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        true, // progressive_disclosure = true
    )
    .await
    .unwrap();

    // Simulate what server.rs does: auto-reveal CLI-flagged bundles
    let cli_bundles = ahma_mcp::config::cli_flagged_bundle_names(&config);
    service.pre_disclose(&cli_bundles);

    service
}

#[tokio::test]
async fn test_cli_flagged_bundles_auto_revealed() {
    init_test_logging();
    let service = create_pd_service_with_auto_reveal().await;

    let tool_names = service.list_tool_names();

    // CLI-flagged bundles (--rust, --git) should be visible immediately
    assert!(
        tool_names.contains(&"cargo".to_string()),
        "cargo should be visible when --rust was passed, got: {:?}",
        tool_names
    );
    assert!(
        tool_names.contains(&"git".to_string()),
        "git should be visible when --git was passed, got: {:?}",
        tool_names
    );

    // Built-in tools should still be present
    assert!(tool_names.contains(&"await".to_string()));
    assert!(tool_names.contains(&"status".to_string()));
    assert!(tool_names.contains(&"sandboxed_shell".to_string()));

    // activate_tools should still be present (there may be other non-flagged bundles)
    assert!(tool_names.contains(&"activate_tools".to_string()));
}

#[tokio::test]
async fn test_cli_flagged_bundle_names_reflects_flags() {
    // Verify the helper function correctly maps CLI flags to bundle names
    let config = ahma_mcp::shell::cli::AppConfig {
        tool_bundles: vec!["rust".to_string(), "simplify".to_string()],
        ..ahma_mcp::shell::cli::AppConfig::default()
    };
    let names = ahma_mcp::config::cli_flagged_bundle_names(&config);

    assert!(names.contains("rust"), "Should contain rust");
    assert!(names.contains("simplify"), "Should contain simplify");
    assert!(
        !names.contains("git"),
        "Should not contain git (not flagged)"
    );
    assert_eq!(names.len(), 2, "Should have exactly 2 flagged bundles");
}

#[tokio::test]
async fn test_cli_flagged_bundle_names_empty_without_flags() {
    let config = ahma_mcp::shell::cli::AppConfig::default();
    let names = ahma_mcp::config::cli_flagged_bundle_names(&config);
    assert!(names.is_empty(), "No flags should produce empty set");
}

#[tokio::test]
async fn test_non_flagged_bundles_remain_hidden_with_auto_reveal() {
    init_test_logging();

    // Create service with only --rust flagged (not --git)
    let monitor_config =
        ahma_mcp::operation_monitor::MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(ahma_mcp::operation_monitor::OperationMonitor::new(
        monitor_config,
    ));
    let shell_config = ahma_mcp::shell_pool::ShellPoolConfig::default();
    let shell_pool = Arc::new(ahma_mcp::shell_pool::ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(ahma_mcp::sandbox::Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    // Load with rust AND git but only flag rust for auto-reveal
    let config_load = ahma_mcp::shell::cli::AppConfig {
        tool_bundles: vec!["rust".to_string(), "git".to_string()],
        ..ahma_mcp::shell::cli::AppConfig::default()
    };
    let tool_configs = load_tool_configs(&config_load, None)
        .await
        .unwrap_or_default();
    let configs = Arc::new(tool_configs);
    let guidance = Arc::new(None::<GuidanceConfig>);

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        true,
    )
    .await
    .unwrap();

    // Only pre-disclose "rust" (simulating rust flag only)
    let config_reveal = ahma_mcp::shell::cli::AppConfig {
        tool_bundles: vec!["rust".to_string()],
        ..ahma_mcp::shell::cli::AppConfig::default()
    };
    let cli_bundles = ahma_mcp::config::cli_flagged_bundle_names(&config_reveal);
    service.pre_disclose(&cli_bundles);

    let tool_names = service.list_tool_names();

    // rust should be visible (auto-revealed)
    assert!(
        tool_names.contains(&"cargo".to_string()),
        "cargo should be visible (auto-revealed)"
    );
    // git should be hidden (not flagged, requires activate_tools reveal)
    assert!(
        !tool_names.contains(&"git".to_string()),
        "git should be hidden (not CLI-flagged)"
    );
}

/// Verifies that `--tools rust` reveals bundles immediately (the default behavior).
/// This is the regression test for the old R1.5.6 regression where --tools hid bundles.
#[tokio::test]
async fn test_tools_reveal_bundles_by_default() {
    init_test_logging();

    let monitor_config =
        ahma_mcp::operation_monitor::MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(ahma_mcp::operation_monitor::OperationMonitor::new(
        monitor_config,
    ));
    let shell_config = ahma_mcp::shell_pool::ShellPoolConfig::default();
    let shell_pool = Arc::new(ahma_mcp::shell_pool::ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(ahma_mcp::sandbox::Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    // Load rust bundle (as --tools rust would).
    let config = ahma_mcp::shell::cli::AppConfig {
        tool_bundles: vec!["rust".to_string()],
        ..ahma_mcp::shell::cli::AppConfig::default()
    };
    let tool_configs = load_tool_configs(&config, None).await.unwrap_or_default();
    let configs = Arc::new(tool_configs);
    let guidance = Arc::new(None::<GuidanceConfig>);

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        true, // progressive_disclosure = true
    )
    .await
    .unwrap();

    // Simulate server.rs: always pre-disclose CLI-flagged bundles.
    let cli_bundles = ahma_mcp::config::cli_flagged_bundle_names(&config);
    service.pre_disclose(&cli_bundles);

    let tool_names = service.list_tool_names();

    // cargo tools should be visible (--tools rust pre-discloses the rust bundle)
    assert!(
        tool_names.contains(&"cargo".to_string()),
        "cargo should be visible when --tools rust was passed, got: {:?}",
        tool_names
    );

    // sandboxed_shell must still be present (built-in, not bundle-gated)
    assert!(
        tool_names.contains(&"sandboxed_shell".to_string()),
        "sandboxed_shell must always be visible, got: {:?}",
        tool_names
    );
}
