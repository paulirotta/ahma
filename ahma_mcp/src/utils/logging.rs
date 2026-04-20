//! # Logging Initialization
//!
//! This module provides a centralized function for initializing the application's
//! logging infrastructure. It uses the `tracing` ecosystem to provide structured,
//! configurable logging.
//!
//! ## Core Functionality
//!
//! - **`init_logging()`**: This is the main function of the module. It is designed to
//!   be called once at the start of the application's lifecycle. It uses a `std::sync::Once`
//!   to ensure that the initialization logic is executed only a single time, even if
//!   the function is called multiple times.
//!
//! ## Logging Configuration
//!
//! The function sets up a multi-layered logging system:
//!
//! 1.  **Environment Filter (`EnvFilter`)**: It configures the logging verbosity based on
//!     the `RUST_LOG` environment variable. If `RUST_LOG` is not set, it defaults to a
//!     sensible configuration: `info` for most crates, but `debug` for the `ahma_mcp`
//!     crate itself.
//!
//! 2.  **File Logging (Default)**: By default (`log_to_file = true`), it creates a daily
//!     rolling log file in the user-specific cache directory (determined by the `directories`
//!     crate). This preserves log history without cluttering the console. It uses
//!     `tracing_appender` to handle file rotation and non-blocking I/O. ANSI colors are
//!     disabled for file output.
//!
//! 3.  **Stderr Logging (Opt-in)**: When `log_to_file = false`, all logs are written to
//!     `stderr` with ANSI color codes enabled for better readability on Mac/Linux terminals.
//!     Error messages appear in red, warnings in yellow, etc. This mode is useful for
//!     debugging and development with tools like MCP Inspector.
//!
//! 4.  **Stderr Fallback**: If file logging is requested but the project's cache directory
//!     cannot be determined (e.g., in a sandboxed or unusual environment), the logger
//!     gracefully falls back to writing logs to `stderr` with colors enabled.
//!
//! ## Usage
//!
//! To enable logging, call `ahma_mcp::utils::logging::init_logging(log_level, log_to_file)`
//! at the beginning of the `main` function.
//!
//! For terminal debugging: `init_logging("debug", false)` (logs to stderr with colors)
//! For production: `init_logging("info", true)` (logs to file without colors)

use ahma_common::observability::{ObservabilityConfig, TelemetryGuard};
use anyhow::Result;
use std::{
    io::stderr,
    path::Path,
    sync::{Mutex, Once},
};
use tracing_subscriber::{EnvFilter, fmt::layer, prelude::*};

static INIT: Once = Once::new();
/// Passes the OTEL guard out of the `call_once` closure to the caller.
static PENDING_GUARD: Mutex<Option<TelemetryGuard>> = Mutex::new(None);

/// Initialize verbose logging for tests.
///
/// This configures a `trace`-level subscriber that logs to stderr.
pub fn init_test_logging() {
    let _ = init_logging("trace", false);
}

/// Initializes the logging system.
///
/// Sets up a global tracing subscriber and, when an OTLP endpoint is configured
/// (`--opentelemetry <url>` or `OTEL_EXPORTER_OTLP_ENDPOINT`), also attaches
/// an OTLP exporting layer.
///
/// Returns a [`TelemetryGuard`] that **must** be kept alive for the duration
/// of the process to ensure buffered spans are flushed on shutdown.  When
/// OTEL is not enabled the guard is a cheap no-op.
///
/// When logging to stderr, ANSI colors are enabled for better readability.
/// When logging to file, ANSI colors are disabled.
///
/// # Errors
///
/// Returns an error if the project directories cannot be determined.
pub fn init_logging(log_level: &str, log_to_file: bool) -> Result<TelemetryGuard> {
    init_logging_with_observability(log_level, log_to_file, None)
}

/// Initializes logging with an optional explicit observability configuration.
///
/// When `observability` is `Some`, that configuration is used for OTEL setup;
/// otherwise configuration is read from environment variables.
pub fn init_logging_with_observability(
    log_level: &str,
    log_to_file: bool,
    observability: Option<ObservabilityConfig>,
) -> Result<TelemetryGuard> {
    INIT.call_once(|| {
        do_setup_logging(log_level, log_to_file, observability);
    });

    // Return the guard produced in this call (None on subsequent calls — guard already held).
    Ok(PENDING_GUARD
        .lock()
        .unwrap()
        .take()
        .unwrap_or_else(TelemetryGuard::none))
}

fn do_setup_logging(
    log_level: &str,
    log_to_file: bool,
    observability: Option<ObservabilityConfig>,
) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{log_level},ahma_mcp=debug")));

    // Build the OTEL layer (no-op when no endpoint is configured).
    let (otel_layer, guard) = {
        let config = observability.unwrap_or_else(|| ObservabilityConfig::from_env("ahma_mcp"));
        ahma_common::observability::create_otel_layer(&config)
    };
    *PENDING_GUARD.lock().unwrap() = Some(guard);

    // Attempt to log to a file, fall back to stderr.
    let file_appender_opt = if log_to_file {
        try_create_file_appender()
    } else {
        None
    };
    if let Some(file_appender) = file_appender_opt {
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::registry()
            .with(env_filter)
            .with(otel_layer)
            .with(layer().with_writer(non_blocking).with_ansi(false))
            .init();
        // The guard is intentionally leaked to ensure logs are flushed on exit.
        Box::leak(Box::new(_guard));
        log_traceparent();
        return;
    }

    // Fallback or explicit stderr logging
    tracing_subscriber::registry()
        .with(env_filter)
        .with(otel_layer)
        .with(layer().with_writer(stderr).with_ansi(true))
        .init();
    log_traceparent();
}

fn try_create_file_appender() -> Option<tracing_appender::rolling::RollingFileAppender> {
    let log_dir = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("log");

    // Test if we can actually write to the log directory before calling
    // tracing_appender::rolling::daily, which panics on permission errors
    // in tracing-appender 0.2.4+.
    if !test_write_permission(&log_dir) {
        return None;
    }

    // Delete old standard `.log` files in `log/` to wipe previous logs.
    // Do not delete directories or symlinks.
    cleanup_old_logs(&log_dir);

    // Use catch_unwind to handle panics from tracing_appender
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tracing_appender::rolling::daily(&log_dir, "ahma_mcp.log")
    }))
    .ok()
}

fn cleanup_old_logs(log_dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(log_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(meta) = std::fs::symlink_metadata(&path)
                && meta.is_file()
                && path.extension().is_some_and(|e| e == "log")
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn log_traceparent() {
    // Log the parent trace context injected by the HTTP bridge (if any).
    // This makes it easy to correlate subprocess and bridge traces even
    // before full parent-child linking is wired up.
    if let Some(tp) = ahma_common::observability::env_traceparent() {
        tracing::debug!(traceparent = %tp, "subprocess trace context from HTTP bridge");
    }
}

/// Test if we can write to the given directory.
///
/// This creates the directory if needed, then attempts to create and remove a test file.
/// Used to check write permissions before calling tracing_appender::rolling::daily
/// which panics on permission errors in tracing-appender 0.2.4+.
fn test_write_permission(dir: &Path) -> bool {
    // Try to create the directory
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }

    // Try to create a test file to verify write permission
    let test_file = dir.join(".ahma_log_test");
    match std::fs::write(&test_file, "test") {
        Ok(()) => {
            // Clean up the test file
            let _ = std::fs::remove_file(&test_file);
            true
        }
        Err(_) => false,
    }
}
