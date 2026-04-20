//! Detekt integration for Kotlin complexity analysis.
//!
//! Runs `./gradlew detektMain` (falling back to `./gradlew detekt`) in the
//! project directory and parses the Checkstyle-format XML report that Detekt
//! produces at `build/reports/detekt/`.
//!
//! # Multi-module Android projects
//!
//! Standard Android projects place reports in per-module directories
//! (`app/build/reports/detekt/`, `lib/build/reports/detekt/`, etc.).  The
//! analyzer walks all immediate subdirectories looking for these report dirs
//! and merges all discovered XML files into a single metrics map.
//!
//! # Requirements
//!
//! - The project must contain a `gradlew` or `gradlew.bat` wrapper.
//! - The project must have the Detekt Gradle plugin configured (detected by
//!   scanning root and module-level `build.gradle.kts` / `build.gradle` for
//!   the word `"detekt"`).
//!
//! # Graceful degradation
//!
//! If Gradle or Detekt is unavailable, or the task fails, the analyzer logs a
//! warning and returns empty metrics — the rca analysis continues unaffected.

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::external::{ExternalAnalyzer, ExternalIssue, ExternalMetrics, Severity};
use crate::simplify::models::Language;

// ---------------------------------------------------------------------------
// Public analyzer struct
// ---------------------------------------------------------------------------

/// Analyzer that invokes `./gradlew detektMain` (or `./gradlew detekt`) and
/// parses the resulting Checkstyle XML report.
pub struct DetektAnalyzer;

impl ExternalAnalyzer for DetektAnalyzer {
    fn name(&self) -> &'static str {
        "detekt"
    }

    fn supports_language(&self, language: Language) -> bool {
        matches!(language, Language::Kotlin)
    }

    fn is_available(&self, project_dir: &Path) -> bool {
        gradlew_path(project_dir).exists() && project_has_detekt(project_dir)
    }

    fn setup_hint(&self, project_dir: &Path) -> Option<String> {
        if !gradlew_path(project_dir).exists() {
            return Some(
                "Kotlin files found but no Gradle wrapper (gradlew) detected. \
                 Run `gradle wrapper` in your project root, then add the detekt plugin \
                 to your build script."
                    .to_string(),
            );
        }
        if !project_has_detekt(project_dir) {
            return Some(
                "Kotlin files found but detekt is not configured. \
                 Add the plugin to your build.gradle.kts:\n\
                 \n\
                 \x20 plugins { id(\"io.gitlab.arturbosch.detekt\") version \"1.23.7\" }\n\
                 \n\
                 See https://detekt.dev/docs/gettingstarted/gradle/"
                    .to_string(),
            );
        }
        None
    }

    fn analyze(&self, project_dir: &Path) -> Result<HashMap<PathBuf, ExternalMetrics>> {
        // Try the source-set-specific task first, then the root task.
        let candidate_tasks = ["detektMain", "detekt"];

        for task in &candidate_tasks {
            match run_detekt_task(project_dir, task) {
                Ok(metrics) => return Ok(metrics),
                Err(e) => {
                    eprintln!("  [detekt] task '{task}' failed: {e:#}");
                }
            }
        }

        // All tasks failed — return empty so the caller can continue.
        Ok(HashMap::new())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Platform-appropriate path to the Gradle wrapper.
fn gradlew_path(project_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        project_dir.join("gradlew.bat")
    } else {
        project_dir.join("gradlew")
    }
}

/// Return `true` if the file at `path` exists and contains the word "detekt".
fn file_contains_detekt(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|c| c.contains("detekt"))
}

/// Heuristic: scan root and module-level Gradle build files for the word "detekt".
///
/// Checks root build files first, then all immediate subdirectory build files
/// (common Android convention: detekt may be applied per-module).
fn project_has_detekt(project_dir: &Path) -> bool {
    // Root-level candidates
    let root_candidates = [
        "build.gradle.kts",
        "build.gradle",
        "settings.gradle.kts",
        "settings.gradle",
        // buildSrc / build-logic convention plugins
        "buildSrc/src/main/kotlin/detekt.gradle.kts",
        "build-logic/src/main/kotlin/detekt.gradle.kts",
    ];

    for name in &root_candidates {
        if file_contains_detekt(&project_dir.join(name)) {
            return true;
        }
    }

    // Scan immediate subdirectories (modules) for build.gradle.kts / build.gradle
    let Ok(entries) = std::fs::read_dir(project_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        for build_file in &["build.gradle.kts", "build.gradle"] {
            if file_contains_detekt(&path.join(build_file)) {
                return true;
            }
        }
    }

    false
}

