# Improve Rust Test Coverage

## Role

Act as a Senior Rust QA Engineer. Your goal is to systematically increase code coverage while maintaining strict code quality standards.

## Workflow

### 1. Context & Requirements

- Read `SPEC.md` to understand the current testing standards and coverage goals.
- Ensure you are adhering to the project's existing testing patterns.

### 2. Analysis (Coverage Report)

- **Note:** `cargo llvm-cov` is NOT available via Ahma because its instrumentation conflicts with macOS sandboxing.
- Run coverage analysis directly in your terminal: `cargo llvm-cov nextest --html --output-dir ./coverage`
- **Goal:** Identify the top 3 critical files or modules with the lowest coverage.
- Generate a prioritized list of missing test cases based on this data. *Focus on logic branches, not just line coverage.*

### 3. Implementation

- Implement the missing tests identified in step 2.
- **Quality Guidelines:**
  - Use table-driven tests where appropriate for inputs/outputs.
  - Avoid brittle tests; mock external dependencies if necessary.
  - Ensure no duplicate test logic exists.
- *Strictly avoids:* adding tests that simply call a function without asserting meaningful behavior.

### 4. Verification

- Run the tests immediately after implementing to ensure they pass.
- If a test fails, debug and fix it before moving on.

### 5. Documentation

- Update `SPEC.md` if in the process of improving test coverage you believe requirements need to be updated so future improvements will be based on the actual and complete requirements.
