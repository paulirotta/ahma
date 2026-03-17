use std::time::Duration;

use reqwest::Client;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::error::LlmMonitorError;
use crate::prompt::build_messages;

/// An OpenAI-compatible LLM client for issue detection in log chunks.
#[derive(Debug, Clone)]
pub struct LlmClient {
    http: Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

impl LlmClient {
    /// Create a new LLM client.
    ///
    /// * `base_url` — Base URL of the OpenAI-compatible API (e.g. `http://localhost:11434/v1`)
    /// * `model` — Model name (e.g. `llama3.2`, `gpt-4o-mini`)
    /// * `api_key` — Optional bearer token; pass `None` for local models (Ollama etc.)
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            api_key,
        }
    }

    /// Analyse a chunk of log lines against the detection prompt.
    ///
    /// Returns `Ok(Some(summary))` if the LLM detected an issue, or `Ok(None)` if clean.
    /// The `timeout` controls the maximum time to wait for the API response.
    pub async fn detect_issues(
        &self,
        detection_prompt: &str,
        chunk: &str,
        timeout: Duration,
    ) -> Result<Option<String>, LlmMonitorError> {
        let messages = build_messages(detection_prompt, chunk);

        let body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": 256,
            "temperature": 0.0,
        });

        debug!(
            "Sending chunk ({} chars) to LLM at {} for analysis",
            chunk.len(),
            self.base_url
        );

        let mut request = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body)
            .timeout(timeout);

        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }

        let response = tokio::time::timeout(timeout, request.send())
            .await
            .map_err(|_| LlmMonitorError::Timeout)?
            .map_err(LlmMonitorError::Http)?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            warn!("LLM API error {}: {}", status, body_text);
            return Err(LlmMonitorError::Parse(format!(
                "HTTP {status}: {body_text}"
            )));
        }

        let json: Value = response.json().await.map_err(LlmMonitorError::Http)?;

        let text = json
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| LlmMonitorError::Parse("missing choices[0].message.content".into()))?
            .trim()
            .to_string();

        // The LLM is instructed to respond with "CLEAN" when no issues are found.
        if text.eq_ignore_ascii_case("clean") || text.to_ascii_uppercase().starts_with("CLEAN") {
            debug!("LLM response: CLEAN");
            Ok(None)
        } else {
            debug!("LLM detected issue: {}", text);
            Ok(Some(text))
        }
    }
}
