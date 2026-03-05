use super::*;
use crate::config::{CommandOption, SubcommandConfig};
use serde_json::json;
use std::path::Path;

fn test_temp_manager() -> TempFileManager {
    TempFileManager::new()
}

/// Helper to create a CommandOption with minimal boilerplate.
fn make_option(name: &str, option_type: &str) -> CommandOption {
    CommandOption {
        name: name.to_string(),
        option_type: option_type.to_string(),
        description: None,
        required: None,
        format: None,
        items: None,
        file_arg: None,
        file_flag: None,
        alias: None,
    }
}

/// Helper to create a CommandOption with path format.
fn make_path_option(name: &str, option_type: &str) -> CommandOption {
    CommandOption {
        name: name.to_string(),
        option_type: option_type.to_string(),
        format: Some("path".to_string()),
        description: None,
        required: None,
        items: None,
        file_arg: None,
        file_flag: None,
        alias: None,
    }
}

/// Helper to create a SubcommandConfig for the find command.
fn make_find_subcommand() -> SubcommandConfig {
    SubcommandConfig {
        name: "find".to_string(),
        description: "Search for files".to_string(),
        enabled: true,
        positional_args_first: Some(true),
        positional_args: Some(vec![make_path_option("path", "string")]),
        options: Some(vec![
            make_option("-name", "string"),
            make_option("-maxdepth", "integer"),
        ]),
        subcommand: None,
        timeout_seconds: None,
        synchronous: None,
        guidance_key: None,
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    }
}

fn make_bool_option(name: &str, alias: &str) -> CommandOption {
    CommandOption {
        name: name.to_string(),
        option_type: "boolean".to_string(),
        description: None,
        required: None,
        format: None,
        items: None,
        file_arg: None,
        file_flag: None,
        alias: Some(alias.to_string()),
    }
}

fn make_file_option(name: &str, flag: Option<&str>) -> CommandOption {
    CommandOption {
        name: name.to_string(),
        option_type: "string".to_string(),
        description: None,
        required: None,
        format: None,
        items: None,
        file_arg: Some(true),
        file_flag: flag.map(str::to_string),
        alias: None,
    }
}

#[tokio::test]
async fn shell_commands_append_redirect_once() {
    let temp_manager = test_temp_manager();
    let mut args_map = Map::new();
    args_map.insert("args".to_string(), json!(["echo hi"]));

    let (program, args_vec) = prepare_command_and_args(
        "/bin/sh -c",
        Some(&args_map),
        None,
        Path::new("."),
        &temp_manager,
    )
    .await
    .expect("command");

    assert_eq!(program, "/bin/sh");
    assert_eq!(args_vec, vec!["-c".to_string(), "echo hi 2>&1".to_string()]);
}

#[tokio::test]
async fn shell_commands_do_not_duplicate_redirect() {
    let temp_manager = test_temp_manager();
    let mut args_map = Map::new();
    args_map.insert("args".to_string(), json!(["ls 2>&1"]));

    let (_, args_vec) = prepare_command_and_args(
        "/bin/sh -c",
        Some(&args_map),
        None,
        Path::new("."),
        &temp_manager,
    )
    .await
    .expect("command");

    assert_eq!(args_vec, vec!["-c".to_string(), "ls 2>&1".to_string()]);
}

#[tokio::test]
async fn non_shell_commands_remain_unchanged() {
    let temp_manager = test_temp_manager();
    let mut args_map = Map::new();
    args_map.insert("args".to_string(), json!(["--version"]));

    let (program, args_vec) =
        prepare_command_and_args("git", Some(&args_map), None, Path::new("."), &temp_manager)
            .await
            .expect("command");

    assert_eq!(program, "git");
    assert_eq!(args_vec, vec!["--version".to_string()]);
}

#[test]
fn test_format_option_flag_standard_option() {
    // Standard options get -- prefix
    assert_eq!(format_option_flag("verbose"), "--verbose");
    assert_eq!(format_option_flag("force"), "--force");
    assert_eq!(
        format_option_flag("working_directory"),
        "--working_directory"
    );
}

#[test]
fn test_format_option_flag_dash_prefixed_option() {
    // Options already starting with - are used as-is
    assert_eq!(format_option_flag("-name"), "-name");
    assert_eq!(format_option_flag("-type"), "-type");
    assert_eq!(format_option_flag("-mtime"), "-mtime");
    // Double-dash options are also preserved
    assert_eq!(format_option_flag("--version"), "--version");
}

