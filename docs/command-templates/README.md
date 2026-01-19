# Command templates (reusable workflows)

These documents are **copy/paste-friendly workflows** for humans and AI coding assistants.

They serve the same purpose as Claude Code’s `.claude/commands/*`: a small, curated library of repeatable procedures that produce consistent results.

## Templates

- `quality-gates.md` — run the repo’s quality gates (fast and full)
- `tool-permissions.md` — inspect and grant permissions safely (default deny)
- `headless-exec.md` — run one-shot/headless prompts (`gestura exec`) safely and reproducibly
- `mcp-management.md` — inspect and manage MCP tools (`gestura mcp …`)

## Guidance

- Prefer running the **fast** gate frequently, and the **full** gate before pushing changes.
- If a step fails, fix forward with the smallest scoped change, then re-run the smallest gate that gives confidence.
