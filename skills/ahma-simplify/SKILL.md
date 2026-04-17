---
name: ahma-simplify
version: 0.5.6
author: Paul Houghton
description: >
  Use this skill when the user asks about code complexity, simplification, maintainability, or
  refactoring. Trigger phrases: "simplify", "reduce complexity", "too complex", "hard to read",
  "refactor", "maintainability", "cognitive complexity", "cyclomatic complexity", "what's the
  most complex file", "code quality metrics", "simplicity score", "ahma-simplify", "hotspot".
  Runs ahma-simplify (via the simplify MCP tool or CLI) to score every file 0-100%, identifies
  the worst hotspot functions, and returns a structured prompt to fix them with minimal, targeted
  changes. Always verifies improvement after editing.
user-invocable: true
---

<!-- version: 0.5.6 | author: Paul Houghton -->

# ahma-simplify Skill

Analyze code complexity across any supported language, identify the worst hotspot functions, fix
them with minimal targeted changes, and verify measurable improvement.

Supports: Rust, Python, JavaScript, TypeScript, Kotlin, C, C++, Java, C#, Go, CSS, HTML.

---

## When This Skill Applies

Auto-load this skill when the user:
- Asks to simplify, clean up, or reduce complexity in any file or directory
- Mentions "cognitive complexity", "cyclomatic complexity", or "maintainability index"
- Asks which file or function is hardest to read or maintain
- Wants a code quality report or simplicity score
- Asks you to refactor for readability without changing behavior
- Uses the `simplify` MCP tool or `ahma-simplify` CLI directly

Skip this skill for:
- Pure functional changes (adding/removing features)
- Performance optimization unrelated to code clarity
- Style changes like formatting or renaming

---

## Prerequisites

One of the following must be available:

**Option A — MCP tool (preferred in VS Code / Cursor / Claude Code):**
The `simplify` tool is active in the current ahma-mcp session (enabled with `--tools simplify`
or `--tools rust,simplify`).

**Option B — Direct CLI:**
`ahma-simplify` is on PATH. Install: `cargo install --path ahma_simplify` or use the install
script at `scripts/install.sh` / `scripts/install.ps1`.

To check availability, run: `ahma-simplify --version`

---

## Language Filtering

To scope the scan to a specific language, mention it after the skill name:

```
/ahma-simplify rust
/ahma-simplify kotlin
/ahma-simplify rust python
```

The language name is passed as the `extensions` argument. Both language names and raw
extensions work, and names are case-insensitive:

| You write | Extensions scanned |
|-----------|--------------------|
| `rust` | `.rs` |
| `kotlin` | `.kt`, `.kts` |
| `python` | `.py` |
| `javascript` | `.js`, `.jsx` |
| `typescript` | `.ts`, `.tsx` |
| `java` | `.java` |
| `c++` or `cpp` | `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh` |
| `c#` or `csharp` | `.cs` |
| `go` | `.go` |
| `html` | `.html`, `.htm` |
| `css` | `.css` |

Omitting a language filter scans all supported extensions automatically.

When the user says `/ahma-simplify rust`, translate to:

**Via MCP tool:**
```
simplify(directory="<project-root>", extensions=["rs"], ai_fix=1)
```

**Via CLI:**
```
ahma-simplify <project-root> --extensions rust --ai-fix 1
```

---

## Core Workflow

Follow this sequence. Do not skip steps.

### Step 1 — Run complexity analysis

**Via MCP tool:**
```
simplify(directory="<project-root>", ai_fix=1)
```

**Via CLI:**
```
ahma-simplify <project-root> --ai-fix 1
```

The tool returns:
1. An overall project simplicity score (0–100%)
2. A ranked list of files by complexity (worst first)
3. Function-level hotspots for the top issue (top 5 functions by cognitive complexity)
4. A structured fix prompt for that specific file

### Step 2 — Read the structured fix prompt

The `--ai-fix N` output ends with a structured prompt block. It contains:
- The exact file path to edit
- The hotspot functions (by name, line range, and metrics)
- Constraints for what to change

**Follow the prompt's constraints exactly:**
- Edit **only** the listed hotspot functions
- Do not refactor the whole file
- Do not change function signatures, public APIs, or behavior
- Do not reduce line count at the expense of clarity

### Step 3 — Apply targeted changes

