use serde_json::json;
use std::path::{Path, PathBuf};

/// Normalize a path string for cross-platform comparison.
///
/// On Windows CI (GitHub Actions), commands like `pwd` often resolve to the
/// MSYS2/Git Bash binary, which outputs Unix-style paths (`/c/Users/...`)
/// instead of native Windows paths (`C:\Users\...`).  This function
/// normalises both representations to a common canonical form so that
/// assertions comparing Rust's `PathBuf::to_string_lossy()` against
/// command output work reliably on every platform.
///
/// Normalisation steps:
/// 1. Trim surrounding whitespace / newlines.
/// 2. Replace backslashes with forward slashes.
/// 3. Convert MSYS drive prefixes `/c/` → `c:/`.
/// 4. Fold to lowercase (Windows paths are case-insensitive).
pub fn normalize_path_for_comparison(path: &str) -> String {
    let s = path.trim();
    // Backslash → forward slash
    let s = s.replace('\\', "/");
    // Strip Windows extended-length path prefix if present
    let s = s.strip_prefix("//?/").unwrap_or(&s).to_string();
    // MSYS drive prefix: /c/ → c:/
    let s = if s.len() >= 3
        && s.starts_with('/')
        && s.as_bytes()[1].is_ascii_alphabetic()
        && s.as_bytes()[2] == b'/'
    {
        format!("{}:{}", &s[1..2], &s[2..])
    } else {
        s
    };
    s.to_lowercase()
}

/// Return `true` if `haystack` contains `needle` after cross-platform
/// path normalisation.  Use this instead of
/// `output.contains(path.to_string_lossy())` whenever the output may
/// come from a shell command on Windows.
pub fn paths_equivalent(haystack: &str, needle: &Path) -> bool {
    let norm_hay = normalize_path_for_comparison(haystack);
    // Resolve Windows 8.3 short paths (e.g., RUNNER~1) to long names if possible
    let needle_str = if let Ok(canonical) = needle.canonicalize() {
        canonical.to_string_lossy().into_owned()
    } else {
        needle.to_string_lossy().into_owned()
    };
    let norm_needle = normalize_path_for_comparison(&needle_str);
    norm_hay.contains(&norm_needle)
}

/// Create a minimal `pwd` tool config file inside `tools_dir`.
///
/// Many integration tests need a tool that simply prints the working
/// directory.  This helper writes the JSON config once and returns the
/// path to the created file.
pub fn create_pwd_tool_config(tools_dir: &Path) -> PathBuf {
    let tool_config = json!({
        "name": "pwd",
        "description": "Print current working directory",
        "command": "pwd",
        "enabled": true,
        "subcommand": [{"name": "default", "description": "Print working directory"}]
    });
    let path = tools_dir.join("pwd.json");
    std::fs::write(&path, serde_json::to_string_pretty(&tool_config).unwrap())
        .expect("Failed to write pwd tool config");
    path
}

/// Parse a file:// URI to a filesystem path.
pub fn parse_file_uri(uri: &str) -> Option<PathBuf> {
    if !uri.starts_with("file://") {
        return None;
    }
    let path_str = uri.strip_prefix("file://")?;
    if path_str.is_empty() {
        return None;
    }
    let decoded = urlencoding::decode(path_str).ok()?;
    Some(PathBuf::from(decoded.into_owned()))
}

/// Encode a filesystem path as a file:// URI path component.
///
/// Produces RFC 8089-compatible URIs.  On Windows, drive-letter paths
/// (`C:\Users\…`) are converted to the canonical `/C:/Users/…` form so
/// that the drive letter appears in the *path* component, not the
/// authority.  This is required for correct round-trip parsing via
/// `url::Url::parse` → `url.to_file_path()`.
pub fn encode_file_uri(path: &Path) -> String {
    let mut path_str = path.to_string_lossy().into_owned();

    // Strip Windows extended-length prefix (\\?\) if present.
    if path_str.starts_with(r"\\?\") {
        path_str = path_str[4..].to_string();
    }

    // Normalise path separators to forward slashes.
    path_str = path_str.replace('\\', "/");

    let mut out = String::with_capacity(path_str.len() + 10);
    out.push_str("file://");

    // On Windows a drive-letter path looks like "C:/Users/…".
    // RFC 8089 §2 requires the path to start with "/" so that it occupies
    // the path component, not the authority.  Insert the leading slash here
    // so we produce "file:///C:/Users/…" (or "file://localhost/C:/Users/…").
    #[cfg(target_os = "windows")]
    {
        let is_drive = path_str.len() >= 2
            && path_str.as_bytes()[0].is_ascii_alphabetic()
            && path_str.as_bytes()[1] == b':';
        if is_drive {
            out.push('/');
        }
    }

    for b in path_str.as_bytes() {
        let b = *b;
        let keep = matches!(
            b,
            b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'-'
                | b'.'
                | b'_'
                | b'~'
                | b'/'
                | b':'  // keep colon for Windows drive letters (e.g. C:/)
        );
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

/// Malformed URI test cases for edge case testing.
pub mod malformed_uris {
    /// URIs that should be rejected (return None from parse_file_uri).
    pub const INVALID: &[&str] = &[
        "",
        "file://",
        "http://localhost/path",
        "https://example.com/file",
        "ftp://server/file",
        "file:",
        "file:/",
    ];

    /// URIs that might look valid but have edge cases.
    pub const EDGE_CASES: &[(&str, Option<&str>)] = &[
        ("file:///tmp/test", Some("/tmp/test")),
        ("file:///", Some("/")),
        ("file:///tmp/test%20file", Some("/tmp/test file")),
        ("file:///tmp/%C3%B1", Some("/tmp/ñ")),
        ("file:///tmp/a%2Fb", Some("/tmp/a/b")),
        ("file:///C:/Windows", Some("/C:/Windows")),
    ];
}
