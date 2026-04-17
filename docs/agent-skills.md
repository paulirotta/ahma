# Agent Skills

Agent skills are optional. They are not required to use ahma for normal build, test, git, or shell workflows.

## `ahma-simplify`

The `ahma-simplify` skill teaches agents how to analyze code complexity, identify hotspot functions, make small targeted fixes, and verify the result using the `simplify` MCP tool or `ahma-simplify` CLI.

The skill file at `skills/ahma-simplify/SKILL.md` follows the `.agents/skills/` format recognized by VS Code, Cursor, and Claude Code.

## Install manually

**Linux / macOS**

```bash
mkdir -p ~/.agents/skills/ahma-simplify
cp skills/ahma-simplify/SKILL.md ~/.agents/skills/ahma-simplify/SKILL.md
```

**Windows**

```powershell
New-Item -ItemType Directory -Force "$HOME\.agents\skills\ahma-simplify"
Copy-Item skills\ahma-simplify\SKILL.md "$HOME\.agents\skills\ahma-simplify\SKILL.md"
```

To enable the skill per project:

```bash
mkdir -p .agents/skills/ahma-simplify
cp skills/ahma-simplify/SKILL.md .agents/skills/ahma-simplify/SKILL.md
```

Once installed, agents load the skill automatically when you ask about complexity, maintainability, or simplification. In Cursor you can also attach it explicitly with `@ahma-simplify`.

The installation script can offer this automatically after the main MCP setup flow. See [docs/installation.md](installation.md).