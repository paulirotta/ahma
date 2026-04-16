//! External analyzer abstraction for language-specific complexity tools.
//!
//! Provides the [`ExternalAnalyzer`] trait and [`AnalyzerRegistry`] for
//! plugging in tool-specific analyzers (Detekt for Kotlin, SwiftLint for Swift,
//! etc.) alongside the built-in rust-code-analysis metrics.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::models::Language;

/// Severity of an issue reported by an external analyzer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// A single issue reported by an external analyzer for a specific location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIssue {
    /// The rule or check ID (e.g., `"CyclomaticComplexMethod"`).
    pub rule: String,
    /// Issue severity.
    pub severity: Severity,
    /// Human-readable description.
    pub message: String,
    /// Name of the function/method containing the issue, when resolvable.
    pub function_name: Option<String>,
    /// 1-based line number of the issue.
    pub start_line: u32,
    /// Numeric complexity value extracted from the message, when present.
    pub complexity_value: Option<f64>,
}

/// Per-file aggregate metrics produced by an external analyzer.
///
/// All numeric fields are `Option` because not every tool reports every metric.
/// Multiple analyzers may contribute to the same file; call [`merge_into`] to
/// accumulate results.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalMetrics {
    /// Total cognitive complexity (sum across all reported functions in this file).
    pub cognitive: Option<f64>,
    /// Total cyclomatic complexity (sum across all reported functions in this file).
    pub cyclomatic: Option<f64>,
    /// Individual issues — used for hotspot detail in AI prompts.
    pub issues: Vec<ExternalIssue>,
    /// Comma-separated names of the analyzer(s) that produced these metrics.
    pub analyzer: String,
}

/// Trait for language-specific external complexity analyzers.
///
/// Implementations wrap tools like Detekt (Kotlin) or SwiftLint (Swift).
/// An external analyzer runs **once per project directory** and returns
/// per-file metrics for all files it analyzed.
pub trait ExternalAnalyzer: Send + Sync {
    /// Short human-readable name used in log messages and report output.
    fn name(&self) -> &'static str;

    /// Returns true if this analyzer handles files of the given language.
    fn supports_language(&self, language: Language) -> bool;

    /// Returns true if this analyzer is runnable for `project_dir`.
    ///
    /// This check must be cheap (e.g., path existence, file content scan).
    /// It must not run the actual analysis.
    fn is_available(&self, project_dir: &Path) -> bool;

    /// Returns a human-readable setup hint to display when the analyzer is
    /// relevant (the project contains supported language files) but
    /// [`is_available`] returns `false`.
    ///
    /// Return `None` (default) to keep the generic skip message.
    fn setup_hint(&self, _project_dir: &Path) -> Option<String> {
        None
    }

    /// Run the full analysis and return a map of absolute file path → metrics.
    ///
    /// The analyzer is responsible for discovering which files to analyze
    /// within `project_dir`. Only Kotlin (or the relevant language) files
    /// will be expected in the result.
    fn analyze(&self, project_dir: &Path) -> Result<HashMap<PathBuf, ExternalMetrics>>;
}

/// Registry of external analyzers. Dispatches to registered analyzers
/// based on which languages appear in the project being analyzed.
#[derive(Default)]
pub struct AnalyzerRegistry {
    analyzers: Vec<Box<dyn ExternalAnalyzer>>,
}

impl AnalyzerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an analyzer. Analyzers are attempted in registration order.
    pub fn register(&mut self, analyzer: Box<dyn ExternalAnalyzer>) {
        self.analyzers.push(analyzer);
    }

    /// Run all registered analyzers that support at least one language in
    /// `languages_present` and are available for `project_dir`.
    ///
    /// Results from multiple analyzers covering the same file are merged
    /// by taking the maximum of each numeric metric (conservative approach:
    /// the worse of the two estimates is used for scoring).
    pub fn run_for_project(
        &self,
        project_dir: &Path,
        languages_present: &HashSet<Language>,
    ) -> HashMap<PathBuf, ExternalMetrics> {
        let mut all_metrics: HashMap<PathBuf, ExternalMetrics> = HashMap::new();

        for analyzer in &self.analyzers {
            let is_relevant = languages_present
                .iter()
                .any(|l| analyzer.supports_language(*l));
            if !is_relevant {
                continue;
            }

            if !analyzer.is_available(project_dir) {
                if let Some(hint) = analyzer.setup_hint(project_dir) {
                    eprintln!("  [{}] {}", analyzer.name(), hint);
                } else {
                    eprintln!(
                        "  [{}] not available for {} — skipping.",
                        analyzer.name(),
                        project_dir.display()
                    );
                }
                continue;
            }

            eprintln!(
                "  Running {} on {}...",
                analyzer.name(),
                project_dir.display()
            );

            match analyzer.analyze(project_dir) {
                Ok(file_metrics) => {
                    eprintln!(
                        "  {} produced metrics for {} files.",
                        analyzer.name(),
                        file_metrics.len()
                    );
                    for (path, m) in file_metrics {
                        merge_into(all_metrics.entry(path).or_default(), m);
                    }
                }
                Err(e) => {
                    eprintln!("  Warning: {} failed: {:#}", analyzer.name(), e);
                }
            }
        }

        all_metrics
    }
}

