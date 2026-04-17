//! File Tools Schema Validation Tests
//!
//! These tests validate that tool definition JSON files have correct option aliases
//! and are compatible with BSD/macOS command-line tools. They catch issues like:
//! - GNU-only flags that don't work on macOS BSD (e.g., cat -E, cat -T)
//! - Typos in option names
//! - Aliases that don't match the actual tool's flag letters

use ahma_mcp::config::{SubcommandConfig, ToolConfig};
use std::collections::HashSet;
use std::path::PathBuf;

fn workspace_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to locate workspace root")
        .to_path_buf()
}

fn load_tool_config(name: &str) -> ToolConfig {
    let config_path = workspace_dir().join(".ahma").join(format!("{}.json", name));
    let contents = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", config_path.display(), e));
    serde_json::from_str(&contents)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {}", config_path.display(), e))
}

/// Assert that a single option alias is a valid single alphanumeric character.
fn validate_option_alias(opt_name: &str, subcmd_name: &str, alias: &str) {
    assert!(
        alias.len() == 1,
        "Option '{}' in subcommand '{}' has multi-char alias '{}' — \
         aliases must be single characters for -X flag format",
        opt_name,
        subcmd_name,
        alias
    );
    let ch = alias.chars().next().unwrap();
    assert!(
        ch.is_ascii_alphanumeric() || ch == '1',
        "Option '{}' in subcommand '{}' has non-alphanumeric alias '{}' — \
         aliases should be alphanumeric characters",
        opt_name,
        subcmd_name,
        alias
    );
}

/// Assert that a single option name is not a known typo.
fn assert_no_known_typos(opt_name: &str, subcmd_name: &str) {
    assert_ne!(
        opt_name, "heml",
        "Option name 'heml' in subcommand '{}' appears to be a typo for 'html'",
        subcmd_name
    );
}

/// Assert that a subcommand has no duplicate option names or aliases.
fn assert_no_duplicates_in_subcommand(subcmd: &SubcommandConfig) {
    let mut seen_names = HashSet::new();
    let mut seen_aliases = HashSet::new();

    if let Some(options) = &subcmd.options {
        for opt in options {
            assert!(
                seen_names.insert(&opt.name),
                "Duplicate option name '{}' in subcommand '{}'",
                opt.name,
                subcmd.name
            );
            if let Some(alias) = &opt.alias {
                assert!(
                    seen_aliases.insert(alias.clone()),
                    "Duplicate alias '{}' in subcommand '{}' (on option '{}')",
                    alias,
                    subcmd.name,
                    opt.name
                );
            }
        }
    }

    if let Some(positional) = &subcmd.positional_args {
        for arg in positional {
            assert!(
                seen_names.insert(&arg.name),
                "Positional arg '{}' conflicts with option name in subcommand '{}'",
                arg.name,
                subcmd.name
            );
        }
    }
}

#[test]
fn all_file_tools_options_have_valid_single_char_aliases() {
    let config = load_tool_config("file-tools");
    let subcommands = config
        .subcommand
        .expect("file-tools should have subcommands");

    for subcmd in &subcommands {
        if let Some(options) = &subcmd.options {
            for opt in options {
                if let Some(alias) = &opt.alias {
                    validate_option_alias(&opt.name, &subcmd.name, alias);
                }
            }
        }
    }
}

#[test]
fn cat_options_use_bsd_compatible_aliases() {
    let config = load_tool_config("file-tools");
    let subcommands = config
        .subcommand
        .expect("file-tools should have subcommands");
    let cat = subcommands
        .iter()
        .find(|s| s.name == "cat")
        .expect("cat subcommand should exist");

    let options = cat.options.as_ref().expect("cat should have options");

    // Check show-ends uses lowercase 'e' (BSD compatible), not 'E' (GNU only)
    let show_ends = options.iter().find(|o| o.name == "show-ends");
    if let Some(opt) = show_ends {
        assert_eq!(
            opt.alias.as_deref(),
            Some("e"),
            "cat show-ends should use BSD-compatible alias 'e', not GNU-only 'E'"
        );
    }

    // Check show-tabs uses lowercase 't' (BSD compatible), not 'T' (GNU only)
    let show_tabs = options.iter().find(|o| o.name == "show-tabs");
    if let Some(opt) = show_tabs {
        assert_eq!(
            opt.alias.as_deref(),
            Some("t"),
            "cat show-tabs should use BSD-compatible alias 't', not GNU-only 'T'"
        );
    }

    // Check number uses 'n'
    let number = options
        .iter()
        .find(|o| o.name == "number")
        .expect("cat should have number option");
    assert_eq!(
        number.alias.as_deref(),
        Some("n"),
        "cat number should use alias 'n'"
    );
}

