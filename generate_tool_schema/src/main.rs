#!/usr/bin/env rust-script
//! # Generate Tool Schema
//!
//! A CLI tool that generates the JSON Schema for the Multi-Tool Definition Format (MTDF).
//! This schema is used to validate tool configuration files in the AHMA ecosystem.
//!
//! ## Usage
//!
//! Run the binary directly with cargo:
//!
//! ```sh
//! cargo run -p generate_tool_schema -- [OUTPUT_DIR]
//! ```
//!
//! If `OUTPUT_DIR` is not provided, it defaults to `docs`.
//! The schema file is written to `[OUTPUT_DIR]/mtdf-schema.json`.

use ahma_mcp::config::ToolConfig;
use ahma_mcp::utils::logging::init_logging;
use std::env;
use std::fs;
use std::path::PathBuf;

/// Generates the JSON schema for [`ToolConfig`] as a pretty-printed JSON string.
///
/// This function uses `schemars` to introspect the `ToolConfig` struct and produce
/// a standard JSON Schema (draft-07).
///
/// # Returns
///
/// Returns a `Result` containing the formatted JSON string or a `serde_json::Error`
/// if serializing the schema fails.
///
/// # Examples
///
/// ```
/// // returns valid JSON string representing the schema
/// let schema = generate_tool_schema::generate_schema_json().unwrap();
/// assert!(schema.contains("\"$schema\":"));
/// ```
pub fn generate_schema_json() -> Result<String, serde_json::Error> {
    let schema = schemars::schema_for!(ToolConfig);
    serde_json::to_string_pretty(&schema)
}

/// Parses the output directory from command line arguments.
///
/// # Arguments
///
/// * `args` - An iterator over command line arguments.
///
/// # Returns
///
/// Returns the first argument as a `PathBuf` if present, otherwise defaults to "docs".
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
/// use generate_tool_schema::parse_output_dir;
///
/// // With arguments
/// let args = vec!["program_name", "custom/out"];
/// assert_eq!(parse_output_dir(args.into_iter()), PathBuf::from("custom/out"));
///
/// // Without arguments (uses default)
/// let args = vec!["program_name"];
/// assert_eq!(parse_output_dir(args.into_iter()), PathBuf::from("docs"));
/// ```
pub fn parse_output_dir<I, S>(mut args: I) -> PathBuf
where
    I: Iterator<Item = S>,
    S: Into<String>,
{
    args.nth(1)
        .map(|s| PathBuf::from(s.into()))
        .unwrap_or_else(|| PathBuf::from("docs"))
}

/// Writes the schema JSON content to a file in the specified directory.
///
/// This function will recursively create the directory structure if it does not exist.
/// The file will be named `mtdf-schema.json`.
///
/// # Arguments
///
/// * `output_dir` - The directory where the file should be written.
/// * `schema_json` - The JSON content to write.
///
/// # Returns
///
/// Returns the full path to the written file, or an `io::Error` if the operation fails.
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
/// use generate_tool_schema::write_schema_to_file;
///
/// let output_dir = PathBuf::from("/tmp/schema_output");
/// let schema_json = "{}";
/// // writes /tmp/schema_output/mtdf-schema.json
/// let path = write_schema_to_file(&output_dir, schema_json).unwrap();
/// ```
pub fn write_schema_to_file(
    output_dir: &PathBuf,
    schema_json: &str,
) -> Result<PathBuf, std::io::Error> {
    fs::create_dir_all(output_dir)?;
    let docs_path = output_dir.join("mtdf-schema.json");
    fs::write(&docs_path, schema_json)?;
    Ok(docs_path)
}

