use ahma_common::timeouts::TestTimeouts;
use ahma_mcp::test_utils::client::ClientBuilder;
use ahma_mcp::test_utils::concurrency::wait_for_condition;
use anyhow::Result;
use std::fs;
use std::time::Duration;
use tempfile::tempdir;

#[tokio::test]
async fn test_config_reload_stays_off_by_default() -> Result<()> {
    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().to_path_buf();

    fs::write(
        tools_dir.join("initial_tool.json"),
        tool_json("initial_tool", "Initial tool"),
    )?;

    let client = ClientBuilder::new().tools_dir(&tools_dir).build().await?;

    let tools = client.list_tools(None).await?;
    assert!(tools.tools.iter().any(|t| t.name == "initial_tool"));
    assert!(!tools.tools.iter().any(|t| t.name == "new_tool"));

    fs::write(
        tools_dir.join("new_tool.json"),
        tool_json("new_tool", "New tool added dynamically"),
    )?;
    fs::write(
        tools_dir.join("initial_tool.json"),
        tool_json("initial_tool", "Modified initial tool"),
    )?;

    tokio::time::sleep(TestTimeouts::scale_millis(750)).await;

    let tools = client.list_tools(None).await?;
    assert!(
        !tools.tools.iter().any(|t| t.name == "new_tool"),
        "New tool should not appear without --hot-reload-tools"
    );
    assert_eq!(
        tools
            .tools
            .iter()
            .find(|t| t.name == "initial_tool")
            .and_then(|t| t.description.clone())
            .as_deref(),
        Some("Initial tool"),
        "Initial tool description should stay unchanged without --hot-reload-tools"
    );

    Ok(())
}

#[tokio::test]
async fn test_config_reload_when_hot_reload_enabled() -> Result<()> {
    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().to_path_buf();

    fs::write(
        tools_dir.join("initial_tool.json"),
        tool_json("initial_tool", "Initial tool"),
    )?;

    let client = ClientBuilder::new()
        .tools_dir(&tools_dir)
        .arg("--hot-reload-tools")
        .build()
        .await?;

    let tools = client.list_tools(None).await?;
    assert!(tools.tools.iter().any(|t| t.name == "initial_tool"));
    assert!(!tools.tools.iter().any(|t| t.name == "new_tool"));

    fs::write(
        tools_dir.join("new_tool.json"),
        tool_json("new_tool", "New tool added dynamically"),
    )?;
    let new_tool_seen = wait_for_condition(reload_timeout(), TestTimeouts::poll_interval(), || {
        let client = &client;
        async move {
            client
                .list_tools(None)
                .await
                .ok()
                .map(|tools| tools.tools.iter().any(|t| t.name == "new_tool"))
                .unwrap_or(false)
        }
    })
    .await;

    assert!(new_tool_seen, "New tool should be present after reload");

    fs::write(
        tools_dir.join("initial_tool.json"),
        tool_json("initial_tool", "Modified initial tool"),
    )?;
    let modified_seen = wait_for_condition(reload_timeout(), TestTimeouts::poll_interval(), || {
        let client = &client;
        async move {
            client
                .list_tools(None)
                .await
                .ok()
                .and_then(|tools| {
                    tools
                        .tools
                        .iter()
                        .find(|t| t.name == "initial_tool")
                        .map(|t| t.description == Some("Modified initial tool".into()))
                })
                .unwrap_or(false)
        }
    })
    .await;

    assert!(
        modified_seen,
        "Modified initial tool should be present after reload"
    );

    fs::remove_file(tools_dir.join("new_tool.json"))?;
    let removed_seen = wait_for_condition(reload_timeout(), TestTimeouts::poll_interval(), || {
        let client = &client;
        async move {
            client
                .list_tools(None)
                .await
                .ok()
                .map(|tools| !tools.tools.iter().any(|t| t.name == "new_tool"))
                .unwrap_or(false)
        }
    })
    .await;

    assert!(
        removed_seen,
        "New tool should be removed after file deletion"
    );

    Ok(())
}

fn tool_json(name: &str, description: &str) -> String {
    format!(
        r#"{{
    "name": "{name}",
    "description": "{description}",
    "command": "echo",
    "timeout_seconds": 10,
    "synchronous": true,
    "enabled": true,
    "subcommand": [
        {{
            "name": "default",
            "description": "Default subcommand"
        }}
    ]
}}
"#
    )
}

fn reload_timeout() -> Duration {
    TestTimeouts::scale_secs(30)
}
