mod analysis;
mod models;
mod report;

use anyhow::{Context, Result};
use clap::Parser;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use analysis::{
    AnalyzerRegistry, ExternalMetrics, get_project_name, is_cargo_workspace, perform_analysis,
};
use models::{FileSimplicity, MetricsResults, resolve_extensions};
use report::{create_report_md, generate_ai_fix_prompt, generate_report};

#[derive(Parser, Debug)]
#[command(
    name = "ahma-simplify",
    author,
    version,
    about = "Analyzes source code metrics and generates a simplicity report",
    long_about = "Analyzes source code metrics and generates a simplicity report.\n\n\
        Scores are calibrated for AI-assisted maintenance. An AI agent making a change\n\
        must hold the relevant context in its context window; large, deeply nested functions\n\
        increase the risk of misunderstanding and regression. The scoring formula rewards\n\
        decomposed, focused code:\n\n\
          Score = 0.4 × MI + 0.3 × Cognitive Density + 0.2 × Peak Cognitive + 0.1 × Length\n\n\
        MI (40%) — function-weighted Maintainability Index; rewards well-structured decomposition.\n\
        Cognitive Density (30%) — cognitive complexity per SLOC; rewards focused functions.\n\
        Peak Cognitive (20%) — complexity of the single worst function; the primary hotspot signal.\n\
        Length Score (10%) — 100% at ≤300 SLOC, scaling down above; reflects context-window pressure.\n\
        Cyclomatic — reported for context only; already embedded inside MI, not double-counted."
)]
struct Cli {
    /// Directory to analyze (absolute or relative)
    directory: PathBuf,

    /// Output directory for analysis results
    #[arg(short, long, default_value = "analysis_results")]
    output: PathBuf,

    /// Number of issues to show in the report
    #[arg(short, long, default_value_t = 50)]
    limit: usize,

    /// Open the report automatically
    #[arg(long)]
    open: bool,

    /// Shorthand for --format html
    #[arg(long)]
    html: bool,

    /// Shorthand for --html and --open combined
    #[arg(long)]
    heml: bool,

    /// File extensions or language names to analyze, comma-separated.
    /// Accepts raw extensions (e.g. rs,py,kt) or language names (e.g. rust,kotlin,python).
    /// Language names are case-insensitive and expand to all their extensions.
    /// Supported languages: rust, python, javascript, typescript, kotlin, c, c++, java, c#, go, html, css.
    /// Default: all supported extensions.
    #[arg(
        short,
        long,
        default_value = "rs,py,js,ts,tsx,c,h,cpp,cc,hpp,hh,cs,java,go,css,html,kt,kts",
        value_delimiter = ','
    )]
    extensions: Vec<String>,

    /// Additional paths/patterns to exclude, as a comma-separated list.
    /// Example: --exclude "**/generated/**,**/vendor/**"
    #[arg(short = 'x', long, value_delimiter = ',')]
    exclude: Vec<String>,

    /// Disable external language-specific analyzers (e.g. Detekt for Kotlin).
    /// When set, only rust-code-analysis metrics are used. Useful for faster
    /// CI runs or when external tools are not available.
    #[arg(long)]
    no_external: bool,

    /// Output directory for CODE_SIMPLICITY.md and CODE_SIMPLICITY.html files.
    /// If omitted (and --html/--open not set), report is printed to stdout.
    /// When specified, writes files to the given directory.
    #[arg(long)]
    output_path: Option<PathBuf>,

    /// Generate an AI fix prompt for the Nth most complex file (1-indexed).
    /// When set, outputs the full simplicity report and a structured prompt
    /// instructing the AI to plan and implement a fix for that issue.
    #[arg(long)]
    ai_fix: Option<usize>,

    /// Verify improvement by re-analyzing a specific file and comparing
    /// against the baseline from the previous analysis run. Shows before/after
    /// metrics with relative improvement percentages.
    #[arg(long)]
    verify: Option<PathBuf>,
}

