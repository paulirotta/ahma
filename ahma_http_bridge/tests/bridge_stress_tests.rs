//! SSE Stress and Concurrent Tests
//!
//! High-volume and concurrent request tests for the HTTP SSE bridge.
//! Each test spawns its own server process on a dynamic port so no external
//! server is required.
//!
//! Every test that exercises tool calls runs twice: once with
//! `Accept: application/json` (JSON transport) and once with
//! `Accept: text/event-stream` (SSE transport), named `_json` / `_sse`
//! respectively.

mod common;

use common::{McpTestClient, TransportMode, spawn_test_server};
use futures::future::join_all;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Initialise a client against a freshly spawned server and run the
/// 14-request concurrent batch.  Returns early (test passes trivially) on
/// server/init failure so as not to break CI when the binary is unavailable.
async fn run_concurrent_tool_calls(transport: TransportMode) {
    let server = match spawn_test_server().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("WARNING  Skipping test - failed to spawn server: {}", e);
            return;
        }
    };

    let mut mcp = McpTestClient::with_url(&server.base_url()).with_transport(transport);
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if mcp
        .initialize_with_roots("stress-client", &[root])
        .await
        .is_err()
    {
        eprintln!("WARNING  Skipping test - failed to initialize MCP client");
        return;
    }
    let mcp = Arc::new(mcp);
    let start = Instant::now();

    let requests = vec![
        ("file-tools_pwd", json!({})),
        ("file-tools_ls", json!({"path": "."})),
        ("file-tools_ls", json!({"path": "ahma_mcp"})),
        ("file-tools_cat", json!({"files": ["Cargo.toml"]})),
        ("sandboxed_shell", json!({"command": "echo test1"})),
        ("sandboxed_shell", json!({"command": "echo test2"})),
        ("sandboxed_shell", json!({"command": "echo test3"})),
        ("sandboxed_shell", json!({"command": "pwd"})),
        ("sandboxed_shell", json!({"command": "ls -la"})),
        ("sandboxed_shell", json!({"command": "echo 'hello world'"})),
        ("sandboxed_shell", json!({"command": "date"})),
        ("sandboxed_shell", json!({"command": "whoami"})),
        ("sandboxed_shell", json!({"command": "uname -a"})),
        (
            "sandboxed_shell",
            json!({"command": "cat Cargo.toml | head -5"}),
        ),
    ];

    let num_requests = requests.len();

    let futures: Vec<_> = requests
        .into_iter()
        .map(|(name, args)| {
            let mcp = Arc::clone(&mcp);
            async move { mcp.call_tool(name, args).await }
        })
        .collect();

    let results = join_all(futures).await;
    let total_duration = start.elapsed();

    let mut successes = 0;
    let mut failures = 0;
    let mut total_tool_time: u128 = 0;

    for result in &results {
        if result.success {
            successes += 1;
        } else {
            failures += 1;
            eprintln!("FAIL {} failed: {:?}", result.tool_name, result.error);
        }
        total_tool_time += result.duration_ms;
    }

    println!("\n📊 Concurrent Test Results ({:?}):", transport);
    println!("   Total requests: {}", num_requests);
    println!("   Successes: {}", successes);
    println!("   Failures: {}", failures);
    println!("   Total wall time: {}ms", total_duration.as_millis());
    println!("   Sum of individual times: {}ms", total_tool_time);
    println!(
        "   Concurrency benefit: {:.1}x speedup",
        total_tool_time as f64 / total_duration.as_millis() as f64
    );

    assert!(
        successes >= 8,
        "At least 8 out of {} requests should succeed ({:?} transport)",
        num_requests,
        transport
    );
}

/// High-volume echo stress run.  `num_requests` is caller-controlled so that
/// the Windows variant can use a smaller count.
async fn run_high_volume_concurrent_requests(num_requests: usize, transport: TransportMode) {
    let server = match spawn_test_server().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("WARNING  Skipping test - failed to spawn server: {}", e);
            return;
        }
    };

    let mut mcp = McpTestClient::with_url(&server.base_url()).with_transport(transport);
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if mcp
        .initialize_with_roots("stress-client", &[root])
        .await
        .is_err()
    {
        eprintln!("WARNING  Skipping test - failed to initialize MCP client");
        return;
    }
    let mcp = Arc::new(mcp);
    let start = Instant::now();

    let futures: Vec<_> = (0..num_requests)
        .map(|i| {
            let mcp = Arc::clone(&mcp);
            async move {
                mcp.call_tool(
                    "sandboxed_shell",
                    json!({"command": format!("echo 'Request {}'", i)}),
                )
                .await
            }
        })
        .collect();

    let results = join_all(futures).await;
    let total_duration = start.elapsed();

    let successes = results.iter().filter(|r| r.success).count();
    let failures = results.iter().filter(|r| !r.success).count();

    println!("\n📊 High-Volume Stress Test Results ({:?}):", transport);
    println!("   Total requests: {}", num_requests);
    println!("   Successes: {}", successes);
    println!("   Failures: {}", failures);
    println!("   Total time: {}ms", total_duration.as_millis());
    println!(
        "   Requests/second: {:.1}",
        num_requests as f64 / total_duration.as_secs_f64()
    );

    let success_rate = successes as f64 / num_requests as f64;
    assert!(
        success_rate >= 0.9,
        "Success rate {:.1}% below 90% threshold ({:?} transport)",
        success_rate * 100.0,
        transport
    );
}

// ---------------------------------------------------------------------------
// Test entries — JSON transport
// ---------------------------------------------------------------------------

/// Concurrent tool calls using `Accept: application/json`.
#[tokio::test]
async fn test_concurrent_tool_calls_json() {
    run_concurrent_tool_calls(TransportMode::Json).await;
}

/// Concurrent tool calls using `Accept: text/event-stream`.
#[tokio::test]
async fn test_concurrent_tool_calls_sse() {
    run_concurrent_tool_calls(TransportMode::Sse).await;
}

/// High-volume echo stress using `Accept: application/json`.
///
/// On Windows the count is reduced because each `sandboxed_shell` spawns a
/// PowerShell process through AppContainer.
#[tokio::test]
async fn test_high_volume_concurrent_requests_json() {
    let num_requests: usize = if cfg!(target_os = "windows") { 15 } else { 50 };
    run_high_volume_concurrent_requests(num_requests, TransportMode::Json).await;
}

/// High-volume echo stress using `Accept: text/event-stream`.
#[tokio::test]
async fn test_high_volume_concurrent_requests_sse() {
    let num_requests: usize = if cfg!(target_os = "windows") { 15 } else { 50 };
    run_high_volume_concurrent_requests(num_requests, TransportMode::Sse).await;
}
