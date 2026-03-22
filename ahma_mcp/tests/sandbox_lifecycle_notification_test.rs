use ahma_common::timeouts::TestTimeouts;
use ahma_mcp::test_utils;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;

/// RAII guard that kills and reaps the child process on drop.
/// Prevents leaking zombie processes when assertions fail mid-test.
struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn test_sandbox_lifecycle_notifications() {
    let binary = test_utils::cli::build_binary_cached("ahma_mcp", "ahma-mcp");
    let temp_dir = tempfile::tempdir().unwrap();
    let tools_dir = temp_dir.path().join("tools");
    std::fs::create_dir(&tools_dir).unwrap();

    // AHMA_DISABLE_SANDBOX=1: this test verifies lifecycle notification emission,
    // not sandbox enforcement. Disabling avoids Landlock/seatbelt interactions and
    // makes the test identical across all CI platforms.
    // AHMA_SKIP_PROBES=1: no tools need availability probing; skip the startup delay.
    let child = Command::new(&binary)
        .args(["serve", "stdio"])
        .env("AHMA_SANDBOX_SCOPE", temp_dir.path())
        .env("AHMA_TOOLS_DIR", &tools_dir)
        .env("AHMA_DISABLE_SANDBOX", "1")
        .env("AHMA_SKIP_PROBES", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn ahma_mcp");

    // Wrap in RAII guard so the child is always killed if the test panics.
    let mut guard = ChildGuard(Some(child));
    let child_ref = guard.0.as_mut().unwrap();

    let mut stdin = child_ref.stdin.take().expect("Failed to open stdin");
    let stdout = child_ref.stdout.take().expect("Failed to open stdout");
    let stderr = child_ref.stderr.take().expect("Failed to open stderr");

    let stderr_handle = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let mut err_log = String::new();
        for line in reader.lines().map_while(Result::ok) {
            err_log.push_str(&line);
            err_log.push('\n');
        }
        err_log
    });

    // Channel: reader thread signals main thread once the initialize response arrives.
    let (init_ok_tx, init_ok_rx) = mpsc::channel::<()>();
    let (tools_ok_tx, tools_ok_rx) = mpsc::channel::<()>();

    let handle = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut output_log = String::new();
        let mut seen_terminated = false;
        let mut init_acked = false;
        let mut tools_acked = false;

        for line in reader.lines() {
            let line = line.expect("Failed to read line");
            output_log.push_str(&line);
            output_log.push('\n');

            // Detect the initialize response: a JSON object with id=1 and a result field.
            if !init_acked && line.contains("\"id\":1") && line.contains("\"result\"") {
                let _ = init_ok_tx.send(());
                init_acked = true;
            }

            // Detect tools/list response: a JSON object with id=2 and a result field.
            if !tools_acked && line.contains("\"id\":2") && line.contains("\"result\"") {
                let _ = tools_ok_tx.send(());
                tools_acked = true;
            }

            if line.contains("notifications/sandbox/terminated") {
                seen_terminated = true;
            }
        }
        (output_log, seen_terminated)
    });

    // Send initialize request.
    let init_req = r#"{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "test", "version": "1.0"} }}"#;
    stdin.write_all(init_req.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();

    // Wait for the server's initialize response before proceeding — deterministic
    // under any CI load instead of a blind fixed-duration sleep.
    init_ok_rx
        .recv_timeout(TestTimeouts::get(
            ahma_common::timeouts::TimeoutCategory::Handshake,
        ))
        .expect("Timed out waiting for initialize response from server");

    // Send initialized notification to complete the MCP handshake.
    let initialized_notif = r#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#;
    stdin.write_all(initialized_notif.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();

    // Send a tools/list request to guarantee the server has processed notifications/initialized.
    // This acts as a synchronization barrier instead of a blind sleep.
    let tools_req = r#"{"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}"#;
    stdin.write_all(tools_req.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();

    // Wait for the tools/list response.
    tools_ok_rx
        .recv_timeout(TestTimeouts::get(
            ahma_common::timeouts::TimeoutCategory::ToolCall,
        ))
        .expect("Timed out waiting for tools/list response from server");

    // Close stdin to signal end of session (clean shutdown).
    drop(stdin);

    // Extract child from guard (guard.drop becomes a no-op) and wait for exit.
    let mut child = guard.0.take().unwrap();
    let _ = child.wait().expect("Failed to wait on child");

    // Join reader thread and collect output.
    let (log, seen) = handle.join().expect("Thread panicked");
    let err_log = stderr_handle.join().unwrap_or_default();

    if !seen {
        println!("STDOUT LOG:\n{}", log);
        println!("STDERR LOG:\n{}", err_log);
    }

    assert!(
        seen,
        "Did not see sandbox/terminated notification in output"
    );
    assert!(
        log.contains("session_ended"),
        "Notification reason mismatch in log: {}",
        log
    );
}
