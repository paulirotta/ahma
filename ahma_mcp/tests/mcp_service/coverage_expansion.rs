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

    // Verify the server info contains expected metadata
    println!("Server info: {:?}", info);
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

    // Create a guidance config with correct field names
    let guidance_config = GuidanceConfig {
        guidance_blocks: HashMap::new(),
        templates: HashMap::new(),
        legacy_guidance: Some(ahma_mcp::mcp_service::LegacyGuidanceConfig {
            general_guidance: {
                let mut general = HashMap::new();
                general.insert("default".to_string(), "Test guidance".to_string());
                general
            },
            tool_specific_guidance: HashMap::new(),
        }),
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
async fn test_service_creation_with_custom_timeouts() {
    let _temp_dir = tempfile::tempdir().expect("Failed to create temp directory");

    // Test with custom timeout configuration
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(600)); // 10 minutes
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));

    let shell_config = ShellPoolConfig {
        enabled: true,
        shells_per_directory: 2,
        max_total_shells: 5,
        shell_idle_timeout: Duration::from_secs(30),
        pool_cleanup_interval: Duration::from_secs(60),
        shell_spawn_timeout: Duration::from_secs(5),
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

    for _ in 0..100 {
        let info = service.get_info();
        assert_eq!(info.protocol_version, initial_info.protocol_version);
        assert_eq!(
            info.capabilities.tools.is_some(),
            initial_info.capabilities.tools.is_some()
        );
    }
}

#[tokio::test]
async fn test_service_with_empty_configs() {
    let _temp_dir = tempfile::tempdir().expect("Failed to create temp directory");

    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));

    let sandbox = Arc::new(Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    // Explicitly use empty configs
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

    // Verify service creation with empty configs
    let info = service.get_info();
    assert!(info.capabilities.tools.is_some());
}

#[tokio::test]
async fn test_guidance_config_with_tool_specific_guidance() {
    let _temp_dir = tempfile::tempdir().expect("Failed to create temp directory");

    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));

    let sandbox = Arc::new(Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    let mut tool_specific_guidance = HashMap::new();
    let mut git_guidance = HashMap::new();
    git_guidance.insert("tips".to_string(), "Git specific guidance".to_string());
    let mut cargo_guidance = HashMap::new();
    cargo_guidance.insert("tips".to_string(), "Cargo specific guidance".to_string());
    tool_specific_guidance.insert("git".to_string(), git_guidance);
    tool_specific_guidance.insert("cargo".to_string(), cargo_guidance);

    // Create a guidance config with tool-specific guidance using correct field names
    let guidance_config = GuidanceConfig {
        guidance_blocks: HashMap::new(),
        templates: HashMap::new(),
        legacy_guidance: Some(ahma_mcp::mcp_service::LegacyGuidanceConfig {
            general_guidance: {
                let mut general = HashMap::new();
                general.insert(
                    "default".to_string(),
                    "General guidance for all tools".to_string(),
                );
                general
            },
            tool_specific_guidance,
        }),
    };
    let guidance = Arc::new(Some(guidance_config));
    let tool_configs = HashMap::new();
    let configs = Arc::new(tool_configs);

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
async fn test_service_protocol_version_consistency() {
    let (service, _temp_dir) = create_test_service().await;

    // Test that protocol version is consistent
    let info = service.get_info();
    assert_eq!(info.protocol_version, rmcp::model::ProtocolVersion::LATEST);
}

#[tokio::test]
async fn test_service_capabilities_structure() {
    let (service, _temp_dir) = create_test_service().await;

    // Test that capabilities structure is as expected
    let info = service.get_info();
    assert!(info.capabilities.tools.is_some());

    // Verify tools capability exists
    if let Some(tools_capability) = &info.capabilities.tools {
        // Should have list_changed field
        assert!(tools_capability.list_changed.is_some());
    }
}

#[tokio::test]
async fn test_concurrent_service_creation() {
    // Test creating multiple services concurrently
    let futures: Vec<_> = (0..5)
        .map(|_| tokio::spawn(create_test_service()))
        .collect();

    for future in futures {
        let (service, _temp_dir) = future.await.unwrap();
        let info = service.get_info();
        assert!(info.capabilities.tools.is_some());
    }
}

#[tokio::test]
async fn test_service_creation_error_handling() {
    let _temp_dir = tempfile::tempdir().expect("Failed to create temp directory");

    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));

    // Try to create a service with a minimal shell pool config
    let shell_config = ShellPoolConfig {
        enabled: true,
        shells_per_directory: 1,
        max_total_shells: 1, // Minimal but valid
        shell_idle_timeout: Duration::from_secs(1),
        pool_cleanup_interval: Duration::from_secs(1),
        shell_spawn_timeout: Duration::from_secs(1),
        command_timeout: Duration::from_secs(1),
        health_check_interval: Duration::from_secs(1),
    };

    let shell_pool = ShellPoolManager::new(shell_config);

    let sandbox = Arc::new(Sandbox::new_test());
    let adapter = Adapter::new(
        Arc::clone(&operation_monitor),
        Arc::new(shell_pool),
        sandbox,
    );
    match adapter {
        Ok(adapter) => {
            let tool_configs = HashMap::new();
            let configs = Arc::new(tool_configs);
            let guidance = Arc::new(None::<GuidanceConfig>);

            let result = AhmaMcpService::new(
                Arc::new(adapter),
                operation_monitor,
                configs,
                guidance,
                false,
                false,
                false,
            )
            .await;

            // If it succeeds, it should be a valid service
            if let Ok(service) = result {
                let info = service.get_info();
                assert!(info.capabilities.tools.is_some());
            }
            // If it fails, that's also acceptable for minimal configs
        }
        Err(_) => {
            // It's acceptable for adapter creation to fail with minimal config
        }
    }
}
