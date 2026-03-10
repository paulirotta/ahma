# Ahma Comprehensive Compliance Report

**Generated:** 2025-01-01  
**Scope:** Full codebase audit against `SPEC.md` (R0-R19) and module-specific requirements  
**Status:** **Partial compliance** — Most requirements implemented with 2 notable mismatches requiring action

---

## Executive Summary

| Module | Status | Confirmed | Mismatches | Notes |
|--------|--------|-----------|------------|-------|
| `ahma-mcp` | WARN Partial | R1-R5, R7.1-R7.5, R15-R17, R19 | R6.9, R7.6 | Nested sandbox handling, workspace config |
| `ahma-http-bridge` | WARN Partial | R8.1-R8.4.6, R8.4.8+ | R8.4.7 | Missing HTTP DELETE endpoint |
| `ahma-http-mcp-client` | COMPLIANT | R9.3-R9.8 | None | OAuth PKCE, token persistence |
| `ahma-mcp --validate` | COMPLIANT | R5.2, R6.4 | None | MTDF validation (consolidated into ahma-mcp) |
| `generate-tool-schema` | COMPLIANT | R6.5 | None | Schema generation |

---

# Module: ahma-mcp

## OK Requirements Confirmed

### R1: Configuration-Driven Tools & Hot-Reloading

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R1.1 | JSON tool definitions | OK | `ahma-mcp/src/config.rs` defines `ToolConfig`, `SubcommandConfig` |
| R1.2 | Directory scanning | OK | `load_tool_configs()` in `config.rs` |
| R1.3 | Startup validation | OK | `ahma-mcp/src/schema_validation.rs` `MtdfValidator` |
| R1.4 | Hot-reload + notification | OK | `mcp_service/mod.rs:202-259` `start_config_watcher()` uses `notify` crate, calls `notify_tool_list_changed()` at line 191 |

**Evidence excerpt:**

```191:198:ahma-mcp/src/mcp_service/mod.rs
        if let Some(peer) = peer_opt {
            if let Err(e) = peer.notify_tool_list_changed().await {
                tracing::error!("Failed to send tools/list_changed notification: {}", e);
            } else {
                tracing::info!("Sent tools/list_changed notification to client");
            }
```

---

### R2/R3: Async-First Architecture

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R2.1 | Async by default | OK | `mcp_service/mod.rs:1420-1443` defaults to `AsyncResultPush` mode |
| R2.2 | Operation ID return | OK | `adapter.rs:421` generates `op_<id>` pattern |
| R2.3 | Progress notifications | OK | `mcp_callback.rs` implements `CallbackSender` |
| R3.1 | Sync override | OK | `--sync` CLI flag or `synchronous: true` in config |

**Evidence excerpt (async default):**

```1437:1443:ahma-mcp/src/mcp_service/mod.rs
            } else {
                // Default to ASYNCHRONOUS mode
                crate::adapter::ExecutionMode::AsyncResultPush
            };
```

---

### R4: Performance (Shell Pool)

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R4.1 | Pre-warmed shell pool | OK | `ahma-mcp/src/shell_pool.rs` `ShellPoolManager` |
| R4.2 | Background replenishment | OK | `start_background_tasks()` in shell_pool.rs |

---

### R5: JSON Schema and Validation (MTDF)

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R5.1 | Schema generation | OK | `schemars::schema_for!(ToolConfig)` in generate-tool-schema |
| R5.2 | Startup validation | OK | `MtdfValidator::validate_tool_config()` |
| R5.3 | `format: "path"` support | OK | `adapter.rs:937-953` validates paths with `path_security::validate_path()` |
| R5.4 | `file_arg`/`file_flag` | OK | `adapter.rs:871-897` creates temp files for multi-line args |

---

### R7: Security First - Kernel-Enforced Sandboxing

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R7.1 | Landlock (Linux) | OK | `sandbox.rs:634-694` `enforce_landlock_sandbox()` |
| R7.2 | Seatbelt (macOS) | OK | `sandbox.rs:524-622` `generate_seatbelt_profile()` |
| R7.3 | Scope immutability | OK | `sandbox.rs:170-173` uses `OnceLock` for `SANDBOX_SCOPES` |
| R7.4 | Path validation | OK | `sandbox.rs:201-237` `validate_path_in_sandbox()` |
| R7.5 | Sandbox prerequisite check | OK | `sandbox.rs:268-285` `check_sandbox_prerequisites()` |

