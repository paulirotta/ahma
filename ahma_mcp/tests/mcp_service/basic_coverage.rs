use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ahma_mcp::adapter::Adapter;
use ahma_mcp::config::load_tool_configs;
use ahma_mcp::mcp_service::{AhmaMcpService, GuidanceConfig};
use ahma_mcp::operation_monitor::{MonitorConfig, OperationMonitor};
use ahma_mcp::sandbox::Sandbox;
use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};
use rmcp::handler::server::ServerHandler;
use tempfile::TempDir;

/// Helper function to create a test AhmaMcpService instance
async fn create_test_service() -> (AhmaMcpService, TempDir) {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");

    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    // Load tool configs from .ahma directory or use empty map
    let tool_configs = if Path::new(".ahma").exists() {
        load_tool_configs(
            &ahma_mcp::shell::cli::AppConfig::default(),
            Some(Path::new(".ahma")),
        )
        .await
        .unwrap_or_default()
    } else {
        HashMap::new()
    };

    let configs = Arc::new(tool_configs);
    let guidance = Arc::new(None::<GuidanceConfig>);

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        false,
    )
    .await
    .unwrap();
    (service, temp_dir)
}

#[tokio::test]
async fn test_get_info_returns_complete_server_info() {
    let (service, _temp_dir) = create_test_service().await;

    let info = service.get_info();

    assert_eq!(info.protocol_version, rmcp::model::ProtocolVersion::LATEST);
    assert!(
        info.capabilities.tools.is_some(),
        "Server should advertise tool capabilities"
    );
}

#[tokio::test]
async fn test_service_creation_with_guidance_config() {
    let _temp_dir = tempfile::tempdir().expect("Failed to create temp directory");

    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    let tool_configs = HashMap::new();
    let configs = Arc::new(tool_configs);

    // Create a guidance config
    let guidance_config = GuidanceConfig {
        guidance_blocks: HashMap::new(),
        templates: HashMap::new(),
        legacy_guidance: None,
    };
    let guidance = Arc::new(Some(guidance_config));

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        false,
    )
    .await
    .unwrap();

    // Verify service was created successfully
    let info = service.get_info();
    assert!(info.capabilities.tools.is_some());
}

#[tokio::test]
async fn test_service_creation_with_existing_tool_configs() {
    let _temp_dir = tempfile::tempdir().expect("Failed to create temp directory");

    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    // Load actual tool configs if they exist
    let tool_configs = if Path::new(".ahma").exists() {
        load_tool_configs(
            &ahma_mcp::shell::cli::AppConfig::default(),
            Some(Path::new(".ahma")),
        )
        .await
        .unwrap_or_default()
    } else {
        HashMap::new()
    };

    let configs = Arc::new(tool_configs);
    let guidance = Arc::new(None::<GuidanceConfig>);

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        false,
    )
    .await
    .unwrap();

    // Verify service was created successfully
    let info = service.get_info();
    assert!(info.capabilities.tools.is_some());
}

#[tokio::test]
async fn test_service_creation_with_custom_shell_config() {
    let _temp_dir = tempfile::tempdir().expect("Failed to create temp directory");

    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(600));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));

    let shell_config = ShellPoolConfig {
        enabled: true,
        shells_per_directory: 1,
        max_total_shells: 5,
        shell_idle_timeout: Duration::from_secs(30),
        pool_cleanup_interval: Duration::from_secs(60),
        shell_spawn_timeout: Duration::from_secs(10),
        command_timeout: Duration::from_secs(120),
        health_check_interval: Duration::from_secs(30),
    };
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    let tool_configs = HashMap::new();
    let configs = Arc::new(tool_configs);
    let guidance = Arc::new(None::<GuidanceConfig>);

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        false,
    )
    .await
    .unwrap();

    // Verify service was created successfully with custom configuration
    let info = service.get_info();
    assert!(info.capabilities.tools.is_some());
}

