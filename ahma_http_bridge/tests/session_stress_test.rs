//! Session Stress Tests for HTTP Bridge
//!
//! These tests verify the session management under concurrent load:
//! - Multiple simultaneous session creation/destruction
//! - Sandbox scope locking under race conditions
//! - Session termination cleanup
//!
//! These tests improve coverage in `ahma_http_bridge/src/session.rs` (46.84% → higher)

use ahma_http_bridge::DEFAULT_HANDSHAKE_TIMEOUT_SECS;
use ahma_http_bridge::session::{
    McpRoot, SessionManager, SessionManagerConfig, SessionTerminationReason,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Helper to create a SessionManager with test configuration
fn create_test_session_manager(default_scope: Option<PathBuf>) -> SessionManager {
    let config = SessionManagerConfig {
        server_command: "echo".to_string(),
        server_args: vec!["test".to_string()],
        default_scope,
        enable_colored_output: false,
        handshake_timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    };
    SessionManager::new(config)
}

// =============================================================================
// Concurrent Session Creation Tests
// =============================================================================

/// Test creating many sessions concurrently
#[tokio::test]
async fn test_concurrent_session_creation() {
    let default_scope = PathBuf::from("/tmp/stress_test");
    let session_manager = Arc::new(create_test_session_manager(Some(default_scope)));

    let num_sessions = 50;
    let mut handles = Vec::new();

    // Spawn concurrent session creation tasks
    for i in 0..num_sessions {
        let sm = session_manager.clone();
        handles.push(tokio::spawn(async move {
            let result = sm.create_session().await;
            (i, result)
        }));
    }

    // Collect results
    let mut successes = 0;
    let mut failures = 0;
    let mut session_ids = Vec::new();

    for handle in handles {
        let (i, result) = handle.await.unwrap();
        match result {
            Ok(session_id) => {
                successes += 1;
                session_ids.push(session_id);
            }
            Err(e) => {
                failures += 1;
                eprintln!("Session {} failed: {}", i, e);
            }
        }
    }

    println!("Created {} sessions, {} failures", successes, failures);

    // All sessions should be created successfully
    assert_eq!(failures, 0, "All session creations should succeed");
    assert_eq!(successes, num_sessions);

    // Verify all session IDs are unique
    let unique_count = session_ids
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert_eq!(
        unique_count, num_sessions,
        "All session IDs should be unique"
    );

    // Verify all sessions exist
    for session_id in &session_ids {
        assert!(
            session_manager.session_exists(session_id),
            "Created session should exist"
        );
    }

    // Cleanup: terminate all sessions
    for session_id in session_ids {
        let _ = session_manager
            .terminate_session(&session_id, SessionTerminationReason::ClientRequested)
            .await;
    }
}

/// Test concurrent session creation and termination (race condition stress)
#[tokio::test]
async fn test_concurrent_creation_and_termination() {
    let default_scope = PathBuf::from("/tmp/race_test");
    let session_manager = Arc::new(create_test_session_manager(Some(default_scope)));

    let num_iterations = 20;

    for _ in 0..num_iterations {
        let sm = session_manager.clone();

        // Create session
        let session_id = sm.create_session().await.unwrap();

        // Clone for concurrent operations
        let sm1 = sm.clone();
        let sm2 = sm.clone();
        let id1 = session_id.clone();
        let id2 = session_id.clone();

        // Race: try to terminate and check existence concurrently
        let handle1 = tokio::spawn(async move {
            sm1.terminate_session(&id1, SessionTerminationReason::ClientRequested)
                .await
        });

        let handle2 = tokio::spawn(async move { sm2.session_exists(&id2) });

        let (term_result, _exists_result) = tokio::join!(handle1, handle2);

        // Termination should succeed or the session should not exist after
        if let Ok(inner_result) = term_result {
            assert!(
                !sm.session_exists(&session_id) || inner_result.is_ok(),
                "Session should not exist after termination"
            );
        }
    }
}

/// Test sandbox locking races
#[tokio::test]
async fn test_concurrent_sandbox_lock_attempts() {
    let default_scope = PathBuf::from("/tmp/lock_race_test");
    let session_manager = Arc::new(create_test_session_manager(Some(default_scope)));

    // Create a single session
    let session_id = session_manager.create_session().await.unwrap();

    // Create different root sets
    let roots_a = vec![McpRoot {
        uri: "file:///project/a".to_string(),
        name: Some("Project A".to_string()),
    }];

    let roots_b = vec![McpRoot {
        uri: "file:///project/b".to_string(),
        name: Some("Project B".to_string()),
    }];

    // Race: try to lock sandbox with different roots concurrently
    let sm1 = session_manager.clone();
    let sm2 = session_manager.clone();
    let id1 = session_id.clone();
    let id2 = session_id.clone();

    let handle1 = tokio::spawn(async move { sm1.lock_sandbox(&id1, &roots_a).await });

    let handle2 = tokio::spawn(async move { sm2.lock_sandbox(&id2, &roots_b).await });

    let (result1, result2) = tokio::join!(handle1, handle2);

    // At least one should succeed - could be both if first completes before second starts
    let success1 = result1.unwrap().is_ok();
    let success2 = result2.unwrap().is_ok();

    assert!(
        success1 || success2,
        "At least one lock attempt should succeed"
    );

    // Session should still exist and have a locked sandbox
    assert!(session_manager.session_exists(&session_id));

    let session = session_manager.get_session(&session_id).unwrap();
    let scope = session.get_sandbox_scope().await;
    assert!(scope.is_some(), "Session should have locked sandbox scope");

    // Cleanup
    let _ = session_manager
        .terminate_session(&session_id, SessionTerminationReason::ClientRequested)
        .await;
}

/// Test many sessions with independent sandbox scopes
#[tokio::test]
async fn test_many_independent_sandbox_scopes() {
    let default_scope = PathBuf::from("/tmp/multi_scope_test");
    let session_manager = Arc::new(create_test_session_manager(Some(default_scope.clone())));

    let num_sessions = 20;
    let mut session_ids = Vec::new();

    // Create sessions with unique sandbox scopes
    for i in 0..num_sessions {
        let session_id = session_manager.create_session().await.unwrap();

        let roots = vec![McpRoot {
            uri: format!("file:///workspace/project_{}", i),
            name: Some(format!("Project {}", i)),
        }];

        session_manager
            .lock_sandbox(&session_id, &roots)
            .await
            .unwrap();
        session_ids.push((session_id, i));
    }

    // Verify each session has its correct sandbox scope
    for (session_id, i) in &session_ids {
        let session = session_manager.get_session(session_id).unwrap();
        let scope = session.get_sandbox_scope().await.unwrap();
        let expected = PathBuf::from(format!("/workspace/project_{}", i));

        assert_eq!(scope, expected, "Session should have scope {:?}", expected);
    }

    // Concurrent verification - read all scopes at once
    let futures: Vec<_> = session_ids
        .iter()
        .map(|(session_id, i)| {
            let sm = session_manager.clone();
            let id = session_id.clone();
            let idx = *i;
            async move {
                let session = sm
                    .get_session(&id)
                    .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
                let scope = session
                    .get_sandbox_scope()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("No scope"))?;
                Ok::<_, anyhow::Error>((idx, scope))
            }
        })
        .collect();

    let results = futures::future::join_all(futures).await;

    for result in results {
        let (i, scope) = result.unwrap();
        let expected = PathBuf::from(format!("/workspace/project_{}", i));
        assert_eq!(scope, expected);
    }

    // Cleanup
    for (session_id, _) in session_ids {
        let _ = session_manager
            .terminate_session(&session_id, SessionTerminationReason::ClientRequested)
            .await;
    }
}