---

### R15: Unified Shell Output (2>&1)

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R15.1 | stderr→stdout redirect | OK | `adapter.rs:836-850` `ensure_shell_redirect()` appends `2>&1` |

**Evidence excerpt:**

```836:850:ahma-mcp/src/adapter.rs
    fn ensure_shell_redirect(script: &mut String) {
        if script.trim_end().ends_with("2>&1") {
            return;
        }
        // ... adds 2>&1
        script.push_str("2>&1");
    }
```

**Test coverage:**

```1091:1102:ahma-mcp/src/adapter.rs
    #[tokio::test]
    async fn shell_commands_append_redirect_once() {
        // ... asserts 2>&1 is appended
        assert_eq!(args_vec, vec!["-c".to_string(), "echo hi 2>&1".to_string()]);
    }
```

---

### R16: Logging Configuration

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R16.1 | File logging default | OK | `logging.rs` uses `tracing-appender` |
| R16.2 | `--log-to-stderr` option | OK | `main_logic.rs:171` CLI arg |

---

### R17: MCP Callback Notifications

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R17.1 | Progress notifications | OK | `mcp_callback.rs` `McpCallbackSender` |
| R17.2 | Client-type detection | OK | `client_type.rs` `McpClientType::from_peer()` |
| R17.3 | Progress token handling | OK | `mcp_service/mod.rs:1473-1481` checks for `progressToken` |

---

### R19: Protocol Stability & Cancellation Handling

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R19.1 | MCP cancellation handling | OK | `mcp_service/mod.rs:881-963` `on_cancelled()` |
| R19.2 | Background op filtering | OK | Lines 909-915 filter out `await`/`status`/`cancel` tools |

**Evidence excerpt:**

```909:915:ahma-mcp/src/mcp_service/mod.rs
                    let background_ops: Vec<_> = active_ops
                        .iter()
                        .filter(|op| {
                            // Only cancel operations that represent actual background processes
                            // NOT synchronous tools like 'await', 'status', 'cancel'
                            !matches!(op.tool_name.as_str(), "await" | "status" | "cancel")
                        })
```

---

## WARN Mismatches Found

### R7.6: Nested Sandbox Environments — MISMATCH

**Requirement (R7.6.1-R7.6.3):**
> "The system **must** detect when it is running inside another sandbox... Upon detection, the system **must** exit with a clear error message..."

**Actual Implementation:**

- Detection: OK `sandbox.rs:368-400` `test_sandbox_exec_available()` correctly detects nested sandbox
- Handling: FAIL `main_logic.rs:231-254` logs warnings and **continues** instead of exiting

**Code Evidence:**

```231:254:ahma-mcp/src/shell/main_logic.rs
        #[cfg(target_os = "macos")]
        {
            if let Err(e) = sandbox::test_sandbox_exec_available() {
                match e {
                    SandboxError::NestedSandboxDetected => {
                        tracing::warn!(
                            "Nested sandbox detected - Ahma is running inside another sandbox (e.g., Cursor IDE)"
                        );
                        tracing::warn!(
                            "Ahma's sandbox will be disabled; the outer sandbox provides security"
                        );
                        tracing::info!(
                            "To suppress this warning, use --no-sandbox or set AHMA_NO_SANDBOX=1"
                        );
                        sandbox::enable_test_mode();
                        no_sandbox = true;
                    }
                    _ => {
                        // Other sandbox errors should be fatal
                        sandbox::exit_with_sandbox_error(&e);
                    }
                }
            }
        }
```

**Impact:** Security-sensitive variance. Requirement demands fail-secure exit; implementation prefers graceful degradation.

**Recommended Fix:** Change to call `sandbox::exit_with_sandbox_error(&e)` for `NestedSandboxDetected` instead of continuing with disabled sandbox.

---

### R6.9: Workspace `default-members` — MISMATCH

**Requirement (R6.9):**
> "The root `Cargo.toml` **must** define `default-members = ["ahma_shell"]` so that `cargo run` executes the main MCP server binary by default."

