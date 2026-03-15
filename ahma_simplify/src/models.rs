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

impl FunctionHotspot {
    const MAX_HOTSPOTS: usize = 5;

    /// Recursively collects function hotspots from the spaces tree,
    /// sorted by cognitive complexity descending, limited to MAX_HOTSPOTS.
    pub fn extract_from_spaces(spaces: &[SpaceEntry]) -> Vec<FunctionHotspot> {
        let mut hotspots = Vec::new();
        Self::collect_functions(spaces, &mut hotspots);
        hotspots.sort_by(|a, b| {
            b.cognitive
                .partial_cmp(&a.cognitive)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hotspots.truncate(Self::MAX_HOTSPOTS);
        hotspots
    }

    fn collect_functions(spaces: &[SpaceEntry], hotspots: &mut Vec<FunctionHotspot>) {
        for entry in spaces {
            Self::collect_functions(&entry.spaces, hotspots);
            if !Self::is_nontrivial_function(entry) {
                continue;
            }
            hotspots.push(Self::from_space_entry(entry));
        }
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
        is_named_function(entry)
            && (entry.metrics.cognitive.sum > 0.0 || entry.metrics.cyclomatic.sum > 1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
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

#[derive(Debug)]
pub struct FileSimplicity {
    pub path: String,
    pub language: Language,
    pub score: f64,
    pub cognitive: f64,
    pub cyclomatic: f64,
    pub sloc: f64,
    pub mi: f64,
    pub hotspots: Vec<FunctionHotspot>,
}

impl FileSimplicity {
    /// Calculates a simplicity score (0-100) for a file based on code metrics.
    ///
    /// # Scoring Formula
    ///
    /// ```text
    /// Score = 0.6 × MI + 0.2 × Cog_Score + 0.2 × Cyc_Score
    /// ```
    ///
    /// Where:
    /// - **MI**: Maintainability Index (Visual Studio variant, 0-100, higher = better)
    /// - **Cog_Score**: `(100 - (cognitive / sloc * 100)).max(0)` - density-based cognitive complexity
    /// - **Cyc_Score**: `(100 - (cyclomatic / sloc * 100)).max(0)` - density-based cyclomatic complexity
    ///
    /// # Weighting Rationale
    ///
    /// - MI (60%): Composite metric already balancing complexity, volume, and LOC
    /// - Cognitive (20%): Human comprehension difficulty (normalized by size)
    /// - Cyclomatic (20%): Testability (normalized by size)
    ///
    /// # Normalization
    ///
    /// Complexity is normalized by SLOC to calculate "complexity density" per 100 lines.
    /// A file with 1 complexity point per line (density = 100 per 100 lines) receives a 0 component score.
    pub fn calculate(results: &MetricsResults, normalized: bool) -> Self {
        let cognitive = results.metrics.cognitive.sum;
        let cyclomatic = results.metrics.cyclomatic.sum;
        let sloc = results.metrics.loc.sloc;
        let mi = Self::resolve_mi(results.metrics.mi.mi_visual_studio, &results.spaces);

        let score = if mi == 0.0 && cognitive == 0.0 && cyclomatic <= 1.0 {
            // Trivial/empty files get perfect score
            100.0
        } else {
            let mi_score = mi.clamp(0.0, 100.0);
            let (cog_score, cyc_score) =
                Self::complexity_scores(cognitive, cyclomatic, sloc, normalized);
            0.6 * mi_score + 0.2 * cog_score + 0.2 * cyc_score
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
            hotspots,
        }
    }

    /// Resolves the effective MI value for a file.
    ///
    /// File-level MI from rust-code-analysis is often 0 because the raw
    /// mi_original goes negative for large files and the Visual Studio
    /// variant clamps to 0. Individual functions have accurate MI values,
    /// so compute a SLOC-weighted average of function-level MIs as fallback.
    fn resolve_mi(file_mi: f64, spaces: &[SpaceEntry]) -> f64 {
        if file_mi == 0.0 && !spaces.is_empty() {
            Self::weighted_function_mi(spaces).unwrap_or(file_mi)
        } else {
            file_mi
        }
    }

    /// Computes cognitive and cyclomatic component scores.
    fn complexity_scores(
        cognitive: f64,
        cyclomatic: f64,
        sloc: f64,
        normalized: bool,
    ) -> (f64, f64) {
        if normalized {
            let sloc_factor = sloc.max(1.0);
            let cog_density = (cognitive / sloc_factor) * 100.0;
            let cyc_density = (cyclomatic / sloc_factor) * 100.0;
            (
                (100.0 - cog_density).max(0.0),
                (100.0 - cyc_density).max(0.0),
            )
        } else {
            ((100.0 - cognitive).max(0.0), (100.0 - cyclomatic).max(0.0))
        }
    }

    /// Computes a SLOC-weighted average MI from function-level spaces.
    /// Returns None if no functions with positive SLOC are found.
    fn weighted_function_mi(spaces: &[SpaceEntry]) -> Option<f64> {
        let mut total_weight = 0.0_f64;
        let mut weighted_sum = 0.0_f64;
        Self::accumulate_function_mi(spaces, &mut weighted_sum, &mut total_weight);
        if total_weight > 0.0 {
            Some((weighted_sum / total_weight).clamp(0.0, 100.0))
        } else {
            None
        }
    }

    fn accumulate_function_mi(
        spaces: &[SpaceEntry],
        weighted_sum: &mut f64,
        total_weight: &mut f64,
    ) {
        for entry in spaces {
            Self::accumulate_function_mi(&entry.spaces, weighted_sum, total_weight);
            if !is_named_function(entry) || entry.metrics.loc.sloc <= 0.0 {
                continue;
            }
            let sloc = entry.metrics.loc.sloc;
            *weighted_sum += entry.metrics.mi.mi_visual_studio * sloc;
            *total_weight += sloc;
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
        assert_eq!(hotspots.len(), 3);
        assert_eq!(hotspots[0].name, "high_complexity");
        assert_eq!(hotspots[1].name, "medium_complexity");
        assert_eq!(hotspots[2].name, "low_complexity");
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
        // mi_score=60, cog_density=10, cyc_density=10 => cog_score=90, cyc_score=90
        // 0.6*60 + 0.2*90 + 0.2*90 = 36 + 18 + 18 = 72
        assert_eq!(simplicity.score, 72.0);
    }

    #[test]
    fn test_file_simplicity_calculate_complex_raw() {
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
        // mi_score=60, cog_score=50, cyc_score=50
        // 0.6*60 + 0.2*50 + 0.2*50 = 36 + 10 + 10 = 56
        assert_eq!(simplicity.score, 56.0);
    }

    #[test]
    fn test_file_simplicity_calculate_high_density() {
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
        // mi_score=60, cog_density=100, cyc_density=100 => cog_score=0, cyc_score=0
        // 0.6*60 + 0.2*0 + 0.2*0 = 36
        assert_eq!(simplicity.score, 36.0);
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
    fn test_mi_fallback_uses_function_level_mi_when_file_level_is_zero() {
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
        // Weighted MI: (70*100 + 80*100) / 200 = 75.0
        assert_eq!(simplicity.mi, 75.0);
        // Score: 0.6*75 + 0.2*(100-5) + 0.2*(100-5) = 45 + 19 + 19 = 83
        assert!(simplicity.score > 80.0);
    }

    #[test]
    fn test_mi_fallback_not_used_when_file_level_mi_is_positive() {
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
        // Should use file-level MI=60, not function-level
        assert_eq!(simplicity.mi, 60.0);
    }
}
