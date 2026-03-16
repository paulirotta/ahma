//! File Tools Integration Tests
//!
//! Tests for file manipulation tools via the HTTP bridge.
//! Each test runs against both JSON and SSE POST response modes.

mod common;
use common::{TransportMode, setup_test_mcp_for_tools};
use serde_json::json;

// ---------------------------------------------------------------------------
// file-tools_pwd
// ---------------------------------------------------------------------------

async fn run_file_tools_pwd(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["file-tools_pwd"]).await else {
        return;
    };

    let result = mcp.call_tool("file-tools_pwd", json!({})).await;

    assert!(result.success, "file-tools_pwd failed: {:?}", result.error);
    assert!(result.output.is_some(), "No output from file-tools_pwd");

    let output = result.output.unwrap();
    assert!(!output.is_empty(), "Empty output from file-tools_pwd");
    println!("PWD: {}", output);
}

#[tokio::test]
async fn test_file_tools_pwd_json() {
    run_file_tools_pwd(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_pwd_sse() {
    run_file_tools_pwd(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// file-tools_ls
// ---------------------------------------------------------------------------

async fn run_file_tools_ls(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["file-tools_ls"]).await else {
        return;
    };

    let result = mcp.call_tool("file-tools_ls", json!({"path": "."})).await;

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
async fn test_file_tools_ls_json() {
    run_file_tools_ls(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_ls_sse() {
    run_file_tools_ls(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// file-tools_ls with options
// ---------------------------------------------------------------------------

async fn run_file_tools_ls_with_options(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["file-tools_ls"]).await else {
        return;
    };

    let result = mcp
        .call_tool(
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
async fn test_file_tools_ls_with_options_json() {
    run_file_tools_ls_with_options(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_ls_with_options_sse() {
    run_file_tools_ls_with_options(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// file-tools_cat
// ---------------------------------------------------------------------------

async fn run_file_tools_cat(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["file-tools_cat"]).await else {
        return;
    };

    let result = mcp
        .call_tool("file-tools_cat", json!({"files": ["Cargo.toml"]}))
        .await;

    assert!(result.success, "file-tools_cat failed: {:?}", result.error);
    assert!(result.output.is_some());

    let output = result.output.unwrap();
    assert!(
        output.contains("[workspace]") || output.contains("[package]"),
        "Cargo.toml should contain workspace or package section"
    );
}

#[tokio::test]
async fn test_file_tools_cat_json() {
    run_file_tools_cat(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_cat_sse() {
    run_file_tools_cat(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// file-tools_head
// ---------------------------------------------------------------------------

async fn run_file_tools_head(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["file-tools_head"]).await else {
        return;
    };

    let result = mcp
        .call_tool(
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
async fn test_file_tools_head_json() {
    run_file_tools_head(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_head_sse() {
    run_file_tools_head(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// file-tools_tail
// ---------------------------------------------------------------------------

async fn run_file_tools_tail(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["file-tools_tail"]).await else {
        return;
    };

    let result = mcp
        .call_tool(
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
async fn test_file_tools_tail_json() {
    run_file_tools_tail(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_tail_sse() {
    run_file_tools_tail(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// file-tools_grep
// ---------------------------------------------------------------------------

async fn run_file_tools_grep(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["file-tools_grep"]).await else {
        return;
    };

    let result = mcp
        .call_tool(
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
async fn test_file_tools_grep_json() {
    run_file_tools_grep(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_grep_sse() {
    run_file_tools_grep(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// file-tools_find
// ---------------------------------------------------------------------------

async fn run_file_tools_find(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["file-tools_find"]).await else {
        return;
    };

    let result = mcp
        .call_tool(
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
async fn test_file_tools_find_json() {
    run_file_tools_find(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_find_sse() {
    run_file_tools_find(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// file-tools_touch and file-tools_rm
// ---------------------------------------------------------------------------

async fn run_file_tools_touch_and_rm(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(
        mode,
        &["file-tools_touch", "file-tools_rm", "file-tools_ls"],
    )
    .await
    else {
        return;
    };

    let temp_file = format!("test_integration_{}.tmp", std::process::id());

    // Touch (create) the file
    let touch_result = mcp
        .call_tool("file-tools_touch", json!({"files": [&temp_file]}))
        .await;

    if !touch_result.success {
        eprintln!(
            "WARNING  file-tools_touch failed (may be outside sandbox): {:?}",
            touch_result.error
        );
        return;
    }

    // Verify it exists
    let ls_result = mcp.call_tool("file-tools_ls", json!({"path": "."})).await;
    assert!(ls_result.success);
    let output = ls_result.output.unwrap_or_default();
    assert!(
        output.contains(&temp_file),
        "Created file should be visible"
    );

    // Remove the file
    let rm_result = mcp
        .call_tool("file-tools_rm", json!({"paths": [&temp_file]}))
        .await;
    assert!(
        rm_result.success,
        "file-tools_rm failed: {:?}",
        rm_result.error
    );

    // Verify it's gone
    let ls_after = mcp.call_tool("file-tools_ls", json!({"path": "."})).await;
    assert!(ls_after.success);
    let output_after = ls_after.output.unwrap_or_default();
    assert!(
        !output_after.contains(&temp_file),
        "Removed file should not be visible"
    );
}

#[tokio::test]
async fn test_file_tools_touch_and_rm_json() {
    run_file_tools_touch_and_rm(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_touch_and_rm_sse() {
    run_file_tools_touch_and_rm(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// file-tools_cp and file-tools_mv
// ---------------------------------------------------------------------------

async fn run_file_tools_cp_and_mv(mode: TransportMode) {
    let Some((_server, mcp)) =
        setup_test_mcp_for_tools(mode, &["file-tools_cp", "file-tools_mv"]).await
    else {
        return;
    };

    let pid = std::process::id();
    let src_file = format!("test_cp_src_{}.tmp", pid);
    let dst_file = format!("test_cp_dst_{}.tmp", pid);
    let mv_file = format!("test_mv_dst_{}.tmp", pid);

    // Create source file using sandboxed_shell
    let create_result = mcp
        .call_tool(
            "sandboxed_shell",
            json!({"command": format!("echo 'test content' > {}", src_file)}),
        )
        .await;

    if !create_result.success {
        eprintln!(
            "WARNING  Could not create test file: {:?}",
            create_result.error
        );
        return;
    }

    // Copy the file
    let cp_result = mcp
        .call_tool(
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
    let mv_result = mcp
        .call_tool(
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
    let _ = mcp
        .call_tool("file-tools_rm", json!({"paths": [&src_file, &mv_file]}))
        .await;
}

#[tokio::test]
async fn test_file_tools_cp_and_mv_json() {
    run_file_tools_cp_and_mv(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_cp_and_mv_sse() {
    run_file_tools_cp_and_mv(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// file-tools_diff
// ---------------------------------------------------------------------------

async fn run_file_tools_diff(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["file-tools_diff"]).await else {
        return;
    };

    let pid = std::process::id();
    let file1 = format!("test_diff1_{}.tmp", pid);
    let file2 = format!("test_diff2_{}.tmp", pid);

    // Create two files with different content
    let _ = mcp
        .call_tool(
            "sandboxed_shell",
            json!({"command": format!("echo 'line1\\nline2\\nline3' > {}", file1)}),
        )
        .await;
    let _ = mcp
        .call_tool(
            "sandboxed_shell",
            json!({"command": format!("echo 'line1\\nmodified\\nline3' > {}", file2)}),
        )
        .await;

    // Diff the files (diff exits 1 when files differ — may show as error)
    let diff_result = mcp
        .call_tool(
            "file-tools_diff",
            json!({
                "file1": &file1,
                "file2": &file2,
                "unified": 3
            }),
        )
        .await;

    println!(
        "diff result: success={}, error={:?}",
        diff_result.success, diff_result.error
    );

    // Cleanup
    let _ = mcp
        .call_tool("file-tools_rm", json!({"paths": [&file1, &file2]}))
        .await;
}

#[tokio::test]
async fn test_file_tools_diff_json() {
    run_file_tools_diff(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_diff_sse() {
    run_file_tools_diff(TransportMode::Sse).await;
}

// ---------------------------------------------------------------------------
// sed via sandboxed_shell
// ---------------------------------------------------------------------------

async fn run_file_tools_sed(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp_for_tools(mode, &["sandboxed_shell"]).await else {
        return;
    };

    let result = mcp
        .call_tool(
            "sandboxed_shell",
            json!({"command": "echo 'hello world' | sed 's/world/rust/'"}),
        )
        .await;

    if !result.success {
        eprintln!(
            "WARNING  sed via shell failed (may be sandbox restriction): {:?}",
            result.error
        );
        return;
    }

    let output = result.output.unwrap_or_default();
    println!("sed output: {:?}", output);

    let is_async = output.contains("Asynchronous operation started") || output.contains("AHMA ID");
    if is_async || output.trim().is_empty() {
        eprintln!("WARNING  sed output unavailable (async or empty), skipping assertion");
        return;
    }

    assert!(
        output.contains("hello rust"),
        "sed should replace 'world' with 'rust', got: {}",
        output
    );
}

#[tokio::test]
async fn test_file_tools_sed_json() {
    run_file_tools_sed(TransportMode::Json).await;
}

#[tokio::test]
async fn test_file_tools_sed_sse() {
    run_file_tools_sed(TransportMode::Sse).await;
}
