//! Live log monitoring pipeline.
//!
//! The livelog pipeline spawns an external command (e.g. `adb logcat`, `ssh … tail -f …`),
//! reads its output line-by-line, accumulates lines into time/size-bounded chunks, and
//! periodically sends each chunk to an OpenAI-compatible LLM for issue detection.
//!
//! When the LLM reports an issue, a [`ProgressUpdate::LogAlert`] notification is pushed
//! to the MCP client via the registered callback.  A cooldown window prevents alert
//! storms when many problematic lines arrive in rapid succession.
//!
//! The pipeline runs inside a `tokio::spawn` task and can be stopped at any time by calling
//! [`tokio_util::sync::CancellationToken::cancel`] on the token that was obtained from the
//! [`crate::operation_monitor::OperationMonitor`] for the corresponding operation.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use ahma_llm_monitor::LlmClient;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::{
    callback_system::{CallbackSender, ProgressUpdate},
    config::LivelogConfig,
    sandbox::Sandbox,
};

/// Run the live-log pipeline until cancelled or the source process exits.
///
/// # Arguments
///
/// * `op_id`              — MCP operation ID used in progress notifications.
/// * `config`             — Livelog tool configuration (source command, LLM settings, etc.)
/// * `sandbox`            — Validated sandbox used to spawn the source process.
/// * `working_dir`        — Working directory for the source process.
/// * `cancellation_token` — Token to stop the pipeline on demand.
/// * `callback`           — Optional MCP progress callback for pushing alerts.
pub async fn run_livelog_pipeline(
    op_id: &str,
    config: &LivelogConfig,
    sandbox: &Arc<Sandbox>,
    working_dir: &std::path::Path,
    cancellation_token: CancellationToken,
    callback: Option<&(dyn CallbackSender + Send + Sync)>,
) {
    let llm = LlmClient::new(
        &config.llm_provider.base_url,
        &config.llm_provider.model,
        config.llm_provider.api_key.clone(),
    );

    let cmd_result = sandbox.create_command(&config.source_command, &config.source_args, working_dir);

    let mut child = match cmd_result {
        Ok(mut cmd) => {
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());
            match cmd.spawn() {
                Ok(child) => child,
                Err(e) => {
                    warn!("livelog[{}]: failed to spawn source process: {}", op_id, e);
                    return;
                }
            }
        }
        Err(e) => {
            warn!("livelog[{}]: failed to create command: {}", op_id, e);
            return;
        }
    };

    info!(
        "livelog[{}]: source process started ({} {:?})",
        op_id, config.source_command, config.source_args
    );

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");

    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut stderr_lines = BufReader::new(stderr).lines();

    let chunk_max_lines = config.chunk_max_lines;
    let chunk_max_duration = Duration::from_secs(config.chunk_max_seconds);
    let ctx = AnalysisCtx {
        llm: &llm,
        detection_prompt: &config.detection_prompt,
        llm_timeout: Duration::from_secs(config.llm_timeout_seconds),
        cooldown: Duration::from_secs(config.cooldown_seconds),
    };

    let mut chunk: Vec<String> = Vec::new();
    let mut chunk_start = Instant::now();
    let mut last_alert: Option<Instant> = None;
    let mut stdout_closed = false;
    let mut stderr_closed = false;

    loop {
        if stdout_closed && stderr_closed {
            info!("livelog[{}]: both streams closed, draining final chunk", op_id);
            if !chunk.is_empty() {
                maybe_analyze(op_id, &ctx, &mut chunk, &mut last_alert, callback).await;
            }
            break;
        }

        let time_remaining = chunk_max_duration.saturating_sub(chunk_start.elapsed());

        tokio::select! {
            biased;

            // Cancellation has priority over everything else.
            _ = cancellation_token.cancelled() => {
                info!("livelog[{}]: cancelled, killing source process", op_id);
                let _ = child.kill().await;
                break;
            }

            // Read a stderr line (biased first so error output is processed promptly).
            result = stderr_lines.next_line(), if !stderr_closed => {
                match result {
                    Ok(Some(line)) => {
                        debug!("livelog[{}] stderr: {}", op_id, line);
                        chunk.push(line);
                    }
                    Ok(None) => {
                        debug!("livelog[{}]: stderr closed", op_id);
                        stderr_closed = true;
                    }
                    Err(e) => {
                        warn!("livelog[{}]: stderr read error: {}", op_id, e);
                        stderr_closed = true;
                    }
                }
            }

            // Read a stdout line.
            result = stdout_lines.next_line(), if !stdout_closed => {
                match result {
                    Ok(Some(line)) => {
                        debug!("livelog[{}] stdout: {}", op_id, line);
                        chunk.push(line);
                    }
                    Ok(None) => {
                        debug!("livelog[{}]: stdout closed", op_id);
                        stdout_closed = true;
                    }
                    Err(e) => {
                        warn!("livelog[{}]: stdout read error: {}", op_id, e);
                        stdout_closed = true;
                    }
                }
            }

            // Time-window expiry — flush whatever we have even if the chunk is not full yet.
            _ = tokio::time::sleep(time_remaining) => {
                debug!("livelog[{}]: chunk time window expired ({} lines)", op_id, chunk.len());
            }
        }

        // Flush the chunk when it hits the size limit or the time window.
        let chunk_full = chunk.len() >= chunk_max_lines;
        let chunk_timed_out = chunk_start.elapsed() >= chunk_max_duration;

        if (chunk_full || chunk_timed_out) && !chunk.is_empty() {
            maybe_analyze(op_id, &ctx, &mut chunk, &mut last_alert, callback).await;
            chunk_start = Instant::now();
        }
    }

    let _ = child.wait().await;
    info!("livelog[{}]: pipeline finished", op_id);
}