#[tokio::test]
async fn test_multiple_service_instances() {
    // Test that multiple service instances can be created concurrently
    let (service1, _temp_dir1) = create_test_service().await;
    let (service2, _temp_dir2) = create_test_service().await;

    // Both should provide consistent info
    let info1 = service1.get_info();
    let info2 = service2.get_info();

    assert_eq!(info1.protocol_version, info2.protocol_version);
    assert_eq!(
        info1.capabilities.tools.is_some(),
        info2.capabilities.tools.is_some()
    );
}

#[tokio::test]
async fn test_service_stability_under_repeated_info_calls() {
    let (service, _temp_dir) = create_test_service().await;

    // Test that get_info is stable under repeated calls
    let initial_info = service.get_info();

    for _ in 0..50 {
        let info = service.get_info();
        assert_eq!(info.protocol_version, initial_info.protocol_version);
        assert_eq!(
            info.capabilities.tools.is_some(),
            initial_info.capabilities.tools.is_some()
        );
    }
}

#[tokio::test]
async fn test_service_protocol_version_consistency() {
    let (service, _temp_dir) = create_test_service().await;

    let info = service.get_info();

    // Verify the protocol version is the expected one
    assert_eq!(info.protocol_version, rmcp::model::ProtocolVersion::LATEST);

    // The protocol version should be consistent across calls
    let info2 = service.get_info();
    assert_eq!(info.protocol_version, info2.protocol_version);
}

#[tokio::test]
async fn test_service_capabilities_structure() {
    let (service, _temp_dir) = create_test_service().await;

    let info = service.get_info();

    // Verify tools capability exists
    assert!(
        info.capabilities.tools.is_some(),
        "Tools capability should be present"
    );

    // Test that the capability is consistent
    let info2 = service.get_info();
    assert_eq!(
        info.capabilities.tools.is_some(),
        info2.capabilities.tools.is_some()
    );
}

#[tokio::test]
async fn test_concurrent_service_creation() {
    // Test creating multiple services concurrently
    let handles: Vec<_> = (0..3)
        .map(|_| tokio::spawn(async { create_test_service().await }))
        .collect();

    let results = futures::future::join_all(handles).await;

    // All should succeed
    for result in results {
        let (service, _temp_dir) = result.expect("Service creation should succeed");
        let info = service.get_info();
        assert!(info.capabilities.tools.is_some());
    }
}

#[tokio::test]
async fn test_service_with_guidance_blocks() {
    let _temp_dir = tempfile::tempdir().expect("Failed to create temp directory");

    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    let tool_configs = HashMap::new();
    let configs = Arc::new(tool_configs);

    // Create guidance config with guidance blocks
    let mut guidance_blocks = HashMap::new();
    guidance_blocks.insert("general".to_string(), "Use tools carefully".to_string());
    guidance_blocks.insert(
        "shell".to_string(),
        "Shell commands should be safe".to_string(),
    );

    let guidance_config = GuidanceConfig {
        guidance_blocks,
        templates: HashMap::new(),
        legacy_guidance: None,
    };
    let guidance = Arc::new(Some(guidance_config));

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        false,
    )
    .await
    .unwrap();

    // Verify service was created successfully
    let info = service.get_info();
    assert!(info.capabilities.tools.is_some());
}

#[tokio::test]
async fn test_service_disabled_shell_pool() {
    let _temp_dir = tempfile::tempdir().expect("Failed to create temp directory");

    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));

    // Create shell config with pool disabled
    let shell_config = ShellPoolConfig {
        enabled: false,
        shells_per_directory: 1,
        max_total_shells: 1,
        shell_idle_timeout: Duration::from_secs(30),
        pool_cleanup_interval: Duration::from_secs(60),
        shell_spawn_timeout: Duration::from_secs(10),
        command_timeout: Duration::from_secs(120),
        health_check_interval: Duration::from_secs(30),
    };
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(Sandbox::new_test());

    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    let tool_configs = HashMap::new();
    let configs = Arc::new(tool_configs);
    let guidance = Arc::new(None::<GuidanceConfig>);

    let service = AhmaMcpService::new(
        adapter,
        operation_monitor,
        configs,
        guidance,
        false,
        false,
        false,
    )
    .await
    .unwrap();

    // Should still work with disabled shell pool
    let info = service.get_info();
    assert!(info.capabilities.tools.is_some());
}
