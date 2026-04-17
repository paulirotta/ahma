use super::fs::get_workspace_dir;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

/// Cached binary paths to avoid redundant builds across tests.
/// Key: (package, binary) tuple as string "package:binary"
static BINARY_CACHE: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

/// Get the path to a binary in the target directory, resolving CARGO_TARGET_DIR correctly.
///
/// This function handles relative `CARGO_TARGET_DIR` paths (e.g., `target`) by resolving
/// them relative to the workspace root. This is critical for CI environments that set
/// `CARGO_TARGET_DIR` to a relative path.
///
/// Does NOT build the binary - caller is responsible for ensuring it exists.
/// For automatic building with caching, use `build_binary_cached()` instead.
pub fn get_binary_path(_package: &str, binary: &str) -> PathBuf {
    let workspace = get_workspace_dir();
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|p| {
            if p.is_absolute() {
                p
            } else {
                workspace.join(p)
            }
        })
        .unwrap_or_else(|_| workspace.join("target"));

    // On Windows, executables have a `.exe` extension; std::env::consts::EXE_EXTENSION
    // is "exe" on Windows and "" on Unix, so appending it is always safe.
    let mut name = binary.to_string();
    if !std::env::consts::EXE_EXTENSION.is_empty() {
        name.push('.');
        name.push_str(std::env::consts::EXE_EXTENSION);
    }

    target_dir.join("debug").join(name)
}

/// Get or build a binary, caching the result.
///
/// This function is optimized for test performance:
/// 1. First checks if the binary already exists (common when running via cargo test/nextest)
/// 2. Only builds if the binary doesn't exist
/// 3. Caches the path to avoid redundant filesystem checks
pub fn build_binary_cached(package: &str, binary: &str) -> PathBuf {
    let cache = BINARY_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{}:{}", package, binary);

    // Fast path: check cache first
    {
        let cache_guard = cache.lock().unwrap();
        if let Some(path) = cache_guard.get(&key) {
            return path.clone();
        }
    }

    let binary_path = get_binary_path(package, binary);

    if binary_path.exists() {
        let mut cache_guard = cache.lock().unwrap();
        cache_guard.insert(key, binary_path.clone());
        return binary_path;
    }

    let workspace = get_workspace_dir();
    let output = Command::new("cargo")
        .current_dir(&workspace)
        .args(["build", "--package", package, "--bin", binary])
        .output()
        .expect("Failed to run cargo build");

    assert!(
        output.status.success(),
        "Failed to build {}: {}",
        binary,
        String::from_utf8_lossy(&output.stderr)
    );

    let mut cache_guard = cache.lock().unwrap();
    cache_guard.insert(key, binary_path.clone());

    binary_path
}

/// Create a command for a binary with test mode enabled (bypasses sandbox checks).
/// Disables the sandbox and skips probes via environment variables.
/// The caller must add the appropriate subcommand (e.g., `serve stdio`, `run`, `tool list`).
pub fn test_command(binary: &Path) -> Command {
    let mut cmd = Command::new(binary);
    cmd.env("AHMA_DISABLE_SANDBOX", "1");
    cmd.env("AHMA_SKIP_PROBES", "1");
    cmd
}
