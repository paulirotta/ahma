use super::error::SandboxError;

/// Check if the platform's sandboxing prerequisites are met.
pub fn check_sandbox_prerequisites() -> Result<(), SandboxError> {
    #[cfg(target_os = "linux")]
    {
        check_landlock_available()
    }

    #[cfg(target_os = "macos")]
    {
        check_macos_sandbox_available()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        #[cfg(target_os = "windows")]
        {
            super::windows::check_windows_sandbox_available()
        }
        #[cfg(not(target_os = "windows"))]
        {
            Err(SandboxError::UnsupportedOs(
                std::env::consts::OS.to_string(),
            ))
        }
    }
}

#[cfg(target_os = "linux")]
fn check_landlock_available() -> Result<(), SandboxError> {
    use std::fs;
    let landlock_abi_path = "/sys/kernel/security/lsm";
    match fs::read_to_string(landlock_abi_path) {
        Ok(content) => {
            if content.contains("landlock") {
                Ok(())
            } else {
                Err(SandboxError::LandlockNotAvailable)
            }
        }
        Err(_) => check_kernel_version_for_landlock(),
    }
}

#[cfg(target_os = "linux")]
fn check_kernel_version_for_landlock() -> Result<(), SandboxError> {
    use std::process::Command;
    let output = Command::new("uname").arg("-r").output().map_err(|_| {
        SandboxError::PrerequisiteFailed("Failed to check kernel version".to_string())
    })?;
    let version_str = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = version_str.trim().split('.').collect();
    if parts.len() >= 2 {
        let major: u32 = parts[0].parse().unwrap_or(0);
        let minor: u32 = parts[1]
            .split('-')
            .next()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0);
        if major > 5 || (major == 5 && minor >= 13) {
            return Ok(());
        }
    }
    Err(SandboxError::PrerequisiteFailed(format!(
        "Landlock requires Linux kernel 5.13 or newer. Current: {}. \
         To run without sandboxing, add the --disable-sandbox parameter to your mcp.json tool definition. \
         Example: \"args\": [\"--mode\", \"http\", \"--disable-sandbox\"]",
        version_str.trim()
    )))
}

#[cfg(target_os = "macos")]
fn check_macos_sandbox_available() -> Result<(), SandboxError> {
    use std::process::Command;
    let result = Command::new("which").arg("sandbox-exec").output();
    match result {
        Ok(output) if output.status.success() => Ok(()),
        _ => Err(SandboxError::MacOSSandboxNotAvailable),
    }
}

#[cfg(target_os = "macos")]
pub fn test_sandbox_exec_available() -> Result<(), SandboxError> {
    use std::process::Command;
    let test_profile = "(version 1)(allow default)";
    let result = Command::new("sandbox-exec")
        .args(["-p", test_profile, "/usr/bin/true"])
        .output();
    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Operation not permitted")
                || stderr.contains("sandbox_apply")
                || output.status.code() == Some(71)
            {
                Err(SandboxError::NestedSandboxDetected)
            } else {
                tracing::debug!("sandbox-exec test failed: {}", stderr);
                Err(SandboxError::NestedSandboxDetected)
            }
        }
        Err(e) => {
            tracing::debug!("sandbox-exec exec failed: {}", e);
            Err(SandboxError::MacOSSandboxNotAvailable)
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn test_sandbox_exec_available() -> Result<(), SandboxError> {
    Ok(())
}

pub fn exit_with_sandbox_error(error: &SandboxError) -> ! {
    eprintln!("\n\u{274c} SECURITY ERROR: Cannot start MCP server\n");
    eprintln!("Reason: {}\n", error);
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// check_sandbox_prerequisites should return Ok or a specific SandboxError
    /// depending on the current platform and environment. We just verify it
    /// doesn't panic and returns a recognizable result.
    #[test]
    fn test_check_sandbox_prerequisites_returns_result() {
        let result = check_sandbox_prerequisites();
        // It should return either Ok(()) or a typed SandboxError—never panic.
        match result {
            Ok(()) => {}
            Err(e) => {
                // The error must be a valid SandboxError variant.
                let msg = e.to_string();
                assert!(!msg.is_empty(), "SandboxError message must be non-empty");
            }
        }
    }

    /// test_sandbox_exec_available on non-macOS returns Ok(()) unconditionally.
    /// On macOS it calls sandbox-exec and returns Ok or a SandboxError.
    #[test]
    fn test_test_sandbox_exec_available_returns_result() {
        let result = test_sandbox_exec_available();
        match result {
            Ok(()) => {}
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    !msg.is_empty(),
                    "Error from test_sandbox_exec_available must be non-empty"
                );
            }
        }
    }

    /// Verify that `SandboxError::LandlockNotAvailable` message is actionable.
    #[test]
    fn test_landlock_error_message_actionable() {
        let err = SandboxError::LandlockNotAvailable;
        let msg = err.to_string();
        assert!(
            msg.contains("--disable-sandbox"),
            "Error should advise --disable-sandbox: {msg}"
        );
    }

    /// Verify that `SandboxError::MacOSSandboxNotAvailable` message is actionable.
    #[test]
    fn test_macos_sandbox_error_message_actionable() {
        let err = SandboxError::MacOSSandboxNotAvailable;
        let msg = err.to_string();
        assert!(
            msg.contains("--disable-sandbox"),
            "Error should advise --disable-sandbox: {msg}"
        );
    }

    /// Verify that `SandboxError::UnsupportedOs` includes the OS name.
    #[test]
    fn test_unsupported_os_error_includes_name() {
        let err = SandboxError::UnsupportedOs("plan9".to_string());
        let msg = err.to_string();
        assert!(msg.contains("plan9"), "Error should name the OS: {msg}");
    }
}
