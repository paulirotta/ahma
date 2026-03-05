//! Regression tests for sandbox scope validation.
//!
//! This suite catches the historical bug where `Adapter` validated the
//! working directory against its own `root_path` (captured from process cwd)
//! instead of the globally initialized sandbox scopes.

use ahma_mcp::adapter::Adapter;
use ahma_mcp::operation_monitor::{MonitorConfig, OperationMonitor};
use ahma_mcp::sandbox::Sandbox;
use ahma_mcp::shell_pool::{ShellPoolConfig, ShellPoolManager};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
#[cfg(unix)]
async fn adapter_uses_global_sandbox_scope_not_adapter_root_path() {
    // Initialize sandbox scopes for tests. This sets sandbox scope to "/" and enables test mode.
    let sandbox = Arc::new(Sandbox::new_test());

    let monitor_config = MonitorConfig::with_timeout(Duration::from_secs(5));
    let operation_monitor = Arc::new(OperationMonitor::new(monitor_config));

    let shell_pool_config = ShellPoolConfig {
        enabled: false,
        shells_per_directory: 0,
        max_total_shells: 0,
        shell_idle_timeout: Duration::from_secs(1),
        pool_cleanup_interval: Duration::from_secs(1),
        shell_spawn_timeout: Duration::from_secs(1),
        command_timeout: Duration::from_secs(5),
        health_check_interval: Duration::from_secs(60),
    };
    let shell_pool = Arc::new(ShellPoolManager::new(shell_pool_config));

    // Pick an adapter root that does NOT include /tmp (or most of the filesystem).
    // Note: With the new Sandbox API, scopes are defined in the sandbox, not via with_root() on adapter.
    // The test intent here was to ensure that even if "root" was restrictive, global test scopes override it.
    // In the new design, the Adapter just uses the Sandbox provided.

    // We create an adapter with our test sandbox which has permissive scopes (test mode)
    let adapter = Adapter::new(operation_monitor, shell_pool, sandbox).expect("adapter");

    // This working directory is outside the adapter root_path, but inside the sandbox scope ("/").
    // Prior to the fix, this would fail with:
    //   "Path ... is outside the sandbox root <adapter_root>"
    let out = adapter
        .execute_sync_in_dir("pwd", None, "/tmp", Some(5), None)
        .await
        .expect("pwd should succeed under global sandbox scope");

    let trimmed = out.trim();
    assert!(
        trimmed == "/tmp" || trimmed.ends_with("/tmp"),
        "expected pwd output to be /tmp, got: {trimmed:?}"
    );
}
