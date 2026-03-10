//! Tool configuration validation module.
//!
//! Validates MTDF tool configuration files against the JSON schema.
//! Used by the `--validate` CLI flag to check tool configs before startup.

use crate::schema_validation::MtdfValidator;
use anyhow::Result;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tracing::{error, info};

/// Result of validating one or more tool configuration files.
pub struct ValidationResult {
    /// Total number of files checked.
    pub files_checked: usize,
    /// Number of files that passed validation.
    pub files_passed: usize,
    /// Number of files that failed validation.
    pub files_failed: usize,
    /// Whether all files passed validation.
    pub all_valid: bool,
}

/// Validates tool configuration files at the given target path.
///
/// The target can be:
/// - A directory (scans for `.json` files)
/// - A single file
/// - A comma-separated list of files and/or directories
///
/// Returns a [`ValidationResult`] summarizing the outcome.
pub fn run_validation(validation_target: &str) -> Result<ValidationResult> {
    let validator = MtdfValidator::new();
    let targets: Vec<String> = validation_target
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    let (files, all_found) = collect_validation_files(targets)?;

    let mut passed = 0usize;
    let mut failed = 0usize;

    for f in &files {
        if validate_file(&validator, f) {
            passed += 1;
        } else {
            failed += 1;
        }
    }

    if !all_found {
        failed += 1; // count missing targets as a failure
    }

    let files_checked = files.len();
    Ok(ValidationResult {
        files_checked,
        files_passed: passed,
        files_failed: failed,
        all_valid: all_found && failed == 0,
    })
}

/// Returns true if `path` matches the legacy `.ahma/tools` directory pattern.
fn is_legacy_ahma_tools_path(path: &Path) -> bool {
    path.file_name().and_then(|s| s.to_str()) == Some("tools")
        && path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            == Some(".ahma")
}

/// Normalizes the validation target path for legacy compatibility.
///
/// If the path matches `.ahma/tools` but doesn't exist, falls back to the
/// parent `.ahma` directory when it exists.
fn normalize_validation_target(path: PathBuf) -> PathBuf {
    if !is_legacy_ahma_tools_path(&path) || path.exists() {
        return path;
    }
    match path.parent() {
        Some(parent) if parent.exists() => parent.to_path_buf(),
        _ => path,
    }
}

/// Resolves target strings into concrete file paths to validate.
///
/// Returns the collected files and whether all targets were found.
fn collect_validation_files(targets: Vec<String>) -> Result<(Vec<PathBuf>, bool)> {
    let mut files = Vec::new();
    let mut all_found = true;

    for target in targets {
        let path = normalize_validation_target(PathBuf::from(target));
        if path.is_dir() {
            files.extend(get_json_files(&path)?);
        } else if path.is_file() {
            files.push(path);
        } else {
            error!("Validation target not found: {}", path.display());
            all_found = false;
        }
    }

    Ok((files, all_found))
}

/// Reads and validates a single tool configuration file.
fn validate_file(validator: &MtdfValidator, file_path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(file_path).inspect_err(|e| {
        error!("Failed to read file {}: {}", file_path.display(), e);
    }) else {
        return false;
    };

    validator
        .validate_tool_config(file_path, &content)
        .inspect(|_| info!("{} is valid.", file_path.display()))
        .inspect_err(|e| error!("Validation failed for {}: {:?}", file_path.display(), e))
        .is_ok()
}

