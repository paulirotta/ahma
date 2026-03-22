use ahma_mcp::adapter::Adapter;
use ahma_mcp::config::load_tool_configs;
use ahma_mcp::mcp_service::{AhmaMcpService, GuidanceConfig};
use ahma_mcp::operation_monitor::{MonitorConfig, OperationMonitor};
use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};
use ahma_mcp::utils::logging::init_test_logging;
use rmcp::handler::server::ServerHandler;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

/// Test full MCP server lifecycle with realistic scenarios
/// This validates the complete integration flow from service creation to basic operations
async fn create_test_service() -> AhmaMcpService {
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(ahma_mcp::sandbox::Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());

    // Load tool configs from .ahma directory
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
async fn test_mcp_service_creation_and_info() {
    init_test_logging();
    // Create MCP service with actual tool configurations
    let service = create_test_service().await;

    // Test basic service info
    let info = service.get_info();
    assert_eq!(info.protocol_version, rmcp::model::ProtocolVersion::LATEST);
    assert!(
        info.capabilities.tools.is_some(),
        "Server should advertise tool capabilities"
    );

    // Verify tools capability is properly configured
    let tools_capability = info.capabilities.tools.unwrap();
    // Just verify that tools capability exists - don't assume specific features
    // The capability should be a valid ToolsCapability structure
    println!("Tools capability: {:?}", tools_capability);
}

#[tokio::test]
async fn test_mcp_service_multiple_creation() {
    init_test_logging();
    // Test that multiple service instances can be created without conflicts
    let service1 = create_test_service().await;
    let service2 = create_test_service().await;

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
async fn test_mcp_service_stability_under_load() {
    init_test_logging();
    // Test service performance under sustained load
    let service = create_test_service().await;

    let start_time = Instant::now();

    // Perform many get_info operations rapidly
    for _ in 0..100 {
        let _ = service.get_info();
    }

    let elapsed = start_time.elapsed();

    // Should complete all operations quickly (less than 1 second)
    assert!(
        elapsed < Duration::from_secs(1),
        "100 get_info operations should complete very quickly"
    );

    // Service should still be responsive after load
    let final_info = service.get_info();
    assert_eq!(
        final_info.protocol_version,
        rmcp::model::ProtocolVersion::LATEST
    );
}

#[tokio::test]
async fn test_mcp_service_with_tool_configs() {
    init_test_logging();
    // Test service behavior with different tool configuration scenarios

    // Test with no tool configs
    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(300));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));
    let shell_config = ShellPoolConfig::default();
    let shell_pool = Arc::new(ShellPoolManager::new(shell_config));
    let sandbox = Arc::new(ahma_mcp::sandbox::Sandbox::new_test());
    let adapter =
        Arc::new(Adapter::new(Arc::clone(&operation_monitor), shell_pool, sandbox).unwrap());
    let configs = Arc::new(HashMap::new());
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

    // Should still work with empty configs
    let info = service.get_info();
    assert_eq!(info.protocol_version, rmcp::model::ProtocolVersion::LATEST);
    assert!(
        info.capabilities.tools.is_some(),
        "Should still advertise tool capabilities"
    );
}
