use crate::session::{McpRoot, SessionManager, request_timeout_secs, tool_call_timeout_secs};
use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::stream::{self, StreamExt};
use serde_json::Value;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, error, info, warn};

/// MCP Session-Id header name (per MCP spec 2025-03-26)
const MCP_SESSION_ID_HEADER: &str = "mcp-session-id";

/// Create a JSON response with appropriate headers
fn json_response(value: Value) -> Response {
    json_response_with_status(StatusCode::OK, value)
}

/// Create a JSON response with the provided status.
fn json_response_with_status(status: StatusCode, value: Value) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&value).unwrap_or_default()))
        .unwrap_or_else(|_| (status, "Failed to create response").into_response())
}

/// Build a JSON-RPC error object.
fn json_rpc_error_value(code: i32, message: &str) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "error": {
            "code": code,
            "message": message
        }
    })
}

/// Attach MCP session header when available.
fn with_session_header(mut response: Response, session_id: &str) -> Response {
    let header_value = HeaderValue::from_str(session_id)
        .ok()
        .unwrap_or_else(|| HeaderValue::from_static("invalid"));
    response
        .headers_mut()
        .insert(MCP_SESSION_ID_HEADER, header_value);
    response
}

/// Create an error response with the provided status and JSON-RPC code.
fn error_response_with_status(status: StatusCode, code: i32, message: &str) -> Response {
    json_response_with_status(status, json_rpc_error_value(code, message))
}

/// Create an error response in the appropriate format
fn error_response(code: i32, message: &str) -> Response {
    error_response_with_status(StatusCode::INTERNAL_SERVER_ERROR, code, message)
}

