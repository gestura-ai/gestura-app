# MCP management (`gestura mcp …`)

This template is a safe, repeatable procedure for inspecting and managing MCP (Model Context Protocol) tool configuration.

## What MCP state is persisted

Gestura persists MCP tool configuration in the user config file:

- `~/.gestura/config.json`
- key: `mcp_tools`

See `docs/CONFIGURATION.md` for the config location and structure.

## Inspect current MCP configuration

1) List configured tools:
   - `gestura mcp list`

2) Inspect protocol status (server version, transports, feature flags):
   - `gestura mcp status`

3) Inspect server capabilities (what features are implemented):
   - `gestura mcp capabilities`

4) List prompts exposed via MCP (if any):
   - `gestura mcp prompts`

## Add / remove tools

- Add a tool:
  - `gestura mcp add` (requires a tool name and an endpoint/command)

- Remove a tool:
  - `gestura mcp remove` (requires the tool name)

After add/remove, the CLI writes updated configuration back to `~/.gestura/config.json`.

## Safety / trust boundaries

- Treat MCP endpoints as **untrusted by default**.
- Prefer explicit permission gating for any tool that can write files, run shell commands, or access network.
- If a tool is no longer needed, remove it (there is no separate enabled/disabled flag in the current config structure).

## Troubleshooting

- If `gestura mcp list` shows no tools, verify `~/.gestura/config.json` exists and is valid JSON.
- If the CLI fails to save config, check file permissions on `~/.gestura/`.

