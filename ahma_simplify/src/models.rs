use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct MetricsResults {
    pub name: String,
    pub metrics: Metrics,
    #[serde(default)]
    pub spaces: Vec<SpaceEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Metrics {
    pub cognitive: Cognitive,
    pub cyclomatic: Cyclomatic,
    pub mi: Mi,
    pub loc: Loc,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Cognitive {
    pub sum: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Cyclomatic {
    pub sum: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Mi {
    pub mi_visual_studio: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Loc {
    pub sloc: f64,
}

/// A parsed entry from the `spaces` array in rust-code-analysis output.
/// Represents a code "space" (function, impl block, closure, etc.) with its metrics.
#[derive(Debug, Deserialize, Serialize)]
pub struct SpaceEntry {
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub kind: String,
    pub metrics: Metrics,
    #[serde(default)]
    pub spaces: Vec<SpaceEntry>,
}

/// A function or method identified as a complexity hotspot within a file.
#[derive(Debug, Clone)]
pub struct FunctionHotspot {
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub cognitive: f64,
    pub cyclomatic: f64,
    pub sloc: f64,
}

fn is_named_function(entry: &SpaceEntry) -> bool {
    entry.kind == "function" && entry.name != "<anonymous>"
}

fn walk_spaces<F>(spaces: &[SpaceEntry], visitor: &mut F)
where
    F: FnMut(&SpaceEntry),
{
    for entry in spaces {
        walk_spaces(&entry.spaces, visitor);
        visitor(entry);
    }
}

impl FunctionHotspot {
    const MAX_HOTSPOTS: usize = 5;

    /// Recursively collects function hotspots from the spaces tree,
    /// sorted by cognitive complexity descending, limited to MAX_HOTSPOTS.
    pub fn extract_from_spaces(spaces: &[SpaceEntry]) -> Vec<FunctionHotspot> {
        let mut hotspots = Vec::new();
        walk_spaces(spaces, &mut |entry| {
            if Self::is_nontrivial_function(entry) {
                hotspots.push(Self::from_space_entry(entry));
            }
        });
        hotspots.sort_by(|a, b| {
            b.cognitive
                .partial_cmp(&a.cognitive)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hotspots.truncate(Self::MAX_HOTSPOTS);
        hotspots
    }

    fn from_space_entry(entry: &SpaceEntry) -> FunctionHotspot {
        FunctionHotspot {
            name: entry.name.clone(),
            start_line: entry.start_line,
            end_line: entry.end_line,
            cognitive: entry.metrics.cognitive.sum,
            cyclomatic: entry.metrics.cyclomatic.sum,
            sloc: entry.metrics.loc.sloc,
        }
    }

    fn is_nontrivial_function(entry: &SpaceEntry) -> bool {
        // Thresholds chosen to filter out trivially simple functions (getters, single-match
        // arms, small helpers) that inflate file-level totals without representing genuine
        // cognitive burden. A function needs meaningful branching or nesting to qualify.
        is_named_function(entry)
            && (entry.metrics.cognitive.sum >= 3.0 || entry.metrics.cyclomatic.sum >= 5.0)
            && entry.metrics.loc.sloc >= 4.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Kotlin,
    Swift,
    Cpp,
    C,
    Java,
    CSharp,
    Go,
    Html,
    Css,
    Unknown,
}

impl Language {
    pub fn from_path(path: &std::path::Path) -> Self {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(Self::from_extension)
            .unwrap_or(Language::Unknown)
    }

    fn from_extension(ext: &str) -> Language {
        match ext {
            "rs" => Language::Rust,
            "py" => Language::Python,
            "js" | "jsx" => Language::JavaScript,
            "ts" | "tsx" => Language::TypeScript,
            "kt" | "kts" => Language::Kotlin,
            "swift" => Language::Swift,
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => Language::Cpp,
            "c" | "h" => Language::C,
            "java" => Language::Java,
            "cs" => Language::CSharp,
            "go" => Language::Go,
            "html" | "htm" => Language::Html,
            "css" => Language::Css,
            _ => Language::Unknown,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Language::Rust => "Rust",
            Language::Python => "Python",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Kotlin => "Kotlin",
            Language::Swift => "Swift",
            Language::Cpp => "C++",
            Language::C => "C",
            Language::Java => "Java",
            Language::CSharp => "C#",
            Language::Go => "Go",
            Language::Html => "HTML",
            Language::Css => "CSS",
            Language::Unknown => "Unknown",
        }
    }
}

/// Resolves language names and file extensions to a normalised list of file extensions.
///
/// Accepts a mix of language names (e.g. `"rust"`, `"kotlin"`) and raw extensions
/// (e.g. `"rs"`, `"kt"`). Language names are matched case-insensitively and expanded
/// to all their associated extensions. Unknown values are passed through unchanged so
/// the extension filter can handle them.
///
/// # Examples
///
/// ```
/// // Language name
/// assert_eq!(resolve_extensions(&["rust".to_string()]), vec!["rs"]);
///
/// // Raw extension (pass-through)
/// assert_eq!(resolve_extensions(&["rs".to_string()]), vec!["rs"]);
///
/// // Mixed, case-insensitive
/// assert_eq!(resolve_extensions(&["Rust".to_string(), "py".to_string()]), vec!["rs", "py"]);
/// ```
pub fn resolve_extensions(inputs: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for input in inputs {
        let exts = language_name_to_extensions(input.trim());
        if exts.is_empty() {
            // Not a known language name — treat as a raw extension.
            result.push(input.clone());
        } else {
            for ext in exts {
                result.push(ext.to_string());
            }
        }
    }
    result
}

/// Maps a language name (case-insensitive) to its file extensions.
/// Returns an empty slice if the name is not recognised as a language alias.
fn language_name_to_extensions(name: &str) -> &'static [&'static str] {
    match name.to_lowercase().as_str() {
        "rust" => &["rs"],
        "python" | "py" => &["py"],
        "javascript" | "js" => &["js", "jsx"],
        "typescript" | "ts" => &["ts", "tsx"],
        "kotlin" | "kt" => &["kt", "kts"],
        "swift" => &["swift"],
        "c++" | "cpp" => &["cpp", "cc", "cxx", "hpp", "hxx", "hh"],
        "java" => &["java"],
        "c#" | "csharp" | "cs" => &["cs"],
        "go" | "golang" => &["go"],
        "html" | "htm" => &["html", "htm"],
        "css" => &["css"],
        // Not a language name — caller will treat it as a raw extension.
        _ => &[],
    }
}

#[derive(Debug)]
pub struct FileSimplicity {
    pub path: String,
    pub language: Language,
    pub score: f64,
    pub cognitive: f64,
    pub cyclomatic: f64,
    pub sloc: f64,
    pub mi: f64,
    /// Cognitive complexity of the single most complex named function in the file.
    /// Zero when no named functions are present. Used as a direct complexity signal
    /// that rewards decomposing the worst hotspot — the primary agentic refactoring op.
    pub peak_cognitive: f64,
    pub hotspots: Vec<FunctionHotspot>,
    /// Names of the analysis tools that contributed to this file's metrics.
    /// Always contains at least `"rust-code-analysis"`. May also contain
    /// `"detekt"`, `"swiftlint"`, etc. when external analyzers ran.
    pub analysis_sources: Vec<String>,
}

impl FileSimplicity {
    /// Calculates a simplicity score (0-100) for a file based on code metrics.
    ///
    /// # Scoring Formula
    ///
    /// ```text
    /// Score = 0.4 × MI + 0.3 × Cog_Score + 0.2 × Peak_Score + 0.1 × Length_Score
    /// ```
    ///
    /// - **MI** (40%): SLOC-weighted average of function-level Maintainability Indices.
    ///   Always prefers function-level MI over raw file MI (see [`resolve_mi`]).
    /// - **Cog_Score** (30%): `(100 - cognitive/sloc×100).max(0)` — cognitive density;
    ///   rewards low nesting depth relative to file size.
    /// - **Peak_Score** (20%): `(100 - max_function_cognitive).max(0)` — directly rewards
    ///   decomposing the single worst function, the primary agentic refactoring operation.
    /// - **Length_Score** (10%): `min(100, 300/sloc×100)` — files ≤300 SLOC score 100;
    ///   larger files are penalised proportionally for the agent context-window cost.
    ///
    /// Cyclomatic complexity is not a standalone term because it is already embedded in
    /// the MI formula (`0.23×G`). Giving it extra weight caused effective over-weighting
    /// on branches vs cognitive nesting depth, which is a worse predictor of agent errors.
    pub fn calculate(results: &MetricsResults, normalized: bool) -> Self {
        let cognitive = results.metrics.cognitive.sum;
        let cyclomatic = results.metrics.cyclomatic.sum;
        let sloc = results.metrics.loc.sloc;
        let mi = Self::resolve_mi(results.metrics.mi.mi_visual_studio, &results.spaces);
        let peak_cognitive = Self::max_function_cognitive(&results.spaces);

        let score = if mi == 0.0 && cognitive == 0.0 && cyclomatic <= 1.0 {
            // Trivial/empty files get perfect score
            100.0
        } else {
            let mi_score = mi.clamp(0.0, 100.0);
            let cog_score = Self::cognitive_density_score(cognitive, sloc, normalized);
            let peak_score = (100.0 - peak_cognitive).max(0.0);
            let length_score = Self::length_score(sloc);
            0.4 * mi_score + 0.3 * cog_score + 0.2 * peak_score + 0.1 * length_score
        };

        let hotspots = FunctionHotspot::extract_from_spaces(&results.spaces);
        let path_obj = std::path::Path::new(&results.name);
        Self {
            path: results.name.clone(),
            language: Language::from_path(path_obj),
            score: score.clamp(0.0, 100.0),
            cognitive,
            cyclomatic,
            sloc,
            mi,
            peak_cognitive,
            hotspots,
            analysis_sources: vec!["rust-code-analysis".to_string()],
        }
    }

    /// Build a [`FileSimplicity`] entry from external analyzer metrics alone,
    /// without a corresponding rust-code-analysis base.
    ///
    /// Used for language files (e.g. Kotlin) where rca cannot produce metrics
    /// but an external tool (Detekt) did analyze the file.  SLOC is estimated
    /// by counting source lines in the file on disk; MI is unknown (0).
    pub fn from_external(
        path: &std::path::Path,
        external: &crate::analysis::ExternalMetrics,
    ) -> Option<Self> {
        let cognitive = external.cognitive.unwrap_or(0.0);
        let cyclomatic = external.cyclomatic.unwrap_or(0.0);
        if cognitive == 0.0 && cyclomatic == 0.0 {
            return None;
        }

        // Estimate SLOC from the file on disk; fall back to a neutral value.
        let sloc = std::fs::read_to_string(path)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count() as f64)
            .unwrap_or(100.0)
            .max(1.0);

        let sloc_factor = sloc;
        let cog_score = (100.0 - (cognitive / sloc_factor) * 100.0).max(0.0);
        // Peak cognitive is unknown for external-only files (no space tree).
        // Use file-level cognitive as a conservative proxy.
        let peak_score = (100.0 - cognitive).max(0.0);
        // MI unknown → treat as 50 (neutral midpoint) so the score stays
        // representative of the complexity components we do have.
        let mi_score = 50.0_f64;
        let length_score = Self::length_score(sloc);
        let score = (0.4 * mi_score + 0.3 * cog_score + 0.2 * peak_score + 0.1 * length_score)
            .clamp(0.0, 100.0);

        let analyzer = if external.analyzer.is_empty() {
            "detekt".to_string()
        } else {
            external.analyzer.clone()
        };

        Some(Self {
            path: path.to_string_lossy().into_owned(),
            language: Language::from_path(path),
            score,
            cognitive,
            cyclomatic,
            sloc,
            mi: 0.0,
            peak_cognitive: cognitive, // no space tree — file-level as proxy
            hotspots: vec![],
            analysis_sources: vec![analyzer],
        })
    }

    /// Like [`calculate`], but merges in supplementary metrics from an external
    /// analyzer (e.g. Detekt for Kotlin files).
    ///
    /// When external data overlaps with rca data, the **higher** (more complex)
    /// value is used — a conservative approach that catches complexity either
    /// analyzer alone might miss. The score is recalculated only if the merged
    /// values differ from the rca values.
    pub fn apply_external(mut self, external: &crate::analysis::ExternalMetrics) -> Self {
        let mut changed = false;

        if let Some(ext_cog) = external.cognitive
            && ext_cog > self.cognitive
        {
            self.cognitive = ext_cog;
            changed = true;
        }
        if let Some(ext_cyc) = external.cyclomatic
            && ext_cyc > self.cyclomatic
        {
            self.cyclomatic = ext_cyc;
            changed = true;
        }

        if changed {
            // Recalculate score with updated complexity values using the same
            // formula as calculate() (normalized density mode, function-weighted MI).
            let mi_score = self.mi.clamp(0.0, 100.0);
            let cog_score = Self::cognitive_density_score(self.cognitive, self.sloc, true);
            let peak_score = (100.0 - self.peak_cognitive).max(0.0);
            let length_score = Self::length_score(self.sloc);
            let new_score =
                0.4 * mi_score + 0.3 * cog_score + 0.2 * peak_score + 0.1 * length_score;
            self.score = new_score.clamp(0.0, 100.0);
        }

        // Record which analyzer contributed regardless of whether values changed.
        if !external.analyzer.is_empty() {
            let sources: Vec<&str> = external.analyzer.split(", ").collect();
            for src in sources {
                let s = src.to_string();
                if !self.analysis_sources.contains(&s) {
                    self.analysis_sources.push(s);
                }
            }
        }

        self
    }

    /// Resolves the effective MI value for a file.
    ///
    /// Always uses the SLOC-weighted average of function-level MI values.
    /// File-level MI from rust-code-analysis is often 0 (the Visual Studio variant
    /// clamps the raw value to 0 for large files) or misleading after refactoring
    /// (extracting helpers into the same file increases the total cyclomatic sum,
    /// pushing file-level MI down even when every individual function becomes simpler).
    /// Function-weighted average MI is immune to both problems.
    ///
    /// Falls back to raw file MI only when no named functions with positive SLOC exist.
    fn resolve_mi(file_mi: f64, spaces: &[SpaceEntry]) -> f64 {
        Self::weighted_function_mi(spaces).unwrap_or(file_mi)
    }

    /// Cognitive density score: `(100 - cognitive/sloc×100).max(0)`.
    ///
    /// In `normalized` mode (default) density is relative to SLOC so files of
    /// different sizes are compared on equal footing. In raw mode, absolute
    /// cognitive complexity is used (mainly kept for backwards compat in tests).
    fn cognitive_density_score(cognitive: f64, sloc: f64, normalized: bool) -> f64 {
        if normalized {
            let density = (cognitive / sloc.max(1.0)) * 100.0;
            (100.0 - density).max(0.0)
        } else {
            (100.0 - cognitive).max(0.0)
        }
    }

    /// Length score: full marks for files ≤300 SLOC, decreasing linearly above.
    ///
    /// 300 SLOC is the approximate size where an agent can hold the entire file
    /// in working memory. Larger files risk incomplete edits or missed context.
    fn length_score(sloc: f64) -> f64 {
        const IDEAL_MAX_SLOC: f64 = 300.0;
        (IDEAL_MAX_SLOC / sloc.max(1.0) * 100.0).min(100.0)
    }

    /// Returns the cognitive complexity of the single most complex named function.
    /// Returns 0.0 when no named functions are present.
    fn max_function_cognitive(spaces: &[SpaceEntry]) -> f64 {
        let mut max_cog = 0.0_f64;
        walk_spaces(spaces, &mut |entry| {
            if is_named_function(entry) {
                max_cog = max_cog.max(entry.metrics.cognitive.sum);
            }
        });
        max_cog
    }

    /// Computes a SLOC-weighted average MI from function-level spaces.
    /// Returns None if no functions with positive SLOC are found.
    fn weighted_function_mi(spaces: &[SpaceEntry]) -> Option<f64> {
        let mut total_weight = 0.0_f64;
        let mut weighted_sum = 0.0_f64;
        walk_spaces(spaces, &mut |entry| {
            if is_named_function(entry) && entry.metrics.loc.sloc > 0.0 {
                let sloc = entry.metrics.loc.sloc;
                weighted_sum += entry.metrics.mi.mi_visual_studio * sloc;
                total_weight += sloc;
            }
        });
        if total_weight > 0.0 {
            Some((weighted_sum / total_weight).clamp(0.0, 100.0))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_space_entry(
        name: &str,
        start_line: u32,
        end_line: u32,
        cognitive: f64,
        cyclomatic: f64,
        sloc: f64,
    ) -> SpaceEntry {
        SpaceEntry {
            name: name.to_string(),
            start_line,
            end_line,
            kind: "function".to_string(),
            metrics: Metrics {
                cognitive: Cognitive { sum: cognitive },
                cyclomatic: Cyclomatic { sum: cyclomatic },
                mi: Mi {
                    mi_visual_studio: 50.0,
                },
                loc: Loc { sloc },
            },
            spaces: vec![],
        }
    }

    #[test]
    fn test_hotspot_extraction_sorts_by_cognitive() {
        let spaces = vec![
            make_space_entry("low_complexity", 1, 10, 2.0, 3.0, 10.0),
            make_space_entry("high_complexity", 20, 50, 25.0, 15.0, 30.0),
            make_space_entry("medium_complexity", 60, 80, 10.0, 8.0, 20.0),
        ];
        let hotspots = FunctionHotspot::extract_from_spaces(&spaces);
        // low_complexity (Cog=2, Cyc=3) falls below the nontrivial threshold
        // (requires Cog>=3 or Cyc>=5) so only the two genuinely complex
        // functions are returned, sorted by cognitive complexity descending.
        assert_eq!(hotspots.len(), 2);
        assert_eq!(hotspots[0].name, "high_complexity");
        assert_eq!(hotspots[1].name, "medium_complexity");
    }

    #[test]
    fn test_hotspot_extraction_skips_anonymous_and_trivial() {
        let spaces = vec![
            make_space_entry("<anonymous>", 1, 5, 3.0, 2.0, 5.0),
            make_space_entry("trivial_fn", 10, 12, 0.0, 1.0, 3.0),
            make_space_entry("real_fn", 20, 40, 8.0, 6.0, 20.0),
        ];
        let hotspots = FunctionHotspot::extract_from_spaces(&spaces);
        assert_eq!(hotspots.len(), 1);
        assert_eq!(hotspots[0].name, "real_fn");
    }

    #[test]
    fn test_hotspot_extraction_limits_to_max() {
        let spaces: Vec<SpaceEntry> = (0..10)
            .map(|i| {
                make_space_entry(
                    &format!("fn_{}", i),
                    i * 10,
                    i * 10 + 9,
                    (i + 1) as f64,
                    5.0,
                    10.0,
                )
            })
            .collect();
        let hotspots = FunctionHotspot::extract_from_spaces(&spaces);
        assert_eq!(hotspots.len(), FunctionHotspot::MAX_HOTSPOTS);
        assert_eq!(hotspots[0].name, "fn_9");
    }

    #[test]
    fn test_hotspot_extraction_recurses_into_impl_blocks() {
        let impl_block = SpaceEntry {
            name: "MyStruct".to_string(),
            start_line: 1,
            end_line: 100,
            kind: "impl".to_string(),
            metrics: Metrics {
                cognitive: Cognitive { sum: 20.0 },
                cyclomatic: Cyclomatic { sum: 15.0 },
                mi: Mi {
                    mi_visual_studio: 50.0,
                },
                loc: Loc { sloc: 100.0 },
            },
            spaces: vec![
                make_space_entry("method_a", 10, 30, 12.0, 8.0, 20.0),
                make_space_entry("method_b", 40, 90, 8.0, 7.0, 50.0),
            ],
        };
        let hotspots = FunctionHotspot::extract_from_spaces(&[impl_block]);
        assert_eq!(hotspots.len(), 2);
        assert_eq!(hotspots[0].name, "method_a");
        assert_eq!(hotspots[1].name, "method_b");
    }

    #[test]
    fn test_file_simplicity_calculate_perfect() {
        let results = MetricsResults {
            name: "perfect.rs".to_string(),
            metrics: Metrics {
                cognitive: Cognitive { sum: 0.0 },
                cyclomatic: Cyclomatic { sum: 0.0 },
                mi: Mi {
                    mi_visual_studio: 100.0,
                },
                loc: Loc { sloc: 0.0 },
            },
            spaces: vec![],
        };
        let simplicity = FileSimplicity::calculate(&results, true);
        assert_eq!(simplicity.score, 100.0);
    }

    #[test]
    fn test_file_simplicity_calculate_complex_large_file() {
        // No spaces → no function-weighted MI → falls back to file_mi=60.
        // sloc=500: cog_density=50/500×100=10% → cog_score=90
        // peak_cog=0 (no spaces) → peak_score=100
        // length=300/500×100=60
        // Score = 0.4×60 + 0.3×90 + 0.2×100 + 0.1×60 = 24+27+20+6 = 77
        let results = MetricsResults {
            name: "complex.rs".to_string(),
            metrics: Metrics {
                cognitive: Cognitive { sum: 50.0 },
                cyclomatic: Cyclomatic { sum: 50.0 },
                mi: Mi {
                    mi_visual_studio: 60.0,
                },
                loc: Loc { sloc: 500.0 },
            },
            spaces: vec![],
        };
        let simplicity = FileSimplicity::calculate(&results, true);
        assert_eq!(simplicity.score, 77.0);
    }

    #[test]
    fn test_file_simplicity_calculate_complex_raw() {
        // raw (normalized=false): cog_score=(100-50)=50; peak=0→100; length=60
        // Score = 0.4×60 + 0.3×50 + 0.2×100 + 0.1×60 = 24+15+20+6 = 65
        let results = MetricsResults {
            name: "complex.rs".to_string(),
            metrics: Metrics {
                cognitive: Cognitive { sum: 50.0 },
                cyclomatic: Cyclomatic { sum: 50.0 },
                mi: Mi {
                    mi_visual_studio: 60.0,
                },
                loc: Loc { sloc: 500.0 },
            },
            spaces: vec![],
        };
        let simplicity = FileSimplicity::calculate(&results, false);
        assert_eq!(simplicity.score, 65.0);
    }

    #[test]
    fn test_file_simplicity_calculate_high_density() {
        // sloc=50: cog_density=50/50×100=100% → cog_score=0
        // peak=0 → peak_score=100; length=300/50×100=600 → clamped to 100
        // Score = 0.4×60 + 0.3×0 + 0.2×100 + 0.1×100 = 24+0+20+10 = 54
        let results = MetricsResults {
            name: "dense.rs".to_string(),
            metrics: Metrics {
                cognitive: Cognitive { sum: 50.0 },
                cyclomatic: Cyclomatic { sum: 50.0 },
                mi: Mi {
                    mi_visual_studio: 60.0,
                },
                loc: Loc { sloc: 50.0 },
            },
            spaces: vec![],
        };
        let simplicity = FileSimplicity::calculate(&results, true);
        assert_eq!(simplicity.score, 54.0);
    }

    #[test]
    fn test_file_simplicity_calculate_trivial_file() {
        let results = MetricsResults {
            name: "trivial.rs".to_string(),
            metrics: Metrics {
                cognitive: Cognitive { sum: 0.0 },
                cyclomatic: Cyclomatic { sum: 1.0 },
                mi: Mi {
                    mi_visual_studio: 0.0,
                },
                loc: Loc { sloc: 7.0 },
            },
            spaces: vec![],
        };
        let simplicity = FileSimplicity::calculate(&results, true);
        assert!(simplicity.score > 90.0);
    }

    #[test]
    fn test_function_weighted_mi_used_when_file_mi_is_zero() {
        // Weighted MI: (70*100 + 80*100) / 200 = 75.0
        let results = MetricsResults {
            name: "complex_file.rs".to_string(),
            metrics: Metrics {
                cognitive: Cognitive { sum: 10.0 },
                cyclomatic: Cyclomatic { sum: 10.0 },
                mi: Mi {
                    mi_visual_studio: 0.0,
                },
                loc: Loc { sloc: 200.0 },
            },
            spaces: vec![
                SpaceEntry {
                    name: "fn_a".to_string(),
                    start_line: 1,
                    end_line: 100,
                    kind: "function".to_string(),
                    metrics: Metrics {
                        cognitive: Cognitive { sum: 5.0 },
                        cyclomatic: Cyclomatic { sum: 5.0 },
                        mi: Mi {
                            mi_visual_studio: 70.0,
                        },
                        loc: Loc { sloc: 100.0 },
                    },
                    spaces: vec![],
                },
                SpaceEntry {
                    name: "fn_b".to_string(),
                    start_line: 101,
                    end_line: 200,
                    kind: "function".to_string(),
                    metrics: Metrics {
                        cognitive: Cognitive { sum: 5.0 },
                        cyclomatic: Cyclomatic { sum: 5.0 },
                        mi: Mi {
                            mi_visual_studio: 80.0,
                        },
                        loc: Loc { sloc: 100.0 },
                    },
                    spaces: vec![],
                },
            ],
        };
        let simplicity = FileSimplicity::calculate(&results, true);
        assert_eq!(simplicity.mi, 75.0);
        // score > 80: MI=75 weighted high, low cognitive density, peak=5
        assert!(simplicity.score > 80.0);
    }

    #[test]
    fn test_function_weighted_mi_used_even_when_file_mi_is_positive() {
        // Previously MI was only overridden when file_mi == 0.
        // Now function-weighted avg is always used; file_mi=60 is ignored.
        // make_space_entry sets MI=50 per function, so weighted avg = 50.
        let results = MetricsResults {
            name: "good_file.rs".to_string(),
            metrics: Metrics {
                cognitive: Cognitive { sum: 5.0 },
                cyclomatic: Cyclomatic { sum: 5.0 },
                mi: Mi {
                    mi_visual_studio: 60.0,
                },
                loc: Loc { sloc: 100.0 },
            },
            spaces: vec![make_space_entry("fn_a", 1, 50, 3.0, 3.0, 50.0)],
        };
        let simplicity = FileSimplicity::calculate(&results, true);
        // make_space_entry hard-codes MI=50 → function-weighted avg = 50 ≠ 60
        assert_eq!(
            simplicity.mi, 50.0,
            "should use function-weighted MI, not raw file MI"
        );
    }

    #[test]
    fn test_peak_cognitive_is_max_function_cognitive() {
        let spaces = vec![
            make_space_entry("simple", 1, 10, 2.0, 3.0, 10.0),
            make_space_entry("complex", 11, 50, 18.0, 10.0, 40.0),
            make_space_entry("medium", 51, 80, 8.0, 6.0, 30.0),
        ];
        let results = MetricsResults {
            name: "mixed.rs".to_string(),
            metrics: Metrics {
                cognitive: Cognitive { sum: 28.0 },
                cyclomatic: Cyclomatic { sum: 19.0 },
                mi: Mi {
                    mi_visual_studio: 0.0,
                },
                loc: Loc { sloc: 80.0 },
            },
            spaces,
        };
        let simplicity = FileSimplicity::calculate(&results, true);
        assert_eq!(simplicity.peak_cognitive, 18.0);
    }

    #[test]
    fn test_helper_extraction_does_not_regress_score() {
        // Simulates extracting a Cog=20 monolithic function into four Cog=5 helpers.
        // The new formula should reward the decomposition (peak_cog drops 20→5).
        let make_results = |peak_cog: f64, func_count: usize| MetricsResults {
            name: "refactored.rs".to_string(),
            metrics: Metrics {
                cognitive: Cognitive {
                    sum: peak_cog + (func_count.saturating_sub(1)) as f64 * 5.0,
                },
                cyclomatic: Cyclomatic {
                    sum: (func_count * 6) as f64,
                },
                mi: Mi {
                    mi_visual_studio: 0.0,
                },
                loc: Loc {
                    sloc: (func_count * 20) as f64,
                },
            },
            spaces: (0..func_count)
                .map(|i| {
                    make_space_entry(
                        &format!("fn_{i}"),
                        (i * 20) as u32,
                        (i * 20 + 19) as u32,
                        if i == 0 { peak_cog } else { 5.0 },
                        6.0,
                        20.0,
                    )
                })
                .collect(),
        };
        let before = FileSimplicity::calculate(&make_results(20.0, 1), true);
        let after = FileSimplicity::calculate(&make_results(5.0, 4), true);
        assert!(
            after.score >= before.score,
            "helper extraction should not regress score: before={:.1} after={:.1}",
            before.score,
            after.score
        );
        assert!(
            after.peak_cognitive < before.peak_cognitive,
            "peak cognitive should drop after extraction"
        );
    }

    // ── resolve_extensions tests ──────────────────────────────────────────────

    #[test]
    fn test_resolve_extensions_language_name_rust() {
        assert_eq!(resolve_extensions(&["rust".to_string()]), vec!["rs"]);
    }

    #[test]
    fn test_resolve_extensions_language_name_kotlin() {
        assert_eq!(
            resolve_extensions(&["kotlin".to_string()]),
            vec!["kt", "kts"]
        );
    }

    #[test]
    fn test_resolve_extensions_language_name_case_insensitive() {
        assert_eq!(resolve_extensions(&["Rust".to_string()]), vec!["rs"]);
        assert_eq!(
            resolve_extensions(&["KOTLIN".to_string()]),
            vec!["kt", "kts"]
        );
        assert_eq!(resolve_extensions(&["Python".to_string()]), vec!["py"]);
    }

    #[test]
    fn test_resolve_extensions_raw_extension_passthrough() {
        // Raw extensions (not matching any language name) pass through unchanged.
        assert_eq!(resolve_extensions(&["rs".to_string()]), vec!["rs"]);
        assert_eq!(resolve_extensions(&["kt".to_string()]), vec!["kt", "kts"]);
    }

    #[test]
    fn test_resolve_extensions_mixed_names_and_extensions() {
        let input = vec!["rust".to_string(), "py".to_string()];
        let result = resolve_extensions(&input);
        assert_eq!(result, vec!["rs", "py"]);
    }

    #[test]
    fn test_resolve_extensions_multiple_language_names() {
        let input = vec!["javascript".to_string(), "typescript".to_string()];
        let result = resolve_extensions(&input);
        assert_eq!(result, vec!["js", "jsx", "ts", "tsx"]);
    }

    #[test]
    fn test_resolve_extensions_unknown_passthrough() {
        // Completely unknown token passes through unchanged.
        assert_eq!(resolve_extensions(&["xyz".to_string()]), vec!["xyz"]);
    }

    #[test]
    fn test_resolve_extensions_empty_input() {
        let result: Vec<String> = resolve_extensions(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_resolve_extensions_language_aliases() {
        // Alias: "golang" → same as "go"
        assert_eq!(resolve_extensions(&["golang".to_string()]), vec!["go"]);
        // Alias: "csharp" → same as "c#"
        assert_eq!(resolve_extensions(&["csharp".to_string()]), vec!["cs"]);
        // Alias: "cpp" → C++ extensions
        let cpp_exts = resolve_extensions(&["cpp".to_string()]);
        assert!(cpp_exts.contains(&"cpp".to_string()));
        assert!(cpp_exts.contains(&"hpp".to_string()));
    }
}
