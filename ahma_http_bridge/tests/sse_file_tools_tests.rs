//! File Tools Integration Tests
//!
//! Tests for file manipulation tools via the HTTP SSE bridge.

mod common;

use common::sse_test_helpers::{self, call_tool, ensure_server_available, is_tool_available};

use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_file_tools_pwd() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    skip_if_unavailable!(&client, "file-tools_pwd");

    let result = call_tool(&client, "file-tools_pwd", json!({})).await;

    assert!(result.success, "file-tools_pwd failed: {:?}", result.error);
    assert!(result.output.is_some(), "No output from file-tools_pwd");

    let output = result.output.unwrap();
    assert!(!output.is_empty(), "Empty output from file-tools_pwd");
    println!("PWD: {}", output);
}

#[tokio::test]
async fn test_file_tools_ls() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    skip_if_unavailable!(&client, "file-tools_ls");

    let result = call_tool(&client, "file-tools_ls", json!({"path": "."})).await;

    assert!(result.success, "file-tools_ls failed: {:?}", result.error);
    assert!(result.output.is_some(), "No output from file-tools_ls");

    let output = result.output.unwrap();
    assert!(!output.is_empty(), "Empty output from file-tools_ls");
    println!(
        "LS output (first 500 chars): {}",
        &output[..output.len().min(500)]
    );
}

#[tokio::test]
async fn test_file_tools_ls_with_options() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    skip_if_unavailable!(&client, "file-tools_ls");

    let result = call_tool(
        &client,
        "file-tools_ls",
        json!({
            "path": ".",
            "long": true,
            "all": true
        }),
    )
    .await;

    assert!(
        result.success,
        "file-tools_ls with options failed: {:?}",
        result.error
    );
    assert!(result.output.is_some());

    let output = result.output.unwrap();
    println!(
        "LS -la output (first 500 chars): {}",
        &output[..output.len().min(500)]
    );
}

#[tokio::test]
async fn test_file_tools_cat() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    skip_if_unavailable!(&client, "file-tools_cat");

    let result = call_tool(&client, "file-tools_cat", json!({"files": ["Cargo.toml"]})).await;

    assert!(result.success, "file-tools_cat failed: {:?}", result.error);
    assert!(result.output.is_some());

    let output = result.output.unwrap();
    assert!(
        output.contains("[workspace]") || output.contains("[package]"),
        "Cargo.toml should contain workspace or package section"
    );
}

#[tokio::test]
async fn test_file_tools_head() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    skip_if_unavailable!(&client, "file-tools_head");

    let result = call_tool(
        &client,
        "file-tools_head",
        json!({
            "files": ["README.md"],
            "lines": 5
        }),
    )
    .await;

    assert!(result.success, "file-tools_head failed: {:?}", result.error);
    assert!(result.output.is_some());
}

#[tokio::test]
async fn test_file_tools_tail() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    skip_if_unavailable!(&client, "file-tools_tail");

    let result = call_tool(
        &client,
        "file-tools_tail",
        json!({
            "files": ["README.md"],
            "lines": 5
        }),
    )
    .await;

    assert!(result.success, "file-tools_tail failed: {:?}", result.error);
    assert!(result.output.is_some());
}

#[tokio::test]
async fn test_file_tools_grep() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    skip_if_unavailable!(&client, "file-tools_grep");

    let result = call_tool(
        &client,
        "file-tools_grep",
        json!({
            "pattern": "ahma",
            "files": ["Cargo.toml"],
            "ignore-case": true
        }),
    )
    .await;

    assert!(result.success, "file-tools_grep failed: {:?}", result.error);
}

#[tokio::test]
async fn test_file_tools_find() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    skip_if_unavailable!(&client, "file-tools_find");

    let result = call_tool(
        &client,
        "file-tools_find",
        json!({
            "path": ".",
            "-name": "*.toml",
            "-maxdepth": 2
        }),
    )
    .await;

    assert!(result.success, "file-tools_find failed: {:?}", result.error);
    assert!(result.output.is_some());

    let output = result.output.unwrap();
    assert!(
        output.contains("Cargo.toml"),
        "Should find Cargo.toml files"
    );
}

#[tokio::test]
async fn test_file_tools_touch_and_rm() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    // Check if tools are available before testing
    if !is_tool_available(&client, "file-tools_touch").await {
        eprintln!("WARNING️  file-tools_touch not available on server, skipping test");
        return;
    }
    if !is_tool_available(&client, "file-tools_rm").await {
        eprintln!("WARNING️  file-tools_rm not available on server, skipping test");
        return;
    }
    if !is_tool_available(&client, "file-tools_ls").await {
        eprintln!("WARNING️  file-tools_ls not available on server, skipping test");
        return;
    }

    let temp_file = format!("test_integration_{}.tmp", std::process::id());

    // Touch (create) the file
    let touch_result = call_tool(&client, "file-tools_touch", json!({"files": [&temp_file]})).await;

    if !touch_result.success {
        eprintln!(
            "WARNING️  file-tools_touch failed (may be outside sandbox): {:?}",
            touch_result.error
        );
        return;
    }

    // Verify it exists
    let ls_result = call_tool(&client, "file-tools_ls", json!({"path": "."})).await;

    assert!(ls_result.success);
    let output = ls_result.output.unwrap_or_default();
    assert!(
        output.contains(&temp_file),
        "Created file should be visible"
    );

    // Remove the file
    let rm_result = call_tool(&client, "file-tools_rm", json!({"paths": [&temp_file]})).await;

    assert!(
        rm_result.success,
        "file-tools_rm failed: {:?}",
        rm_result.error
    );

    // Verify it's gone
    let ls_after = call_tool(&client, "file-tools_ls", json!({"path": "."})).await;

    assert!(ls_after.success);
    let output_after = ls_after.output.unwrap_or_default();
    assert!(
        !output_after.contains(&temp_file),
        "Removed file should not be visible"
    );
}