fn main() -> Result<()> {
    let mut cli = Cli::parse();

    // If --heml is set, it triggers both --html and --open
    if cli.heml {
        cli.html = true;
        cli.open = true;
    }

    if let Some(ref verify_path) = cli.verify {
        let extensions = resolve_extensions(&cli.extensions);
        return run_verify(verify_path, &cli.output, &cli.directory, &extensions);
    }

    let directory =
        dunce::canonicalize(&cli.directory).context("Failed to canonicalize directory")?;
    prepare_output_directory(&cli.output)?;

    let is_workspace = is_cargo_workspace(&cli.directory);
    let extensions = resolve_extensions(&cli.extensions);

    // Build the external analyzer registry unless the user requested rca-only.
    let registry = build_registry(cli.no_external);
    let registry_ref = if cli.no_external {
        None
    } else {
        Some(&registry)
    };

    let external_metrics = perform_analysis(
        &directory,
        &cli.output,
        is_workspace,
        &extensions,
        &cli.exclude,
        registry_ref,
    )?;

    let mut files_simplicity = load_metrics(&cli.output, true, &external_metrics)?;
    if files_simplicity.is_empty() {
        eprintln!("No analysis files found in {}.", cli.output.display());
        return Ok(());
    }

    sort_files_by_simplicity(&mut files_simplicity);
    let project_name = get_project_name(&directory);

    // Determine output mode: write to file if --output-path, --html, or --open is set
    let write_to_file = cli.output_path.is_some() || cli.html || cli.open;

    if write_to_file {
        let report_output_dir = determine_report_output_dir(&cli.output_path)?;
        fs::create_dir_all(&report_output_dir)
            .context("Failed to create report output directory")?;

        generate_report(
            &files_simplicity,
            is_workspace,
            cli.limit,
            &directory,
            cli.html,
            &project_name,
            &report_output_dir,
        )?;

        print_report_locations(&report_output_dir, cli.html);

        if let Some(issue_number) = cli.ai_fix {
            handle_ai_fix_from_file(
                issue_number,
                &report_output_dir,
                &files_simplicity,
                &directory,
            )?;
        }

        if cli.open
            && let Err(e) = open_report(&report_output_dir, cli.html)
        {
            eprintln!("Warning: Failed to open report: {}", e);
        }
    } else {
        // Default: output markdown to stdout
        let md_content = create_report_md(
            &files_simplicity,
            is_workspace,
            cli.limit,
            &directory,
            &project_name,
        );

        if let Some(issue_number) = cli.ai_fix {
            handle_ai_fix_to_stdout(&md_content, issue_number, &files_simplicity, &directory);
        } else {
            println!("{}", md_content);
        }
    }

    Ok(())
}

fn handle_ai_fix_from_file(
    issue_number: usize,
    report_output_dir: &Path,
    files_simplicity: &[FileSimplicity],
    directory: &Path,
) -> Result<()> {
    let md_path = report_output_dir.join("CODE_SIMPLICITY.md");
    let report_content = fs::read_to_string(&md_path).context("Failed to read generated report")?;
    handle_ai_fix_to_stdout(&report_content, issue_number, files_simplicity, directory);
    Ok(())
}

fn handle_ai_fix_to_stdout(
    report_content: &str,
    issue_number: usize,
    files_simplicity: &[FileSimplicity],
    directory: &Path,
) {
    println!("{}", report_content);

    match generate_ai_fix_prompt(files_simplicity, issue_number, directory) {
        Some(prompt) => println!("\n{}", prompt),
        None => eprintln!(
            "Warning: Issue #{} is out of range (only {} files analyzed).",
            issue_number,
            files_simplicity.len()
        ),
    }
}

/// Build the default external analyzer registry.
///
/// When `no_external` is true returns an empty registry so the caller can
/// still call `perform_analysis` with `None` (no external analysis runs).
fn build_registry(no_external: bool) -> AnalyzerRegistry {
    let mut registry = AnalyzerRegistry::new();
    if !no_external {
        registry.register(Box::new(analysis::detekt::DetektAnalyzer));
    }
    registry
}

fn prepare_output_directory(output: &Path) -> Result<()> {
    if output.exists() {
        eprintln!(
            "Clearing existing analysis results in {}...",
            output.display()
        );
        let _ = fs::remove_dir_all(output);
    }
    fs::create_dir_all(output).context("Failed to create output directory")
}

fn resolve_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .context("Failed to get current directory")?
        .join(path))
}

fn is_file_path(path: &Path) -> bool {
    path.extension().is_some()
        || path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().contains('.'))
}

