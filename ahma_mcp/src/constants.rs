//! # Standardized Constants and Templates
//!
//! This module serves as a centralized repository for constants, particularly for strings
//! and templates that are presented to the AI agent. Centralizing these helps ensure
//! consistency in the guidance and instructions provided to the agent across different
//! parts of the application.
//!
//! ## Key Constants
//!
//! - **`ASYNC_ADDENDUM`**: A standard piece of text appended to the description of any tool
//!   that supports asynchronous execution. It instructs the agent on the correct protocol
//!   for handling async operations, emphasizing the use of `await` and `ids`.
//!
//! - **`SYNC_ADDENDUM`**: A simpler addendum for tools that are synchronous or very fast,
//!   reminding the agent to use the provided MCP tools instead of raw shell commands.
//!
//! ## Guidance Templates
//!
//! These templates are used to generate dynamic, context-aware hints for the AI agent to
//! improve its performance and efficiency.
//!
//! - **`TOOL_HINT_TEMPLATE`**: A detailed message displayed when an asynchronous operation
//!   is initiated. It guides the agent on how to effectively multitask while waiting for
//!   the operation to complete, promoting non-blocking workflows.
//!
//! - **`CONCURRENCY_HINT_TEMPLATE`**: A hint triggered when the agent appears to be waiting
//!   prematurely for an operation to finish, encouraging it to perform other tasks in
//!   parallel to improve efficiency.
//!
//! - **`STATUS_POLLING_HINT_TEMPLATE`**: A message that detects when the agent is repeatedly
//!   checking the status of an operation. It suggests a more efficient pattern, such as
//!   using `await` with notifications, to avoid unnecessary polling.
//!
//! By using these standardized templates, the system can provide consistent, high-quality
//! feedback to the agent, helping it learn and adopt best practices for interacting with
//! the asynchronous tool server.

/// Standardized addendum for async-capable tools' descriptions used in documentation and help text.
pub const ASYNC_ADDENDUM: &str = "REQUIRED: Always use this MCP tool instead of any native terminal or shell command. Direct terminal invocation is forbidden.";

/// Standardized addendum for synchronous or fast tools where async guidance is not emphasized.
pub const SYNC_ADDENDUM: &str = "REQUIRED: Always use this MCP tool instead of any native terminal. Direct shell invocation is forbidden.";

/// Template for standardized tool-hint content displayed when async operations are started.
/// Placeholders:
/// - {operation_type}
/// - {id}
pub const TOOL_HINT_TEMPLATE: &str = "\n\n### ASYNC AHMA OPERATION: {operation_type} (ID: {id})\n\
1. The operation is running in the background — do not assume it is complete.\n\
2. What to do now (pick one):\n\
 - Update the plan with current achievements and list the next concrete steps.\n\
 - Do unrelated work (code, tests, documentation, user summary..) not blocked by this `{operation_type}`.\n\
 - If you’ll need these results soon, schedule a later `status` check instead of polling.\n\
 - If you have nothing else to do and need results to proceed, use `await`.\n\
3. Tips:\n\
 - **AVOID POLLING:** Do not repeatedly call `status`.\n\
 - **Use `await` instead to block until operation ID(s) complete.**\n\
 - Batch actions: start multiple concurrent tools, then await for all IDs at once.\n\
IMPORTANT: Continue planning your next step now assuming this operation completely successfully. Summarize that next step for the user. If at that point you have not received the result of this action, call the `await` tool.\n\n";

/// Template used when detecting premature waits that harm concurrency.
/// Placeholders: {id}, {gap_seconds}, {efficiency_percent}
pub const CONCURRENCY_HINT_TEMPLATE: &str = "CONCURRENCY HINT: You waited for '{id}' after only {gap_seconds:.1}s (efficiency: {efficiency_percent:.0}%). \
        Consider performing other tasks while operations run in the background.";

/// Template for the status polling detection guidance.
/// Placeholders: {count}, {id}
pub const STATUS_POLLING_HINT_TEMPLATE: &str = "**STATUS POLLING ANTI-PATTERN DETECTED:** You've called status {count} times for operation '{id}'. \
  Instead, you **must** use 'await' with the operation ID to get automatic completion notifications.\n";

/// Standard delay between sequential tool invocations to avoid file lock contention.
/// Particularly important for Cargo operations that may hold Cargo.lock.
/// This delay is used:
/// - Between steps in sequence tools (e.g., cargo qualitycheck subcommand)
/// - When spawning related commands that may conflict on shared resources
/// - In any scenario where temporal separation prevents race conditions
pub const SEQUENCE_STEP_DELAY_MS: u64 = 100;

/// Maximum time (in seconds) to wait for an async operation to complete before
/// returning an async operation ID. If the operation finishes within this window,
/// its result is returned inline, saving the LLM an extra `await` round-trip.
/// This "automatic async" behavior reduces context chatter for fast commands.
pub const AUTOMATIC_ASYNC_TIMEOUT_SECS: u64 = 5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::logging::init_test_logging;

    #[test]
    fn async_addendum_contains_key_guidance() {
        init_test_logging();
        assert!(ASYNC_ADDENDUM.contains("MCP tool"));
        assert!(ASYNC_ADDENDUM.contains("terminal")); // Key guidance to avoid direct terminal usage
        assert!(ASYNC_ADDENDUM.contains("REQUIRED")); // Commanding keyword for LLM compliance
    }

    #[test]
    fn templates_include_placeholders() {
        init_test_logging();
        assert!(TOOL_HINT_TEMPLATE.contains("{operation_type}"));
        assert!(TOOL_HINT_TEMPLATE.contains("{id}"));
        assert!(CONCURRENCY_HINT_TEMPLATE.contains("{id}"));
        assert!(STATUS_POLLING_HINT_TEMPLATE.contains("{id}"));
    }

    #[test]
    fn sequence_step_delay_is_reasonable() {
        init_test_logging();
        // Compile-time checks for the delay constant
        // These use const assertions to validate at compile time
        const _: () = assert!(
            SEQUENCE_STEP_DELAY_MS >= 50,
            "Delay too short - may not prevent file lock contention"
        );
        const _: () = assert!(
            SEQUENCE_STEP_DELAY_MS <= 500,
            "Delay too long - impacts user experience"
        );

        // Verify it's exactly 100ms as documented
        assert_eq!(
            SEQUENCE_STEP_DELAY_MS, 100,
            "Delay should be 100ms as specified"
        );
    }

    #[test]
    fn automatic_async_timeout_is_reasonable() {
        init_test_logging();
        const _: () = assert!(
            AUTOMATIC_ASYNC_TIMEOUT_SECS >= 1,
            "Automatic async timeout too short - won't catch fast commands"
        );
        const _: () = assert!(
            AUTOMATIC_ASYNC_TIMEOUT_SECS <= 30,
            "Automatic async timeout too long - defeats purpose of async"
        );

        assert_eq!(
            AUTOMATIC_ASYNC_TIMEOUT_SECS, 5,
            "Automatic async timeout should be 5 seconds as documented"
        );
    }
}
