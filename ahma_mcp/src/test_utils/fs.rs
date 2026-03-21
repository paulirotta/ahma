use crate::client::Client;
use anyhow::Result;
use std::path::{Path, PathBuf};
use tempfile::{TempDir, tempdir};

/// Get the workspace directory for tests
#[allow(dead_code)]
pub fn get_workspace_dir() -> PathBuf {
    // In a workspace, CARGO_MANIFEST_DIR points to the crate directory (ahma_mcp)
    // We need to go up one level to get to the workspace root where test-data lives
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Failed to get workspace root")
        .to_path_buf()
}

/// Get a path relative to the workspace root
#[allow(dead_code)]
pub fn get_workspace_path<P: AsRef<Path>>(relative: P) -> PathBuf {
    get_workspace_dir().join(relative)
}

/// Get the `.ahma` directory path
#[allow(dead_code)]
pub fn get_tools_dir() -> PathBuf {
    get_workspace_path(".ahma")
}

/// Get the absolute path to the workspace tools directory
#[allow(dead_code)]
pub fn get_workspace_tools_dir() -> std::path::PathBuf {
    get_workspace_path(".ahma")
}

/// Verify that a path exists and is a file
pub async fn file_exists(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .map(|m| m.is_file())
        .unwrap_or(false)
}

/// Verify that a path exists and is a directory
pub async fn dir_exists(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false)
}

/// Read a file and return its contents as a string
pub async fn read_file_contents(path: &Path) -> Result<String> {
    Ok(tokio::fs::read_to_string(path).await?)
}

/// Write contents to a file
pub async fn write_file_contents(path: &Path, contents: &str) -> Result<()> {
    Ok(tokio::fs::write(path, contents).await?)
}

/// Create a temporary directory with tool configs for testing
pub async fn create_temp_tools_dir() -> Result<(TempDir, Client)> {
    let temp_dir = tempdir()?;
    let tools_dir = temp_dir.path().join("tools");
    tokio::fs::create_dir_all(&tools_dir).await?;

    let mut client = Client::new();
    client
        .start_process_with_args(Some(tools_dir.to_str().unwrap()), &["--disable-sandbox"])
        .await?;

    Ok((temp_dir, client))
}