/// Handles requests in session isolation mode.
#[tracing::instrument(skip_all, fields(method, session_id))]
pub async fn handle_session_isolated_request(
    session_manager: Arc<SessionManager>,
    headers: HeaderMap,
    payload: Value,
) -> Response {
    let session_id = headers
        .get(MCP_SESSION_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let method = payload.get("method").and_then(|m| m.as_str());

    tracing::Span::current().record("method", method.unwrap_or(""));
    tracing::Span::current().record("session_id", session_id.as_deref().unwrap_or(""));
    debug!(method = ?method, session_id = ?session_id, has_id = payload.get("id").is_some(), "Incoming MCP request");

    if method == Some("initialize") && session_id.is_none() {
        return handle_initialize(&session_manager, &payload).await;
    }
    if let Some(session_id) = session_id {
        return handle_existing_session_request(&session_manager, &session_id, method, &payload)
            .await;
    }

    debug!(
        "Request without session ID for non-initialize method: {:?}",
        method
    );
    error_response_with_status(
        StatusCode::BAD_REQUEST,
        -32600,
        "Missing Mcp-Session-Id header. Send initialize request first.",
    )
}

fn validate_initialize_payload(payload: &Value) -> Option<Response> {
    if payload
        .get("params")
        .and_then(|p| p.get("protocolVersion"))
        .and_then(|v| v.as_str())
        .is_none()
    {
        Some(error_response(
            -32602,
            "Invalid initialize params: missing params.protocolVersion",
        ))
    } else {
        None
    }
}

async fn handle_initialize_error(
    session_manager: &SessionManager,
    session_id: &str,
    error: crate::error::BridgeError,
) -> Response {
    error!(session_id = %session_id, "Failed to send initialize request: {}", error);
    let _ = session_manager
        .terminate_session(
            session_id,
            crate::session::SessionTerminationReason::ProcessCrashed,
        )
        .await;
    error_response(-32603, &format!("Failed to initialize session: {}", error))
}

/// Handles initialization requests by creating a new session.
#[tracing::instrument(skip_all, fields(session_id))]
async fn handle_initialize(session_manager: &SessionManager, payload: &Value) -> Response {
    debug!("Processing initialize request (no session ID)");

    if let Some(err_response) = validate_initialize_payload(payload) {
        return err_response;
    }

    info!("Creating new session for initialize request");
    let new_session_id = match session_manager.create_session().await {
        Ok(id) => id,
        Err(e) => {
            error!("Failed to create session: {}", e);
            return error_response(-32603, &format!("Failed to create session: {}", e));
        }
    };

    info!(session_id = %new_session_id, "Session created, forwarding initialize request");
    match session_manager
        .send_request(
            &new_session_id,
            payload,
            Some(Duration::from_secs(request_timeout_secs())),
        )
        .await
    {
        Ok(response) => with_session_header(json_response(response), &new_session_id),
        Err(e) => handle_initialize_error(session_manager, &new_session_id, e).await,
    }
}

fn check_session_exists(session_manager: &SessionManager, session_id: &str) -> Option<Response> {
    if !session_manager.session_exists(session_id) {
        warn!(session_id = %session_id, "Request for non-existent or terminated session");
        Some(error_response_with_status(
            StatusCode::FORBIDDEN,
            -32600,
            "Session not found or terminated",
        ))
    } else {
        None
    }
}

async fn handle_roots_changed_request(
    session_manager: &SessionManager,
    session_id: &str,
) -> Option<Response> {
    if let Err(e) = session_manager.handle_roots_changed(session_id).await {
        error!(session_id = %session_id, "Roots change rejected: {}", e);
        Some(error_response_with_status(
            StatusCode::FORBIDDEN,
            -32600,
            "Session terminated: roots change not allowed",
        ))
    } else {
        None
    }
}

/// Handles requests for an existing session.
async fn handle_existing_session_request(
    session_manager: &SessionManager,
    session_id: &str,
    method: Option<&str>,
    payload: &Value,
) -> Response {
    if let Some(response) = check_session_exists(session_manager, session_id) {
        return response;
    }

    if method == Some("notifications/roots/list_changed")
        && let Some(response) = handle_roots_changed_request(session_manager, session_id).await
    {
        return response;
    }

    if method == Some("tools/call")
        && let Some(response) = check_sandbox_lock(session_manager, session_id)
    {
        return response;
    }

    let is_initialized_notification = method == Some("notifications/initialized");
    if is_initialized_notification {
        debug!(session_id = %session_id, "Received notifications/initialized");
    }

    let is_client_response = is_client_response(method, payload);

    if let Some(response) = check_initialization_required(
        session_manager,
        session_id,
        method,
        is_initialized_notification,
        is_client_response,
    )
    .await
    {
        return response;
    }

    if is_client_response {
        return handle_client_response(session_manager, session_id, payload).await;
    }
    forward_request(
        session_manager,
        session_id,
        method,
        payload,
        is_initialized_notification,
    )
    .await
}

/// Checks whether MCP initialization is required and waits for it if so.
///
/// Returns `Some(Response)` if initialization timed out, `None` to proceed.
async fn check_initialization_required(
    session_manager: &SessionManager,
    session_id: &str,
    method: Option<&str>,
    is_initialized_notification: bool,
    is_client_response: bool,
) -> Option<Response> {
    if is_initialized_notification || is_client_response {
        return None;
    }

    let session = session_manager.get_session(session_id)?;
    if session.is_mcp_initialized() {
        return None;
    }

    wait_for_initialization(&session, session_id, method).await
}

fn is_client_response(method: Option<&str>, payload: &Value) -> bool {
    method.is_none()
        && payload.get("id").is_some()
        && (payload.get("result").is_some() || payload.get("error").is_some())
}

fn build_handshake_timeout_response(
    session_manager: &SessionManager,
    session_id: &str,
    elapsed_secs: u64,
    sse_connected: bool,
    mcp_initialized: bool,
) -> Response {
    let error_msg = handshake_timeout_message(
        elapsed_secs,
        sse_connected,
        mcp_initialized,
        session_manager.requires_client_roots(),
    );
    error!(session_id = %session_id, "Handshake timeout: SSE={}, initialized={}", sse_connected, mcp_initialized);
    with_session_header(
        error_response_with_status(StatusCode::GATEWAY_TIMEOUT, -32002, &error_msg),
        session_id,
    )
}

fn get_conflict_message(requires_client_roots: bool) -> &'static str {
    if requires_client_roots {
        "Sandbox initializing from client roots. This server requires roots/list from client; configure --sandbox-scope for clients without roots support."
    } else {
        "Sandbox initializing from client roots or explicit fallback scope - retry tools/call after handshake completes"
    }
}

