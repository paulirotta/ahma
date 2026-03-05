//! Patched transport for MCP stdio connections.
//!
//! This module adapts the `rmcp` transport to support both line-delimited JSON
//! and Content-Length framed messages on stdin/stdout. It is primarily used by
//! the HTTP bridge and testing utilities.
//!
//! ## Security
//! This transport only handles framing. It does not perform authentication or
//! validation; callers should enforce sandboxing and schema checks elsewhere.

use rmcp::{
    service::{RoleServer, RxJsonRpcMessage, TxJsonRpcMessage},
    transport::Transport,
};
use serde_json::Value;
use std::io::Error;
use std::sync::Arc;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::sync::Mutex;
use tracing;

/// Transport wrapper supporting mixed MCP framing styles.
#[derive(Clone)]
pub struct PatchedTransport<R, W> {
    reader: Arc<Mutex<R>>,
    writer: Arc<Mutex<W>>,
}

impl<R, W> PatchedTransport<R, W>
where
    R: AsyncBufRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    /// Create a patched transport from an async reader and writer.
    ///
    /// # Arguments
    /// * `reader` - Async buffered reader (stdin or TCP stream).
    /// * `writer` - Async writer (stdout or TCP stream).
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: Arc::new(Mutex::new(reader)),
            writer: Arc::new(Mutex::new(writer)),
        }
    }
}

/// Convenience alias for stdio-based patched transport.
pub type PatchedStdioTransport = PatchedTransport<BufReader<tokio::io::Stdin>, tokio::io::Stdout>;

impl PatchedStdioTransport {
    /// Create a patched transport bound to process stdin/stdout.
    pub fn new_stdio() -> Self {
        Self::new(BufReader::new(tokio::io::stdin()), tokio::io::stdout())
    }
}

impl<R, W> Transport<RoleServer> for PatchedTransport<R, W>
where
    R: AsyncBufRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Error = Error;

    fn send(
        &mut self,
        msg: TxJsonRpcMessage<RoleServer>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let writer = self.writer.clone();
        let json_res =
            serde_json::to_string(&msg).map_err(|e| Error::new(std::io::ErrorKind::InvalidData, e));

        async move {
            let mut json = json_res?;
            json.push('\n');
            let mut w = writer.lock().await;
            tracing::debug!("[AhmaTransport] SEND: {}", json.trim_end());
            w.write_all(json.as_bytes()).await?;
            w.flush().await?;
            Ok(())
        }
    }

    fn receive(
        &mut self,
    ) -> impl std::future::Future<Output = Option<RxJsonRpcMessage<RoleServer>>> + Send {
        let reader = self.reader.clone();

        async move {
            let mut r = reader.lock().await;
            loop {
                // Peek/Read logic to handle both Line-Delimited and Content-Length framed messages
                let mut first_line = String::new();
                match r.read_line(&mut first_line).await {
                    Ok(0) => return None, // EOF
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!("[AhmaTransport] Read Error: {}", e);
                        return None;
                    }
                }

                let message_body = if first_line.starts_with("Content-Length:") {
                    // Header Mode
                    let len_str = first_line
                        .trim()
                        .strip_prefix("Content-Length:")
                        .unwrap_or("0")
                        .trim();
                    let content_len: usize = len_str.parse().unwrap_or(0);
                    tracing::debug!(
                        "[AhmaTransport] Detected Header Framing. Content-Length: {}",
                        content_len
                    );

                    // Skip remaining headers until empty line
                    loop {
                        let mut h = String::new();
                        if let Ok(n) = r.read_line(&mut h).await {
                            if n == 0 || h.trim().is_empty() {
                                break;
                            }
                        } else {
                            break;
                        }
                    }

                    // Read exact bytes
                    let mut buf = vec![0u8; content_len];
                    if let Err(e) = r.read_exact(&mut buf).await {
                        tracing::debug!("[AhmaTransport] Failed to read body: {}", e);
                        continue;
                    }
                    match String::from_utf8(buf) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::debug!("[AhmaTransport] Invalid UTF-8 body: {}", e);
                            continue;
                        }
                    }
                } else {
                    // Line Mode
                    first_line
                };

                // Try to parse as Value to inspect and patch
                let mut value: Value = match serde_json::from_str(&message_body) {
                    Ok(v) => {
                        tracing::debug!("[AhmaTransport] RECV RAW: {}", message_body.trim());
                        v
                    }
                    Err(e) => {
                        if !message_body.trim().is_empty() {
                            tracing::debug!(
                                "[AhmaTransport] Invalid JSON: {} | Content: {}",
                                e,
                                message_body
                            );
                        }
                        // Ignore invalid JSON lines and continue loop
                        continue;
                    }
                };

                // --- PATCHING LOGIC ---
                if let Some(method) = value.get("method").and_then(|v| v.as_str())
                    && method == "initialize"
                {
                    tracing::debug!(
                        "[AhmaTransport] Detected 'initialize' request. Checking capabilities..."
                    );
                    if let Some(params) = value.get_mut("params")
                        && let Some(caps) = params.get_mut("capabilities")
                        && let Some(tasks) = caps.get("tasks")
                    {
                        if tasks.is_object() {
                            tracing::debug!(
                                "[AhmaTransport] Patching: Removing 'tasks' capability object"
                            );
                            if let Some(caps_obj) = caps.as_object_mut() {
                                caps_obj.remove("tasks");
                            }
                        } else {
                            tracing::debug!(
                                "[AhmaTransport] 'tasks' capability found but not an object: {:?}",
                                tasks
                            );
                        }
                    }
                }
                // -----------------------

                match serde_json::from_value(value) {
                    Ok(msg) => return Some(msg),
                    Err(e) => {
                        tracing::debug!("[AhmaTransport] Deserialization failed: {}", e);
                        continue;
                    }
                }
            }
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        // Ensure any buffered data is flushed and the writer is cleanly shutdown
        // so that outgoing notifications are delivered to the peer before
        // the transport is considered closed.
        let writer = self.writer.clone();
        let mut w = writer.lock().await;
        // Attempt to flush any buffered bytes first
        if let Err(e) = w.flush().await {
            tracing::debug!("[AhmaTransport] Flush on close failed: {}", e);
            return Err(e);
        }
        // Then try a graceful shutdown of the writer (if supported)
        if let Err(e) = AsyncWriteExt::shutdown(&mut *w).await {
            tracing::debug!("[AhmaTransport] Shutdown on close failed: {}", e);
            return Err(e);
        }
        Ok(())
    }
}
