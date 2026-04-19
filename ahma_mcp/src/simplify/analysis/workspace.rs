use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Workspace / project discovery
// ---------------------------------------------------------------------------

/// Resolve which directories to analyse: workspace members or just the root.
pub(crate) fn workspace_analysis_dirs(
    directory: &Path,
    is_workspace: bool,
) -> Result<Vec<PathBuf>> {
    if is_workspace {
        let dirs: Vec<_> = get_workspace_members(directory)?
            .into_iter()
            .map(|m| directory.join(m))
            .filter(|p| p.is_dir())
            .collect();
        if !dirs.is_empty() {
            return Ok(dirs);
        }
    }
    Ok(vec![directory.to_path_buf()])
}

/// Parse `[workspace] members` from a Cargo.toml string.
fn parse_workspace_members(content: &str) -> Option<Vec<String>> {
    let value: toml::Value = content.parse().ok()?;
    let members = value.get("workspace")?.get("members")?.as_array()?;
    let names: Vec<String> = members
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    (!names.is_empty()).then_some(names)
}

/// Detect workspace members from Cargo.toml `[workspace] members`, or fall
/// back to discovering subdirectories that contain a Cargo.toml.
fn get_workspace_members(dir: &Path) -> Result<Vec<String>> {
    let explicit = fs::read_to_string(dir.join("Cargo.toml"))
        .ok()
        .and_then(|c| parse_workspace_members(&c));
    Ok(explicit.unwrap_or_else(|| discover_member_directories(dir)))
}

/// Returns true if `path` looks like a Cargo workspace member directory.
fn is_workspace_member(path: &std::path::Path, name: &str) -> bool {
    path.is_dir() && path.join("Cargo.toml").exists() && name != "target" && !name.starts_with('.')
}

/// Fallback: discover subdirectories that contain a Cargo.toml.
fn discover_member_directories(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            is_workspace_member(&path, name).then(|| name.to_string())
        })
        .collect()
}

pub fn get_project_name(dir: &Path) -> String {
    let cargo_toml = dir.join("Cargo.toml");
    if let Ok(content) = fs::read_to_string(cargo_toml)
        && let Ok(value) = content.parse::<toml::Value>()
        && let Some(name) = value
            .get("package")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
    {
        return name.to_string();
    }
    dir.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn is_cargo_workspace(dir: &Path) -> bool {
    fs::read_to_string(dir.join("Cargo.toml"))
        .map(|content| content.contains("[workspace]"))
        .unwrap_or(false)
}
