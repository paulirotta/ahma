//! # Live Log Monitor
//!
//! Provides real-time monitoring of stdout/stderr streams from long-running processes.
//! When error or warning patterns are detected in the output, the monitor pushes
//! a context snapshot (recent stdout + stderr lines) to the AI client via MCP
//! progress notifications.
//!
//! ## Key Components
//!
//! - [`LogLevel`](crate::log_monitor::LogLevel): Standard Rust-style log levels (Error, Warn, Info, Debug, Trace).
//! - [`MonitorStream`](crate::log_monitor::MonitorStream): Which stream(s) to monitor for trigger patterns.
//! - [`LogMonitorConfig`](crate::log_monitor::LogMonitorConfig): Configuration for a monitoring session.
//! - [`LogLevelDetector`](crate::log_monitor::LogLevelDetector): Compiled regex patterns for detecting log levels in output lines.
//! - [`LogRingBuffer`](crate::log_monitor::LogRingBuffer): Fixed-capacity ring buffers for recent stdout/stderr lines.
//! - [`LogMonitor`](crate::log_monitor::LogMonitor): Orchestrator that ties detection, buffering, and rate-limiting together.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ahma_mcp::log_monitor::{LogMonitor, LogMonitorConfig, LogLevel, MonitorStream};
//!
//! let config = LogMonitorConfig {
//!     monitor_level: LogLevel::Error,
//!     monitor_stream: MonitorStream::Stderr,
//!     rate_limit_seconds: 60,
//! };
//! let mut monitor = LogMonitor::new(config);
//!
//! // Feed lines as they arrive from the process
//! if let Some(snapshot) = monitor.process_line("error[E0308]: mismatched types", true) {
//!     // snapshot contains the last 100 stdout + 100 stderr lines plus the trigger
//!     println!("Alert! {}", snapshot.trigger_line);
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::sync::OnceLock;
use std::time::Instant;

use regex::{Regex, RegexSet};

/// Number of recent lines retained per stream for context snapshots.
/// This is a fixed constant, not user-configurable.
pub const LOG_CONTEXT_LINES: usize = 100;

/// Default rate limit between successive log alerts (seconds).
pub const DEFAULT_RATE_LIMIT_SECONDS: u64 = 60;

const REDACTION_PLACEHOLDER: &str = "[REDACTED]";

fn redaction_rules() -> &'static Vec<(Regex, &'static str)> {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            (
                Regex::new(r"(?i)\bAuthorization\b\s*:\s*\S+\s+\S+")
                    .expect("authorization header redaction regex must compile"),
                "Authorization: [REDACTED]",
            ),
            (
                Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._+/=-]{8,}")
                    .expect("bearer redaction regex must compile"),
                "Bearer [REDACTED]",
            ),
            (
                Regex::new(r"(?i)\b(password|passwd|token|secret|api[_-]?key)\b\s*[:=]\s*(\S+)")
                    .expect("key-value secret redaction regex must compile"),
                "$1=[REDACTED]",
            ),
            (
                Regex::new(r"\bAKIA[0-9A-Z]{16}\b").expect("aws key redaction regex must compile"),
                REDACTION_PLACEHOLDER,
            ),
            (
                Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b")
                    .expect("github token redaction regex must compile"),
                REDACTION_PLACEHOLDER,
            ),
            (
                Regex::new(r"\bsk-[A-Za-z0-9]{16,}\b")
                    .expect("api token redaction regex must compile"),
                REDACTION_PLACEHOLDER,
            ),
        ]
    })
}

/// Redact common secret/token patterns from a single output line.
pub fn redact_sensitive_line(line: &str) -> String {
    let mut redacted = line.to_owned();
    for (pattern, replacement) in redaction_rules() {
        redacted = pattern.replace_all(&redacted, *replacement).into_owned();
    }
    redacted
}

/// Redact common secret/token patterns from multi-line output.
pub fn redact_sensitive_text(text: &str) -> String {
    text.split('\n')
        .map(redact_sensitive_line)
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// LogLevel
// ---------------------------------------------------------------------------

/// Standard log severity levels, matching Rust's `tracing` / `log` conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Only match error-level messages.
    Error,
    /// Match warn and error messages.
    Warn,
    /// Match info, warn, and error messages.
    Info,
    /// Match debug and above.
    Debug,
    /// Match everything (trace and above).
    Trace,
}