**Actual State:**

```1:9:Cargo.toml
[workspace]
default-members = [
  "ahma-mcp",

  "generate-tool-schema",
  "ahma-http-bridge",
  "ahma-http-mcp-client",
]
members = ["ahma-mcp", "generate-tool-schema", "ahma-http-bridge", "ahma-http-mcp-client"]
```

- No `ahma_shell` crate exists
- Binary is `[[bin]] name = "ahma-mcp"` in `ahma-mcp/Cargo.toml:22-24`
- `default-members` lists all crates, not just the shell

**Impact:** Developer UX mismatch. `cargo run` behavior differs from spec.

**Recommended Fix:** Either:

1. Update SPEC.md to reflect current architecture (binary in ahma-mcp), or
2. Create `ahma_shell` crate and set `default-members = ["ahma_shell"]`

---

# Module: ahma-http-bridge

## OK Requirements Confirmed

### R8: HTTP Bridge

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R8.1 | HTTP/SSE transport | OK | `bridge.rs:81-83` POST/GET `/mcp` routes |
| R8.2 | Content negotiation | OK | JSON response handling in `json_response()` |
| R8.3 | Debug colored output | OK | `session.rs:788-825` colored STDIN/STDOUT/STDERR |
| R8.4.1 | Session creation on initialize | OK | `bridge.rs:283-343` creates session on first `initialize` |
| R8.4.2 | `Mcp-Session-Id` header | OK | `bridge.rs:59` constant, lines 317-322 set in response |
| R8.4.3 | Session routing | OK | `bridge.rs:344-365` routes by session ID |
| R8.4.4 | Sandbox from roots/list | OK | `bridge.rs:539-579` locks sandbox from roots response |
| R8.4.5 | Sandbox immutability | OK | `session.rs:650-651` checks `sandbox_locked` before modification |
| R8.4.6 | Roots change rejection | OK | `session.rs:694-726` `handle_roots_changed()` terminates session |

### Session Handshake State Machine

| State | Description | Evidence |
|-------|-------------|----------|
| `AwaitingBoth` | Initial state | `session.rs:47` |
| `AwaitingSseOnly` | SSE connected first | `session.rs:48` |
| `AwaitingMcpOnly` | MCP initialized first | `session.rs:50` |
| `RootsRequested` | Both ready, roots/list_changed sent | `session.rs:52` |
| `Complete` | Sandbox locked | `session.rs:54` |

**Handshake tests:** `tests/handshake_state_machine_test.rs`, `tests/handshake_timeout_test.rs`

---

## WARN Mismatch Found

### R8.4.7: HTTP DELETE Session Termination — MISSING

**Requirement (R8.4.7):**
> "HTTP DELETE with `Mcp-Session-Id` terminates session and subprocess."

**Actual State:**

- Programmatic API exists: `session.rs:729-754` `terminate_session()`
- No HTTP DELETE route registered in `bridge.rs`

**Router Evidence:**

```81:86:ahma-http-bridge/src/bridge.rs
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/mcp", post(handle_mcp_request).get(handle_sse_stream))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);
```

**Impact:** External clients cannot terminate sessions via HTTP DELETE as specified.

**Recommended Fix:** Add DELETE handler:

```rust
.route("/mcp", post(handle_mcp_request).get(handle_sse_stream).delete(handle_session_delete))
```

With handler:

```rust
async fn handle_session_delete(
    State(state): State<Arc<BridgeState>>,
    headers: HeaderMap,
) -> Response {
    let session_id = headers.get(MCP_SESSION_ID_HEADER)...;
    match state.session_manager.terminate_session(&session_id, ClientRequested).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
```

---

# Module: ahma-http-mcp-client

## OK Fully Compliant

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R9.3 | OAuth PKCE flow | OK | `client.rs:112-178` `perform_oauth_flow()` with `PkceCodeChallenge` |
| R9.4 | Token persistence | OK | `client.rs:304-334` `save_token()`, `load_token()` |
| R9.5 | Token path override | OK | `client.rs:329-334` `AHMA_HTTP_CLIENT_TOKEN_PATH` env var |
| R9.8 | Transport implementation | OK | `client.rs:222-294` `impl Transport<RoleClient>` |

