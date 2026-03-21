//! Common test helpers for SSE integration tests
//!
//! This module provides shared utilities for testing tools via the HTTP SSE bridge.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::{TestServerInstance, spawn_test_server};

// Thread-local storage for the current test's server URL
std::thread_local! {
    static CURRENT_SERVER_URL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Get the SSE server URL from thread-local storage or environment.
pub fn get_sse_url() -> String {
    if let Ok(url) = env::var("AHMA_TEST_SSE_URL") {
        return url;
    }
    CURRENT_SERVER_URL.with(|url| {
        url.borrow()
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:5721".to_string())
    })
}

/// Ensure the test server is running and return the server instance.
/// The returned value MUST be kept alive for the duration of the test.
/// If AHMA_TEST_SSE_URL is set, returns None and uses that URL.
pub async fn ensure_server_available() -> Option<TestServerInstance> {
    // If user specified a custom URL, just check if it's available
    if let Ok(url) = env::var("AHMA_TEST_SSE_URL") {
        let health_url = format!("{}/health", url);
        let client = Client::builder()
            .http2_prior_knowledge()
            .build()
            .expect("Failed to build HTTP/2 health-check client");
        match client
            .get(&health_url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => return None,
            _ => {
                eprintln!("WARNING️  Custom server at {} not available", url);
                return None;
            }
        }
    }

    // Spawn our own server with dynamic port
    match spawn_test_server().await {
        Ok(server) => {
            CURRENT_SERVER_URL.with(|url| {
                *url.borrow_mut() = Some(server.base_url());
            });
            Some(server)
        }
        Err(e) => {
            eprintln!("WARNING️  Failed to spawn test server: {}", e);
            None
        }
    }
}

/// Check if a specific tool is available on the server
pub async fn is_tool_available(client: &Client, tool_name: &str) -> bool {
    let request = JsonRpcRequest::list_tools();
    match send_request(client, &request).await {
        Ok(response) => response
            .result
            .and_then(|r| r.get("tools").cloned())
            .and_then(|t| t.as_array().cloned())
            .map(|tools| {
                tools.iter().any(|t| {
                    t.get("name")
                        .and_then(|n| n.as_str())
                        .map(|n| n == tool_name)
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Atomic request ID counter for JSON-RPC
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// Get next request ID
fn next_request_id() -> u64 {
    REQUEST_ID.fetch_add(1, Ordering::SeqCst)
}

/// JSON-RPC request structure
#[derive(Debug, Serialize)]
pub struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: Value,
}

impl JsonRpcRequest {
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: next_request_id(),
            method: method.to_string(),
            params,
        }
    }

    pub fn call_tool(name: &str, arguments: Value) -> Self {
        Self::new(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments
            }),
        )
    }

    pub fn list_tools() -> Self {
        Self::new("tools/list", json!({}))
    }
}

/// JSON-RPC response structure
#[derive(Debug, Deserialize)]
pub struct JsonRpcResponse {
    #[allow(dead_code)]
    pub jsonrpc: String,
    #[allow(dead_code)]
    pub id: u64,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub data: Option<Value>,
}

/// Tool call result
#[derive(Debug)]
pub struct ToolCallResult {
    pub tool_name: String,
    pub success: bool,
    pub duration_ms: u128,
    pub error: Option<String>,
    pub output: Option<String>,
}

/// Send a JSON-RPC request to the MCP endpoint
pub async fn send_request(
    client: &Client,
    request: &JsonRpcRequest,
) -> Result<JsonRpcResponse, String> {
    let url = format!("{}/mcp", get_sse_url());

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(request)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", status, text));
    }

    response
        .json::<JsonRpcResponse>()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))
}

/// Call a tool and return the result
pub async fn call_tool(client: &Client, name: &str, arguments: Value) -> ToolCallResult {
    let start = Instant::now();
    let request = JsonRpcRequest::call_tool(name, arguments);

    match send_request(client, &request).await {
        Ok(response) => {
            let duration_ms = start.elapsed().as_millis();

            if let Some(error) = response.error {
                ToolCallResult {
                    tool_name: name.to_string(),
                    success: false,
                    duration_ms,
                    error: Some(format!("[{}] {}", error.code, error.message)),
                    output: None,
                }
            } else {
                let output = response.result.map(|v| {
                    if let Some(content) = v.get("content") {
                        if let Some(arr) = content.as_array() {
                            arr.iter()
                                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n")
                        } else {
                            serde_json::to_string_pretty(&v).unwrap_or_default()
                        }
                    } else {
                        serde_json::to_string_pretty(&v).unwrap_or_default()
                    }
                });

                ToolCallResult {
                    tool_name: name.to_string(),
                    success: true,
                    duration_ms,
                    error: None,
                    output,
                }
            }
        }
        Err(e) => ToolCallResult {
            tool_name: name.to_string(),
            success: false,
            duration_ms: start.elapsed().as_millis(),
            error: Some(e),
            output: None,
        },
    }
}

/// Helper macro to skip test if tool is not available
#[macro_export]
macro_rules! skip_if_unavailable {
    ($client:expr, $tool_name:expr) => {
        if !$crate::sse_test_helpers::is_tool_available($client, $tool_name).await {
            eprintln!(
                "WARNING️  {} not available on server, skipping test",
                $tool_name
            );
            return;
        }
    };
}
