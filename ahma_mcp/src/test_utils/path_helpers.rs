//! Cross-platform absolute-path helpers for tests.
//!
//! Lexical path logic (`normalize_path`, `normalize_path_lexically`) is
//! identical on Linux, macOS and Windows, but the *anchor* differs:
//!
//! | Platform | Root  | `test_abs(&["a","b"])` |
//! |----------|-------|------------------------|
//! | Unix     | `/`   | `/a/b`                 |
//! | Windows  | `C:\` | `C:\a\b`               |
//!
//! Using these helpers instead of hard-coding `/a/b/c` strings lets every
//! normalization test run without a `#[cfg(unix)]` guard.
//!
//! # Example
//!
//! ```rust,ignore
//! use ahma_mcp::test_utils::path_helpers::{test_abs, test_root};
//!
//! let path = test_abs(&["a", ".", "b"]);  // /a/./b  or  C:\a\.\b
//! let expected = test_abs(&["a", "b"]);   // /a/b    or  C:\a\b
//! assert_eq!(normalize_path_lexically(&path), expected);
//! ```

use std::path::PathBuf;

/// Returns the platform-appropriate filesystem root used by test helpers.
///
/// * Unix  → `PathBuf::from("/")`
/// * Windows → `PathBuf::from("C:\\")`
pub fn test_root() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/")
    }
    #[cfg(windows)]
    {
        PathBuf::from("C:\\")
    }
    // Fallback for any platform the above doesn't cover during cross-compile
    // probes — this branch is unreachable at runtime but silences the
    // "no matching cfg" warning that some versions of rustc emit.
    #[cfg(not(any(unix, windows)))]
    {
        PathBuf::from("/")
    }
}

/// Build an absolute path anchored at [`test_root()`] by joining `components`.
///
/// Each element of `components` is passed to [`PathBuf::join`], so you may
/// include `.` (`CurDir`), `..` (`ParentDir`), or normal names.
///
/// ```
/// # use ahma_mcp::test_utils::path_helpers::test_abs;
/// // Unix:    /a/b/c
/// // Windows: C:\a\b\c
/// let p = test_abs(&["a", "b", "c"]);
/// assert!(p.is_absolute());
/// ```
pub fn test_abs(components: &[&str]) -> PathBuf {
    let mut p = test_root();
    for &c in components {
        p = p.join(c);
    }
    p
}
