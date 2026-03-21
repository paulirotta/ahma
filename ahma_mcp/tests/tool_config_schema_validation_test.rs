//! Tool Configuration Schema Validation Tests
//!
//! This test module validates that all tool configuration files in the `.ahma/` directory
//! conform to the MTDF schema and are correctly structured.

use ahma_mcp::schema_validation::MtdfValidator;
use std::path::PathBuf;

/// Helper function to get the path to a config file
fn get_config_path(config_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".ahma")
        .join(format!("{}.json", config_name))
}

/// Helper function to validate a tool configuration
fn validate_tool_config(config_name: &str) -> Result<(), String> {
    let config_path = get_config_path(config_name);
    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read {}: {}", config_name, e))?;

    let validator = MtdfValidator::new();
    validator
        .validate_tool_config(&config_path, &content)
        .map(|_| ())
        .map_err(|errors| {
            let error_msgs: Vec<String> = errors
                .iter()
                .map(|e| format!("  - {}: {}", e.field_path, e.message))
                .collect();
            format!(
                "Validation failed for {}:\n{}",
                config_name,
                error_msgs.join("\n")
            )
        })
}

#[test]
fn test_rust_config_schema_validation() {
    validate_tool_config("rust").expect("rust.json should pass schema validation");
}

#[test]
fn test_file_tools_config_schema_validation() {
    validate_tool_config("file-tools").expect("file-tools.json should pass schema validation");
}

#[test]
fn test_gh_config_schema_validation() {
    validate_tool_config("gh").expect("gh.json should pass schema validation");
}

#[test]
fn test_git_config_schema_validation() {
    validate_tool_config("git").expect("git.json should pass schema validation");
}

#[test]
fn test_kotlin_config_schema_validation() {
    validate_tool_config("kotlin").expect("kotlin.json should pass schema validation");
}

#[test]
fn test_python_config_schema_validation() {
    validate_tool_config("python").expect("python.json should pass schema validation");
}

#[test]
fn test_all_configs_are_enabled() {
    let config_names = ["rust", "file-tools", "gh", "git", "kotlin", "python"];

    for config_name in &config_names {
        let config_path = get_config_path(config_name);
        let content = std::fs::read_to_string(&config_path)
            .unwrap_or_else(|_| panic!("Failed to read {}.json", config_name));

        let json: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|_| panic!("Failed to parse {}.json", config_name));

        let enabled = json
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| panic!("{}.json should have 'enabled' field", config_name));

        assert!(
            enabled,
            "{}.json should be enabled (found enabled=false)",
            config_name
        );
    }
}

#[test]
fn test_all_configs_have_valid_structure() {
    let config_names = ["rust", "file-tools", "gh", "git", "kotlin", "python"];
    let validator = MtdfValidator::new();

    for config_name in &config_names {
        let config_path = get_config_path(config_name);
        let content = std::fs::read_to_string(&config_path)
            .unwrap_or_else(|_| panic!("Failed to read {}.json", config_name));

        let config = validator
            .validate_tool_config(&config_path, &content)
            .unwrap_or_else(|_| panic!("{}.json should be valid", config_name));

        // Verify essential fields
        assert!(
            !config.name.is_empty(),
            "{}: name should not be empty",
            config_name
        );
        assert!(
            !config.command.is_empty(),
            "{}: command should not be empty",
            config_name
        );
        assert!(
            !config.description.is_empty(),
            "{}: description should not be empty",
            config_name
        );
        assert!(
            config.subcommand.as_ref().is_some_and(|s| !s.is_empty()),
            "{}: should have at least one subcommand",
            config_name
        );
    }
}