/// Merge `src` into `target`.
///
/// Numeric fields take the maximum (more conservative / higher complexity wins).
/// Issues lists are concatenated.
pub fn merge_into(target: &mut ExternalMetrics, src: ExternalMetrics) {
    target.cognitive = max_option(target.cognitive, src.cognitive);
    target.cyclomatic = max_option(target.cyclomatic, src.cyclomatic);
    target.issues.extend(src.issues);
    if target.analyzer.is_empty() {
        target.analyzer = src.analyzer;
    } else if !src.analyzer.is_empty() && !target.analyzer.contains(&src.analyzer) {
        target.analyzer.push_str(", ");
        target.analyzer.push_str(&src.analyzer);
    }
}

fn max_option(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(av), Some(bv)) => Some(av.max(bv)),
        (Some(av), None) => Some(av),
        (None, Some(bv)) => Some(bv),
        (None, None) => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopAnalyzer {
        lang: Language,
    }

    impl ExternalAnalyzer for NoopAnalyzer {
        fn name(&self) -> &'static str {
            "noop"
        }
        fn supports_language(&self, language: Language) -> bool {
            language == self.lang
        }
        fn is_available(&self, _project_dir: &Path) -> bool {
            true
        }
        fn analyze(&self, _project_dir: &Path) -> Result<HashMap<PathBuf, ExternalMetrics>> {
            let mut map = HashMap::new();
            map.insert(
                PathBuf::from("/project/src/Foo.kt"),
                ExternalMetrics {
                    cognitive: Some(15.0),
                    cyclomatic: Some(8.0),
                    issues: vec![],
                    analyzer: "noop".to_string(),
                },
            );
            Ok(map)
        }
    }

    #[test]
    fn test_registry_runs_matching_analyzer() {
        let mut registry = AnalyzerRegistry::new();
        registry.register(Box::new(NoopAnalyzer {
            lang: Language::Kotlin,
        }));

        let mut langs = HashSet::new();
        langs.insert(Language::Kotlin);

        let result = registry.run_for_project(Path::new("/project"), &langs);
        assert_eq!(result.len(), 1);
        let m = &result[&PathBuf::from("/project/src/Foo.kt")];
        assert_eq!(m.cognitive, Some(15.0));
        assert_eq!(m.cyclomatic, Some(8.0));
    }

    #[test]
    fn test_registry_skips_non_matching_language() {
        let mut registry = AnalyzerRegistry::new();
        registry.register(Box::new(NoopAnalyzer {
            lang: Language::Kotlin,
        }));

        let mut langs = HashSet::new();
        langs.insert(Language::Rust); // Kotlin analyzer should be skipped

        let result = registry.run_for_project(Path::new("/project"), &langs);
        assert!(result.is_empty());
    }

    #[test]
    fn test_merge_into_takes_max() {
        let mut target = ExternalMetrics {
            cognitive: Some(10.0),
            cyclomatic: Some(5.0),
            issues: vec![],
            analyzer: "a".to_string(),
        };
        let src = ExternalMetrics {
            cognitive: Some(20.0),
            cyclomatic: Some(3.0),
            issues: vec![],
            analyzer: "b".to_string(),
        };
        merge_into(&mut target, src);
        assert_eq!(target.cognitive, Some(20.0)); // max
        assert_eq!(target.cyclomatic, Some(5.0)); // max
        assert!(target.analyzer.contains("a") && target.analyzer.contains("b"));
    }

    #[test]
    fn test_merge_into_none_handling() {
        let mut target = ExternalMetrics::default();
        let src = ExternalMetrics {
            cognitive: Some(7.0),
            cyclomatic: None,
            issues: vec![],
            analyzer: "x".to_string(),
        };
        merge_into(&mut target, src);
        assert_eq!(target.cognitive, Some(7.0));
        assert_eq!(target.cyclomatic, None);
    }
}
