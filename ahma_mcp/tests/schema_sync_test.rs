use ahma_mcp::config::ToolConfig;
use schemars::schema_for;
use std::fs;
use std::path::Path;

#[test]
fn test_schema_is_up_to_date() {
    // Generate the schema from the current code
    let current_schema = schema_for!(ToolConfig);
    let current_json = serde_json::to_string_pretty(&current_schema).unwrap();

    // Find the schema file in the workspace
    // We try a few locations depending on where the test is run
    let paths = [
        "../docs/mtdf-schema.json",
        "docs/mtdf-schema.json",
        "../../docs/mtdf-schema.json",
    ];

    let mut found_path = None;
    for path in paths {
        if Path::new(path).exists() {
            found_path = Some(path);
            break;
        }
    }

    let schema_path = found_path.expect("Could not find docs/mtdf-schema.json in workspace");
    let saved_json = fs::read_to_string(schema_path).expect("Failed to read docs/mtdf-schema.json");

    // Normalize line endings (CRLF -> LF) for cross-platform comparison
    let current_normalized = current_json.trim().replace("\r\n", "\n");
    let saved_normalized = saved_json.trim().replace("\r\n", "\n");

    if current_normalized != saved_normalized {
        panic!(
            "The MTDF schema in {} is out of date with the code in ahma_mcp/src/config.rs.\n\n\
             TO FIX: Run the following command from the workspace root:\n\n\
             cargo run -p generate_tool_schema\n",
            schema_path
        );
    }
}
