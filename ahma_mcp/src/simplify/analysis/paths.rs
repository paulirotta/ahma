use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Path utilities
// ---------------------------------------------------------------------------

pub fn get_relative_path(path: &Path, base_dir: &Path) -> PathBuf {
    // Use dunce::canonicalize to avoid Windows \\?\ prefix issues
    let abs_path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let abs_base = dunce::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
    abs_path
        .strip_prefix(&abs_base)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| path.to_path_buf())
}

pub fn get_package_name(path: &Path, base_dir: &Path) -> String {
    get_relative_path(path, base_dir)
        .components()
        .find_map(|c| {
            if let std::path::Component::Normal(s) = c {
                Some(s.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}
