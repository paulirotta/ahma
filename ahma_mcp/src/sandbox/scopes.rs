use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};

use super::types::SandboxMode;

/// Returns `true` when `path` is a filesystem root — every absolute path on
/// this platform is a descendant of it — making it unsuitable as a sandbox
/// scope because it provides no containment.
///
/// | Platform | Root examples                         |
/// |----------|---------------------------------------|
/// | Unix     | `/`                                   |
/// | Windows  | `C:\`, `D:\`, `\\server\share` (UNC)  |
fn is_filesystem_root(path: &Path) -> bool {
    use std::path::Component;
    let mut it = path.components();
    match it.next() {
        // Unix root: just a single RootDir component
        Some(Component::RootDir) => it.next().is_none(),
        // Windows drive root: Prefix("C:") + RootDir, nothing after
        Some(Component::Prefix(_)) => {
            matches!(it.next(), Some(Component::RootDir)) && it.next().is_none()
        }
        _ => false,
    }
}

/// Canonicalize and validate a list of sandbox scopes.
///
/// Rejects filesystem roots and empty paths in Strict mode.
/// Falls back to raw paths in Test mode when canonicalization fails.
///
/// For symlink-aware compatibility, this preserves both canonical and absolute
/// alias paths (when they differ). This allows equivalent paths to validate
/// correctly even when lexical normalization is used for non-existent targets.
pub(super) fn canonicalize_scopes(
    scopes: Vec<PathBuf>,
    mode: SandboxMode,
    context: &str,
) -> Result<Vec<PathBuf>> {
    let cwd = std::env::current_dir().ok();
    let mut canonicalized = Vec::with_capacity(scopes.len() * 2);

    let mut push_unique = |candidate: PathBuf| {
        if !canonicalized.contains(&candidate) {
            canonicalized.push(candidate);
        }
    };

    for scope in scopes {
        if mode != SandboxMode::Test && (is_filesystem_root(&scope) || scope == Path::new("")) {
            return Err(anyhow!(
                "Filesystem root or empty path is not a valid sandbox scope \
                 (path: '{}', OS: {}). {}",
                scope.display(),
                std::env::consts::OS,
                context
            ));
        }

        let absolute_alias = if scope.is_absolute() {
            Some(scope.clone())
        } else {
            cwd.as_ref()
                .map(|c| normalize_path_lexically(&c.join(&scope)))
        };

        let canonical = match std::fs::canonicalize(&scope) {
            Ok(c) => c,
            Err(e) => {
                if mode == SandboxMode::Test {
                    scope.clone()
                } else {
                    return Err(anyhow!(
                        "Failed to canonicalize sandbox scope '{}': {}",
                        scope.display(),
                        e
                    ));
                }
            }
        };

        if mode != SandboxMode::Test && is_filesystem_root(&canonical) {
            return Err(anyhow!(
                "Filesystem root is not a valid sandbox scope \
                 (resolved from '{}', OS: {}). {}",
                scope.display(),
                std::env::consts::OS,
                context
            ));
        }

        push_unique(canonical.clone());

        if let Some(alias) = absolute_alias {
            let alias = normalize_path_lexically(&alias);
            if alias != canonical {
                push_unique(alias);
            }
        }
    }
    Ok(canonicalized)
}

