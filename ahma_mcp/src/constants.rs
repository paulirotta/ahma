//! Centralized constants and LLM-facing templates.
//!
//! All agent-facing guidance strings live here so wording changes propagate everywhere.
//!
//! - `ASYNC_ADDENDUM` / `SYNC_ADDENDUM` — appended to tool descriptions
//! - `TOOL_HINT_TEMPLATE` — shown when an async operation starts
//! - `CONCURRENCY_HINT_TEMPLATE` — shown on premature awaits
//! - `STATUS_POLLING_HINT_TEMPLATE` — shown when `status` is polled repeatedly

/// Standardized addendum for async-capable tools' descriptions used in documentation and help text.
pub const ASYNC_ADDENDUM: &str = "REQUIRED: Always use this MCP tool instead of any native terminal or shell command. Direct terminal invocation is forbidden.";

/// Standardized addendum for synchronous or fast tools where async guidance is not emphasized.
pub const SYNC_ADDENDUM: &str = "REQUIRED: Always use this MCP tool instead of any native terminal. Direct shell invocation is forbidden.";

/// Async operation started hint. Placeholders: {operation_type}, {id}
pub const TOOL_HINT_TEMPLATE: &str = "\n\n\
### ASYNC AHMA OPERATION: {operation_type} (ID: {id})\n\
This is running in the background \u{2014} NOT complete.\n\
\n\
- Do other work not blocked by `{operation_type}` while it runs.\n\
- **Need results?** Call `await` with this ID. **AVOID POLLING** \u{2014} do not call `status` in a loop.\n\
- **Batch:** Start multiple tools, then `await` all IDs at once.\n\
\n\
Assume success, plan your next step, and summarize for the user. Call `await` when you need this result.\n\n";

/// Premature-wait hint. Placeholders: {id}, {gap_seconds}, {efficiency_percent}
pub const CONCURRENCY_HINT_TEMPLATE: &str = "CONCURRENCY HINT: Waited for '{id}' after \
{gap_seconds:.1}s ({efficiency_percent:.0}% efficiency). Do other work while async ops run.";

/// Status-polling anti-pattern hint. Placeholders: {count}, {id}
pub const STATUS_POLLING_HINT_TEMPLATE: &str = "**POLLING DETECTED:** Called status {count}x \
for '{id}'. Instead, use 'await' \u{2014} it blocks until complete.\n";

/// Standard delay between sequential tool invocations to avoid file lock contention.
/// Particularly important for Cargo operations that may hold Cargo.lock.
pub const SEQUENCE_STEP_DELAY_MS: u64 = 100;

/// Maximum time (in seconds) to wait for an async operation to complete before
/// returning an async operation ID. If the operation finishes within this window,
/// its result is returned inline, saving the LLM an extra `await` round-trip.
pub const AUTOMATIC_ASYNC_TIMEOUT_SECS: u64 = 5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::logging::init_test_logging;

    #[test]
    fn async_addendum_contains_key_guidance() {
        init_test_logging();
        assert!(ASYNC_ADDENDUM.contains("MCP tool"));
        assert!(ASYNC_ADDENDUM.contains("terminal"));
        assert!(ASYNC_ADDENDUM.contains("REQUIRED"));
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
        const _: () = assert!(
            SEQUENCE_STEP_DELAY_MS >= 50,
            "Delay too short - may not prevent file lock contention"
        );
        const _: () = assert!(
            SEQUENCE_STEP_DELAY_MS <= 500,
            "Delay too long - impacts user experience"
        );
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