/// Return `true` if `output` indicates that `task` was not found in the Gradle project.
fn is_task_not_found_error(output: &std::process::Output, task: &str) -> bool {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    combined.contains(&format!("Task '{task}' not found"))
        || combined.contains("Could not determine the dependencies")
}

/// Parse each path with `parse_fn`, merging results into `all_metrics`.
///
/// Errors are logged but do not abort the merge.
fn merge_reports_into(
    paths: &[PathBuf],
    all_metrics: &mut HashMap<PathBuf, ExternalMetrics>,
    label: &str,
    mut parse_fn: impl FnMut(&Path) -> Result<HashMap<PathBuf, ExternalMetrics>>,
) {
    for path in paths {
        match parse_fn(path) {
            Ok(file_metrics) => {
                for (p, m) in file_metrics {
                    super::external::merge_into(all_metrics.entry(p).or_default(), m);
                }
            }
            Err(e) => eprintln!(
                "  [detekt] failed to parse {label}{}: {e:#}",
                path.display()
            ),
        }
    }
}

/// Run a Gradle Detekt task and return per-file metrics aggregated from all
/// module report directories.
fn run_detekt_task(project_dir: &Path, task: &str) -> Result<HashMap<PathBuf, ExternalMetrics>> {
    let gradlew = gradlew_path(project_dir);
    eprintln!("  [detekt] running {} {}...", gradlew.display(), task);

    // --no-daemon: prevent orphaned Gradle daemons between analysis sessions.
    // --continue:  run all modules even if one fails, so we collect reports
    //              from every module instead of stopping at the first violation.
    // Detekt exits non-zero when it finds violations — do not treat as fatal.
    let output = Command::new(&gradlew)
        .args([task, "--no-daemon", "--continue"])
        .current_dir(project_dir)
        .output()
        .with_context(|| format!("Failed to spawn {} {}", gradlew.display(), task))?;

    if !output.status.success() {
        if is_task_not_found_error(&output, task) {
            anyhow::bail!("Gradle task '{}' not found in project", task);
        }
        // Any other failure: try to parse whatever reports exist.
        eprintln!(
            "  [detekt] task '{task}' exited with status {} (violations are expected)",
            output.status
        );
    }

    // Collect all XML reports produced across root and module directories,
    // then merge them into a single map.
    let xml_paths = collect_xml_reports(project_dir, task);
    let sarif_paths = collect_sarif_reports(project_dir);
    if xml_paths.is_empty() && sarif_paths.is_empty() {
        anyhow::bail!(
            "Detekt report directory '{}' not found. \
             Ensure the detekt Gradle plugin is applied and the task ran successfully.",
            project_dir.join("build/reports/detekt").display()
        );
    }

    let mut all_metrics: HashMap<PathBuf, ExternalMetrics> = HashMap::new();
    merge_reports_into(&xml_paths, &mut all_metrics, "", |p| {
        parse_checkstyle_xml(p)
    });
    // Also process SARIF reports — some projects configure SARIF for IDE integration.
    merge_reports_into(&sarif_paths, &mut all_metrics, "SARIF ", |p| {
        parse_sarif_report(p, project_dir)
    });
    Ok(all_metrics)
}

/// Build the ordered list of candidate detekt report directories.
///
/// Includes the root-level directory first, followed by one per immediate
/// subdirectory (standard Android multi-module layout).
fn report_candidate_dirs(project_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![project_dir.join("build/reports/detekt")];
    if let Ok(entries) = std::fs::read_dir(project_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path.join("build/reports/detekt"));
            }
        }
    }
    dirs
}