impl LogLevel {
    /// Returns `true` if `detected` is severe enough to trigger at this monitor level.
    ///
    /// A monitor set to `Warn` triggers on both `Warn` and `Error`.
    /// A monitor set to `Error` triggers only on `Error`.
    pub fn should_trigger(self, detected: LogLevel) -> bool {
        // Lower ordinal = more severe (Error < Warn < Info < Debug < Trace).
        detected <= self
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogLevel::Error => write!(f, "error"),
            LogLevel::Warn => write!(f, "warn"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Debug => write!(f, "debug"),
            LogLevel::Trace => write!(f, "trace"),
        }
    }
}

impl std::str::FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "error" => Ok(LogLevel::Error),
            "warn" | "warning" => Ok(LogLevel::Warn),
            "info" => Ok(LogLevel::Info),
            "debug" => Ok(LogLevel::Debug),
            "trace" => Ok(LogLevel::Trace),
            other => Err(format!(
                "Unknown log level '{}'. Expected: error, warn, info, debug, trace",
                other
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// MonitorStream
// ---------------------------------------------------------------------------

/// Which output stream(s) to scan for log-level trigger patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum MonitorStream {
    /// Only scan stderr for triggers (default for most CLI tools).
    #[default]
    Stderr,
    /// Only scan stdout for triggers (e.g., Android logcat writes to stdout).
    Stdout,
    /// Scan both streams.
    Both,
}

impl fmt::Display for MonitorStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MonitorStream::Stderr => write!(f, "stderr"),
            MonitorStream::Stdout => write!(f, "stdout"),
            MonitorStream::Both => write!(f, "both"),
        }
    }
}

