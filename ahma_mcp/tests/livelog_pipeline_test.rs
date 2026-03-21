//! Integration tests for the livelog pipeline.
//!
//! Tests `run_livelog_pipeline()` end-to-end using:
//! - A real mock source command (`echo` / `printf`) that produces known output.
//! - A wiremock server standing in for the LLM endpoint.
//! - A `MockCallbackSender` that captures `ProgressUpdate::LogAlert` notifications.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use ahma_mcp::callback_system::{CallbackError, CallbackSender, ProgressUpdate};
use ahma_mcp::config::{LivelogConfig, LlmProviderConfig};
use ahma_mcp::livelog::run_livelog_pipeline;
use ahma_mcp::sandbox::{Sandbox, SandboxMode};

// ---------------------------------------------------------------------------
// Mock callback sender
// ---------------------------------------------------------------------------

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
// Helper: build a minimal LivelogConfig pointing at a wiremock LLM server
// ---------------------------------------------------------------------------

fn make_config(
    source_command: &str,
    source_args: Vec<String>,
    llm_base_url: &str,
    detection_prompt: &str,
) -> LivelogConfig {
    LivelogConfig {
        source_command: source_command.to_string(),
        source_args,
        detection_prompt: detection_prompt.to_string(),
        llm_provider: LlmProviderConfig {
            base_url: llm_base_url.to_string(),
            model: "test-model".to_string(),
            api_key: None,
        },
        // Use a tiny chunk so the single echo output is flushed quickly.
        chunk_max_lines: 1,
        chunk_max_seconds: 5,
        cooldown_seconds: 0, // no cooldown between alerts in tests
        llm_timeout_seconds: 10,
    }
}

fn make_llm_response(content: &str) -> serde_json::Value {
    json!({
        "choices": [{"message": {"content": content, "role": "assistant"}}]
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// When the LLM returns "CLEAN", no LogAlert should be sent.
#[tokio::test]
async fn test_livelog_pipeline_clean_response_no_alert() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(make_llm_response("CLEAN")))
        .mount(&server)
        .await;

    let temp_dir = tempdir().unwrap();
    let sandbox = Arc::new(
        Sandbox::new(
            vec![temp_dir.path().to_path_buf()],
            SandboxMode::Test,
            false,
            false,
            false,
        )
        .unwrap(),
    );

    let config = make_config(
        "echo",
        vec!["INFO everything is fine".to_string()],
        &server.uri(),
        "look for crashes",
    );

    let callback = MockCallback::new();
    let token = CancellationToken::new();

    run_livelog_pipeline(
        "test-op-clean",
        &config,
        &sandbox,
        temp_dir.path(),
        token,
        Some(&callback),
    )
    .await;

    let alerts = callback.captured_alerts();
    assert!(
        alerts.is_empty(),
        "expected no alerts for CLEAN response, got: {:?}",
        alerts
    );
}

/// When the LLM returns an issue summary, a LogAlert is delivered.
#[tokio::test]
async fn test_livelog_pipeline_issue_detected_sends_alert() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(make_llm_response(
            "NullPointerException at MainActivity line 42",
        )))
        .mount(&server)
        .await;

    let temp_dir = tempdir().unwrap();
    let sandbox = Arc::new(
        Sandbox::new(
            vec![temp_dir.path().to_path_buf()],
            SandboxMode::Test,
            false,
            false,
            false,
        )
        .unwrap(),
    );

    let config = make_config(
        "echo",
        vec!["FATAL EXCEPTION: NullPointerException".to_string()],
        &server.uri(),
        "look for crashes",
    );

    let callback = MockCallback::new();
    let token = CancellationToken::new();

    run_livelog_pipeline(
        "test-op-issue",
        &config,
        &sandbox,
        temp_dir.path(),
        token,
        Some(&callback),
    )
    .await;

    let alerts = callback.captured_alerts();
    assert_eq!(alerts.len(), 1, "expected exactly one alert");

    match &alerts[0] {
        ProgressUpdate::LogAlert {
            id,
            llm_summary,
            trigger_lines,
            ..
        } => {
            assert_eq!(id, "test-op-issue");
            let summary = llm_summary.as_deref().unwrap_or("");
            assert!(
                summary.contains("NullPointerException"),
                "summary should mention the exception: {summary}"
            );
            assert!(trigger_lines.is_some(), "trigger_lines should be populated");
        }
        other => panic!("expected LogAlert, got: {:?}", other),
    }
}

