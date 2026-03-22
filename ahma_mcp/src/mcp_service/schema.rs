//! Schema generation for MCP tools.
//!
//! Contains functions for generating JSON schemas from tool configurations.

use serde_json::{Map, Value, json};
use std::sync::Arc;

use crate::config::{CommandOption, SubcommandConfig, ToolConfig};

use super::types::GuidanceConfig;

/// Normalizes option types to JSON Schema types.
pub fn normalize_option_type(option_type: &str) -> &'static str {
    match option_type {
        "bool" | "boolean" => "boolean",
        "int" | "integer" => "integer",
        "array" => "array",
        "number" => "number",
        "string" => "string",
        _ => "string",
    }
}

/// Builds a common string property schema with a description.
pub fn string_property(description: &str) -> Value {
    json!({
        "type": "string",
        "description": description
    })
}

/// Builds a string property schema with `format: path`.
pub fn path_property(description: &str) -> Value {
    json!({
        "type": "string",
        "description": description,
        "format": "path"
    })
}

/// Builds a string enum property schema with a description.
pub fn enum_string_property(description: &str, values: &[&str]) -> Value {
    json!({
        "type": "string",
        "description": description,
        "enum": values
    })
}

/// Builds a string enum property schema with a default value.
pub fn enum_string_property_with_default(
    description: &str,
    values: &[&str],
    default_value: &str,
) -> Value {
    json!({
        "type": "string",
        "description": description,
        "enum": values,
        "default": default_value
    })
}

/// Builds a JSON object input schema for built-in MCP tools.
pub fn object_input_schema(
    properties: Map<String, Value>,
    required: &[&str],
) -> Arc<Map<String, Value>> {
    let required_values = required
        .iter()
        .map(|key| Value::String((*key).to_string()))
        .collect::<Vec<_>>();
    Arc::new(build_schema_object(properties, required_values))
}

fn items_schema_from_spec(spec: &crate::config::ItemsSpec) -> Map<String, Value> {
    let mut schema = Map::new();
    schema.insert("type".into(), Value::String(spec.item_type.clone()));
    schema.extend(
        spec.format
            .as_ref()
            .map(|f| ("format".into(), Value::String(f.clone()))),
    );
    schema.extend(
        spec.description
            .as_ref()
            .map(|d| ("description".into(), Value::String(d.clone()))),
    );
    schema
}

/// Builds the items schema falling back to the parent option's attributes.
fn items_schema_from_option(option: &CommandOption) -> Map<String, Value> {
    let mut schema = Map::new();
    schema.insert("type".into(), Value::String("string".into()));
    if let Some(f) = &option.format {
        schema.insert("format".into(), Value::String(f.clone()));
    }
    schema
}

/// Builds the items schema for array-type options.
pub fn build_items_schema(option: &CommandOption) -> Map<String, Value> {
    option
        .items
        .as_ref()
        .map(items_schema_from_spec)
        .unwrap_or_else(|| items_schema_from_option(option))
}

/// Generates the JSON schema for a subcommand's options.
pub fn get_schema_for_options(sub_config: &SubcommandConfig) -> (Map<String, Value>, Vec<Value>) {
    let mut properties = Map::new();
    let mut required = Vec::new();

    sub_config
        .options
        .iter()
        .flatten()
        .for_each(|opt| process_option(opt, &mut properties, &mut required));
    sub_config
        .positional_args
        .iter()
        .flatten()
        .for_each(|arg| process_positional_arg(arg, &mut properties, &mut required));

    (properties, required)
}

/// Builds a JSON schema map for a single argument or option.
fn build_arg_schema(arg: &CommandOption) -> Map<String, Value> {
    let mut schema = Map::new();
    let param_type = normalize_option_type(&arg.option_type);
    schema.insert("type".to_string(), Value::String(param_type.to_string()));

    schema.extend(
        arg.description
            .as_ref()
            .map(|d| ("description".to_string(), Value::String(d.clone()))),
    );
    schema.extend(
        arg.format
            .as_ref()
            .map(|f| ("format".to_string(), Value::String(f.clone()))),
    );

    if param_type == "array" {
        schema.insert("items".to_string(), Value::Object(build_items_schema(arg)));
    }
    schema
}

fn insert_arg(arg: &CommandOption, properties: &mut Map<String, Value>, required: &mut Vec<Value>) {
    let schema = build_arg_schema(arg);
    properties.insert(arg.name.clone(), Value::Object(schema));
    if arg.required.unwrap_or(false) {
        required.push(Value::String(arg.name.clone()));
    }
}