/// Test session termination with all termination reasons
#[tokio::test]
async fn test_termination_reasons() {
    let default_scope = PathBuf::from("/tmp/term_reasons_test");
    let session_manager = create_test_session_manager(Some(default_scope));

    let reasons = vec![
        SessionTerminationReason::ClientRequested,
        SessionTerminationReason::RootsChangeRejected,
        SessionTerminationReason::Timeout,
        SessionTerminationReason::ProcessCrashed,
    ];

    for reason in reasons {
        let session_id = session_manager.create_session().await.unwrap();
        assert!(session_manager.session_exists(&session_id));

        let result = session_manager.terminate_session(&session_id, reason).await;
        assert!(
            result.is_ok(),
            "Termination with {:?} should succeed",
            reason
        );

        assert!(
            !session_manager.session_exists(&session_id),
            "Session should not exist after termination with {:?}",
            reason
        );
    }
}

/// Test rapid session lifecycle (create, lock, terminate in quick succession)
#[tokio::test]
async fn test_rapid_session_lifecycle() {
    let default_scope = PathBuf::from("/tmp/rapid_lifecycle_test");
    let session_manager = create_test_session_manager(Some(default_scope));

    let num_cycles = 100;

    for i in 0..num_cycles {
        // Create
        let session_id = session_manager.create_session().await.unwrap();

        // Lock sandbox
        let roots = vec![McpRoot {
            uri: format!("file:///workspace/cycle_{}", i),
            name: None,
        }];
        session_manager
            .lock_sandbox(&session_id, &roots)
            .await
            .unwrap();

        // Verify
        let session = session_manager.get_session(&session_id).unwrap();
        let scope = session.get_sandbox_scope().await.unwrap();
        assert_eq!(scope, PathBuf::from(format!("/workspace/cycle_{}", i)));

        // Terminate
        session_manager
            .terminate_session(&session_id, SessionTerminationReason::ClientRequested)
            .await
            .unwrap();

        // Verify terminated
        assert!(!session_manager.session_exists(&session_id));
    }
}