/// Checks if the sandbox is locked for `tools/call` requests.
fn check_sandbox_lock(session_manager: &SessionManager, session_id: &str) -> Option<Response> {
    let session = session_manager.get_session(session_id)?;

    let current_state = session.current_sandbox_state();
    if current_state.is_active() {
        return None;
    }

    if let ahma_common::sandbox_state::SandboxState::Failed { error } = current_state {
        return Some(with_session_header(
            error_response_with_status(
                StatusCode::FORBIDDEN,
                -32000,
                &format!("Sandbox configuration failed: {}", error),
            ),
            session_id,
        ));
    }
    if let ahma_common::sandbox_state::SandboxState::Terminated = current_state {
        return Some(with_session_header(
            error_response_with_status(StatusCode::FORBIDDEN, -32000, "Session terminated"),
            session_id,
        ));
    }

    let (sse_connected, mcp_initialized) =
        (session.is_sse_connected(), session.is_mcp_initialized());
    debug!(session_id = %session_id, sse_connected, mcp_initialized, sandbox_locked = false, "tools/call blocked - sandbox not yet locked");

    if let Some(elapsed_secs) = session.is_handshake_timed_out() {
        return Some(build_handshake_timeout_response(
            session_manager,
            session_id,
            elapsed_secs,
            sse_connected,
            mcp_initialized,
        ));
    }

    Some(with_session_header(
        error_response_with_status(
            StatusCode::CONFLICT,
            -32001,
            get_conflict_message(session_manager.requires_client_roots()),
        ),
        session_id,
    ))
}

/// Build the detailed error message for a handshake timeout.
fn handshake_timeout_message(
    elapsed_secs: u64,
    sse_connected: bool,
    mcp_initialized: bool,
    requires_roots: bool,
) -> String {
    let roots_requirement = if requires_roots {
        "No explicit server sandbox scope is configured; client roots/list is required."
    } else {
        "Server has explicit fallback sandbox scope configured for no-roots clients."
    };

    format!(
        "Handshake timeout after {}s - sandbox not locked. \
            SSE connected: {}, MCP initialized: {}. \
            Ensure client: 1) opens SSE stream (GET /mcp with session header), \
            2) sends notifications/initialized, \
            3) responds to roots/list request over SSE. {} \
            Use --handshake-timeout-secs to adjust timeout.",
        elapsed_secs, sse_connected, mcp_initialized, roots_requirement
    )
}

/// Waits for MCP initialization before forwarding a request.
async fn wait_for_initialization(
    session: &crate::session::Session,
    session_id: &str,
    method: Option<&str>,
) -> Option<Response> {
    debug!(
        session_id = %session_id,
        method = ?method,
        "Waiting for MCP initialization before forwarding request"
    );

    let init_timeout = Duration::from_secs(30);
    let wait_result = tokio::time::timeout(init_timeout, session.wait_for_mcp_initialized()).await;

    debug!(
        session_id = %session_id,
        method = ?method,
        "Wait for MCP initialization result: {:?}",
        wait_result
    );

    if wait_result.is_err() {
        warn!(
            session_id = %session_id,
            method = ?method,
            "Timeout waiting for MCP initialization"
        );
        return Some(with_session_header(
            error_response_with_status(
                StatusCode::GATEWAY_TIMEOUT,
                -32002,
                "Timeout waiting for MCP initialization - client must send notifications/initialized first",
            ),
            session_id,
        ));
    }
    debug!(
        session_id = %session_id,
        method = ?method,
        "MCP initialized, proceeding with request"
    );
    None
}

/// Handles a client response (e.g. to `roots/list`).
async fn handle_client_response(
    session_manager: &SessionManager,
    session_id: &str,
    payload: &Value,
) -> Response {
    let response_id = payload.get("id");
    let has_result = payload.get("result").is_some();
    let has_error = payload.get("error").is_some();

    debug!(
        session_id = %session_id,
        response_id = ?response_id,
        has_result = has_result,
        has_error = has_error,
        "Received client response (SSE callback), forwarding to subprocess"
    );

    // Check if this is a roots/list response - extract roots and lock sandbox
    if let Some(result) = payload.get("result") {
        try_lock_sandbox_from_roots(session_manager, session_id, result).await;
    }

    // Always forward response to subprocess
    if let Err(e) = session_manager.send_message(session_id, payload).await {
        error!(
            session_id = %session_id,
            "Failed to forward client response: {}", e
        );
        return error_response(-32603, &format!("Failed to forward response: {}", e));
    }

    with_session_header(
        json_response_with_status(StatusCode::ACCEPTED, serde_json::json!({})),
        session_id,
    )
}

/// Returns true if the sandbox should be locked based on roots and SSE state.
fn should_lock_sandbox(
    mcp_roots: &[McpRoot],
    session_manager: &SessionManager,
    session_id: &str,
) -> bool {
    !mcp_roots.is_empty()
        || session_manager
            .get_session(session_id)
            .is_some_and(|s| s.is_sse_connected())
}

