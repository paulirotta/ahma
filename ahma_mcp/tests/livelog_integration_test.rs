//! End-to-end integration tests for the livelog pipeline (R9.5).
//!
//! These tests exercise the full pipeline at two layers:
//!
//! **Layer 1 — Handler integration** (`handle_livelog_start` → `run_livelog_pipeline`):
//! Calls the handler directly with a mock callback and a wiremock LLM endpoint.  Verifies
//! that the operation lifecycle (registered → in-progress → completed) and the
//! `ProgressUpdate::LogAlert` notification delivery work end-to-end.
//!
//! **Layer 2 — MCP dispatch** (`tools/call` → handler):
//! Uses an in-process MCP pair to verify that the MCP service routes livelog tool calls
//! through the correct dispatch path and returns the expected "monitoring started" response.
//!
//! All tests use:
//! - `echo` / `printf` as a short-lived source command (no long-running processes).
//! - A `wiremock` `MockServer` standing in for the LLM endpoint.
//! - `tempfile::tempdir()` for all filesystem state (no repository pollution).

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use tempfile::tempdir;
use tokio::fs;
use tokio::time::sleep;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use ahma_mcp::callback_system::{CallbackError, CallbackSender, ProgressUpdate};
use ahma_mcp::config::{LivelogConfig, LlmProviderConfig, ToolConfig, ToolType};
use ahma_mcp::mcp_service::handlers::livelog_tool::handle_livelog_start;
use ahma_mcp::operation_monitor::{MonitorConfig, OperationMonitor, OperationStatus};
use ahma_mcp::sandbox::{Sandbox, SandboxMode};
use ahma_mcp::test_utils::concurrency::{
    CI_DEFAULT_TIMEOUT, CI_QUICK_TIMEOUT, wait_for_operation_terminal,
};
use ahma_mcp::test_utils::in_process::create_in_process_mcp_from_dir;
use ahma_mcp::utils::logging::init_test_logging;
use rmcp::model::CallToolRequestParams;

// ---------------------------------------------------------------------------
// Shared mock callback sender
// ---------------------------------------------------------------------------

/// Captures all `ProgressUpdate::LogAlert` notifications emitted by the pipeline.
#[derive(Clone, Default)]
struct MockCallback {
    alerts: Arc<Mutex<Vec<ProgressUpdate>>>,
}

impl MockCallback {
    fn new() -> Self {
        Self::default()
    }

    fn captured_alerts(&self) -> Vec<ProgressUpdate> {
        self.alerts.lock().unwrap().clone()
    }
}

#[async_trait]
impl CallbackSender for MockCallback {
    async fn send_progress(&self, update: ProgressUpdate) -> Result<(), CallbackError> {
        if matches!(update, ProgressUpdate::LogAlert { .. }) {
            self.alerts.lock().unwrap().push(update);
        }
        Ok(())
    }

    async fn should_cancel(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_llm_response(content: &str) -> serde_json::Value {
    json!({
        "choices": [{"message": {"content": content, "role": "assistant"}}]
    })
}

/// Build a minimal `LivelogConfig` pointing at `llm_base_url`.
/// `chunk_max_lines=1` ensures each line is flushed as its own chunk immediately.
fn make_livelog_config(
    source_command: &str,
    source_args: Vec<String>,
    llm_base_url: &str,
) -> LivelogConfig {
    LivelogConfig {
        source_command: source_command.to_string(),
        source_args,
        detection_prompt: "look for crashes and errors".to_string(),
        llm_provider: LlmProviderConfig {
            base_url: llm_base_url.to_string(),
            model: "test-model".to_string(),
            api_key: None,
        },
        chunk_max_lines: 1,
        chunk_max_seconds: 5,
        cooldown_seconds: 0, // disabled so each chunk can trigger independently
        llm_timeout_seconds: 10,
    }
}

/// Wrap a `LivelogConfig` in a `ToolConfig` suitable for `handle_livelog_start`.
fn make_tool_config(name: &str, livelog: LivelogConfig) -> ToolConfig {
    ToolConfig {
        name: name.to_string(),
        description: "test livelog tool".to_string(),
        command: livelog.source_command.clone(),
        tool_type: Some(ToolType::Livelog),
        livelog: Some(livelog),
        ..ToolConfig::default()
    }
}

/// Create a `Sandbox` in test mode (bypasses path validation) scoped to `scope`.
fn test_sandbox(scope: &std::path::Path) -> Arc<Sandbox> {
    Arc::new(
        Sandbox::new(
            vec![scope.to_path_buf()],
            SandboxMode::Test,
            false,
            false,
            false,
        )
        .unwrap(),
    )
}

/// Create an `OperationMonitor` suitable for tests (30 s default timeout).
fn test_monitor() -> Arc<OperationMonitor> {
    Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
        Duration::from_secs(30),
    )))
}