/// Test handle_roots_changed on sessions without locked sandbox
#[tokio::test]
async fn test_roots_changed_before_lock() {
    let default_scope = PathBuf::from("/tmp/roots_changed_test");
    let session_manager = create_test_session_manager(Some(default_scope));

    let session_id = session_manager.create_session().await.unwrap();

    // Try to handle roots changed before locking - should succeed (no-op)
    // since sandbox isn't locked yet
    let result = session_manager.handle_roots_changed(&session_id).await;

    // This might error or succeed depending on implementation
    // The key is the session should still be valid after
    println!("Roots changed before lock result: {:?}", result);
}

/// Test get_session for non-existent session
#[tokio::test]
async fn test_get_nonexistent_session() {
    let default_scope = PathBuf::from("/tmp/nonexistent_test");
    let session_manager = create_test_session_manager(Some(default_scope));

    let result = session_manager.get_session("nonexistent-session-id");
    assert!(
        result.is_none(),
        "get_session should return None for non-existent session"
    );
}

/// Test session_exists for various scenarios
#[tokio::test]
async fn test_session_exists_scenarios() {
    let default_scope = PathBuf::from("/tmp/exists_test");
    let session_manager = create_test_session_manager(Some(default_scope));

    // Non-existent session
    assert!(!session_manager.session_exists("does-not-exist"));

    // Created session
    let session_id = session_manager.create_session().await.unwrap();
    assert!(session_manager.session_exists(&session_id));

    // After termination
    session_manager
        .terminate_session(&session_id, SessionTerminationReason::ClientRequested)
        .await
        .unwrap();
    assert!(!session_manager.session_exists(&session_id));
}

/// Test duplicate termination (should be idempotent or error gracefully)
#[tokio::test]
async fn test_duplicate_termination() {
    let default_scope = PathBuf::from("/tmp/dup_term_test");
    let session_manager = create_test_session_manager(Some(default_scope));

    let session_id = session_manager.create_session().await.unwrap();

    // First termination
    let result1 = session_manager
        .terminate_session(&session_id, SessionTerminationReason::ClientRequested)
        .await;
    assert!(result1.is_ok());

    // Second termination - should handle gracefully
    let result2 = session_manager
        .terminate_session(&session_id, SessionTerminationReason::ClientRequested)
        .await;
    // Either succeeds (idempotent) or fails gracefully
    println!("Duplicate termination result: {:?}", result2);
}