/// Find the best XML report in `reports_dir` for `report_name`.
///
/// Tries `report_name` first; falls back to the first `.xml` file found.
fn find_xml_in_reports_dir(reports_dir: &Path, report_name: &str) -> Option<PathBuf> {
    let primary = reports_dir.join(report_name);
    if primary.is_file() {
        return Some(primary);
    }
    std::fs::read_dir(reports_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")))
}

/// Collect all detekt Checkstyle XML reports for `task` under `project_dir`.
///
/// Checks three locations in priority order:
/// 1. `<project_dir>/build/reports/detekt/` — root-level task (single-module)
/// 2. `<project_dir>/<module>/build/reports/detekt/` for each immediate
///    subdirectory — standard Android multi-module layout
///
/// Within each reports directory the conventional file name for the task is
/// tried first (`main.xml` for `detektMain`, `detekt.xml` for `detekt`), then
/// any `.xml` file is accepted as fallback.
fn collect_xml_reports(project_dir: &Path, task: &str) -> Vec<PathBuf> {
    // Derive the conventional filename from the task name.
    // detektMain → main.xml,  detekt → detekt.xml
    let suffix = task.strip_prefix("detekt").unwrap_or("").to_lowercase();
    let report_name = if suffix.is_empty() {
        "detekt.xml".to_string()
    } else {
        format!("{suffix}.xml")
    };

    report_candidate_dirs(project_dir)
        .into_iter()
        .filter(|d| d.is_dir())
        .filter_map(|d| find_xml_in_reports_dir(&d, &report_name))
        .collect()
}

// ---------------------------------------------------------------------------
// Checkstyle XML parser
// ---------------------------------------------------------------------------

/// Parse a Checkstyle-format XML file produced by Detekt.
///
/// Expected structure:
/// ```xml
/// <?xml version="1.0" encoding="UTF-8"?>
/// <checkstyle version="8.0">
///   <file name="/abs/path/to/File.kt">
///     <error line="10" column="5" severity="warning"
///            message="The function processData has a cyclomatic complexity of 12 (threshold = 1)"
///            source="detekt.complexity.CyclomaticComplexMethod"/>
///   </file>
/// </checkstyle>
/// ```
fn parse_checkstyle_xml(path: &Path) -> Result<HashMap<PathBuf, ExternalMetrics>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read Detekt report '{}'", path.display()))?;

    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(true);

    let mut metrics: HashMap<PathBuf, ExternalMetrics> = HashMap::new();
    let mut current_file: Option<PathBuf> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"file" => {
                current_file = get_attr(e, b"name").map(PathBuf::from);
                if let Some(ref p) = current_file {
                    metrics.entry(p.clone()).or_insert_with(|| ExternalMetrics {
                        analyzer: "detekt".to_string(),
                        ..ExternalMetrics::default()
                    });
                }
            }
            Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == b"error"
                    && let Some(ref file_path) = current_file
                    && let Some(issue) = parse_error_element(e)
                {
                    let entry =
                        metrics
                            .entry(file_path.clone())
                            .or_insert_with(|| ExternalMetrics {
                                analyzer: "detekt".to_string(),
                                ..ExternalMetrics::default()
                            });
                    accumulate_issue(entry, &issue);
                    entry.issues.push(issue);
                }
            }
            Ok(Event::End(ref e)) if e.name().as_ref() == b"file" => {
                current_file = None;
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                eprintln!(
                    "  [detekt] XML parse error at position {}: {e}",
                    reader.error_position()
                );
                break;
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(metrics)
}

/// Extract the value of an XML attribute by name, unescaping entity references.
fn get_attr(e: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.unescape_value().ok())
        .map(|v| v.into_owned())
}

/// Parse a single `<error …/>` element into an [`ExternalIssue`].
fn parse_error_element(e: &quick_xml::events::BytesStart<'_>) -> Option<ExternalIssue> {
    let start_line: u32 = get_attr(e, b"line")?.parse().unwrap_or(0);
    let message = get_attr(e, b"message").unwrap_or_default();
    let source = get_attr(e, b"source").unwrap_or_default();
    let severity_str = get_attr(e, b"severity").unwrap_or_default();

    // Extract the bare rule name from the fully-qualified source ID
    // e.g. "detekt.complexity.CyclomaticComplexMethod" → "CyclomaticComplexMethod"
    let rule = source.rsplit('.').next().unwrap_or(&source).to_string();

    let severity = match severity_str.as_str() {
        "error" => Severity::Error,
        "info" => Severity::Info,
        _ => Severity::Warning,
    };

    Some(ExternalIssue {
        rule,
        severity,
        function_name: extract_function_name(&message),
        complexity_value: extract_complexity_value(&message),
        message,
        start_line,
    })
}

