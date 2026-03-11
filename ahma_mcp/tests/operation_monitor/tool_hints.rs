//! # Test: Async Operation Response Includes Tool Hints
//!
//! **Purpose:** Verify that when an asynchronous operation is started, the response
//! includes comprehensive guidance for the AI agent on how to handle it effectively.
//!
//! **Current Behavior (FAILING):** Response only says "AHMA ID: op_123"
//!
//! **Expected Behavior (SHOULD PASS AFTER FIX):** Response includes the full TOOL_HINT_TEMPLATE
//! with placeholders replaced:
//! - "AHMA ID:"
//! - The actual operation ID
//! - "Use `await` to block until operation ID(s) complete"
//! - "AVOID POLLING"
//! - Guidance on what to do while waiting
//!
//! **Why This Matters:**
//! Without these hints, AI agents don't know they should use the `await` tool.
//! They finish execution thinking the operation is complete, leading to false positives
//! where the AI reports success when tests/builds actually failed.

use ahma_mcp::tool_hints;

#[test]
fn test_tool_hints_preview_includes_all_required_elements() {
    // Test the tool_hints::preview function directly
    let hint = tool_hints::preview("op_12345", "build");

    // CRITICAL: Hint MUST include these elements
    assert!(
        hint.contains("AHMA ID:"),
        "Hint must include async operation header. Got: {}",
        hint
    );

    // Verify operation ID and type are included
    assert!(
        hint.contains("op_12345"),
        "Hint must include operation ID. Got: {}",
        hint
    );

    assert!(
        hint.contains("build"),
        "Hint must include operation type. Got: {}",
        hint
    );

    // Verify key guidance is present
    assert!(
        hint.contains("running in the background"),
        "Hint must explain operation is running in background. Got: {}",
        hint
    );

    assert!(
        hint.contains("await"),
        "Hint must mention the await tool. Got: {}",
        hint
    );

    assert!(
        hint.contains("AVOID POLLING"),
        "Hint must warn against status polling. Got: {}",
        hint
    );

    // Verify placeholders were replaced (not left as {id})
    assert!(
        !hint.contains("{id}"),
        "Placeholders must be replaced, found {{id}} in: {}",
        hint
    );

    assert!(
        !hint.contains("{operation_type}"),
        "Placeholders must be replaced, found {{operation_type}} in: {}",
        hint
    );
}

#[test]
fn test_tool_hints_preview_replacements() {
    // Test with different operation types
    let test_cases = vec![
        ("op_123", "test", vec!["op_123", "test"]),
        ("op_abc", "build", vec!["op_abc", "build"]),
        (
            "op_xyz789",
            "quality_check",
            vec!["op_xyz789", "quality_check"],
        ),
    ];

    for (op_id, op_type, expected_contents) in test_cases {
        let hint = tool_hints::preview(op_id, op_type);

        for expected in expected_contents {
            assert!(
                hint.contains(expected),
                "Hint for {} / {} must contain '{}'. Got: {}",
                op_id,
                op_type,
                expected,
                hint
            );
        }

        // Verify no placeholders remain
        assert!(
            !hint.contains("{operation"),
            "No placeholders should remain in hint for {} / {}. Got: {}",
            op_id,
            op_type,
            hint
        );
    }
}
