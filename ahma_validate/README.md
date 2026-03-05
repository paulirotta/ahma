# Ahma Validate

Validation tool for Ahma tool configurations and schemas.

## Overview

`ahma_validate` is a utility for validating tool configuration files and ensuring they conform to the expected JSON schema format.

## Usage

```bash
# Validate all tools in a directory
ahma_validate --tools-dir ./tools

# Validate a specific tool configuration
ahma_validate --tool-file ./tools/my_tool.json
```

## What It Validates

- JSON syntax correctness
- Required fields presence
- Schema compliance
- Tool name uniqueness
- Command path validity

## License

MIT OR Apache-2.0