fn determine_report_output_dir(output_path: &Option<PathBuf>) -> Result<PathBuf> {
    let path = match output_path {
        Some(p) => resolve_path(p)?,
        None => std::env::current_dir().context("Failed to get current directory")?,
    };

    if !is_file_path(&path) {
        return Ok(path);
    }

    path.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("Invalid output path: cannot determine parent directory"))
}

fn try_parse_metrics_file(
    path: &Path,
    normalized: bool,
    external: &HashMap<PathBuf, ExternalMetrics>,
) -> Option<FileSimplicity> {
    let content = fs::read_to_string(path).ok()?;
    match toml::from_str::<MetricsResults>(&content) {
        Ok(results) => {
            let fs_base = FileSimplicity::calculate(&results, normalized);
            // Look up external metrics by the source file's absolute path.
            let fs_merged = match external.get(Path::new(&results.name)) {
                Some(ext) => fs_base.apply_external(ext),
                None => fs_base,
            };
            Some(fs_merged)
        }
        Err(e) => {
            eprintln!("Error parsing {}: {}", path.display(), e);
            None
        }
    }
}

fn load_metrics(
    output: &Path,
    normalized: bool,
    external: &HashMap<PathBuf, ExternalMetrics>,
) -> Result<Vec<FileSimplicity>> {
    eprintln!("Aggregating metrics from {}...", output.display());

    // Phase 1: load rca-based TOML metrics and merge external where available.
    let mut covered_paths: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut files_simplicity: Vec<FileSimplicity> = WalkDir::new(output)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "toml"))
        .filter_map(|e| {
            let result = try_parse_metrics_file(e.path(), normalized, external)?;
            // Track the source path so we don't double-count in Phase 2.
            if let Ok(content) = fs::read_to_string(e.path()) {
                if let Ok(parsed) = toml::from_str::<MetricsResults>(&content) {
                    covered_paths.insert(PathBuf::from(&parsed.name));
                }
            }
            Some(result)
        })
        .collect();

    // Phase 2: create synthetic entries for files covered only by external
    // analyzers (e.g. Kotlin files where rca produced no metrics).
    for (src_path, ext_metrics) in external {
        if covered_paths.contains(src_path) {
            continue;
        }
        if let Some(fs) = FileSimplicity::from_external(src_path, ext_metrics) {
            files_simplicity.push(fs);
        }
    }

    Ok(files_simplicity)
}

fn sort_files_by_simplicity(files: &mut [FileSimplicity]) {
    files.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap()
            .then_with(|| b.cognitive.partial_cmp(&a.cognitive).unwrap())
    });
}

fn print_report_locations(directory: &Path, html: bool) {
    eprintln!(
        "Report generated: {}",
        directory.join("CODE_SIMPLICITY.md").display()
    );
    if html {
        eprintln!(
            "Report generated: {}",
            directory.join("CODE_SIMPLICITY.html").display()
        );
    }
}

fn open_report(directory: &Path, html: bool) -> Result<()> {
    let open_path = if html {
        directory.join("CODE_SIMPLICITY.html")
    } else {
        directory.join("CODE_SIMPLICITY.md")
    };
    opener::open(&open_path).context("Failed to open report")
}

fn run_verify(
    verify_path: &Path,
    output_dir: &Path,
    base_dir: &Path,
    extensions: &[String],
) -> Result<()> {
    let abs_verify = if verify_path.is_absolute() {
        verify_path.to_path_buf()
    } else {
        std::env::current_dir()?.join(verify_path)
    };
    let canonical_verify = dunce::canonicalize(&abs_verify)
        .with_context(|| format!("File not found: {}", verify_path.display()))?;

    let baseline = find_baseline_metrics(output_dir, &canonical_verify)?;
    let baseline_simplicity = FileSimplicity::calculate(&baseline, true);

    let temp_output = tempfile::tempdir().context("Failed to create temp directory")?;
    let parent_dir = canonical_verify
        .parent()
        .context("Cannot determine parent directory")?;
    analysis::run_analysis(parent_dir, temp_output.path(), extensions, &[], None)?;

    let current = find_baseline_metrics(temp_output.path(), &canonical_verify)?;
    let current_simplicity = FileSimplicity::calculate(&current, true);

    let rel_path = analysis::get_relative_path(
        &canonical_verify,
        &dunce::canonicalize(base_dir).unwrap_or(base_dir.to_path_buf()),
    );
    print_verification(
        &rel_path.to_string_lossy(),
        &baseline_simplicity,
        &current_simplicity,
    );

    Ok(())
}

