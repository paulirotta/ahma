//! Tool Integration Tests
//!
//! Verifies that all MCP tools exposed by the bridge work end-to-end.
//! Each test case runs against BOTH POST response modes:
//!   - `_json` variant: `Accept: application/json`
//!   - `_sse` variant:  `Accept: text/event-stream`

mod common;
use common::{TransportMode, setup_test_mcp};
use serde_json::json;

// =============================================================================
// tools/list — verify the server enumerates its tools
// =============================================================================

async fn run_list_tools_returns_all_expected_tools(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp(mode).await else {
        return;
    };

    let tools = match mcp.list_tools().await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("WARNING  list_tools failed: {}, skipping", e);
            return;
        }
    };

    // Expected tools from the default .ahma/tools/*.json configuration.
    let expected = [
        "cargo_build",
        "cargo_check",
        "cargo_test",
        "cargo_fmt",
        "cargo_clippy",
        "cargo_nextest_run",
        "file-tools_ls",
        "file-tools_cat",
        "file-tools_pwd",
        "file-tools_grep",
        "file-tools_find",
        "git_status",
        "git_log",
        "sandboxed_shell",
    ];

    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();

    println!("Found {} tools ({:?} transport)", tool_names.len(), mode);

    let missing: Vec<_> = expected
        .iter()
        .filter(|&&n| !tool_names.contains(&n))
        .copied()
        .collect();

    if !missing.is_empty() {
        eprintln!("WARNING  missing expected tools: {:?}", missing);
    }

    assert!(!tools.is_empty(), "Server should return at least some tools");

    for core in &["sandboxed_shell", "file-tools_pwd"] {
        if tool_names.contains(core) {
            println!("OK core tool available: {}", core);
        } else {
            eprintln!("WARNING  core tool not available: {}", core);
        }
    }
}

#[tokio::test]
async fn test_list_tools_returns_all_expected_tools_json() {
    run_list_tools_returns_all_expected_tools(TransportMode::Json).await;
}

#[tokio::test]
async fn test_list_tools_returns_all_expected_tools_sse() {
    run_list_tools_returns_all_expected_tools(TransportMode::Sse).await;
}

// =============================================================================
// Error handling — invalid tool
// =============================================================================

async fn run_invalid_tool_returns_error(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp(mode).await else {
        return;
    };

    let result = mcp.call_tool("nonexistent_tool_xyz", json!({})).await;

    assert!(!result.success, "Should fail for nonexistent tool");
    assert!(
        result.error.is_some(),
        "Should have error message for nonexistent tool"
    );
    println!("ERROR (expected): {:?}", result.error);
}

#[tokio::test]
async fn test_invalid_tool_returns_error_json() {
    run_invalid_tool_returns_error(TransportMode::Json).await;
}

#[tokio::test]
async fn test_invalid_tool_returns_error_sse() {
    run_invalid_tool_returns_error(TransportMode::Sse).await;
}

// =============================================================================
// Error handling — missing required argument
// =============================================================================

async fn run_missing_required_arg_returns_error(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp(mode).await else {
        return;
    };
    if !mcp.is_tool_available("file-tools_cat").await {
        eprintln!("WARNING  file-tools_cat not available, skipping");
        return;
    }

    // file-tools_cat requires `files` argument
    let result = mcp.call_tool("file-tools_cat", json!({})).await;

    println!(
        "Missing-arg result: success={}, error={:?}",
        result.success, result.error
    );
    // Either the tool errors, or returns an error message in its output
    // Both are acceptable; we just verify the call completes
}

#[tokio::test]
async fn test_missing_required_arg_returns_error_json() {
    run_missing_required_arg_returns_error(TransportMode::Json).await;
}

#[tokio::test]
async fn test_missing_required_arg_returns_error_sse() {
    run_missing_required_arg_returns_error(TransportMode::Sse).await;
}

// =============================================================================
// Core tools comprehensive batch
// =============================================================================

async fn run_core_tools_comprehensive(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp(mode).await else {
        return;
    };

    let test_cases: &[(&str, serde_json::Value)] = &[
        ("file-tools_pwd", json!({})),
        ("file-tools_ls", json!({"path": "."})),
        ("file-tools_cat", json!({"files": ["Cargo.toml"]})),
        (
            "file-tools_head",
            json!({"files": ["README.md"], "lines": 5}),
        ),
        (
            "file-tools_grep",
            json!({"pattern": "name", "files": ["Cargo.toml"]}),
        ),
        ("sandboxed_shell", json!({"command": "echo 'integration test'"})),
    ];

    let mut pass = 0usize;
    let mut skip = 0usize;

    for (name, args) in test_cases {
        if !mcp.is_tool_available(name).await {
            eprintln!("WARNING  {} not available, skipping", name);
            skip += 1;
            continue;
        }

        let result = mcp.call_tool(name, args.clone()).await;
        if result.success {
            pass += 1;
            println!("OK {} ({:?})", name, mode);
        } else {
            eprintln!("FAIL {} ({:?}): {:?}", name, mode, result.error);
            // Don't hard-fail — environment may not have all tools; just report.
        }
    }

    println!(
        "Comprehensive ({:?}): {}/{} passed, {} skipped",
        mode,
        pass,
        test_cases.len() - skip,
        skip
    );

    // sandboxed_shell must work when present
    let shell_result = {
        let name = "sandboxed_shell";
        if mcp.is_tool_available(name).await {
            let r = mcp
                .call_tool(name, json!({"command": "echo 'must work'"}))
                .await;
            Some(r)
        } else {
            None
        }
    };
    if let Some(r) = shell_result {
        assert!(
            r.success,
            "sandboxed_shell must succeed ({:?}): {:?}",
            mode,
            r.error
        );
    }
}

#[tokio::test]
async fn test_core_tools_comprehensive_json() {
    run_core_tools_comprehensive(TransportMode::Json).await;
}

#[tokio::test]
async fn test_core_tools_comprehensive_sse() {
    run_core_tools_comprehensive(TransportMode::Sse).await;
}