const POLL_INTERVAL: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------
// Layer 1 — Handler integration tests
// ---------------------------------------------------------------------------

/// Full pipeline: echo produces one line, LLM returns an issue summary → one LogAlert.
#[tokio::test]
async fn test_livelog_handler_issue_detected_sends_alert() {
    init_test_logging();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(make_llm_response("NullPointerException at line 42")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let temp_dir = tempdir().unwrap();
    let sandbox = test_sandbox(temp_dir.path());
    let monitor = test_monitor();

    let config = make_tool_config(
        "test-livelog",
        make_livelog_config(
            "echo",
            vec!["FATAL EXCEPTION: NullPointerException".to_string()],
            &server.uri(),
        ),
    );

    let callback = MockCallback::new();
    let op_id = handle_livelog_start(
        "test-op-issue".to_string(),
        &config,
        &serde_json::Map::new(),
        monitor.clone(),
        sandbox,
        Some(Box::new(callback.clone())),
    )
    .await
    .expect("handle_livelog_start should succeed");

    // Wait for the background pipeline to complete.
    let completed =
        wait_for_operation_terminal(&monitor, &op_id, CI_DEFAULT_TIMEOUT, POLL_INTERVAL).await;
    assert!(
        completed,
        "operation '{op_id}' should reach a terminal state within the timeout"
    );

    // Verify the operation reached Completed (not Failed/Cancelled).
    let op = monitor.get_operation(&op_id).await;
    if let Some(op) = op {
        assert_eq!(
            op.state,
            OperationStatus::Completed,
            "operation should be Completed, was {:?}",
            op.state
        );
    }

    // Verify exactly one alert was delivered.
    let alerts = callback.captured_alerts();
    assert_eq!(
        alerts.len(),
        1,
        "expected exactly one LogAlert, got: {alerts:?}"
    );

    match &alerts[0] {
        ProgressUpdate::LogAlert {
            id,
            llm_summary,
            trigger_lines,
            ..
        } => {
            assert_eq!(id, "test-op-issue", "alert id should match operation id");
            let summary = llm_summary.as_deref().unwrap_or("");
            assert!(
                summary.contains("NullPointerException"),
                "summary should mention the exception, got: {summary}"
            );
            assert!(trigger_lines.is_some(), "trigger_lines should be populated");
        }
        other => panic!("expected LogAlert, got: {other:?}"),
    }
}

/// Full pipeline: echo produces one line, LLM returns "CLEAN" → no LogAlert.
#[tokio::test]
async fn test_livelog_handler_clean_response_no_alert() {
    init_test_logging();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(make_llm_response("CLEAN")))
        .mount(&server)
        .await;

    let temp_dir = tempdir().unwrap();
    let sandbox = test_sandbox(temp_dir.path());
    let monitor = test_monitor();

    let config = make_tool_config(
        "test-livelog",
        make_livelog_config(
            "echo",
            vec!["INFO everything is fine".to_string()],
            &server.uri(),
        ),
    );

    let callback = MockCallback::new();
    let op_id = handle_livelog_start(
        "test-op-clean".to_string(),
        &config,
        &serde_json::Map::new(),
        monitor.clone(),
        sandbox,
        Some(Box::new(callback.clone())),
    )
    .await
    .expect("handle_livelog_start should succeed");

    let completed =
        wait_for_operation_terminal(&monitor, &op_id, CI_DEFAULT_TIMEOUT, POLL_INTERVAL).await;
    assert!(completed, "operation should complete");

    let alerts = callback.captured_alerts();
    assert!(
        alerts.is_empty(),
        "expected no alerts for CLEAN response, got: {alerts:?}"
    );
}

