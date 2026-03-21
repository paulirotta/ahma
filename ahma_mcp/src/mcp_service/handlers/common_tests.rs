use super::*;
use crate::operation_monitor::{Operation, OperationStatus};
use serde_json::json;

fn make_map(pairs: &[(&str, &str)]) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert(k.to_string(), json!(*v));
    }
    m
}

fn make_op(id: &str, tool: &str, status: OperationStatus) -> Operation {
    let mut op = Operation::new(id.to_string(), tool.to_string(), String::new(), None);
    op.state = status;
    op
}

#[test]
fn test_parse_comma_separated_filter_basic() {
    let args = make_map(&[("tools", "cargo,clippy,nextest")]);
    let result = parse_comma_separated_filter(&args, "tools");
    assert_eq!(result, vec!["cargo", "clippy", "nextest"]);
}

#[test]
fn test_parse_comma_separated_filter_trims_whitespace() {
    let args = make_map(&[("tools", "  cargo , clippy , nextest  ")]);
    let result = parse_comma_separated_filter(&args, "tools");
    assert_eq!(result, vec!["cargo", "clippy", "nextest"]);
}

#[test]
fn test_parse_comma_separated_filter_filters_empty_segments() {
    let args = make_map(&[("tools", "cargo,,nextest,")]);
    let result = parse_comma_separated_filter(&args, "tools");
    assert_eq!(result, vec!["cargo", "nextest"]);
}

#[test]
fn test_parse_comma_separated_filter_missing_key() {
    let args = make_map(&[]);
    let result = parse_comma_separated_filter(&args, "tools");
    assert!(result.is_empty());
}

#[test]
fn test_parse_comma_separated_filter_single_value() {
    let args = make_map(&[("tools", "cargo")]);
    let result = parse_comma_separated_filter(&args, "tools");
    assert_eq!(result, vec!["cargo"]);
}

#[test]
fn test_parse_comma_separated_filter_only_commas() {
    let args = make_map(&[("tools", ",,,")]);
    let result = parse_comma_separated_filter(&args, "tools");
    assert!(result.is_empty());
}

#[test]
fn test_parse_tool_filters_delegates_to_tools_key() {
    let args = make_map(&[("tools", "cargo,clippy")]);
    let result = parse_tool_filters(&args);
    assert_eq!(result, vec!["cargo", "clippy"]);
}

#[test]
fn test_parse_tool_filters_empty_args() {
    let args = make_map(&[]);
    assert!(parse_tool_filters(&args).is_empty());
}

#[test]
fn test_parse_id_present() {
    let args = make_map(&[("id", "op-1234")]);
    assert_eq!(parse_id(&args), Some("op-1234".to_string()));
}

#[test]
fn test_parse_id_absent() {
    let args = make_map(&[]);
    assert_eq!(parse_id(&args), None);
}

#[test]
fn test_operation_matches_filters_no_filters_no_id() {
    let op = make_op("op-1", "cargo_build", OperationStatus::Completed);
    assert!(operation_matches_filters(&op, &[], None));
}

#[test]
fn test_operation_matches_filters_matching_tool_prefix() {
    let op = make_op("op-1", "cargo_build", OperationStatus::Completed);
    let filters = vec!["cargo".to_string()];
    assert!(operation_matches_filters(&op, &filters, None));
}

#[test]
fn test_operation_matches_filters_non_matching_tool_prefix() {
    let op = make_op("op-1", "cargo_build", OperationStatus::Completed);
    let filters = vec!["npm".to_string()];
    assert!(!operation_matches_filters(&op, &filters, None));
}

#[test]
fn test_operation_matches_filters_matching_id() {
    let op = make_op("op-42", "cargo_build", OperationStatus::Completed);
    assert!(operation_matches_filters(&op, &[], Some("op-42")));
}

#[test]
fn test_operation_matches_filters_non_matching_id() {
    let op = make_op("op-42", "cargo_build", OperationStatus::Completed);
    assert!(!operation_matches_filters(&op, &[], Some("op-99")));
}

#[test]
fn test_operation_matches_filters_tool_and_id_both_match() {
    let op = make_op("op-42", "cargo_build", OperationStatus::Completed);
    let filters = vec!["cargo".to_string()];
    assert!(operation_matches_filters(&op, &filters, Some("op-42")));
}

#[test]
fn test_operation_matches_filters_tool_matches_but_id_mismatch() {
    let op = make_op("op-42", "cargo_build", OperationStatus::Completed);
    let filters = vec!["cargo".to_string()];
    assert!(!operation_matches_filters(&op, &filters, Some("op-99")));
}

#[test]
fn test_serialize_operations_to_content_empty() {
    let ops: Vec<Operation> = vec![];
    let result = serialize_operations_to_content(&ops);
    assert!(result.is_empty());
}

#[test]
fn test_serialize_operations_to_content_single_op() {
    let op = make_op("op-1", "cargo_build", OperationStatus::Completed);
    let result = serialize_operations_to_content(&[op]);
    assert_eq!(result.len(), 1);
    let text = result[0].as_text().map(|t| t.text.as_str()).unwrap_or("");
    assert!(
        text.contains("op-1"),
        "Serialized content should contain op id: {text}"
    );
}

#[test]
fn test_serialize_operations_to_content_multiple_ops() {
    let ops = vec![
        make_op("op-1", "cargo_build", OperationStatus::Completed),
        make_op("op-2", "cargo_test", OperationStatus::Failed),
    ];
    let result = serialize_operations_to_content(&ops);
    assert_eq!(result.len(), 2);
}

#[test]
fn test_extract_output_none_result() {
    assert_eq!(extract_output_from_result(&None), "");
}

#[test]
fn test_extract_output_string_result() {
    let result = Some(json!("error: compilation failed"));
    assert_eq!(
        extract_output_from_result(&result),
        "error: compilation failed"
    );
}

#[test]
fn test_extract_output_stdout_only_exit_zero() {
    let result = Some(json!({
        "stdout": "hello world",
        "stderr": "",
        "exit_code": 0
    }));
    assert_eq!(extract_output_from_result(&result), "hello world");
}

#[test]
fn test_extract_output_stderr_only_exit_zero() {
    let result = Some(json!({
        "stdout": "",
        "stderr": "warning: unused variable",
        "exit_code": 0
    }));
    assert_eq!(
        extract_output_from_result(&result),
        "warning: unused variable"
    );
}

#[test]
fn test_extract_output_both_stdout_and_stderr_exit_zero() {
    let result = Some(json!({
        "stdout": "output",
        "stderr": "warning",
        "exit_code": 0
    }));
    let out = extract_output_from_result(&result);
    assert!(out.contains("output"));
    assert!(out.contains("warning"));
}

#[test]
fn test_extract_output_nonzero_exit_code() {
    let result = Some(json!({
        "stdout": "some output",
        "stderr": "error text",
        "exit_code": 1
    }));
    let out = extract_output_from_result(&result);
    assert!(
        out.contains("Exit code: 1"),
        "Non-zero exit should show exit code: {out}"
    );
    assert!(out.contains("some output"));
    assert!(out.contains("error text"));
}

#[test]
fn test_extract_output_arbitrary_json_fallback() {
    let result = Some(json!({"nested": {"key": "value"}}));
    let out = extract_output_from_result(&result);
    assert!(
        out.contains("nested"),
        "Fallback should serialize JSON: {out}"
    );
}
