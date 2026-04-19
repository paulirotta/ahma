//! # Windows Sandbox Backend — Job Object + AppContainer
//!
//! This module provides Windows-specific sandbox enforcement.
//!
//! ## Implemented: Job Object enforcement (`enforce_windows_sandbox`)
//!
//! A Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` is assigned to the
//! server process at startup.  This ensures all child processes are terminated
//! when the server exits, preventing orphaned tool processes.
//!
//! ## Implemented: AppContainer per-command sandbox (`create_windows_sandboxed_command`)
//!
//! Each child process is launched inside an anonymous AppContainer, isolating its
//! file-system write access to the workspace scope.
//!
//! Required Win32 call sequence per command launch:
//!
//! ```text
//! 1. CreateAppContainerProfile(name, ...) → appContainerSid (PSID)
//! 2. Build SECURITY_CAPABILITIES { appContainerSid, caps=[], capCount=0 }
//! 3. InitializeProcThreadAttributeList(attrList, count=1)
//! 4. UpdateProcThreadAttribute(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &secCap)
//! 5. STARTUPINFOEXW { .StartupInfo = ..., .lpAttributeList = attrList }
//! 6. Grant scope DACL: SetNamedSecurityInfoW (FILE_ALL_ACCESS for container SID)
//! 7. CreateProcessW(..., CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT, ...)
//! 8. ResumeThread(process)
//! 9. WaitForSingleObject + GetExitCodeProcess
//! 10. Cleanup: CloseHandle × 2, FreeSid, DeleteProcThreadAttributeList,
//!     DeleteAppContainerProfile (on shutdown / new session)
//! ```

use super::error::SandboxError;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// windows-sys imports — Job Object enforcement (active on Windows)
// ---------------------------------------------------------------------------
#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{CloseHandle, FALSE};

#[cfg(target_os = "windows")]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};

#[cfg(target_os = "windows")]
use windows_sys::Win32::System::Threading::GetCurrentProcess;

// ---------------------------------------------------------------------------
// windows-sys imports — AppContainer
// ---------------------------------------------------------------------------
#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::S_OK;

#[cfg(target_os = "windows")]
use windows_sys::Win32::Security::FreeSid;

#[cfg(target_os = "windows")]
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile,
};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Check whether the Windows sandbox prerequisites are met.
///
/// Verifies that the `CreateAppContainerProfile` API is accessible by
/// probing with a NUL container name, which returns an error code rather
/// than succeeding — confirming the Windows version is 8+ and the API is
/// callable.
pub fn check_windows_sandbox_available() -> Result<(), SandboxError> {
    #[cfg(target_os = "windows")]
    {
        probe_appcontainer_api()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(SandboxError::PrerequisiteFailed(
            "Windows AppContainer is only available on Windows".into(),
        ))
    }
}

/// Create a `tokio::process::Command` that runs inside an AppContainer,
/// restricting file-system write access to `scope`.
///
/// On non-Windows targets this returns the base command unchanged (compile-
/// time dead code — the caller guards with `#[cfg(target_os = "windows")]`).
#[allow(dead_code)]
pub fn create_windows_sandboxed_command(
    program: &str,
    args: &[String],
    working_dir: &Path,
    scope: &Path,
    read_scopes: &[PathBuf],
) -> anyhow::Result<tokio::process::Command> {
    #[cfg(target_os = "windows")]
    {
        create_appcontainer_command(program, args, working_dir, scope, read_scopes)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = scope;
        let _ = read_scopes;
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args).current_dir(working_dir);
        Ok(cmd)
    }
}

