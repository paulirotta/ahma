# Ahma Tool Configurations

This directory contains tool configuration files for the Ahma server (ahma_mcp). These configurations define how AI agents can interact with various command-line tools in a safe and structured way.

## How Tool Loading Works

AHMA has a three-tier tool model:

### 1. Core Built-in Tools (always available, no configuration needed)
- **sandboxed_shell** — Execute shell commands in the security sandbox
- **await** — Wait for async operations to complete
- **status** — Query operation status without blocking
- **cancel** — Cancel running operations

These are implemented directly in Rust and cannot be overridden by JSON configurations. Their names are reserved.

### 2. Bundled Tool Configs (opt-in via CLI flags)
Standard tool configurations are compiled into the `ahma_mcp` binary. They are only offered to MCP clients when explicitly enabled via a CLI flag:

| Flag | Tool Name | Description |
|------|-----------|-------------|
| `--rust` | `cargo` | Rust build, test, clippy, fmt, etc. |
| `--fileutils` | `file-tools` | Unix file operations (ls, cp, mv, rm, grep, etc.) |
| `--git` | `git` | Git version control |
| `--github` | `gh` | GitHub CLI (PRs, issues, releases) |
| `--python` | `python` | Python interpreter and pip |
| `--gradle` | `gradlew` | Android Gradle wrapper |
| `--simplify` | `simplify` | Code complexity metrics |

Example: `ahma_mcp --mode stdio --rust --git --fileutils`

### 3. Local `.ahma/` Overrides (automatic)
If a `.ahma/` directory exists in the current working directory, all `*.json` files in it are loaded automatically — no CLI flag needed.

**Override rule:** If a local `.ahma/*.json` file defines a tool with the same `name` as a bundled tool, the local version **replaces** the bundled one entirely. This lets you customize tool descriptions, options, and subcommands for your project.

Example: placing a `.ahma/rust.json` with `"name": "cargo"` will override the bundled cargo tool definition when `--rust` is also passed.

## Available Tool Configurations

Run the validation tool to ensure your configuration is correct:

```bash
# Validate a specific configuration using the example runners
cargo run --example cargo_tool
cargo run --example file-tools
cargo run --example gh_tool
cargo run --example git_tool
cargo run --example gradlew_tool
cargo run --example python_tool

# Or run schema validation tests
cargo test --test tool_config_schema_validation_test

# Or run all tests including execution tests
cargo nextest run --package ahma_mcp --test tool_config_schema_validation_test
cargo nextest run --package ahma_mcp --test tool_examples_execution_test
```

### 4. Verify Your Configuration Works

After copying and enabling a configuration in `.ahma/`, restart the ahma_mcp server to load the new tool:

```bash
# The server will automatically discover and load enabled configurations from .ahma/
ahma_mcp --tools-dir .ahma
```

## Configuration Format

All tool configurations follow the MCP Tool Definition Format (MTDF) schema. Here's a minimal example:

```json
{
    "name": "mytool",
    "description": "Description of what the tool does",
    "command": "command-to-execute",
    "enabled": true,
    "timeout_seconds": 300,
    "subcommand": [
        {
            "name": "subcommand_name",
            "description": "What this subcommand does",
            "options": [
                {
                    "name": "option-name",
                    "type": "string",
                    "description": "What this option does",
                    "required": false
                }
            ],
            "synchronous": true
        }
    ]
}
```

## Validation Tools

### Command-Line Validation

```bash
# Validate all example configs
cargo nextest run -p ahma_mcp tool_config_schema_validation

# Run a specific example to see detailed output
cargo run --example cargo_tool

# Check if configuration is parseable
jq . .ahma/cargo.json
```

### Programmatic Validation

Use the `MtdfValidator` from `ahma_mcp`:

```rust
use ahma_mcp::schema_validation::MtdfValidator;
use std::path::Path;

let validator = MtdfValidator::new();
let config_path = Path::new(".ahma/mytool.json");
let content = std::fs::read_to_string(config_path)?;

match validator.validate_tool_config(config_path, &content) {
    Ok(config) => println!("OK Valid configuration"),
    Err(errors) => {
        eprintln!("FAIL Validation errors:");
        for error in errors {
            eprintln!("  - {}: {}", error.field_path, error.message);
        }
    }
}
```

## Security Considerations

- **Path Security**: All file paths are automatically validated and scoped to the current working directory
- **Sandbox Mode**: Commands run in isolated environments with restricted permissions
- **Timeout Protection**: All operations have configurable timeouts to prevent hanging
- **Command Whitelisting**: Only explicitly configured commands and subcommands are available

## Troubleshooting

### Configuration Not Loading

1. Ensure the JSON file is valid: `jq . .ahma/mytool.json`
2. Check that `"enabled": true` is set
3. Verify file permissions: `ls -la .ahma/`
4. Check server logs for parsing errors

### Validation Fails

1. Run the corresponding example: `cargo run --example mytool`
2. Check for schema violations in the error output
3. Compare with working examples in `ahma_mcp/examples/configs/`
4. Verify all required fields are present: `name`, `description`, `command`, `enabled`

### Tool Not Available in AI

1. Restart the `ahma_mcp` server
2. Verify tool is enabled: `grep enabled .ahma/mytool.json`
3. Check server initialization logs
4. Ensure the underlying command is installed: `which command-name`

## Schema Documentation

Full MTDF schema documentation is available at:
- `docs/mtdf-schema.json` - JSON Schema definition
- `ahma_mcp/docs/mtdf-schema.json` - Core library schema

## Contributing

To add a new tool configuration:

1. Create the JSON file in `ahma_mcp/examples/configs/`
2. Add a corresponding example in `ahma_mcp/examples/toolname.rs`
3. Add tests in `ahma_mcp/tests/tool_config_schema_validation_test.rs`
4. Add execution tests in `ahma_mcp/tests/tool_examples_execution_test.rs`
5. Update `ahma_mcp/Cargo.toml` with example declaration
6. Run all tests: `cargo nextest run --workspace`

## License

Tool configurations in this directory follow the project's dual MIT/Apache-2.0 license.
