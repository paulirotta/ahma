use std::time::Duration;

use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use ahma_llm_monitor::LlmClient;

fn make_response(content: &str) -> serde_json::Value {
    json!({
        "choices": [{"message": {"content": content, "role": "assistant"}}]
    })
}

#[tokio::test]
async fn test_detect_issues_clean_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(make_response("CLEAN")))
        .mount(&server)
        .await;

    let client = LlmClient::new(server.uri(), "test-model", None);
    let result = client
        .detect_issues(
            "look for crashes",
            "INFO app started",
            Duration::from_secs(5),
        )
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_none(), "expected None (clean)");
}

#[tokio::test]
async fn test_detect_issues_issue_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(make_response(
            "NullPointerException in MainActivity line 42",
        )))
        .mount(&server)
        .await;

    let client = LlmClient::new(server.uri(), "test-model", None);
    let result = client
        .detect_issues(
            "look for crashes or exceptions",
            "FATAL Exception: NullPointerException",
            Duration::from_secs(5),
        )
        .await;

    assert!(result.is_ok());
    let summary = result.unwrap();
    assert!(summary.is_some(), "expected Some(summary)");
    assert!(summary.unwrap().contains("NullPointerException"));
}

#[tokio::test]
async fn test_detect_issues_clean_case_insensitive() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(make_response("clean")))
        .mount(&server)
        .await;

    let client = LlmClient::new(server.uri(), "test-model", None);
    let result = client
        .detect_issues(
            "look for errors",
            "DEBUG heartbeat ok",
            Duration::from_secs(5),
        )
        .await;

    assert!(result.is_ok());
    assert!(
        result.unwrap().is_none(),
        "expected None for lowercase 'clean'"
    );
}

#[tokio::test]
async fn test_detect_issues_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let client = LlmClient::new(server.uri(), "test-model", None);
    let result = client
        .detect_issues("look for errors", "some log", Duration::from_secs(5))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_detect_issues_sends_bearer_auth() {
    use wiremock::matchers::header;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer my-secret-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(make_response("CLEAN")))
        .mount(&server)
        .await;

    let client = LlmClient::new(server.uri(), "test-model", Some("my-secret-key".into()));
    let result = client
        .detect_issues("look for errors", "INFO ok", Duration::from_secs(5))
        .await;

    assert!(result.is_ok());
}
