# Agent Skills

Agent skills are optional. They are not required to use ahma for normal build, test, git, or shell workflows.

## `ahma`

The `ahma` skill teaches agents how to use `ahma-mcp` effectively: sandboxed shell execution, tool bundles, progressive disclosure, async/await patterns, code complexity analysis (`ahma-mcp simplify`), and more.

The skill file at `skills/ahma/SKILL.md` follows the `.agents/skills/` format recognized by VS Code, Cursor, and Claude Code.

## Install manually

**Linux / macOS**

```bash
mkdir -p ~/.agents/skills/ahma
cp skills/ahma/SKILL.md ~/.agents/skills/ahma/SKILL.md
```

**Windows**

```powershell
New-Item -ItemType Directory -Force "$HOME\.agents\skills\ahma"
Copy-Item skills\ahma\SKILL.md "$HOME\.agents\skills\ahma\SKILL.md"
```

To enable the skill per project:

```bash
mkdir -p .agents/skills/ahma
cp skills/ahma/SKILL.md .agents/skills/ahma/SKILL.md
```

Once installed, agents load the skill automatically when you ask about ahma, sandboxed execution, code complexity, or simplification. In Cursor you can also attach it explicitly with `@ahma`. Invoke sub-workflows with `/ahma simplify`, `/ahma help`, etc.

The installation script can offer this automatically after the main MCP setup flow. See [docs/installation.md](installation.md).

For code complexity analysis details, see [SIMPLIFY.md](../SIMPLIFY.md).