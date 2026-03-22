use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};
use ahma_mcp::test_utils::http::{HttpMcpTestClient, spawn_http_bridge};
use anyhow::Context;
use serde_json::json;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

fn short_sleep_command() -> &'static str {
    #[cfg(windows)]
    {
        // `sandboxed_shell` executes through powershell on Windows.
        "Start-Sleep -Milliseconds 200"
    }

    #[cfg(not(windows))]
    {
        "sleep 0.2"
    }
}

#[tokio::test]
async fn test_http_no_progress_token_does_not_emit_progress_notifications() -> anyhow::Result<()> {
    let server = spawn_http_bridge().await?;
    let mut client = HttpMcpTestClient::new(server.base_url());

    // sandboxed_shell is a core built-in tool - no JSON config needed

    // Handshake
    client.initialize().await?;

    // Start SSE + roots/list handshake
    let client_root_dir = TempDir::new().context("Failed to create temp dir (client_root)")?;
    let mut events_rx = client
        .start_sse_events(vec![client_root_dir.path().to_path_buf()])
        .await?;

    // Wait for sandbox to lock (platform-aware retry: Windows CI is 3-5x slower).
    let sandbox_deadline =
        tokio::time::Instant::now() + TestTimeouts::get(TimeoutCategory::SandboxReady);

    // tools/call WITHOUT _meta.progressToken
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "sandboxed_shell",
            "arguments": {
                "command": short_sleep_command(),
                "working_directory": client_root_dir.path().to_string_lossy()
            }
        }
    });
    let tool_resp = loop {
        let (resp, _) = client.send_request(&tool_call).await?;
        let is_sandbox_init = resp
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_i64())
            == Some(-32001);
        if !is_sandbox_init {
            break resp;
        }
        if tokio::time::Instant::now() > sandbox_deadline {
            anyhow::bail!("sandbox did not become ready in time");
        }
        sleep(TestTimeouts::poll_interval()).await;
    };
    assert!(tool_resp.get("error").is_none(), "tools/call must succeed");

    // Assert: no notifications/progress arrive within a short window.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(200), events_rx.recv()).await
        else {
            continue;
        };

        if ev.get("method").and_then(|m| m.as_str()) == Some("notifications/progress") {
            anyhow::bail!("unexpected notifications/progress without client progressToken: {ev}");
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_http_progress_token_is_echoed_in_progress_notifications() -> anyhow::Result<()> {
    let server = spawn_http_bridge().await?;
    let mut client = HttpMcpTestClient::new(server.base_url());

    // sandboxed_shell is a core built-in tool - no JSON config needed

    // Handshake
    client.initialize().await?;

    // Start SSE + roots/list handshake
    let client_root_dir = TempDir::new().context("Failed to create temp dir (client_root)")?;
    let mut events_rx = client
        .start_sse_events(vec![client_root_dir.path().to_path_buf()])
        .await?;

    // Wait for sandbox to lock (platform-aware retry: Windows CI is 3-5x slower).
    let sandbox_deadline =
        tokio::time::Instant::now() + TestTimeouts::get(TimeoutCategory::SandboxReady);

    let token = "tok_http_1";
    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "_meta": { "progressToken": token },
            "name": "sandboxed_shell",
            "arguments": {
                "command": short_sleep_command(),
                "working_directory": client_root_dir.path().to_string_lossy()
            }
        }
    });
    let tool_resp = loop {
        let (resp, _) = client.send_request(&tool_call).await?;
        let is_sandbox_init = resp
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_i64())
            == Some(-32001);
        if !is_sandbox_init {
            break resp;
        }
        if tokio::time::Instant::now() > sandbox_deadline {
            anyhow::bail!("sandbox did not become ready in time");
        }
        sleep(TestTimeouts::poll_interval()).await;
    };
    assert!(tool_resp.get("error").is_none(), "tools/call must succeed");

    // Expect at least one notifications/progress with matching token.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(ev)) =
            tokio::time::timeout(Duration::from_millis(500), events_rx.recv()).await
        {
            if ev.get("method").and_then(|m| m.as_str()) != Some("notifications/progress") {
                continue;
            }
            let got = ev
                .get("params")
                .and_then(|p| p.get("progressToken"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert_eq!(
                got, token,
                "progressToken must be echoed from request _meta"
            );
            return Ok(());
        }
    }

    anyhow::bail!("did not observe notifications/progress with token {token}");
}
