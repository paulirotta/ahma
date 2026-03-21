use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

use ahma_mcp::{schema_validation::MtdfValidator, utils::logging::init_test_logging};

#[tokio::test]
async fn test_error_message_quality_and_helpfulness() -> Result<()> {
    init_test_logging();
    let validator = MtdfValidator::new();

    let common_mistakes = vec![
        (
            r#"{"name": test, "description": "desc", "command": "cmd"}"#.to_string(),
            "Invalid JSON",
        ),
        (
            r#"{"name": 123, "description": "desc", "command": "cmd"}"#.to_string(),
            "invalid type: integer",
        ),
        (
            json!({
                "name": "test",
                "description": "Test tool",
                "command": "test",
                "subcommand": [{
                    "name": "run",
                    "description": "Runs test",
                    "options": [{
                        "name": "verbose",
                        "type": "bool",
                        "description": "Verbose output"
                    }]
                }]
            })
            .to_string(),
            "Use 'boolean' instead of 'bool'",
        ),
        (
            json!({
                "name": "async_test",
                "description": "Async test tool",
                "command": "async_test",
                "subcommand": [{
                    "name": "build",
                    "description": "Asynchronously builds stuff",
                    "force_synchronous": true
                }]
            })
            .to_string(),
            "Logical inconsistency",
        ),
    ];

    for (config, expected_error_content) in common_mistakes {
        let result = validator.validate_tool_config(&PathBuf::from("mistake.json"), &config);
        assert!(result.is_err(), "Config should be invalid: {}", config);

        let errors = result.unwrap_err();
        let error_report = validator.format_errors(&errors, &PathBuf::from("mistake.json"));

        assert!(
            error_report.contains(expected_error_content),
            "Error report should contain '{}', but got: {}",
            expected_error_content,
            error_report
        );

        if !expected_error_content.contains("Invalid JSON")
            && !expected_error_content.contains("invalid type")
        {
            assert!(
                error_report.contains("Suggestion:") || error_report.contains("Common fixes:"),
                "Error report should contain suggestions: {}",
                error_report
            );
        }
    }

    let multi_error_config = json!({
        "description": 123,
        "command": "",
        "timeout_seconds": -1,
        "unknown_field": "value"
    })
    .to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("multi_error.json"), &multi_error_config);
    assert!(result.is_err());

    let errors = result.unwrap_err();
    let error_report = validator.format_errors(&errors, &PathBuf::from("multi_error.json"));

    assert!(error_report.contains("Found") && error_report.contains("error(s):"));
    assert!(error_report.contains("docs/tool-schema-guide.md"));
    assert!(error_report.contains("Common fixes:"));
    assert!(error_report.contains("1.") || error_report.contains("1 "));

    Ok(())
}