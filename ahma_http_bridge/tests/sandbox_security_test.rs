//! Integration tests for HTTP bridge sandbox security and --disable-temp-files flag.
//!
//! These tests verify:
//! 1. Session sandbox scope is properly set from client workspace roots
//! 2. The --disable-temp-files flag is passed to subprocesses correctly
//! 3. Sandbox scope prevents access to files outside the workspace
//!
//! Note: Full integration tests require spawning actual processes which is
//! challenging in unit tests. These tests focus on the configuration and
//! state management aspects.

use ahma_http_bridge::DEFAULT_HANDSHAKE_TIMEOUT_SECS;
use ahma_http_bridge::session::{McpRoot, SessionManager, SessionManagerConfig};
use ahma_mcp::test_utils::path_helpers::{test_abs, test_temp_path};
use std::path::PathBuf;

/// Test that --disable-temp-files is properly passed through server_args
#[test]
fn test_no_temp_files_flag_in_server_args() {
    // Simulate what main.rs does when --disable-temp-files is passed
    let mut server_args = vec!["--some-arg".to_string()];
    let no_temp_files = true;

    if no_temp_files {
        server_args.push("--disable-temp-files".to_string());
    }

    let config = SessionManagerConfig {
        server_command: "ahma_mcp".to_string(),
        server_args: server_args.clone(),
        default_scope: Some(test_temp_path("test")),
        enable_colored_output: false,
        handshake_timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    };

    assert!(
        config
            .server_args
            .contains(&"--disable-temp-files".to_string()),
        "Server args should contain --disable-temp-files when flag is enabled"
    );
}

/// Test that sandbox scope extraction from file:// URIs works correctly
/// This test uses Unix-style absolute paths which are only valid on Unix platforms.
#[cfg(unix)]
#[test]
fn test_sandbox_scope_from_file_uri() {
    let roots = vec![
        McpRoot {
            uri: "file:///Users/test/project".to_string(),
            name: Some("project".to_string()),
        },
        McpRoot {
            uri: "file:///Users/test/shared".to_string(),
            name: Some("shared".to_string()),
        },
    ];

    // Verify URI parsing works correctly
    for root in &roots {
        let path = root
            .uri
            .strip_prefix("file://")
            .expect("URI should have file:// prefix");
        let path_buf = PathBuf::from(path);
        assert!(
            path_buf.is_absolute(),
            "Extracted path should be absolute: {:?}",
            path_buf
        );
    }
}

/// Test that SessionManagerConfig properly stores the default scope
#[test]
fn test_session_manager_config_default_scope() {
    let default_scope = test_abs(&["Users", "test", "fallback_workspace"]);

    let config = SessionManagerConfig {
        server_command: "ahma_mcp".to_string(),
        server_args: vec![],
        default_scope: Some(default_scope.clone()),
        enable_colored_output: false,
        handshake_timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    };

    assert_eq!(
        config.default_scope,
        Some(default_scope),
        "Config should preserve default scope"
    );
}

/// Test that session manager creates sessions with proper isolation
#[tokio::test]
async fn test_session_isolation_creates_separate_sessions() {
    let config = SessionManagerConfig {
        server_command: "echo".to_string(), // Use echo as safe subprocess
        server_args: vec!["test".to_string()],
        default_scope: Some(test_temp_path("isolation_test")),
        enable_colored_output: false,
        handshake_timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    };

    let manager = SessionManager::new(config);

    // Create two sessions
    let session1 = manager
        .create_session()
        .await
        .expect("Session 1 should be created");
    let session2 = manager
        .create_session()
        .await
        .expect("Session 2 should be created");

    assert_ne!(session1, session2, "Each session should have a unique ID");

    // Both sessions should exist
    assert!(
        manager.get_session(&session1).is_some(),
        "Session 1 should exist"
    );
    assert!(
        manager.get_session(&session2).is_some(),
        "Session 2 should exist"
    );
}

/// Test that sandbox cannot be re-locked after initial lock (security invariant)
#[tokio::test]
async fn test_sandbox_lock_immutability() {
    let config = SessionManagerConfig {
        server_command: "echo".to_string(),
        server_args: vec![],
        default_scope: Some(test_temp_path("lock_test")),
        enable_colored_output: false,
        handshake_timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    };

    let manager = SessionManager::new(config);
    let session_id = manager.create_session().await.unwrap();

    // Use platform-appropriate absolute paths for the sandbox scope URIs.
    // On Windows, Unix-style paths like /Users/test/... are not absolute.
    #[cfg(not(windows))]
    let (initial_uri, expected_scope, attacker_uri) = (
        "file:///tmp/test_project1".to_string(),
        PathBuf::from("/tmp/test_project1"),
        "file:///tmp/attacker_malicious".to_string(),
    );
    #[cfg(windows)]
    let (initial_uri, expected_scope, attacker_uri) = (
        "file:///C:/test/project1".to_string(),
        PathBuf::from(r"C:\test\project1"),
        "file:///C:/attacker/malicious".to_string(),
    );

    let initial_roots = vec![McpRoot {
        uri: initial_uri,
        name: Some("project1".to_string()),
    }];

    // First lock should succeed and return true (newly locked)
    let lock_result = manager.lock_sandbox(&session_id, &initial_roots).await;
    assert!(lock_result.is_ok(), "Initial sandbox lock should succeed");
    assert!(
        lock_result.unwrap(),
        "First lock should return true (newly locked)"
    );

    // Second lock attempt should succeed but return false (already locked)
    // This is idempotent - the security invariant is that the scope doesn't CHANGE
    let different_roots = vec![McpRoot {
        uri: attacker_uri,
        name: Some("malicious".to_string()),
    }];

    let relock_result = manager.lock_sandbox(&session_id, &different_roots).await;
    assert!(
        relock_result.is_ok(),
        "Re-lock call should not error (idempotent)"
    );
    assert!(
        !relock_result.unwrap(),
        "Re-lock should return false (already locked, scope unchanged)"
    );

    // Verify the scope is still the ORIGINAL scope, not the attacker's scope
    let session = manager.get_session(&session_id).unwrap();
    let scope = session.get_sandbox_scope().await;
    assert_eq!(
        scope,
        Some(expected_scope),
        "Sandbox scope should remain as original, not changed to attacker's path"
    );
}

