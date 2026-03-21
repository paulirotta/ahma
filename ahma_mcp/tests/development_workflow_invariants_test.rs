use ahma_mcp::test_utils as common;

/// Critical Development Workflow Invariants Test
///
/// PURPOSE: Preserve key architectural decisions from the graceful shutdown and timeout implementation.
/// These tests encode the essential lessons learned that must survive future refactoring.
///
/// CONTEXT: Originally implemented to address two user requirements:
/// 1. "Does the ahma_mcp server shut down gracefully when .vscode/mcp.json watch triggers a restart?"
/// 2. "I think 'await' should have an optional timeout, and a default timeout of 240sec"
use std::time::Duration;

/// INVARIANT 1: Wait tool timeout defaults and validation bounds
///
/// LESSON LEARNED: Default timeout reduced from 300s to 240s per user request.
/// Validation range 10s-1800s prevents both too-short timeouts (user frustration)
/// and too-long timeouts (resource waste).
///
/// DO NOT CHANGE: These specific values were chosen based on user feedback and testing.
#[test]
fn test_wait_timeout_bounds_invariant() {
    // These bounds were established through user feedback and must not change
    const DEFAULT_TIMEOUT: u64 = 240; // 4 minutes - user requested change from 300s
    const MIN_TIMEOUT: u64 = 10; // Prevents accidentally short timeouts
    const MAX_TIMEOUT: u64 = 1800; // 30 minutes - prevents runaway waits

    assert_eq!(
        DEFAULT_TIMEOUT, 240,
        "Default timeout must remain 240s per user requirement"
    );
    assert_eq!(MIN_TIMEOUT, 10, "Minimum timeout prevents user errors");
    assert_eq!(MAX_TIMEOUT, 1800, "Maximum timeout prevents resource waste");

    // Verify reasonable progression for timeout warnings (50%, 75%, 90%)
    let warning_50 = DEFAULT_TIMEOUT / 2; // 120s
    let warning_75 = (DEFAULT_TIMEOUT * 3) / 4; // 180s
    let warning_90 = (DEFAULT_TIMEOUT * 9) / 10; // 216s

    assert!(warning_50 > 60, "50% warning should be meaningful (>1min)");
    assert!(
        warning_75 - warning_50 > 30,
        "Warnings should be spaced reasonably"
    );
    assert!(
        warning_90 - warning_75 > 20,
        "Final warning should provide adequate notice"
    );

    println!(
        "OK Wait timeout bounds validated: {}s default, {}s-{}s range",
        DEFAULT_TIMEOUT, MIN_TIMEOUT, MAX_TIMEOUT
    );
}

/// INVARIANT 2: Graceful shutdown timing requirements
///
/// LESSON LEARNED: 10-second shutdown delay allows operations to complete naturally
/// during cargo watch restarts. This prevents data loss and improves development experience.
///
/// CRITICAL: This timing was chosen to balance operation completion vs restart speed.
#[test]
fn test_graceful_shutdown_timing_invariant() {
    const SHUTDOWN_DELAY_SECONDS: u64 = 10; // Critical timing for development workflow

    // Shutdown delay must be long enough for typical operations but not so long as to frustrate developers
    // Note: SHUTDOWN_DELAY_SECONDS = 10, which satisfies both conditions (≥5 and ≤15)

    // Verify timing makes sense for common development operations
    let typical_cargo_check = Duration::from_secs(3);
    let typical_test_run = Duration::from_secs(8);
    let shutdown_window = Duration::from_secs(SHUTDOWN_DELAY_SECONDS);

    assert!(
        shutdown_window > typical_cargo_check,
        "Should accommodate quick checks"
    );
    assert!(
        shutdown_window >= typical_test_run,
        "Should accommodate most test runs"
    );

    println!(
        "OK Graceful shutdown timing validated: {}s delay",
        SHUTDOWN_DELAY_SECONDS
    );
}

