use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

use ahma_mcp::{
    schema_validation::{MtdfValidator, ValidationErrorType},
    utils::logging::init_test_logging,
};

#[tokio::test]
async fn test_complex_configuration_validation_scenarios() -> Result<()> {
    init_test_logging();
    let validator = MtdfValidator::new();

    let inheritance_config = json!({
        "name": "inheritance_test",
        "description": "Test synchronous behavior inheritance",
        "command": "inherit",
        "force_synchronous": false,
        "subcommand": [
            {
                "name": "inherit_sync",
                "description": "Inherits synchronous behavior - returns results immediately"
            },
            {
                "name": "override_async",
                "description": "Overrides to async. Returns id immediately. Results pushed via notification when complete. Continue with other tasks.",
                "force_synchronous": false
            }
        ]
    })
    .to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("inheritance.json"), &inheritance_config);
    assert!(result.is_ok(), "Inheritance config should be valid: {:?}", result);

    let enablement_config = json!({
        "name": "enablement_test",
        "description": "Test enablement logic",
        "command": "enable",
        "enabled": false,
        "subcommand": [{
            "name": "enabled_sub",
            "description": "This subcommand claims to be enabled",
            "enabled": true
        }]
    })
    .to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("enablement.json"), &enablement_config);
    assert!(result.is_ok(), "Config should parse without enablement checks yet");

    let guidance_key_config = json!({
        "name": "guidance_key_test",
        "description": "Test guidance_key bypass",
        "command": "guidance",
        "subcommand": [{
            "name": "with_guidance_key",
            "description": "Asynchronous operation - returns id immediately",
            "guidance_key": "shared_guidance",
            "force_synchronous": false
        }]
    })
    .to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("guidance_key.json"), &guidance_key_config);
    assert!(result.is_ok(), "Guidance key config should be valid: {:?}", result);

    let contradictory_config = json!({
        "name": "contradictory_test",
        "description": "Test contradictory descriptions",
        "command": "contradict",
        "subcommand": [{
            "name": "sync_with_async_desc",
            "description": "This command returns an id and sends notifications asynchronously",
            "force_synchronous": true
        }]
    })
    .to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("contradictory.json"), &contradictory_config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        e.error_type == ValidationErrorType::LogicalInconsistency
            && e.message.contains("mentions")
            && e.message.contains("async behavior")
    }));

    let mixed_config = json!({
        "name": "mixed_test",
        "description": "Test mixed valid/invalid subcommands",
        "command": "mixed",
        "subcommand": [
            {
                "name": "valid_async",
                "description": "Valid async subcommand - returns id immediately, results pushed via notification when complete, continue with other tasks",
                "force_synchronous": false,
                "options": [{
                    "name": "valid_option",
                    "type": "boolean",
                    "description": "Valid option"
                }]
            },
            {
                "name": "invalid_sub",
                "description": "Invalid subcommand with bad option type",
                "options": [{
                    "name": "invalid_option",
                    "type": "invalid_type",
                    "description": "Invalid option"
                }]
            },
            {
                "name": "another_valid",
                "description": "Another valid subcommand - synchronous operation returns results immediately",
                "force_synchronous": true
            }
        ]
    })
    .to_string();

    let result = validator.validate_tool_config(&PathBuf::from("mixed.json"), &mixed_config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        e.field_path.contains("options[0]")
            || (e.field_path == "mixed.json" && e.message.contains("invalid_type"))
    }));

    Ok(())
}