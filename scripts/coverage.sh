#!/bin/bash
# 
# Fetch the latest branches
echo "Checking code coverage..."
echo
cargo llvm-cov --json --workspace --output-path coverage.json
echo
echo "Generating condensed report..."
cargo run -p ahma_common --features coverage --bin ahma_coverage --quiet
echo
echo "Code coverage reports generated:"
echo " - Full JSON: coverage.json"
echo " - Condensed Markdown: coverage_summary.md (Recommended for LLMs)"
