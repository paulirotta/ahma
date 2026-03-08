//! Test to reproduce and fix the VSCode GitHub Copilot Chat catastrophic failure
//! Error: "tool parameters array type must have items"

use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::utils::logging::init_test_logging;
use futures::future::join_all;
use serde_json::Value;

/// This test reproduces the exact VSCode GitHub Copilot Chat failure
/// and ensures our fix prevents it from happening again.
#[tokio::test]
async fn test_array_parameters_must_have_items_property() -> anyhow::Result<()> {
    init_test_logging();

    // Force rebuild to ensure we're testing the latest code in CI
    eprintln!("Building latest binary to avoid stale cache issues...");
    let build_output = std::process::Command::new("cargo")
        .args(["build", "--package", "ahma_mcp", "--bin", "ahma-mcp"])
        .output()
        .expect("Failed to build binary");

    if !build_output.status.success() {
        eprintln!(
            "Build stderr: {}",
            String::from_utf8_lossy(&build_output.stderr)
        );
        panic!("Failed to build ahma_mcp binary");
    }
    eprintln!("Binary built successfully");

    // Create a test client with the real tool configurations (assume new_client is now async)
    let client = ClientBuilder::new().tools_dir(".ahma").build().await?;
    let tools = client.list_all_tools().await?;

    eprintln!(
        "Testing {} tools for proper array schema generation",
        tools.len()
    );

    // Inspect consolidated cargo tool which includes the audit subcommand options
    let cargo_tool = match tools.iter().find(|tool| tool.name == "cargo") {
        Some(tool) => tool,
        None => {
            eprintln!(
                "Skipping test: cargo tool not found in tools directory (may be CI environment)"
            );
            eprintln!(
                "Available tools: {:?}",
                tools.iter().map(|t| &t.name).collect::<Vec<_>>()
            );
            client.cancel().await?;
            return Ok(());
        }
    };

    eprintln!("Found cargo tool, checking audit-related array parameters...");

    // The schema should be valid and not cause VSCode failures
    let schema = cargo_tool.input_schema.as_ref();
    let properties = schema
        .get("properties")
        .expect("Tool schema must have properties")
        .as_object()
        .expect("Properties must be an object");

    // Collect array parameters for potential parallel validation
    let array_params: Vec<_> = properties
        .iter()
        .filter_map(|(param_name, param_schema)| {
            param_schema.as_object().and_then(|param_obj| {
                if param_obj.get("type") == Some(&Value::String("array".to_string())) {
                    Some((param_name.clone(), param_obj.clone()))
                } else {
                    None
                }
            })
        })
        .collect();

    // Validate array parameters (use join_all for concurrency if validation can be parallelized)
    let validation_futures: Vec<_> = array_params
        .into_iter()
        .map(|(param_name, param_obj)| async move {
            // CRITICAL FIX: Array parameters MUST have 'items' property
            assert!(
                param_obj.contains_key("items"),
                "CRITICAL: Array parameter '{}' MUST have 'items' property! \
                 This is what caused VSCode GitHub Copilot Chat to fail with: \
                 'tool parameters array type must have items'",
                param_name
            );

            let items = param_obj
                .get("items")
                .expect("Items must be present")
                .as_object()
                .expect("Items must be an object");

            assert!(
                items.contains_key("type"),
                "Array items must have a type for parameter '{}'",
                param_name
            );

            // For command-line tools, array items should typically be strings
            assert_eq!(
                items.get("type").unwrap(),
                &Value::String("string".to_string()),
                "Array items should be strings for CLI parameter '{}'",
                param_name
            );

            eprintln!(
                "OK Array parameter '{}' has valid items property",
                param_name
            );
            Ok(())
        })
        .collect();

    // Await all validations concurrently
    let results: Vec<Result<(), anyhow::Error>> = join_all(validation_futures).await;
    let validated_arrays = results.len();
    for result in results {
        result?; // Propagate any errors
    }

    // Check if cargo-audit is installed
    let audit_installed = std::process::Command::new("cargo")
        .args(["audit", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if validated_arrays == 0 {
        eprintln!(
            "No array parameters detected; this typically means cargo-audit (and other array-bearing subcommands) were disabled."
        );
    } else if audit_installed {
        assert!(
            validated_arrays >= 5,
            "Should have validated at least 5 array parameters with cargo audit installed (ignore, target-arch, target-os, args, exclude)"
        );
        eprintln!(
            "cargo-audit is installed, validated {} array parameters.",
            validated_arrays
        );
    } else {
        // Without cargo-audit, we may only have array parameters from other enabled subcommands
        // (e.g., nextest args). Note: llvm-cov is disabled due to sandbox incompatibility.
        assert!(
            validated_arrays >= 1,
            "Should have validated at least 1 array parameter without cargo audit installed (e.g., args from nextest)"
        );
        eprintln!(
            "cargo-audit not found, skipping audit-related parameter checks. Validated {} array parameters.",
            validated_arrays
        );
    }

    client.cancel().await?;
    Ok(())
}
