//! Tests for path security and sandboxing
use ahma_mcp::skip_if_disabled_async;

use ahma_mcp::test_utils::in_process::create_in_process_mcp_with_scope;
use ahma_mcp::utils::logging::init_test_logging;
use rmcp::{ServiceError, model::CallToolRequestParams};
use serde_json::json;

/// Path validation - validates that working_directory is within allowed workspace
#[tokio::test]
async fn test_path_validation_success() {
    init_test_logging();
    skip_if_disabled_async!("sandboxed_shell");
    let scope = std::env::current_dir().unwrap();
    let mcp = create_in_process_mcp_with_scope(&scope.join(".ahma"), vec![scope])
        .await
        .unwrap();

    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": "echo test",
            "working_directory": "."
        }))
        .unwrap(),
    );

    let result = mcp.client.call_tool(params).await;
    assert!(result.is_ok());
}

/// Test path validation rejects absolute paths outside workspace
#[tokio::test]
async fn test_path_validation_failure_absolute() {
    init_test_logging();
    skip_if_disabled_async!("sandboxed_shell");
    let scope = std::env::current_dir().unwrap();
    let mcp = create_in_process_mcp_with_scope(&scope.join(".ahma"), vec![scope])
        .await
        .unwrap();

    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": "echo test",
            "working_directory": "/etc"
        }))
        .unwrap(),
    );

    let result = mcp.client.call_tool(params).await;
    assert!(result.is_err());
    let error = result.unwrap_err();
    match error {
        ServiceError::McpError(mcp_error) => {
            // The error code might be INTERNAL_ERROR (-32603) because it comes from anyhow error in adapter
            // or INVALID_PARAMS (-32602) if we mapped it.
            // In mcp_service.rs, we map Err(e) to McpError::internal_error.
            // So we should expect INTERNAL_ERROR.
            // assert_eq!(mcp_error.code, ErrorCode::INVALID_PARAMS);
            // Default mode is now async, so error says "Async execution failed"
            assert!(mcp_error.message.contains("Async execution failed"));
            assert!(mcp_error.message.contains("outside the sandbox root"));
        }
        _ => panic!("Expected McpError, got {:?}", error),
    }
}

/// Test path validation rejects relative paths that escape workspace
#[tokio::test]
async fn test_path_validation_failure_relative() {
    init_test_logging();
    skip_if_disabled_async!("sandboxed_shell");
    let scope = std::env::current_dir().unwrap();
    let mcp = create_in_process_mcp_with_scope(&scope.join(".ahma"), vec![scope])
        .await
        .unwrap();

    let params = CallToolRequestParams::new("sandboxed_shell").with_arguments(
        serde_json::from_value(json!({
            "command": "echo test",
            "working_directory": "../"
        }))
        .unwrap(),
    );

    let result = mcp.client.call_tool(params).await;
    assert!(result.is_err());
    let error = result.unwrap_err();
    match error {
        ServiceError::McpError(mcp_error) => {
            // Default mode is now async, so error says "Async execution failed"
            assert!(mcp_error.message.contains("Async execution failed"));
            assert!(mcp_error.message.contains("outside the sandbox root"));
        }
        _ => panic!("Expected McpError, got {:?}", error),
    }
}