/// Immutable per-pipeline configuration threaded into [`maybe_analyze`].
struct AnalysisCtx<'a> {
    llm: &'a LlmClient,
    detection_prompt: &'a str,
    llm_timeout: Duration,
    cooldown: Duration,
}

/// Send `chunk` to the LLM for analysis; fire a `LogAlert` if issues are found.
///
/// Respects the cooldown window — if an alert was sent recently the chunk is
/// discarded without calling the LLM.
async fn maybe_analyze(
    op_id: &str,
    ctx: &AnalysisCtx<'_>,
    chunk: &mut Vec<String>,
    last_alert: &mut Option<Instant>,
    callback: Option<&(dyn CallbackSender + Send + Sync)>,
) {
    let (llm, detection_prompt, llm_timeout, cooldown) =
        (ctx.llm, ctx.detection_prompt, ctx.llm_timeout, ctx.cooldown);
    // Enforce cooldown before hitting the LLM.
    if let Some(last) = last_alert
        && last.elapsed() < cooldown {
            debug!(
                "livelog[{}]: cooldown active ({:.1}s remaining), skipping LLM check",
                op_id,
                (cooldown - last.elapsed()).as_secs_f32()
            );
            chunk.clear();
            return;
        }

    let chunk_text = chunk.join("\n");
    let trigger_lines: Vec<String> = std::mem::take(chunk); // ownership + clears in one step

    match llm
        .detect_issues(detection_prompt, &chunk_text, llm_timeout)
        .await
    {
        Ok(Some(summary)) => {
            info!("livelog[{}]: LLM detected issue: {}", op_id, summary);
            *last_alert = Some(Instant::now());

            if let Some(cb) = callback {
                let alert = ProgressUpdate::LogAlert {
                    id: op_id.to_string(),
                    trigger_level: "error".to_string(),
                    context_snapshot: chunk_text,
                    llm_summary: Some(summary),
                    trigger_lines: Some(trigger_lines),
                };
                if let Err(e) = cb.send_progress(alert).await {
                    warn!("livelog[{}]: failed to send alert: {:?}", op_id, e);
                }
            }
        }
        Ok(None) => {
            debug!("livelog[{}]: LLM response: clean", op_id);
        }
        Err(e) => {
            warn!("livelog[{}]: LLM error: {}", op_id, e);
        }
    }
}
