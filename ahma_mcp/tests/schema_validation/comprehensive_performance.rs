use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;
use tokio::time::Instant;

use ahma_mcp::{schema_validation::MtdfValidator, utils::logging::init_test_logging};

#[tokio::test]
async fn test_performance_for_large_tool_sets() -> Result<()> {
    init_test_logging();
    let validator = MtdfValidator::new();

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
            "force_synchronous": (i % 5 != 0),
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

    let start_time = Instant::now();
    let result = validator.validate_tool_config(&PathBuf::from("large.json"), &large_config);
    let validation_time = start_time.elapsed();

    assert!(
        validation_time < std::time::Duration::from_secs(3),
        "Validation took too long: {:?}",
        validation_time
    );
    assert!(result.is_ok(), "Large config should be valid: {:?}", result);

    let mut invalid_subcommands = Vec::new();
    for i in 0..30 {
        invalid_subcommands.push(json!({
            "name": format!("invalid_subcommand_{}", i),
            "description": "",
            "options": [{
                "name": format!("option_{}", i),
                "type": "invalid_type",
                "description": "Some option"
            }]
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

    assert!(
        error_validation_time < std::time::Duration::from_secs(2),
        "Error validation took too long: {:?}",
        error_validation_time
    );

    let errors = error_result.expect_err("invalid config should fail");
    assert!(errors.len() >= 30, "Should find many errors, got: {}", errors.len());

    Ok(())
}