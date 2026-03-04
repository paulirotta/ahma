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
//! Required Win32 call sequence:
//!
//! ```text
//! 1. CreateJobObjectW(NULL, NULL)                          → job HANDLE
//! 2. SetInformationJobObject(job,
//!      JobObjectExtendedLimitInformation,
//!      JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
//!          BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
//!      })
//! 3. AssignProcessToJobObject(job, GetCurrentProcess())
//! 4. Intentionally keep the job handle open (leaking it to process lifetime)
//!    so the kill-on-close trigger fires at process exit, not at scope exit.
//! ```
//!
//! **Note**: Job Objects do **not** restrict file-system access by path.
//! Path security requires AppContainer (R6.3 — see below).
//!
//! ## Not yet implemented: AppContainer (`check_windows_sandbox_available`)
//!
//! Each child command should be spawned inside a fresh anonymous AppContainer
//! so the OS enforces file-access restrictions at the kernel level.
//!
//! Required Win32 call sequence per command launch:
//!
//! ```text
//! 1. CreateAppContainerProfile(name, ...)         → appContainerSid (PSID)
//! 2. Build SECURITY_CAPABILITIES { appContainerSid, caps=[], capCount=0 }
//! 3. InitializeProcThreadAttributeList(attrList, count=1)
//! 4. UpdateProcThreadAttribute(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &secCap)
//! 5. STARTUPINFOEXW { .StartupInfo = ..., .lpAttributeList = attrList }
//! 6. CreateProcessW(..., CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT, ...)
//! 7. Add file-permission grants for the workspace scope ACL (SetNamedSecurityInfoW)
//! 8. ResumeThread(process)
//! 9. Cleanup: CloseHandle × 2, FreeSid, DeleteProcThreadAttributeList
//!    (DeleteAppContainerProfile on shutdown)
//! ```
//!
//! `check_windows_sandbox_available()` currently **fails closed** until AppContainer
//! is implemented.  Acceptance criteria (SPEC.md § R6.3):
//!
//! - [ ] `check_windows_sandbox_available()` returns `Ok(())` on Win8+
//! - [ ] `create_windows_sandboxed_command()` launches in an AppContainer
//! - [ ] Write outside sandbox scope is OS-blocked (integration test)
//! - [ ] `tools/call` before lock returns HTTP 409 / JSON-RPC `-32001`
//! - [ ] `C:\`, `D:\`, UNC roots rejected by `canonicalize_scopes` ✓ (already done)
//! - [ ] All sandbox-gating integration tests pass on `windows-latest` CI

use super::error::SandboxError;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// windows-sys imports — Job Object enforcement (active)
// ---------------------------------------------------------------------------
use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

// ---------------------------------------------------------------------------
// windows-sys imports — AppContainer (not yet active; listed for reference)
// ---------------------------------------------------------------------------
// Uncomment when implementing create_windows_sandboxed_command (§ R6.3):
//
// use windows_sys::Win32::Foundation::{HANDLE, S_OK};
// use windows_sys::Win32::Security::{PSID, FreeSid, SECURITY_CAPABILITIES};
// use windows_sys::Win32::Security::Isolation::{
//     CreateAppContainerProfile, DeleteAppContainerProfile,
// };
// use windows_sys::Win32::System::Threading::{
//     CreateProcessW, STARTUPINFOEXW, PROCESS_INFORMATION,
//     InitializeProcThreadAttributeList, UpdateProcThreadAttribute,
//     DeleteProcThreadAttributeList, ResumeThread,
//     EXTENDED_STARTUPINFO_PRESENT, CREATE_SUSPENDED,
//     PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
// };

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Check whether the Windows sandbox prerequisites are met.
///
/// When the AppContainer implementation lands this function will:
/// 1. Verify `IsWindows8OrGreater()` (or RtlGetVersion ≥ 6.2.0).
/// 2. Probe `CreateAppContainerProfile` to confirm the API is accessible.
///
/// **Current status**: always returns `PrerequisiteFailed` so strict mode
/// fails closed.  Set `AHMA_NO_SANDBOX=1` or pass `--no-sandbox` to proceed
/// without sandbox enforcement.
pub fn check_windows_sandbox_available() -> Result<(), SandboxError> {
    // TODO(windows-sandbox): replace with real version + API probe.
    // Example structure once implemented:
    //
    //   let ver = rtl_get_version()?;
    //   if ver.dwMajorVersion < 6 || (ver.dwMajorVersion == 6 && ver.dwMinorVersion < 2) {
    //       return Err(SandboxError::PrerequisiteFailed(
    //           "AppContainer requires Windows 8 / Server 2012 or later".into(),
    //       ));
    //   }
    //   probe_appcontainer_api()?;
    //   Ok(())

    Err(SandboxError::PrerequisiteFailed(
        "Windows AppContainer sandbox backend is not yet implemented. \
         Pass --no-sandbox (or set AHMA_NO_SANDBOX=1) to run without \
         file-system containment, or wait for the AppContainer backend \
         (SPEC.md § R6.3). See https://github.com/paulirotta/ahma/issues."
            .to_string(),
    ))
}

