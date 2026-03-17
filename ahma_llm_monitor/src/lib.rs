//! # Ahma LLM Monitor
//!
//! OpenAI-compatible LLM client for log analysis and issue detection.
//! Used by the live log monitoring pipeline to detect issues described
//! in plain English via a detection prompt.

pub mod client;
pub mod error;
pub mod prompt;

pub use client::LlmClient;
pub use error::LlmMonitorError;
