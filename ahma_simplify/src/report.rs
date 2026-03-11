use crate::analysis::{get_package_name, get_relative_path};
use crate::models::{FileSimplicity, Language};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct RepoSummary {
    pub avg_score: f64,
    pub language_summaries: HashMap<Language, LanguageSummary>,
}

pub struct LanguageSummary {
    pub score: f64,
    pub package_scores: Vec<(String, f64)>,
}

impl RepoSummary {
    pub fn from_files(files: &[FileSimplicity], base_dir: &Path) -> Self {
        let avg_score = if files.is_empty() {
            0.0
        } else {
            files.iter().map(|f| f.score).sum::<f64>() / files.len() as f64
        };

        let mut lang_map: HashMap<Language, Vec<&FileSimplicity>> = HashMap::new();
        for f in files {
            lang_map.entry(f.language).or_default().push(f);
        }

        let mut language_summaries = HashMap::new();

        for (lang, lang_files) in lang_map {
            let lang_avg = if lang_files.is_empty() {
                0.0
            } else {
                lang_files.iter().map(|f| f.score).sum::<f64>() / lang_files.len() as f64
            };

            let mut package_map: HashMap<String, Vec<f64>> = HashMap::new();
            for f in &lang_files {
                let package = get_package_name(Path::new(&f.path), base_dir);
                package_map.entry(package).or_default().push(f.score);
            }

            let mut package_scores: Vec<(String, f64)> = package_map
                .into_iter()
                .map(|(p, scores)| {
                    let avg = scores.iter().sum::<f64>() / scores.len() as f64;
                    (p, avg)
                })
                .collect();
            package_scores.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

            language_summaries.insert(
                lang,
                LanguageSummary {
                    score: lang_avg,
                    package_scores,
                },
            );
        }

        Self {
            avg_score,
            language_summaries,
        }
    }
}