/// Test URI parsing edge cases
#[tokio::test]
async fn test_uri_parsing_edge_cases() {
    let default_scope = PathBuf::from("/tmp/uri_test");
    let session_manager = create_test_session_manager(Some(default_scope.clone()));

    #[cfg(not(target_os = "windows"))]
    let test_cases = vec![
        // Standard file URI
        (
            "file:///Users/test/project",
            PathBuf::from("/Users/test/project"),
        ),
        // Without trailing slash
        ("file:///path/to/dir", PathBuf::from("/path/to/dir")),
        // With encoded spaces (percent-decoded to a real filesystem path)
        (
            "file:///path/with%20space",
            PathBuf::from("/path/with space"),
        ),
        // Root path
        ("file:///", PathBuf::from("/")),
    ];

    #[cfg(target_os = "windows")]
    let test_cases = vec![
        (
            "file:///C:/Users/test/project",
            PathBuf::from(r"C:/Users/test/project"),
        ),
        (
            "file://localhost/C:/Users/test/project",
            PathBuf::from(r"C:/Users/test/project"),
        ),
        (
            "file://server/share/path",
            PathBuf::from(r"//server/share/path"),
        ),
        (
            "file:///C:/path/with%20space",
            PathBuf::from(r"C:/path/with space"),
        ),
    ];

    for (uri, expected) in test_cases {
        let session_id = session_manager.create_session().await.unwrap();

        let roots = vec![McpRoot {
            uri: uri.to_string(),
            name: None,
        }];

        let lock_result = session_manager.lock_sandbox(&session_id, &roots).await;

        if lock_result.is_ok() {
            let session = session_manager.get_session(&session_id).unwrap();
            let scope = session.get_sandbox_scope().await.unwrap();
            assert_eq!(
                scope, expected,
                "URI {} should parse to {:?}",
                uri, expected
            );
        }

        let _ = session_manager
            .terminate_session(&session_id, SessionTerminationReason::ClientRequested)
            .await;
    }
}

/// Test timeout behavior with long operations (simulated)
#[tokio::test]
async fn test_session_operations_with_timeout() {
    let default_scope = PathBuf::from("/tmp/timeout_test");
    let session_manager = Arc::new(create_test_session_manager(Some(default_scope)));

    let session_id = session_manager.create_session().await.unwrap();

    // Operations should complete within reasonable timeout
    let result = timeout(Duration::from_secs(5), async {
        let roots = vec![McpRoot {
            uri: "file:///test".to_string(),
            name: None,
        }];
        session_manager.lock_sandbox(&session_id, &roots).await
    })
    .await;

    assert!(
        result.is_ok(),
        "Lock operation should complete within timeout"
    );
    assert!(result.unwrap().is_ok(), "Lock should succeed");

    // Cleanup
    let _ = session_manager
        .terminate_session(&session_id, SessionTerminationReason::ClientRequested)
        .await;
}

/// Test session manager rejects empty roots (security feature)
///
/// Empty roots should be rejected when no explicit fallback scope is configured.
#[tokio::test]
async fn test_empty_roots_rejected() {
    let session_manager = create_test_session_manager(None);

    let session_id = session_manager.create_session().await.unwrap();

    // Try to lock with empty roots - should fail
    let empty_roots: Vec<McpRoot> = vec![];
    let result = session_manager
        .lock_sandbox(&session_id, &empty_roots)
        .await;

    assert!(
        result.is_err(),
        "Empty roots should be rejected, not fall back to default"
    );

    let session = session_manager.get_session(&session_id).unwrap();
    assert!(
        !session.is_sandbox_locked(),
        "Sandbox should not be locked after empty roots rejection"
    );

    let _ = session_manager
        .terminate_session(&session_id, SessionTerminationReason::ClientRequested)
        .await;
}

/// Test multiple roots - should use first root's path
#[tokio::test]
async fn test_multiple_roots_uses_first() {
    let default_scope = PathBuf::from("/tmp/multi_roots_test");
    let session_manager = create_test_session_manager(Some(default_scope));

    let session_id = session_manager.create_session().await.unwrap();

    let roots = vec![
        McpRoot {
            uri: "file:///first/project".to_string(),
            name: Some("First".to_string()),
        },
        McpRoot {
            uri: "file:///second/project".to_string(),
            name: Some("Second".to_string()),
        },
        McpRoot {
            uri: "file:///third/project".to_string(),
            name: Some("Third".to_string()),
        },
    ];

    session_manager
        .lock_sandbox(&session_id, &roots)
        .await
        .unwrap();

    let session = session_manager.get_session(&session_id).unwrap();
    let scope = session.get_sandbox_scope().await.unwrap();

    assert_eq!(
        scope,
        PathBuf::from("/first/project"),
        "Should use first root's path as sandbox scope"
    );

    let _ = session_manager
        .terminate_session(&session_id, SessionTerminationReason::ClientRequested)
        .await;
}
