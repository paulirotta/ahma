use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn workspace_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to locate workspace root")
        .to_path_buf()
}

/// Ensure the generate_tool_schema binary produces an MTDF schema file in the target output directory.
#[test]
fn test_generate_schema_binary_outputs_schema() -> Result<()> {
    let workspace_dir = workspace_dir();
    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path();

    let output_dir_arg = output_dir.to_string_lossy().to_string();

    let command_output = Command::new("cargo")
        .current_dir(&workspace_dir)
        .arg("run")
        .arg("--package")
        .arg("generate_tool_schema")
        .arg("--bin")
        .arg("generate-tool-schema")
        .arg("--")
        .arg(&output_dir_arg)
        .output()?;

    assert!(
        command_output.status.success(),
        "generate_tool_schema should exit successfully: stderr={}",
        String::from_utf8_lossy(&command_output.stderr)
    );

    let schema_path = output_dir.join("mtdf-schema.json");
    assert!(schema_path.exists(), "Schema file should be generated");

    let schema_contents = fs::read_to_string(&schema_path)?;
    assert!(
        schema_contents.contains("\"$schema\"") && schema_contents.contains("ToolConfig"),
        "Schema output should include basic metadata"
    );

    Ok(())
}
