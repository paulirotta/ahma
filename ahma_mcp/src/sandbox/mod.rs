//! # Kernel-Level Sandboxing for Secure Command Execution
//!
//! This module provides platform-specific sandboxing mechanisms to enforce strict
//! file system boundaries. The AI can freely operate within the sandbox scope but
//! has zero access outside it.
//!
//! ## Platform Support
//!
//! - **Linux**: Uses Landlock (kernel 5.13+) for kernel-level file system access control.
//! - **macOS**: Uses sandbox-exec with Seatbelt profiles for file system access control.
//! - **Windows**: Backend under development (currently fails closed in strict mode).
//!
//! ## Architecture
//!
//! The `Sandbox` struct encapsulates the security context (allowed roots, strictness, temp file policy).
//! It is passed to the `Adapter` to validate paths and wrap commands.

mod command;
pub(crate) mod core;
mod error;
#[cfg(target_os = "linux")]
mod landlock;
mod prerequisites;
mod scopes;
#[cfg(target_os = "macos")]
mod seatbelt;
mod types;
#[cfg(target_os = "windows")]
mod windows;

pub use core::Sandbox;
pub use error::SandboxError;
#[cfg(target_os = "linux")]
pub use landlock::enforce_landlock_sandbox;
pub use prerequisites::{
    check_sandbox_prerequisites, exit_with_sandbox_error, test_sandbox_exec_available,
};
pub use scopes::normalize_path_lexically;
pub use types::{SandboxMode, ScopesGuard};
#[cfg(target_os = "windows")]
pub use windows::{check_windows_sandbox_available, enforce_windows_sandbox};
