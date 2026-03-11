//! Platform-aware timeout utilities.
//!
//! Windows CI runners are significantly slower than Linux/macOS, particularly for:
//! - Process spawning and stdio communication
//! - File system operations
//! - Network operations
//!
//! This module provides a centralized way to scale timeouts based on platform
//! and environment (e.g., coverage mode), avoiding scattered platform-specific
//! timeout logic throughout the codebase.
//!
//! # Usage
//!
//! ```rust
//! use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};
//! use std::time::Duration;
//!
//! // Get a timeout for a specific category
//! let timeout = TestTimeouts::get(TimeoutCategory::ProcessSpawn);
//!
//! // Scale a custom duration
//! let custom = TestTimeouts::scale(Duration::from_secs(5));
//!
//! // Get the platform multiplier directly
//! let multiplier = TestTimeouts::multiplier();
//! ```
//!
//! # Design Rationale
//!
//! Rather than hardcoding platform checks throughout tests, this module:
//! 1. Centralizes timeout logic for consistency
//! 2. Provides semantic categories (spawn, handshake, tool call) with sensible defaults
//! 3. Allows environment-based overrides for debugging
//! 4. Accounts for coverage mode which adds additional overhead

use std::time::Duration;

/// Timeout categories with platform-aware defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutCategory {
    /// Process spawn and initial startup (binary loading, shell pool init)
    ProcessSpawn,
    /// MCP handshake completion (initialize, roots/list exchange)
    Handshake,
    /// Individual tool call execution
    ToolCall,
    /// Waiting for sandbox readiness after roots exchange
    SandboxReady,
    /// HTTP request/response cycle
    HttpRequest,
    /// SSE stream operations
    SseStream,
    /// Health check polling
    HealthCheck,
    /// Test cleanup operations
    Cleanup,
    /// Short operations (sub-second on fast platforms)
    Quick,
}

/// Platform-aware timeout configuration.
pub struct TestTimeouts;

impl TestTimeouts {
    /// Get the platform multiplier.
    ///
    /// - Windows: 4x base timeout (CI runners are significantly slower)
    /// - Coverage mode: Additional 2x on top of platform multiplier
    /// - Default: 1x
    pub fn multiplier() -> u64 {
        let base = if cfg!(windows) {
            4
        } else {
            1
        };

        let coverage_multiplier = if is_coverage_mode() { 2 } else { 1 };

        base * coverage_multiplier
    }

    /// Get the timeout for a specific category.
    pub fn get(category: TimeoutCategory) -> Duration {
        let base_secs = match category {
            TimeoutCategory::ProcessSpawn => 30,
            TimeoutCategory::Handshake => 60,
            TimeoutCategory::ToolCall => 30,
            TimeoutCategory::SandboxReady => 60,
            TimeoutCategory::HttpRequest => 30,
            TimeoutCategory::SseStream => 120,
            TimeoutCategory::HealthCheck => 15,
            TimeoutCategory::Cleanup => 10,
            TimeoutCategory::Quick => 5,
        };

        Duration::from_secs(base_secs * Self::multiplier())
    }

    /// Scale a custom duration by the platform multiplier.
    pub fn scale(duration: Duration) -> Duration {
        duration * Self::multiplier() as u32
    }

    /// Scale milliseconds by the platform multiplier.
    pub fn scale_millis(millis: u64) -> Duration {
        Duration::from_millis(millis * Self::multiplier())
    }

    /// Scale seconds by the platform multiplier.
    pub fn scale_secs(secs: u64) -> Duration {
        Duration::from_secs(secs * Self::multiplier())
    }

    /// Get an iterator delay (for polling loops) that's appropriate for the platform.
    /// On Windows, we use longer delays to avoid overwhelming slow CI.
    pub fn poll_interval() -> Duration {
        if cfg!(windows) {
            Duration::from_millis(500)
        } else {
            Duration::from_millis(100)
        }
    }

    /// Get a short delay for inter-operation pauses.
    /// Useful after SSE exchanges before polling, etc.
    pub fn short_delay() -> Duration {
        if cfg!(windows) {
            Duration::from_secs(3)
        } else {
            Duration::from_millis(100)
        }
    }
}

/// Check if running in coverage mode (adds significant overhead).
fn is_coverage_mode() -> bool {
    std::env::var_os("LLVM_PROFILE_FILE").is_some() || std::env::var_os("CARGO_LLVM_COV").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multiplier_is_at_least_one() {
        assert!(TestTimeouts::multiplier() >= 1);
    }

    #[test]
    fn test_scale_preserves_zero() {
        assert_eq!(TestTimeouts::scale(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn test_categories_return_positive_durations() {
        let categories = [
            TimeoutCategory::ProcessSpawn,
            TimeoutCategory::Handshake,
            TimeoutCategory::ToolCall,
            TimeoutCategory::SandboxReady,
            TimeoutCategory::HttpRequest,
            TimeoutCategory::SseStream,
            TimeoutCategory::HealthCheck,
            TimeoutCategory::Cleanup,
            TimeoutCategory::Quick,
        ];

        for cat in categories {
            let timeout = TestTimeouts::get(cat);
            assert!(timeout.as_secs() > 0, "{:?} should have positive timeout", cat);
        }
    }

    #[test]
    fn test_scale_secs_matches_scale() {
        let secs = 10;
        assert_eq!(
            TestTimeouts::scale_secs(secs),
            TestTimeouts::scale(Duration::from_secs(secs))
        );
    }

    #[test]
    fn test_poll_interval_is_reasonable() {
        let interval = TestTimeouts::poll_interval();
        assert!(interval.as_millis() >= 100);
        assert!(interval.as_millis() <= 1000);
    }
}
