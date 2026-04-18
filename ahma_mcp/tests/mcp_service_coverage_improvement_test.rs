//! MCP Service Coverage Improvement Tests
//!
//! These tests target low-coverage areas in mcp_service.rs to improve overall
//! code coverage. Based on coverage analysis showing 25.73% line coverage.

use ahma_mcp::test_utils::in_process::create_in_process_mcp_from_dir;
use ahma_mcp::utils::logging::init_test_logging;
use serde_json::json;
use tempfile::tempdir;

/// Test the normalize_option_type function behavior through schema generation
#[tokio::test]
async fn test_schema_generation_normalizes_option_types() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();

    // Create a tool config with various option types
    let tool_json = json!({
        "name": "test_types",
        "description": "Test option type normalization",
        "command": "test",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Default subcommand",
            "options": [
                {"name": "bool_opt", "type": "bool", "description": "Boolean option"},
                {"name": "boolean_opt", "type": "boolean", "description": "Boolean option"},
                {"name": "int_opt", "type": "int", "description": "Integer option"},
                {"name": "integer_opt", "type": "integer", "description": "Integer option"},
                {"name": "array_opt", "type": "array", "description": "Array option"},
                {"name": "number_opt", "type": "number", "description": "Number option"},
                {"name": "string_opt", "type": "string", "description": "String option"},
                {"name": "unknown_opt", "type": "unknown", "description": "Unknown type defaults to string"}
            ]
        }]
    });

    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir).unwrap();
    std::fs::write(
        tools_dir.join("test_types.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.unwrap();

    let tools = mcp.client.list_all_tools().await.unwrap();
    let test_tool = tools.iter().find(|t| t.name.as_ref() == "test_types");
    assert!(test_tool.is_some(), "test_types tool should be registered");

    let tool = test_tool.unwrap();
    let schema = tool.input_schema.as_ref();
    let properties = schema.get("properties").unwrap().as_object().unwrap();

    // Verify type normalization
    assert_eq!(
        properties.get("bool_opt").unwrap()["type"],
        "boolean",
        "bool should normalize to boolean"
    );
    assert_eq!(
        properties.get("boolean_opt").unwrap()["type"],
        "boolean",
        "boolean should remain boolean"
    );
    assert_eq!(
        properties.get("int_opt").unwrap()["type"],
        "integer",
        "int should normalize to integer"
    );
    assert_eq!(
        properties.get("integer_opt").unwrap()["type"],
        "integer",
        "integer should remain integer"
    );
    assert_eq!(
        properties.get("array_opt").unwrap()["type"],
        "array",
        "array should remain array"
    );
    assert_eq!(
        properties.get("number_opt").unwrap()["type"],
        "number",
        "number should remain number"
    );
    assert_eq!(
        properties.get("string_opt").unwrap()["type"],
        "string",
        "string should remain string"
    );
    assert_eq!(
        properties.get("unknown_opt").unwrap()["type"],
        "string",
        "unknown types should default to string"
    );
}

/// Test schema generation for array options with items specification
#[tokio::test]
async fn test_schema_generation_array_with_items() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();

    let tool_json = json!({
        "name": "test_arrays",
        "description": "Test array option with items",
        "command": "test",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Default subcommand",
            "options": [
                {
                    "name": "simple_array",
                    "type": "array",
                    "description": "Simple string array"
                },
                {
                    "name": "typed_array",
                    "type": "array",
                    "description": "Array with item type spec",
                    "items": {
                        "type": "string",
                        "format": "path",
                        "description": "File path item"
                    }
                },
                {
                    "name": "format_array",
                    "type": "array",
                    "description": "Array with format on option",
                    "format": "path"
                }
            ]
        }]
    });

    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir).unwrap();
    std::fs::write(
        tools_dir.join("test_arrays.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.unwrap();

    let tools = mcp.client.list_all_tools().await.unwrap();
    let test_tool = tools.iter().find(|t| t.name.as_ref() == "test_arrays");
    assert!(test_tool.is_some(), "test_arrays tool should be registered");

    let tool = test_tool.unwrap();
    let schema = tool.input_schema.as_ref();
    let properties = schema.get("properties").unwrap().as_object().unwrap();

    // Verify simple array has items
    let simple_array = properties.get("simple_array").unwrap();
    assert_eq!(simple_array["type"], "array");
    assert!(
        simple_array.get("items").is_some(),
        "Array should have items"
    );
    assert_eq!(simple_array["items"]["type"], "string");

    // Verify typed array with full items spec
    let typed_array = properties.get("typed_array").unwrap();
    assert_eq!(typed_array["type"], "array");
    let items = typed_array.get("items").unwrap();
    assert_eq!(items["type"], "string");
    assert_eq!(items["format"], "path");
    assert_eq!(items["description"], "File path item");
}

/// Test schema generation with positional arguments
#[tokio::test]
async fn test_schema_generation_positional_args() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();

    let tool_json = json!({
        "name": "test_positional",
        "description": "Test positional arguments",
        "command": "test",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Default subcommand",
            "positional_args": [
                {
                    "name": "required_arg",
                    "type": "string",
                    "description": "Required positional arg",
                    "required": true
                },
                {
                    "name": "optional_arg",
                    "type": "string",
                    "description": "Optional positional arg",
                    "required": false
                },
                {
                    "name": "path_arg",
                    "type": "string",
                    "description": "Path positional arg",
                    "format": "path"
                },
                {
                    "name": "array_arg",
                    "type": "array",
                    "description": "Array positional arg",
                    "items": {"type": "string"}
                }
            ]
        }]
    });

    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir).unwrap();
    std::fs::write(
        tools_dir.join("test_positional.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.unwrap();

    let tools = mcp.client.list_all_tools().await.unwrap();
    let test_tool = tools.iter().find(|t| t.name.as_ref() == "test_positional");
    assert!(
        test_tool.is_some(),
        "test_positional tool should be registered"
    );

    let tool = test_tool.unwrap();
    let schema = tool.input_schema.as_ref();
    let properties = schema.get("properties").unwrap().as_object().unwrap();
    let required = schema.get("required").map(|r| r.as_array().unwrap());

    // Verify positional args are in schema
    assert!(
        properties.contains_key("required_arg"),
        "required_arg should be in properties"
    );
    assert!(
        properties.contains_key("optional_arg"),
        "optional_arg should be in properties"
    );
    assert!(
        properties.contains_key("path_arg"),
        "path_arg should be in properties"
    );

    // Verify required field is set correctly
    assert!(required.is_some(), "Should have required array");
    let required_names: Vec<&str> = required
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        required_names.contains(&"required_arg"),
        "required_arg should be in required"
    );
    assert!(
        !required_names.contains(&"optional_arg"),
        "optional_arg should not be in required"
    );

    // Verify format is preserved
    let path_arg = properties.get("path_arg").unwrap();
    assert_eq!(path_arg["format"], "path");

    // Verify array positional arg
    let array_arg = properties.get("array_arg").unwrap();
    assert_eq!(array_arg["type"], "array");
    assert!(array_arg.get("items").is_some());
}

