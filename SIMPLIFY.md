# Code Complexity Analysis

Ahma includes a built-in code complexity analyzer (`ahma-mcp simplify`) that scores every source
file in your project, identifies the worst hotspot functions, and returns a structured AI prompt
to fix them with minimal, targeted changes.

Supports: **Rust, Python, JavaScript, TypeScript, Kotlin, C, C++, Java, C#, Go, CSS, HTML**.

---

## Quick Start

```bash
# Analyze the current directory and get fix instructions for the worst file
ahma-mcp simplify . --ai-fix 1

# Rust files only
ahma-mcp simplify . --extensions rust --ai-fix 1

# Verify improvement after editing
ahma-mcp simplify . --verify src/my_module.rs
```

Or via the `simplify` MCP tool (requires `--tools simplify` or `--tools rust,simplify` at
server startup):

```
simplify(directory=".", ai_fix=1)
```

---

## Installation

`ahma-mcp simplify` is built into the `ahma-mcp` binary — no separate install needed.

**Quick install (Linux/macOS):**
```bash
curl -sSf https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.sh | bash
```

**Windows (PowerShell 5.1+):**
```powershell
irm https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.ps1 | iex
```

**From source:**
```bash
cargo build --release -p ahma_mcp
```

The `simplify` feature is enabled by default. To build without it (smaller binary):
```bash
cargo build --release -p ahma_mcp --no-default-features
```

---

## Usage

### Basic analysis

```bash
# Analyze all supported files, get fix prompt for the worst file
ahma-mcp simplify <directory> --ai-fix 1

# Get fix prompt for the 2nd worst file
ahma-mcp simplify <directory> --ai-fix 2

# Show top 20 issues in the report (default: 50)
ahma-mcp simplify <directory> --limit 20
```

### Language filtering

```bash
# Single language (name or raw extension)
ahma-mcp simplify . --extensions rust
ahma-mcp simplify . --extensions rs

# Multiple languages
ahma-mcp simplify . --extensions rust,python

# Kotlin only
ahma-mcp simplify . --extensions kotlin
```

| Language name | Extensions scanned |
|---------------|--------------------|
| `rust` | `.rs` |
| `kotlin` | `.kt`, `.kts` |
| `python` | `.py` |
| `javascript` | `.js`, `.jsx` |
| `typescript` | `.ts`, `.tsx` |
| `java` | `.java` |
| `c++` / `cpp` | `.cpp`, `.cc`, `.hpp`, `.hh` |
| `c#` / `csharp` | `.cs` |
| `go` | `.go` |
| `html` | `.html`, `.htm` |
| `css` | `.css` |

### Verification

After editing a file, re-analyze it to confirm improvement:

```bash
ahma-mcp simplify <directory> --verify src/my_module.rs
```

Output shows before/after metrics with a verdict:

| Verdict | Meaning |
|---------|---------|
| Significant improvement (≥10%) | Success — move to next issue |
| Modest improvement (1–9%) | Acceptable |
| No change | Hotspot functions may not have been modified |
| Regression | Revert and try a different approach |

### Report output

```bash
# Write report to a directory (CODE_SIMPLICITY.md + CODE_SIMPLICITY.html)
ahma-mcp simplify <directory> --output-path ./reports

# Generate HTML report
ahma-mcp simplify <directory> --html

# Exclude generated code
ahma-mcp simplify <directory> --exclude '**/generated/**,**/vendor/**'
```

---

## Score Interpretation

Each file receives a composite score (0–100%):

```
Score = 0.4 × MI + 0.3 × Cognitive Density + 0.2 × Peak Cognitive + 0.1 × Length Score
```

| Component | Weight | What it measures |
|-----------|--------|-----------------|
| **MI** | 40% | Function-weighted Maintainability Index; rewards decomposed, well-structured code |
| **Cognitive Density** | 30% | Cognitive complexity normalised by SLOC; rewards focused, readable functions |
| **Peak Cognitive** | 20% | Cognitive complexity of the single worst function |
| **Length Score** | 10% | 100% at ≤300 SLOC, scaling down linearly above that |

| Score Range | Status | Guidance |
|-------------|--------|----------|
| 85–100% | Excellent | No action needed |
| 70–84% | Good | Acceptable; fix only the worst outliers |
| 55–69% | Fair | Plan a simplification sprint |
| 40–54% | Poor | Prioritize before adding features |
| 0–39% | Critical | Address now; maintenance cost is high |

A project score below 70% is a signal to run `--ai-fix` on the top 3–5 files.

---

## MCP Tool Reference

Tool name: `simplify` (requires `--tools simplify` at ahma-mcp startup).

| Argument | Type | Default | Purpose |
|----------|------|---------|---------|
| `directory` | path (required) | — | Project root to analyze |
| `ai_fix` | integer | — | Issue number for fix prompt (1 = worst file) |
| `limit` | integer | 50 | Issues to include in report |
| `verify` | path | — | Re-analyze a file vs. baseline |
| `extensions` | array | all | Restrict to file types (e.g. `["rs","py"]`) |
| `exclude` | array | — | Additional glob patterns to exclude |
| `output_path` | path | — | Write report to directory instead of stdout |
| `html` | boolean | false | Also generate HTML report |

### MCP tool invocation examples

```
# Via /ahma simplify skill subcommand
/ahma simplify
/ahma simplify rust
/ahma simplify kotlin 2

# Direct MCP tool call
simplify(directory=".", ai_fix=1)
simplify(directory=".", extensions=["rs"], ai_fix=1)
simplify(directory=".", verify="src/my_module.rs")
```

---

## AI Workflow

When using the `simplify` MCP tool or `/ahma simplify` chat command, follow this sequence:

1. **Run analysis** — `simplify(directory=".", ai_fix=1)`
2. **Read the structured fix prompt** in the output — it lists exact hotspot functions and constraints
3. **Apply targeted changes** to only the listed hotspot functions
4. **Verify improvement** — `simplify(directory=".", verify="<edited-file>")`
5. **Iterate** — move to `ai_fix=2`, `ai_fix=3`, etc.

### Anti-patterns to avoid

- Do not refactor the whole file; follow the hotspot list exactly
- Do not add comments to improve scores — structural change is required
- Do not inline complex logic to reduce function count
- Do not skip the verify step — metric confirmation is required

---

## CI Integration

The project itself tracks code simplicity on every push. The CI report is published at:
[paulirotta.github.io/ahma/CODE_SIMPLICITY.html](https://paulirotta.github.io/ahma/CODE_SIMPLICITY.html)

To add simplify to your own CI pipeline:

```yaml
- name: Code Simplicity Report
  run: ahma-mcp simplify . --limit 20 --html --output-path ./simplicity-report
```

---

## See Also

- [docs/agent-skills.md](docs/agent-skills.md) — AI agent skill configuration
- [README.md](README.md) — Main Ahma documentation
- [SPEC.md](SPEC.md) — Full specification and architecture