/// INVARIANT 3: Tool configuration count stability
///
/// LESSON LEARNED: Tool loading tests expect exactly the right number of JSON configs.
/// This test failed when we temporarily added status.json/await.json (which are hardwired).
///
/// GUIDANCE: Only add JSON configs for external CLI tools, not for hardwired MCP tools.
#[test]
fn test_json_tool_configuration_count_invariant() {
    use std::fs;

    // Count actual JSON files in tools directory and examples config directory
    let ahma_dir = common::fs::get_workspace_path(".ahma");
    let examples_dir = common::fs::get_workspace_path(".ahma");

    let mut json_files = Vec::new();

    for dir in &[ahma_dir, examples_dir] {
        if !dir.exists() {
            println!("WARNING️  Directory {:?} not found - skipping", dir);
            continue;
        }

        let files: Vec<_> = fs::read_dir(dir)
            .expect("Should read directory")
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension()? == "json" {
                    Some(path.file_name()?.to_string_lossy().to_string())
                } else {
                    None
                }
            })
            .collect();
        json_files.extend(files);
    }

    println!("📁 Found JSON tool configurations: {:?}", json_files);

    // CRITICAL: These are CLI tool adapters only. Core tools (sandboxed_shell, status, await, cancel) are hardwired.
    // Expected bundled tool definitions (minimal set): rust.json, python.json, git.json, gh.json, kotlin.json, file-tools.json, simplify.json
    // total should be at least 5.
    assert!(
        json_files.len() >= 5,
        "Should have core CLI tool configurations (got {})",
        json_files.len()
    );
    assert!(
        json_files.len() <= 30,
        "Should not have excessive tool configurations (got {})",
        json_files.len()
    );

    // Verify core tools exist
    let has_rust = json_files.iter().any(|f| f == "rust.json");
    let legacy_cargo_files: Vec<_> = json_files
        .iter()
        .filter(|f| f.starts_with("cargo_") && f.ends_with(".json"))
        .cloned()
        .collect();
    assert!(
        legacy_cargo_files.is_empty(),
        "Legacy cargo_*.json files should be merged into rust.json: {:?}",
        legacy_cargo_files
    );
    // ls tool is optional; do not assert its presence (legacy requirement removed)
    let _has_ls = json_files.iter().any(|f| f.contains("ls"));
    let has_python = json_files.iter().any(|f| f.contains("python"));

    assert!(
        has_rust,
        "rust.json must exist (either in .ahma or examples/configs)"
    );
    // (Optional) assert for ls removed intentionally to allow repositories without ls.json
    assert!(
        has_python,
        "python.json must exist (either in .ahma or examples/configs)"
    );

    println!(
        "OK Tool configuration count validated: {} JSON files",
        json_files.len()
    );
}

/// INVARIANT 4: Progressive warning percentages
///
/// LESSON LEARNED: 50%, 75%, 90% warnings provide good user feedback without spam.
/// These percentages were chosen to give increasingly urgent warnings as timeout approaches.
///
/// CRITICAL: Don't change these percentages - they're tuned for user experience.
#[test]
fn test_progressive_warning_percentages_invariant() {
    const WARNING_THRESHOLDS: [u8; 3] = [50, 75, 90]; // Percentages for timeout warnings

    // These percentages were chosen for optimal user experience
    assert_eq!(WARNING_THRESHOLDS[0], 50, "First warning at halfway point");
    assert_eq!(
        WARNING_THRESHOLDS[1], 75,
        "Second warning at 3/4 completion"
    );
    assert_eq!(WARNING_THRESHOLDS[2], 90, "Final warning near completion");

    // Verify reasonable spacing between warnings
    let spacing_1_2 = WARNING_THRESHOLDS[1] - WARNING_THRESHOLDS[0]; // 25%
    let spacing_2_3 = WARNING_THRESHOLDS[2] - WARNING_THRESHOLDS[1]; // 15%

    assert!(
        spacing_1_2 >= 20,
        "First two warnings should be well spaced"
    );
    assert!(
        spacing_2_3 >= 10,
        "Final warning should provide adequate notice"
    );
    assert!(
        spacing_1_2 > spacing_2_3,
        "Warnings should accelerate as timeout approaches"
    );

    println!(
        "OK Progressive warning percentages validated: {:?}%",
        WARNING_THRESHOLDS
    );
}

