//! HTTP/3 (QUIC) server for the Ahma HTTP bridge.
//!
//! ## Design
//!
//! - JSON (`application/json`) request/response endpoints are served over HTTP/3.
//! - SSE (`text/event-stream`) is **not** supported over HTTP/3 — requests with
//!   `Accept: text/event-stream` receive `406 Not Acceptable`. SSE requires HTTP/2.
//! - Each QUIC connection spawns a task; each h3 stream within that connection
//!   spawns another task. The axum router is cloned cheaply and reused.
//! - TLS uses a self-signed certificate; see the `cert` sub-module.

pub mod cert;

use anyhow::Result;
use axum::Router;
use axum::http::{Response, StatusCode};
use bytes::{BufMut, Bytes, BytesMut};
use h3::server::RequestStream;
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;
use tracing::{debug, error, warn};

/// Build a `quinn::ServerConfig` from a rustls TLS configuration.
///
/// The TLS config must include the `h3` ALPN token (set by [`cert::build_quic_tls_config`]).
pub fn build_quinn_server_config(
    tls_config: Arc<rustls::ServerConfig>,
) -> Result<quinn::ServerConfig> {
    let quic_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
        .map_err(|e| anyhow::anyhow!("Failed to create QUIC crypto config: {}", e))?;
    Ok(quinn::ServerConfig::with_crypto(Arc::new(quic_crypto)))
}

/// Accept HTTP/3 connections on the given QUIC endpoint and route them through `router`.
///
/// Runs until the endpoint is closed. Individual connection and request errors are
/// logged at debug/warn level and do not terminate the accept loop.
pub async fn serve_quic(endpoint: quinn::Endpoint, router: Router) {
    while let Some(incoming) = endpoint.accept().await {
        let router = router.clone();
        tokio::spawn(async move {
            let conn = match incoming.await {
                Ok(c) => c,
                Err(e) => {
                    debug!("QUIC incoming connection rejected: {e}");
                    return;
                }
            };
            let remote = conn.remote_address();
            debug!("HTTP/3 connection accepted from {remote}");

            let h3_conn = h3_quinn::Connection::new(conn);
            let mut h3_conn = match h3::server::Connection::new(h3_conn).await {
                Ok(c) => c,
                Err(e) => {
                    warn!("HTTP/3 handshake failed from {remote}: {e}");
                    return;
                }
            };

            loop {
                match h3_conn.accept().await {
                    Ok(Some(resolver)) => {
                        let router = router.clone();
                        tokio::spawn(async move {
                            let (req, stream) = match resolver.resolve_request().await {
                                Ok(pair) => pair,
                                Err(e) => {
                                    debug!("HTTP/3 request resolve error from {remote}: {e}");
                                    return;
                                }
                            };
                            if let Err(e) = handle_h3_request(req, stream, router).await {
                                debug!("HTTP/3 request handler error from {remote}: {e}");
                            }
                        });
                    }
                    Ok(None) => {
                        debug!("HTTP/3 connection closed by {remote}");
                        break;
                    }
                    Err(e) => {
                        // All errors from accept() are connection-level in h3 0.0.8
                        debug!("HTTP/3 connection error from {remote}: {e}");
                        break;
                    }
                }
            }
        });
    }
}

/// Handle a single HTTP/3 request.
///
/// SSE requests (`Accept: text/event-stream`) receive `406 Not Acceptable`.
/// All other requests are proxied through the axum `router`.
async fn handle_h3_request<S>(
    req: axum::http::Request<()>,
    mut stream: RequestStream<S, Bytes>,
    router: Router,
) -> Result<()>
where
    S: h3::quic::SendStream<Bytes> + h3::quic::RecvStream + Unpin + Send + 'static,
{
    // Reject SSE requests — SSE requires HTTP/2.
    let wants_sse = req
        .headers()
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("text/event-stream"));

    if wants_sse {
        let resp = Response::builder()
            .status(StatusCode::NOT_ACCEPTABLE)
            .header("content-type", "text/plain; charset=utf-8")
            .body(())
            .unwrap();
        stream.send_response(resp).await?;
        stream
            .send_data(Bytes::from_static(
                b"SSE (text/event-stream) is not supported over HTTP/3 (QUIC). \
                  Use HTTP/2 for SSE.",
            ))
            .await?;
        stream.finish().await?;
        return Ok(());
    }

    // Collect the request body from h3 DATA frames.
    let mut body_bytes = BytesMut::new();
    while let Some(chunk) = stream.recv_data().await? {
        body_bytes.put(chunk);
    }

    // Reconstruct the full request with the collected body.
    let (parts, _unit) = req.into_parts();
    let axum_req =
        axum::http::Request::from_parts(parts, axum::body::Body::from(body_bytes.freeze()));

    // Call the axum router.
    let axum_resp = match router.oneshot(axum_req).await {
        Ok(r) => r,
        Err(e) => {
            error!("Axum service error handling HTTP/3 request: {e}");
            let resp = Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(())
                .unwrap();
            stream.send_response(resp).await?;
            stream.finish().await?;
            return Ok(());
        }
    };

    // Forward the response: headers first, then body.
    let (resp_parts, resp_body) = axum_resp.into_parts();
    let h3_resp = Response::from_parts(resp_parts, ());
    stream.send_response(h3_resp).await?;

    let response_bytes = resp_body
        .collect()
        .await
        .map_err(|e| anyhow::anyhow!("Response body collection failed: {e}"))?
        .to_bytes();

    if !response_bytes.is_empty() {
        stream.send_data(response_bytes).await?;
    }
    stream.finish().await?;

    Ok(())
}
