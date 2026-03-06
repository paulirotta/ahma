use ahma_mcp::test_utils::concurrency::wait_for_operation_terminal;
use ahma_mcp::test_utils::fs::get_workspace_path;
use ahma_mcp::utils::logging::init_test_logging;
/// Test multi-line argument handling functionality
///
/// This test verifies that the adapter correctly handles multi-line strings and special characters
/// in command arguments by either using file-based argument passing or safe shell escaping.
use ahma_mcp::{
    adapter::{Adapter, AsyncExecOptions},
    config::{CommandOption, SubcommandConfig},
    operation_monitor::{MonitorConfig, OperationMonitor},
    sandbox::Sandbox,
    shell_pool::{ShellPoolConfig, ShellPoolManager},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tempfile::tempdir;

/// Check if we can apply a new sandbox (i.e., we're not already sandboxed)
fn can_apply_sandbox() -> bool {
    // Try to run sandbox-exec with a simple command
    let result = std::process::Command::new("sandbox-exec")
        .args(["-p", "(version 1)(allow default)", "true"])
        .output();
    match result {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Skip tests that require sandbox when we're already in a nested sandbox
macro_rules! skip_if_nested_sandbox {
    () => {
        if !can_apply_sandbox() {
            eprintln!("SKIPPED: Cannot apply nested sandbox (likely running inside MCP sandbox)");
            return;
        }
    };
}

#[tokio::test]
async fn test_simple_git_commit_without_multiline() {
    skip_if_nested_sandbox!();
    init_test_logging();
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let repo_path = temp_dir.path();

    // Initialize a git repository for testing
    let init_result = std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git init");

    assert!(init_result.status.success(), "Git init failed");

    // Configure git user for testing
    std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to configure git user email");

    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to configure git user name");

    // Create a test file to commit
    std::fs::write(repo_path.join("test.txt"), "test content").expect("Failed to create test file");

    // Add the file to git
    let add_result = std::process::Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git add");

    assert!(add_result.status.success(), "Git add failed");

    // Test git commit directly to make sure it works
    let direct_commit = std::process::Command::new("git")
        .args(["commit", "-m", "test commit"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git commit directly");

    if !direct_commit.status.success() {
        eprintln!(
            "Direct commit stderr: {}",
            String::from_utf8_lossy(&direct_commit.stderr)
        );
        eprintln!(
            "Direct commit stdout: {}",
            String::from_utf8_lossy(&direct_commit.stdout)
        );
        panic!("Direct git commit failed");
    }

    println!("Direct git commit succeeded!");
}

#[tokio::test]
async fn test_multiline_argument_with_echo() {
    init_test_logging();
    let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
        Duration::from_secs(30),
    )));
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));

    // Create a sandbox with root scope
    let scopes = vec![std::path::PathBuf::from("/")];
    let sandbox =
        Arc::new(Sandbox::new(scopes, ahma_mcp::sandbox::SandboxMode::Test, false, false).unwrap());

    let adapter =
        Adapter::new(monitor.clone(), shell_pool, sandbox).expect("Failed to create adapter");

    // Create a config for echo that supports file arguments
    let echo_config = SubcommandConfig {
        name: "default".to_string(),
        description: "Echo command".to_string(),
        options: None,
        positional_args_first: None,
        positional_args: Some(vec![CommandOption {
            name: "text".to_string(),
            alias: None,
            option_type: "string".to_string(),
            description: Some("Text to echo".to_string()),
            format: None,
            items: None,
            required: Some(true),
            file_arg: None, // Echo doesn't support files, should use escaping
            file_flag: None,
        }]),
        synchronous: None,
        timeout_seconds: None,
        enabled: true,
        guidance_key: None,
        subcommand: None,

        sequence: None,

        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    };

    // Test with a multi-line string that should trigger escaping
    let multiline_text = "Line 1\nLine 2\nLine 3";

    let args = json!({
        "text": multiline_text
    });

    let result = adapter
        .execute_async_in_dir_with_options(
            "echo_test",
            "echo",
            "/tmp",
            AsyncExecOptions {
                id: Some("test_echo_multiline".to_string()),
                args: args.as_object().cloned(),
                timeout: Some(10),
                callback: None,
                subcommand_config: Some(&echo_config),
                log_monitor_config: None,
            },
        )
        .await;

    // Wait for the operation to complete
    let completed = wait_for_operation_terminal(
        &monitor,
        "test_echo_multiline",
        Duration::from_secs(5),
        Duration::from_millis(50),
    )
    .await;

    assert!(
        completed,
        "Timed out waiting for echo operation to complete"
    );

    // Check the operation result through the monitor
    let operation = monitor.get_operation("test_echo_multiline").await;
    println!("Operation result: {:?}", operation);

    // The command should succeed (proper escaping prevents shell injection)
    assert!(
        result.is_ok(),
        "Echo with multi-line text failed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_multiline_git_commit_with_real_tool() {
    skip_if_nested_sandbox!();
    init_test_logging();
    skip_if_nested_sandbox!();
    init_test_logging();
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let repo_path = temp_dir.path();

    // Initialize a git repository for testing
    let init_result = std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git init");

    assert!(init_result.status.success(), "Git init failed");

    // Configure git user for testing
    std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to configure git user email");

    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to configure git user name");

    // Create a test file to commit
    std::fs::write(repo_path.join("test.txt"), "test content").expect("Failed to create test file");

    // Add the file to git
    let add_result = std::process::Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git add");

    assert!(add_result.status.success(), "Git add failed");

    // Setup the adapter
    let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
        Duration::from_secs(30),
    )));
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));

    // Create sandbox with temp_dir as a scope
    let scopes = vec![temp_dir.path().to_path_buf(), std::env::temp_dir()];
    let sandbox =
        Arc::new(Sandbox::new(scopes, ahma_mcp::sandbox::SandboxMode::Test, false, false).unwrap());

    let adapter =
        Adapter::new(monitor.clone(), shell_pool, sandbox).expect("Failed to create adapter");

    // Load the real git tool configuration
    let git_tool_path = get_workspace_path(".ahma/git.json");
    let git_config_str =
        std::fs::read_to_string(git_tool_path).expect("Failed to read git tool config");
    let git_tool: ahma_mcp::config::ToolConfig =
        serde_json::from_str(&git_config_str).expect("Failed to parse git tool config");

    // Find the commit subcommand
    let commit_subcommand = git_tool
        .subcommand
        .as_ref()
        .and_then(|subcommands| subcommands.iter().find(|s| s.name == "commit"))
        .expect("Failed to find commit subcommand in git tool config");

    // Test with a multi-line commit message that should trigger file-based handling
    let multiline_message = "feat: implement multi-line argument handling\n\nThis commit adds support for:\n- Automatic detection of problematic arguments\n- File-based argument passing for tools that support it\n- Safe shell escaping as fallback\n\nTested with git commit -F functionality.";

    let args = json!({
        "message": multiline_message
    });

    println!("Testing git commit with multi-line message through real tool config...");

    // Execute using the adapter with the real commit subcommand config
    // Note: The adapter expects the command to include the subcommand (normally added by mcp_service.rs)
    let result = adapter
        .execute_async_in_dir_with_options(
            "git_commit",
            "git commit", // Must include subcommand name
            repo_path.to_str().unwrap(),
            AsyncExecOptions {
                id: Some("test_multiline_commit_real".to_string()),
                args: args.as_object().cloned(),
                timeout: Some(30),
                callback: None,
                subcommand_config: Some(commit_subcommand),
                log_monitor_config: None,
            },
        )
        .await;

    println!("Git commit result: {:?}", result);

    // Verify the command was executed successfully
    assert!(
        result.is_ok(),
        "Git commit with multi-line message failed: {:?}",
        result
    );

    // Wait for the async operation to complete
    let operation = monitor
        .wait_for_operation("test_multiline_commit_real")
        .await
        .expect("Operation not found");

    assert!(
        operation.state.is_terminal(),
        "Operation should be in terminal state: {:?}",
        operation.state
    );

    // Check if the operation succeeded
    if let ahma_mcp::operation_monitor::OperationStatus::Failed = operation.state {
        if let Some(result) = &operation.result {
            eprintln!("Operation failed with result: {}", result);
        }
        panic!("Git commit operation failed: {:?}", operation);
    }

    // Verify the commit was actually created
    let log_result = std::process::Command::new("git")
        .args(["log", "--oneline", "-1"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git log");

    if !log_result.status.success() {
        eprintln!(
            "Git log stderr: {}",
            String::from_utf8_lossy(&log_result.stderr)
        );
        eprintln!(
            "Git log stdout: {}",
            String::from_utf8_lossy(&log_result.stdout)
        );

        // Debug: Check if there are any files staged
        let status_result = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to run git status");
        eprintln!(
            "Git status after commit attempt: {}",
            String::from_utf8_lossy(&status_result.stdout)
        );

        panic!("Git log failed - commit was not created");
    }
    let log_output = String::from_utf8_lossy(&log_result.stdout);
    assert!(
        log_output.contains("implement multi-line argument handling"),
        "Commit message not found in git log: {}",
        log_output
    );
}

#[tokio::test]
async fn test_multiline_git_commit_message() {
    skip_if_nested_sandbox!();
    init_test_logging();
    skip_if_nested_sandbox!();
    init_test_logging();
    let temp_dir = tempdir().expect("Failed to create temp dir");
    let repo_path = temp_dir.path();

    // Initialize a git repository for testing
    let init_result = std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git init");

    assert!(init_result.status.success(), "Git init failed");

    // Configure git user for testing
    std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to configure git user email");

    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to configure git user name");

    // Create a test file to commit
    std::fs::write(repo_path.join("test.txt"), "test content").expect("Failed to create test file");

    // Add the file to git
    let add_result = std::process::Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git add");

    assert!(add_result.status.success(), "Git add failed");

    // Setup the adapter
    let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
        Duration::from_secs(30),
    )));
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));

    // Create sandbox with temp_dir as a scope
    let scopes = vec![temp_dir.path().to_path_buf(), std::env::temp_dir()];
    let sandbox =
        Arc::new(Sandbox::new(scopes, ahma_mcp::sandbox::SandboxMode::Test, false, false).unwrap());

    let adapter =
        Adapter::new(monitor.clone(), shell_pool, sandbox).expect("Failed to create adapter");

    // Create a config for git commit with file_arg support
    let commit_config = SubcommandConfig {
        name: "commit".to_string(),
        description: "Record changes to the repository".to_string(),
        options: Some(vec![CommandOption {
            name: "message".to_string(),
            alias: Some("m".to_string()),
            option_type: "string".to_string(),
            description: Some("Use the given message as the commit message".to_string()),
            format: None,
            items: None,
            required: None,
            file_arg: Some(true),
            file_flag: Some("-F".to_string()),
        }]),
        positional_args_first: None,
        positional_args: None,
        synchronous: Some(true), // Changed to async for proper operation tracking
        timeout_seconds: None,
        enabled: true,
        guidance_key: None,
        subcommand: None,

        sequence: None,

        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    };

    // Test with a multi-line commit message that should trigger file-based handling
    let multiline_message = "feat: implement multi-line argument handling\n\nThis commit adds support for:\n- Automatic detection of problematic arguments\n- File-based argument passing for tools that support it\n- Safe shell escaping as fallback\n\nTested with git commit -F functionality.";

    let args = json!({
        "message": multiline_message
    });

    // Execute the git commit command
    // Note: The adapter expects the command to include the subcommand (normally added by mcp_service.rs)
    let result = adapter
        .execute_async_in_dir_with_options(
            "git_commit",
            "git commit", // Must include subcommand name
            repo_path.to_str().unwrap(),
            AsyncExecOptions {
                id: Some("test_multiline_commit".to_string()),
                args: args.as_object().cloned(),
                timeout: Some(30),
                callback: None,
                subcommand_config: Some(&commit_config),
                log_monitor_config: None,
            },
        )
        .await;

    // Verify the command was executed successfully
    if let Err(e) = &result {
        eprintln!("Git commit failed with error: {:?}", e);
    }
    println!("Git commit result: {:?}", result);

    // Check what operations exist in the monitor
    let all_operations = monitor.get_all_active_operations().await;
    println!("All operations after execution: {:?}", all_operations);

    assert!(
        result.is_ok(),
        "Git commit with multi-line message failed: {:?}",
        result
    );

    // Wait for the operation to complete
    let completed = wait_for_operation_terminal(
        &monitor,
        "test_multiline_commit",
        Duration::from_secs(10),
        Duration::from_millis(50),
    )
    .await;

    if let Some(operation) = monitor.get_operation("test_multiline_commit").await {
        if let Some(result_data) = &operation.result {
            println!("Operation result: {:?}", result_data);
        }
        if matches!(
            operation.state,
            ahma_mcp::operation_monitor::OperationStatus::Failed
        ) {
            eprintln!("Operation failed: {:?}", operation);
        }
    }

    assert!(completed, "Timed out waiting for git commit operation");

    // Debug: Check if there are any files staged
    let status_result = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git status");
    eprintln!(
        "Git status after commit attempt: {}",
        String::from_utf8_lossy(&status_result.stdout)
    );

    // Verify the commit was actually created
    let log_result = std::process::Command::new("git")
        .args(["log", "--oneline", "-1"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git log");

    if !log_result.status.success() {
        eprintln!(
            "Git log stderr: {}",
            String::from_utf8_lossy(&log_result.stderr)
        );
        eprintln!(
            "Git log stdout: {}",
            String::from_utf8_lossy(&log_result.stdout)
        );
        panic!("Git log failed");
    }
    let log_output = String::from_utf8_lossy(&log_result.stdout);
    assert!(
        log_output.contains("implement multi-line argument handling"),
        "Commit message not found in git log: {}",
        log_output
    );
}

#[tokio::test]
async fn test_special_characters_in_arguments() {
    init_test_logging();
    let monitor = Arc::new(OperationMonitor::new(MonitorConfig::with_timeout(
        Duration::from_secs(30),
    )));
    let shell_pool = Arc::new(ShellPoolManager::new(ShellPoolConfig::default()));
    let sandbox = Arc::new(Sandbox::new_test());

    let adapter = Adapter::new(monitor, shell_pool, sandbox).expect("Failed to create adapter");

    // Test with a simple echo command that handles special characters
    let echo_config = SubcommandConfig {
        name: "echo".to_string(),
        description: "Display a line of text".to_string(),
        options: None,
        positional_args_first: None,
        positional_args: Some(vec![CommandOption {
            name: "text".to_string(),
            alias: None,
            option_type: "string".to_string(),
            description: Some("Text to display".to_string()),
            format: None,
            items: None,
            required: Some(true),
            file_arg: None, // No file support, should use escaping
            file_flag: None,
        }]),
        synchronous: None,
        timeout_seconds: None,
        enabled: true,
        guidance_key: None,
        subcommand: None,

        sequence: None,

        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    };

    // Test with text containing special characters
    let special_text = "Hello 'world' with \"quotes\" and $variables and `backticks`";

    let args = json!({
        "text": special_text
    });

    let result = adapter
        .execute_async_in_dir_with_options(
            "echo_test",
            "echo",
            "/tmp",
            AsyncExecOptions {
                id: Some("test_special_chars".to_string()),
                args: args.as_object().cloned(),
                timeout: Some(10),
                callback: None,
                subcommand_config: Some(&echo_config),
                log_monitor_config: None,
            },
        )
        .await;

    // The command should succeed (proper escaping prevents shell injection)
    assert!(
        result.is_ok(),
        "Echo with special characters failed: {:?}",
        result
    );
}

#[test]
fn test_needs_file_handling_detection() {
    init_test_logging();
    // Test the static method for detecting problematic strings
    assert!(ahma_mcp::adapter::needs_file_handling("line1\nline2"));
    assert!(ahma_mcp::adapter::needs_file_handling(
        "text with 'single quotes'"
    ));
    assert!(ahma_mcp::adapter::needs_file_handling(
        "text with \"double quotes\""
    ));
    assert!(ahma_mcp::adapter::needs_file_handling(
        "text with $variables"
    ));
    assert!(ahma_mcp::adapter::needs_file_handling(
        "text with `backticks`"
    ));
    assert!(ahma_mcp::adapter::needs_file_handling(
        "text with \\backslashes"
    ));

    // Very long strings should also use file handling
    let long_string = "a".repeat(10000);
    assert!(ahma_mcp::adapter::needs_file_handling(&long_string));

    // Normal strings should not need file handling
    assert!(!ahma_mcp::adapter::needs_file_handling("simple text"));
    assert!(!ahma_mcp::adapter::needs_file_handling("text with spaces"));
    assert!(!ahma_mcp::adapter::needs_file_handling("text-with-dashes"));
}

#[test]
fn test_shell_argument_escaping() {
    init_test_logging();
    // Test the shell escaping functionality
    assert_eq!(
        ahma_mcp::adapter::escape_shell_argument("simple"),
        "'simple'"
    );

    assert_eq!(
        ahma_mcp::adapter::escape_shell_argument("text with spaces"),
        "'text with spaces'"
    );

    // Test escaping of embedded single quotes
    assert_eq!(
        ahma_mcp::adapter::escape_shell_argument("don't break"),
        "'don'\"'\"'t break'"
    );

    assert_eq!(
        ahma_mcp::adapter::escape_shell_argument("it's a 'test'"),
        "'it'\"'\"'s a '\"'\"'test'\"'\"''"
    );
}
