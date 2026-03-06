use anyhow::{Result, anyhow};
use dunce;
use std::path::{Path, PathBuf};

use super::error::SandboxError;
use super::scopes;
use super::types::{SandboxMode, ScopesGuard};

/// The security context for the Ahma session.
pub struct Sandbox {
    pub(super) scopes: std::sync::RwLock<Vec<PathBuf>>,
    pub(super) read_scopes: Vec<PathBuf>,
    pub(super) mode: SandboxMode,
    pub(super) no_temp_files: bool,
}

impl Clone for Sandbox {
    fn clone(&self) -> Self {
        Self {
            scopes: std::sync::RwLock::new(self.scopes.read().unwrap().clone()),
            read_scopes: self.read_scopes.clone(),
            mode: self.mode,
            no_temp_files: self.no_temp_files,
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
    ) -> Result<Self> {
        let canonicalized = scopes::canonicalize_scopes(
            scopes,
            mode,
            "Specify explicit directories with --sandbox-scope or --working-directories. \
             Example: --sandbox-scope /home/user/project",
        )?;

        let mut read_scopes = Vec::new();

        if livelog && mode != SandboxMode::Test {
            for scope in &canonicalized {
                let log_dir = scope.join("log");
                if let Ok(entries) = std::fs::read_dir(&log_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Ok(meta) = std::fs::symlink_metadata(&path)
                            && meta.is_symlink()
                            && path.extension().is_some_and(|e| e == "log")
                            && let Ok(target) = std::fs::read_link(&path)
                            && let Ok(canonical_target) = dunce::canonicalize(log_dir.join(&target))
                            && canonical_target.is_file()
                        {
                            tracing::info!(
                                "Adding --livelog read-only scope for symlink target: {}",
                                canonical_target.display()
                            );
                            read_scopes.push(canonical_target);
                        }
                    }
                }
            }
        }

        Ok(Self {
            scopes: std::sync::RwLock::new(canonicalized),
            read_scopes,
            mode,
            no_temp_files,
        })
    }

    /// Create a sandbox in Test mode (bypasses restrictions).
    pub fn new_test() -> Self {
        Self {
            scopes: std::sync::RwLock::new(vec![PathBuf::from("/")]),
            read_scopes: Vec::new(),
            mode: SandboxMode::Test,
            no_temp_files: false,
        }
    }

    /// Update the sandbox scopes.
    pub fn update_scopes(&self, scopes: Vec<PathBuf>) -> Result<()> {
        let canonicalized = scopes::canonicalize_scopes(
            scopes,
            self.mode,
            "Client must provide valid workspace roots.",
        )?;

        let mut current_scopes = self.scopes.write().unwrap();
        *current_scopes = canonicalized;
        Ok(())
    }

    /// Check if the sandbox is in test mode.
    pub fn is_test_mode(&self) -> bool {
        self.mode == SandboxMode::Test
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

        if self.is_path_allowed(&canonical, &scopes_guard) {
            self.check_security_policies(path, &canonical)?;
            Ok(canonical)
        } else {
            Err(SandboxError::PathOutsideSandbox {
                path: path.to_path_buf(),
                scopes: scopes_guard.to_vec(),
            }
            .into())
        }
    }

    fn should_bypass_validation(&self, scopes_guard: &[PathBuf]) -> bool {
        self.mode == SandboxMode::Test
            && (scopes_guard.is_empty() || scopes_guard.iter().any(|s| s == Path::new("/")))
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

        Ok(match dunce::canonicalize(&full_path) {
            Ok(p) => p,
            Err(_) => {
                if let Some(parent) = full_path.parent() {
                    if let Ok(parent_canonical) = dunce::canonicalize(parent) {
                        if let Some(name) = full_path.file_name() {
                            parent_canonical.join(name)
                        } else {
                            parent_canonical
                        }
                    } else {
                        scopes::normalize_path_lexically(&full_path)
                    }
                } else {
                    scopes::normalize_path_lexically(&full_path)
                }
            }
        })
    }

    fn is_path_allowed(&self, canonical: &Path, scopes_guard: &[PathBuf]) -> bool {
        let canonical_stripped = strip_extended_prefix(canonical);
        scopes_guard
            .iter()
            .any(|scope| canonical_stripped.starts_with(strip_extended_prefix(scope)))
    }

    fn check_security_policies(&self, original_path: &Path, canonical: &Path) -> Result<()> {
        if self.no_temp_files {
            let path_str = canonical.to_string_lossy();
            if path_str.starts_with("/tmp")
                || path_str.starts_with("/var/folders")
                || path_str.starts_with("/private/tmp")
                || path_str.starts_with("/private/var/folders")
                || path_str.starts_with("/dev")
            {
                return Err(SandboxError::HighSecurityViolation {
                    path: original_path.to_path_buf(),
                }
                .into());
            }

            if let Ok(temp_dir) = dunce::canonicalize(std::env::temp_dir())
                && canonical.starts_with(&temp_dir)
            {
                return Err(SandboxError::HighSecurityViolation {
                    path: original_path.to_path_buf(),
                }
                .into());
            }
        }
        Ok(())
    }
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