/// Accumulate an `ExternalIssue` into the per-file aggregate metrics.
///
/// Cyclomatic and cognitive totals are summed across all reported functions.
/// When the issue carries a concrete complexity value we use it; otherwise we
/// count the issue as 1 unit (the function exceeded the threshold).
fn accumulate_issue(entry: &mut ExternalMetrics, issue: &ExternalIssue) {
    let increment = issue.complexity_value.unwrap_or(1.0);
    match rule_kind(&issue.rule) {
        RuleKind::Cyclomatic => {
            *entry.cyclomatic.get_or_insert(0.0) += increment;
        }
        RuleKind::Cognitive => {
            *entry.cognitive.get_or_insert(0.0) += increment;
        }
        RuleKind::Other => {}
    }
}

#[derive(Debug)]
enum RuleKind {
    Cyclomatic,
    Cognitive,
    Other,
}

fn rule_kind(rule: &str) -> RuleKind {
    let lower = rule.to_lowercase();
    if lower.contains("cyclomatic") {
        RuleKind::Cyclomatic
    } else if lower.contains("cognitive") {
        RuleKind::Cognitive
    } else {
        RuleKind::Other
    }
}

// ---------------------------------------------------------------------------
// Message parsing helpers
// ---------------------------------------------------------------------------

/// Extract the function name from a Detekt message.
///
/// Handles both plain (`The function processData …`) and
/// backtick-quoted (`The function \`handleResult\` …`) forms.
fn extract_function_name(message: &str) -> Option<String> {
    let prefix = "function ";
    let start = message.find(prefix)?;
    let rest = &message[start + prefix.len()..];

    let name = if let Some(inner) = rest.strip_prefix('`') {
        // Backtick-quoted name: `handleResult`
        let end = inner.find('`')?;
        inner[..end].to_string()
    } else {
        // Plain name terminated by the next whitespace character.
        rest.split_whitespace().next()?.to_string()
    };

    if name.is_empty() { None } else { Some(name) }
}