Common patterns for reducing complexity:
- Extract deeply nested logic into well-named helper functions
- Replace complex boolean chains with named predicates
- Replace multi-branch match/switch arms with a lookup table or strategy function
- Flatten early-return cascades (guard clauses)
- Break apart functions with high SLOC alongside high cognitive complexity

**For test files specifically:** If a hotspot is a test file with many small test functions,
skip it. High test count is expected; do not consolidate tests. If a single test function is
individually complex (large setup, many assertions), consider splitting it.

### Step 4 — Verify improvement

After editing, re-analyze the modified file:

**Via MCP tool:**
```
simplify(directory="<project-root>", verify="<path-to-edited-file>")
```

**Via CLI:**
```
ahma-simplify <project-root> --verify <path-to-edited-file>
```

The output shows before/after metrics with a verdict:
- **Significant improvement** (≥10% score gain) — success, continue
- **Modest improvement** (1–9% gain) — acceptable, move to next issue
- **No change** — review if the hotspot functions were actually modified
- **Regression** — revert and try a different approach

### Step 5 — Iterate

Move to the next most complex file:

```
simplify(directory="<project-root>", ai_fix=2)
```

Or CLI: `ahma-simplify <project-root> --ai-fix 2`

Continue iterating until the project score is satisfactory or the user stops.

---

## Score Interpretation

Each file receives a composite score:

```
Score = 0.6 × Maintainability Index + 0.2 × Cognitive Score + 0.2 × Cyclomatic Score
```

| Score Range | Status | Guidance |
|-------------|--------|----------|
| 85–100% | Excellent | No action needed |
| 70–84% | Good | Acceptable; fix only the worst outliers |
| 55–69% | Fair | Plan a simplification sprint |
| 40–54% | Poor | Prioritize before adding features |
| 0–39% | Critical | Address now; maintenance cost is high |

A project overall score below 70% is a signal to run `--ai-fix` on the top 3–5 files.

---

## MCP Tool Reference

Tool name: `simplify`

| Argument | Type | Default | Purpose |
|----------|------|---------|---------|
| `directory` | path (required) | — | Project root to analyze |
| `ai_fix` | integer | — | Issue number to generate fix prompt for (1 = worst file) |
| `limit` | integer | 50 | Number of issues to include in report |
| `verify` | path | — | Re-analyze a file and compare to baseline |
| `extensions` | array | all | Restrict to specific file types (e.g. `["rs","py"]`) |
| `exclude` | array | — | Additional glob patterns to exclude |
| `output_path` | path | — | Write report to directory instead of stdout |
| `html` | boolean | false | Also generate HTML report |

---

## Extended Documentation

If you need additional detail on any topic, read the adjacent files in this skill folder:
- `scoring-formula.md` — Full derivation of the composite score and edge cases
- `hotspot-guide.md` — Detailed guidance on each hotspot function metric

---

## Anti-Patterns to Avoid

1. **Do not refactor the whole file** when only 1–2 functions are hotspots. Follow the fix
   prompt's hotspot list exactly.

2. **Do not add comments to reduce complexity scores.** The Maintainability Index is not
   improved by comments alone; structural simplification is needed.

3. **Do not inline complex logic to reduce function count.** Fewer functions with more
   complexity each makes scores worse, not better.

4. **Do not run `--ai-fix` without reading the structured prompt.** The prompt contains
   file-specific context that prevents generic, incorrect refactors.

5. **Do not skip Step 4 (verify).** Complexity improvements are only real if the metrics
   confirm it. Syntactically cleaner code can still have higher cognitive complexity.

---

## Quick Reference

```bash
# Analyze and get fix prompt for worst file
ahma-simplify . --ai-fix 1

# Rust files only (language name or raw extension both work)
ahma-simplify . --extensions rust --ai-fix 1
ahma-simplify . --extensions rs --ai-fix 1

# Kotlin files only
ahma-simplify . --extensions kotlin --ai-fix 1

# Multiple languages
ahma-simplify . --extensions rust,python --ai-fix 1

# Analyze and get fix prompt for 2nd worst file
ahma-simplify . --ai-fix 2

# Verify improvement after editing
ahma-simplify . --verify src/my_module.rs

# Full report to file
ahma-simplify . --output-path ./reports

# HTML report, open in browser
ahma-simplify . --heml

# Exclude generated code
ahma-simplify . --exclude '**/generated/**,**/vendor/**' --ai-fix 1
```
