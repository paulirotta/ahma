# ahma_simplify

A code simplicity metrics aggregator that analyzes source code using the [rust-code-analysis](https://github.com/mozilla/rust-code-analysis) library and generates comprehensive simplicity reports.

Part of the [Ahma](../README.md) workspace.

## Installation

```bash
# Install ahma_simplify (all analysis dependencies are compiled in)
cargo install --path ahma_simplify
```

## Usage

```bash
# Analyze a single crate
ahma_simplify /path/to/crate

# Analyze with HTML report
ahma_simplify /path/to/project --html

# Custom output directory
ahma_simplify /path/to/project -o my_results

# Limit emergency items shown
ahma_simplify /path/to/project --limit 5

# Open report automatically
ahma_simplify /path/to/project --html --open

# Analyze multiple languages (comma-separated list)
ahma_simplify /path/to/project --extensions rs,py,js

# All supported languages example
ahma_simplify /path/to/project --extensions rs,py,js,ts,tsx,c,h,cpp,cc,hpp,hh,cs,java,go,css,html

# Exclude custom paths (comma-separated list)
ahma_simplify /path/to/project --exclude "**/generated/**,**/vendor/**"

# Convenience wrapper script (analyzes the whole repo)
./scripts/code-simplicity.sh
```

## Scoring Formula

Each file receives a simplicity score (0-100%) based on:

```
Score = 0.6 × MI + 0.2 × Cog_Score + 0.2 × Cyc_Score
```

Where:
- **MI**: Maintainability Index (Visual Studio variant, 0-100)
- **Cog_Score**: `(100 - (cognitive / sloc * 100)).max(0.0)`
- **Cyc_Score**: `(100 - (cyclomatic / sloc * 100)).max(0.0)`

### Weighting Rationale

| Metric | Weight | Rationale |
|--------|--------|-----------|
| Maintainability Index | 60% | Composite metric already balancing complexity, volume, and LOC |
| Cognitive Complexity | 20% | Measures comprehension difficulty per 100 lines |
| Cyclomatic Complexity | 20% | Measures testability per 100 lines |

### Normalization

Complexity metrics are normalized by SLOC to calculate density per 100 lines. This ensures small dense files are penalized more than large sparse files with the same total complexity.

### Known Limitations

1. **Double-counting**: MI already incorporates cyclomatic complexity in its formula, so cyclomatic is partially double-weighted.

2. **Clamping**: Density >100 per 100 lines is treated the same as density=100 for scoring purposes.

## Output

### Markdown Report (`CODE_SIMPLICITY.md`)

- Overall repository simplicity percentage
- Per-crate/package simplicity breakdown
- Top N "code complexity issues" with culprit identification
- Metrics glossary

### HTML Report (`CODE_SIMPLICITY.html`)

Styled HTML version of the markdown report (use `--html` flag).

## Metrics Explained

| Metric | Description | Good | Concerning |
|--------|-------------|------|------------|
| Cognitive Complexity | How hard to understand control flow | <10 | >20 |
| Cyclomatic Complexity | Number of independent paths | <10 | >20 |
| SLOC | Source lines of code | <300 | >500 |
| Maintainability Index | Ease of maintenance (higher=better) | >50 | <30 |

## Workspace vs Single Crate

- **Workspace detected**: Reports "Simplicity by Crate" with per-member breakdown
- **Single crate**: Reports "Simplicity by Package" with directory-based grouping

## Multi-Language Support

Analyzes multiple languages using a comma-separated list of extensions via the `--extensions` (or `-e`) flag.

**Supported languages:**
- **Rust**: `rs`
- **Python**: `py`
- **JavaScript**: `js`
- **TypeScript**: `ts`, `tsx`
- **C**: `c`, `h`
- **C++**: `cpp`, `cc`, `hpp`, `hh`
- **C#**: `cs`
- **Java**: `java`
- **Go**: `go`
- **CSS**: `css`
- **HTML**: `html`

Example usage:
```bash
ahma_simplify . --extensions rs,py,js
```

## Development

```bash
# Run tests
cargo test -p ahma_simplify

# Build
cargo build -p ahma_simplify --release
```