fn find_baseline_metrics(output_dir: &Path, target_path: &Path) -> Result<MetricsResults> {
    let target_str = target_path.to_string_lossy();

    WalkDir::new(output_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "toml"))
        .find_map(|entry| {
            let content = fs::read_to_string(entry.path()).ok()?;
            let results: MetricsResults = toml::from_str(&content).ok()?;
            (results.name == target_str).then_some(results)
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No baseline metrics found for {} in {}. Run a full analysis first.",
                target_str,
                output_dir.display()
            )
        })
}

fn print_verification(path: &str, before: &FileSimplicity, after: &FileSimplicity) {
    println!("=== VERIFICATION: {} ===\n", path);
    println!("BEFORE -> AFTER (CHANGE)");

    print_metric_row("Simplicity", before.score, after.score, "%", true);
    print_metric_row("  MI 40%", before.mi, after.mi, "", true);
    print_metric_row(
        "  Cognitive density 30%",
        before.cognitive,
        after.cognitive,
        "",
        false,
    );
    print_metric_row(
        "  Peak cognitive 20%",
        before.peak_cognitive,
        after.peak_cognitive,
        "",
        false,
    );
    print_metric_row("  SLOC / length 10%", before.sloc, after.sloc, "", false);
    print_metric_row(
        "Cyclomatic (info only)",
        before.cyclomatic,
        after.cyclomatic,
        "",
        false,
    );

    println!();
    print_verdict(before.score, after.score);
}

fn print_verdict(before_score: f64, after_score: f64) {
    let improvement = after_score - before_score;
    let msg = if improvement > 5.0 {
        "VERDICT: Significant improvement achieved."
    } else if improvement > 0.0 {
        "VERDICT: Modest improvement. Consider further refactoring."
    } else if improvement == 0.0 {
        "VERDICT: No change detected."
    } else {
        "VERDICT: Regression detected - complexity increased."
    };
    println!("{}", msg);
}

fn get_direction_label(pct: f64, higher_is_better: bool) -> &'static str {
    let is_positive = pct > 0.0;
    match (higher_is_better, is_positive) {
        (true, true) => "improvement",
        (true, false) => "regression",
        (false, true) => "increase",
        (false, false) => "reduction",
    }
}

fn format_metric_change(before: f64, after: f64, suffix: &str, higher_is_better: bool) -> String {
    if before == 0.0 {
        if after == 0.0 {
            return "unchanged".to_string();
        }
        return format!("+{:.0}{}", after, suffix);
    }

    let pct = ((after - before) / before) * 100.0;
    let label = get_direction_label(pct, higher_is_better);
    format!("{:.0}% {}", pct, label)
}

fn print_metric_row(label: &str, before: f64, after: f64, suffix: &str, higher_is_better: bool) {
    let change = format_metric_change(before, after, suffix, higher_is_better);
    println!(
        "  {:12} {:>6.0}{} -> {:>6.0}{} ({})",
        label, before, suffix, after, suffix, change
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing() {
        let args = vec!["ahma_simplify", ".", "--output", "results"];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.directory, PathBuf::from("."));
        assert_eq!(cli.output, PathBuf::from("results"));
        assert_eq!(cli.output_path, None);
    }

    #[test]
    fn test_cli_parsing_with_output_path() {
        let args = vec![
            "ahma_simplify",
            ".",
            "--output",
            "results",
            "--output-path",
            "/tmp",
        ];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.directory, PathBuf::from("."));
        assert_eq!(cli.output, PathBuf::from("results"));
        assert_eq!(cli.output_path, Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn test_cli_parsing_with_ai_fix() {
        let args = vec!["ahma_simplify", ".", "--ai-fix", "1"];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.ai_fix, Some(1));
    }

    #[test]
    fn test_cli_parsing_without_ai_fix() {
        let args = vec!["ahma_simplify", "."];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.ai_fix, None);
    }

    #[test]
    fn test_cli_parsing_with_verify() {
        let args = vec!["ahma_simplify", ".", "--verify", "src/main.rs"];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.verify, Some(PathBuf::from("src/main.rs")));
    }

    #[test]
    fn test_cli_parsing_without_verify() {
        let args = vec!["ahma_simplify", "."];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.verify, None);
    }
}
