use ahma_mcp::test_utils::in_process::create_in_process_mcp_empty;
use anyhow::Result;

fn assert_valid_tool_schema(schema_json: &serde_json::Value) {
    let obj = schema_json.as_object().expect("schema should be an object");
    assert!(
        obj.contains_key("type")
            || obj.contains_key("properties")
            || obj.contains_key("oneOf")
            || obj.contains_key("anyOf"),
        "Schema missing type information: {schema_json}"
    );
}

#[tokio::test]
async fn test_tool_schema_validation() -> Result<()> {
    let mcp = create_in_process_mcp_empty().await?;
    let tools = mcp.client.list_all_tools().await?;

    for tool in &tools {
        assert!(!tool.name.is_empty());
        let desc = tool
            .description
            .as_ref()
            .expect("tool should have description");
        assert!(!desc.is_empty());

        let schema_json = serde_json::to_value(&*tool.input_schema)?;
        assert_valid_tool_schema(&schema_json);
    }

    Ok(())
}
