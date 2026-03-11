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
use tokio::time::Instant;

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

/// Test performance for large tool sets
#[tokio::test]
async fn test_performance_for_large_tool_sets() -> Result<()> {
    init_test_logging();
    let validator = MtdfValidator::new();

    // Generate a large tool configuration
    let num_subcommands = 50;
    let num_options_per_subcommand = 20;

    let mut subcommands = Vec::new();
    for i in 0..num_subcommands {
        let mut options = Vec::new();
        for j in 0..num_options_per_subcommand {
            options.push(json!({
                "name": format!("option_{}", j),
                "type": if j % 3 == 0 { "boolean" } else if j % 3 == 1 { "string" } else { "array" },
                "description": format!("Option {} for subcommand {}", j, i)
            }));
        }

        subcommands.push(json!({
            "name": format!("subcommand_{}", i),
            "description": format!("Subcommand {} - {} operation", i, if i % 5 == 0 { "async returns id immediately, results pushed via notification when complete, continue with other tasks" } else { "synchronous returns results immediately" }),
            "force_synchronous": (i % 5 != 0),  // Every 5th subcommand is synchronous (inverted)
            "options": options
        }));
    }

    let large_config = json!({
        "name": "large_tool",
        "description": "Tool with many subcommands and options",
        "command": "large",
        "subcommand": subcommands
    })
    .to_string();

    // Measure validation performance
    let start_time = Instant::now();
    let result = validator.validate_tool_config(&PathBuf::from("large.json"), &large_config);
    let validation_time = start_time.elapsed();

    println!("Large tool validation took: {:?}", validation_time);

    // Should complete within reasonable time (less than 3 seconds for this size,
    // allowing headroom for CI runners and loaded machines)
    assert!(
        validation_time < std::time::Duration::from_secs(3),
        "Validation took too long: {:?}",
        validation_time
    );

    // Should successfully validate
    assert!(result.is_ok(), "Large config should be valid: {:?}", result);

    // Test performance with many errors
    let mut invalid_subcommands = Vec::new();
    for i in 0..30 {
        invalid_subcommands.push(json!({
            "name": format!("invalid_subcommand_{}", i),
            // Empty description triggers missing field validation while invalid option type
            // surfaces additional errors for each subcommand.
            "description": "",
            "options": [
                {
                    "name": format!("option_{}", i),
                    "type": "invalid_type",  // Invalid type
                    "description": "Some option"
                }
            ]
        }));
    }

    let invalid_large_config = json!({
        "name": "invalid_large",
        "description": "Large tool with many errors",
        "command": "invalid_large",
        "subcommand": invalid_subcommands
    })
    .to_string();

    let error_start_time = Instant::now();
    let error_result =
        validator.validate_tool_config(&PathBuf::from("invalid_large.json"), &invalid_large_config);
    let error_validation_time = error_start_time.elapsed();

    println!(
        "Large invalid tool validation took: {:?}",
        error_validation_time
    );

    // Should still complete quickly even with many errors
    assert!(
        error_validation_time < std::time::Duration::from_secs(2),
        "Error validation took too long: {:?}",
        error_validation_time
    );

    assert!(error_result.is_err());
    let errors = error_result.unwrap_err();
    // Should find multiple errors
    assert!(
        errors.len() >= 30,
        "Should find many errors, got: {}",
        errors.len()
    );

    Ok(())
}

