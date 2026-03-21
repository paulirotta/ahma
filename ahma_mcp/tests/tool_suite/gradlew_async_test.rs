//! Tests for kotlin.json async subcommand behavior.
//!
//! Verifies that async subcommands (build, assembleDebug, etc.) are correctly
//! configured and that synchronous subcommands are properly flagged.

use ahma_mcp::config::ToolConfig;

#[test]
fn test_kotlin_async_subcommands_are_not_synchronous() {
    let json = include_str!("../../../.ahma/kotlin.json");
    let config: ToolConfig = serde_json::from_str(json).expect("kotlin.json should parse");
    let subs = config.subcommand.as_ref().expect("should have subcommands");

    let async_names = [
        "build",
        "assembleDebug",
        "assembleRelease",
        "installDebug",
        "installRelease",
        "test",
        "connectedAndroidTest",
        "lint",
        "custom",
    ];

    for name in &async_names {
        if let Some(sub) = subs.iter().find(|s| s.name == *name) {
            assert!(
                sub.synchronous != Some(true),
                "subcommand '{}' should be async (synchronous=false)",
                name
            );
        }
    }
}

#[test]
fn test_kotlin_sync_subcommands_are_synchronous() {
    let json = include_str!("../../../.ahma/kotlin.json");
    let config: ToolConfig = serde_json::from_str(json).expect("kotlin.json should parse");
    let subs = config.subcommand.as_ref().expect("should have subcommands");

    let sync_names = ["tasks", "help", "dependencies"];

    for name in &sync_names {
        if let Some(sub) = subs.iter().find(|s| s.name == *name) {
            assert!(
                sub.synchronous == Some(true),
                "subcommand '{}' should be synchronous",
                name
            );
        }
    }
}

#[test]
fn test_kotlin_all_subcommands_have_working_directory() {
    let json = include_str!("../../../.ahma/kotlin.json");
    let config: ToolConfig = serde_json::from_str(json).expect("kotlin.json should parse");
    let subs = config.subcommand.as_ref().expect("should have subcommands");

    for sub in subs {
        let has_wd = sub
            .options
            .as_ref()
            .map(|opts| opts.iter().any(|o| o.name == "working_directory"))
            .unwrap_or(false);
        assert!(
            has_wd,
            "subcommand '{}' should have a working_directory option",
            sub.name
        );
    }
}