impl std::str::FromStr for MonitorStream {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stderr" => Ok(MonitorStream::Stderr),
            "stdout" => Ok(MonitorStream::Stdout),
            "both" => Ok(MonitorStream::Both),
            other => Err(format!(
                "Unknown monitor stream '{}'. Expected: stderr, stdout, both",
                other
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// LogMonitorConfig
// ---------------------------------------------------------------------------

/// Configuration for a log monitoring session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogMonitorConfig {
    /// Minimum severity level that triggers an alert.
    pub monitor_level: LogLevel,
    /// Which stream(s) to scan for trigger patterns.
    pub monitor_stream: MonitorStream,
    /// Minimum seconds between successive alerts for the same operation.
    pub rate_limit_seconds: u64,
}

impl Default for LogMonitorConfig {
    fn default() -> Self {
        Self {
            monitor_level: LogLevel::Error,
            monitor_stream: MonitorStream::Stderr,
            rate_limit_seconds: DEFAULT_RATE_LIMIT_SECONDS,
        }
    }
}

// ---------------------------------------------------------------------------
// LogSnapshot
// ---------------------------------------------------------------------------

/// A point-in-time snapshot of recent output captured when a trigger fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogSnapshot {
    /// The line that triggered the alert.
    pub trigger_line: String,
    /// The detected severity of the trigger line.
    pub trigger_level: LogLevel,
    /// Whether the trigger came from stderr (`true`) or stdout (`false`).
    pub trigger_is_stderr: bool,
    /// Recent stdout lines (up to [`LOG_CONTEXT_LINES`]).
    pub stdout_context: Vec<String>,
    /// Recent stderr lines (up to [`LOG_CONTEXT_LINES`]).
    pub stderr_context: Vec<String>,
}

impl LogSnapshot {
    /// Format the snapshot as a multi-section string suitable for progress notification messages.
    pub fn format_for_notification(&self) -> String {
        let source = if self.trigger_is_stderr {
            "stderr"
        } else {
            "stdout"
        };

        let mut sections = Vec::new();

        sections.push(format!(
            "=== LOG ALERT ({}: {}) ===",
            self.trigger_level, source
        ));
        sections.push(format!(
            "Trigger: {}",
            redact_sensitive_line(&self.trigger_line)
        ));

        if !self.stderr_context.is_empty() {
            sections.push(format!(
                "\n--- stderr (last {} lines) ---",
                self.stderr_context.len()
            ));
            sections.push(
                self.stderr_context
                    .iter()
                    .map(|line| redact_sensitive_line(line))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }

        if !self.stdout_context.is_empty() {
            sections.push(format!(
                "\n--- stdout (last {} lines) ---",
                self.stdout_context.len()
            ));
            sections.push(
                self.stdout_context
                    .iter()
                    .map(|line| redact_sensitive_line(line))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }

        sections.join("\n")
    }
}

// ---------------------------------------------------------------------------
// LogRingBuffer
// ---------------------------------------------------------------------------

/// Fixed-capacity ring buffers for recent stdout and stderr lines.
///
/// Each buffer holds at most [`LOG_CONTEXT_LINES`] entries. Older lines are
/// evicted in FIFO order when the buffer is full.
#[derive(Debug, Clone)]
pub struct LogRingBuffer {
    stdout: VecDeque<String>,
    stderr: VecDeque<String>,
}

impl LogRingBuffer {
    /// Create a new empty ring buffer pair.
    pub fn new() -> Self {
        Self {
            stdout: VecDeque::with_capacity(LOG_CONTEXT_LINES),
            stderr: VecDeque::with_capacity(LOG_CONTEXT_LINES),
        }
    }

    /// Push a line to the stdout buffer.
    pub fn push_stdout(&mut self, line: String) {
        if self.stdout.len() >= LOG_CONTEXT_LINES {
            self.stdout.pop_front();
        }
        self.stdout.push_back(line);
    }

    /// Push a line to the stderr buffer.
    pub fn push_stderr(&mut self, line: String) {
        if self.stderr.len() >= LOG_CONTEXT_LINES {
            self.stderr.pop_front();
        }
        self.stderr.push_back(line);
    }

    /// Push a line to the appropriate buffer.
    pub fn push(&mut self, line: String, is_stderr: bool) {
        if is_stderr {
            self.push_stderr(line);
        } else {
            self.push_stdout(line);
        }
    }

    /// Take a snapshot of both buffers (clones the current contents).
    pub fn snapshot(
        &self,
        trigger_line: String,
        trigger_level: LogLevel,
        trigger_is_stderr: bool,
    ) -> LogSnapshot {
        LogSnapshot {
            trigger_line,
            trigger_level,
            trigger_is_stderr,
            stdout_context: self.stdout.iter().cloned().collect(),
            stderr_context: self.stderr.iter().cloned().collect(),
        }
    }

    /// Number of stdout lines currently buffered.
    pub fn stdout_len(&self) -> usize {
        self.stdout.len()
    }

    /// Number of stderr lines currently buffered.
    pub fn stderr_len(&self) -> usize {
        self.stderr.len()
    }

    /// Clear both buffers.
    pub fn clear(&mut self) {
        self.stdout.clear();
        self.stderr.clear();
    }
}

impl Default for LogRingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// LogLevelDetector
// ---------------------------------------------------------------------------

/// Detects log severity levels in output lines using compiled regex patterns.
///
/// Patterns are grouped by severity. A line is tested against progressively less
/// severe pattern sets until a match is found or all sets are exhausted.
///
/// Pattern groups:
/// - **Error**: `error[E0308]:`, `[ERROR]`, `ERROR:`, `E/` (logcat), `FATAL`, `panic`, `panicked`
/// - **Warn**: `warning[...]:`, `[WARN]`, `[WARNING]`, `WARN:`, `WARNING:`, `W/` (logcat)
/// - **Info**: `[INFO]`, `INFO:`, `I/` (logcat)
/// - **Debug**: `[DEBUG]`, `DEBUG:`, `D/` (logcat)
/// - **Trace**: `[TRACE]`, `TRACE:`, `V/` (logcat — verbose)
pub struct LogLevelDetector {
    error_patterns: RegexSet,
    warn_patterns: RegexSet,
    info_patterns: RegexSet,
    debug_patterns: RegexSet,
    trace_patterns: RegexSet,
}

impl LogLevelDetector {
    /// Build a new detector with the default pattern sets.
    pub fn new() -> Self {
        let error_patterns = RegexSet::new([
            // Rust compiler errors: error[E0308]: ...
            r"(?i)^error(\[E\d+\])?:",
            // Bracketed log format: [ERROR] ...
            r"(?i)^\[ERROR\]",
            // Prefixed log format: ERROR: ...
            r"(?i)^ERROR:",
            // Android logcat: E/Tag: ...
            r"^E/",
            // Fatal errors
            r"(?i)^FATAL",
            // Rust panics
            r"(?i)^thread '.+' panicked",
            // Generic panic
            r"(?i)^panicked at",
        ])
        .expect("Error regex patterns must compile");

        let warn_patterns = RegexSet::new([
            // Rust compiler warnings: warning[unused]: ... or warning: ...
            r"(?i)^warning(\[.+\])?:",
            // Bracketed: [WARN] or [WARNING]
            r"(?i)^\[WARN(ING)?\]",
            // Prefixed: WARN: or WARNING:
            r"(?i)^WARN(ING)?:",
            // Android logcat: W/Tag: ...
            r"^W/",
        ])
        .expect("Warn regex patterns must compile");

        let info_patterns = RegexSet::new([
            r"(?i)^\[INFO\]",
            r"(?i)^INFO:",
            // Android logcat: I/Tag: ...
            r"^I/",
        ])
        .expect("Info regex patterns must compile");

        let debug_patterns = RegexSet::new([
            r"(?i)^\[DEBUG\]",
            r"(?i)^DEBUG:",
            // Android logcat: D/Tag: ...
            r"^D/",
        ])
        .expect("Debug regex patterns must compile");

        let trace_patterns = RegexSet::new([
            r"(?i)^\[TRACE\]",
            r"(?i)^TRACE:",
            // Android logcat verbose: V/Tag: ...
            r"^V/",
        ])
        .expect("Trace regex patterns must compile");

        Self {
            error_patterns,
            warn_patterns,
            info_patterns,
            debug_patterns,
            trace_patterns,
        }
    }

    /// Detect the log level of a line, if any recognized pattern matches.
    ///
    /// Returns the most severe matching level. Checks error first, then warn, etc.
    pub fn detect(&self, line: &str) -> Option<LogLevel> {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            return None;
        }

        if self.error_patterns.is_match(trimmed) {
            return Some(LogLevel::Error);
        }
        if self.warn_patterns.is_match(trimmed) {
            return Some(LogLevel::Warn);
        }
        if self.info_patterns.is_match(trimmed) {
            return Some(LogLevel::Info);
        }
        if self.debug_patterns.is_match(trimmed) {
            return Some(LogLevel::Debug);
        }
        if self.trace_patterns.is_match(trimmed) {
            return Some(LogLevel::Trace);
        }
        None
    }
}

impl Default for LogLevelDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LogLevelDetector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogLevelDetector")
            .field("error_patterns", &self.error_patterns.len())
            .field("warn_patterns", &self.warn_patterns.len())
            .field("info_patterns", &self.info_patterns.len())
            .field("debug_patterns", &self.debug_patterns.len())
            .field("trace_patterns", &self.trace_patterns.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// LogMonitor
// ---------------------------------------------------------------------------

/// Orchestrates log-level detection, ring-buffer management, and rate limiting.
///
/// Feed output lines via [`LogMonitor::process_line`]. When a trigger fires (detected
/// level is severe enough AND rate limit has elapsed), a [`LogSnapshot`] is returned
/// containing recent context for both streams.
pub struct LogMonitor {
    config: LogMonitorConfig,
    buffer: LogRingBuffer,
    detector: LogLevelDetector,
    last_alert_time: Option<Instant>,
}

impl LogMonitor {
    /// Create a new monitor with the given configuration.
    pub fn new(config: LogMonitorConfig) -> Self {
        Self {
            config,
            buffer: LogRingBuffer::new(),
            detector: LogLevelDetector::new(),
            last_alert_time: None,
        }
    }

    /// Process a single output line from the monitored process.
    ///
    /// The line is always added to the ring buffer regardless of whether an alert fires.
    ///
    /// # Returns
    ///
    /// `Some(LogSnapshot)` if the line triggers an alert (severity meets threshold AND
    /// rate limit has elapsed), otherwise `None`.
    pub fn process_line(&mut self, line: &str, is_stderr: bool) -> Option<LogSnapshot> {
        // Always buffer the line first
        self.buffer.push(line.to_owned(), is_stderr);

        // Check if this stream is being monitored for triggers
        if !self.is_stream_monitored(is_stderr) {
            return None;
        }

        // Detect log level
        let detected_level = self.detector.detect(line)?;

        // Check if the detected level meets the configured threshold
        if !self.config.monitor_level.should_trigger(detected_level) {
            return None;
        }

        // Rate limiting
        if let Some(last) = self.last_alert_time
            && last.elapsed().as_secs() < self.config.rate_limit_seconds
        {
            return None;
        }

        // Fire the alert
        self.last_alert_time = Some(Instant::now());
        Some(
            self.buffer
                .snapshot(line.to_owned(), detected_level, is_stderr),
        )
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &LogMonitorConfig {
        &self.config
    }

    /// Returns a reference to the ring buffer.
    pub fn buffer(&self) -> &LogRingBuffer {
        &self.buffer
    }

    /// Check whether the given stream is monitored for triggers.
    fn is_stream_monitored(&self, is_stderr: bool) -> bool {
        match self.config.monitor_stream {
            MonitorStream::Stderr => is_stderr,
            MonitorStream::Stdout => !is_stderr,
            MonitorStream::Both => true,
        }
    }

    /// Manually reset the rate limiter (useful for testing).
    #[cfg(test)]
    pub fn reset_rate_limit(&mut self) {
        self.last_alert_time = None;
    }
}

impl fmt::Debug for LogMonitor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogMonitor")
            .field("config", &self.config)
            .field("buffer_stdout_len", &self.buffer.stdout_len())
            .field("buffer_stderr_len", &self.buffer.stderr_len())
            .field("last_alert_time", &self.last_alert_time)
            .finish()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // LogLevel tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
    }

    #[test]
    fn test_log_level_should_trigger() {
        // Error monitor: only triggers on Error
        assert!(LogLevel::Error.should_trigger(LogLevel::Error));
        assert!(!LogLevel::Error.should_trigger(LogLevel::Warn));
        assert!(!LogLevel::Error.should_trigger(LogLevel::Info));

        // Warn monitor: triggers on Error and Warn
        assert!(LogLevel::Warn.should_trigger(LogLevel::Error));
        assert!(LogLevel::Warn.should_trigger(LogLevel::Warn));
        assert!(!LogLevel::Warn.should_trigger(LogLevel::Info));

        // Trace monitor: triggers on everything
        assert!(LogLevel::Trace.should_trigger(LogLevel::Error));
        assert!(LogLevel::Trace.should_trigger(LogLevel::Warn));
        assert!(LogLevel::Trace.should_trigger(LogLevel::Info));
        assert!(LogLevel::Trace.should_trigger(LogLevel::Debug));
        assert!(LogLevel::Trace.should_trigger(LogLevel::Trace));
    }

    #[test]
    fn test_log_level_display_and_parse() {
        for level in [
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ] {
            let s = level.to_string();
            let parsed: LogLevel = s.parse().unwrap();
            assert_eq!(parsed, level);
        }
        // "warning" alias
        assert_eq!("warning".parse::<LogLevel>().unwrap(), LogLevel::Warn);
        // unknown
        assert!("unknown".parse::<LogLevel>().is_err());
    }

    // -----------------------------------------------------------------------
    // MonitorStream tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_monitor_stream_default() {
        assert_eq!(MonitorStream::default(), MonitorStream::Stderr);
    }