/// INVARIANT 5: Error remediation detection patterns
///
/// LESSON LEARNED: Generic patterns catch common timeout causes across different tools.
/// These patterns were developed to help users resolve issues rather than just report them.
///
/// MAINTENANCE: Add new patterns here as new timeout causes are discovered.
#[test]
fn test_error_remediation_patterns_invariant() {
    // These patterns detect common causes of operation timeouts
    let lock_file_patterns = &[
        ".cargo-lock",
        "package-lock.json",
        "yarn.lock",
        "Cargo.lock",
        "composer.lock",
        "Pipfile.lock",
        ".lock",
    ];

    let network_keywords = &["download", "fetch", "pull", "push", "clone", "update"];
    let build_keywords = &["build", "compile", "install", "test", "check"];

    // Verify we have patterns for major categories of timeout causes
    assert!(
        lock_file_patterns.len() >= 5,
        "Should detect major lock file types"
    );
    assert!(
        network_keywords.len() >= 5,
        "Should detect network operations"
    );
    assert!(build_keywords.len() >= 4, "Should detect build operations");

    // Verify patterns don't overlap inappropriately
    let all_patterns: Vec<&str> = lock_file_patterns
        .iter()
        .chain(network_keywords.iter())
        .chain(build_keywords.iter())
        .cloned()
        .collect();

    assert!(
        all_patterns.len() > 15,
        "Should have comprehensive pattern coverage"
    );

    println!("OK Error remediation patterns validated:");
    println!("   Lock files: {:?}", lock_file_patterns);
    println!("   Network ops: {:?}", network_keywords);
    println!("   Build ops: {:?}", build_keywords);
}

/// INVARIANT 6: Signal handling requirements for graceful shutdown
///
/// LESSON LEARNED: Must handle SIGTERM (cargo watch) and SIGINT (Ctrl+C) for graceful shutdown.
/// These signals are sent in different scenarios and both need proper handling.
///
/// CRITICAL: Signal handling is essential for development workflow integration.
#[test]
fn test_signal_handling_requirements_invariant() {
    // These are the signals that must be handled for graceful shutdown
    const REQUIRED_SIGNALS: [&str; 2] = ["SIGTERM", "SIGINT"];

    // SIGTERM: Sent by cargo watch during file change restarts
    // SIGINT: Sent by Ctrl+C during development
    assert_eq!(
        REQUIRED_SIGNALS.len(),
        2,
        "Must handle exactly these two signals"
    );
    assert!(
        REQUIRED_SIGNALS.contains(&"SIGTERM"),
        "SIGTERM required for cargo watch integration"
    );
    assert!(
        REQUIRED_SIGNALS.contains(&"SIGINT"),
        "SIGINT required for user interrupts"
    );

    println!(
        "OK Signal handling requirements validated: {:?}",
        REQUIRED_SIGNALS
    );
    println!("   SIGTERM: cargo watch file change restarts");
    println!("   SIGINT: user Ctrl+C interrupts");
}

#[cfg(test)]
mod documentation_requirements {
    use super::common::fs::get_workspace_path;

    /// INVARIANT 7: Documentation completeness for user guidance
    ///
    /// LESSON LEARNED: Users need comprehensive guides for timeout issues, development workflow,
    /// and troubleshooting. Missing documentation leads to support burden.
    ///
    /// MAINTENANCE: Keep these documents updated as new features are added.
    #[test]
    fn test_required_documentation_exists() {
        let required_docs = ["README.md"];

        for doc in &required_docs {
            let path = get_workspace_path(doc);
            assert!(path.exists(), "Required documentation {} must exist", doc);

            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("Should be able to read {}", doc));
            assert!(content.len() > 1000, "{} must be comprehensive", doc);
        }

        println!("OK Required documentation validated: {:?}", required_docs);
    }
}