/// Create a `tokio::process::Command` that runs inside an AppContainer,
/// restricting file-system write access to `scope`.
///
/// **Current status**: not yet implemented — falls through to an unsandboxed
/// command so callers can at least verify compilation; enforcement is blocked
/// by `check_windows_sandbox_available` returning `Err` in strict mode.
///
/// When implemented, this will:
/// 1. Build a deterministic AppContainer profile name from `scope`.
/// 2. Create (or fetch) the container SID.
/// 3. Grant the container SID write access to `scope` via a DACL.
/// 4. Launch the process with `EXTENDED_STARTUPINFO_PRESENT`.
#[allow(dead_code)]
pub fn create_windows_sandboxed_command(
    program: &str,
    args: &[String],
    working_dir: &Path,
    _scope: &Path,
) -> anyhow::Result<tokio::process::Command> {
    // TODO(windows-sandbox): launch in AppContainer once implementation lands.
    //
    // Rough sequence:
    //   let container_name = appcontainer_name_for_scope(scope);
    //   let sid = get_or_create_appcontainer_profile(&container_name)?;
    //   grant_dacl_access(scope, sid)?;
    //   let cmd = spawn_in_appcontainer(program, args, working_dir, sid)?;
    //   FreeSid(sid);
    //   Ok(cmd)

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).current_dir(working_dir);
    Ok(cmd)
}

/// Compute a stable AppContainer profile name from a sandbox scope path.
///
/// AppContainer names must be ≤ 64 characters and contain only alphanumeric
/// characters plus `-` and `.`.  We derive a name by base64url-encoding a
/// truncated SHA-256 of the canonical scope path.
#[allow(dead_code)]
fn appcontainer_name_for_scope(scope: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    scope.hash(&mut h);
    format!("ahma-sandbox-{:016x}", h.finish())
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
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job == 0 {
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
        // HANDLE is isize — std::mem::forget prevents Rust from running any
        // (no-op) drop but makes the intent explicit.
        std::mem::forget(job);
        tracing::info!("Windows Job Object enforcement active (kill-on-close)");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers (stubs — filled in during implementation)
// ---------------------------------------------------------------------------

/// Convert a `PathBuf` scope to an OS-safe ACL and grant it to the given
/// AppContainer SID.  Called after the container SID is obtained and before
/// the child process resumes.
#[allow(dead_code)]
fn grant_dacl_access(_scope: &PathBuf, _container_sid_ptr: usize) -> anyhow::Result<()> {
    // TODO(windows-sandbox): build a DACL granting FILE_ALL_ACCESS to
    // container_sid for scope and all descendants.
    todo!("grant_dacl_access — implement with SetNamedSecurityInfoW")
}
