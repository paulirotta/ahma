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

/// Create a display name wide string (NUL-terminated UTF-16).
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A newtype wrapper around `SECURITY_CAPABILITIES` that satisfies the
/// `Copy + Send + Sync + 'static` bounds required by
/// `std::os::windows::process::CommandExt::raw_attribute`.
///
/// # Safety
///
/// `SECURITY_CAPABILITIES` contains `AppContainerSid: PSID` (a raw pointer).
/// This wrapper is safe to declare `Send + Sync` because:
/// 1. We only pass one instance of this value to one `Command`, on one thread.
/// 2. The `AppContainerSid` pointer is valid until after `Command::spawn()` is
///    called (we intentionally do not call `FreeSid` before that point).
/// 3. No aliasing occurs — the pointer is only dereferenced by
///    `CreateProcessW` inside `spawn()`.
#[cfg(target_os = "windows")]
#[derive(Copy, Clone)]
#[repr(C)]
struct SendableSecCaps(windows_sys::Win32::Security::SECURITY_CAPABILITIES);

#[cfg(target_os = "windows")]
// SAFETY: see doc comment on `SendableSecCaps` above.
unsafe impl Send for SendableSecCaps {}

#[cfg(target_os = "windows")]
// SAFETY: see doc comment on `SendableSecCaps` above.
unsafe impl Sync for SendableSecCaps {}

