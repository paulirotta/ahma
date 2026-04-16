# Requirements Tracking

## Complexity Workflow

- Date: 2026-04-16
- Issue: #6
- Target: `ahma_mcp/src/test_utils/client.rs`
- Status: Edited
- Rationale: Source utility code had genuine concentrated complexity in startup command construction/configuration hotspots (`get_test_binary_path`, `build`, `run_command`), so targeted flattening/refactoring was warranted.
- Validation: simplify verify reports `Simplicity 39% -> 40%`, `Cognitive 42 -> 33`, `Cyclomatic 56 -> 55`; focused `cargo check -p ahma_mcp` passed.
- Next Step: Continue to next issue when requested.

- Date: 2026-04-16
- Issue: #1
- Target: `ahma_http_bridge/tests/common/server.rs`
- Status: Edited (minimal hotspot refactor)
- Rationale: Not a many-small-tests file; this is shared test server infrastructure where hotspot control flow was genuinely simplifiable without API/behavior changes.
- Validation: `simplify verify` reported significant improvement (36% -> 68%).
- Focused Check: Launched `cargo check -p ahma_http_bridge` via `sandboxed_shell` (AHMA op id: `op_8`).
- Next Step: Continue to next issue when requested.

- Date: 2026-04-16
- Issue: #9
- Target: `ahma_http_bridge/tests/sse_streaming_test.rs`
- Status: Skipped (no code edits)
- Rationale: Test-only file with assertion-heavy SSE coverage; high complexity score is primarily driven by test scenario breadth and expected control-flow checks, not production algorithm complexity.
- Validation: Ran simplify verify on target file; metrics unchanged as expected.
- Next Step: Continue to next issue when requested.

- Date: 2026-04-16
- Issue: #7
- Target: `ahma_mcp/tests/test_utils_coverage_test.rs`
- Status: Skipped (no code edits)
- Rationale: Test-only file with many small, focused coverage tests. Complexity is distributed across case-enumeration and assertions, which is expected and desirable for test breadth; no concentrated hotspot function warranted refactoring.
- Validation: Ran simplify verify on target file; metrics unchanged as expected.
- Next Step: Continue to next issue when requested.

- Date: 2026-04-16
- Issue: #8
- Target: `ahma_http_bridge/tests/fast_error_response_test.rs`
- Status: Skipped (no code edits)
- Rationale: Test-only file with explicit request/response scenarios and assertion-heavy checks. Reported complexity is distributed across test coverage paths (initialization, timing checks, transport handling), not concentrated algorithmic logic that would benefit from extraction.
- Validation: Run simplify verify on target file and report unchanged metrics.
- Next Step: Continue to next issue when requested.
