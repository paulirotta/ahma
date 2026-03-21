use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

use ahma_mcp::{
    schema_validation::{MtdfValidator, ValidationErrorType},
    utils::logging::init_test_logging,
};

#[tokio::test]
async fn test_field_validation_edge_cases() -> Result<()> {
    init_test_logging();
    let validator = MtdfValidator::new();

    let all_types_config = json!({
        "name": "all_types_test",
        "description": "Test all valid option types",
        "command": "types",
        "subcommand": [{
            "name": "type_test",
            "description": "Tests all types. Synchronous operation returns results immediately.",
            "force_synchronous": true,
            "options": [
                {"name": "bool_option", "type": "boolean", "description": "Boolean option"},
                {"name": "str_option", "type": "string", "description": "String option"},
                {"name": "int_option", "type": "integer", "description": "Integer option"},
                {"name": "array_option", "type": "array", "description": "Array option"}
            ]
        }]
    })
    .to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("all_types.json"), &all_types_config);
    assert!(result.is_ok(), "All valid types config should pass: {:?}", result);

    let type_mistakes = vec![("bool", "boolean"), ("int", "integer"), ("str", "string")];

    for (wrong_type, correct_type) in type_mistakes {
        let mistake_config = json!({
            "name": "type_mistake",
            "description": "Test type mistake",
            "command": "mistake",
            "subcommand": [{
                "name": "test",
                "description": "Synchronous test operation",
                "force_synchronous": true,
                "options": [{
                    "name": "test_option",
                    "type": wrong_type,
                    "description": "Test option"
                }]
            }]
        })
        .to_string();

        let result =
            validator.validate_tool_config(&PathBuf::from("mistake.json"), &mistake_config);
        assert!(result.is_err(), "Wrong type '{}' should be invalid", wrong_type);

        let errors = result.unwrap_err();
        let has_helpful_suggestion = errors.iter().any(|e| {
            e.error_type == ValidationErrorType::InvalidValue
                && e.suggestion
                    .as_ref()
                    .is_some_and(|s| s.contains(correct_type))
        });
        assert!(
            has_helpful_suggestion,
            "Should have helpful suggestion for '{}' -> '{}'",
            wrong_type,
            correct_type
        );
    }

    Ok(())
}