/// Scans a directory for top-level `.json` files (non-recursive).
fn get_json_files(dir: &Path) -> Result<Vec<PathBuf>> {
    Ok(fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_temp_dir_with_files(files: &[(&str, &str)]) -> TempDir {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        for (name, content) in files {
            let file_path = temp_dir.path().join(name);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).expect("Failed to create parent dirs");
            }
            fs::write(&file_path, content).expect("Failed to write file");
        }
        temp_dir
    }

    // ==================== get_json_files tests ====================

    #[test]
    fn test_get_json_files_returns_only_json_files() {
        let temp_dir = setup_temp_dir_with_files(&[
            ("tool1.json", "{}"),
            ("tool2.json", "{}"),
            ("readme.txt", "text"),
            ("config.yaml", "yaml: true"),
        ]);

        let files = get_json_files(temp_dir.path()).expect("Should succeed");

        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|p| p.extension().unwrap() == "json"));
    }

    #[test]
    fn test_get_json_files_empty_directory() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        let files = get_json_files(temp_dir.path()).expect("Should succeed");

        assert!(files.is_empty());
    }

    #[test]
    fn test_get_json_files_no_json_files() {
        let temp_dir =
            setup_temp_dir_with_files(&[("readme.md", "# Readme"), ("config.toml", "[config]")]);

        let files = get_json_files(temp_dir.path()).expect("Should succeed");

        assert!(files.is_empty());
    }

    #[test]
    fn test_get_json_files_nonexistent_directory() {
        let result = get_json_files(Path::new("/nonexistent/path/12345"));

        assert!(result.is_err());
    }

    #[test]
    fn test_get_json_files_ignores_subdirectories() {
        let temp_dir =
            setup_temp_dir_with_files(&[("tool.json", "{}"), ("subdir/nested.json", "{}")]);

        let files = get_json_files(temp_dir.path()).expect("Should succeed");

        // Should only find top-level json files, not nested ones
        assert_eq!(files.len(), 1);
        assert!(files[0].file_name().unwrap() == "tool.json");
    }

    // ==================== run_validation tests ====================

    /// Creates a minimal valid MTDF tool configuration
    /// Required fields: name, description, command
    fn valid_tool_config() -> &'static str {
        r#"{
            "name": "test_tool",
            "description": "A test tool for validation",
            "command": "echo"
        }"#
    }

    #[test]
    fn test_run_validation_valid_single_file() {
        let temp_dir = setup_temp_dir_with_files(&[("tool.json", valid_tool_config())]);

        let target = temp_dir
            .path()
            .join("tool.json")
            .to_string_lossy()
            .to_string();

        let result = run_validation(&target).expect("Should succeed");

        assert!(result.all_valid);
        assert_eq!(result.files_checked, 1);
        assert_eq!(result.files_passed, 1);
        assert_eq!(result.files_failed, 0);
    }

    #[test]
    fn test_run_validation_valid_directory() {
        let temp_dir = setup_temp_dir_with_files(&[
            ("tools/tool1.json", valid_tool_config()),
            ("tools/tool2.json", valid_tool_config()),
        ]);

        let target = temp_dir.path().join("tools").to_string_lossy().to_string();

        let result = run_validation(&target).expect("Should succeed");

        assert!(result.all_valid);
        assert_eq!(result.files_checked, 2);
        assert_eq!(result.files_passed, 2);
    }

    #[test]
    fn test_run_validation_comma_separated_files() {
        let temp_dir = setup_temp_dir_with_files(&[
            ("tool1.json", valid_tool_config()),
            ("tool2.json", valid_tool_config()),
        ]);

        let file1 = temp_dir
            .path()
            .join("tool1.json")
            .to_string_lossy()
            .to_string();
        let file2 = temp_dir
            .path()
            .join("tool2.json")
            .to_string_lossy()
            .to_string();

        let target = format!("{},{}", file1, file2);

        let result = run_validation(&target).expect("Should succeed");

        assert!(result.all_valid);
        assert_eq!(result.files_checked, 2);
    }

    #[test]
    fn test_run_validation_nonexistent_target() {
        let result = run_validation("/nonexistent/path/12345").expect("Should succeed");

        assert!(!result.all_valid);
    }

    #[test]
    fn test_run_validation_invalid_json_content() {
        let temp_dir = setup_temp_dir_with_files(&[("tool.json", "{ invalid json }")]);

        let target = temp_dir
            .path()
            .join("tool.json")
            .to_string_lossy()
            .to_string();

        let result = run_validation(&target).expect("Should succeed");

        assert!(!result.all_valid);
        assert_eq!(result.files_failed, 1);
    }

    #[test]
    fn test_run_validation_empty_directory() {
        let temp_dir = setup_temp_dir_with_files(&[]);

        // Create an empty tools directory
        fs::create_dir(temp_dir.path().join("tools")).expect("Failed to create tools dir");

        let target = temp_dir.path().join("tools").to_string_lossy().to_string();

        let result = run_validation(&target).expect("Should succeed");

        assert!(result.all_valid); // Empty directory is valid (no files to fail)
        assert_eq!(result.files_checked, 0);
    }

    #[test]
    fn test_run_validation_mixed_valid_invalid() {
        let temp_dir = setup_temp_dir_with_files(&[
            ("tools/valid.json", valid_tool_config()),
            ("tools/invalid.json", "{ not json }"),
        ]);

        let target = temp_dir.path().join("tools").to_string_lossy().to_string();

        let result = run_validation(&target).expect("Should succeed");

        assert!(!result.all_valid);
        assert_eq!(result.files_checked, 2);
        assert_eq!(result.files_passed, 1);
        assert_eq!(result.files_failed, 1);
    }

    #[test]
    fn test_run_validation_missing_required_fields() {
        // Tool config missing required 'name' field
        let invalid_tool = r#"{
            "description": "Missing name field",
            "inputSchema": {
                "type": "object"
            }
        }"#;

        let temp_dir = setup_temp_dir_with_files(&[("tool.json", invalid_tool)]);

        let target = temp_dir
            .path()
            .join("tool.json")
            .to_string_lossy()
            .to_string();

        let result = run_validation(&target).expect("Should succeed");

        assert!(!result.all_valid);
        assert_eq!(result.files_failed, 1);
    }
}