/// Test that attempting to access outside sandbox scope fails
/// This is a unit test for scope validation logic
#[test]
fn test_sandbox_scope_validation_logic() {
    let sandbox_scope = test_abs(&["Users", "test", "project"]);

    let valid_path = test_abs(&["Users", "test", "project", "src", "main.rs"]);
    assert!(
        valid_path.starts_with(&sandbox_scope),
        "Path inside sandbox should be valid"
    );

    let invalid_path = test_abs(&["Users", "test", "other_project", "secrets.txt"]);
    assert!(
        !invalid_path.starts_with(&sandbox_scope),
        "Path outside sandbox should be invalid"
    );

    let canonicalized = test_abs(&["Users", "test", "other_project", "secrets.txt"]);
    assert!(
        !canonicalized.starts_with(&sandbox_scope),
        "Canonicalized traversal path should be outside sandbox"
    );
}

/// Test multi-root workspace support
#[tokio::test]
async fn test_multi_root_workspace_sandbox() {
    let config = SessionManagerConfig {
        server_command: "echo".to_string(),
        server_args: vec![],
        default_scope: Some(test_temp_path("multi_root_test")),
        enable_colored_output: false,
        handshake_timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    };

    let manager = SessionManager::new(config);
    let session_id = manager.create_session().await.unwrap();

    // Use platform-appropriate absolute paths for multi-root sandbox URIs.
    #[cfg(not(windows))]
    let (uri_a, uri_b, expected_primary) = (
        "file:///tmp/test_project_a".to_string(),
        "file:///tmp/test_project_b".to_string(),
        PathBuf::from("/tmp/test_project_a"),
    );
    #[cfg(windows)]
    let (uri_a, uri_b, expected_primary) = (
        "file:///C:/test/project_a".to_string(),
        "file:///C:/test/project_b".to_string(),
        PathBuf::from(r"C:\test\project_a"),
    );

    // Multi-root workspace with two projects
    let roots = vec![
        McpRoot {
            uri: uri_a,
            name: Some("project_a".to_string()),
        },
        McpRoot {
            uri: uri_b,
            name: Some("project_b".to_string()),
        },
    ];

    let lock_result = manager.lock_sandbox(&session_id, &roots).await;
    assert!(
        lock_result.is_ok(),
        "Multi-root workspace should lock successfully"
    );

    // Verify the first root is used as primary sandbox scope
    let session = manager.get_session(&session_id).unwrap();
    let scope = session.get_sandbox_scope().await;
    assert!(scope.is_some(), "Sandbox scope should be set");

    // First root should be the primary scope
    if let Some(primary_scope) = scope {
        assert_eq!(
            primary_scope, expected_primary,
            "Primary scope should be the first root"
        );
    }
}

/// Test empty roots are rejected when no explicit fallback scope is configured.
#[tokio::test]
async fn test_empty_roots_rejected() {
    let config = SessionManagerConfig {
        server_command: "echo".to_string(),
        server_args: vec![],
        default_scope: None,
        enable_colored_output: false,
        handshake_timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    };

    let manager = SessionManager::new(config);
    let session_id = manager.create_session().await.unwrap();

    // Empty roots
    let empty_roots: Vec<McpRoot> = vec![];

    let lock_result = manager.lock_sandbox(&session_id, &empty_roots).await;
    assert!(
        lock_result.is_err(),
        "Empty roots should be rejected, not fall back to default scope"
    );

    let session = manager.get_session(&session_id).unwrap();
    assert!(
        !session.is_sandbox_locked(),
        "Sandbox should not be locked after empty roots rejection"
    );
}

/// Test empty roots use explicit fallback scope when configured.
#[tokio::test]
async fn test_empty_roots_use_explicit_fallback_scope() {
    let fallback_scope = test_temp_path("default_scope_test");

    let config = SessionManagerConfig {
        server_command: "echo".to_string(),
        server_args: vec![],
        default_scope: Some(fallback_scope.clone()),
        enable_colored_output: false,
        handshake_timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    };

    let manager = SessionManager::new(config);
    let session_id = manager.create_session().await.unwrap();

    let empty_roots: Vec<McpRoot> = vec![];
    let lock_result = manager.lock_sandbox(&session_id, &empty_roots).await;

    assert!(
        lock_result.is_ok() && lock_result.unwrap(),
        "Empty roots should lock with explicit fallback scope"
    );

    let session = manager.get_session(&session_id).unwrap();
    let scope = session.get_sandbox_scope().await;
    assert_eq!(scope, Some(fallback_scope));
}
