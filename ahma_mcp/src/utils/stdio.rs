//! Safe stdout notification delivery for the subprocess-to-bridge protocol.
//!
//! In HTTP bridge mode, per-session `ahma-mcp` subprocesses communicate with
//! the bridge via stdin/stdout pipes.  Sandbox lifecycle notifications
//! (`configured`, `failed`, `terminated`) are written as raw JSON-RPC to
//! stdout so the bridge can intercept and broadcast them.
//!
//! **Why not `println!`?**  `println!` panics on *any* write error.  On
//! Windows, OS error 232 ("The pipe is being closed") fires when the bridge
//! kills the subprocess or closes its end of the pipe during shutdown.  On
//! Unix, SIGPIPE can cause similar issues.  A panic here is never useful:
//!
//! - **During shutdown** the notification is best-effort; the bridge is
//!   already tearing down.
//! - **During active operation** a broken pipe means the bridge crashed.
//!   The subprocess should log and exit, not panic with a stack trace.
//!
//! See SPEC.md R5.6.1 for the formal requirement.

use std::io::{self, ErrorKind, Write};

/// Write a JSON-RPC notification to stdout for the bridge to read.
///
/// The notification is prefixed with `\n` and followed by `\n` (via
/// `writeln!`) to ensure the bridge's line-oriented reader can parse it
/// even if it arrives concatenated with a previous partial message.
///
/// # Error handling
///
/// - **Broken pipe** (`ErrorKind::BrokenPipe`, Windows error 232): logged
///   at `debug` level and treated as success.  This is expected when the
///   bridge closes the pipe during shutdown.
/// - **Other I/O errors**: logged at `warn` level and returned so callers
///   can decide whether to continue or abort.
///
/// # Returns
///
/// `Ok(())` on success or broken pipe, `Err(io::Error)` on unexpected
/// write failures.
pub fn emit_stdout_notification(json: &str) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    match writeln!(stdout, "\n{}", json) {
        Ok(()) => {
            let _ = stdout.flush();
            Ok(())
        }
        Err(e) if is_broken_pipe(&e) => {
            tracing::debug!("stdout pipe closed (broken pipe) — notification not delivered");
            Ok(())
        }
        Err(e) => {
            tracing::warn!("Unexpected stdout write error: {}", e);
            Err(e)
        }
    }
}

/// Returns `true` for broken-pipe errors on both Unix and Windows.
///
/// - Unix: `ErrorKind::BrokenPipe` (EPIPE)
/// - Windows: `ErrorKind::BrokenPipe` (mapped from OS error 232)
fn is_broken_pipe(e: &io::Error) -> bool {
    e.kind() == ErrorKind::BrokenPipe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_broken_pipe_unix_style() {
        let err = io::Error::new(ErrorKind::BrokenPipe, "pipe closed");
        assert!(is_broken_pipe(&err));
    }

    #[test]
    fn test_is_not_broken_pipe() {
        let err = io::Error::new(ErrorKind::NotFound, "not found");
        assert!(!is_broken_pipe(&err));
    }

    #[test]
    fn test_emit_stdout_notification_success() {
        let json = r#"{"jsonrpc":"2.0","method":"test"}"#;
        assert!(emit_stdout_notification(json).is_ok());
    }
}
