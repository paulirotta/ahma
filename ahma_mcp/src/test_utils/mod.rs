//! Test helper utilities for Ahma MCP.
//!
//! This module provides reusable helpers for integration and unit tests,
//! including sandbox setup, temporary project scaffolding, and MCP client
//! conveniences. These APIs are intended for test-only code paths.

pub mod assertions;
pub mod cli;
pub mod client;
pub mod concurrency;
pub mod config;
pub mod fs;
pub mod http;
pub mod path_helpers;
pub mod project;
pub mod stdio;

// Helper function to check if a tool is disabled (used by macros)
pub fn is_tool_disabled(tool_name: &str) -> bool {
    // Check environment variable first (e.g., AHMA_DISABLE_TOOL_GH=true)
    let env_var = format!("AHMA_DISABLE_TOOL_{}", tool_name.to_uppercase());
    if std::env::var(&env_var)
        .map(|v| v.to_lowercase() == "true" || v == "1")
        .unwrap_or(false)
    {
        return true;
    }

    let workspace_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get workspace root")
        .to_path_buf();

    // Paths to check for tool configuration
    let config_paths = [
        workspace_dir
            .join(".ahma")
            .join(format!("{}.json", tool_name)),
        workspace_dir
            .join(".ahma")
            .join(format!("{}.json", tool_name)),
    ];

    for config_path in config_paths {
        if config_path.exists()
            && let Ok(content) = std::fs::read_to_string(&config_path)
        {
            // Simple check for "enabled": false
            if content.contains(r#""enabled": false"#) || content.contains(r#""enabled":false"#) {
                return true;
            }
        }
    }

    false
}

/// Initialize verbose logging for tests.
pub fn init_test_logging() {
    let _ = crate::utils::logging::init_logging("trace", false);
}

/// Initialize the test sandbox environment (logging, etc).
pub fn init_test_sandbox() {
    init_test_logging();
}

/// Strip ANSI escape sequences
#[allow(dead_code)]
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // ESC
            if let Some('[') = chars.peek() {
                // consume '['
                chars.next();
                // consume until a terminator in @A–Z[\]^_`a–z{|}~ (0x40..=0x7E)
                while let Some(&nc) = chars.peek() {
                    let code = nc as u32;
                    if (0x40..=0x7E).contains(&code) {
                        // end of CSI sequence
                        chars.next();
                        break;
                    } else {
                        chars.next();
                    }
                }
                continue; // skip entire escape sequence
            }
            // If it's ESC but not CSI, skip just ESC
            continue;
        }
        out.push(c);
    }
    out
}

// Macros need to be exported at crate level usually.
// Since we are in `test_utils`, users will use `ahma_mcp::skip_if_disabled!`.

/// Macro to skip a synchronous test if a tool is disabled.
#[macro_export]
macro_rules! skip_if_disabled {
    ($tool_name:expr) => {
        if $crate::test_utils::is_tool_disabled($tool_name) {
            eprintln!(
                "WARNING️  Skipping test - {} is disabled in config",
                $tool_name
            );
            return;
        }
    };
}

/// Macro to skip an async test that returns Result if a tool is disabled.
#[macro_export]
macro_rules! skip_if_disabled_async_result {
    ($tool_name:expr) => {
        if $crate::test_utils::is_tool_disabled($tool_name) {
            eprintln!(
                "WARNING️  Skipping test - {} is disabled in config",
                $tool_name
            );
            return Ok(());
        }
    };
}

/// Macro to skip an async test (no return value) if a tool is disabled.
#[macro_export]
macro_rules! skip_if_disabled_async {
    ($tool_name:expr) => {
        if $crate::test_utils::is_tool_disabled($tool_name) {
            eprintln!(
                "WARNING️  Skipping test - {} is disabled in config",
                $tool_name
            );
            return;
        }
    };
}