/// The cooldown window suppresses a second alert fired within cooldown_seconds.
#[tokio::test]
async fn test_livelog_pipeline_cooldown_suppresses_second_alert() {
    let server = MockServer::start().await;
    // Always return an "issue detected" response.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(make_llm_response("Error: crash detected")),
        )
        .mount(&server)
        .await;

    let temp_dir = tempdir().unwrap();
    let sandbox = Arc::new(
        Sandbox::new(
            vec![temp_dir.path().to_path_buf()],
            SandboxMode::Test,
            false,
            false,
            false,
        )
        .unwrap(),
    );

    // Two lines = two chunks (chunk_max_lines = 1), but cooldown = 300s.
    // On a real source that only exits after producing two lines we use printf.
    let mut config = make_config(
        "printf",
        vec!["line1\\nline2\\n".to_string()],
        &server.uri(),
        "look for errors",
    );
    config.cooldown_seconds = 300; // very long cooldown

    let callback = MockCallback::new();
    let token = CancellationToken::new();

    run_livelog_pipeline(
        "test-op-cooldown",
        &config,
        &sandbox,
        temp_dir.path(),
        token,
        Some(&callback),
    )
    .await;

    let alerts = callback.captured_alerts();
    // First chunk triggers alert; second chunk should be silenced by cooldown.
    assert_eq!(
        alerts.len(),
        1,
        "cooldown should suppress second alert, got {} alerts",
        alerts.len()
    );
}

/// When the pipeline is cancelled, it terminates promptly.
#[tokio::test]
async fn test_livelog_pipeline_cancellation_stops_pipeline() {
    // Use `sleep` as an "infinite" source that never exits on its own.
    // We cancel immediately after spawning.
    let server = MockServer::start().await;
    // LLM is never reached because we cancel before any chunk is produced.

    let temp_dir = tempdir().unwrap();
    let sandbox = Arc::new(
        Sandbox::new(
            vec![temp_dir.path().to_path_buf()],
            SandboxMode::Test,
            false,
            false,
            false,
        )
        .unwrap(),
    );

    let config = make_config(
        "sleep",
        vec!["60".to_string()],
        &server.uri(),
        "look for crashes",
    );

    let callback = MockCallback::new();
    let token = CancellationToken::new();

    // Cancel after a short delay so the pipeline can actually start.
    let token_clone = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        token_clone.cancel();
    });

    let start = std::time::Instant::now();
    run_livelog_pipeline(
        "test-op-cancel",
        &config,
        &sandbox,
        temp_dir.path(),
        token,
        Some(&callback),
    )
    .await;
    let elapsed = start.elapsed();

    // Should finish well before the 60s sleep timeout.
    assert!(
        elapsed < Duration::from_secs(5),
        "pipeline should stop promptly after cancellation, took {:.2?}",
        elapsed
    );
    assert!(
        callback.captured_alerts().is_empty(),
        "no alerts expected after immediate cancellation"
    );
}