pub fn generate_report(
    files: &[FileSimplicity],
    is_workspace: bool,
    limit: usize,
    output_dir: &Path,
    generate_html: bool,
    project_name: &str,
    report_output_dir: &Path,
) -> Result<(), std::io::Error> {
    let md_content = create_report_md(files, is_workspace, limit, output_dir, project_name);

    fs::write(report_output_dir.join("CODE_SIMPLICITY.md"), &md_content)?;

    if generate_html {
        let mut options = pulldown_cmark::Options::empty();
        options.insert(pulldown_cmark::Options::ENABLE_TABLES);
        options.insert(pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
        let parser = pulldown_cmark::Parser::new_ext(&md_content, options);
        let mut html_output = String::new();
        pulldown_cmark::html::push_html(&mut html_output, parser);

        let style = "
                body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif; line-height: 1.6; color: #24292e; max-width: 900px; margin: 0 auto; padding: 40px 20px; background-color: #f6f8fa; }
                h1, h2, h3 { color: #1b1f23; border-bottom: 1px solid #eaecef; padding-bottom: 0.3em; margin-top: 1.5em; }
                pre { background-color: #f6f8fa; padding: 16px; border-radius: 6px; overflow: auto; }
                code { font-family: ui-monospace, SFMono-Regular, SF Mono, Menlo, Consolas, Liberation Mono, monospace; background-color: rgba(27,31,35,0.05); padding: 0.2em 0.4em; border-radius: 3px; }
                blockquote { padding: 0 1em; color: #6a737d; border-left: 0.25em solid #dfe2e1; margin: 0; }
                table { border-spacing: 0; border-collapse: collapse; width: 100%; margin: 1em 0; }
                table td, table th { padding: 6px 13px; border: 1px solid #dfe2e1; }
                table tr { background-color: #fff; border-top: 1px solid #c6cbd1; }
                table tr:nth-child(2n) { background-color: #f6f8fa; }
            ";

        let full_html = format!(
            "<!DOCTYPE html>\n<html>\n<head>\n<meta charset='UTF-8'>\n<title>Code Simplicity Report</title>\n<style>\n{}\n</style>\n</head>\n<body>\n{}\n</body>\n</html>",
            style, html_output
        );
        fs::write(report_output_dir.join("CODE_SIMPLICITY.html"), full_html)?;
    }
    Ok(())
}

pub fn create_report_md(
    files: &[FileSimplicity],
    is_workspace: bool,
    limit: usize,
    base_dir: &Path,
    project_name: &str,
) -> String {
    let summary = RepoSummary::from_files(files, base_dir);
    let mut report = String::new();

    write_header(&mut report, project_name, summary.avg_score);
    write_executive_summary(&mut report, summary.avg_score);
    write_package_simplicity(&mut report, &summary, is_workspace);
    write_emergencies(&mut report, files, limit, base_dir);
    write_glossary(&mut report);

    report
}

fn write_header(report: &mut String, project_name: &str, avg_score: f64) {
    report.push_str(&format!("# Code Simplicity Metrics: {}\n\n", project_name));
    report.push_str(&format!(
        "## Overall Repository Simplicity: **{:.0}%**\n\n",
        avg_score
    ));
    let now = chrono::Local::now();
    report.push_str(&format!(
        "*Generated on: {}*\n\n",
        now.format("%Y-%m-%d %H:%M:%S")
    ));
}

fn write_executive_summary(report: &mut String, avg_score: f64) {
    report.push_str("### Executive Summary\n");
    let msg = if avg_score > 80.0 {
        "The repository has good simplicity overall. Focus on isolated high-complexity files.\n\n"
    } else if avg_score > 60.0 {
        "The repository has moderate technical debt. Consider refactoring the top complexity issues.\n\n"
    } else {
        "The repository requires significant architectural review. Multiple areas show high complexity.\n\n"
    };
    report.push_str(msg);
}

fn write_package_simplicity(report: &mut String, summary: &RepoSummary, is_workspace: bool) {
    // Sort languages by name for consistent output
    let mut languages: Vec<_> = summary.language_summaries.keys().collect();
    languages.sort_by(|a, b| a.display_name().cmp(b.display_name()));

    for lang in languages {
        if let Some(lang_summary) = summary.language_summaries.get(lang) {
            report.push_str(&format!(
                "## {} Simplicity (Avg: {:.0}%)\n\n",
                lang.display_name(),
                lang_summary.score
            ));

            let group_label = match lang {
                Language::Rust => {
                    if is_workspace {
                        "Crate"
                    } else {
                        "Module"
                    }
                }
                Language::Python | Language::JavaScript | Language::TypeScript => "Module",
                _ => "Directory",
            };

            if lang_summary.package_scores.len() > 1 {
                report.push_str(&format!("### By {}\n\n", group_label));

                for (i, (p, score)) in lang_summary.package_scores.iter().enumerate() {
                    report.push_str(&format!("{}. **{}**: {:.0}%\n", i + 1, p, score));
                }
                report.push('\n');
            }
        }
    }
}

/// Compute display names for a list of files, disambiguating entries that share
/// a basename by adding the minimal parent directory context needed for uniqueness.
fn disambiguate_display_names(files: &[&FileSimplicity], base_dir: &Path) -> Vec<String> {
    let entries: Vec<(PathBuf, String)> = files
        .iter()
        .map(|f| {
            let path = Path::new(&f.path);
            let rel_path = get_relative_path(path, base_dir);
            let basename = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| rel_path.to_string_lossy().to_string());
            (rel_path, basename)
        })
        .collect();

    let mut basename_counts: HashMap<&str, usize> = HashMap::new();
    for (_, basename) in &entries {
        *basename_counts.entry(basename.as_str()).or_insert(0) += 1;
    }

    entries
        .iter()
        .enumerate()
        .map(|(i, (rel_path, basename))| {
            if basename_counts.get(basename.as_str()).copied().unwrap_or(0) <= 1 {
                return basename.clone();
            }

            let components: Vec<String> = rel_path
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect();
            let n = components.len();

            let siblings: Vec<Vec<String>> = entries
                .iter()
                .enumerate()
                .filter(|(j, (_, b))| *j != i && b == basename)
                .map(|(_, (rp, _))| {
                    rp.components()
                        .map(|c| c.as_os_str().to_string_lossy().to_string())
                        .collect()
                })
                .collect();

            for depth in 2..=n {
                let candidate = components[n - depth..].join("/");
                let is_unique = siblings.iter().all(|other_comps| {
                    let on = other_comps.len();
                    if depth > on {
                        true
                    } else {
                        other_comps[on - depth..].join("/") != candidate
                    }
                });
                if is_unique {
                    return candidate;
                }
            }

            rel_path.to_string_lossy().to_string()
        })
        .collect()
}

fn write_emergencies(report: &mut String, files: &[FileSimplicity], limit: usize, base_dir: &Path) {
    let mut lang_map: HashMap<Language, Vec<&FileSimplicity>> = HashMap::new();
    for f in files {
        lang_map.entry(f.language).or_default().push(f);
    }

    let mut languages: Vec<_> = lang_map.keys().collect();
    languages.sort_by(|a, b| a.display_name().cmp(b.display_name()));

    for lang in languages {
        let lang_files = lang_map.get(lang).unwrap();
        let display_limit = std::cmp::min(limit, lang_files.len());
        let displayed: Vec<&FileSimplicity> =
            lang_files.iter().take(display_limit).copied().collect();
        let display_names = disambiguate_display_names(&displayed, base_dir);

        report.push_str(&format!(
            "## Top {display_limit} {} Code Complexity Issues (Lowest Simplicity)\n\n",
            lang.display_name()
        ));

        for (i, f) in displayed.iter().enumerate() {
            let culprit = identify_culprit(f);
            let path = Path::new(&f.path);
            let rel_path = get_relative_path(path, base_dir);
            let rel_str = rel_path.to_string_lossy();

            report.push_str(&format!(
                "{}. **{}**: Simplicity: {:.0}% ({})**\n\t{}\n",
                i + 1,
                display_names[i],
                f.score,
                culprit,
                rel_str
            ));
            report.push_str(&format!(
                "    - Metrics: Cog: {}, Cyc: {}, SLOC: {}, MI: {:.1}\n",
                f.cognitive, f.cyclomatic, f.sloc, f.mi
            ));
            if !f.hotspots.is_empty() {
                report.push_str("    - **Hotspots**:\n");
                for h in &f.hotspots {
                    report.push_str(&format!(
                        "      - `{}()` lines {}-{}: Cog={}, Cyc={}, SLOC={}\n",
                        h.name,
                        h.start_line,
                        h.end_line,
                        h.cognitive as u32,
                        h.cyclomatic as u32,
                        h.sloc as u32
                    ));
                }
            }
        }
        report.push('\n');
    }
}

fn identify_culprit(f: &FileSimplicity) -> &'static str {
    if f.cognitive > 20.0 {
        "High Cognitive Complexity"
    } else if f.cyclomatic > 20.0 {
        "High Cyclomatic Complexity"
    } else if f.sloc > 500.0 {
        "Mega-file"
    } else if f.mi < 50.0 {
        "Low Maintainability Index"
    } else {
        "General Complexity"
    }
}

/// Generates a structured AI prompt instructing the parent AI to simplify a
/// specific issue from the complexity report.
///
/// `files` must already be sorted by score ascending (worst first).
/// `issue_number` is 1-indexed (1 = most complex file).
///
/// Returns `None` if `issue_number` is out of bounds or files is empty.
pub fn generate_ai_fix_prompt(
    files: &[FileSimplicity],
    issue_number: usize,
    base_dir: &Path,
) -> Option<String> {
    if issue_number == 0 || files.is_empty() {
        return None;
    }
    let index = issue_number - 1;
    let file = files.get(index)?;

    let rel_path = get_relative_path(Path::new(&file.path), base_dir);
    let rel_str = rel_path.to_string_lossy();
    let culprit = identify_culprit(file);

    Some(format!(
        "\
=== SIMPLIFY CODE: ISSUE #{issue_number} ===

TARGET: {rel_str}
SIMPLICITY: {score:.0}% | ISSUE: {culprit}
METRICS: Cognitive={cog}, Cyclomatic={cyc}, SLOC={sloc}, MI={mi:.1}

STEP 1 - READ the simplicity report above and the target file. The report
         identifies specific hotspot functions causing complexity.

STEP 2 - PLAN (briefly): Which hotspot functions will you refactor, and how?

STEP 3 - IMPLEMENT focused changes:
   - Target ONLY the hotspot functions listed in the report
   - Make minimal changes to achieve measurable improvement
   - Do NOT refactor surrounding code unless directly needed
   - Prefer early returns, guard clauses, and extracting helpers
   - Break functions longer than ~40 lines into focused units

STEP 4 - VERIFY by running the project's test suite.

Execute all steps now. Do not stop at planning.",
        issue_number = issue_number,
        rel_str = rel_str,
        score = file.score,
        culprit = culprit,
        cog = file.cognitive,
        cyc = file.cyclomatic,
        sloc = file.sloc,
        mi = file.mi,
    ))
}

fn write_glossary(report: &mut String) {
    report.push_str("\n---\n\n## Metrics Glossary\n\n");
    report.push_str("### Cognitive Complexity\n- **Description**: Measures how hard it is to understand the control flow of the code. [See](https://axify.io/blog/cognitive-complexity)\n- **How to Improve**: Extract complex conditions into well-named functions and reduce nesting levels.\n\n");
    report.push_str("### Cyclomatic Complexity\n- **Description**: Measures the number of linearly independent paths through the source code. [See](https://www.nist.gov/publications/structured-testing-software-testing-methodology-using-cyclomatic-complexity-metric)\n- **How to Improve**: Use polymorphic abstractions instead of complex switch/if-else chains, and break down large functions into smaller components.\n\n");
    report.push_str("### Source Lines of Code (SLOC)\n- **Description**: A measure of the size of the computer program by counting the number of lines in the text of the source code. [See](https://en.wikipedia.org/wiki/Source_lines_of_code)\n- **How to Improve**: Remove dead code and refactor repetitive logic into reusable helper functions or macros.\n\n");
    report.push_str("### Maintainability Index (MI)\n- **Description**: A composite metric representing the relative ease of maintaining the code; higher is better. [See](https://learn.microsoft.com/en-us/visualstudio/code-quality/code-metrics-maintainability-index-range-and-meaning)\n- **How to Improve**: Simultaneously reduce complexity (both cognitive and cyclomatic) and file size to boost the index.\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::FunctionHotspot;

    /// Helper to construct a FileSimplicity for tests without hotspots.
    fn test_file(
        path: &str,
        lang: Language,
        score: f64,
        cog: f64,
        cyc: f64,
        sloc: f64,
        mi: f64,
    ) -> FileSimplicity {
        FileSimplicity {
            path: path.to_string(),
            language: lang,
            score,
            cognitive: cog,
            cyclomatic: cyc,
            sloc,
            mi,
            hotspots: vec![],
        }
    }

    /// Helper to construct a FileSimplicity with hotspots for testing report output.
    #[allow(clippy::too_many_arguments)]
    fn test_file_with_hotspots(
        path: &str,
        lang: Language,
        score: f64,
        cog: f64,
        cyc: f64,
        sloc: f64,
        mi: f64,
        hotspots: Vec<FunctionHotspot>,
    ) -> FileSimplicity {
        FileSimplicity {
            path: path.to_string(),
            language: lang,
            score,
            cognitive: cog,
            cyclomatic: cyc,
            sloc,
            mi,
            hotspots,
        }
    }

    #[test]
    fn test_create_report_structure() {
        let files = vec![
            test_file(
                "pkg1/file1.rs",
                Language::Rust,
                80.0,
                10.0,
                5.0,
                100.0,
                100.0,
            ),
            test_file(
                "pkg2/file2.rs",
                Language::Rust,
                40.0,
                30.0,
                25.0,
                600.0,
                40.0,
            ),
        ];

        let report = create_report_md(&files, false, 10, Path::new("."), "test_project");
        assert!(report.contains("# Code Simplicity Metrics: test_project"));
        assert!(report.contains("## Overall Repository Simplicity: **60%**"));
        assert!(report.contains("## Rust Simplicity"));
    }

    #[test]
    fn test_report_multi_language_emergencies() {
        let files = vec![
            test_file("file1.rs", Language::Rust, 50.0, 20.0, 15.0, 100.0, 50.0),
            test_file("file2.py", Language::Python, 40.0, 25.0, 20.0, 150.0, 40.0),
        ];

        let report = create_report_md(&files, false, 10, Path::new("."), "test_multi");

        assert!(report.contains("## Top 1 Rust Code Complexity Issues"));
        assert!(report.contains("## Top 1 Python Code Complexity Issues"));
        assert!(report.contains("file1.rs"));
        assert!(report.contains("file2.py"));
        assert!(!report.contains("(Rust)"));
        assert!(!report.contains("(Python)"));
    }

    #[test]
    fn test_disambiguate_unique_basenames_unchanged() {
        let files = [
            test_file("src/foo.rs", Language::Rust, 50.0, 20.0, 15.0, 100.0, 50.0),
            test_file("src/bar.rs", Language::Rust, 40.0, 25.0, 20.0, 150.0, 40.0),
        ];
        let refs: Vec<&FileSimplicity> = files.iter().collect();
        let names = disambiguate_display_names(&refs, Path::new("."));
        assert_eq!(names, vec!["foo.rs", "bar.rs"]);
    }

    #[test]
    fn test_disambiguate_duplicate_basenames_adds_parent_dir() {
        let files = [
            test_file(
                "project/src/analysis/translation.rs",
                Language::Rust,
                36.0,
                92.0,
                174.0,
                1191.0,
                0.0,
            ),
            test_file(
                "project/src/views/translation.rs",
                Language::Rust,
                36.0,
                88.0,
                68.0,
                786.0,
                0.0,
            ),
        ];
        let refs: Vec<&FileSimplicity> = files.iter().collect();
        let names = disambiguate_display_names(&refs, Path::new("."));
        assert_eq!(
            names,
            vec!["analysis/translation.rs", "views/translation.rs"]
        );
    }

    #[test]
    fn test_disambiguate_three_files_same_basename() {
        let files = [
            test_file(
                "crate_a/src/lib.rs",
                Language::Rust,
                50.0,
                10.0,
                5.0,
                100.0,
                50.0,
            ),
            test_file(
                "crate_b/src/lib.rs",
                Language::Rust,
                40.0,
                20.0,
                10.0,
                200.0,
                40.0,
            ),
            test_file(
                "crate_c/src/lib.rs",
                Language::Rust,
                30.0,
                30.0,
                15.0,
                300.0,
                30.0,
            ),
        ];
        let refs: Vec<&FileSimplicity> = files.iter().collect();
        let names = disambiguate_display_names(&refs, Path::new("."));
        assert_eq!(
            names,
            vec![
                "crate_a/src/lib.rs",
                "crate_b/src/lib.rs",
                "crate_c/src/lib.rs"
            ]
        );
    }

    #[test]
    fn test_disambiguate_mixed_unique_and_duplicate() {
        let files = [
            test_file(
                "src/analysis/translation.rs",
                Language::Rust,
                36.0,
                92.0,
                174.0,
                1191.0,
                0.0,
            ),
            test_file(
                "src/unique_file.rs",
                Language::Rust,
                50.0,
                10.0,
                5.0,
                100.0,
                50.0,
            ),
            test_file(
                "src/views/translation.rs",
                Language::Rust,
                36.0,
                88.0,
                68.0,
                786.0,
                0.0,
            ),
        ];
        let refs: Vec<&FileSimplicity> = files.iter().collect();
        let names = disambiguate_display_names(&refs, Path::new("."));
        assert_eq!(
            names,
            vec![
                "analysis/translation.rs",
                "unique_file.rs",
                "views/translation.rs"
            ]
        );
    }

    #[test]
    fn test_report_disambiguates_same_basename_files() {
        let files = vec![
            test_file(
                "project/src/analysis/translation.rs",
                Language::Rust,
                36.0,
                92.0,
                174.0,
                1191.0,
                0.0,
            ),
            test_file(
                "project/src/views/translation.rs",
                Language::Rust,
                36.0,
                88.0,
                68.0,
                786.0,
                0.0,
            ),
        ];

        let report = create_report_md(&files, false, 10, Path::new("."), "test_disambig");

        assert!(report.contains("**analysis/translation.rs**"));
        assert!(report.contains("**views/translation.rs**"));
        assert!(!report.contains("**translation.rs**"));
    }

    #[test]
    fn test_report_includes_hotspots() {
        let files = vec![test_file_with_hotspots(
            "src/complex.rs",
            Language::Rust,
            35.0,
            45.0,
            30.0,
            800.0,
            0.0,
            vec![
                FunctionHotspot {
                    name: "handle_request".to_string(),
                    start_line: 145,
                    end_line: 210,
                    cognitive: 28.0,
                    cyclomatic: 15.0,
                    sloc: 65.0,
                },
                FunctionHotspot {
                    name: "process_message".to_string(),
                    start_line: 312,
                    end_line: 350,
                    cognitive: 10.0,
                    cyclomatic: 8.0,
                    sloc: 38.0,
                },
            ],
        )];

        let report = create_report_md(&files, false, 10, Path::new("."), "test_hotspots");

        assert!(report.contains("**Hotspots**:"));
        assert!(report.contains("`handle_request()` lines 145-210: Cog=28, Cyc=15"));
        assert!(report.contains("`process_message()` lines 312-350: Cog=10, Cyc=8"));
    }

    #[test]
    fn test_report_no_hotspots_section_when_empty() {
        let files = vec![test_file(
            "src/simple.rs",
            Language::Rust,
            80.0,
            5.0,
            3.0,
            50.0,
            90.0,
        )];

        let report = create_report_md(&files, false, 10, Path::new("."), "test_no_hotspots");

        assert!(!report.contains("**Hotspots**"));
    }

    #[test]
    fn test_generate_ai_fix_prompt_issue_1() {
        let files = vec![
            test_file(
                "src/complex.rs",
                Language::Rust,
                25.0,
                45.0,
                30.0,
                800.0,
                35.0,
            ),
            test_file(
                "src/moderate.rs",
                Language::Rust,
                65.0,
                12.0,
                8.0,
                200.0,
                70.0,
            ),
        ];

        let prompt = generate_ai_fix_prompt(&files, 1, Path::new(".")).unwrap();

        assert!(prompt.contains("ISSUE #1"));
        assert!(prompt.contains("src/complex.rs"));
        assert!(prompt.contains("25%"));
        assert!(prompt.contains("High Cognitive Complexity"));
        assert!(prompt.contains("Cognitive=45"));
        assert!(prompt.contains("Cyclomatic=30"));
        assert!(prompt.contains("SLOC=800"));
        assert!(prompt.contains("MI=35.0"));
        assert!(prompt.contains("STEP 1"));
        assert!(prompt.contains("STEP 4"));
        assert!(prompt.contains("hotspot functions"));
        assert!(prompt.contains("Target ONLY"));
    }

    #[test]
    fn test_generate_ai_fix_prompt_issue_2() {
        let files = vec![
            test_file(
                "src/worst.rs",
                Language::Rust,
                20.0,
                50.0,
                40.0,
                900.0,
                30.0,
            ),
            test_file(
                "src/second.rs",
                Language::Rust,
                40.0,
                10.0,
                25.0,
                300.0,
                45.0,
            ),
        ];

        let prompt = generate_ai_fix_prompt(&files, 2, Path::new(".")).unwrap();

        assert!(prompt.contains("ISSUE #2"));
        assert!(prompt.contains("src/second.rs"));
        assert!(prompt.contains("High Cyclomatic Complexity"));
    }

    #[test]
    fn test_generate_ai_fix_prompt_out_of_bounds() {
        let files = vec![test_file(
            "src/only.rs",
            Language::Rust,
            50.0,
            15.0,
            10.0,
            100.0,
            60.0,
        )];
        assert!(generate_ai_fix_prompt(&files, 2, Path::new(".")).is_none());
    }

    #[test]
    fn test_generate_ai_fix_prompt_zero_issue() {
        let files = vec![test_file(
            "src/file.rs",
            Language::Rust,
            50.0,
            15.0,
            10.0,
            100.0,
            60.0,
        )];
        assert!(generate_ai_fix_prompt(&files, 0, Path::new(".")).is_none());
    }

    #[test]
    fn test_generate_ai_fix_prompt_empty_files() {
        let files: Vec<FileSimplicity> = vec![];
        assert!(generate_ai_fix_prompt(&files, 1, Path::new(".")).is_none());
    }

    #[test]
    fn test_generate_ai_fix_prompt_mega_file() {
        let files = vec![test_file(
            "src/huge.rs",
            Language::Rust,
            45.0,
            15.0,
            15.0,
            800.0,
            55.0,
        )];

        let prompt = generate_ai_fix_prompt(&files, 1, Path::new(".")).unwrap();
        assert!(prompt.contains("Mega-file"));
    }

    #[test]
    fn test_generate_ai_fix_prompt_low_mi() {
        let files = vec![test_file(
            "src/unmaintainable.rs",
            Language::Rust,
            35.0,
            10.0,
            10.0,
            200.0,
            40.0,
        )];

        let prompt = generate_ai_fix_prompt(&files, 1, Path::new(".")).unwrap();
        assert!(prompt.contains("Low Maintainability Index"));
    }

    #[test]
    fn test_identify_culprit_all_variants() {
        let high_cog = test_file("a.rs", Language::Rust, 30.0, 25.0, 10.0, 100.0, 50.0);
        assert_eq!(identify_culprit(&high_cog), "High Cognitive Complexity");

        let high_cyc = test_file("b.rs", Language::Rust, 30.0, 10.0, 25.0, 100.0, 50.0);
        assert_eq!(identify_culprit(&high_cyc), "High Cyclomatic Complexity");

        let mega = test_file("c.rs", Language::Rust, 30.0, 10.0, 10.0, 600.0, 50.0);
        assert_eq!(identify_culprit(&mega), "Mega-file");

        let low_mi = test_file("d.rs", Language::Rust, 30.0, 10.0, 10.0, 100.0, 40.0);
        assert_eq!(identify_culprit(&low_mi), "Low Maintainability Index");

        let general = test_file("e.rs", Language::Rust, 30.0, 10.0, 10.0, 100.0, 60.0);
        assert_eq!(identify_culprit(&general), "General Complexity");
    }
}