#[test]
fn test_format_option_flag_empty_string() {
    // Empty string should get -- prefix (edge case)
    assert_eq!(format_option_flag(""), "--");
}

#[tokio::test]
async fn find_command_args_with_dash_prefix() {
    let temp_manager = test_temp_manager();
    let subcommand_config = make_find_subcommand();

    let mut args_map = Map::new();
    args_map.insert("path".to_string(), json!("."));
    args_map.insert("-name".to_string(), json!("*.toml"));
    args_map.insert("-maxdepth".to_string(), json!(1));

    let (program, args_vec) = prepare_command_and_args(
        "find",
        Some(&args_map),
        Some(&subcommand_config),
        Path::new("."),
        &temp_manager,
    )
    .await
    .expect("command");

    assert_eq!(program, "find");
    // With positional_args_first: true, path should come BEFORE options
    // This is required by both BSD and GNU find
    assert!(
        args_vec.contains(&"-name".to_string()),
        "Should contain -name, got: {:?}",
        args_vec
    );
    assert!(
        args_vec.contains(&"-maxdepth".to_string()),
        "Should contain -maxdepth, got: {:?}",
        args_vec
    );
    assert!(
        args_vec.contains(&"*.toml".to_string()),
        "Should contain pattern value, got: {:?}",
        args_vec
    );
    assert!(
        args_vec.contains(&"1".to_string()),
        "Should contain depth value, got: {:?}",
        args_vec
    );
    // With positional_args_first: true, the path should be the first argument
    // (path is expanded to absolute path due to format: "path")
    let first_arg = args_vec.first().expect("Should have at least one argument");
    assert!(
        std::path::Path::new(first_arg).is_absolute() || first_arg == ".",
        "First argument should be a path, got: {:?}",
        args_vec
    );
    // Verify path comes before options
    let name_idx = args_vec.iter().position(|s| s == "-name").unwrap();
    let maxdepth_idx = args_vec.iter().position(|s| s == "-maxdepth").unwrap();
    assert!(
        0 < name_idx && 0 < maxdepth_idx,
        "Path (index 0) should come before options (-name at {}, -maxdepth at {}): {:?}",
        name_idx,
        maxdepth_idx,
        args_vec
    );
    // Should NOT contain --maxdepth or ---name
    assert!(
        !args_vec.iter().any(|s| s == "--maxdepth"),
        "Should NOT contain --maxdepth, got: {:?}",
        args_vec
    );
    assert!(
        !args_vec.iter().any(|s| s == "---name"),
        "Should NOT contain ---name, got: {:?}",
        args_vec
    );
}

#[tokio::test]
async fn boolean_option_uses_alias_when_true() {
    let temp_manager = test_temp_manager();
    let subcommand_config = SubcommandConfig {
        name: "demo".to_string(),
        description: "demo".to_string(),
        enabled: true,
        positional_args_first: None,
        positional_args: None,
        options: Some(vec![make_bool_option("verbose", "v")]),
        subcommand: None,
        timeout_seconds: None,
        synchronous: None,
        guidance_key: None,
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    };

    let mut args_map = Map::new();
    args_map.insert("verbose".to_string(), json!("true"));

    let (_, args_vec) = prepare_command_and_args(
        "mycmd",
        Some(&args_map),
        Some(&subcommand_config),
        Path::new("."),
        &temp_manager,
    )
    .await
    .expect("command");

    assert_eq!(args_vec, vec!["-v".to_string()]);
}

#[tokio::test]
async fn reserved_runtime_keys_are_not_emitted_as_cli_args() {
    let temp_manager = test_temp_manager();
    let mut args_map = Map::new();
    args_map.insert("working_directory".to_string(), json!("/tmp"));
    args_map.insert("execution_mode".to_string(), json!("Synchronous"));
    args_map.insert("timeout_seconds".to_string(), json!(5));
    args_map.insert("args".to_string(), json!(["positional"]));
    args_map.insert("name".to_string(), json!("value"));

    let (_, args_vec) = prepare_command_and_args(
        "mycmd",
        Some(&args_map),
        None,
        Path::new("."),
        &temp_manager,
    )
    .await
    .expect("command");

    assert_eq!(
        args_vec,
        vec![
            "--name".to_string(),
            "value".to_string(),
            "positional".to_string()
        ]
    );
}

