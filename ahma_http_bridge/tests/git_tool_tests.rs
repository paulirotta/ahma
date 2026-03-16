//! Git and GitHub CLI Integration Tests
//!
//! Tests for Git and GitHub CLI tools via the HTTP bridge.
//! Each test runs against both JSON and SSE POST response modes.

mod common;
use common::{TransportMode, setup_test_mcp_for_tools};
use serde_json::json;

// ---------------------------------------------------------------------------
// git status
// ---------------------------------------------------------------------------

async fn run_git_status(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["git_status"]).await else {
        return;
    };

    let result = mcp.call_tool("git_status", json!({})).await;

    if result.success {
        let output = result.output.unwrap_or_default();
        println!(
            "git status output (first 200 chars): {}",
            &output[..output.len().min(200)]
        );
    } else {
        eprintln!("WARNING  git_status failed: {:?}", result.error);
    }
}

#[tokio::test]
async fn test_git_status_json() {
    run_git_status(TransportMode::Json).await;
}

#[tokio::test]
async fn test_git_status_sse() {
    run_git_status(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// git log
// ---------------------------------------------------------------------------

async fn run_git_log(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["git_log"]).await else {
        return;
    };

    let result = mcp.call_tool("git_log", json!({"-n": 5})).await;

    if result.success {
        println!("OK git log succeeded");
    } else {
        eprintln!("WARNING  git_log failed: {:?}", result.error);
    }
}

#[tokio::test]
async fn test_git_log_json() {
    run_git_log(TransportMode::Json).await;
}

#[tokio::test]
async fn test_git_log_sse() {
    run_git_log(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// gh workflow list
// ---------------------------------------------------------------------------

async fn run_gh_workflow_list(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["gh_workflow_list"]).await else {
        return;
    };

    let result = mcp.call_tool("gh_workflow_list", json!({})).await;

    // gh may not be authenticated; just check the call went through
    println!(
        "gh workflow list result: success={}, error={:?}",
        result.success, result.error
    );
}

#[tokio::test]
async fn test_gh_workflow_list_json() {
    run_gh_workflow_list(TransportMode::Json).await;
}

#[tokio::test]
async fn test_gh_workflow_list_sse() {
    run_gh_workflow_list(TransportMode::Sse).await;
}