/// Normalize a path lexically (without filesystem access).
pub fn normalize_path_lexically(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut stack = Vec::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Never pop a root anchor (RootDir or Prefix like "C:") —
                // that would allow escaping to an invalid path on Windows.
                if stack
                    .last()
                    .is_some_and(|c| !matches!(c, Component::RootDir | Component::Prefix(_)))
                {
                    stack.pop();
                }
            }
            c => stack.push(c),
        }
    }

    stack.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::path_helpers::{test_abs, test_root};
    use std::path::PathBuf;

    // -----------------------------------------------------------------------
    // is_filesystem_root — cross-platform
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_filesystem_root_unix_root() {
        assert!(is_filesystem_root(&test_root()));
    }

    #[test]
    fn test_is_filesystem_root_unix_subdir() {
        assert!(!is_filesystem_root(&test_abs(&["home", "user"])));
    }

    #[test]
    fn test_is_filesystem_root_relative_path() {
        assert!(!is_filesystem_root(Path::new("relative/path")));
    }

    #[test]
    fn test_is_filesystem_root_empty() {
        assert!(!is_filesystem_root(Path::new("")));
    }

    #[cfg(windows)]
    #[test]
    fn test_is_filesystem_root_windows_drive_roots() {
        assert!(is_filesystem_root(Path::new("C:\\")));
        assert!(is_filesystem_root(Path::new("D:\\")));
    }

    #[cfg(windows)]
    #[test]
    fn test_is_filesystem_root_windows_subdir_not_root() {
        assert!(!is_filesystem_root(Path::new("C:\\Users\\test")));
    }

    #[cfg(windows)]
    #[test]
    fn test_is_filesystem_root_windows_unc_root_vs_subpath() {
        // UNC share root (trailing slash → Prefix + RootDir, nothing after)
        assert!(is_filesystem_root(Path::new("\\\\server\\share\\")));
        // UNC subpath is NOT a root
        assert!(!is_filesystem_root(Path::new("\\\\server\\share\\path")));
    }

    // -----------------------------------------------------------------------
    // normalize_path_lexically — dotdot safety, cross-platform
    // -----------------------------------------------------------------------

    #[test]
    fn test_normalize_extra_dotdot_cannot_escape_unix_root() {
        // Excess `..` beyond root must never collapse the RootDir sentinel.
        let result = normalize_path_lexically(&test_abs(&["a", "..", "..", ".."]));
        assert_eq!(result, test_root());
    }

    #[test]
    fn test_normalize_removes_current_dir() {
        assert_eq!(
            normalize_path_lexically(&test_abs(&["a", ".", "b"])),
            test_abs(&["a", "b"]),
        );
    }

    #[test]
    fn test_normalize_removes_parent_dir() {
        assert_eq!(
            normalize_path_lexically(&test_abs(&["a", "b", "..", "c"])),
            test_abs(&["a", "c"]),
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_normalize_dotdot_cannot_escape_windows_drive_root() {
        // Multiple `..` beyond drive root must not pop Prefix or RootDir.
        let result = normalize_path_lexically(Path::new("C:\\a\\..\\..\\.."));
        assert_eq!(result, PathBuf::from("C:\\"));
    }

    #[cfg(windows)]
    #[test]
    fn test_normalize_drive_relative_dotdot_stays_on_drive() {
        // Drive-relative path: `..` must not pop the Prefix anchor.
        // e.g. C:..\escape must remain under the C: prefix, not become bare "escape".
        let result = normalize_path_lexically(Path::new("C:..\\escape"));
        assert!(
            result
                .to_str()
                .map(|s| s.starts_with("C:"))
                .unwrap_or(false),
            "expected C:-prefixed result, got {}",
            result.display()
        );
    }

    // -----------------------------------------------------------------------
    // canonicalize_scopes — root rejection (cross-platform & Windows-only)
    // -----------------------------------------------------------------------

    #[test]
    fn test_canonicalize_rejects_unix_root_in_strict_mode() {
        let err = canonicalize_scopes(vec![test_root()], SandboxMode::Strict, "test context")
            .unwrap_err();
        assert!(
            err.to_string().contains("not a valid sandbox scope"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_canonicalize_rejects_empty_path_in_strict_mode() {
        let result =
            canonicalize_scopes(vec![PathBuf::from("")], SandboxMode::Strict, "test context");
        assert!(
            result.is_err(),
            "empty path must be rejected in Strict mode"
        );
    }

    #[test]
    fn test_canonicalize_accepts_real_subdir() {
        let dir = tempfile::tempdir().unwrap();
        let result = canonicalize_scopes(
            vec![dir.path().to_path_buf()],
            SandboxMode::Strict,
            "test context",
        );
        assert!(
            result.is_ok(),
            "real existing dir must be accepted: {:?}",
            result
        );
    }

    #[test]
    fn test_canonicalize_root_allowed_in_test_mode() {
        // SandboxMode::Test bypasses the root guard (used by test harnesses).
        let result =
            canonicalize_scopes(vec![PathBuf::from("/")], SandboxMode::Test, "test context");
        // Might succeed or fail for other reasons, but NOT the sandbox root guard.
        if let Err(e) = result {
            assert!(
                !e.to_string().contains("not a valid sandbox scope"),
                "Test mode must not apply root guard: {e}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn test_canonicalize_rejects_windows_drive_root_in_strict_mode() {
        let err = canonicalize_scopes(
            vec![PathBuf::from("C:\\")],
            SandboxMode::Strict,
            "test context",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("not a valid sandbox scope"),
            "C:\\ must be rejected as sandbox scope: {err}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_canonicalize_rejects_unc_root_in_strict_mode() {
        let err = canonicalize_scopes(
            vec![PathBuf::from("\\\\server\\share\\")],
            SandboxMode::Strict,
            "test context",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("not a valid sandbox scope"),
            "UNC root must be rejected as sandbox scope: {err}"
        );
    }
}