/// Generates a preview string of the schema JSON.
///
/// This is used for console output to show the user what was generated.
/// It indents the content with 4 spaces and truncates it if it exceeds `max_lines`.
///
/// # Arguments
///
/// * `schema_json` - The full JSON string.
/// * `max_lines` - The maximum number of lines to show before truncating.
///
/// # Returns
///
/// A formatted string ready for printing to stdout.
///
/// # Examples
///
/// ```
/// use generate_tool_schema::generate_preview;
///
/// let json = "{\n  \"key\": \"val\"\n}";
/// let preview = generate_preview(json, 5);
/// assert!(preview.contains("    {"));
/// ```
pub fn generate_preview(schema_json: &str, max_lines: usize) -> String {
    let total_lines = schema_json.lines().count();
    let lines: Vec<&str> = schema_json.lines().take(max_lines).collect();

    let mut preview = String::new();
    for line in &lines {
        preview.push_str(&format!("    {}\n", line));
    }

    if total_lines > max_lines {
        preview.push_str(&format!(
            "    ... and {} more lines\n",
            total_lines - max_lines
        ));
    }

    preview
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging("info", false)?;

    // Reject flag-like arguments (e.g. --help, --version) before touching the
    // filesystem. Without this, "--help" would be treated as an output directory
    // name, creating a literal "--help/" directory in the working tree.
    if env::args().skip(1).any(|a| a.starts_with('-')) {
        eprintln!("Usage: generate-tool-schema [OUTPUT_DIR]");
        eprintln!("  OUTPUT_DIR  Directory to write mtdf-schema.json (default: docs)");
        return Ok(());
    }

    // Generate schema JSON
    let schema_json = generate_schema_json()?;

    // Parse output directory from arguments
    let output_dir = parse_output_dir(env::args());

    // Write schema to file
    let docs_path = write_schema_to_file(&output_dir, &schema_json)?;

    println!("OK Generated MTDF JSON Schema at: {}", docs_path.display());
    println!("  Schema size: {} bytes", schema_json.len());

    // Show preview
    println!("  Preview:");
    print!("{}", generate_preview(&schema_json, 10));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_generate_schema_json_produces_valid_json() {
        let result = generate_schema_json();
        assert!(result.is_ok(), "Schema generation should succeed");

        let json_str = result.unwrap();
        assert!(!json_str.is_empty(), "Schema JSON should not be empty");

        // Verify it's valid JSON by parsing it
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json_str);
        assert!(parsed.is_ok(), "Schema should be valid JSON");
    }

    #[test]
    fn test_generate_schema_json_contains_expected_structure() {
        let json_str = generate_schema_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // JSON Schema should have a $schema key or other schema-related keys
        assert!(
            parsed.get("$schema").is_some() || parsed.get("$defs").is_some(),
            "Schema should have JSON Schema structure"
        );
    }

    #[test]
    fn test_parse_output_dir_with_custom_path() {
        let args = vec!["program", "/custom/path"];
        let result = parse_output_dir(args.into_iter());
        assert_eq!(result, PathBuf::from("/custom/path"));
    }

    #[test]
    fn test_parse_output_dir_with_default() {
        let args = vec!["program"];
        let result = parse_output_dir(args.into_iter());
        assert_eq!(result, PathBuf::from("docs"));
    }

    #[test]
    fn test_parse_output_dir_with_empty_args() {
        let args: Vec<String> = vec![];
        let result = parse_output_dir(args.into_iter());
        assert_eq!(result, PathBuf::from("docs"));
    }

    #[test]
    fn test_parse_output_dir_ignores_extra_args() {
        let args = vec!["program", "/first/path", "/second/path", "/third/path"];
        let result = parse_output_dir(args.into_iter());
        assert_eq!(result, PathBuf::from("/first/path"));
    }

    #[test]
    fn test_write_schema_to_file_creates_directory_and_file() {
        let temp_dir = tempdir().unwrap();
        let output_dir = temp_dir.path().join("nested/output");

        let schema_json = r#"{"test": "schema"}"#;
        let result = write_schema_to_file(&output_dir, schema_json);

        assert!(result.is_ok(), "Writing schema should succeed");
        let docs_path = result.unwrap();
        assert_eq!(docs_path, output_dir.join("mtdf-schema.json"));
        assert!(docs_path.exists(), "Schema file should exist");

        let content = fs::read_to_string(&docs_path).unwrap();
        assert_eq!(content, schema_json);
    }

    #[test]
    fn test_write_schema_to_file_overwrites_existing() {
        let temp_dir = tempdir().unwrap();
        let output_dir = temp_dir.path().to_path_buf();

        // Write first version
        let _ = write_schema_to_file(&output_dir, r#"{"version": 1}"#).unwrap();

        // Write second version
        let docs_path = write_schema_to_file(&output_dir, r#"{"version": 2}"#).unwrap();

        let content = fs::read_to_string(&docs_path).unwrap();
        assert_eq!(content, r#"{"version": 2}"#);
    }

    #[test]
    fn test_generate_preview_with_short_content() {
        let schema_json = "line1\nline2\nline3";
        let preview = generate_preview(schema_json, 10);

        assert!(preview.contains("line1"));
        assert!(preview.contains("line2"));
        assert!(preview.contains("line3"));
        assert!(
            !preview.contains("more lines"),
            "Should not show 'more lines' for short content"
        );
    }

    #[test]
    fn test_generate_preview_with_long_content() {
        let schema_json = (1..=20)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let preview = generate_preview(&schema_json, 10);

        assert!(preview.contains("line1"));
        assert!(preview.contains("line10"));
        assert!(!preview.contains("    line11"), "Should not show line 11");
        assert!(preview.contains("... and 10 more lines"));
    }

    #[test]
    fn test_generate_preview_with_empty_content() {
        let schema_json = "";
        let preview = generate_preview(schema_json, 10);
        assert_eq!(preview, "");
    }

    #[test]
    fn test_generate_preview_preserves_indentation() {
        let schema_json = "{\n  \"key\": \"value\"\n}";
        let preview = generate_preview(schema_json, 10);

        assert!(preview.contains("    {"), "Should add indentation prefix");
        assert!(
            preview.contains("      \"key\": \"value\""),
            "Should preserve original indentation"
        );
    }

    #[test]
    fn test_generate_preview_exact_boundary() {
        let schema_json = (1..=10)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let preview = generate_preview(&schema_json, 10);

        assert!(preview.contains("line10"));
        assert!(
            !preview.contains("more lines"),
            "Should not show 'more lines' at exact boundary"
        );
    }

    #[test]
    fn test_full_workflow_integration() {
        let temp_dir = tempdir().unwrap();
        let output_dir = temp_dir.path().to_path_buf();

        // Generate schema
        let schema_json = generate_schema_json().unwrap();

        // Write to file
        let docs_path = write_schema_to_file(&output_dir, &schema_json).unwrap();

        // Read back and verify
        let content = fs::read_to_string(&docs_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Should be valid JSON Schema
        assert!(parsed.is_object());

        // Generate preview
        let preview = generate_preview(&schema_json, 10);
        assert!(!preview.is_empty());
    }
}
