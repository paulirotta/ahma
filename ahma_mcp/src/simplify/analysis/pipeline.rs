use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::conversion::analyze_file;
use super::exclusion::should_exclude;
use super::external::{AnalyzerRegistry, ExternalMetrics};
use super::workspace::workspace_analysis_dirs;
use crate::simplify::models::Language;

// ---------------------------------------------------------------------------
// Public analysis API (drop-in replacement for the old CLI-based version)
// ---------------------------------------------------------------------------

/// Analyses all source files under `dir`, writes per-file TOML metric results
/// into `output_dir`, and optionally runs external analyzers via `registry`.
///
/// Returns a map of absolute file path → external metrics for any files that
/// were covered by an external analyzer.  The map is empty when `registry` is
/// `None` or no analyzers are available.
pub fn run_analysis(
    dir: &Path,
    output_dir: &Path,
    extensions: &[String],
    custom_excludes: &[String],
    registry: Option<&AnalyzerRegistry>,
) -> Result<HashMap<PathBuf, ExternalMetrics>> {
    eprintln!("Analyzing {}...", dir.display());

    let allowed_exts: HashSet<&str> = extensions
        .iter()
        .map(|e| e.trim_start_matches('.'))
        .collect();

    // Run rca analysis and collect the set of languages seen.
    let mut languages_present: HashSet<Language> = HashSet::new();
    let mut analyzed_count = 0usize;
    let count = source_files(dir, &allowed_exts, custom_excludes).try_fold(
        0usize,
        |count, path| -> Result<usize> {
            if let Some(lang) = file_language(&path) {
                languages_present.insert(lang);
            }
            let had_metrics = write_metrics_toml(&path, dir, output_dir)?;
            if had_metrics {
                analyzed_count += 1;
            }
            Ok(count + 1)
        },
    )?;

    eprintln!("  Analyzed {count} files ({analyzed_count} with metrics).");

    // Run external analyzers when a registry is provided.
    let external = match registry {
        Some(reg) if !languages_present.is_empty() => reg.run_for_project(dir, &languages_present),
        _ => HashMap::new(),
    };

    Ok(external)
}

/// Check if a file matches the extension filter and is not excluded.
fn is_matching_source_file(
    path: &Path,
    allowed_exts: &HashSet<&str>,
    custom_excludes: &[String],
) -> bool {
    if should_exclude(path, custom_excludes) {
        return false;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            allowed_exts.is_empty() || allowed_exts.contains(ext.to_lowercase().as_str())
        })
}

/// Detect the language of a file from its extension.
fn file_language(path: &Path) -> Option<Language> {
    let ext = path.extension()?.to_str()?;
    match ext.to_lowercase().as_str() {
        "kt" | "kts" => Some(Language::Kotlin),
        "rs" => Some(Language::Rust),
        "java" => Some(Language::Java),
        "py" => Some(Language::Python),
        "js" | "mjs" | "cjs" => Some(Language::JavaScript),
        "ts" | "tsx" => Some(Language::TypeScript),
        "swift" => Some(Language::Swift),
        "go" => Some(Language::Go),
        "cpp" | "cc" | "cxx" => Some(Language::Cpp),
        "c" | "h" => Some(Language::C),
        "cs" => Some(Language::CSharp),
        _ => None,
    }
}

/// Iterate source files in `dir` matching extension and exclusion filters.
fn source_files<'a>(
    dir: &'a Path,
    allowed_exts: &'a std::collections::HashSet<&'a str>,
    custom_excludes: &'a [String],
) -> impl Iterator<Item = PathBuf> + 'a {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(move |path| is_matching_source_file(path, allowed_exts, custom_excludes))
}

/// Ensure the parent directory of `path` exists.
fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    Ok(())
}

/// Analyse a single file and write its metrics as TOML into `output_dir`.
/// Returns `true` if metrics were produced (rca could analyze the file),
/// `false` if the file was iterated but rca had no metrics for it.
fn write_metrics_toml(path: &Path, dir: &Path, output_dir: &Path) -> Result<bool> {
    let Some(results) = analyze_file(path) else {
        return Ok(false);
    };
    let toml_content = toml::to_string(&results).context("Failed to serialize metrics to TOML")?;
    let relative = path.strip_prefix(dir).unwrap_or(path);
    let toml_path = output_dir.join(relative.with_extension("toml"));
    ensure_parent_dir(&toml_path)?;
    fs::write(&toml_path, toml_content)
        .with_context(|| format!("Failed to write {}", toml_path.display()))?;
    Ok(true)
}

pub fn perform_analysis(
    directory: &Path,
    output: &Path,
    is_workspace: bool,
    extensions: &[String],
    custom_excludes: &[String],
    registry: Option<&AnalyzerRegistry>,
) -> Result<HashMap<PathBuf, ExternalMetrics>> {
    let dirs = workspace_analysis_dirs(directory, is_workspace)?;
    let mut all_external: HashMap<PathBuf, ExternalMetrics> = HashMap::new();
    for dir in &dirs {
        let external = run_analysis(dir, output, extensions, custom_excludes, registry)?;
        all_external.extend(external);
    }
    Ok(all_external)
}
