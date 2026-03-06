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

use anyhow::Result;
#[cfg(feature = "opentelemetry")]
use opentelemetry::trace::TracerProvider;
#[cfg(feature = "opentelemetry")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "opentelemetry")]
use opentelemetry_sdk::{
    Resource,
    trace::{self as sdktrace, SdkTracerProvider},
};
use std::{io::stderr, path::Path, sync::Once};
use tracing_subscriber::{EnvFilter, fmt::layer, prelude::*};

static INIT: Once = Once::new();

/// Initialize verbose logging for tests.
///
/// This configures a `trace`-level subscriber that logs to stderr.
pub fn init_test_logging() {
    init_logging("trace", false).expect("Failed to initialize test logging");
}

/// Initializes the logging system.
///
/// This function sets up a global tracing subscriber. It can be configured to
/// log to stderr or to a daily rolling file in the project's cache directory.
///
/// When logging to stderr, ANSI colors are enabled for better readability.
/// When logging to file, ANSI colors are disabled.
///
/// # Errors
///
/// Returns an error if the project directories cannot be determined.
pub fn init_logging(log_level: &str, log_to_file: bool) -> Result<()> {
    INIT.call_once(|| {
        let env_filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(format!("{log_level},ahma_mcp=debug")));

        // Attempt to log to a file, fall back to stderr.
        if log_to_file {
            let log_dir = std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("log");

            // Test if we can actually write to the log directory before calling
            // tracing_appender::rolling::daily, which panics on permission errors
            // in tracing-appender 0.2.4+.
            let can_write = test_write_permission(&log_dir);

            if can_write {
                // Delete old standard `.log` files in `log/` to wipe previous logs.
                // Do not delete directories or symlinks.
                if let Ok(entries) = std::fs::read_dir(&log_dir) {
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

            // Try to create the file appender, fall back to stderr if it fails
            // Use catch_unwind to handle panics from tracing_appender
            let file_appender_result = if can_write {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    tracing_appender::rolling::daily(&log_dir, "ahma_mcp.log")
                }))
            } else {
                Err(Box::new("Cannot write to log directory") as Box<dyn std::any::Any + Send>)
            };

            if let Ok(file_appender) = file_appender_result {
                // Successfully created file appender
                let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

                let subscriber = tracing_subscriber::registry()
                    .with(env_filter)
                    .with(layer().with_writer(non_blocking).with_ansi(false));

                #[cfg(feature = "opentelemetry")]
                let subscriber = subscriber.with(init_otel());

                subscriber.init();
                // The guard is intentionally leaked to ensure logs are flushed on exit.
                Box::leak(Box::new(_guard));
                return;
            }
        }

        // Fallback or explicit stderr logging
        let subscriber = tracing_subscriber::registry()
            .with(env_filter)
            .with(layer().with_writer(stderr).with_ansi(true));

        #[cfg(feature = "opentelemetry")]
        let subscriber = subscriber.with(init_otel());

        subscriber.init();
    });

    Ok(())
}

#[cfg(feature = "opentelemetry")]
fn init_otel<S>() -> Option<tracing_opentelemetry::OpenTelemetryLayer<S, sdktrace::Tracer>>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() || std::env::var("AHMA_TRACING").is_ok()
    {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint("http://localhost:4318/v1/traces")
            .build()
            .ok()?;

        let resource = Resource::builder().with_service_name("ahma_mcp").build();

        let provider = SdkTracerProvider::builder()
            .with_resource(resource)
            .with_batch_exporter(exporter)
            .build();

        let tracer = provider.tracer("ahma_mcp");

        Some(tracing_opentelemetry::layer().with_tracer(tracer))
    } else {
        None
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