/// Test schema generation with multiple subcommands using enum
#[tokio::test]
async fn test_schema_generation_multiple_subcommands() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();

    let tool_json = json!({
        "name": "test_multi_sub",
        "description": "Test multiple subcommands",
        "command": "test",
        "enabled": true,
        "subcommand": [
            {
                "name": "build",
                "description": "Build subcommand",
                "options": [
                    {"name": "release", "type": "boolean", "description": "Release mode"}
                ]
            },
            {
                "name": "test",
                "description": "Test subcommand",
                "options": [
                    {"name": "verbose", "type": "boolean", "description": "Verbose output", "required": true}
                ]
            },
            {
                "name": "run",
                "description": "Run subcommand",
                "options": [
                    {"name": "args", "type": "array", "description": "Arguments"}
                ]
            }
        ]
    });

    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir).unwrap();
    std::fs::write(
        tools_dir.join("test_multi_sub.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.unwrap();

    let tools = mcp.client.list_all_tools().await.unwrap();

    // Subcommands are flattened into individual tools: <parent>_<subcommand>
    let build_tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "test_multi_sub_build");
    let test_tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "test_multi_sub_test");
    let run_tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "test_multi_sub_run");

    assert!(
        build_tool.is_some(),
        "test_multi_sub_build should be registered"
    );
    assert!(
        test_tool.is_some(),
        "test_multi_sub_test should be registered"
    );
    assert!(
        run_tool.is_some(),
        "test_multi_sub_run should be registered"
    );

    // Verify build tool has release option
    let build_schema = build_tool.unwrap().input_schema.as_ref();
    let build_props = build_schema.get("properties").unwrap().as_object().unwrap();
    assert!(
        build_props.contains_key("release"),
        "release should be in build tool"
    );

    // Verify test tool has verbose option (required)
    let test_schema = test_tool.unwrap().input_schema.as_ref();
    let test_props = test_schema.get("properties").unwrap().as_object().unwrap();
    assert!(
        test_props.contains_key("verbose"),
        "verbose should be in test tool"
    );
    let test_required = test_schema.get("required").unwrap().as_array().unwrap();
    let test_required_strs: Vec<&str> = test_required.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        test_required_strs.contains(&"verbose"),
        "verbose should be required in test tool"
    );

    // Verify run tool has args option
    let run_schema = run_tool.unwrap().input_schema.as_ref();
    let run_props = run_schema.get("properties").unwrap().as_object().unwrap();
    assert!(run_props.contains_key("args"), "args should be in run tool");
}

