use anyhow::{Result, anyhow};
use dunce;
use std::path::{Path, PathBuf};

use super::error::SandboxError;
use super::scopes;
use super::types::{SandboxMode, ScopesGuard};

// ─────────────────────────────────────────────────────────────────────────────
// Livelog symlink resolution helpers
// ─────────────────────────────────────────────────────────────────────────────

fn resolve_livelog_scopes(canonicalized: &[PathBuf]) -> Vec<PathBuf> {
    canonicalized
        .iter()
        .filter_map(|scope| resolve_log_dir_symlinks(&log_dir_for_scope(scope)))
        .flatten()
        .collect()
}

fn log_dir_for_scope(scope: &Path) -> PathBuf {
    scope.join("log")
}

fn resolve_log_dir_symlinks(log_dir: &Path) -> Option<Vec<PathBuf>> {
    let entries = std::fs::read_dir(log_dir).ok()?;
    Some(
        entries
            .flatten()
            .filter_map(|entry| {
                let target = resolve_log_symlink(&entry.path(), log_dir)?;
                tracing::info!(
                    "Adding --livelog read-only scope for symlink target: {}",
                    target.display()
                );
                Some(target)
            })
            .collect(),
    )
}

fn resolve_log_symlink(path: &Path, log_dir: &Path) -> Option<PathBuf> {
    if !is_log_symlink(path) {
        return None;
    }

    resolve_log_symlink_target(path, log_dir)
}

fn is_log_symlink(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    meta.is_symlink() && path.extension().is_some_and(|ext| ext == "log")
}

fn resolve_log_symlink_target(path: &Path, log_dir: &Path) -> Option<PathBuf> {
    let target = std::fs::read_link(path).ok()?;
    let canonical_target = dunce::canonicalize(log_dir.join(&target)).ok()?;
    canonical_target.is_file().then_some(canonical_target)
}

// ─────────────────────────────────────────────────────────────────────────────
// Security policy helpers
// ─────────────────────────────────────────────────────────────────────────────

fn is_blocked_temp_path(path_str: &str) -> bool {
    const BLOCKED_PREFIXES: &[&str] = &[
        "/tmp",
        "/var/folders",
        "/private/tmp",
        "/private/var/folders",
        "/dev",
    ];
    BLOCKED_PREFIXES
        .iter()
        .any(|prefix| path_str.starts_with(prefix))
}

fn is_in_temp_dir(path: &Path) -> bool {
    dunce::canonicalize(std::env::temp_dir())
        .map(|temp_dir| path.starts_with(&temp_dir))
        .unwrap_or(false)
}

fn canonicalize_with_fallback(full_path: &Path) -> PathBuf {
    if let Some(parent) = full_path.parent()
        && let Ok(parent_canonical) = dunce::canonicalize(parent)
    {
        return full_path
            .file_name()
            .map(|name| parent_canonical.join(name))
            .unwrap_or(parent_canonical);
    }
    scopes::normalize_path_lexically(full_path)
}

/// The security context for the Ahma session.
pub struct Sandbox {
    pub(super) scopes: std::sync::RwLock<Vec<PathBuf>>,
    pub(super) read_scopes: Vec<PathBuf>,
    pub(super) mode: SandboxMode,
    pub(super) no_temp_files: bool,
    /// When true, the canonical temp directory is preserved across scope updates.
    pub(super) tmp_access: bool,
}

impl Clone for Sandbox {
    fn clone(&self) -> Self {
        Self {
            scopes: std::sync::RwLock::new(self.scopes.read().unwrap().clone()),
            read_scopes: self.read_scopes.clone(),
            mode: self.mode,
            no_temp_files: self.no_temp_files,
            tmp_access: self.tmp_access,
        }
    }
}

impl std::fmt::Debug for Sandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sandbox")
            .field("scopes", &self.scopes.read().unwrap())
            .field("read_scopes", &self.read_scopes)
            .field("mode", &self.mode)
            .field("no_temp_files", &self.no_temp_files)
            .field("tmp_access", &self.tmp_access)
            .finish()
    }
}

impl Sandbox {
    /// Create a new Sandbox with the given scopes.
    pub fn new(
        scopes: Vec<PathBuf>,
        mode: SandboxMode,
        no_temp_files: bool,
        livelog: bool,
        tmp_access: bool,
    ) -> Result<Self> {
        let canonicalized = scopes::canonicalize_scopes(
            scopes,
            mode,
            "Specify explicit directories with --sandbox-scope or --working-directories. \
             Example: --sandbox-scope /home/user/project",
        )?;

        let read_scopes = if livelog && mode != SandboxMode::Test {
            resolve_livelog_scopes(&canonicalized)
        } else {
            Default::default()
        };

        Ok(Self {
            scopes: std::sync::RwLock::new(canonicalized),
            read_scopes,
            mode,
            no_temp_files,
            tmp_access,
        })
    }

    /// Create a sandbox in Test mode (bypasses restrictions).
    pub fn new_test() -> Self {
        Self {
            scopes: std::sync::RwLock::new(vec![PathBuf::from("/")]),
            read_scopes: Vec::new(),
            mode: SandboxMode::Test,
            no_temp_files: false,
            tmp_access: false,
        }
    }

