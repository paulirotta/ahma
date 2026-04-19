//! # Ahma Common: Shared Foundation and CI Stability
//!
//! `ahma_common` provides a collection of shared types, constants, and utilities
//! used across the entire Ahma workspace. Its primary goal is to provide a single,
//! reliable "ground truth" for cross-platform behavior and environmental scaling.
//!
//! ## Core Utilities
//!
//! - **[`timeouts`]**: The most critical component for CI stability. It provides
//!   platform-aware timeout scaling to ensure that slow CI runners (particularly
//!   Windows) don't experience intermittent failures during process spawning or
//!   network operations.
//!
//! ## Design Goal: Workspace Consistency
//!
//! By centralizing these primitives here, we ensure that both the core server and the
//! bridges behave consistently regardless of the OS they are running on.

pub mod observability;
pub mod sandbox_state;
pub mod state_machine;
pub mod timeouts;