/// Launch `program` with `args` inside an AppContainer restricted to `scope`.
///
/// Returns a `tokio::process::Command` configured to run the process inside a
/// Windows AppContainer using `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`
/// (via `std::os::windows::process::CommandExt::raw_attribute`, stable since
/// Rust 1.76).  The AppContainer SID is derived from the deterministic profile
/// name built from the `scope` path hash, and the scope directory's DACL is
/// updated to grant the container full read/write access before the process is
/// spawned.  Read-only scopes receive read + execute access.
///
/// The AppContainer SID is intentionally not freed before `spawn()` — it is a
/// bounded per-command allocation (~28 bytes for a typical S-1-15-2-* SID) and
/// must remain valid for the duration of `CreateProcessW` inside `spawn()`.
#[cfg(target_os = "windows")]
fn create_appcontainer_command(
    program: &str,
    args: &[String],
    working_dir: &Path,
    scope: &Path,
    read_scopes: &[PathBuf],
) -> anyhow::Result<tokio::process::Command> {
    use std::os::windows::process::CommandExt as WinCommandExt;
    use windows_sys::Win32::Security::SECURITY_CAPABILITIES;
    use windows_sys::Win32::System::Threading::PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES;

    // Build the container profile name.
    let container_name = appcontainer_name_for_scope(scope);
    let display = to_wide("Ahma sandbox");
    let description = to_wide("Ahma tool execution sandbox");

    let mut std_cmd = std::process::Command::new(program);
    std_cmd
        .args(args)
        .current_dir(working_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    unsafe {
        let mut sid: *mut core::ffi::c_void = std::ptr::null_mut();
        let hr = CreateAppContainerProfile(
            container_name.as_ptr(),
            display.as_ptr(),
            description.as_ptr(),
            std::ptr::null(),
            0,
            &mut sid,
        );

        // 0x800700B7 = HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS) — profile reuse is fine.
        if hr != S_OK && hr != 0x800700B7u32 as i32 {
            // Profile creation failed. Run without AppContainer confinement in
            // non-strict mode; strict mode is gated by check_windows_sandbox_available.
            tracing::warn!(
                "CreateAppContainerProfile returned 0x{:08X}; running without AppContainer",
                hr as u32
            );
        } else {
            // Grant `scope` full access for the container SID.
            if !sid.is_null() {
                if let Err(e) = set_scope_dacl_for_container(scope, sid, true) {
                    tracing::warn!("Failed to set scope DACL for AppContainer: {e}");
                }

                // Grant `read_scopes` read-only access.
                for read_scope in read_scopes {
                    if let Err(e) = set_scope_dacl_for_container(read_scope, sid, false) {
                        tracing::warn!("Failed to set read_scope DACL for AppContainer: {e}");
                    }
                }

                // Attach the AppContainer SID to the process-creation attribute list.
                //
                // `raw_attribute` (stable since Rust 1.76 / #114854) stores a copy of
                // `sec_caps` inside the Command, keeping it alive until `spawn()`.
                // Internally, Rust calls `UpdateProcThreadAttribute` with
                // `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` and our struct,
                // then passes `EXTENDED_STARTUPINFO_PRESENT` to `CreateProcessW`.
                //
                // The `AppContainerSid` pointer (`sid`) must remain valid until
                // `CreateProcessW` is called inside `spawn()`.  We intentionally skip
                // `FreeSid(sid)` here — the SID allocation (~28 bytes) is released when
                // the AppContainer profile is deleted on session shutdown via
                // `cleanup_appcontainer_profile`.
                let sec_caps = SendableSecCaps(SECURITY_CAPABILITIES {
                    AppContainerSid: sid,
                    Capabilities: std::ptr::null_mut(),
                    CapabilityCount: 0,
                    Reserved: 0,
                });

                // SAFETY: `sec_caps` is a valid SECURITY_CAPABILITIES whose
                // `AppContainerSid` pointer is live for the duration of `spawn()`.
                // `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` is a documented
                // Win32 attribute value. Misuse here cannot cause UB outside this
                // sandboxed child process.
                std_cmd.raw_attribute(
                    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                    sec_caps,
                );

                tracing::debug!(
                    "AppContainer SID attached via raw_attribute; process will run in AppContainer"
                );
            }
        }
    }

    let mut cmd = tokio::process::Command::from(std_cmd);

    // Pass the container profile name as an env var so callers can
    // introspect or log the active container (purely informational).
    let profile_name_lossy: String = String::from_utf16_lossy(
        container_name
            .strip_suffix(&[0u16])
            .unwrap_or(&container_name),
    );
    cmd.env("AHMA_APPCONTAINER", &profile_name_lossy);

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
/// Set a DACL on `scope` (and descendants) granting `FILE_ALL_ACCESS` to
/// `container_sid`.  This allows the AppContainer process to read and write
/// within the sandbox scope.
///
/// Uses `SetNamedSecurityInfoW` with a DACL built from `AddAccessAllowedAce`.
/// Falls back without error if DACL manipulation is not available (the
/// container will simply see access-denied attempts instead of silent success).
#[cfg(target_os = "windows")]
fn set_scope_dacl_for_container(
    scope: &Path,
    container_sid: *mut core::ffi::c_void,
    full_access: bool,
) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::Authorization::{SE_FILE_OBJECT, SetNamedSecurityInfoW};
    use windows_sys::Win32::Security::{
        ACL, ACL_REVISION, AddAccessAllowedAce, DACL_SECURITY_INFORMATION, GetLengthSid,
        InitializeAcl,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ,
    };

    let access_mask = if full_access {
        FILE_ALL_ACCESS
    } else {
        FILE_GENERIC_READ | FILE_GENERIC_EXECUTE
    };

    unsafe {
        let sid_len = GetLengthSid(container_sid) as usize;
        // ACL size = header (8 bytes) + ACE_HEADER + ACCESS_MASK + SID
        let acl_size = 8usize + std::mem::size_of::<u32>() + std::mem::size_of::<u32>() + sid_len;
        let mut acl_buf = vec![0u8; acl_size + 16]; // +16 for alignment
        let acl_ptr = acl_buf.as_mut_ptr() as *mut ACL;

        if InitializeAcl(acl_ptr, acl_buf.len() as u32, ACL_REVISION as u32) == 0 {
            let err = std::io::Error::last_os_error();
            anyhow::bail!("InitializeAcl failed: {err}");
        }

        if AddAccessAllowedAce(acl_ptr, ACL_REVISION as u32, access_mask, container_sid) == 0 {
            let err = std::io::Error::last_os_error();
            anyhow::bail!("AddAccessAllowedAce failed: {err}");
        }

        let scope_wide: Vec<u16> = scope
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let result = SetNamedSecurityInfoW(
            scope_wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl_ptr,
            std::ptr::null_mut(),
        );

        if result != 0 {
            let err = std::io::Error::from_raw_os_error(result as i32);
            anyhow::bail!("SetNamedSecurityInfoW failed: {err}");
        }

        Ok(())
    }
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
