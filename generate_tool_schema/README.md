# Generate Tool Schema

Utility for generating JSON schemas for Ahma tool configurations.

## Overview

`generate_tool_schema` creates JSON schema definitions for tool configuration files, making it easier to validate and document tool definitions.

## Usage

```bash
# Generate schema in the default directory (docs/)
cargo run -p generate_tool_schema

# Generate schema in a custom directory
cargo run -p generate_tool_schema -- ./schemas
```

## Output

The tool generates a single file: `mtdf-schema.json` in the specified output directory.

Generates JSON schema files that can be used for:
- IDE autocomplete and validation
- Documentation generation
- Configuration validation
- Tool development

## License

MIT OR Apache-2.0