    /// Update the sandbox scopes, preserving the temp directory if `--tmp` was set.
    pub fn update_scopes(&self, scopes: Vec<PathBuf>) -> Result<()> {
        let mut canonicalized = scopes::canonicalize_scopes(
            scopes,
            self.mode,
            "Client must provide valid workspace roots.",
        )?;

        if let Some(canonical_temp) = self.preserved_temp_dir(&canonicalized) {
            tracing::info!(
                "Preserving temp directory in sandbox scopes via --tmp: {:?}",
                canonical_temp
            );
            canonicalized.push(canonical_temp);
        }

        let mut current_scopes = self.scopes.write().unwrap();
        *current_scopes = canonicalized;
        Ok(())
    }

    /// Check if the sandbox is in test mode.
    pub fn is_test_mode(&self) -> bool {
        self.mode == SandboxMode::Test
    }

    /// Returns true when tool calls can execute against sandboxed roots.
    ///
    /// In test mode, tool calls are always allowed. In normal modes, at least one
    /// rooted scope must be configured (typically via roots/list).
    pub fn is_ready_for_tool_calls(&self) -> bool {
        self.is_test_mode() || !self.scopes().is_empty()
    }

    /// Check if no-temp-files mode is enabled.
    pub fn is_no_temp_files(&self) -> bool {
        self.no_temp_files
    }

    /// Get the allowed scopes.
    pub fn scopes(&self) -> ScopesGuard<'_> {
        ScopesGuard(self.scopes.read().unwrap())
    }

    /// Get the read-only scopes (for --livelog symlink targets).
    pub fn read_scopes(&self) -> &[PathBuf] {
        &self.read_scopes
    }

    /// Check if a path is within any of the sandbox scopes.
    pub fn validate_path(&self, path: &Path) -> Result<PathBuf> {
        let scopes_guard = self.scopes();

        if self.should_bypass_validation(&scopes_guard) {
            return self.resolve_test_path(path);
        }

        let canonical = self.resolve_path(path, &scopes_guard)?;

        if !self.is_path_allowed(&canonical, &scopes_guard) {
            return Err(SandboxError::PathOutsideSandbox {
                path: path.to_path_buf(),
                scopes: scopes_guard.to_vec(),
            }
            .into());
        }

        self.check_security_policies(path, &canonical)?;
        Ok(canonical)
    }

    fn should_bypass_validation(&self, scopes_guard: &[PathBuf]) -> bool {
        self.mode == SandboxMode::Test && is_test_root_scope(scopes_guard)
    }

    fn resolve_test_path(&self, path: &Path) -> Result<PathBuf> {
        // Use dunce::canonicalize to avoid the \\?\ extended-length prefix that
        // std::fs::canonicalize adds on Windows; that prefix is accepted by most
        // Win32 APIs but rejected by CreateProcess as a working directory
        // (OS error 267 "The directory name is invalid").
        dunce::canonicalize(path).or_else(|_| Ok(path.to_path_buf()))
    }

    fn resolve_path(&self, path: &Path, scopes_guard: &[PathBuf]) -> Result<PathBuf> {
        let first_scope = scopes_guard
            .first()
            .ok_or_else(|| anyhow!("No sandbox scopes configured"))?;

        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            first_scope.join(path)
        };

        Ok(dunce::canonicalize(&full_path)
            .unwrap_or_else(|_| canonicalize_with_fallback(&full_path)))
    }

    fn is_path_allowed(&self, canonical: &Path, scopes_guard: &[PathBuf]) -> bool {
        let canonical_stripped = strip_extended_prefix(canonical);
        scopes_guard
            .iter()
            .any(|scope| canonical_stripped.starts_with(strip_extended_prefix(scope)))
    }

    fn check_security_policies(&self, original_path: &Path, canonical: &Path) -> Result<()> {
        if !self.no_temp_files {
            return Ok(());
        }

        let path_str = canonical.to_string_lossy();
        let is_blocked = is_blocked_temp_path(&path_str) || is_in_temp_dir(canonical);
        if !is_blocked {
            return Ok(());
        }

        Err(SandboxError::HighSecurityViolation {
            path: original_path.to_path_buf(),
        }
        .into())
    }

    fn preserved_temp_dir(&self, canonicalized: &[PathBuf]) -> Option<PathBuf> {
        if !self.tmp_access {
            return None;
        }

        let canonical_temp = dunce::canonicalize(std::env::temp_dir()).ok()?;
        (!canonicalized.contains(&canonical_temp)).then_some(canonical_temp)
    }
}

fn is_test_root_scope(scopes_guard: &[PathBuf]) -> bool {
    scopes_guard.is_empty() || scopes_guard.iter().any(|scope| scope == Path::new("/"))
}

/// Strip the Windows extended-length path prefix (`\\?\`) if present.
/// Returns an owned `PathBuf`; on non-Windows this is always a clone.
///
/// Windows `std::fs::canonicalize` may add or omit `\\?\` depending on
/// the input form.  Stripping before `starts_with` comparisons lets paths
/// referring to the same location compare equal.
fn strip_extended_prefix(path: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(stripped) = path.as_os_str().to_string_lossy().strip_prefix(r"\\?\") {
        return PathBuf::from(stripped);
    }
    path.to_path_buf()
}