    #[test]
    fn test_monitor_stream_parse() {
        assert_eq!(
            "stderr".parse::<MonitorStream>().unwrap(),
            MonitorStream::Stderr
        );
        assert_eq!(
            "stdout".parse::<MonitorStream>().unwrap(),
            MonitorStream::Stdout
        );
        assert_eq!(
            "both".parse::<MonitorStream>().unwrap(),
            MonitorStream::Both
        );
        assert!("invalid".parse::<MonitorStream>().is_err());
    }

    // -----------------------------------------------------------------------
    // LogRingBuffer tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_ring_buffer_empty() {
        let buf = LogRingBuffer::new();
        assert_eq!(buf.stdout_len(), 0);
        assert_eq!(buf.stderr_len(), 0);
        let snap = buf.snapshot("trigger".into(), LogLevel::Error, true);
        assert!(snap.stdout_context.is_empty());
        assert!(snap.stderr_context.is_empty());
    }

    #[test]
    fn test_ring_buffer_push_and_snapshot() {
        let mut buf = LogRingBuffer::new();
        buf.push_stdout("line1".into());
        buf.push_stdout("line2".into());
        buf.push_stderr("err1".into());
        assert_eq!(buf.stdout_len(), 2);
        assert_eq!(buf.stderr_len(), 1);

        let snap = buf.snapshot("trigger".into(), LogLevel::Error, true);
        assert_eq!(snap.stdout_context, vec!["line1", "line2"]);
        assert_eq!(snap.stderr_context, vec!["err1"]);
        assert_eq!(snap.trigger_line, "trigger");
    }