fn process_option(
    option: &CommandOption,
    properties: &mut Map<String, Value>,
    required: &mut Vec<Value>,
) {
    // Options always emit a description (defaulting to empty string)
    let mut schema = build_arg_schema(option);
    schema
        .entry("description")
        .or_insert_with(|| Value::String(String::new()));
    properties.insert(option.name.clone(), Value::Object(schema));
    if option.required.unwrap_or(false) {
        required.push(Value::String(option.name.clone()));
    }
}

fn process_positional_arg(
    arg: &CommandOption,
    properties: &mut Map<String, Value>,
    required: &mut Vec<Value>,
) {
    insert_arg(arg, properties, required);
}

/// Builds the path for a subcommand given its prefix and name.
fn subcommand_path(prefix: &str, name: &str) -> String {
    match (prefix.is_empty(), name == "default") {
        (true, _) => name.to_string(),
        (_, true) => prefix.to_string(),
        _ => format!("{}_{}", prefix, name),
    }
}

/// Pushes a leaf subcommand or recurses into nested subcommands.
fn push_subcommand_or_recurse<'a>(
    sub: &'a SubcommandConfig,
    current_path: String,
    leaves: &mut Vec<(String, &'a SubcommandConfig)>,
) {
    match &sub.subcommand {
        Some(nested_subcommands) => {
            collect_leaf_subcommands(nested_subcommands, &current_path, leaves)
        }
        None => leaves.push((current_path, sub)),
    }
}

/// Recursively collects all leaf subcommands from a tool's configuration.
pub fn collect_leaf_subcommands<'a>(
    subcommands: &'a [SubcommandConfig],
    prefix: &str,
    leaves: &mut Vec<(String, &'a SubcommandConfig)>,
) {
    subcommands.iter().filter(|s| s.enabled).for_each(|sub| {
        push_subcommand_or_recurse(sub, subcommand_path(prefix, &sub.name), leaves);
    });
}

/// Generates the JSON schema for a tool configuration file.
pub fn generate_schema_for_tool_config(
    tool_config: &ToolConfig,
    guidance: &Option<GuidanceConfig>,
) -> Arc<Map<String, Value>> {
    let mut leaf_subcommands = Vec::new();
    if let Some(subcommands) = &tool_config.subcommand {
        collect_leaf_subcommands(subcommands, "", &mut leaf_subcommands);
    }

    // Suppress unused guidance warning - guidance is used for tool descriptions, not schemas
    let _ = guidance;

    match leaf_subcommands.as_slice() {
        [(name, cfg)] if name == "default" => Arc::new(generate_single_command_schema(
            tool_config,
            &(name.clone(), *cfg),
        )),
        _ => Arc::new(generate_multi_command_schema(
            tool_config,
            &leaf_subcommands,
        )),
    }
}

fn add_working_directory_property(properties: &mut Map<String, Value>) {
    properties.insert(
        "working_directory".to_string(),
        serde_json::json!({
            "type": "string",
            "description": "Working directory for command execution",
            "format": "path"
        }),
    );
}

fn build_schema_object(properties: Map<String, Value>, required: Vec<Value>) -> Map<String, Value> {
    let mut schema = Map::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".to_string(), Value::Array(required));
    }
    schema
}

fn generate_single_command_schema(
    tool_config: &ToolConfig,
    leaf_subcommand: &(String, &SubcommandConfig),
) -> Map<String, Value> {
    let (_, sub_config) = leaf_subcommand;
    let (mut properties, required) = get_schema_for_options(sub_config);

    if tool_config.name != "cargo" {
        add_working_directory_property(&mut properties);
    }

    build_schema_object(properties, required)
}

/// Public wrapper for generating a single-subcommand schema (used by tool flattening).
pub fn generate_single_command_schema_pub(
    tool_config: &ToolConfig,
    leaf_subcommand: &(String, &SubcommandConfig),
) -> Map<String, Value> {
    generate_single_command_schema(tool_config, leaf_subcommand)
}

/// Processes a single subcommand entry for multi-command schema generation.
///
/// Returns the enum value and an optional `oneOf` entry (with if/then clause).
fn build_subcommand_entry(
    path: &str,
    sub_config: &SubcommandConfig,
    all_properties: &mut Map<String, Value>,
) -> (Value, Value) {
    let (sub_properties, sub_required) = get_schema_for_options(sub_config);
    all_properties.extend(sub_properties);

    let if_clause = serde_json::json!({
        "properties": { "subcommand": { "const": path } }
    });

    let mut one_of_entry = Map::new();
    one_of_entry.insert("if".to_string(), if_clause);
    if !sub_required.is_empty() {
        one_of_entry.insert(
            "then".to_string(),
            serde_json::json!({ "required": sub_required }),
        );
    }

    (Value::String(path.to_string()), Value::Object(one_of_entry))
}

