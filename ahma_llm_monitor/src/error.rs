use thiserror::Error;

/// Errors that can occur during LLM monitor operations.
#[derive(Debug, Error)]
pub enum LlmMonitorError {
    /// The HTTP request to the LLM API failed.
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The LLM API returned a response that couldn't be parsed.
    #[error("LLM returned unexpected response: {0}")]
    Parse(String),

    /// The LLM request exceeded the configured timeout.
    #[error("LLM request timed out")]
    Timeout,
}
