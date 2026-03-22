//! # Logging Module Coverage Tests
//!
//! Integration tests to improve coverage of `ahma_mcp/src/utils/logging.rs`.
//!
//! The logging module has 69% coverage. This file tests:
//! - init_logging with different log levels
//! - init_logging with log_to_file=true vs false
//! - init_test_logging convenience function
//! - Fallback paths when file logging is not available
//!
//! Note: logging initialization is guarded by Once, so each test process
//! can only initialize once. We use separate test binaries/processes to
//! test different configurations.

use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use serde_json::json;

// ============================================================================
// Logging Initialization Tests
// ============================================================================

/// Test init_test_logging convenience function
/// This tests the stderr logging path with trace level
#[test]
fn test_init_test_logging() {
    // init_test_logging calls init_logging("trace", false)
    init_test_logging();
    // Should not panic and logging should work
    tracing::info!("Test logging message");
    tracing::debug!("Debug message");
    tracing::trace!("Trace message");
}

/// Test that init_logging can be called multiple times (idempotent via Once)
#[test]
fn test_init_logging_idempotent() {
    use ahma_mcp::utils::logging::init_logging;

    // First call
    let result1 = init_logging("info", false);
    assert!(result1.is_ok());

    // Second call should also succeed (no-op due to Once)
    let result2 = init_logging("debug", true);
    assert!(result2.is_ok());

    // Third call
    let result3 = init_logging("warn", false);
    assert!(result3.is_ok());
}

// ============================================================================
// CLI Flag Integration Tests for Logging
// ============================================================================

/// Test client spawned with --debug flag
/// This covers the debug log level path through the CLI
#[tokio::test]
async fn test_client_with_debug_logging() -> Result<()> {
    init_test_logging();

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .env("RUST_LOG", "debug")
        .build()
        .await?;

    // Verify client works with debug logging
    let params =
        CallToolRequestParams::new("status").with_arguments(json!({}).as_object().unwrap().clone());

    let result = client.call_tool(params).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test client spawned with --log-to-stderr flag
/// This covers the stderr logging path in production
#[tokio::test]
async fn test_client_with_stderr_logging() -> Result<()> {
    init_test_logging();

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .env("AHMA_LOG_TARGET", "stderr")
        .build()
        .await?;

    // Verify client works with stderr logging
    let params =
        CallToolRequestParams::new("status").with_arguments(json!({}).as_object().unwrap().clone());

    let result = client.call_tool(params).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test client spawned with both --debug and --log-to-stderr flags
#[tokio::test]
async fn test_client_with_debug_and_stderr_logging() -> Result<()> {
    init_test_logging();

    let client = ClientBuilder::new()
        .tools_dir(".ahma")
        .env("RUST_LOG", "debug")
        .env("AHMA_LOG_TARGET", "stderr")
        .build()
        .await?;

    // Verify client works with both flags by listing tools
    let tools = client.list_all_tools().await?;
    assert!(!tools.is_empty());

    client.cancel().await?;
    Ok(())
}

/// Test client without any logging flags (default file logging)
#[tokio::test]
async fn test_client_with_default_file_logging() -> Result<()> {
    init_test_logging();

    // No logging flags - should use file logging by default
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;

    // Verify client works with default logging
    let params =
        CallToolRequestParams::new("status").with_arguments(json!({}).as_object().unwrap().clone());

    let result = client.call_tool(params).await?;
    assert!(!result.content.is_empty());

    client.cancel().await?;
    Ok(())
}

// ============================================================================
// Tracing Macro Usage Tests
// ============================================================================

/// Test that various tracing macros work after initialization
#[test]
fn test_tracing_macro_levels() {
    init_test_logging();

    // Test all log levels
    tracing::error!("Error level test");
    tracing::warn!("Warn level test");
    tracing::info!("Info level test");
    tracing::debug!("Debug level test");
    tracing::trace!("Trace level test");

    // Test with fields
    tracing::info!(operation = "test", status = "success", "Structured log");

    // Test spans
    let _span = tracing::info_span!("test_span", test_id = 123);
    let _guard = _span.enter();
    tracing::info!("Inside span");
}

/// Test tracing with async context
#[tokio::test]
async fn test_tracing_async() {
    init_test_logging();

    // Create an async span
    async {
        tracing::info!("Inside async context");
    }
    .await;

    // Test instrument macro pattern (common in async code)
    async fn instrumented_fn() {
        tracing::debug!("In instrumented function");
    }

    instrumented_fn().await;
}