#[tokio::test]
async fn file_arg_uses_configured_flag_and_writes_content() {
    let temp_manager = test_temp_manager();
    let subcommand_config = SubcommandConfig {
        name: "demo".to_string(),
        description: "demo".to_string(),
        enabled: true,
        positional_args_first: None,
        positional_args: None,
        options: Some(vec![make_file_option("input", Some("-f"))]),
        subcommand: None,
        timeout_seconds: None,
        synchronous: None,
        guidance_key: None,
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    };

    let mut args_map = Map::new();
    args_map.insert("input".to_string(), json!("line 1\nline 2"));

    let (_, args_vec) = prepare_command_and_args(
        "mycmd",
        Some(&args_map),
        Some(&subcommand_config),
        Path::new("."),
        &temp_manager,
    )
    .await
    .expect("command");

    assert_eq!(args_vec.len(), 2);
    assert_eq!(args_vec[0], "-f");

    let path = std::path::PathBuf::from(&args_vec[1]);
    assert!(
        path.exists(),
        "Expected temp file to exist: {}",
        args_vec[1]
    );
    let contents =
        std::fs::read_to_string(&path).expect("failed to read generated temp file content");
    assert_eq!(contents, "line 1\nline 2");
}

/// Creates a SubcommandConfig mimicking the grep subcommand from file-tools.json.
fn make_grep_subcommand() -> SubcommandConfig {
    SubcommandConfig {
        name: "grep".to_string(),
        description: "Search text patterns in files".to_string(),
        enabled: true,
        positional_args_first: None,
        positional_args: Some(vec![
            make_option("pattern", "string"),
            CommandOption {
                name: "files".to_string(),
                option_type: "array".to_string(),
                format: Some("path".to_string()),
                description: None,
                required: None,
                items: None,
                file_arg: None,
                file_flag: None,
                alias: None,
            },
        ]),
        options: Some(vec![
            make_path_option("working_directory", "string"),
            make_bool_option("recursive", "r"),
            make_bool_option("ignore-case", "i"),
            make_bool_option("line-number", "n"),
            make_bool_option("count", "c"),
            make_bool_option("files-with-matches", "l"),
            make_bool_option("invert-match", "v"),
            make_bool_option("word-regexp", "w"),
            make_bool_option("extended-regexp", "E"),
        ]),
        subcommand: None,
        timeout_seconds: None,
        synchronous: None,
        guidance_key: None,
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    }
}

/// Creates a SubcommandConfig mimicking the cat subcommand with BSD-compatible aliases.
fn make_cat_subcommand() -> SubcommandConfig {
    SubcommandConfig {
        name: "cat".to_string(),
        description: "Display file contents".to_string(),
        enabled: true,
        positional_args_first: None,
        positional_args: Some(vec![CommandOption {
            name: "files".to_string(),
            option_type: "array".to_string(),
            format: Some("path".to_string()),
            description: None,
            required: None,
            items: None,
            file_arg: None,
            file_flag: None,
            alias: None,
        }]),
        options: Some(vec![
            make_path_option("working_directory", "string"),
            make_bool_option("number", "n"),
            make_bool_option("show-ends", "e"),
            make_bool_option("show-tabs", "t"),
        ]),
        subcommand: None,
        timeout_seconds: None,
        synchronous: None,
        guidance_key: None,
        sequence: None,
        step_delay_ms: None,
        availability_check: None,
        install_instructions: None,
    }
}

#[tokio::test]
async fn unknown_args_are_skipped_when_schema_present() {
    // This test catches the --source bug: when a schema is present, unknown keys
    // like "source" should NOT be emitted as --source CLI flags.
    let temp_manager = test_temp_manager();
    let subcommand_config = make_grep_subcommand();

    let mut args_map = Map::new();
    args_map.insert("pattern".to_string(), json!("chrono"));
    args_map.insert("source".to_string(), json!("Cargo.toml")); // NOT a valid grep option
    args_map.insert("subcommand".to_string(), json!("grep")); // Also not a grep option

    let (_, args_vec) = prepare_command_and_args(
        "grep",
        Some(&args_map),
        Some(&subcommand_config),
        Path::new("."),
        &temp_manager,
    )
    .await
    .expect("command");

    assert!(
        !args_vec.iter().any(|s| s == "--source"),
        "Unknown arg 'source' should NOT be emitted as --source. Got: {:?}",
        args_vec
    );
    assert!(
        !args_vec.iter().any(|s| s == "--subcommand"),
        "Unknown arg 'subcommand' should NOT be emitted as --subcommand. Got: {:?}",
        args_vec
    );
    // The pattern positional arg should still be present
    assert!(
        args_vec.contains(&"chrono".to_string()),
        "Pattern positional arg should be present. Got: {:?}",
        args_vec
    );
}

