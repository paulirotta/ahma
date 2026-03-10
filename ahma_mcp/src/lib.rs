//! # Ahma Core
//!
//! Ahma (Finnish for wolverine) is a fast and fearless engine for wrapping CLI tools for AI use.
//! It provides the foundational library that powers the Ahma server.
//!
//! ## Core Mission
//!
//! `ahma_mcp` is a universal, high-performance Model Context Protocol (MCP) server designed to
//! dynamically adapt any command-line tool for use by AI agents. Its purpose is to provide a
//! consistent, powerful, and non-blocking bridge between AI and the vast ecosystem of
//! command-line utilities.
//!
//! ## Key Functional Requirements
//!
//! - **Dynamic Tool Adaptation**: Adapt CLI tools via JSON files (MTDF). Hot-reload support.
//! - **Async-First Execution**: Background operation management via `id` and progress notifications.
//! - **Performance**: Pre-warmed shell pool (`zsh`) for 5-20ms command startup latency.
//! - **Safe Scoping**: Kernel-level sandboxing (Landlock on Linux, Seatbelt on macOS).
//! - **Selective Sync Override**: Support forcing operations to run synchronously when needed.
//!
//! ## Architecture & Core Concepts
//!
//! ### Kernel-Level Sandboxing
//!
//! Ahma implements **kernel-level sandboxing** to protect your system. The sandbox scope is set
//! once at server startup and cannot be changed during the session.
//! - **Linux (Landlock)**: Uses Landlock for FS restrictions (kernel 5.13+).
//! - **macOS (Seatbelt)**: Uses `sandbox-exec` with generated SBPL profiles.
//! - **Detection**: Ahma automatically detects nested sandboxes (e.g., inside Cursor/VS Code)
//!   and exits unless sandboxing is explicitly disabled (`--no-sandbox` / `AHMA_NO_SANDBOX=1`).
//! - **Compatibility Mode**: On unsupported Linux kernels, explicit no-sandbox mode allows
//!   startup with a warning.
//!
//! ### Async-First Workflow
//!
//! Tools execute asynchronously by default. This allows the AI agent to continue its workflow
//! while long-running operations (like builds or tests) run in the background.
//! - **`status`**: Non-blocking progress checks.
//! - **`await`**: Blocking wait for completion (use sparingly).
//!
//! ### Spec-Driven Development (SDD)
//!
//! This project follows a lightweight SDD workflow:
//! 1. **Specify**: Define "what" and "why" in a feature spec.
//! 2. **Plan**: Translate requirements into technical implementation steps.
//! 3. **Implement**: Code the solution following the approved plan.
//! 4. **Verify**: Ensure compliance via automated tests and schema sync.
//!
//! ## AI Integration Guide
//!
//! For AI agents interacting with this library or the resulting MCP server:
//! - **Tool Calls**: Prefer `sandboxed_shell` for complex pipelines.
//! - **Concurrency**: You can run multiple tools in parallel by not awaiting immediately.
//! - **Schema**: Always validate configurations against `docs/mtdf-schema.json`.
//!
//! ## Modules
//!
//! - **`adapter`**: Primary engine for executing external command-line tools.
//! - **`config`**: MTDF (Multi-Tool Definition Format) configuration models.
//! - **`mcp_service`**: Implements the `rmcp::ServerHandler` for the MCP protocol.
//! - **`operation_monitor`**: Tracks progress of background tasks.
//! - **`shell_pool`**: Reusable shell processes (`zsh`) to minimize overhead.
//! - **`sandbox`**: Security enforcement logic.
//! - **`callback_system`**: Event notification system.

// Public modules
/// Core adapter for tool execution.
pub mod adapter;
/// Progress callback system for async operations.
pub mod callback_system;
mod check_service_ext;
/// Client helpers for talking to Ahma.
pub mod client;
/// Client type helpers and compatibility flags.
pub mod client_type;
/// Tool configuration models and loaders.
pub mod config;
/// Constants used for guidance and tool hints.
pub mod constants;
/// Live log monitoring for streaming processes.
pub mod log_monitor;
/// Logging helpers for the core crate.
pub mod logging;
/// MCP callback sender integration.
pub mod mcp_callback;
/// MCP server implementation.
pub mod mcp_service;
/// Operation monitor for async tasks.
pub mod operation_monitor;
/// Path security checks for sandbox enforcement.
pub mod path_security;
/// Retry policies and helpers.
pub mod retry;
/// Sandbox configuration and enforcement.
pub mod sandbox;
/// JSON schema validation utilities.
pub mod schema_validation;
/// CLI shell entry points.
pub mod shell;
/// Shell pooling and execution.
pub mod shell_pool;
/// Terminal output helpers for callbacks.
pub mod terminal_output;
/// Tool availability checks and guidance.
pub mod tool_availability;
/// Tool hint formatting.
pub mod tool_hints;
/// Transport patching for stdio MCP.
pub mod transport_patch;
/// Shared utilities.
pub mod utils;
/// Tool configuration validation.
pub mod validation;

// Test utilities
/// Test helpers for integration and unit tests.
pub mod test_utils;

// Re-export main types for easier use
pub use adapter::Adapter;

pub use mcp_service::AhmaMcpService;