/// Extract a numeric complexity value from a Detekt message.
///
/// Handles patterns such as:
/// - `"…cyclomatic complexity of 12 (threshold…"` → `12.0`
/// - `"…is too long (85/1)…"` → `85.0`
fn extract_complexity_value(message: &str) -> Option<f64> {
    // Pattern: " of <N>" (used by CyclomaticComplexMethod, CognitiveComplexMethod)
    if let Some(pos) = message.find(" of ") {
        let rest = &message[pos + " of ".len()..];
        let num: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if let Ok(v) = num.parse::<f64>() {
            return Some(v);
        }
    }

    // Pattern: "(<N>/<threshold>)" used by LongMethod and similar rules.
    if let Some(start) = message.rfind('(') {
        let rest = &message[start + 1..];
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !num.is_empty()
            && let Ok(v) = num.parse::<f64>()
        {
            return Some(v);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// SARIF report support
// ---------------------------------------------------------------------------

/// Return all SARIF files in `reports_dir`.
fn sarif_files_in_dir(reports_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(reports_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("sarif"))
        })
        .collect()
}

/// Collect all Detekt SARIF reports under `project_dir`.
///
/// Looks in the same report directories as [`collect_xml_reports`].
fn collect_sarif_reports(project_dir: &Path) -> Vec<PathBuf> {
    report_candidate_dirs(project_dir)
        .into_iter()
        .filter(|d| d.is_dir())
        .flat_map(|d| sarif_files_in_dir(&d))
        .collect()
}

/// Build the `uriBaseId → base path` map from a SARIF run object.
fn sarif_base_paths(run: &serde_json::Value) -> HashMap<String, PathBuf> {
    run.get("originalUriBaseIds")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(key, val)| {
                    let uri = val.get("uri")?.as_str()?;
                    let path_str = uri.strip_prefix("file://").unwrap_or(uri);
                    Some((key.clone(), PathBuf::from(path_str)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve the artifact file path from a SARIF `physicalLocation` object.
fn resolve_sarif_path(
    phys: &serde_json::Value,
    base_paths: &HashMap<String, PathBuf>,
    project_dir: &Path,
) -> Option<PathBuf> {
    let artifact = phys.get("artifactLocation");
    let uri = artifact
        .and_then(|a| a.get("uri"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let uri_base_id = artifact
        .and_then(|a| a.get("uriBaseId"))
        .and_then(|v| v.as_str())
        .unwrap_or("%SRCROOT%");

    let file_path = if let Some(base) = base_paths.get(uri_base_id) {
        base.join(uri)
    } else {
        let raw = uri.strip_prefix("file://").unwrap_or(uri);
        if Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            project_dir.join(raw)
        }
    };
    Some(file_path)
}

/// Extract a single SARIF result into a `(file_path, issue)` pair.
fn parse_sarif_result(
    result: &serde_json::Value,
    base_paths: &HashMap<String, PathBuf>,
    project_dir: &Path,
) -> Option<(PathBuf, ExternalIssue)> {
    let rule_id = result.get("ruleId").and_then(|v| v.as_str()).unwrap_or("");
    // "detekt.complexity/CyclomaticComplexMethod" → "CyclomaticComplexMethod"
    let rule = rule_id.rsplit('/').next().unwrap_or(rule_id).to_string();

    let message = result
        .get("message")
        .and_then(|m| m.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let level_str = result
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("warning");
    let severity = match level_str {
        "error" => Severity::Error,
        "note" => Severity::Info,
        _ => Severity::Warning,
    };

    let phys = result
        .get("locations")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|l| l.get("physicalLocation"))?;

    let start_line: u32 = phys
        .get("region")
        .and_then(|r| r.get("startLine"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let file_path = resolve_sarif_path(phys, base_paths, project_dir)?;

    Some((
        file_path,
        ExternalIssue {
            rule,
            severity,
            function_name: extract_function_name(&message),
            complexity_value: extract_complexity_value(&message),
            message,
            start_line,
        },
    ))
}

/// Parse a SARIF 2.1.0 report produced by Detekt into per-file metrics.
///
/// Uses `serde_json::Value` traversal to avoid a full schema struct, keeping
/// the implementation compact and resilient to schema variations.
fn parse_sarif_report(
    path: &Path,
    project_dir: &Path,
) -> Result<HashMap<PathBuf, ExternalMetrics>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read SARIF report '{}'", path.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("Cannot parse SARIF JSON from '{}'", path.display()))?;

    let mut metrics: HashMap<PathBuf, ExternalMetrics> = HashMap::new();

    let Some(runs) = doc.get("runs").and_then(|v| v.as_array()) else {
        return Ok(metrics);
    };

    for run in runs {
        let base_paths = sarif_base_paths(run);
        let Some(results) = run.get("results").and_then(|v| v.as_array()) else {
            continue;
        };
        for result in results {
            let Some((file_path, issue)) = parse_sarif_result(result, &base_paths, project_dir)
            else {
                continue;
            };
            let entry = metrics.entry(file_path).or_insert_with(|| ExternalMetrics {
                analyzer: "detekt".to_string(),
                ..ExternalMetrics::default()
            });
            accumulate_issue(entry, &issue);
            entry.issues.push(issue);
        }
    }

    Ok(metrics)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal Checkstyle XML produced by Detekt
    const SAMPLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<checkstyle version="8.0">
  <file name="/project/src/main/kotlin/com/example/Manager.kt">
    <error line="15" column="5" severity="warning"
           message="The function processData has a cyclomatic complexity of 12 (threshold = 10)"
           source="detekt.complexity.CyclomaticComplexMethod"/>
    <error line="50" column="5" severity="warning"
           message="The function handleResult has a cognitive complexity of 8 (threshold = 5)"
           source="detekt.complexity.CognitiveComplexMethod"/>
    <error line="80" column="5" severity="warning"
           message="The function buildQuery is too long (85/40)"
           source="detekt.style.LongMethod"/>
  </file>
  <file name="/project/src/main/kotlin/com/example/Service.kt">
    <error line="10" column="1" severity="warning"
           message="The function doWork has a cyclomatic complexity of 5 (threshold = 10)"
           source="detekt.complexity.CyclomaticComplexMethod"/>
  </file>
</checkstyle>"#;

    fn parse_sample() -> HashMap<PathBuf, ExternalMetrics> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), SAMPLE_XML).unwrap();
        parse_checkstyle_xml(tmp.path()).unwrap()
    }

    #[test]
    fn test_parse_finds_both_files() {
        let result = parse_sample();
        assert_eq!(result.len(), 2, "Should find 2 files");
    }

    #[test]
    fn test_parse_aggregates_cyclomatic() {
        let result = parse_sample();
        let manager = &result[&PathBuf::from("/project/src/main/kotlin/com/example/Manager.kt")];
        // CyclomaticComplexMethod reports 12; LongMethod is "Other" → not counted
        assert_eq!(manager.cyclomatic, Some(12.0));
    }

    #[test]
    fn test_parse_aggregates_cognitive() {
        let result = parse_sample();
        let manager = &result[&PathBuf::from("/project/src/main/kotlin/com/example/Manager.kt")];
        assert_eq!(manager.cognitive, Some(8.0));
    }

    #[test]
    fn test_parse_issue_count() {
        let result = parse_sample();
        let manager = &result[&PathBuf::from("/project/src/main/kotlin/com/example/Manager.kt")];
        assert_eq!(manager.issues.len(), 3, "Manager.kt should have 3 issues");
    }

    #[test]
    fn test_parse_function_name() {
        let result = parse_sample();
        let manager = &result[&PathBuf::from("/project/src/main/kotlin/com/example/Manager.kt")];
        let first = &manager.issues[0];
        assert_eq!(first.function_name.as_deref(), Some("processData"));
    }

    #[test]
    fn test_parse_complexity_value() {
        let result = parse_sample();
        let manager = &result[&PathBuf::from("/project/src/main/kotlin/com/example/Manager.kt")];
        assert_eq!(manager.issues[0].complexity_value, Some(12.0));
        assert_eq!(manager.issues[1].complexity_value, Some(8.0));
        // LongMethod uses (85/40) pattern
        assert_eq!(manager.issues[2].complexity_value, Some(85.0));
    }

    #[test]
    fn test_parse_single_file_cyclomatic() {
        let result = parse_sample();
        let service = &result[&PathBuf::from("/project/src/main/kotlin/com/example/Service.kt")];
        assert_eq!(service.cyclomatic, Some(5.0));
        assert_eq!(service.cognitive, None);
    }

    #[test]
    fn test_extract_function_name_plain() {
        assert_eq!(
            extract_function_name("The function processData has a cyclomatic complexity of 12"),
            Some("processData".to_string())
        );
    }

    #[test]
    fn test_extract_function_name_backtick() {
        assert_eq!(
            extract_function_name("The function `handleResult` is too long"),
            Some("handleResult".to_string())
        );
    }

    #[test]
    fn test_extract_complexity_value_of_pattern() {
        assert_eq!(
            extract_complexity_value("complexity of 12 (threshold = 10)"),
            Some(12.0)
        );
    }

    #[test]
    fn test_extract_complexity_value_slash_pattern() {
        assert_eq!(extract_complexity_value("is too long (85/40)"), Some(85.0));
    }

    #[test]
    fn test_detekt_supports_kotlin_only() {
        let analyzer = DetektAnalyzer;
        assert!(analyzer.supports_language(Language::Kotlin));
        assert!(!analyzer.supports_language(Language::Rust));
        assert!(!analyzer.supports_language(Language::Java));
    }

    // --- SARIF parser tests ---

    const SAMPLE_SARIF: &str = r#"{
  "$schema": "https://schemastore.azurewebsites.net/schemas/json/sarif-2.1.0-rtm.5.json",
  "version": "2.1.0",
  "runs": [{
    "tool": { "driver": { "name": "detekt", "version": "1.23.7", "rules": [] } },
    "originalUriBaseIds": {
      "%SRCROOT%": { "uri": "file:///project/" }
    },
    "results": [
      {
        "ruleId": "detekt.complexity/CyclomaticComplexMethod",
        "level": "warning",
        "message": { "text": "The function processData has a cyclomatic complexity of 12 (threshold = 10)." },
        "locations": [{
          "physicalLocation": {
            "artifactLocation": { "uri": "src/main/kotlin/Manager.kt", "uriBaseId": "%SRCROOT%" },
            "region": { "startLine": 15 }
          }
        }]
      },
      {
        "ruleId": "detekt.complexity/CognitiveComplexMethod",
        "level": "warning",
        "message": { "text": "The function handleResult has a cognitive complexity of 8 (threshold = 5)." },
        "locations": [{
          "physicalLocation": {
            "artifactLocation": { "uri": "src/main/kotlin/Manager.kt", "uriBaseId": "%SRCROOT%" },
            "region": { "startLine": 50 }
          }
        }]
      },
      {
        "ruleId": "detekt.style/LongMethod",
        "level": "warning",
        "message": { "text": "The function buildQuery is too long (85/40)." },
        "locations": [{
          "physicalLocation": {
            "artifactLocation": { "uri": "src/main/kotlin/Manager.kt", "uriBaseId": "%SRCROOT%" },
            "region": { "startLine": 80 }
          }
        }]
      }
    ]
  }]
}"#;

    #[test]
    fn test_sarif_parse_finds_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sarif_path = tmp.path().join("detekt.sarif");
        std::fs::write(&sarif_path, SAMPLE_SARIF).unwrap();
        let project_dir = PathBuf::from("/project");
        let result = parse_sarif_report(&sarif_path, &project_dir).unwrap();
        assert_eq!(result.len(), 1, "should find 1 file from SARIF");
    }

    #[test]
    fn test_sarif_parse_aggregates_complexity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sarif_path = tmp.path().join("detekt.sarif");
        std::fs::write(&sarif_path, SAMPLE_SARIF).unwrap();
        let project_dir = PathBuf::from("/project");
        let result = parse_sarif_report(&sarif_path, &project_dir).unwrap();
        let manager = result.values().next().expect("should have one file entry");
        assert_eq!(manager.cyclomatic, Some(12.0), "cyclomatic from SARIF");
        assert_eq!(manager.cognitive, Some(8.0), "cognitive from SARIF");
        assert_eq!(manager.issues.len(), 3, "all 3 issues captured");
    }

    #[test]
    fn test_sarif_parse_function_names() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sarif_path = tmp.path().join("detekt.sarif");
        std::fs::write(&sarif_path, SAMPLE_SARIF).unwrap();
        let project_dir = PathBuf::from("/project");
        let result = parse_sarif_report(&sarif_path, &project_dir).unwrap();
        let manager = result.values().next().unwrap();
        assert_eq!(
            manager.issues[0].function_name.as_deref(),
            Some("processData")
        );
        assert_eq!(
            manager.issues[1].function_name.as_deref(),
            Some("handleResult")
        );
    }

    // --- setup_hint tests ---

    #[test]
    fn test_setup_hint_no_gradlew() {
        let tmp = tempfile::TempDir::new().unwrap();
        // No gradlew present → first hint
        let hint = DetektAnalyzer.setup_hint(tmp.path());
        assert!(
            hint.is_some(),
            "should return a hint when gradlew is absent"
        );
        let msg = hint.unwrap();
        assert!(
            msg.contains("no Gradle wrapper"),
            "hint should mention missing gradlew: {msg}"
        );
        assert!(
            msg.contains("gradle wrapper"),
            "hint should suggest running `gradle wrapper`: {msg}"
        );
    }

    #[test]
    fn test_setup_hint_gradlew_but_no_detekt() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Create the platform-correct gradlew file but no detekt config
        std::fs::write(gradlew_path(tmp.path()), "#!/bin/sh\n").unwrap();
        let hint = DetektAnalyzer.setup_hint(tmp.path());
        assert!(
            hint.is_some(),
            "should return a hint when detekt is not configured"
        );
        let msg = hint.unwrap();
        assert!(
            msg.contains("detekt is not configured"),
            "hint should say detekt is not configured: {msg}"
        );
        assert!(
            msg.contains("detekt.dev"),
            "hint should include the detekt docs URL: {msg}"
        );
    }

    #[test]
    fn test_setup_hint_none_when_detekt_configured() {
        let tmp = tempfile::TempDir::new().unwrap();
        // gradlew + build.gradle.kts mentioning detekt → available, no hint needed
        std::fs::write(gradlew_path(tmp.path()), "#!/bin/sh\n").unwrap();
        std::fs::write(
            tmp.path().join("build.gradle.kts"),
            r#"plugins { id("io.gitlab.arturbosch.detekt") version "1.23.7" }"#,
        )
        .unwrap();
        let hint = DetektAnalyzer.setup_hint(tmp.path());
        assert!(
            hint.is_none(),
            "should return None when detekt is properly configured"
        );
    }
}
