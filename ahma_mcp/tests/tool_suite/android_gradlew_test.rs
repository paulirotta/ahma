//! Tests for kotlin.json (Android/Gradle tools) config loading and validation.
//!
//! These tests verify that the kotlin tool config file loads correctly, all
//! subcommands parse, and the custom catch-all subcommand is present.
//!
//! Feature-gated with `#[cfg(feature = "android")]` tests require an Android SDK.

use ahma_mcp::config::ToolConfig;

#[test]
fn test_kotlin_config_loads() {
    let json = include_str!("../../../.ahma/kotlin.json");
    let config: ToolConfig = serde_json::from_str(json).expect("kotlin.json should parse");
    assert_eq!(config.name, "kotlin");
    assert_eq!(config.command, "./gradlew");
    assert!(config.enabled);
}

#[test]
fn test_kotlin_config_has_subcommands() {
    let json = include_str!("../../../.ahma/kotlin.json");
    let config: ToolConfig = serde_json::from_str(json).expect("kotlin.json should parse");
    let subs = config.subcommand.as_ref().expect("should have subcommands");
    assert!(
        subs.len() > 10,
        "kotlin.json should have many subcommands, got {}",
        subs.len()
    );

    // Verify some key subcommands exist
    let names: Vec<&str> = subs.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"tasks"), "should have 'tasks' subcommand");
    assert!(names.contains(&"build"), "should have 'build' subcommand");
    assert!(
        names.contains(&"assembleDebug"),
        "should have 'assembleDebug' subcommand"
    );
    assert!(names.contains(&"test"), "should have 'test' subcommand");
    assert!(
        names.contains(&"custom"),
        "should have 'custom' catch-all subcommand"
    );
}

#[test]
fn test_kotlin_config_custom_subcommand() {
    let json = include_str!("../../../.ahma/kotlin.json");
    let config: ToolConfig = serde_json::from_str(json).expect("kotlin.json should parse");
    let subs = config.subcommand.as_ref().expect("should have subcommands");
    let custom = subs
        .iter()
        .find(|s| s.name == "custom")
        .expect("'custom' subcommand missing");
    assert!(
        custom.description.contains("Gradle task"),
        "custom description should mention Gradle task"
    );
    assert!(custom.synchronous != Some(true), "custom should be async");
}

#[test]
fn test_kotlin_config_has_availability_check() {
    let json = include_str!("../../../.ahma/kotlin.json");
    let config: ToolConfig = serde_json::from_str(json).expect("kotlin.json should parse");
    let check = config
        .availability_check
        .as_ref()
        .expect("should have availability_check");
    assert_eq!(check.command.as_deref(), Some("java"));
}

#[test]
fn test_kotlin_config_synchronous_subcommands() {
    let json = include_str!("../../../.ahma/kotlin.json");
    let config: ToolConfig = serde_json::from_str(json).expect("kotlin.json should parse");
    let subs = config.subcommand.as_ref().expect("should have subcommands");

    // tasks and help should be synchronous
    let tasks = subs.iter().find(|s| s.name == "tasks").unwrap();
    assert!(
        tasks.synchronous == Some(true),
        "tasks should be synchronous"
    );

    let help = subs.iter().find(|s| s.name == "help").unwrap();
    assert!(help.synchronous == Some(true), "help should be synchronous");

    // build and assembleDebug should be async
    let build = subs.iter().find(|s| s.name == "build").unwrap();
    assert!(build.synchronous != Some(true), "build should be async");
}
