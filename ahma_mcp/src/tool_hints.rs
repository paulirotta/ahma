//! Pure helpers for generating LLM-facing async operation hints.
//!
//! `preview()` populates `TOOL_HINT_TEMPLATE` with a concrete operation ID and type.
//! Pure (no async, no side effects) so it can be called from `#[test]` contexts.

/// Populate the async-operation hint template with concrete values.
pub fn preview(id: &str, operation_type: &str) -> String {
    use crate::constants::TOOL_HINT_TEMPLATE;
    TOOL_HINT_TEMPLATE
        .replace("{operation_type}", operation_type)
        .replace("{id}", id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::logging::init_test_logging;

    #[test]
    fn preview_replaces_placeholders() {
        init_test_logging();
        let out = preview("abc123", "build");
        assert!(out.contains("abc123"));
        assert!(out.contains("build"));
        assert!(!out.contains("{id}"));
        assert!(!out.contains("{operation_type}"));
    }
}
