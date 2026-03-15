use anyhow::Result;
use serde_json::Value;

/// Resolves a boolean value from a JSON value.
/// Handles both native boolean values and string representations ("true"/"false").
/// Returns `None` for non-bool/non-string types (numbers, null, arrays, objects).
pub(super) fn resolve_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::String(s) => Some(s.eq_ignore_ascii_case("true")),
        _ => None,
    }
}

/// Converts a serde_json::Value to a string, handling recursion.
pub(super) fn coerce_cli_value(value: &Value) -> Result<Option<String>> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        Value::Number(n) => Ok(Some(n.to_string())),
        Value::Bool(b) => Ok(Some(b.to_string())),
        Value::Array(arr) => {
            let mut result = Vec::new();
            for item in arr {
                if let Some(s) = coerce_cli_value(item)? {
                    result.push(s);
                }
            }
            if result.is_empty() {
                return Ok(None);
            }
            Ok(Some(result.join(" ")))
        }
        // For other types like Object, we don't want to convert them to a string.
        _ => Ok(None),
    }
}

pub(super) fn is_reserved_runtime_key(key: &str) -> bool {
    matches!(
        key,
        "args" | "working_directory" | "execution_mode" | "timeout_seconds"
    )
}

/// Checks if a string contains characters that are problematic for shell argument passing
pub fn needs_file_handling(value: &str) -> bool {
    value.contains('\n')
        || value.contains('\r')
        || value.contains('\'')
        || value.contains('"')
        || value.contains('\\')
        || value.contains('`')
        || value.contains('$')
        || value.len() > 8192 // Also handle very long arguments via file
}

/// Formats an option name as a command-line flag.
///
/// If the option name already starts with a dash (e.g., "-name" for `find`),
/// it's used as-is. Otherwise, it's prefixed with "--" for standard long options.
pub fn format_option_flag(key: &str) -> String {
    if key.starts_with('-') {
        key.to_string()
    } else {
        format!("--{}", key)
    }
}

