//! Tests for livelog tool configuration: correct deserialization, field defaults,
//! and validation that required fields are rejected when absent.

use ahma_mcp::config::{LivelogConfig, LlmProviderConfig, ToolConfig, ToolType};

// ---------------------------------------------------------------------------
// Full livelog config round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_livelog_config_full_roundtrip() {
    let json = r#"{
        "name": "android-logcat",
        "description": "Monitor Android logs",
        "command": "adb",
        "tool_type": "livelog",
        "enabled": true,
        "livelog": {
            "source_command": "adb",
            "source_args": ["-d", "logcat", "-v", "threadtime"],
            "detection_prompt": "Look for crashes",
            "llm_provider": {
                "base_url": "http://localhost:11434/v1",
                "model": "llama3.2"
            },
            "chunk_max_lines": 50,
            "chunk_max_seconds": 30,
            "cooldown_seconds": 60,
            "llm_timeout_seconds": 30
        }
    }"#;

    let config: ToolConfig = serde_json::from_str(json).expect("should deserialize");
    assert_eq!(config.tool_type, Some(ToolType::Livelog));

    let lc = config.livelog.expect("livelog block should be present");
    assert_eq!(lc.source_command, "adb");
    assert_eq!(lc.source_args, vec!["-d", "logcat", "-v", "threadtime"]);
    assert_eq!(lc.detection_prompt, "Look for crashes");
    assert_eq!(lc.llm_provider.base_url, "http://localhost:11434/v1");
    assert_eq!(lc.llm_provider.model, "llama3.2");
    assert!(lc.llm_provider.api_key.is_none());
    assert_eq!(lc.chunk_max_lines, 50);
    assert_eq!(lc.chunk_max_seconds, 30);
    assert_eq!(lc.cooldown_seconds, 60);
    assert_eq!(lc.llm_timeout_seconds, 30);
}

// ---------------------------------------------------------------------------
// Default field values
// ---------------------------------------------------------------------------

#[test]
fn test_livelog_config_defaults_applied() {
    let json = r#"{
        "source_command": "tail",
        "source_args": ["-f", "app.log"],
        "detection_prompt": "Look for errors",
        "llm_provider": {
            "base_url": "http://localhost:11434/v1",
            "model": "llama3.2"
        }
    }"#;

    let lc: LivelogConfig = serde_json::from_str(json).expect("should deserialize");
    assert_eq!(lc.chunk_max_lines, 50, "default chunk_max_lines");
    assert_eq!(lc.chunk_max_seconds, 30, "default chunk_max_seconds");
    assert_eq!(lc.cooldown_seconds, 60, "default cooldown_seconds");
    assert_eq!(lc.llm_timeout_seconds, 30, "default llm_timeout_seconds");
    assert!(lc.source_args.contains(&"-f".to_string()));
}

// ---------------------------------------------------------------------------
// Optional api_key
// ---------------------------------------------------------------------------

#[test]
fn test_llm_provider_config_with_api_key() {
    let json = r#"{
        "base_url": "https://api.openai.com/v1",
        "model": "gpt-4o-mini",
        "api_key": "sk-test-key"
    }"#;

    let lc: LlmProviderConfig = serde_json::from_str(json).expect("should deserialize");
    assert_eq!(lc.api_key.as_deref(), Some("sk-test-key"));
}

#[test]
fn test_llm_provider_config_without_api_key() {
    let json = r#"{
        "base_url": "http://localhost:11434/v1",
        "model": "llama3.2"
    }"#;

    let lc: LlmProviderConfig = serde_json::from_str(json).expect("should deserialize");
    assert!(lc.api_key.is_none());
}

// ---------------------------------------------------------------------------
// tool_type = command (default) does not require livelog block
// ---------------------------------------------------------------------------

#[test]
fn test_tool_type_command_no_livelog_required() {
    let json = r#"{
        "name": "cargo_build",
        "description": "Build the project",
        "command": "cargo",
        "enabled": true
    }"#;

    let config: ToolConfig = serde_json::from_str(json).expect("normal tool should deserialize");
    // tool_type is None (omitted) — treated as Command
    assert!(
        config.tool_type.is_none() || config.tool_type == Some(ToolType::Command),
        "expected Command or absent tool_type"
    );
    assert!(
        config.livelog.is_none(),
        "no livelog block for command tools"
    );
}

// ---------------------------------------------------------------------------
// Missing required LivelogConfig fields are rejected
// ---------------------------------------------------------------------------

#[test]
fn test_livelog_config_missing_source_command_fails() {
    let json = r#"{
        "source_args": ["-f", "app.log"],
        "detection_prompt": "Look for errors",
        "llm_provider": {
            "base_url": "http://localhost:11434/v1",
            "model": "llama3.2"
        }
    }"#;

    let result = serde_json::from_str::<LivelogConfig>(json);
    assert!(result.is_err(), "should fail without source_command");
}