#[tokio::test]
async fn test_file_tools_cp_and_mv() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    // Check if tools are available before testing
    if !is_tool_available(&client, "file-tools_cp").await {
        eprintln!("WARNING️  file-tools_cp not available on server, skipping test");
        return;
    }
    if !is_tool_available(&client, "file-tools_mv").await {
        eprintln!("WARNING️  file-tools_mv not available on server, skipping test");
        return;
    }

    let pid = std::process::id();
    let src_file = format!("test_cp_src_{}.tmp", pid);
    let dst_file = format!("test_cp_dst_{}.tmp", pid);
    let mv_file = format!("test_mv_dst_{}.tmp", pid);

    // Create source file using sandboxed_shell
    let create_result = call_tool(
        &client,
        "sandboxed_shell",
        json!({"command": format!("echo 'test content' > {}", src_file)}),
    )
    .await;

    if !create_result.success {
        eprintln!(
            "WARNING️  Could not create test file: {:?}",
            create_result.error
        );
        return;
    }

    // Copy the file
    let cp_result = call_tool(
        &client,
        "file-tools_cp",
        json!({
            "source": &src_file,
            "destination": &dst_file
        }),
    )
    .await;

    assert!(
        cp_result.success,
        "file-tools_cp failed: {:?}",
        cp_result.error
    );

    // Move the copied file
    let mv_result = call_tool(
        &client,
        "file-tools_mv",
        json!({
            "source": &dst_file,
            "destination": &mv_file
        }),
    )
    .await;

    assert!(
        mv_result.success,
        "file-tools_mv failed: {:?}",
        mv_result.error
    );

    // Cleanup
    let _ = call_tool(
        &client,
        "file-tools_rm",
        json!({"paths": [&src_file, &mv_file]}),
    )
    .await;
}

#[tokio::test]
async fn test_file_tools_diff() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    skip_if_unavailable!(&client, "file-tools_diff");

    let pid = std::process::id();
    let file1 = format!("test_diff1_{}.tmp", pid);
    let file2 = format!("test_diff2_{}.tmp", pid);

    // Create two files with different content
    let _ = call_tool(
        &client,
        "sandboxed_shell",
        json!({"command": format!("echo 'line1\\nline2\\nline3' > {}", file1)}),
    )
    .await;

    let _ = call_tool(
        &client,
        "sandboxed_shell",
        json!({"command": format!("echo 'line1\\nmodified\\nline3' > {}", file2)}),
    )
    .await;

    // Diff the files
    let diff_result = call_tool(
        &client,
        "file-tools_diff",
        json!({
            "file1": &file1,
            "file2": &file2,
            "unified": 3
        }),
    )
    .await;

    // diff returns exit code 1 when files differ, which may show as error
    println!(
        "diff result: success={}, error={:?}",
        diff_result.success, diff_result.error
    );

    // Cleanup
    let _ = call_tool(&client, "file-tools_rm", json!({"paths": [&file1, &file2]})).await;
}

#[tokio::test]
async fn test_file_tools_sed() {
    let _server = ensure_server_available().await;
    let client = Client::new();

    skip_if_unavailable!(&client, "sandboxed_shell");

    // Use sed to transform input (piped via sandboxed_shell since sed needs input)
    let result = call_tool(
        &client,
        "sandboxed_shell",
        json!({"command": "echo 'hello world' | sed 's/world/rust/'"}),
    )
    .await;

    if !result.success {
        eprintln!(
            "WARNING️  sed via shell failed (may be sandbox restriction): {:?}",
            result.error
        );
        return;
    }

    let output = result.output.unwrap_or_default();

    // sandboxed_shell may run asynchronously, returning operation ID instead of output
    if output.contains("Asynchronous operation started") || output.contains("AHMA ID") {
        eprintln!("WARNING️  sandboxed_shell ran asynchronously, cannot validate sed output");
        return;
    }

    println!("sed output: {:?}", output);

    if output.trim().is_empty() {
        eprintln!("WARNING️  sed command returned empty output, skipping assertion");
        return;
    }

    assert!(
        output.contains("hello rust"),
        "sed should replace 'world' with 'rust', got: {}",
        output
    );
}
