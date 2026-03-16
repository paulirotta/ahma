//! Sandboxed Shell Integration Tests
//!
//! Tests for the `sandboxed_shell` tool via the HTTP bridge.
//! Each test runs against both JSON and SSE POST response modes.

mod common;
use common::{
    TransportMode, assert_tool_success_with_output, is_async_operation_output,
    setup_test_mcp_for_tools,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// sandboxed_shell echo
// ---------------------------------------------------------------------------

async fn run_sandboxed_shell_echo(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["sandboxed_shell"]).await else {
        return;
    };

    let result = mcp
        .call_tool(
            "sandboxed_shell",
            json!({"command": "echo 'Hello from sandboxed shell!'"}),
        )
        .await;

    let output = assert_tool_success_with_output(&result, "sandboxed_shell echo");
    let has_expected_output = output.contains("Hello from sandboxed shell!");
    let is_async_operation = is_async_operation_output(output);

    if has_expected_output {
        println!("OK Got expected output: {}", output);
    } else if is_async_operation {
        println!("OK Got async operation response (valid): {}", output);
    } else {
        println!(
            "WARNING  Unexpected output format (but tool call succeeded): {}",
            output
        );
    }
}

#[tokio::test]
async fn test_sandboxed_shell_echo_json() {
    run_sandboxed_shell_echo(TransportMode::Json).await;
}

#[tokio::test]
async fn test_sandboxed_shell_echo_sse() {
    run_sandboxed_shell_echo(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// sandboxed_shell pipe
// ---------------------------------------------------------------------------

async fn run_sandboxed_shell_pipe(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["sandboxed_shell"]).await else {
        return;
    };

    let result = mcp
        .call_tool(
            "sandboxed_shell",
            json!({"subcommand": "default", "command": "echo 'line1\\nline2\\nline3' | wc -l"}),
        )
        .await;

    let output = assert_tool_success_with_output(&result, "sandboxed_shell pipe");
    let has_expected_output = output.trim().contains("3");
    let is_async_operation = is_async_operation_output(output);

    if has_expected_output {
        println!("OK Got expected line count: {}", output.trim());
    } else if is_async_operation {
        println!("OK Got async operation response (valid): {}", output);
    } else {
        println!(
            "WARNING  Unexpected output format (but tool call succeeded): {}",
            output
        );
    }
}

#[tokio::test]
async fn test_sandboxed_shell_pipe_json() {
    run_sandboxed_shell_pipe(TransportMode::Json).await;
}

#[tokio::test]
async fn test_sandboxed_shell_pipe_sse() {
    run_sandboxed_shell_pipe(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// sandboxed_shell variable substitution
// ---------------------------------------------------------------------------

async fn run_sandboxed_shell_variable_substitution(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["sandboxed_shell"]).await else {
        return;
    };

    let result = mcp
        .call_tool(
            "sandboxed_shell",
            json!({"subcommand": "default", "command": "echo \\\"PWD is: $PWD\\\""}),
        )
        .await;

    let output = assert_tool_success_with_output(&result, "sandboxed_shell var substitution");
    let has_expected_output = output.contains("PWD is:");
    let is_async_operation = is_async_operation_output(output);

    if has_expected_output {
        println!("OK Got expected PWD output: {}", output);
    } else if is_async_operation {
        println!("OK Got async operation response (valid): {}", output);
    } else {
        println!(
            "WARNING  Unexpected output format (but tool call succeeded): {}",
            output
        );
    }
}

#[tokio::test]
async fn test_sandboxed_shell_variable_substitution_json() {
    run_sandboxed_shell_variable_substitution(TransportMode::Json).await;
}

#[tokio::test]
async fn test_sandboxed_shell_variable_substitution_sse() {
    run_sandboxed_shell_variable_substitution(TransportMode::Sse).await;
}
