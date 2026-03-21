//! Comprehensive schema validation testing for Phase 7 requirements.
//!
//! **NOTE:** Most tests in this file are currently ignored as they test
//! features not yet implemented in schema_validation.rs. These are aspirational
//! TDD tests that define the behavior we want to implement in Phase 7.
//!
//! This test module targets:
//! - MTDF compliance edge cases  
//! - Recursive subcommand validation
//! - Performance for large tool sets
//! - Error message quality and helpfulness
//! - Complex configuration validation scenarios

use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

use ahma_mcp::{
    schema_validation::{MtdfValidator, ValidationErrorType},
    utils::logging::init_test_logging,
};

/// Test MTDF compliance edge cases
#[tokio::test]
async fn test_mtdf_compliance_edge_cases() -> Result<()> {
    init_test_logging();
    let validator = MtdfValidator::new();

    // Test minimal valid configuration
    let minimal_config = json!({
        "name": "minimal",
        "description": "Minimal tool",
        "command": "echo"
    })
    .to_string();

    let result = validator.validate_tool_config(&PathBuf::from("minimal.json"), &minimal_config);
    assert!(result.is_ok(), "Minimal config should be valid");

    // Test configuration with all optional fields
    let maximal_config = json!({
        "name": "maximal",
        "description": "Maximal tool with all features",
        "command": "complex_tool",
        "enabled": true,
        "timeout_seconds": 600,
        "force_synchronous": false,
        "guidance_key": "complex_guidance",
        "hints": {
            "build": "Use for building projects",
            "test": "Use for testing projects",
            "custom": {
                "default": "This is a complex tool"
            }
        },
        "subcommand": [
            {
                "name": "build",
                "description": "Build project - async operation returns id immediately, results pushed via notification when complete, continue with other tasks",
                "enabled": true,
                "force_synchronous": false,
                "guidance_key": "build_guidance",
                "options": [
                    {
                        "name": "release",
                        "type": "boolean",
                        "description": "Build in release mode"
                    },
                    {
                        "name": "target",
                        "type": "string", 
                        "description": "Target architecture"
                    },
                    {
                        "name": "features",
                        "type": "array",
                        "description": "Features to enable"
                    }
                ],
                "positional_args": [
                    {
                        "name": "project_path",
                        "type": "string",
                        "description": "Path to project"
                    }
                ]
            }
        ]
    }).to_string();

    let result = validator.validate_tool_config(&PathBuf::from("maximal.json"), &maximal_config);
    assert!(
        result.is_ok(),
        "Maximal config should be valid: {:?}",
        result
    );

    // Test edge case: empty command
    let empty_command = json!({
        "name": "empty_cmd",
        "description": "Tool with empty command",
        "command": ""
    })
    .to_string();

    let result = validator.validate_tool_config(&PathBuf::from("empty_cmd.json"), &empty_command);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.field_path == "command"
        && e.error_type == ValidationErrorType::ConstraintViolation
        && e.message.contains("cannot be empty")));

    // Test edge case: extreme timeouts
    let extreme_timeout = json!({
        "name": "extreme",
        "description": "Tool with extreme timeout",
        "command": "slow_tool",
        "timeout_seconds": 7200  // 2 hours
    })
    .to_string();

    let result = validator.validate_tool_config(&PathBuf::from("extreme.json"), &extreme_timeout);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.field_path == "timeout_seconds"
        && e.error_type == ValidationErrorType::ConstraintViolation
        && e.message.contains("should not exceed 3600")));

    // Test edge case: zero timeout
    let zero_timeout = json!({
        "name": "zero",
        "description": "Tool with zero timeout",
        "command": "instant_tool",
        "timeout_seconds": 0
    })
    .to_string();

    let result = validator.validate_tool_config(&PathBuf::from("zero.json"), &zero_timeout);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.field_path == "timeout_seconds"
        && e.error_type == ValidationErrorType::ConstraintViolation
        && e.message.contains("should be at least 1")));

    Ok(())
}

/// Test recursive subcommand validation
#[tokio::test]
async fn test_recursive_subcommand_validation() -> Result<()> {
    init_test_logging();
    let validator = MtdfValidator::new();

    // Test deeply nested subcommands
    let nested_config = json!({
        "name": "nested_tool",
        "description": "Tool with nested subcommands",
        "command": "nested",
        "subcommand": [
            {
                "name": "level1",
                "description": "First level command - synchronous operation returns results immediately",
                "force_synchronous": true,
                "subcommand": [
                    {
                        "name": "level2",
                        "description": "Second level command - async operation returns id immediately, results pushed via notification when complete, continue with other tasks",
                        "force_synchronous": false,
                        "subcommand": [
                            {
                                "name": "level3",
                                "description": "Third level command - quick synchronous operation returns results immediately",
                                "force_synchronous": true,
                                "options": [
                                    {
                                        "name": "deep_option",
                                        "type": "boolean",
                                        "description": "Deep nested option"
                                    }
                                ]
                            }
                        ]
                    }
                ]
            }
        ]
    }).to_string();

    let result = validator.validate_tool_config(&PathBuf::from("nested.json"), &nested_config);
    assert!(
        result.is_ok(),
        "Nested config should be valid: {:?}",
        result
    );

    // Test invalid nested structure - missing required fields
    let invalid_nested = json!({
        "name": "invalid_nested",
        "description": "Tool with invalid nested structure",
        "command": "invalid",
        "subcommand": [
            {
                "name": "parent",
                "description": "Parent command - async operation with proper guidance returns id immediately, results pushed via notification when complete, continue with other tasks",
                "subcommand": [
                    {
                        // Intentionally empty to trigger validation on required fields
                        "name": "",
                        "description": ""
                    }
                ]
            }
        ]
    }).to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("invalid_nested.json"), &invalid_nested);
    assert!(result.is_err());
    let errors = result.unwrap_err();

    // The validator should catch missing required fields in nested subcommands
    // Note: due to implementation details, nested subcommand errors may not have fully qualified paths
    // The error message uses capitalized field names (e.g., "Name cannot be empty")
    assert!(errors.iter().any(
        |e| e.error_type == ValidationErrorType::MissingRequiredField && e.message.contains("Name")
    ));

    // Test circular or self-referential structure (malformed JSON test)
    let malformed_nested = json!({
        "name": "malformed",
        "description": "Malformed nested structure",
        "command": "malformed",
        "subcommand": [
            {
                "name": "parent",
                "description": "Parent command",
                "subcommand": "not_an_array"  // Should be array
            }
        ]
    })
    .to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("malformed.json"), &malformed_nested);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| {
        e.error_type == ValidationErrorType::SchemaViolation && e.message.contains("invalid type")
    }));

    Ok(())
}