/// Pipeline continues after first alert: three chunks, no cooldown → three alerts.
///
/// This validates that the pipeline keeps running and sending notifications after
/// the first alert is delivered — a subtle regression risk.
#[cfg(unix)]
#[tokio::test]
async fn test_livelog_handler_multiple_alerts_pipeline_continues() {
    init_test_logging();

    let server = MockServer::start().await;
    // Always return an issue summary.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(make_llm_response("Error: crash detected")),
        )
        .expect(3)
        .mount(&server)
        .await;

    let temp_dir = tempdir().unwrap();
    let sandbox = test_sandbox(temp_dir.path());
    let monitor = test_monitor();

    // printf emits three lines; chunk_max_lines=1 → three chunks → three LLM calls.
    let config = make_tool_config(
        "test-livelog",
        make_livelog_config(
            "printf",
            vec!["err1\\nerr2\\nerr3\\n".to_string()],
            &server.uri(),
        ),
    );

    let callback = MockCallback::new();
    let op_id = handle_livelog_start(
        "test-op-multi".to_string(),
        &config,
        &serde_json::Map::new(),
        monitor.clone(),
        sandbox,
        Some(Box::new(callback.clone())),
    )
    .await
    .expect("handle_livelog_start should succeed");

    let completed =
        wait_for_operation_terminal(&monitor, &op_id, CI_DEFAULT_TIMEOUT, POLL_INTERVAL).await;
    assert!(completed, "operation should complete");

    let alerts = callback.captured_alerts();
    assert_eq!(
        alerts.len(),
        3,
        "expected three alerts (one per chunk), got {}: {alerts:?}",
        alerts.len()
    );
}

/// Cooldown suppresses the second alert when `cooldown_seconds` is large.
///
/// Two chunks are produced (two lines, chunk_max_lines=1) but only the first
/// should trigger an alert; the second is silenced by the cooldown window.
#[cfg(unix)]
#[tokio::test]
async fn test_livelog_handler_cooldown_suppresses_second_alert() {
    init_test_logging();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(make_llm_response("Error: crash detected")),
        )
        .mount(&server)
        .await;

    let temp_dir = tempdir().unwrap();
    let sandbox = test_sandbox(temp_dir.path());
    let monitor = test_monitor();

    let mut livelog = make_livelog_config(
        "printf",
        vec!["line1\\nline2\\n".to_string()],
        &server.uri(),
    );
    livelog.cooldown_seconds = 300; // very long cooldown

    let config = make_tool_config("test-livelog", livelog);

    let callback = MockCallback::new();
    let op_id = handle_livelog_start(
        "test-op-cooldown".to_string(),
        &config,
        &serde_json::Map::new(),
        monitor.clone(),
        sandbox,
        Some(Box::new(callback.clone())),
    )
    .await
    .expect("handle_livelog_start should succeed");

    let completed =
        wait_for_operation_terminal(&monitor, &op_id, CI_DEFAULT_TIMEOUT, POLL_INTERVAL).await;
    assert!(completed, "operation should complete");

    let alerts = callback.captured_alerts();
    assert_eq!(
        alerts.len(),
        1,
        "cooldown should suppress second alert; got {} alerts: {alerts:?}",
        alerts.len()
    );
}