/// Prepare a string for shell argument passing by escaping special characters.
///
/// This function wraps the string in single quotes and handles any embedded single quotes.
///
/// # Purpose
///
/// "Escaping" here means neutralizing special characters (like spaces, `$`, quotes, etc.)
/// so the shell treats the value as a single piece of text (a literal string) rather than
/// interpreting it as code or multiple arguments. This prevents "shell injection" attacks.
///
/// # When to use
///
/// Use this only as a fallback when you cannot use file-based data passing. Passing data
/// through temporary files is generally robust, but if you must construct a raw command
/// string with arguments, this function ensures those arguments are safe.
pub fn escape_shell_argument(value: &str) -> String {
    // Use single quotes and escape any embedded single quotes
    if value.contains('\'') {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    } else {
        format!("'{}'", value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    // --- resolve_bool ---
    #[test]
    fn test_resolve_bool_native_true() {
        assert_eq!(resolve_bool(&Value::Bool(true)), Some(true));
    }

    #[test]
    fn test_resolve_bool_native_false() {
        assert_eq!(resolve_bool(&Value::Bool(false)), Some(false));
    }

    #[test]
    fn test_resolve_bool_string_true_lowercase() {
        assert_eq!(resolve_bool(&json!("true")), Some(true));
    }

    #[test]
    fn test_resolve_bool_string_true_uppercase() {
        assert_eq!(resolve_bool(&json!("TRUE")), Some(true));
    }

    #[test]
    fn test_resolve_bool_string_false() {
        assert_eq!(resolve_bool(&json!("false")), Some(false));
    }

    #[test]
    fn test_resolve_bool_non_bool_returns_none() {
        assert_eq!(resolve_bool(&json!(42)), None);
        assert_eq!(resolve_bool(&Value::Null), None);
    }

    #[test]
    fn test_resolve_bool_string_not_true_is_false() {
        // Any string that's not "true" (case-insensitive) returns Some(false)
        assert_eq!(resolve_bool(&json!("not a bool")), Some(false));
        assert_eq!(resolve_bool(&json!("yes")), Some(false));
        assert_eq!(resolve_bool(&json!("1")), Some(false));
    }

    // --- coerce_cli_value ---
    #[test]
    fn test_coerce_cli_value_null() {
        assert_eq!(coerce_cli_value(&Value::Null).unwrap(), None);
    }

    #[test]
    fn test_coerce_cli_value_string() {
        assert_eq!(
            coerce_cli_value(&json!("hello")).unwrap(),
            Some("hello".to_string())
        );
    }

    #[test]
    fn test_coerce_cli_value_number() {
        assert_eq!(
            coerce_cli_value(&json!(42)).unwrap(),
            Some("42".to_string())
        );
        assert_eq!(
            coerce_cli_value(&json!(1.5)).unwrap(),
            Some("1.5".to_string())
        );
    }

    #[test]
    fn test_coerce_cli_value_bool() {
        assert_eq!(
            coerce_cli_value(&json!(true)).unwrap(),
            Some("true".to_string())
        );
        assert_eq!(
            coerce_cli_value(&json!(false)).unwrap(),
            Some("false".to_string())
        );
    }

    #[test]
    fn test_coerce_cli_value_array_non_empty() {
        assert_eq!(
            coerce_cli_value(&json!(["a", "b", "c"])).unwrap(),
            Some("a b c".to_string())
        );
    }

    #[test]
    fn test_coerce_cli_value_array_with_nulls_skipped() {
        assert_eq!(
            coerce_cli_value(&json!(["a", null, "c"])).unwrap(),
            Some("a c".to_string())
        );
    }

    #[test]
    fn test_coerce_cli_value_array_empty() {
        assert_eq!(coerce_cli_value(&json!([])).unwrap(), None);
    }

    #[test]
    fn test_coerce_cli_value_array_all_nulls() {
        assert_eq!(coerce_cli_value(&json!([null, null])).unwrap(), None);
    }

    #[test]
    fn test_coerce_cli_value_object_returns_none() {
        assert_eq!(coerce_cli_value(&json!({"key": "val"})).unwrap(), None);
    }

    // --- is_reserved_runtime_key ---
    #[test]
    fn test_is_reserved_runtime_key_reserved() {
        assert!(is_reserved_runtime_key("args"));
        assert!(is_reserved_runtime_key("working_directory"));
        assert!(is_reserved_runtime_key("execution_mode"));
        assert!(is_reserved_runtime_key("timeout_seconds"));
    }

    #[test]
    fn test_is_reserved_runtime_key_not_reserved() {
        assert!(!is_reserved_runtime_key("name"));
        assert!(!is_reserved_runtime_key("path"));
        assert!(!is_reserved_runtime_key(""));
    }

    // --- needs_file_handling ---
    #[test]
    fn test_needs_file_handling_newline() {
        assert!(needs_file_handling("line1\nline2"));
    }

    #[test]
    fn test_needs_file_handling_carriage_return() {
        assert!(needs_file_handling("line1\rline2"));
    }

    #[test]
    fn test_needs_file_handling_single_quote() {
        assert!(needs_file_handling("it's"));
    }

    #[test]
    fn test_needs_file_handling_double_quote() {
        assert!(needs_file_handling(r#"say "hello""#));
    }

    #[test]
    fn test_needs_file_handling_backslash() {
        assert!(needs_file_handling("path\\to\\file"));
    }

    #[test]
    fn test_needs_file_handling_backtick() {
        assert!(needs_file_handling("run `command`"));
    }

    #[test]
    fn test_needs_file_handling_dollar() {
        assert!(needs_file_handling("$HOME"));
    }

    #[test]
    fn test_needs_file_handling_long_string() {
        let long = "a".repeat(8193);
        assert!(needs_file_handling(&long));
    }

    #[test]
    fn test_needs_file_handling_safe_string() {
        assert!(!needs_file_handling("simple"));
        assert!(!needs_file_handling("path/to/file"));
        assert!(!needs_file_handling(&"a".repeat(8192)));
    }

    // --- format_option_flag ---
    #[test]
    fn test_format_option_flag_standard() {
        assert_eq!(format_option_flag("verbose"), "--verbose");
    }

    #[test]
    fn test_format_option_flag_dash_prefixed() {
        assert_eq!(format_option_flag("-name"), "-name");
    }

    // --- escape_shell_argument ---
    #[test]
    fn test_escape_shell_argument_no_quotes() {
        assert_eq!(escape_shell_argument("simple"), "'simple'");
    }

    #[test]
    fn test_escape_shell_argument_with_single_quotes() {
        assert_eq!(
            escape_shell_argument("it's"),
            "'it'\"'\"'s'"
        );
    }
}