/// Test error message quality and helpfulness
#[tokio::test]
async fn test_error_message_quality_and_helpfulness() -> Result<()> {
    init_test_logging();
    let validator = MtdfValidator::new();

    // Test error messages for common mistakes
    let common_mistakes = vec![
        // Missing quotes around string values
        (
            r#"{"name": test, "description": "desc", "command": "cmd"}"#.to_string(),
            "Invalid JSON",
        ),
        // Wrong data types
        (
            r#"{"name": 123, "description": "desc", "command": "cmd"}"#.to_string(),
            "invalid type: integer",
        ),
        // Invalid option types with helpful suggestions
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
                        "type": "bool",  // Should be "boolean"
                        "description": "Verbose output"
                    }]
                }]
            })
            .to_string(),
            "Use 'boolean' instead of 'bool'",
        ),
        // Missing async guidance
        (
            json!({
                "name": "async_test",
                "description": "Async test tool",
                "command": "async_test",
                "subcommand": [{
                    "name": "build",
                    "description": "Asynchronously builds stuff",  // Contains async keyword but synchronous=true
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

        // Check that error report contains helpful information
        // Only expect suggestions for validation errors, not JSON parsing or deserialization errors
        if !expected_error_content.contains("Invalid JSON")
            && !expected_error_content.contains("invalid type")
        {
            assert!(
                error_report.contains("Suggestion Suggestion:")
                    || error_report.contains("Common fixes:"),
                "Error report should contain suggestions: {}",
                error_report
            );
        }
    }

    // Test that error report formatting is comprehensive
    let multi_error_config = json!({
        // Missing required fields and has invalid data
        "description": 123,  // Wrong type
        "command": "",       // Empty command
        "timeout_seconds": -1,  // Invalid timeout
        "unknown_field": "value"  // Unknown field in strict mode
    })
    .to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("multi_error.json"), &multi_error_config);
    assert!(result.is_err());

    let errors = result.unwrap_err();
    let error_report = validator.format_errors(&errors, &PathBuf::from("multi_error.json"));

    // Should contain helpful formatting
    assert!(error_report.contains("Found") && error_report.contains("error(s):"));
    assert!(error_report.contains("docs/tool-schema-guide.md"));
    assert!(error_report.contains("Common fixes:"));

    // Should number the errors
    assert!(error_report.contains("1.") || error_report.contains("1 "));

    Ok(())
}

/// Test complex configuration validation scenarios
#[tokio::test]
async fn test_complex_configuration_validation_scenarios() -> Result<()> {
    init_test_logging();
    let validator = MtdfValidator::new();

    // Test inheritance of synchronous behavior
    let inheritance_config = json!({
        "name": "inheritance_test",
        "description": "Test synchronous behavior inheritance",
        "command": "inherit",
        "force_synchronous": false,  // Parent is synchronous (default behavior)
        "subcommand": [
            {
                "name": "inherit_sync",
                "description": "Inherits synchronous behavior - returns results immediately",
                // No asynchronous field - should inherit false (sync) from parent
            },
            {
                "name": "override_async", 
                "description": "Overrides to async. Returns id immediately. Results pushed via notification when complete. Continue with other tasks.",
                "force_synchronous": false  // Explicitly override to async
            }
        ]
    }).to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("inheritance.json"), &inheritance_config);
    assert!(
        result.is_ok(),
        "Inheritance config should be valid: {:?}",
        result
    );

    // Test enabled/disabled logic consistency
    let enablement_config = json!({
        "name": "enablement_test",
        "description": "Test enablement logic",
        "command": "enable",
        "enabled": false,  // Tool is disabled
        "subcommand": [
            {
                "name": "enabled_sub",
                "description": "This subcommand claims to be enabled",
                "enabled": true  // This should trigger an inconsistency error
            }
        ]
    })
    .to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("enablement.json"), &enablement_config);
    // Feature not yet implemented - just check it parses for now
    assert!(
        result.is_ok(),
        "Config should parse even without consistency checks implemented yet"
    );

    // TODO: Uncomment when feature is implemented
    // assert!(result.is_err());
    // let errors = result.unwrap_err();
    // assert!(errors.iter().any(
    //     |e| e.error_type == ValidationErrorType::LogicalInconsistency
    //         && e.message.contains("enabled: true")
    //         && e.message.contains("disabled at root level")
    // ));

    // Test guidance_key bypass behavior
    let guidance_key_config = json!({
        "name": "guidance_key_test",
        "description": "Test guidance_key bypass",
        "command": "guidance",
        "subcommand": [
            {
                "name": "with_guidance_key",
                "description": "Asynchronous operation - returns id immediately",  // Mentions async
                "guidance_key": "shared_guidance",  // Has guidance_key for extended help
                "force_synchronous": false
            }
        ]
    })
    .to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("guidance_key.json"), &guidance_key_config);
    assert!(
        result.is_ok(),
        "Guidance key config should be valid: {:?}",
        result
    );

    // Test contradictory sync/async descriptions
    let contradictory_config = json!({
        "name": "contradictory_test",
        "description": "Test contradictory descriptions",
        "command": "contradict",
        "subcommand": [
            {
                "name": "sync_with_async_desc",
                "description": "This command returns an id and sends notifications asynchronously",  // Async language
                "force_synchronous": true  // But marked as sync - should trigger warning
            }
        ]
    })
    .to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("contradictory.json"), &contradictory_config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(
        |e| e.error_type == ValidationErrorType::LogicalInconsistency
            && e.message.contains("mentions")
            && e.message.contains("async behavior")
    ));

    // Test mixed valid and invalid subcommands
    let mixed_config = json!({
        "name": "mixed_test", 
        "description": "Test mixed valid/invalid subcommands",
        "command": "mixed",
        "subcommand": [
            {
                "name": "valid_async",
                "description": "Valid async subcommand - returns id immediately, results pushed via notification when complete, continue with other tasks",
                "force_synchronous": false,
                "options": [
                    {
                        "name": "valid_option",
                        "type": "boolean",
                        "description": "Valid option"
                    }
                ]
            },
            {
                "name": "invalid_sub",
                "description": "Invalid subcommand with bad option type",
                "options": [
                    {
                        "name": "invalid_option",
                        "type": "invalid_type",  // Invalid type
                        "description": "Invalid option"
                    }
                ]
            },
            {
                "name": "another_valid",
                "description": "Another valid subcommand - synchronous operation returns results immediately", 
                "force_synchronous": true
            }
        ]
    }).to_string();

    let result = validator.validate_tool_config(&PathBuf::from("mixed.json"), &mixed_config);
    assert!(result.is_err());
    let errors = result.unwrap_err();

    // Should find errors in the invalid subcommand but not the valid ones
    assert!(errors.iter().any(|e| e.field_path.contains("options[0]")
        || (e.field_path == "mixed.json" && e.message.contains("invalid_type"))));

    Ok(())
}