**Test coverage:** Token round-trip, env override, minimal fields — `client.rs:336-490`

---

# Module: ahma-mcp --validate (formerly ahma-validate)

## OK Fully Compliant

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R5.2/R6.4 | Validation mode | OK | `ahma_mcp/src/validation.rs` uses `MtdfValidator` |
| Exit codes | 0 valid, non-zero invalid | OK | `cli.rs` returns `Err` on validation failure |
| `--debug` | Debug logging | OK | Uses global `--debug` flag from ahma-mcp CLI |
| Multiple targets | Comma-separated support | OK | `validation.rs` splits by comma |

**Test coverage:** 12 unit tests in `validation.rs` + integration tests in `cli_binary_integration_test.rs`

---

# Module: generate-tool-schema

## OK Fully Compliant

| Req | Description | Status | Evidence |
|-----|-------------|--------|----------|
| R6.5 | Schema generation | OK | `main.rs:14-17` `schemars::schema_for!(ToolConfig)` |
| File output | `mtdf-schema.json` | OK | `main.rs:39` joins with output dir |
| Preview | First N lines | OK | `main.rs:46-63` `generate_preview()` |

**Test coverage:** 12 tests covering generation, file writing, preview — `main.rs:87-262`

---

# Suggested Tests to Add

## 1. R7.6 Nested Sandbox Exit Test

**Target:** `ahma-mcp/tests/nested_sandbox_exit_test.rs`

```rust
#[test]
#[cfg(target_os = "macos")]
fn test_nested_sandbox_detection_exits_with_error() {
    // Spawn ahma-mcp in a simulated nested sandbox environment
    // Assert: process exits with non-zero code
    // Assert: stderr contains "SECURITY ERROR" or "nested sandbox"
}
```

## 2. R8.4.7 HTTP DELETE Session Test

**Target:** `ahma-http-bridge/tests/session_delete_test.rs`

```rust
#[tokio::test]
async fn test_delete_session_terminates_subprocess() {
    // 1. Create session via POST /mcp with initialize
    // 2. Send DELETE /mcp with Mcp-Session-Id header
    // 3. Assert: 204 No Content
    // 4. Assert: subsequent requests return 404
}
```

---

# Requirements Coverage Matrix

| Requirement | ahma-mcp | ahma-http-bridge | ahma-http-mcp-client | generate-tool-schema |
|-------------|-----------|------------------|----------------------|---------------------|
| R0 (Terminology) | OK | OK | OK | OK |
| R1 (Config/Hot-reload) | OK | — | — | — |
| R2 (Async-first) | OK | — | — | — |
| R3 (Sync override) | OK | — | — | — |
| R4 (Shell pool) | OK | — | — | — |
| R5 (MTDF validation) | OK (incl. --validate) | — | — | OK |
| R6 (Modular arch) | WARN R6.9 | OK | OK | OK |
| R7 (Sandbox) | WARN R7.6 | OK | — | — |
| R8 (HTTP bridge) | — | WARN R8.4.7 | — | — |
| R9 (OAuth) | — | — | OK | — |
| R10 (Meta-params) | OK | — | — | — |
| R11 (Dependencies) | OK | OK | OK | OK |
| R12 (Error handling) | OK | OK | OK | OK |
| R13 (Testing) | OK | OK | OK | OK |
| R15 (Unified output) | OK | — | — | — |
| R16 (Logging) | OK | — | — | — |
| R17 (Callbacks) | OK | — | — | — | — |
| R19 (Cancellation) | OK | — | — | — | — |

**Legend:** OK Confirmed | WARN Mismatch | — Not applicable

---

# Summary of Action Items

| Priority | Issue | Location | Recommended Fix |
|----------|-------|----------|-----------------|
| **HIGH** | R7.6 Nested sandbox should exit | `main_logic.rs:231-254` | Change to `exit_with_sandbox_error()` |
| **MEDIUM** | R8.4.7 Missing DELETE endpoint | `bridge.rs:81-83` | Add `.delete(handle_session_delete)` |
| **LOW** | R6.9 Workspace default-members | `Cargo.toml:1-8` | Update SPEC.md or restructure |

---

*End of Compliance Report*