/// LLM returning HTTP 500 should not crash the pipeline — it logs a warning
/// and continues to the next chunk.
#[tokio::test]
async fn test_livelog_pipeline_llm_http_500_graceful() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&server)
        .await;

    let temp_dir = tempdir().unwrap();
    let sandbox = Arc::new(
        Sandbox::new(
            vec![temp_dir.path().to_path_buf()],
            SandboxMode::Test,
            false,
            false,
            false,
        )
        .unwrap(),
    );

    let config = make_config(
        "echo",
        vec!["ERROR something broke".to_string()],
        &server.uri(),
        "look for errors",
    );

    let callback = MockCallback::new();
    let token = CancellationToken::new();

    // Pipeline should complete without panic/hang despite LLM 500.
    run_livelog_pipeline(
        "test-op-500",
        &config,
        &sandbox,
        temp_dir.path(),
        token,
        Some(&callback),
    )
    .await;

    // The LLM error means no alert is generated (the error is logged, not propagated).
    assert!(
        callback.captured_alerts().is_empty(),
        "LLM 500 should not produce an alert"
    );
}

/// LLM returning invalid JSON should be handled gracefully — no crash, no alert.
#[tokio::test]
async fn test_livelog_pipeline_llm_malformed_json_graceful() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not valid json {{{"))
        .mount(&server)
        .await;

    let temp_dir = tempdir().unwrap();
    let sandbox = Arc::new(
        Sandbox::new(
            vec![temp_dir.path().to_path_buf()],
            SandboxMode::Test,
            false,
            false,
            false,
        )
        .unwrap(),
    );

    let config = make_config(
        "echo",
        vec!["WARN something fishy".to_string()],
        &server.uri(),
        "look for warnings",
    );

    let callback = MockCallback::new();
    let token = CancellationToken::new();

    run_livelog_pipeline(
        "test-op-bad-json",
        &config,
        &sandbox,
        temp_dir.path(),
        token,
        Some(&callback),
    )
    .await;

    assert!(
        callback.captured_alerts().is_empty(),
        "malformed LLM JSON should not produce an alert"
    );
}

/// With cooldown=0, every chunk that triggers an LLM issue should fire an alert.
#[tokio::test]
async fn test_livelog_pipeline_zero_cooldown_fires_all_alerts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(make_llm_response("Bug detected!")))
        .expect(2) // two chunks → two LLM calls
        .mount(&server)
        .await;

    let temp_dir = tempdir().unwrap();
    let sandbox = Arc::new(
        Sandbox::new(
            vec![temp_dir.path().to_path_buf()],
            SandboxMode::Test,
            false,
            false,
            false,
        )
        .unwrap(),
    );

    // Two lines with chunk_max_lines=1 → two chunks.
    let mut config = make_config(
        "printf",
        vec!["line1\\nline2\\n".to_string()],
        &server.uri(),
        "look for bugs",
    );
    config.cooldown_seconds = 0; // no suppression

    let callback = MockCallback::new();
    let token = CancellationToken::new();

    run_livelog_pipeline(
        "test-op-zero-cd",
        &config,
        &sandbox,
        temp_dir.path(),
        token,
        Some(&callback),
    )
    .await;

    let alerts = callback.captured_alerts();
    assert_eq!(
        alerts.len(),
        2,
        "both chunks should produce alerts with cooldown=0, got {}",
        alerts.len()
    );
}

/// A source command that does not exist should not crash the pipeline.
#[tokio::test]
async fn test_livelog_pipeline_source_not_found_graceful() {
    let server = MockServer::start().await;
    // LLM mock is set up but should never be reached.

    let temp_dir = tempdir().unwrap();
    let sandbox = Arc::new(
        Sandbox::new(
            vec![temp_dir.path().to_path_buf()],
            SandboxMode::Test,
            false,
            false,
            false,
        )
        .unwrap(),
    );

    let config = make_config(
        "this_command_definitely_does_not_exist_12345",
        vec![],
        &server.uri(),
        "unused",
    );

    let callback = MockCallback::new();
    let token = CancellationToken::new();

    // Should complete without panicking.
    run_livelog_pipeline(
        "test-op-not-found",
        &config,
        &sandbox,
        temp_dir.path(),
        token,
        Some(&callback),
    )
    .await;

    assert!(
        callback.captured_alerts().is_empty(),
        "no alerts expected when source command not found"
    );
}