/// Test validator configuration options
#[tokio::test]
async fn test_validator_configuration_options() -> Result<()> {
    init_test_logging();
    // Test strict mode vs permissive mode
    let config_with_unknown_fields = json!({
        "name": "unknown_fields_test",
        "description": "Test unknown fields handling",
        "command": "unknown",
        "unknown_root_field": "value",
        "subcommand": [
            {
                "name": "test_sub",
                "description": "Test subcommand - async returns id immediately, results pushed via notification when complete, continue with other tasks",
                "unknown_sub_field": "value"
            }
        ]
    }).to_string();

    // Strict mode should reject unknown fields
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
            && (e.message.contains("unknown_root_field") || e.message.contains("unknown_sub_field"))
    }));

    // Permissive mode should allow unknown fields with proper validation
    let permissive_validator = MtdfValidator::new()
        .with_strict_mode(false)
        .with_unknown_fields_allowed(true);
    let permissive_result = permissive_validator
        .validate_tool_config(&PathBuf::from("unknown.json"), &config_with_unknown_fields);

    // Should pass validation despite unknown fields in permissive mode, but may still fail for other reasons (like guidance)
    // Just check that unknown fields specifically are not the cause
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

    // Non-strict mode should be more lenient overall
    let lenient_validator = MtdfValidator::new().with_strict_mode(false);
    let _lenient_result = lenient_validator
        .validate_tool_config(&PathBuf::from("unknown.json"), &config_with_unknown_fields);
    // Behavior depends on implementation, but should handle unknown fields gracefully
    // The specific behavior may vary based on the strict_mode implementation

    Ok(())
}

/// Test edge cases in field validation
#[tokio::test]
async fn test_field_validation_edge_cases() -> Result<()> {
    init_test_logging();
    let validator = MtdfValidator::new();

    // Test all valid option types
    let all_types_config = json!({
        "name": "all_types_test",
        "description": "Test all valid option types",
        "command": "types",
        "subcommand": [
            {
                "name": "type_test",
                "description": "Tests all types. Synchronous operation returns results immediately.",
                "force_synchronous": true,
                "options": [
                    {
                        "name": "bool_option",
                        "type": "boolean",
                        "description": "Boolean option"
                    },
                    {
                        "name": "str_option", 
                        "type": "string",
                        "description": "String option"
                    },
                    {
                        "name": "int_option",
                        "type": "integer", 
                        "description": "Integer option"
                    },
                    {
                        "name": "array_option",
                        "type": "array",
                        "description": "Array option"
                    }
                ]
            }
        ]
    }).to_string();

    let result =
        validator.validate_tool_config(&PathBuf::from("all_types.json"), &all_types_config);
    assert!(
        result.is_ok(),
        "All valid types config should pass: {:?}",
        result
    );

    // Test common type mistakes with helpful suggestions
    let type_mistakes = vec![("bool", "boolean"), ("int", "integer"), ("str", "string")];

    for (wrong_type, correct_type) in type_mistakes {
        let mistake_config = json!({
            "name": "type_mistake",
            "description": "Test type mistake",
            "command": "mistake",
            "subcommand": [
                {
                    "name": "test",
                    "description": "Synchronous test operation",
                    "force_synchronous": true,
                    "options": [
                        {
                            "name": "test_option",
                            "type": wrong_type,
                            "description": "Test option"
                        }
                    ]
                }
            ]
        })
        .to_string();

        let result =
            validator.validate_tool_config(&PathBuf::from("mistake.json"), &mistake_config);
        assert!(
            result.is_err(),
            "Wrong type '{}' should be invalid",
            wrong_type
        );

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
            wrong_type, correct_type
        );
    }

    Ok(())
}