#[test]
fn grep_options_are_bsd_compatible() {
    let config = load_tool_config("file-tools");
    let subcommands = config
        .subcommand
        .expect("file-tools should have subcommands");
    let grep = subcommands
        .iter()
        .find(|s| s.name == "grep")
        .expect("grep subcommand should exist");

    let options = grep.options.as_ref().expect("grep should have options");

    // Verify all expected grep options exist with correct aliases
    let expected: Vec<(&str, &str)> = vec![
        ("recursive", "r"),
        ("ignore-case", "i"),
        ("line-number", "n"),
        ("count", "c"),
        ("files-with-matches", "l"),
        ("invert-match", "v"),
        ("word-regexp", "w"),
        ("extended-regexp", "E"),
    ];

    for (name, expected_alias) in &expected {
        let opt = options
            .iter()
            .find(|o| o.name == *name)
            .unwrap_or_else(|| panic!("grep should have option '{}'", name));
        assert_eq!(
            opt.alias.as_deref(),
            Some(*expected_alias),
            "grep option '{}' should have alias '{}'",
            name,
            expected_alias
        );
    }

    // Verify grep does NOT define 'source' as an option (it's positional on mv/cp only)
    let has_source = options.iter().any(|o| o.name == "source");
    assert!(
        !has_source,
        "grep should NOT have a 'source' option — that belongs to mv/cp"
    );

    // Verify 'pattern' is a positional arg, not an option
    let positional = grep
        .positional_args
        .as_ref()
        .expect("grep should have positional args");
    assert!(
        positional.iter().any(|p| p.name == "pattern"),
        "grep 'pattern' should be a positional arg"
    );
}

#[test]
fn touch_timestamp_format_is_correct() {
    let config = load_tool_config("file-tools");
    let subcommands = config
        .subcommand
        .expect("file-tools should have subcommands");
    let touch = subcommands
        .iter()
        .find(|s| s.name == "touch")
        .expect("touch subcommand should exist");

    let options = touch.options.as_ref().expect("touch should have options");
    let timestamp = options
        .iter()
        .find(|o| o.name == "timestamp")
        .expect("touch should have timestamp option");

    let desc = timestamp
        .description
        .as_deref()
        .expect("timestamp should have description");

    // Should NOT contain the incorrect YYYY-MM-DD format
    assert!(
        !desc.contains("YYYY-MM-DD"),
        "touch timestamp description should not use YYYY-MM-DD format (wrong for both BSD and GNU touch -t). \
         Got: {}",
        desc
    );
    // Should contain the correct format
    assert!(
        desc.contains("MMDDhhmm"),
        "touch timestamp description should document the correct format [[CC]YY]MMDDhhmm[.SS]. \
         Got: {}",
        desc
    );
}

#[test]
fn sed_in_place_description_mentions_bsd() {
    let config = load_tool_config("file-tools");
    let subcommands = config
        .subcommand
        .expect("file-tools should have subcommands");
    let sed = subcommands
        .iter()
        .find(|s| s.name == "sed")
        .expect("sed subcommand should exist");

    let options = sed.options.as_ref().expect("sed should have options");
    let in_place = options
        .iter()
        .find(|o| o.name == "in-place")
        .expect("sed should have in-place option");

    let desc = in_place
        .description
        .as_deref()
        .expect("in-place should have description");

    // Description should mention BSD compatibility since BSD sed -i requires a suffix argument
    assert!(
        desc.to_lowercase().contains("bsd"),
        "sed in-place description should mention BSD compatibility. Got: {}",
        desc
    );
}

#[test]
fn no_option_names_are_typos_in_simplify() {
    let config = load_tool_config("simplify");
    let subcommands = config.subcommand.expect("simplify should have subcommands");

    for subcmd in &subcommands {
        if let Some(options) = &subcmd.options {
            for opt in options {
                assert_no_known_typos(&opt.name, &subcmd.name);
            }
        }
    }
}

#[test]
fn all_tool_configs_parse_successfully() {
    // Validate that all .ahma/*.json files parse without errors
    let tools_dir = workspace_dir().join(".ahma");
    let entries: Vec<_> = std::fs::read_dir(&tools_dir)
        .unwrap_or_else(|e| panic!("Failed to read .ahma directory: {}", e))
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect();

    assert!(
        !entries.is_empty(),
        ".ahma/ directory should contain JSON tool files"
    );

    for entry in &entries {
        let path = entry.path();
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e));
        let _config: ToolConfig = serde_json::from_str(&contents)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {}", path.display(), e));
    }
}

#[test]
fn no_duplicate_option_names_within_subcommands() {
    let config = load_tool_config("file-tools");
    let subcommands = config
        .subcommand
        .expect("file-tools should have subcommands");

    for subcmd in &subcommands {
        assert_no_duplicates_in_subcommand(subcmd);
    }
}
