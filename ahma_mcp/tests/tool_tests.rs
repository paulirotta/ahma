//! Consolidated tool-specific tests
//!
//! This module consolidates all tool-specific tests into a single test binary
//! to optimize cargo nextest test discovery time.

pub use ahma_mcp::test_utils as common;

#[path = "tool_suite/advanced_await_functionality_test.rs"]
mod advanced_await_functionality_test;

#[path = "tool_suite/android_gradlew_test.rs"]
mod android_kotlin_test;

#[path = "tool_suite/android_logcat_test.rs"]
mod android_logcat_test;

#[path = "tool_suite/await_fix_verification_test.rs"]
mod await_fix_verification_test;

#[path = "tool_suite/await_manual_verification_test.rs"]
mod await_manual_verification_test;

#[path = "tool_suite/cancel_tool_message_test.rs"]
mod cancel_tool_message_test;

#[path = "tool_suite/cancellation_reason_test.rs"]
mod cancellation_reason_test;

#[path = "tool_suite/clippy_test.rs"]
mod clippy_test;

#[path = "tool_suite/comprehensive_tool_fixes_tdd.rs"]
mod comprehensive_tool_fixes_tdd;

#[path = "tool_suite/enhanced_await_tool_test.rs"]
mod enhanced_await_tool_test;

#[path = "tool_suite/file_tools_ls_bug_test.rs"]
mod file_tools_ls_bug_test;

#[path = "tool_suite/gh_tool_expansion_test.rs"]
mod gh_tool_expansion_test;

#[path = "tool_suite/gradlew_async_test.rs"]
mod kotlin_async_test;

#[path = "tool_suite/gradlew_interactive_test.rs"]
mod kotlin_interactive_test;

#[path = "tool_suite/intelligent_await_timeout_test.rs"]
mod intelligent_await_timeout_test;

#[path = "tool_suite/intelligent_timeout_verification_test.rs"]
mod intelligent_timeout_verification_test;

#[path = "tool_suite/json_tool_test.rs"]
mod json_tool_test;

#[path = "tool_suite/ls_tool_command_construction_test.rs"]
mod ls_tool_command_construction_test;

#[path = "tool_suite/sequence_tool_test.rs"]
mod sequence_tool_test;

#[path = "tool_suite/synchronous_inheritance_test.rs"]
mod synchronous_inheritance_test;

#[path = "tool_suite/synchronous_override_test.rs"]
mod synchronous_override_test;

#[path = "tool_suite/test_sequence_tool_blocks_when_synchronous.rs"]
mod test_sequence_tool_blocks_when_synchronous;

#[path = "tool_suite/timeout_parameter_test.rs"]
mod timeout_parameter_test;

#[path = "tool_suite/tool_availability_test.rs"]
mod tool_availability_test;

#[path = "tool_suite/tool_behavior_consistency_test.rs"]
mod tool_behavior_consistency_test;

#[path = "tool_suite/tool_loading_bug_test.rs"]
mod tool_loading_bug_test;

#[path = "tool_suite/tool_validation_tdd_test.rs"]
mod tool_validation_tdd_test;

#[path = "tool_suite/comprehensive_tool_json_coverage_test.rs"]
mod comprehensive_tool_json_coverage_test;

#[path = "tool_suite/tool_execution_integration_test.rs"]
mod tool_execution_integration_test;