#[tokio::test]
async fn unknown_args_passthrough_without_schema() {
    // When no SubcommandConfig is provided (schema-less tool), unknown keys
    // should still be emitted as --{key} flags for backwards compatibility.
    let temp_manager = test_temp_manager();

    let mut args_map = Map::new();
    args_map.insert("custom-flag".to_string(), json!("value"));
    args_map.insert("another".to_string(), json!("thing"));

    let (_, args_vec) = prepare_command_and_args(
        "mycmd",
        Some(&args_map),
        None, // No schema
        Path::new("."),
        &temp_manager,
    )
    .await
    .expect("command");

    assert!(
        args_vec.contains(&"--custom-flag".to_string()),
        "Without schema, unknown args should pass through. Got: {:?}",
        args_vec
    );
    assert!(
        args_vec.contains(&"--another".to_string()),
        "Without schema, unknown args should pass through. Got: {:?}",
        args_vec
    );
}

#[tokio::test]
async fn grep_options_emit_correct_alias_flags() {
    // Verify that boolean options with aliases emit the short alias form (-i, -n, etc.)
    let temp_manager = test_temp_manager();
    let subcommand_config = make_grep_subcommand();

    let mut args_map = Map::new();
    args_map.insert("pattern".to_string(), json!("test"));
    args_map.insert("ignore-case".to_string(), json!(true));
    args_map.insert("line-number".to_string(), json!(true));
    args_map.insert("recursive".to_string(), json!(true));

    let (_, args_vec) = prepare_command_and_args(
        "grep",
        Some(&args_map),
        Some(&subcommand_config),
        Path::new("."),
        &temp_manager,
    )
    .await
    .expect("command");

    assert!(
        args_vec.contains(&"-i".to_string()),
        "ignore-case should emit -i. Got: {:?}",
        args_vec
    );
    assert!(
        args_vec.contains(&"-n".to_string()),
        "line-number should emit -n. Got: {:?}",
        args_vec
    );
    assert!(
        args_vec.contains(&"-r".to_string()),
        "recursive should emit -r. Got: {:?}",
        args_vec
    );
    // Should NOT contain long forms
    assert!(
        !args_vec.iter().any(|s| s == "--ignore-case"),
        "Should use alias -i not --ignore-case. Got: {:?}",
        args_vec
    );
}

#[tokio::test]
async fn cat_bsd_aliases_use_lowercase() {
    // Verify that cat's show-ends and show-tabs use BSD-compatible lowercase aliases
    let temp_manager = test_temp_manager();
    let subcommand_config = make_cat_subcommand();

    let mut args_map = Map::new();
    args_map.insert("show-ends".to_string(), json!(true));
    args_map.insert("show-tabs".to_string(), json!(true));
    args_map.insert("number".to_string(), json!(true));

    let (_, args_vec) = prepare_command_and_args(
        "cat",
        Some(&args_map),
        Some(&subcommand_config),
        Path::new("."),
        &temp_manager,
    )
    .await
    .expect("command");

    assert!(
        args_vec.contains(&"-e".to_string()),
        "show-ends should use BSD-compatible -e alias. Got: {:?}",
        args_vec
    );
    assert!(
        args_vec.contains(&"-t".to_string()),
        "show-tabs should use BSD-compatible -t alias. Got: {:?}",
        args_vec
    );
    assert!(
        args_vec.contains(&"-n".to_string()),
        "number should use -n alias. Got: {:?}",
        args_vec
    );
    // Should NOT contain GNU-only uppercase flags
    assert!(
        !args_vec.iter().any(|s| s == "-E"),
        "Should NOT emit GNU-only -E for show-ends. Got: {:?}",
        args_vec
    );
    assert!(
        !args_vec.iter().any(|s| s == "-T"),
        "Should NOT emit GNU-only -T for show-tabs. Got: {:?}",
        args_vec
    );
}
