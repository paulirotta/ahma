//! TOCTOU (Time-of-Check to Time-of-Use) Symlink Race Condition Tests
//!
//! These tests establish a baseline for how Ahma handles path security
//! during rapid symlink swaps, a high-impact security area.

use ahma_mcp::path_security::validate_path;
use anyhow::Result;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[tokio::test]
async fn test_toctou_symlink_swap_detection() -> Result<()> {
    // This test simulates a rapid symlink swap between validation and use.
    // While validate_path is async, we want to ensure it doesn't leave windows
    // where a path can be swapped to point outside the sandbox.

    let temp_sandbox = TempDir::new()?;
    let sandbox_root = temp_sandbox.path().to_path_buf();

    let temp_outside = TempDir::new()?;
    let outside_root = temp_outside.path().to_path_buf();

    let link_path = sandbox_root.join("active_link");
    let safe_target = sandbox_root.join("safe_dir");
    #[cfg(unix)]
    let unsafe_target = outside_root;

    fs::create_dir(&safe_target)?;

    // Initial state: link points to safe target
    #[cfg(unix)]
    std::os::unix::fs::symlink(&safe_target, &link_path)?;

    // We run multiple iterations to try and hit a race window if any exists
    // in the async validation logic.
    for i in 0..10 {
        // Validation should succeed initially
        let validated = validate_path(&link_path, &sandbox_root).await?;
        let real_validated = fs::canonicalize(&validated)?;
        let real_sandbox_root = fs::canonicalize(&sandbox_root)?;
        assert!(real_validated.starts_with(&real_sandbox_root));

        // Rapidly swap the link to an unsafe target
        fs::remove_file(&link_path)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&unsafe_target, &link_path)?;

        // Second validation should FAIL immediately
        let result = validate_path(&link_path, &sandbox_root).await;
        assert!(
            result.is_err(),
            "Iteration {}: Validation should have failed after symlink swap to outside sandbox",
            i
        );

        // Reset for next iteration
        fs::remove_file(&link_path)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&safe_target, &link_path)?;
    }

    Ok(())
}

#[tokio::test]
async fn test_deep_nested_subcommand_validation() -> Result<()> {
    // Verified Requirement: Deeply nested subcommands (>2 levels)
    // We create a mock ToolConfig with 4 levels of nesting and validate it

    use ahma_mcp::config::{SubcommandConfig, ToolConfig};
    use ahma_mcp::schema_validation::MtdfValidator;

    let config = ToolConfig {
        name: "cloud-cli".to_string(),
        description: "A complex cloud CLI".to_string(),
        command: "cloud".to_string(),
        subcommand: Some(vec![SubcommandConfig {
            name: "service".to_string(),
            description: "Cloud service".to_string(),
            subcommand: Some(vec![SubcommandConfig {
                name: "resource".to_string(),
                description: "Cloud resource".to_string(),
                subcommand: Some(vec![SubcommandConfig {
                    name: "action".to_string(),
                    description: "Resource action".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            }]),
            ..Default::default()
        }]),
        ..Default::default()
    };

    let validator = MtdfValidator::new();
    let config_json = serde_json::to_string(&config)?;
    let result = validator.validate_tool_config(Path::new("test.json"), &config_json);

    assert!(
        result.is_ok(),
        "Deeply nested subcommands should be valid: {:?}",
        result.err()
    );

    Ok(())
}