/// Cancellation via `OperationMonitor` stops the pipeline promptly.
///
/// A long-running source command (`sleep 60`) is used to simulate a stream that
/// never exits on its own.  The test cancels through the monitor's token and
/// verifies the pipeline terminates well before the sleep timeout.
#[tokio::test]
async fn test_livelog_handler_cancel_via_monitor_stops_pipeline() {
    init_test_logging();

    let server = MockServer::start().await;
    // LLM should never be reached because we cancel before any chunk is produced.

    let temp_dir = tempdir().unwrap();
    let sandbox = test_sandbox(temp_dir.path());
    let monitor = test_monitor();

    let config = make_tool_config(
        "test-livelog",
        make_livelog_config("sleep", vec!["60".to_string()], &server.uri()),
    );

    let callback = MockCallback::new();
    let op_id = handle_livelog_start(
        "test-op-cancel".to_string(),
        &config,
        &serde_json::Map::new(),
        monitor.clone(),
        sandbox,
        Some(Box::new(callback.clone())),
    )
    .await
    .expect("handle_livelog_start should succeed");

    // Give the background task a moment to start, then cancel through the monitor.
    sleep(Duration::from_millis(50)).await;
    if let Some(op) = monitor.get_operation(&op_id).await {
        op.cancellation_token.cancel();
    }

    let start = std::time::Instant::now();
    let completed =
        wait_for_operation_terminal(&monitor, &op_id, CI_QUICK_TIMEOUT, POLL_INTERVAL).await;
    let elapsed = start.elapsed();

    assert!(
        completed,
        "operation should reach a terminal state after cancellation"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "pipeline should stop promptly after cancellation, took {elapsed:.2?}"
    );
    assert!(
        callback.captured_alerts().is_empty(),
        "no alerts expected after cancellation"
    );
}

/// LLM returning HTTP 500 is handled gracefully: pipeline completes, no panic.
#[tokio::test]
async fn test_livelog_handler_llm_http_error_graceful() {
    init_test_logging();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&server)
        .await;

    let temp_dir = tempdir().unwrap();
    let sandbox = test_sandbox(temp_dir.path());
    let monitor = test_monitor();

    let config = make_tool_config(
        "test-livelog",
        make_livelog_config("echo", vec!["some log output".to_string()], &server.uri()),
    );

    let callback = MockCallback::new();
    let op_id = handle_livelog_start(
        "test-op-llm-err".to_string(),
        &config,
        &serde_json::Map::new(),
        monitor.clone(),
        sandbox,
        Some(Box::new(callback.clone())),
    )
    .await
    .expect("handle_livelog_start should succeed");

    // Pipeline should still complete (LLM error is non-fatal).
    let completed =
        wait_for_operation_terminal(&monitor, &op_id, CI_DEFAULT_TIMEOUT, POLL_INTERVAL).await;
    assert!(
        completed,
        "pipeline should complete even when LLM returns HTTP 500"
    );
    assert!(
        callback.captured_alerts().is_empty(),
        "no alerts expected when LLM errors"
    );
}

/// Missing `livelog` config block returns an error — the operation is never registered.
#[tokio::test]
async fn test_livelog_handler_missing_livelog_block_returns_error() {
    init_test_logging();

    let temp_dir = tempdir().unwrap();
    let sandbox = test_sandbox(temp_dir.path());
    let monitor = test_monitor();

    // A ToolConfig with `tool_type: Livelog` but no `livelog` block.
    let config = ToolConfig {
        name: "bad-tool".to_string(),
        description: "missing livelog config".to_string(),
        command: "echo".to_string(),
        tool_type: Some(ToolType::Livelog),
        livelog: None,
        ..ToolConfig::default()
    };

    let result = handle_livelog_start(
        "test-op-bad".to_string(),
        &config,
        &serde_json::Map::new(),
        monitor.clone(),
        sandbox,
        None,
    )
    .await;

    assert!(
        result.is_err(),
        "should return an error when livelog block is missing"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("livelog"),
        "error message should mention 'livelog', got: {err_msg}"
    );
}

// ---------------------------------------------------------------------------
// Layer 2 — MCP dispatch tests
// ---------------------------------------------------------------------------

