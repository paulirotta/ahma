//! Tests for kotlin.json interactive tool properties.
//!
//! Verifies timeout, hints, and description fields of the kotlin tool config.

use ahma_mcp::config::ToolConfig;

#[test]
fn test_kotlin_config_has_timeout() {
    let json = include_str!("../../../.ahma/kotlin.json");
    let config: ToolConfig = serde_json::from_str(json).expect("kotlin.json should parse");
    assert_eq!(
        config.timeout_seconds,
        Some(600),
        "should have 600s timeout"
    );
}

#[test]
fn test_kotlin_config_has_hints() {
    let json = include_str!("../../../.ahma/kotlin.json");
    let config: ToolConfig = serde_json::from_str(json).expect("kotlin.json should parse");
    assert!(
        config.hints.build.is_some(),
        "hints should have 'build' key"
    );
    assert!(config.hints.test.is_some(), "hints should have 'test' key");
}

#[test]
fn test_kotlin_config_has_install_instructions() {
    let json = include_str!("../../../.ahma/kotlin.json");
    let config: ToolConfig = serde_json::from_str(json).expect("kotlin.json should parse");
    assert!(
        config.install_instructions.is_some(),
        "should have install_instructions"
    );
    let inst = config.install_instructions.unwrap();
    assert!(
        inst.contains("Java"),
        "install_instructions should mention Java"
    );
}
