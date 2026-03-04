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
        "Landlock requires Linux kernel 5.13 or newer. Current: {}.",
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
