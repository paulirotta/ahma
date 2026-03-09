use ahma_mcp::schema_validation::MtdfValidator;
use anyhow::{Result, ensure};
use clap::Parser;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tracing::{error, info, instrument};

/// Ahma Tool Configuration Validator
///
/// This CLI tool validates tool configuration files against the MTDF schema.
/// It verifies that the JSON structure matches expected schemas and checks for
/// internal consistency.
///
/// # Examples
///
/// Validate the default `.ahma` directory:
/// ```bash
/// ahma_validate
/// ```
///
/// Validate a specific file:
/// ```bash
/// ahma_validate --validation-target my_tool.json
/// ```
///
/// Validate multiple targets (files and directories):
/// ```bash
/// ahma_validate --validation-target "tool1.json,tool2.json,./config"
/// ```
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about,
    long_about = "Validates tool configurations against the MTDF schema and checks for other inconsistencies."
)]
struct Cli {
    /// Path to the directory containing tool JSON configuration files, a comma-separated list of files, or blank to validate '.ahma'.
    #[arg(default_value = ".ahma")]
    validation_target: String,

    /// Enable debug logging.
    #[arg(short, long, global = true)]
    debug: bool,
}

/// Entry point for the application.
///
/// Parses command line arguments, initializes logging, and executes the validation logic.
/// Returns an error if validation fails for any target or if an unexpected error occurs.
#[instrument]
fn main() -> Result<()> {
    let cli = Cli::parse();
    ahma_mcp::utils::logging::init_logging(if cli.debug { "debug" } else { "info" }, false)?;

    ensure!(
        run_validation_mode(&cli)?,
        "Some configurations are invalid. Please check the error messages above."
    );

    println!("All configurations are valid.");
    info!("All configurations are valid.");
    Ok(())
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

/// Runs the tool validation process based on the CLI arguments.
///
/// Returns `Ok(true)` if all configurations are valid, `Ok(false)` if any is invalid.
fn run_validation_mode(cli: &Cli) -> Result<bool> {
    let validator = MtdfValidator::new();
    let targets: Vec<String> = cli
        .validation_target
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    let (files, all_found) = collect_validation_files(targets)?;

    let all_validated = files
        .iter()
        .fold(true, |valid, f| validate_file(&validator, f) && valid);

    Ok(all_found && all_validated)
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

    // ==================== CLI parsing tests ====================

    #[test]
    fn test_cli_default_values() {
        let cli = Cli::parse_from(["ahma_validate"]);

        assert_eq!(cli.validation_target, ".ahma");
        assert!(!cli.debug);
    }

    #[test]
    fn test_cli_custom_validation_target() {
        let cli = Cli::parse_from(["ahma_validate", "custom/path"]);

        assert_eq!(cli.validation_target, "custom/path");
    }

    #[test]
    fn test_cli_debug_flag() {
        let cli = Cli::parse_from(["ahma_validate", "--debug"]);

        assert!(cli.debug);
    }

    #[test]
    fn test_cli_short_debug_flag() {
        let cli = Cli::parse_from(["ahma_validate", "-d"]);

        assert!(cli.debug);
    }

    #[test]
    fn test_cli_comma_separated_targets() {
        let cli = Cli::parse_from(["ahma_validate", "file1.json,file2.json,dir/"]);

        assert_eq!(cli.validation_target, "file1.json,file2.json,dir/");
    }

    // ==================== run_validation_mode tests ====================

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
    fn test_run_validation_mode_valid_single_file() {
        let temp_dir = setup_temp_dir_with_files(&[("tool.json", valid_tool_config())]);

        let cli = Cli {
            validation_target: temp_dir
                .path()
                .join("tool.json")
                .to_string_lossy()
                .to_string(),
            debug: false,
        };

        let result = run_validation_mode(&cli);

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_run_validation_mode_valid_directory() {
        let temp_dir = setup_temp_dir_with_files(&[
            ("tools/tool1.json", valid_tool_config()),
            ("tools/tool2.json", valid_tool_config()),
        ]);

        let cli = Cli {
            validation_target: temp_dir.path().join("tools").to_string_lossy().to_string(),
            debug: false,
        };

        let result = run_validation_mode(&cli);

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_run_validation_mode_comma_separated_files() {
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

        let cli = Cli {
            validation_target: format!("{},{}", file1, file2),
            debug: false,
        };

        let result = run_validation_mode(&cli);

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_run_validation_mode_nonexistent_target() {
        let cli = Cli {
            validation_target: "/nonexistent/path/12345".to_string(),
            debug: false,
        };

        let result = run_validation_mode(&cli);

        assert!(result.is_ok());
        assert!(!result.unwrap()); // Should return false for invalid
    }

    #[test]
    fn test_run_validation_mode_invalid_json_content() {
        let temp_dir = setup_temp_dir_with_files(&[("tool.json", "{ invalid json }")]);

        let cli = Cli {
            validation_target: temp_dir
                .path()
                .join("tool.json")
                .to_string_lossy()
                .to_string(),
            debug: false,
        };

        let result = run_validation_mode(&cli);

        assert!(result.is_ok());
        assert!(!result.unwrap()); // Should return false for invalid JSON
    }

    #[test]
    fn test_run_validation_mode_empty_directory() {
        let temp_dir = setup_temp_dir_with_files(&[]);

        // Create an empty tools directory
        fs::create_dir(temp_dir.path().join("tools")).expect("Failed to create tools dir");

        let cli = Cli {
            validation_target: temp_dir.path().join("tools").to_string_lossy().to_string(),
            debug: false,
        };

        let result = run_validation_mode(&cli);

        assert!(result.is_ok());
        assert!(result.unwrap()); // Empty directory is valid (no files to fail)
    }

    #[test]
    fn test_run_validation_mode_mixed_valid_invalid() {
        let temp_dir = setup_temp_dir_with_files(&[
            ("tools/valid.json", valid_tool_config()),
            ("tools/invalid.json", "{ not json }"),
        ]);

        let cli = Cli {
            validation_target: temp_dir.path().join("tools").to_string_lossy().to_string(),
            debug: false,
        };

        let result = run_validation_mode(&cli);

        assert!(result.is_ok());
        assert!(!result.unwrap()); // One invalid file should make whole result false
    }

    #[test]
    fn test_run_validation_mode_missing_required_fields() {
        // Tool config missing required 'name' field
        let invalid_tool = r#"{
            "description": "Missing name field",
            "inputSchema": {
                "type": "object"
            }
        }"#;

        let temp_dir = setup_temp_dir_with_files(&[("tool.json", invalid_tool)]);

        let cli = Cli {
            validation_target: temp_dir
                .path()
                .join("tool.json")
                .to_string_lossy()
                .to_string(),
            debug: false,
        };

        let result = run_validation_mode(&cli);

        assert!(result.is_ok());
        assert!(!result.unwrap()); // Missing required field should fail validation
    }
}
