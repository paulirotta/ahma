use std::path::{Path, PathBuf};

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

/// Encode a filesystem path as a file:// URI.
pub fn encode_file_uri(path: &Path) -> String {
    let mut path_str = path.to_string_lossy().into_owned();

    // Strip Windows extended prefix
    if path_str.starts_with(r"\\?\") {
        path_str = path_str[4..].to_string();
    }

    // Convert backslashes to forward slashes
    path_str = path_str.replace('\\', "/");

    let mut out = String::with_capacity(path_str.len() + 8);
    out.push_str("file://");

    #[cfg(target_os = "windows")]
    if path_str.chars().nth(1) == Some(':') {
        // Windows drive paths typically have a leading slash in URIs
        out.push('/');
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
