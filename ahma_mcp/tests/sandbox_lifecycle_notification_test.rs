use ahma_mcp::test_utils;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

#[test]
fn test_sandbox_lifecycle_notifications() {
    let binary = test_utils::cli::build_binary_cached("ahma_mcp", "ahma-mcp");
    let temp_dir = tempfile::tempdir().unwrap();
    let tools_dir = temp_dir.path().join("tools");
    std::fs::create_dir(&tools_dir).unwrap();

    let mut cmd = Command::new(&binary);
    cmd.arg("--mode")
        .arg("stdio")
        .arg("--sandbox-scope")
        .arg(temp_dir.path())
        .arg("--tools-dir")
        .arg(&tools_dir);

    // When running inside a nested sandbox (mcp_ahma_sandboxed_shell / VS Code / Docker),
    // set AHMA_NO_SANDBOX so the child process can start; app-level path checks still apply.
    #[cfg(target_os = "macos")]
    if ahma_mcp::sandbox::test_sandbox_exec_available().is_err() {
        cmd.env("AHMA_NO_SANDBOX", "1");
    }
    #[cfg(windows)]
    if ahma_mcp::sandbox::check_windows_sandbox_available().is_err() {
        cmd.env("AHMA_NO_SANDBOX", "1");
    }

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn ahma_mcp");

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");

    // Spawn thread to read stdout and capture output
    let handle = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut output_log = String::new();
        let mut seen_terminated = false;

        for line in reader.lines() {
            let line = line.expect("Failed to read line");
            output_log.push_str(&line);
            output_log.push('\n');

            if line.contains("notifications/sandbox/terminated") {
                seen_terminated = true;
            }
        }
        (output_log, seen_terminated)
    });

    // Send initialize request
    let init_req = r#"{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "test", "version": "1.0"} }}"#;
    stdin.write_all(init_req.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();

    // Short sleep to allow server to process initialize
    thread::sleep(Duration::from_millis(200));

    // Send initialized notification to complete handshake
    let initialized_notif = r#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#;
    stdin.write_all(initialized_notif.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();

    // Short sleep to allow server to process initialized
    thread::sleep(Duration::from_millis(200));

    // Close stdin to signal end of session (clean shutdown)
    drop(stdin);

    // Wait for process to exit
    let _ = child.wait().expect("Failed to wait on child");

    // Join reader thread
    let (log, seen) = handle.join().expect("Thread panicked");

    if !seen {
        println!("STDOUT LOG:\n{}", log);
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
