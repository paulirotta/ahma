use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

use ahma_mcp::{
    schema_validation::{MtdfValidator, ValidationErrorType},
    utils::logging::init_test_logging,
};

#[tokio::test]
async fn test_validator_configuration_options() -> Result<()> {
    init_test_logging();

    let config_with_unknown_fields = json!({
        "name": "unknown_fields_test",
        "description": "Test unknown fields handling",
        "command": "unknown",
        "unknown_root_field": "value",
        "subcommand": [{
            "name": "test_sub",
            "description": "Test subcommand - async returns id immediately, results pushed via notification when complete, continue with other tasks",
            "unknown_sub_field": "value"
        }]
    })
    .to_string();

    let strict_validator = MtdfValidator::new()
        .with_strict_mode(true)
        .with_unknown_fields_allowed(false);
    let strict_result = strict_validator
        .validate_tool_config(&PathBuf::from("unknown.json"), &config_with_unknown_fields);
    assert!(strict_result.is_err());
    let strict_errors = strict_result.unwrap_err();
    assert!(strict_errors.iter().any(|e| {
        e.error_type == ValidationErrorType::SchemaViolation
            && e.message.contains("unknown field")
            && (e.message.contains("unknown_root_field")
                || e.message.contains("unknown_sub_field"))
    }));

    let permissive_validator = MtdfValidator::new()
        .with_strict_mode(false)
        .with_unknown_fields_allowed(true);
    let permissive_result = permissive_validator
        .validate_tool_config(&PathBuf::from("unknown.json"), &config_with_unknown_fields);

    if let Err(errors) = &permissive_result {
        let unknown_field_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.error_type == ValidationErrorType::UnknownField)
            .collect();
        assert!(
            unknown_field_errors.is_empty(),
            "Permissive mode should not report unknown field errors: {:?}",
            unknown_field_errors
        );
    }

    let lenient_validator = MtdfValidator::new().with_strict_mode(false);
    let _ = lenient_validator
        .validate_tool_config(&PathBuf::from("unknown.json"), &config_with_unknown_fields);

    Ok(())
}