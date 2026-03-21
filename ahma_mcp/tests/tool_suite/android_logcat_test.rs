//! Tests for android-logcat.json livelog tool configuration.
//!
//! Validates that the android-logcat livelog tool config loads correctly,
//! has the right tool_type, and all required livelog fields are present.

use ahma_mcp::config::{ToolConfig, ToolType};

#[test]
fn test_android_logcat_config_loads() {
    let json = include_str!("../../../.ahma/android-logcat.json");
    let config: ToolConfig = serde_json::from_str(json).expect("android-logcat.json should parse");
    assert_eq!(config.name, "android-logcat");
    assert_eq!(config.tool_type, Some(ToolType::Livelog));
    assert_eq!(config.command, "adb");
}

#[test]
fn test_android_logcat_livelog_block() {
    let json = include_str!("../../../.ahma/android-logcat.json");
    let config: ToolConfig = serde_json::from_str(json).expect("android-logcat.json should parse");
    let lc = config.livelog.expect("livelog block required");
    assert_eq!(lc.source_command, "adb");
    assert!(lc.source_args.contains(&"logcat".to_string()));
    assert!(!lc.detection_prompt.is_empty());
    assert_eq!(lc.llm_provider.model, "llama3.2");
}

#[test]
fn test_android_logcat_has_detection_prompt() {
    let json = include_str!("../../../.ahma/android-logcat.json");
    let config: ToolConfig = serde_json::from_str(json).expect("android-logcat.json should parse");
    let lc = config.livelog.unwrap();
    // Detection prompt should mention crash-related keywords
    let prompt = lc.detection_prompt.to_lowercase();
    assert!(
        prompt.contains("crash") || prompt.contains("exception") || prompt.contains("fatal"),
        "detection_prompt should reference crash/exception/fatal patterns"
    );
}