#[test]
fn test_livelog_config_missing_detection_prompt_fails() {
    let json = r#"{
        "source_command": "tail",
        "llm_provider": {
            "base_url": "http://localhost:11434/v1",
            "model": "llama3.2"
        }
    }"#;

    let result = serde_json::from_str::<LivelogConfig>(json);
    assert!(result.is_err(), "should fail without detection_prompt");
}

#[test]
fn test_livelog_config_missing_llm_provider_fails() {
    let json = r#"{
        "source_command": "tail",
        "source_args": ["-f", "app.log"],
        "detection_prompt": "Look for errors"
    }"#;

    let result = serde_json::from_str::<LivelogConfig>(json);
    assert!(result.is_err(), "should fail without llm_provider");
}

// ---------------------------------------------------------------------------
// tool_type = livelog deserializes from snake_case JSON value
// ---------------------------------------------------------------------------

#[test]
fn test_tool_type_livelog_deserialization() {
    let json = r#""livelog""#;
    let tt: ToolType = serde_json::from_str(json).expect("should parse livelog");
    assert_eq!(tt, ToolType::Livelog);
}

#[test]
fn test_tool_type_command_deserialization() {
    let json = r#""command""#;
    let tt: ToolType = serde_json::from_str(json).expect("should parse command");
    assert_eq!(tt, ToolType::Command);
}

// ---------------------------------------------------------------------------
// Load from actual bundled config files
// ---------------------------------------------------------------------------

#[test]
fn test_android_logcat_json_loads() {
    let json = include_str!("../../.ahma/android-logcat.json");
    let config: ToolConfig = serde_json::from_str(json).expect("android-logcat.json should parse");
    assert_eq!(config.name, "android-logcat");
    assert_eq!(config.tool_type, Some(ToolType::Livelog));
    let lc = config.livelog.expect("livelog block required");
    assert_eq!(lc.source_command, "adb");
    assert!(!lc.detection_prompt.is_empty());
}

#[test]
fn test_rust_log_monitor_json_loads() {
    let json = include_str!("../../.ahma/rust-log-monitor.json");
    let config: ToolConfig =
        serde_json::from_str(json).expect("rust-log-monitor.json should parse");
    assert_eq!(config.name, "rust-log-monitor");
    assert_eq!(config.tool_type, Some(ToolType::Livelog));
    let lc = config.livelog.expect("livelog block required");
    assert_eq!(lc.source_command, "tail");
    assert!(lc.source_args.contains(&"-F".to_string()));
    assert!(!lc.detection_prompt.is_empty());
}

// ---------------------------------------------------------------------------
// Edge cases: zero-valued tunables
// ---------------------------------------------------------------------------

#[test]
fn test_livelog_config_zero_chunk_max_lines() {
    let json = r#"{
        "source_command": "tail",
        "source_args": ["-f", "app.log"],
        "detection_prompt": "Look for errors",
        "llm_provider": {
            "base_url": "http://localhost:11434/v1",
            "model": "llama3.2"
        },
        "chunk_max_lines": 0
    }"#;

    let lc: LivelogConfig = serde_json::from_str(json).expect("zero chunk_max_lines should parse");
    assert_eq!(lc.chunk_max_lines, 0);
}

#[test]
fn test_livelog_config_zero_cooldown() {
    let json = r#"{
        "source_command": "tail",
        "source_args": ["-f", "app.log"],
        "detection_prompt": "Look for errors",
        "llm_provider": {
            "base_url": "http://localhost:11434/v1",
            "model": "llama3.2"
        },
        "cooldown_seconds": 0
    }"#;

    let lc: LivelogConfig = serde_json::from_str(json).expect("zero cooldown should parse");
    assert_eq!(lc.cooldown_seconds, 0);
}

// ---------------------------------------------------------------------------
// Invalid LLM provider config
// ---------------------------------------------------------------------------

#[test]
fn test_llm_provider_missing_base_url_fails() {
    let json = r#"{
        "model": "llama3.2"
    }"#;

    let result = serde_json::from_str::<LlmProviderConfig>(json);
    assert!(result.is_err(), "should fail without base_url");
}

#[test]
fn test_llm_provider_missing_model_fails() {
    let json = r#"{
        "base_url": "http://localhost:11434/v1"
    }"#;

    let result = serde_json::from_str::<LlmProviderConfig>(json);
    assert!(result.is_err(), "should fail without model");
}

// ---------------------------------------------------------------------------
// tool_type = livelog without livelog block deserializes (handler catches this)
// ---------------------------------------------------------------------------

#[test]
fn test_livelog_tool_type_without_block_deserializes() {
    let json = r#"{
        "name": "broken_livelog",
        "description": "Missing livelog block",
        "command": "tail",
        "tool_type": "livelog",
        "enabled": true
    }"#;

    let config: ToolConfig =
        serde_json::from_str(json).expect("should deserialize even without livelog block");
    assert_eq!(config.tool_type, Some(ToolType::Livelog));
    assert!(config.livelog.is_none(), "livelog block should be absent");
}
