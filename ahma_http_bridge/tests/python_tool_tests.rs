//! Python Tools Integration Tests
//!
//! Tests for Python execution tools via the HTTP bridge.
//! Each test runs against both JSON and SSE POST response modes.

mod common;
use common::{TransportMode, setup_test_mcp_for_tools};
use serde_json::json;

// ---------------------------------------------------------------------------
// python version
// ---------------------------------------------------------------------------

async fn run_python_version(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["python"]).await else {
        return;
    };

    let result = mcp
        .call_tool("python", json!({"subcommand": "version"}))
        .await;

    println!(
        "python version result: success={}, error={:?}",
        result.success, result.error
    );
    if result.success {
        println!(
            "python version output: {}",
            result.output.unwrap_or_default()
        );
    }
}

#[tokio::test]
async fn test_python_version_json() {
    run_python_version(TransportMode::Json).await;
}

#[tokio::test]
async fn test_python_version_sse() {
    run_python_version(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// python code
// ---------------------------------------------------------------------------

async fn run_python_code(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["python"]).await else {
        return;
    };

    let result = mcp
        .call_tool(
            "python",
            json!({"subcommand": "code", "command": "print('Hello from Python!')"}),
        )
        .await;

    println!(
        "python code result: success={}, error={:?}",
        result.success, result.error
    );
    if result.success {
        println!("python code output: {}", result.output.unwrap_or_default());
    }
}

#[tokio::test]
async fn test_python_code_json() {
    run_python_code(TransportMode::Json).await;
}

#[tokio::test]
async fn test_python_code_sse() {
    run_python_code(TransportMode::Sse).await;
}
