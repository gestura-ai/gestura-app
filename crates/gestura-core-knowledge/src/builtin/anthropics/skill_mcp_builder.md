# MCP Builder (Anthropic Skill)

**Derived from:** `anthropics/skills` (`skills/mcp-builder/SKILL.md`)  
**License:** Apache-2.0 (per upstream folder `LICENSE.txt`)  
**Source URL:** https://raw.githubusercontent.com/anthropics/skills/main/skills/mcp-builder/SKILL.md  
**Modifications:** condensed + reformatted for Gestura’s built-in knowledge.

## Use this when
You’re building an MCP server and want a practical, high-quality implementation checklist.

## Core guidance
### API coverage vs workflow tools
- Prefer **broad API coverage** when unsure.
- Add workflow tools only where they materially simplify repeated tasks.

### Tool naming
- Clear, action-oriented names (e.g., `github_list_issues`)
- Consistent prefixes by domain

### Context management
- Return focused data; paginate aggressively
- Provide filters rather than dumping huge payloads

### Errors
- Actionable, suggest next steps

## Implementation outline
1. Read MCP spec + chosen SDK docs
2. Plan tool surface area (start with common operations)
3. Implement:
   - auth + client
   - pagination
   - error helpers
   - structured outputs (when possible)
4. Test:
   - compilation
   - inspector/manual tool calls
5. Create evaluations (10 realistic, read-only questions)

## Recommended evaluation qualities
- independent questions
- verifiable answers
- stable over time
- multi-step tool usage