    #[test]
    fn test_ring_buffer_capacity_eviction() {
        let mut buf = LogRingBuffer::new();
        for i in 0..150 {
            buf.push_stdout(format!("line{}", i));
        }
        assert_eq!(buf.stdout_len(), LOG_CONTEXT_LINES);
        let snap = buf.snapshot("trigger".into(), LogLevel::Error, false);
        // Should contain lines 50-149 (the most recent 100)
        assert_eq!(snap.stdout_context.first().unwrap(), "line50");
        assert_eq!(snap.stdout_context.last().unwrap(), "line149");
    }

    #[test]
    fn test_ring_buffer_push_dispatches_by_stream() {
        let mut buf = LogRingBuffer::new();
        buf.push("stderr_line".into(), true);
        buf.push("stdout_line".into(), false);
        assert_eq!(buf.stdout_len(), 1);
        assert_eq!(buf.stderr_len(), 1);
    }

    #[test]
    fn test_ring_buffer_clear() {
        let mut buf = LogRingBuffer::new();
        buf.push_stdout("a".into());
        buf.push_stderr("b".into());
        buf.clear();
        assert_eq!(buf.stdout_len(), 0);
        assert_eq!(buf.stderr_len(), 0);
    }

    // -----------------------------------------------------------------------
    // LogLevelDetector tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_rust_compiler_errors() {
        let det = LogLevelDetector::new();
        assert_eq!(
            det.detect("error[E0308]: mismatched types"),
            Some(LogLevel::Error)
        );
        assert_eq!(
            det.detect("error: could not compile `myproject`"),
            Some(LogLevel::Error)
        );
    }

    #[test]
    fn test_detect_rust_compiler_warnings() {
        let det = LogLevelDetector::new();
        assert_eq!(
            det.detect("warning: unused variable: `x`"),
            Some(LogLevel::Warn)
        );
        assert_eq!(
            det.detect("warning[unused]: unused import"),
            Some(LogLevel::Warn)
        );
    }

    #[test]
    fn test_detect_bracketed_format() {
        let det = LogLevelDetector::new();
        assert_eq!(det.detect("[ERROR] something bad"), Some(LogLevel::Error));
        assert_eq!(det.detect("[WARN] something iffy"), Some(LogLevel::Warn));
        assert_eq!(det.detect("[WARNING] something iffy"), Some(LogLevel::Warn));
        assert_eq!(det.detect("[INFO] something normal"), Some(LogLevel::Info));
        assert_eq!(det.detect("[DEBUG] detailed info"), Some(LogLevel::Debug));
        assert_eq!(det.detect("[TRACE] very detailed"), Some(LogLevel::Trace));
    }

    #[test]
    fn test_detect_prefixed_format() {
        let det = LogLevelDetector::new();
        assert_eq!(det.detect("ERROR: disk full"), Some(LogLevel::Error));
        assert_eq!(det.detect("WARN: low memory"), Some(LogLevel::Warn));
        assert_eq!(det.detect("WARNING: low memory"), Some(LogLevel::Warn));
        assert_eq!(det.detect("INFO: started"), Some(LogLevel::Info));
        assert_eq!(det.detect("DEBUG: checking"), Some(LogLevel::Debug));
        assert_eq!(det.detect("TRACE: entering fn"), Some(LogLevel::Trace));
    }

    #[test]
    fn test_detect_android_logcat() {
        let det = LogLevelDetector::new();
        assert_eq!(
            det.detect("E/ActivityManager: Process crashed"),
            Some(LogLevel::Error)
        );
        assert_eq!(det.detect("W/System: Slow operation"), Some(LogLevel::Warn));
        assert_eq!(
            det.detect("I/ActivityManager: Displayed activity"),
            Some(LogLevel::Info)
        );
        assert_eq!(
            det.detect("D/dalvikvm: GC_CONCURRENT"),
            Some(LogLevel::Debug)
        );
        assert_eq!(
            det.detect("V/SomeTag: verbose output"),
            Some(LogLevel::Trace)
        );
    }

    #[test]
    fn test_detect_panic() {
        let det = LogLevelDetector::new();
        assert_eq!(
            det.detect("thread 'main' panicked at 'assertion failed'"),
            Some(LogLevel::Error)
        );
        assert_eq!(
            det.detect("panicked at 'index out of bounds'"),
            Some(LogLevel::Error)
        );
        assert_eq!(det.detect("FATAL: cannot continue"), Some(LogLevel::Error));
    }

    #[test]
    fn test_detect_case_insensitive() {
        let det = LogLevelDetector::new();
        assert_eq!(det.detect("Error: something"), Some(LogLevel::Error));
        assert_eq!(det.detect("ERROR: something"), Some(LogLevel::Error));
        assert_eq!(det.detect("Warning: something"), Some(LogLevel::Warn));
    }

    #[test]
    fn test_detect_no_match() {
        let det = LogLevelDetector::new();
        assert_eq!(det.detect(""), None);
        assert_eq!(det.detect("   "), None);
        assert_eq!(det.detect("Compiling myproject v0.1.0"), None);
        assert_eq!(det.detect("Finished dev [unoptimized + debuginfo]"), None);
        assert_eq!(det.detect("Running `target/debug/myproject`"), None);
    }

    #[test]
    fn test_detect_false_positive_avoidance() {
        let det = LogLevelDetector::new();
        // "0 errors" should NOT match — it doesn't start with "error"
        assert_eq!(det.detect("0 errors, 0 warnings"), None);
        // "error_count = 0" — doesn't match "error:" pattern
        assert_eq!(det.detect("error_count = 0"), None);
        // Midline "error" without the right prefix — no match
        assert_eq!(det.detect("found 0 error(s) in code"), None);
    }

    #[test]
    fn test_detect_leading_whitespace() {
        let det = LogLevelDetector::new();
        // Lines with leading whitespace should still match after trimming
        assert_eq!(det.detect("  error: something"), Some(LogLevel::Error));
        assert_eq!(det.detect("\t[WARN] something"), Some(LogLevel::Warn));
    }

    // -----------------------------------------------------------------------
    // LogMonitor tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_monitor_triggers_on_error() {
        let config = LogMonitorConfig {
            monitor_level: LogLevel::Error,
            monitor_stream: MonitorStream::Stderr,
            rate_limit_seconds: 0, // No rate limiting for test
        };
        let mut monitor = LogMonitor::new(config);

        // Non-error line: no trigger
        assert!(monitor.process_line("Compiling myproject", true).is_none());

        // Error line on stderr: triggers
        let snap = monitor
            .process_line("error[E0308]: mismatched types", true)
            .expect("should trigger");
        assert_eq!(snap.trigger_level, LogLevel::Error);
        assert!(snap.trigger_is_stderr);
    }

    #[test]
    fn test_monitor_warns_at_error_level_no_trigger() {
        let config = LogMonitorConfig {
            monitor_level: LogLevel::Error,
            monitor_stream: MonitorStream::Stderr,
            rate_limit_seconds: 0,
        };
        let mut monitor = LogMonitor::new(config);

        // Warning on error-level monitor: no trigger
        assert!(
            monitor
                .process_line("warning: unused variable", true)
                .is_none()
        );
    }

    #[test]
    fn test_monitor_warns_at_warn_level_triggers() {
        let config = LogMonitorConfig {
            monitor_level: LogLevel::Warn,
            monitor_stream: MonitorStream::Stderr,
            rate_limit_seconds: 0,
        };
        let mut monitor = LogMonitor::new(config);

        let snap = monitor
            .process_line("warning: unused variable", true)
            .expect("should trigger");
        assert_eq!(snap.trigger_level, LogLevel::Warn);
    }

    #[test]
    fn test_monitor_stream_filter_stderr_only() {
        let config = LogMonitorConfig {
            monitor_level: LogLevel::Error,
            monitor_stream: MonitorStream::Stderr,
            rate_limit_seconds: 0,
        };
        let mut monitor = LogMonitor::new(config);

        // Error on stdout: not monitored
        assert!(
            monitor
                .process_line("error: something bad", false)
                .is_none()
        );

        // Error on stderr: monitored
        assert!(monitor.process_line("error: something bad", true).is_some());
    }

    #[test]
    fn test_monitor_stream_filter_stdout_only() {
        let config = LogMonitorConfig {
            monitor_level: LogLevel::Error,
            monitor_stream: MonitorStream::Stdout,
            rate_limit_seconds: 0,
        };
        let mut monitor = LogMonitor::new(config);

        // Error on stderr: not monitored
        assert!(monitor.process_line("error: something bad", true).is_none());

        // Error on stdout: monitored
        assert!(
            monitor
                .process_line("error: something bad", false)
                .is_some()
        );
    }

    #[test]
    fn test_monitor_stream_filter_both() {
        let config = LogMonitorConfig {
            monitor_level: LogLevel::Error,
            monitor_stream: MonitorStream::Both,
            rate_limit_seconds: 0,
        };
        let mut monitor = LogMonitor::new(config);

        assert!(monitor.process_line("error: from stderr", true).is_some());
        // Reset rate limit for next check
        monitor.reset_rate_limit();
        assert!(monitor.process_line("error: from stdout", false).is_some());
    }

    #[test]
    fn test_monitor_rate_limiting() {
        let config = LogMonitorConfig {
            monitor_level: LogLevel::Error,
            monitor_stream: MonitorStream::Stderr,
            rate_limit_seconds: 3600, // 1 hour — effectively infinite for this test
        };
        let mut monitor = LogMonitor::new(config);

        // First error: triggers
        assert!(monitor.process_line("error: first", true).is_some());

        // Second error within rate limit: suppressed
        assert!(monitor.process_line("error: second", true).is_none());

        // Third error within rate limit: still suppressed
        assert!(monitor.process_line("error: third", true).is_none());
    }

    #[test]
    fn test_monitor_buffers_even_when_not_triggering() {
        let config = LogMonitorConfig {
            monitor_level: LogLevel::Error,
            monitor_stream: MonitorStream::Stderr,
            rate_limit_seconds: 0,
        };
        let mut monitor = LogMonitor::new(config);

        // Non-error lines are still buffered
        for i in 0..5 {
            monitor.process_line(&format!("info line {}", i), false);
            monitor.process_line(&format!("stderr line {}", i), true);
        }

        assert_eq!(monitor.buffer().stdout_len(), 5);
        assert_eq!(monitor.buffer().stderr_len(), 5);

        // When an error triggers, the context includes all buffered lines
        let snap = monitor
            .process_line("error: kaboom", true)
            .expect("should trigger");
        assert_eq!(snap.stdout_context.len(), 5);
        // 5 previous stderr lines + the trigger line itself
        assert_eq!(snap.stderr_context.len(), 6);
    }

    #[test]
    fn test_monitor_snapshot_context() {
        let config = LogMonitorConfig {
            monitor_level: LogLevel::Error,
            monitor_stream: MonitorStream::Stderr,
            rate_limit_seconds: 0,
        };
        let mut monitor = LogMonitor::new(config);

        // Fill context
        for i in 0..3 {
            monitor.process_line(&format!("stdout_{}", i), false);
            monitor.process_line(&format!("stderr_{}", i), true);
        }

        let snap = monitor
            .process_line("error: the problem", true)
            .expect("should trigger");

        assert_eq!(snap.trigger_line, "error: the problem");
        assert_eq!(snap.trigger_level, LogLevel::Error);
        assert!(snap.trigger_is_stderr);
        assert_eq!(
            snap.stdout_context,
            vec!["stdout_0", "stdout_1", "stdout_2"]
        );
        // stderr has the 3 previous lines + the trigger line
        assert_eq!(
            snap.stderr_context,
            vec!["stderr_0", "stderr_1", "stderr_2", "error: the problem"]
        );
    }

    // -----------------------------------------------------------------------
    // LogSnapshot formatting tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_snapshot_format_for_notification() {
        let snap = LogSnapshot {
            trigger_line: "error: test failure".into(),
            trigger_level: LogLevel::Error,
            trigger_is_stderr: true,
            stdout_context: vec!["Compiling...".into(), "Running tests...".into()],
            stderr_context: vec!["error: test failure".into()],
        };

        let formatted = snap.format_for_notification();
        assert!(formatted.contains("LOG ALERT (error: stderr)"));
        assert!(formatted.contains("Trigger: error: test failure"));
        assert!(formatted.contains("stderr (last 1 lines)"));
        assert!(formatted.contains("stdout (last 2 lines)"));
    }

    #[test]
    fn test_redact_sensitive_line() {
        let line = "Authorization: Bearer abcdef1234567890 token=supersecret sk-1234567890ABCDEF";
        let redacted = redact_sensitive_line(line);
        assert!(!redacted.contains("abcdef1234567890"));
        assert!(!redacted.contains("supersecret"));
        assert!(!redacted.contains("sk-1234567890ABCDEF"));
        assert!(redacted.contains("Authorization: [REDACTED]"));
        assert!(redacted.contains("token=[REDACTED]"));
    }

    #[test]
    fn test_snapshot_format_redacts_secrets() {
        let snap = LogSnapshot {
            trigger_line: "error: token=abc123def456".into(),
            trigger_level: LogLevel::Error,
            trigger_is_stderr: true,
            stdout_context: vec!["api_key: my-real-key".into()],
            stderr_context: vec!["Authorization: Bearer secret-token-value".into()],
        };

        let formatted = snap.format_for_notification();
        assert!(!formatted.contains("abc123def456"));
        assert!(!formatted.contains("my-real-key"));
        assert!(!formatted.contains("secret-token-value"));
        assert!(formatted.contains("[REDACTED]"));
    }
}
