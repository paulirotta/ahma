# AHMA Tool Configuration Examples

This directory contains example Rust programs that demonstrate how to validate tool configurations for the AHMA MCP server.

## Example Programs

Each example loads and validates a specific tool configuration from the `configs/` directory:

- **[cargo_tool.rs](cargo_tool.rs)** - Validates `configs/cargo.json` (Rust build tool)
- **[file-tools.rs](file-tools.rs)** - Validates `configs/file-tools.json` (Unix file operations)
- **[gh_tool.rs](gh_tool.rs)** - Validates `configs/gh.json` (GitHub CLI)
- **[git_tool.rs](git_tool.rs)** - Validates `configs/git.json` (Git version control)
- **[kotlin_tool.rs](kotlin_tool.rs)** - Validates `configs/kotlin.json` (Kotlin/Android Gradle wrapper)
- **[python_tool.rs](python_tool.rs)** - Validates `configs/python.json` (Python interpreter)

## Running Examples

Run any example using cargo:

```bash
# Run a specific example
cargo run --example cargo_tool
cargo run --example file-tools
cargo run --example gh_tool
cargo run --example git_tool
cargo run --example gradlew_tool
cargo run --example python_tool
```

## Example Output

When you run an example, you'll see output like:

```
Loading cargo tool configuration from: /path/to/ahma-mcp/examples/configs/cargo.json
OK Configuration is valid!

📋 Tool Details:
   Name: cargo
   Description: Rust's build tool and package manager
   Command: cargo
   Enabled: true
   Subcommands: 14

🔧 Available Subcommands:
   - build: Compile the current package.
   - run: Run a binary or example of the local package.
   - add: Add dependencies to a Cargo.toml manifest file.
   ...
```

## Configuration Files

All configuration JSON files are located in the [configs/](configs/) directory and follow the MCP Tool Definition Format (MTDF) schema. These configs have `"enabled": true` set and can be copied to the `.ahma/` directory for actual use.

## Validation

The examples use the `MtdfValidator` from `ahma-mcp::schema_validation` to validate configurations against the MTDF schema. This ensures:

- Required fields are present
- Field types are correct
- Values are within acceptable ranges
- Logical consistency is maintained

## Testing

Integration tests verify that all examples run successfully:

```bash
# Run schema validation tests
cargo nextest run --package ahma-mcp --test tool_config_schema_validation_test

# Run execution tests
cargo nextest run --package ahma-mcp --test tool_examples_execution_test
```

## Creating New Tool Configurations

To add a new tool configuration:

1. Create `configs/newtool.json` following the MTDF schema
2. Create `newtool.rs` example based on existing examples
3. Add to `Cargo.toml`:
   ```toml
   [[example]]
   name = "newtool"
   path = "examples/newtool.rs"
   ```
4. Add tests in `tests/tool_config_schema_validation_test.rs`
5. Add execution tests in `tests/tool_examples_execution_test.rs`

## See Also

- [/.ahma/README.md](../../.ahma/README.md) - Comprehensive guide to tool configurations
- [/docs/mtdf-schema.json](/docs/mtdf-schema.json) - JSON Schema definition
- [ahma-mcp/src/schema_validation.rs](../src/schema_validation.rs) - Validation implementation