/// Attempt to lock sandbox from a `roots/list` style result payload.
async fn try_lock_sandbox_from_roots(
    session_manager: &SessionManager,
    session_id: &str,
    result: &Value,
) {
    let Some(roots) = result.get("roots").and_then(|r| r.as_array()) else {
        warn!(session_id = %session_id, "roots/list response missing 'roots' array or it is invalid: {:?}", result);
        return;
    };

    let mcp_roots: Vec<McpRoot> = roots
        .iter()
        .filter_map(|r| {
            let parsed = serde_json::from_value::<McpRoot>(r.clone());
            match &parsed {
                Ok(pr) => info!(session_id = %session_id, "Successfully parsed root: {:?}", pr),
                Err(e) => warn!(session_id = %session_id, "Failed to parse root {:?}: {}", r, e),
            }
            parsed.ok()
        })
        .collect();

    info!(session_id = %session_id, "Extracted {} valid McpRoot instances from {} raw roots", mcp_roots.len(), roots.len());

    if !should_lock_sandbox(&mcp_roots, session_manager, session_id) {
        debug!(
            session_id = %session_id,
            "Skipping sandbox lock from empty roots/list response (SSE not connected yet)"
        );
        return;
    }

    info!(
        session_id = %session_id,
        roots = ?mcp_roots,
        "Locking sandbox from roots/list response"
    );

    match session_manager.lock_sandbox(session_id, &mcp_roots).await {
        Ok(true) => {
            info!(
                session_id = %session_id,
                "Sandbox locked from first roots/list response"
            );
        }
        Ok(false) => {}
        Err(e) => {
            warn!(
                session_id = %session_id,
                "Failed to record sandbox scopes: {}", e
            );
        }
    }
}

/// Handle roots/list response side-effect: lock sandbox.
async fn handle_roots_list_response(
    session_manager: &SessionManager,
    session_id: &str,
    method: Option<&str>,
    response: &Value,
) {
    if method == Some("roots/list")
        && let Some(result) = response.get("result")
    {
        try_lock_sandbox_from_roots(session_manager, session_id, result).await;
    }
}

/// Mark the session as MCP-initialized if applicable.
async fn mark_session_initialized(
    session_manager: &SessionManager,
    session_id: &str,
    is_initialized_notification: bool,
) {
    if !is_initialized_notification {
        return;
    }
    if let Some(session) = session_manager.get_session(session_id)
        && let Err(e) = session.mark_mcp_initialized().await
    {
        warn!(
            session_id = %session_id,
            "Failed to mark MCP initialized: {}", e
        );
    }
}

/// Forwards a request to the session manager.
async fn forward_request(
    session_manager: &SessionManager,
    session_id: &str,
    method: Option<&str>,
    payload: &Value,
    is_initialized_notification: bool,
) -> Response {
    let request_timeout = if method == Some("tools/call") {
        calculate_tool_timeout(payload)
    } else {
        Duration::from_secs(request_timeout_secs())
    };

    match session_manager
        .send_request(session_id, payload, Some(request_timeout))
        .await
    {
        Ok(response) => {
            handle_roots_list_response(session_manager, session_id, method, &response).await;
            mark_session_initialized(session_manager, session_id, is_initialized_notification)
                .await;
            with_session_header(json_response(response), session_id)
        }
        Err(e) => {
            error!(session_id = %session_id, "Failed to send request: {}", e);
            error_response(-32603, &format!("Failed to send request: {}", e))
        }
    }
}

fn calculate_tool_timeout(payload: &Value) -> Duration {
    let arg_timeout_secs = payload
        .get("params")
        .and_then(|p| p.get("arguments"))
        .and_then(|a| a.get("timeout_seconds"))
        .and_then(|v| v.as_u64());

    let default_secs = tool_call_timeout_secs();
    let effective_secs = arg_timeout_secs
        .map(|v| v.min(600)) // Cap at 10 minutes
        .unwrap_or(default_secs);

    Duration::from_secs(effective_secs)
}

// ─── POST SSE streaming ──────────────────────────────────────────────