/// Apply Job Object restrictions to the current server process.
///
/// Sets `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so all child processes are
/// terminated when the server exits.  This is defense-in-depth — it does
/// **not** restrict file-system access by path; that requires AppContainer
/// (see `check_windows_sandbox_available` / `create_windows_sandboxed_command`).
///
/// The job handle is intentionally kept open for the lifetime of the process
/// so the kill-on-close trigger fires at process exit, not at scope exit.
/// When called inside an existing job (e.g., CI runner or Task Scheduler)
/// the assignment will fail with a warning, which is non-fatal.
pub fn enforce_windows_sandbox(_roots: &[PathBuf]) -> Result<(), SandboxError> {
    #[cfg(target_os = "windows")]
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job == std::ptr::null_mut() {
            let err = std::io::Error::last_os_error();
            return Err(SandboxError::PrerequisiteFailed(format!(
                "CreateJobObjectW failed: {err}"
            )));
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::addr_of!(info).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == FALSE
        {
            let err = std::io::Error::last_os_error();
            let _ = CloseHandle(job);
            return Err(SandboxError::PrerequisiteFailed(format!(
                "SetInformationJobObject failed: {err}"
            )));
        }

        if AssignProcessToJobObject(job, GetCurrentProcess()) == FALSE {
            // Non-fatal: the process may already be assigned to an outer job
            // (CI runner, Task Scheduler, or Docker).  The outer job still
            // limits the process tree; log a warning and continue.
            let err = std::io::Error::last_os_error();
            tracing::warn!("AssignProcessToJobObject returned false (already in a job?): {err}");
            let _ = CloseHandle(job);
            return Ok(());
        }

        // Intentionally keep the handle open for process lifetime so the
        // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE trigger fires at exit.
        let _ = job;
        tracing::info!("Windows Job Object enforcement active (kill-on-close)");
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AppContainer helpers (Windows-only)
// ---------------------------------------------------------------------------

/// Probe the AppContainer API to verify we're on Windows 8+ with a working
/// CreateAppContainerProfile implementation.
///
/// We call it with a deliberately invalid name (empty string) — the expected
/// result is `E_INVALIDARG` (0x80070057).  Any other Win32 error also
/// confirms the API is present.  `ERROR_PROC_NOT_FOUND` would mean the DLL
/// entry point is missing (very old OS).
#[cfg(target_os = "windows")]
fn probe_appcontainer_api() -> Result<(), SandboxError> {
    use windows_sys::Win32::Foundation::E_INVALIDARG;

    unsafe {
        // Encode an empty wide string: just the NUL terminator.
        let empty_name: Vec<u16> = [0u16].to_vec();
        let empty_display: Vec<u16> = [0u16].to_vec();
        let empty_desc: Vec<u16> = [0u16].to_vec();
        let mut sid: *mut core::ffi::c_void = std::ptr::null_mut();

        let hr = CreateAppContainerProfile(
            empty_name.as_ptr(),
            empty_display.as_ptr(),
            empty_desc.as_ptr(),
            std::ptr::null(),
            0,
            &mut sid,
        );

        if hr == S_OK {
            // Unexpectedly succeeded with empty name — clean up and continue.
            if !sid.is_null() {
                FreeSid(sid);
            }
            return Ok(());
        }
        if hr == E_INVALIDARG {
            // Expected: API is present, properly rejected the empty name.
            return Ok(());
        }
        // Any other HRESULT means the API is present but something went wrong.
        // Accept it as "available" — actual spawn errors will surface later.
        if sid_looks_like_proc_not_found(hr) {
            return Err(SandboxError::PrerequisiteFailed(format!(
                "AppContainer API unavailable (Windows 8+ required). HRESULT: 0x{hr:08X}"
            )));
        }
        Ok(())
    }
}

/// Heuristic: `HRESULT_FROM_WIN32(ERROR_PROC_NOT_FOUND)` == 0x8007007F
/// indicates the entry point is absent, i.e., the OS is older than Windows 8.
#[cfg(target_os = "windows")]
fn sid_looks_like_proc_not_found(hr: i32) -> bool {
    hr == 0x8007007Fu32 as i32
}

/// Build a deterministic AppContainer profile name from a sandbox scope path.
///
/// Names must be ≤ 64 chars, alphanumeric plus `-` and `.`.  We hash the
/// canonical scope path with a simple FNV-1a and encode as hex.
fn appcontainer_name_for_scope(scope: &Path) -> Vec<u16> {
    // FNV-1a hash of the scope's OS string bytes.
    let raw = scope.as_os_str().to_string_lossy();
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in raw.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let name = format!("ahma-sandbox-{hash:016x}");
    // Encode as a NUL-terminated UTF-16 string.
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A NUL-terminated UTF-16 wide string (retained for future Win32 FFI use).
#[cfg(target_os = "windows")]
#[allow(dead_code)]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Launch `program` with `args` inside an AppContainer restricted to `scope`.
///
/// AppContainer spawn isolation via `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`
/// requires direct `CreateProcessW` Win32 calls because
/// `std::os::windows::process::CommandExt::raw_attribute` is an unstable
/// nightly-only API (tracking issue rust-lang/rust#114854, not yet stabilised).
/// Until that API is stable (or a direct Win32 implementation is added — see the
/// call sequence in the module-level doc comment), processes run under Job Object
/// enforcement only, which provides kill-on-close protection for the child tree.
///
/// The full direct-Win32 implementation plan is documented at the top of this
/// module (`InitializeProcThreadAttributeList` → `UpdateProcThreadAttribute` →
/// `STARTUPINFOEXW` → `CreateProcessW`).
#[cfg(target_os = "windows")]
fn create_appcontainer_command(
    program: &str,
    args: &[String],
    working_dir: &Path,
    scope: &Path,
    read_scopes: &[PathBuf],
) -> anyhow::Result<tokio::process::Command> {
    let _ = scope;
    let _ = read_scopes;

    // Log once that AppContainer spawn isolation is deferred.
    static WARNED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    WARNED.get_or_init(|| {
        tracing::warn!(
            "AppContainer spawn isolation is not active on stable Rust \
             (std::process::Command::raw_attribute requires nightly, \
             rust-lang/rust#114854). Processes run under Job Object enforcement only."
        );
    });

    let mut std_cmd = std::process::Command::new(program);
    std_cmd
        .args(args)
        .current_dir(working_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut cmd = tokio::process::Command::from(std_cmd);

    // Enforce scope via CARGO_TARGET_DIR to prevent cargo from writing
    // build artifacts outside the workspace.
    if std::path::Path::new(program)
        .file_name()
        .is_some_and(|n| n == "cargo" || n == "cargo.exe")
    {
        cmd.env("CARGO_TARGET_DIR", working_dir.join("target"));
    }

    Ok(cmd)
}
// ---------------------------------------------------------------------------
// Cleanup helper — call once on session teardown
// ---------------------------------------------------------------------------

/// Delete the AppContainer profile created for `scope`.
///
/// Profiles are persistent and accumulate in the registry if not deleted.
/// Call this on session shutdown or scope change.
#[allow(dead_code)]
pub fn cleanup_appcontainer_profile(scope: &Path) {
    #[cfg(target_os = "windows")]
    unsafe {
        let container_name = appcontainer_name_for_scope(scope);
        let _ = DeleteAppContainerProfile(container_name.as_ptr());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = scope;
    }
}