fn generate_multi_command_schema(
    tool_config: &ToolConfig,
    leaf_subcommands: &[(String, &SubcommandConfig)],
) -> Map<String, Value> {
    let mut all_properties = Map::new();
    let (subcommand_enum, one_of): (Vec<_>, Vec<_>) = leaf_subcommands
        .iter()
        .map(|(path, sub_config)| build_subcommand_entry(path, sub_config, &mut all_properties))
        .unzip();

    let has_subcommands = !subcommand_enum.is_empty();
    if has_subcommands {
        all_properties.insert(
            "subcommand".to_string(),
            serde_json::json!({ "type": "string", "description": "The subcommand to execute.", "enum": subcommand_enum }),
        );
    }

    if tool_config.name != "cargo" {
        add_working_directory_property(&mut all_properties);
    }

    let required = if has_subcommands {
        vec![Value::String("subcommand".to_string())]
    } else {
        vec![]
    };
    let mut schema = build_schema_object(all_properties, required);
    if !one_of.is_empty() {
        schema.insert("oneOf".to_string(), Value::Array(one_of));
    }
    schema
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CommandOption, ItemsSpec, SubcommandConfig};

    // ============= normalize_option_type tests =============

    #[test]
    fn test_normalize_option_type_bool_variants() {
        assert_eq!(normalize_option_type("bool"), "boolean");
        assert_eq!(normalize_option_type("boolean"), "boolean");
    }

    #[test]
    fn test_normalize_option_type_int_variants() {
        assert_eq!(normalize_option_type("int"), "integer");
        assert_eq!(normalize_option_type("integer"), "integer");
    }

    #[test]
    fn test_normalize_option_type_passthrough() {
        assert_eq!(normalize_option_type("array"), "array");
        assert_eq!(normalize_option_type("number"), "number");
        assert_eq!(normalize_option_type("string"), "string");
    }

    #[test]
    fn test_normalize_option_type_unknown_defaults_to_string() {
        assert_eq!(normalize_option_type("unknown"), "string");
        assert_eq!(normalize_option_type("foo"), "string");
        assert_eq!(normalize_option_type(""), "string");
    }

    // ============= build_items_schema tests =============

    #[test]
    fn test_build_items_schema_without_items_spec() {
        let option = CommandOption {
            name: "files".to_string(),
            option_type: "array".to_string(),
            description: Some("List of files".to_string()),
            required: None,
            format: None,
            items: None,
            file_arg: None,
            file_flag: None,
            alias: None,
        };

        let schema = build_items_schema(&option);
        assert_eq!(schema.get("type").unwrap(), "string");
        assert!(schema.get("format").is_none());
    }

    #[test]
    fn test_build_items_schema_with_format_on_option() {
        let option = CommandOption {
            name: "files".to_string(),
            option_type: "array".to_string(),
            description: Some("List of files".to_string()),
            required: None,
            format: Some("path".to_string()),
            items: None,
            file_arg: None,
            file_flag: None,
            alias: None,
        };

        let schema = build_items_schema(&option);
        assert_eq!(schema.get("type").unwrap(), "string");
        assert_eq!(schema.get("format").unwrap(), "path");
    }

    #[test]
    fn test_build_items_schema_with_full_items_spec() {
        let option = CommandOption {
            name: "files".to_string(),
            option_type: "array".to_string(),
            description: Some("List of files".to_string()),
            required: None,
            format: None,
            items: Some(ItemsSpec {
                item_type: "string".to_string(),
                format: Some("path".to_string()),
                description: Some("A file path".to_string()),
            }),
            file_arg: None,
            file_flag: None,
            alias: None,
        };

        let schema = build_items_schema(&option);
        assert_eq!(schema.get("type").unwrap(), "string");
        assert_eq!(schema.get("format").unwrap(), "path");
        assert_eq!(schema.get("description").unwrap(), "A file path");
    }

    #[test]
    fn test_build_items_schema_items_type_override() {
        // Items spec should override default string type
        let option = CommandOption {
            name: "ids".to_string(),
            option_type: "array".to_string(),
            description: Some("List of IDs".to_string()),
            required: None,
            format: None,
            items: Some(ItemsSpec {
                item_type: "integer".to_string(),
                format: None,
                description: None,
            }),
            file_arg: None,
            file_flag: None,
            alias: None,
        };

        let schema = build_items_schema(&option);
        assert_eq!(schema.get("type").unwrap(), "integer");
    }

    // ============= collect_leaf_subcommands tests =============

    fn make_subcommand(name: &str, description: &str, enabled: bool) -> SubcommandConfig {
        SubcommandConfig {
            name: name.to_string(),
            description: description.to_string(),
            subcommand: None,
            options: None,
            positional_args: None,
            positional_args_first: None,
            timeout_seconds: None,
            synchronous: None,
            enabled,
            guidance_key: None,
            sequence: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
        }
    }

    fn make_subcommand_with_nested(
        name: &str,
        description: &str,
        enabled: bool,
        nested: Vec<SubcommandConfig>,
    ) -> SubcommandConfig {
        SubcommandConfig {
            name: name.to_string(),
            description: description.to_string(),
            subcommand: Some(nested),
            options: None,
            positional_args: None,
            positional_args_first: None,
            timeout_seconds: None,
            synchronous: None,
            enabled,
            guidance_key: None,
            sequence: None,
            step_delay_ms: None,
            availability_check: None,
            install_instructions: None,
        }
    }

    #[test]
    fn test_collect_leaf_subcommands_single_default() {
        let subcommands = vec![make_subcommand("default", "Default subcommand", true)];

        let mut leaves = Vec::new();
        collect_leaf_subcommands(&subcommands, "", &mut leaves);

        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].0, "default");
        assert_eq!(leaves[0].1.name, "default");
    }

    #[test]
    fn test_collect_leaf_subcommands_multiple_at_same_level() {
        let subcommands = vec![
            make_subcommand("build", "Build", true),
            make_subcommand("test", "Test", true),
        ];

        let mut leaves = Vec::new();
        collect_leaf_subcommands(&subcommands, "", &mut leaves);

        assert_eq!(leaves.len(), 2);
        let names: Vec<&str> = leaves.iter().map(|(path, _)| path.as_str()).collect();
        assert!(names.contains(&"build"));
        assert!(names.contains(&"test"));
    }

    #[test]
    fn test_collect_leaf_subcommands_nested() {
        let nested = vec![
            make_subcommand("child1", "Child 1", true),
            make_subcommand("child2", "Child 2", true),
        ];

        let subcommands = vec![make_subcommand_with_nested(
            "parent", "Parent", true, nested,
        )];

        let mut leaves = Vec::new();
        collect_leaf_subcommands(&subcommands, "", &mut leaves);

        assert_eq!(leaves.len(), 2);
        let paths: Vec<&str> = leaves.iter().map(|(path, _)| path.as_str()).collect();
        assert!(paths.contains(&"parent_child1"));
        assert!(paths.contains(&"parent_child2"));
    }

    #[test]
    fn test_collect_leaf_subcommands_skips_disabled() {
        let subcommands = vec![
            make_subcommand("enabled", "Enabled", true),
            make_subcommand("disabled", "Disabled", false),
        ];

        let mut leaves = Vec::new();
        collect_leaf_subcommands(&subcommands, "", &mut leaves);

        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].0, "enabled");
    }

    #[test]
    fn test_collect_leaf_subcommands_deeply_nested() {
        let level3 = vec![make_subcommand("leaf", "Leaf", true)];
        let level2 = vec![make_subcommand_with_nested("mid", "Mid", true, level3)];
        let level1 = vec![make_subcommand_with_nested("top", "Top", true, level2)];

        let mut leaves = Vec::new();
        collect_leaf_subcommands(&level1, "", &mut leaves);

        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].0, "top_mid_leaf");
    }

    #[test]
    fn test_collect_leaf_subcommands_with_prefix() {
        let subcommands = vec![make_subcommand("child", "Child", true)];

        let mut leaves = Vec::new();
        collect_leaf_subcommands(&subcommands, "parent", &mut leaves);

        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].0, "parent_child");
    }

    #[test]
    fn test_collect_leaf_subcommands_default_uses_prefix() {
        // When subcommand name is "default", it should use just the prefix
        let subcommands = vec![make_subcommand("default", "Default child", true)];

        let mut leaves = Vec::new();
        collect_leaf_subcommands(&subcommands, "parent", &mut leaves);

        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].0, "parent");
    }
}