/// Test nested subcommands schema generation
#[tokio::test]
async fn test_schema_generation_nested_subcommands() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();

    let tool_json = json!({
        "name": "test_nested",
        "description": "Test nested subcommands",
        "command": "test",
        "enabled": true,
        "subcommand": [
            {
                "name": "parent",
                "description": "Parent subcommand",
                "subcommand": [
                    {
                        "name": "child1",
                        "description": "Child 1 subcommand",
                        "options": [
                            {"name": "opt1", "type": "string", "description": "Option 1"}
                        ]
                    },
                    {
                        "name": "child2",
                        "description": "Child 2 subcommand",
                        "options": [
                            {"name": "opt2", "type": "boolean", "description": "Option 2"}
                        ]
                    }
                ]
            }
        ]
    });

    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir).unwrap();
    std::fs::write(
        tools_dir.join("test_nested.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.unwrap();

    let tools = mcp.client.list_all_tools().await.unwrap();

    // Nested subcommands flatten to <parent>_<sub>_<child> tools
    let child1_tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "test_nested_parent_child1");
    let child2_tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "test_nested_parent_child2");

    assert!(
        child1_tool.is_some(),
        "test_nested_parent_child1 should be registered"
    );
    assert!(
        child2_tool.is_some(),
        "test_nested_parent_child2 should be registered"
    );

    // Verify opt1 is in child1 schema
    let child1_schema = child1_tool.unwrap().input_schema.as_ref();
    let child1_props = child1_schema
        .get("properties")
        .unwrap()
        .as_object()
        .unwrap();
    assert!(
        child1_props.contains_key("opt1"),
        "opt1 should be in test_nested_parent_child1"
    );

    // Verify opt2 is in child2 schema
    let child2_schema = child2_tool.unwrap().input_schema.as_ref();
    let child2_props = child2_schema
        .get("properties")
        .unwrap()
        .as_object()
        .unwrap();
    assert!(
        child2_props.contains_key("opt2"),
        "opt2 should be in test_nested_parent_child2"
    );
}

/// Test disabled tools and subcommands are skipped
#[tokio::test]
async fn test_disabled_tools_not_in_list() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();

    let disabled_tool = json!({
        "name": "disabled_tool",
        "description": "This tool is disabled",
        "command": "disabled",
        "enabled": false
    });

    let enabled_tool = json!({
        "name": "enabled_tool",
        "description": "This tool is enabled",
        "command": "echo",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Default"
        }]
    });

    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir).unwrap();
    std::fs::write(
        tools_dir.join("disabled_tool.json"),
        serde_json::to_string_pretty(&disabled_tool).unwrap(),
    )
    .unwrap();
    std::fs::write(
        tools_dir.join("enabled_tool.json"),
        serde_json::to_string_pretty(&enabled_tool).unwrap(),
    )
    .unwrap();

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.unwrap();

    let tools = mcp.client.list_all_tools().await.unwrap();

    let disabled = tools.iter().find(|t| t.name.as_ref() == "disabled_tool");
    let enabled = tools.iter().find(|t| t.name.as_ref() == "enabled_tool");

    assert!(disabled.is_none(), "Disabled tool should not be in list");
    assert!(enabled.is_some(), "Enabled tool should be in list");
}

/// Test disabled subcommands are skipped in schema generation
#[tokio::test]
async fn test_disabled_subcommands_skipped() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();

    let tool_json = json!({
        "name": "test_disabled_sub",
        "description": "Test with disabled subcommand",
        "command": "test",
        "enabled": true,
        "subcommand": [
            {
                "name": "enabled_sub",
                "description": "Enabled subcommand",
                "enabled": true
            },
            {
                "name": "disabled_sub",
                "description": "Disabled subcommand",
                "enabled": false
            }
        ]
    });

    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir).unwrap();
    std::fs::write(
        tools_dir.join("test_disabled_sub.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.unwrap();

    let tools = mcp.client.list_all_tools().await.unwrap();

    // Subcommands are flattened into individual tools: <parent>_<subcommand>
    let enabled_tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "test_disabled_sub_enabled_sub");
    let disabled_tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "test_disabled_sub_disabled_sub");

    assert!(
        enabled_tool.is_some(),
        "test_disabled_sub_enabled_sub tool should be registered"
    );
    assert!(
        disabled_tool.is_none(),
        "test_disabled_sub_disabled_sub tool should NOT be registered"
    );
}

