//! Handler for `tool_type: livelog` tools.
//!
//! A livelog tool spawns a long-running source command (e.g. `adb logcat`),
//! pipes its output through an LLM for issue detection, and pushes
//! [`ProgressUpdate::LogAlert`] notifications to the MCP client whenever the
//! LLM finds problems matching the `detection_prompt`.

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use serde_json::{Map, Value};
use tracing::info;

use crate::{
    callback_system::CallbackSender,
    config::ToolConfig,
    livelog::run_livelog_pipeline,
    operation_monitor::{Operation, OperationMonitor, OperationStatus},
    sandbox::Sandbox,
};

/// Start a live-log monitoring session and return the operation ID immediately.
///
/// The source process is spawned inside a background `tokio` task.  Log chunks are
/// forwarded to the configured LLM; when issues are detected a
/// [`ProgressUpdate::LogAlert`] notification is pushed to the MCP client.
///
/// # Arguments
///
/// * `op_id`    — Pre-generated operation ID (should match the ID in the callback).
/// * `config`   — Tool configuration (must have `livelog` field populated).
/// * `params`   — MCP call params (used for optional `working_directory` override).
/// * `monitor`  — Operation monitor for lifecycle tracking.
/// * `sandbox`  — Sandbox used to spawn the source process.
/// * `callback` — Optional progress callback; pass `None` if no MCP peer is attached.
pub async fn handle_livelog_start(
    op_id: String,
    config: &ToolConfig,
    params: &Map<String, Value>,
    monitor: Arc<OperationMonitor>,
    sandbox: Arc<Sandbox>,
    callback: Option<Box<dyn CallbackSender>>,
) -> Result<String> {
    let livelog = config.livelog.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "tool '{}' has tool_type=livelog but no 'livelog' config block",
            config.name
        )
    })?;

    let working_dir = params
        .get("working_directory")
        .and_then(Value::as_str)
        .unwrap_or(".");

    // Validate the working directory against the sandbox scope up front so we
    // can return an error to the caller before spawning anything.
    let safe_wd = sandbox
        .validate_path(std::path::Path::new(working_dir))
        .map_err(|e| anyhow::anyhow!("Invalid working directory '{}': {}", working_dir, e))?;

    let timeout = config.timeout_seconds.map(Duration::from_secs);

    let operation = Operation::new_with_timeout(
        op_id.clone(),
        config.name.clone(),
        format!(
            "Livelog: {} {:?} (detection: {})",
            livelog.source_command,
            livelog.source_args,
            &livelog.detection_prompt
                .chars()
                .take(60)
                .collect::<String>()
        ),
        None,
        timeout,
    );
    monitor.add_operation(operation).await;

    info!(
        "livelog_tool: registered operation '{}' for tool '{}'",
        op_id, config.name
    );

    // Clone everything that needs to move into the background task.
    let op_id_task = op_id.clone();
    let livelog_config = livelog.clone();
    let monitor_task = monitor.clone();
    let sandbox_task = sandbox.clone();

    tokio::spawn(async move {
        // Retrieve the cancellation token from the monitor (set when the operation
        // was registered so callers can cancel via `cancel_tool`).
        let cancellation_token = match monitor_task.get_operation(&op_id_task).await {
            Some(op) => op.cancellation_token.clone(),
            None => {
                tracing::error!(
                    "livelog_tool: operation '{}' disappeared from monitor before task started",
                    op_id_task
                );
                return;
            }
        };

        monitor_task
            .update_status(&op_id_task, OperationStatus::InProgress, None)
            .await;

        // The callback is an Option<Box<dyn CallbackSender + Send + Sync>>.
        // We borrow it as Option<&dyn CallbackSender> for the pipeline.
        let cb_ref: Option<&(dyn CallbackSender + Send + Sync)> = callback
            .as_ref()
            .map(|b| b.as_ref() as &(dyn CallbackSender + Send + Sync));

        run_livelog_pipeline(
            &op_id_task,
            &livelog_config,
            &sandbox_task,
            &safe_wd,
            cancellation_token,
            cb_ref,
        )
        .await;

        monitor_task
            .update_status(&op_id_task, OperationStatus::Completed, None)
            .await;

        info!("livelog_tool: operation '{}' completed", op_id_task);
    });

    Ok(op_id)
}
