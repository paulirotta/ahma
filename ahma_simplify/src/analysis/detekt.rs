//! Detekt integration for Kotlin complexity analysis.
//!
//! Runs `./gradlew detektMain` (falling back to `./gradlew detekt`) in the
//! project directory and parses the Checkstyle-format XML report that Detekt
//! produces at `build/reports/detekt/`.
//!
//! # Requirements
//!
//! - The project must contain a `gradlew` or `gradlew.bat` wrapper.
//! - The project must have the Detekt Gradle plugin configured (detected by
//!   scanning `build.gradle.kts` / `build.gradle` for the word `"detekt"`).
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
use crate::models::Language;

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

/// Heuristic: scan likely Gradle build files for the word "detekt".
fn project_has_detekt(project_dir: &Path) -> bool {
    let candidates = [
        "build.gradle.kts",
        "build.gradle",
        "settings.gradle.kts",
        "settings.gradle",
        // Common Android monorepo layout
        "app/build.gradle.kts",
        "app/build.gradle",
    ];

    for name in &candidates {
        if let Ok(content) = std::fs::read_to_string(project_dir.join(name))
            && content.contains("detekt") {
                return true;
            }
    }
    false
}

/// Run a Gradle Detekt task and return per-file metrics.
fn run_detekt_task(project_dir: &Path, task: &str) -> Result<HashMap<PathBuf, ExternalMetrics>> {
    let gradlew = gradlew_path(project_dir);
    eprintln!("  [detekt] running {} {}...", gradlew.display(), task);

    // --no-daemon prevents leaving a Gradle daemon running between analysis
    // sessions. Detekt exits non-zero when it finds violations, which is
    // expected — do not treat that as a fatal spawn error.
    let output = Command::new(&gradlew)
        .args([task, "--no-daemon"])
        .current_dir(project_dir)
        .output()
        .with_context(|| format!("Failed to spawn {} {}", gradlew.display(), task))?;

    // Distinguish "task not found" from "task ran but found violations".
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{stdout}{stderr}");
        // Gradle reports missing tasks in both stdout and stderr.
        if combined.contains(&format!("Task '{task}' not found"))
            || combined.contains("Could not determine the dependencies")
        {
            anyhow::bail!("Gradle task '{}' not found in project", task);
        }
        // Any other failure: try to parse whatever report exists.
        eprintln!(
            "  [detekt] task '{task}' exited with status {} (violations are expected)",
            output.status
        );
    }

    // Find the XML report. Convention:
    //   detektMain → build/reports/detekt/main.xml
    //   detekt     → build/reports/detekt/detekt.xml
    let report_path = locate_xml_report(project_dir, task)?;
    parse_checkstyle_xml(&report_path)
}

/// Derive the conventional XML report path for a Detekt Gradle task name.
fn locate_xml_report(project_dir: &Path, task: &str) -> Result<PathBuf> {
    let reports_dir = project_dir.join("build/reports/detekt");

    // Strip the "detekt" prefix to get the source-set suffix ("Main", "Test", …).
    let suffix = task.strip_prefix("detekt").unwrap_or("").to_lowercase();
    let report_name = if suffix.is_empty() {
        "detekt.xml".to_string()
    } else {
        format!("{suffix}.xml")
    };

    let primary = reports_dir.join(&report_name);
    if primary.exists() {
        return Ok(primary);
    }

    // Fallback: look for any .xml file in the detekt reports directory.
    let fallback = std::fs::read_dir(&reports_dir)
        .with_context(|| {
            format!(
                "Detekt report directory '{}' not found. \
                 Ensure the detekt Gradle plugin is applied and the task ran successfully.",
                reports_dir.display()
            )
        })?
        .filter_map(|e| e.ok())
        .find(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
        })
        .map(|e| e.path())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No XML report found in '{}'. \
                 Run Gradle with `--report xml` or check your detekt configuration.",
                reports_dir.display()
            )
        })?;

    Ok(fallback)
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
            Ok(Event::Start(ref e)) => {
                if e.name().as_ref() == b"file" {
                    current_file = get_attr(e, b"name").map(PathBuf::from);
                    if let Some(ref p) = current_file {
                        metrics.entry(p.clone()).or_insert_with(|| ExternalMetrics {
                            analyzer: "detekt".to_string(),
                            ..ExternalMetrics::default()
                        });
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == b"error"
                    && let Some(ref file_path) = current_file
                        && let Some(issue) = parse_error_element(e) {
                            let entry = metrics.entry(file_path.clone()).or_insert_with(|| {
                                ExternalMetrics {
                                    analyzer: "detekt".to_string(),
                                    ..ExternalMetrics::default()
                                }
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
            && let Ok(v) = num.parse::<f64>() {
                return Some(v);
            }
    }

    None
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
}