/// Build an SSE response from a single JSON value, assigning an event ID from the session.
fn sse_single_event_response(session: &crate::session::Session, value: Value) -> Response {
    let json_str = serde_json::to_string(&value).unwrap_or_default();
    let id = session.assign_event_id(&json_str);
    let event_stream = stream::once(async move {
        Ok::<_, Infallible>(Event::default().id(id.to_string()).data(json_str))
    });
    Sse::new(event_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Build an SSE error response (errors are always returned as JSON, regardless of Accept).
fn sse_error_json_response(status: StatusCode, code: i32, message: &str) -> Response {
    error_response_with_status(status, code, message)
}

/// Handles POST requests that accept `text/event-stream` (SSE) responses.
///
/// Per MCP Streamable HTTP spec, POST with `Accept: text/event-stream` returns
/// an SSE stream containing the JSON-RPC response event plus any interleaved
/// server notifications. For requests (with `id`), the stream forwards broadcast
/// events and delivers the response, then closes. For notifications (no `id`),
/// a single acknowledgment event is returned.
/// Handles requests in session isolation mode (SSE transport).
#[tracing::instrument(skip_all, fields(method, session_id))]
pub async fn handle_session_isolated_request_sse(
    session_manager: Arc<SessionManager>,
    headers: HeaderMap,
    payload: Value,
) -> Response {
    let session_id = headers
        .get(MCP_SESSION_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let method = payload.get("method").and_then(|m| m.as_str());
    let has_id = payload.get("id").is_some();

    tracing::Span::current().record("method", method.unwrap_or(""));
    tracing::Span::current().record("session_id", session_id.as_deref().unwrap_or(""));
    debug!(method = ?method, session_id = ?session_id, has_id, "Incoming MCP POST SSE request");

    // Initialize: create session, forward, return SSE with response
    if method == Some("initialize") && session_id.is_none() {
        return handle_initialize_sse(&session_manager, &payload).await;
    }

    let Some(session_id) = session_id else {
        return sse_error_json_response(
            StatusCode::BAD_REQUEST,
            -32600,
            "Missing Mcp-Session-Id header. Send initialize request first.",
        );
    };

    // Validate session exists
    if let Some(response) = check_session_exists(&session_manager, &session_id) {
        return response;
    }

    // Client responses and notifications that modify state use the same JSON path
    let is_client_response = is_client_response(method, &payload);
    if is_client_response {
        return handle_client_response(&session_manager, &session_id, &payload).await;
    }

    // Roots changed check
    if method == Some("notifications/roots/list_changed")
        && let Some(response) = handle_roots_changed_request(&session_manager, &session_id).await
    {
        return response;
    }

    // Sandbox gating for tools/call
    if method == Some("tools/call")
        && let Some(response) = check_sandbox_lock(&session_manager, &session_id)
    {
        return response;
    }

    let is_initialized_notification = method == Some("notifications/initialized");

    // Wait for MCP initialization if needed
    if let Some(response) = check_initialization_required(
        &session_manager,
        &session_id,
        method,
        is_initialized_notification,
        false,
    )
    .await
    {
        return response;
    }

    // For notifications (no id): forward and return a single SSE ack event
    if !has_id {
        return forward_notification_sse(
            &session_manager,
            &session_id,
            method,
            &payload,
            is_initialized_notification,
        )
        .await;
    }

    // For requests (has id): subscribe to broadcast, forward request, stream
    // broadcast events + response event
    forward_request_sse(
        &session_manager,
        &session_id,
        method,
        &payload,
        is_initialized_notification,
    )
    .await
}

/// Handle initialize with SSE response.
async fn handle_initialize_sse(session_manager: &SessionManager, payload: &Value) -> Response {
    if let Some(err_response) = validate_initialize_payload(payload) {
        return err_response;
    }

    let new_session_id = match session_manager.create_session().await {
        Ok(id) => id,
        Err(e) => {
            error!("Failed to create session: {}", e);
            return error_response(-32603, &format!("Failed to create session: {}", e));
        }
    };

    match session_manager
        .send_request(
            &new_session_id,
            payload,
            Some(Duration::from_secs(request_timeout_secs())),
        )
        .await
    {
        Ok(response) => {
            let session = session_manager.get_session(&new_session_id);
            let json_str = serde_json::to_string(&response).unwrap_or_default();
            let (id, event_stream) = if let Some(ref s) = session {
                let id = s.assign_event_id(&json_str);
                (id, json_str)
            } else {
                (1, json_str)
            };

            let sse_stream = stream::once(async move {
                Ok::<_, Infallible>(Event::default().id(id.to_string()).data(event_stream))
            });
            let mut resp = Sse::new(sse_stream)
                .keep_alive(KeepAlive::default())
                .into_response();
            let header_value = HeaderValue::from_str(&new_session_id)
                .unwrap_or_else(|_| HeaderValue::from_static("invalid"));
            resp.headers_mut()
                .insert(MCP_SESSION_ID_HEADER, header_value);
            resp
        }
        Err(e) => handle_initialize_error(session_manager, &new_session_id, e).await,
    }
}

/// Forward a notification (no id) and return a single SSE ack event.
async fn forward_notification_sse(
    session_manager: &SessionManager,
    session_id: &str,
    _method: Option<&str>,
    payload: &Value,
    is_initialized_notification: bool,
) -> Response {
    let request_timeout = Duration::from_secs(request_timeout_secs());

    match session_manager
        .send_request(session_id, payload, Some(request_timeout))
        .await
    {
        Ok(response) => {
            mark_session_initialized(session_manager, session_id, is_initialized_notification)
                .await;
            if let Some(session) = session_manager.get_session(session_id) {
                with_session_header(sse_single_event_response(&session, response), session_id)
            } else {
                with_session_header(json_response(response), session_id)
            }
        }
        Err(e) => {
            error!(session_id = %session_id, "Failed to forward notification: {}", e);
            error_response(-32603, &format!("Failed to send request: {}", e))
        }
    }
}

/// Forward a request (has id) and return an SSE stream with interleaved
/// broadcast events and the response event.
async fn forward_request_sse(
    session_manager: &SessionManager,
    session_id: &str,
    method: Option<&str>,
    payload: &Value,
    is_initialized_notification: bool,
) -> Response {
    let session = match session_manager.get_session(session_id) {
        Some(s) => s,
        None => {
            return error_response_with_status(
                StatusCode::FORBIDDEN,
                -32600,
                "Session not found or terminated",
            );
        }
    };

    // Subscribe to broadcast BEFORE sending the request so we don't miss events
    let rx = session.subscribe();

    let request_timeout = if method == Some("tools/call") {
        calculate_tool_timeout(payload)
    } else {
        Duration::from_secs(request_timeout_secs())
    };

    // Send the request to the subprocess
    let response_result = session_manager
        .send_request(session_id, payload, Some(request_timeout))
        .await;

    match response_result {
        Ok(response) => {
            handle_roots_list_response(session_manager, session_id, method, &response).await;
            mark_session_initialized(session_manager, session_id, is_initialized_notification)
                .await;

            // Build SSE stream: broadcast events that arrived during processing + the response
            let response_json = serde_json::to_string(&response).unwrap_or_default();
            let response_id = session.assign_event_id(&response_json);

            // Collect any broadcast events that arrived while waiting for the response
            let session_clone = session.clone();
            let sid = session_id.to_string();
            let notification_stream =
                BroadcastStream::new(rx).filter_map(move |result| {
                    let sid = sid.clone();
                    let session_ref = session_clone.clone();
                    async move {
                        match result {
                            Ok((id, msg)) => {
                                debug!(session_id = %sid, event_id = id, "POST SSE notification: {}", msg);
                                Some(Ok::<_, Infallible>(
                                    Event::default().id(id.to_string()).data(msg),
                                ))
                            }
                            Err(
                                tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(
                                    n,
                                ),
                            ) => {
                                session_ref.record_lagged_events(n);
                                Some(Ok(Event::default().comment(format!(
                                    "lagged: {} events dropped",
                                    n
                                ))))
                            }
                        }
                    }
                });

            // The response event is emitted last, then the stream closes
            let response_event = stream::once(async move {
                Ok::<_, Infallible>(
                    Event::default()
                        .id(response_id.to_string())
                        .data(response_json),
                )
            });

            // Take only notifications that arrived before the response, then emit response
            // Since we already awaited the response, any events in the broadcast channel
            // were emitted during request processing. We take a small window then close.
            let combined = notification_stream
                .take_until(tokio::time::sleep(Duration::from_millis(50)))
                .chain(response_event);

            with_session_header(
                Sse::new(combined)
                    .keep_alive(KeepAlive::default())
                    .into_response(),
                session_id,
            )
        }
        Err(e) => {
            error!(session_id = %session_id, "Failed to send request: {}", e);
            error_response(-32603, &format!("Failed to send request: {}", e))
        }
    }
}
