# ahma-simplify — Requirements

## Purpose

`ahma-simplify` is a code simplicity metrics aggregator that uses the [rust-code-analysis](https://github.com/mozilla/rust-code-analysis) library to analyze source code and generate comprehensive simplicity reports (Markdown and HTML).

It is a workspace member of the [Ahma](../SPEC.md) project.

## Functional Requirements

### R1: Analysis Library

- **R1.1**: The tool links the `rust-code-analysis` library at compile time — no external CLI binary is required.
- **R1.2**: _(Removed)_ Previously required a PATH check; no longer applicable.

### R2: Analysis Scope

- **R2.1**: Accepts a directory path as the primary argument.
- **R2.2**: Detects Cargo workspaces and analyzes each member crate individually.
- **R2.3**: Falls back to single-directory analysis when no workspace is detected.
- **R2.4**: Supports multi-language analysis via `--extensions` flag (default: all supported extensions).
- **R2.5**: Applies default exclusion patterns (target, node_modules, build dirs, VCS, IDE configs).
- **R2.6**: Supports additional user-defined exclusion patterns via `--exclude`.

### R3: Simplicity Scoring

- **R3.1**: Each file receives a composite simplicity score (0–100%) using:
  ```
  Score = 0.6 × MI + 0.2 × Cog_Score + 0.2 × Cyc_Score
  ```
- **R3.2**: Cognitive and cyclomatic complexity are SLOC-normalized (density per 100 lines).
- **R3.3**: Trivial/empty files (MI=0, cognitive=0, cyclomatic≤1) receive a perfect 100% score.
- **R3.4**: When the file-level Maintainability Index is 0 (common for large files where `mi_original` goes negative), the score uses a SLOC-weighted average of function-level MI values as a fallback.

### R4: Report Generation

- **R4.1**: By default, outputs markdown report to **stdout**. When `--output-path`, `--html`, or `--open` is specified, writes `CODE_SIMPLICITY.md` to the specified directory (or current working directory if only `--html`/`--open` is used).
- **R4.2**: `--html` flag additionally generates `CODE_SIMPLICITY.html` with styled output (implies file output mode).
- **R4.3**: Reports include: overall simplicity, per-language breakdown, per-crate/package scores, top N issues, and a metrics glossary.
- **R4.4**: `--limit N` controls how many issues are listed (default: 10).
- **R4.5**: `--open` flag opens the report in the default system viewer (implies file output mode).
- **R4.6**: Each file in the complexity issues section includes **function-level hotspots** — the top 5 functions by cognitive complexity with their line ranges, cognitive, cyclomatic, and SLOC metrics.
- **R4.7**: Test files (`*_test.rs`, `**/tests/**`) are intentionally included in complexity reports. Complex tests are a maintenance burden: they obscure debugging, discourage adding new test cases, and often indicate overly complex production APIs.
- **R4.8**: `--output-path <dir>` explicitly requests file output to the specified directory.

### R5: Output Directory

- **R5.1**: Intermediate analysis output (TOML files) is stored in a configurable directory (`--output`, default: `analysis_results`).
- **R5.2**: The output directory is cleared before each run.

### R6: AI Fix Prompt

- **R6.1**: `--ai-fix N` generates a structured prompt for the Nth most complex file.
- **R6.2**: The prompt references the report for hotspot details rather than duplicating them.
- **R6.3**: The prompt constrains the AI to focus only on hotspot functions, making minimal changes.

### R7: Verification

- **R7.1**: `--verify <file>` re-analyzes a specific file and compares against the previous baseline.
- **R7.2**: Displays before/after metrics with relative improvement percentages for simplicity, cognitive, cyclomatic, SLOC, and MI.
- **R7.3**: Reports a verdict: significant improvement, modest improvement, no change, or regression.

## Non-Functional Requirements

- **NF1**: The tool is a synchronous CLI binary (no async runtime needed).
- **NF2**: Uses workspace-level dependency versions where available.
- **NF3**: All tests must be deterministic and fast (<100ms each).

## CI Integration

The `job-coverage` step in `.github/workflows/build.yml` runs `ahma-simplify` and publishes `CODE_SIMPLICITY.md` / `CODE_SIMPLICITY.html` to GitHub Pages alongside the coverage report.