/// `tools/call` on a livelog tool returns "Live log monitoring started" immediately.
///
/// This verifies the MCP service dispatch path (mcp_service/mod.rs ~line 728) is
/// wired correctly: `ToolType::Livelog` → `handle_livelog_start` → operation ID
/// returned as tool content.
#[tokio::test]
async fn test_livelog_mcp_dispatch_returns_operation_id() -> Result<()> {
    init_test_logging();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(make_llm_response("CLEAN")))
        .mount(&server)
        .await;

    // Write a livelog tool config to a temp .ahma/ dir.
    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    let tool_config = json!({
        "name": "test-livelog-mcp",
        "description": "test livelog tool for MCP dispatch verification",
        "command": "echo",
        "enabled": true,
        "tool_type": "livelog",
        "livelog": {
            "source_command": "echo",
            "source_args": ["hello from livelog"],
            "detection_prompt": "look for errors",
            "llm_provider": {
                "base_url": server.uri(),
                "model": "test-model"
            },
            "chunk_max_lines": 1,
            "chunk_max_seconds": 5,
            "cooldown_seconds": 0,
            "llm_timeout_seconds": 10
        }
    });
    fs::write(
        tools_dir.join("test-livelog-mcp.json"),
        serde_json::to_string(&tool_config)?,
    )
    .await?;

    // Wire up an in-process MCP pair.
    let mcp = create_in_process_mcp_from_dir(&tools_dir)
        .await
        .map_err(|e| {
            // Non-fatal: log and skip if the service fails to start (e.g. missing tools).
            eprintln!(
                "WARNING  test_livelog_mcp_dispatch_returns_operation_id: MCP init failed: {e}"
            );
            e
        })?;

    // Call the livelog tool via MCP tools/call.
    let params = CallToolRequestParams::new(Cow::Borrowed("test-livelog-mcp"))
        .with_arguments(json!({}).as_object().unwrap().clone());

    let result = mcp.client.call_tool(params).await.map_err(|e| {
        eprintln!("WARNING  test_livelog_mcp_dispatch_returns_operation_id: call_tool failed: {e}");
        e
    })?;

    // The tool call should succeed (not an MCP error).
    assert!(
        !result.is_error.unwrap_or(false),
        "livelog tool call should succeed, got error: {result:?}"
    );

    // Response text should confirm monitoring started and include an operation ID.
    let response_text = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        response_text.contains("Live log monitoring started"),
        "response should confirm monitoring started, got: {response_text}"
    );
    assert!(
        response_text.contains("Operation ID"),
        "response should include an operation ID, got: {response_text}"
    );

    Ok(())
}

/// `tools/call` with a livelog tool that has no `livelog` config block returns an MCP error.
#[tokio::test]
async fn test_livelog_mcp_dispatch_missing_livelog_config_returns_error() -> Result<()> {
    init_test_logging();

    // Write a tool config with tool_type=livelog but no livelog block.
    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join(".ahma");
    fs::create_dir_all(&tools_dir).await?;

    let tool_config = json!({
        "name": "bad-livelog",
        "description": "livelog tool with missing livelog block",
        "command": "echo",
        "enabled": true,
        "tool_type": "livelog"
        // No "livelog" key
    });
    fs::write(
        tools_dir.join("bad-livelog.json"),
        serde_json::to_string(&tool_config)?,
    )
    .await?;

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.map_err(|e| {
        eprintln!("WARNING  test_livelog_mcp_dispatch_missing_livelog_config_returns_error: MCP init failed: {e}");
        e
    })?;

    let params = CallToolRequestParams::new(Cow::Borrowed("bad-livelog"))
        .with_arguments(json!({}).as_object().unwrap().clone());

    // The server returns an MCP protocol error (not a CallToolResult with is_error=true)
    // because the config is invalid before any operation is started.
    let err = mcp
        .client
        .call_tool(params)
        .await
        .expect_err("expected an MCP error for missing livelog config");

    let err_msg = err.to_string();
    assert!(
        err_msg.contains("livelog"),
        "error should mention 'livelog', got: {err_msg}"
    );

    Ok(())
}