/// Test default subcommand handling (no subcommand parameter needed)
#[tokio::test]
async fn test_default_subcommand_no_enum() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();

    let tool_json = json!({
        "name": "test_default_only",
        "description": "Test with only default subcommand",
        "command": "test",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "The only subcommand",
            "options": [
                {"name": "flag", "type": "boolean", "description": "A flag"}
            ]
        }]
    });

    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir).unwrap();
    std::fs::write(
        tools_dir.join("test_default_only.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.unwrap();

    let tools = mcp.client.list_all_tools().await.unwrap();
    let test_tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "test_default_only");
    assert!(
        test_tool.is_some(),
        "test_default_only tool should be registered"
    );

    let tool = test_tool.unwrap();
    let schema = tool.input_schema.as_ref();
    let properties = schema.get("properties").unwrap().as_object().unwrap();

    // Should NOT have subcommand property when only default exists
    assert!(
        !properties.contains_key("subcommand"),
        "Should not have subcommand property for default-only tool"
    );

    // Should have the flag option
    assert!(properties.contains_key("flag"), "Should have flag option");
}

/// Test working_directory is added for non-cargo tools
#[tokio::test]
async fn test_working_directory_added_for_non_cargo_tools() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();

    let tool_json = json!({
        "name": "test_wd",
        "description": "Test working directory addition",
        "command": "test",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Default"
        }]
    });

    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir).unwrap();
    std::fs::write(
        tools_dir.join("test_wd.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.unwrap();

    let tools = mcp.client.list_all_tools().await.unwrap();
    let test_tool = tools.iter().find(|t| t.name.as_ref() == "test_wd");
    assert!(test_tool.is_some());

    let tool = test_tool.unwrap();
    let schema = tool.input_schema.as_ref();
    let properties = schema.get("properties").unwrap().as_object().unwrap();

    // Non-cargo tools should have working_directory
    assert!(
        properties.contains_key("working_directory"),
        "Non-cargo tools should have working_directory"
    );
    let wd_prop = properties.get("working_directory").unwrap();
    assert_eq!(wd_prop["format"], "path");
}

/// Test await and status tools are always present
#[tokio::test]
async fn test_hardwired_tools_always_present() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();

    // Create empty tools directory - no custom tools
    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir).unwrap();

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.unwrap();

    let tools = mcp.client.list_all_tools().await.unwrap();

    let await_tool = tools.iter().find(|t| t.name.as_ref() == "await");
    let status_tool = tools.iter().find(|t| t.name.as_ref() == "status");

    assert!(await_tool.is_some(), "await tool should always be present");
    assert!(
        status_tool.is_some(),
        "status tool should always be present"
    );

    // Verify await tool has proper parameters
    let await_schema = await_tool.unwrap().input_schema.as_ref();
    let await_props = await_schema.get("properties").unwrap().as_object().unwrap();
    assert!(await_props.contains_key("id"));
    assert!(await_props.contains_key("tools"));

    // Verify status tool has proper parameters
    let status_schema = status_tool.unwrap().input_schema.as_ref();
    let status_props = status_schema
        .get("properties")
        .unwrap()
        .as_object()
        .unwrap();
    assert!(status_props.contains_key("id"));
    assert!(status_props.contains_key("tools"));
}

/// Test required options are properly marked in schema
#[tokio::test]
async fn test_required_options_in_schema() {
    init_test_logging();
    let temp_dir = tempdir().unwrap();

    let tool_json = json!({
        "name": "test_required",
        "description": "Test required options",
        "command": "test",
        "enabled": true,
        "subcommand": [{
            "name": "default",
            "description": "Default",
            "options": [
                {"name": "required_opt", "type": "string", "description": "Required", "required": true},
                {"name": "optional_opt", "type": "string", "description": "Optional", "required": false},
                {"name": "default_opt", "type": "string", "description": "Default (not required)"}
            ]
        }]
    });

    let tools_dir = temp_dir.path().join(".ahma");
    std::fs::create_dir_all(&tools_dir).unwrap();
    std::fs::write(
        tools_dir.join("test_required.json"),
        serde_json::to_string_pretty(&tool_json).unwrap(),
    )
    .unwrap();

    let mcp = create_in_process_mcp_from_dir(&tools_dir).await.unwrap();

    let tools = mcp.client.list_all_tools().await.unwrap();
    let test_tool = tools.iter().find(|t| t.name.as_ref() == "test_required");
    assert!(test_tool.is_some());

    let tool = test_tool.unwrap();
    let schema = tool.input_schema.as_ref();
    let required = schema
        .get("required")
        .map(|r| r.as_array().unwrap().clone())
        .unwrap_or_default();
    let required_names: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();

    assert!(
        required_names.contains(&"required_opt"),
        "required_opt should be in required array"
    );
    assert!(
        !required_names.contains(&"optional_opt"),
        "optional_opt should not be in required array"
    );
    assert!(
        !required_names.contains(&"default_opt"),
        "default_opt should not be in required array"
    );
}
