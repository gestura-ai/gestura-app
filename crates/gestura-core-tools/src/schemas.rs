//! Provider-specific tool schemas (OpenAI / Anthropic / Gemini)
//!
//! Gestura's pipeline keeps a text prompt format for portability, but some LLM
//! providers require tool definitions to be passed **out-of-band** (as JSON
//! schema) to enable structured tool calls.
//!
//! This module converts Gestura's [`ToolDefinition`] inventory into provider-
//! specific schemas.

use crate::registry::ToolDefinition;
use serde_json::Value;

/// Provider-specific tool schema bundles.
#[derive(Debug, Clone, Default)]
pub struct ProviderToolSchemas {
    /// OpenAI-compatible `tools: [{type:"function", function:{...}}]`.
    pub openai: Vec<Value>,
    /// Anthropic `tools: [{name, description, input_schema}]`.
    pub anthropic: Vec<Value>,
    /// Gemini `functionDeclarations: [{name, description, parameters}]`.
    pub gemini: Vec<Value>,
}

impl ProviderToolSchemas {
    /// Merge another set of schemas into this one.
    pub fn merge(&mut self, other: ProviderToolSchemas) {
        self.openai.extend(other.openai);
        self.anthropic.extend(other.anthropic);
        self.gemini.extend(other.gemini);
    }
}

/// Build provider tool schemas for a set of tool definitions.
///
/// Note: We intentionally only include schemas for tools that have a well-
/// defined structured interface in `gestura-core`'s pipeline.
pub fn build_provider_tool_schemas(tools: &[&'static ToolDefinition]) -> ProviderToolSchemas {
    let mut out = ProviderToolSchemas::default();

    for tool in tools {
        if let Some((openai, anthropic, gemini)) = schema_for_tool(tool.name, tool.description) {
            out.openai.push(openai);
            out.anthropic.push(anthropic);
            out.gemini.push(gemini);
        }
    }

    out
}

fn schema_for_tool(name: &str, summary: &str) -> Option<(Value, Value, Value)> {
    // Keep schemas small but precise; avoid huge `oneOf` trees.
    let (description, input_schema) = match name {
        "shell" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Shell command to run"},
                    "cwd": {"type": "string", "description": "Working directory (optional)"},
                    "env": {"type": "object", "description": "Environment variables", "additionalProperties": {"type": "string"}},
                    "timeout_secs": {"type": "integer", "description": "Timeout in seconds (optional, default 60)"}
                },
                "required": ["command"],
                "additionalProperties": true
            }),
        ),
        "file" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "description": "File operation to perform. 'write' requires 'path' and 'content'. 'edit' requires 'path', 'old', and 'new'. 'search' requires 'path' and 'pattern'. 'read', 'list', and 'tree' require 'path' (defaults to '.' if omitted).",
                        "enum": ["read", "write", "edit", "list", "tree", "search"]
                    },
                    "path": {
                        "type": "string",
                        "description": "Path to file or directory. REQUIRED for most operations (defaults to '.' for list/tree/search if omitted)."
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to file. REQUIRED when operation='write'."
                    },
                    "old": {
                        "type": "string",
                        "description": "Old string to find and replace. REQUIRED when operation='edit'."
                    },
                    "new": {
                        "type": "string",
                        "description": "New string to replace with. REQUIRED when operation='edit'."
                    },
                    "pattern": {
                        "type": "string",
                        "description": "Search pattern (regex). REQUIRED when operation='search'."
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "Whether to search recursively in subdirectories (optional, for search operation, default true)"
                    },
                    "max_matches": {
                        "type": "integer",
                        "description": "Maximum number of matches to return (optional, for search operation)"
                    },
                    "show_hidden": {
                        "type": "boolean",
                        "description": "Whether to include hidden files/directories (optional, for list/tree operations, default false)"
                    },
                    "max_entries": {
                        "type": "integer",
                        "description": "Maximum number of entries to return (optional, for list operation)"
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Maximum directory depth to traverse (optional, for tree operation)"
                    },
                    "start": {
                        "type": "integer",
                        "description": "Starting line number for partial file read (optional, for read operation, 1-based)"
                    },
                    "end": {
                        "type": "integer",
                        "description": "Ending line number for partial file read (optional, for read operation, 1-based, inclusive)"
                    }
                },
                "required": ["operation"],
                "additionalProperties": true
            }),
        ),
        "git" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "description": "Git operation",
                        "enum": ["status", "diff", "diff-staged", "log", "branches"]
                    },
                    "path": {"type": "string", "description": "Repository path (optional, default '.')"}
                },
                "required": ["operation"],
                "additionalProperties": true
            }),
        ),
        "web" | "web_search" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["fetch", "search"],
                        "description": "Operation to perform. 'fetch' requires 'url' parameter. 'search' requires 'query' parameter."
                    },
                    "url": {
                        "type": "string",
                        "description": "URL to fetch. REQUIRED when operation='fetch'."
                    },
                    "query": {
                        "type": "string",
                        "description": "Search query. REQUIRED when operation='search'."
                    },
                    "num_results": {
                        "type": "integer",
                        "description": "Number of search results to return (optional, for search operation, default varies by provider)"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Alias for num_results (optional, for search operation)"
                    }
                },
                "required": ["operation"],
                "additionalProperties": true
            }),
        ),
        "code" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": [
                            "stats", "map", "symbols", "references", "definition",
                            "deps", "lint", "test", "glob", "grep",
                            "batch_read", "batch_edit", "outline"
                        ],
                        "description": "Code operation to perform:\n\
                            • stats        — line/language counts for a directory\n\
                            • map          — repository structure map (file types, key files)\n\
                            • symbols      — extract top-level symbols from a file\n\
                            • references   — find all references to a symbol (requires: symbol)\n\
                            • definition   — find the first definition of a symbol (requires: symbol)\n\
                            • deps         — list Cargo.toml dependencies\n\
                            • lint         — run cargo clippy (optional: fix=true)\n\
                            • test         — run cargo test (optional: filter)\n\
                            • glob         — find files matching a glob pattern (requires: pattern)\n\
                            • grep         — regex search in file contents (requires: pattern)\n\
                            • batch_read   — read multiple files at once (requires: paths)\n\
                            • batch_edit   — apply multiple str-replace edits (requires: edits)\n\
                            • outline      — structured symbol outline of a file"
                    },
                    "path": {
                        "type": "string",
                        "description": "Root directory or file path (default '.'). Used by stats, map, symbols, references, definition, deps, lint, test, glob, grep, outline."
                    },
                    "symbol": {
                        "type": "string",
                        "description": "Symbol name to search for. REQUIRED for: references, definition."
                    },
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern (for glob) or regex pattern (for grep). REQUIRED for: glob, grep."
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Maximum directory depth for map (default 4).",
                        "default": 4
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of results for glob and grep (default 100).",
                        "default": 100
                    },
                    "file_glob": {
                        "type": "string",
                        "description": "Optional glob to filter which files are searched by grep (e.g. '*.rs')."
                    },
                    "context_lines": {
                        "type": "integer",
                        "description": "Number of context lines before and after each grep match (default 2).",
                        "default": 2
                    },
                    "case_sensitive": {
                        "type": "boolean",
                        "description": "Whether grep is case-sensitive (default false).",
                        "default": false
                    },
                    "paths": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "List of file paths to read. REQUIRED for: batch_read."
                    },
                    "edits": {
                        "type": "array",
                        "description": "List of str-replace edit operations. REQUIRED for: batch_edit.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path":    {"type": "string", "description": "File to edit."},
                                "old_str": {"type": "string", "description": "Exact string to find."},
                                "new_str": {"type": "string", "description": "Replacement string."}
                            },
                            "required": ["path", "old_str", "new_str"]
                        }
                    },
                    "fix": {
                        "type": "boolean",
                        "description": "Pass --fix to cargo clippy (lint operation only, default false).",
                        "default": false
                    },
                    "filter": {
                        "type": "string",
                        "description": "Test name filter for cargo test (test operation only)."
                    }
                },
                "required": ["operation"],
                "additionalProperties": false
            }),
        ),
        "task" | "tasks" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["create", "update_status", "update", "delete", "list", "get_hierarchy"],
                        "description": "Task operation to perform"
                    },
                    "task_id": {
                        "type": "string",
                        "description": "Task ID. REQUIRED for update_status, update, delete operations."
                    },
                    "name": {
                        "type": "string",
                        "description": "Task name. REQUIRED for create operation."
                    },
                    "description": {
                        "type": "string",
                        "description": "Task description. Optional for create and update operations."
                    },
                    "status": {
                        "type": "string",
                        "enum": ["notstarted", "inprogress", "completed", "cancelled"],
                        "description": "Task status. REQUIRED for update_status operation. Use 'notstarted', 'inprogress', 'completed', or 'cancelled'."
                    },
                    "parent_id": {
                        "type": "string",
                        "description": "Parent task ID for creating subtasks. Optional for create operation."
                    }
                },
                "required": ["operation"],
                "additionalProperties": true
            }),
        ),
        "screenshot" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["screenshot", "capture"],
                        "description": "Optional operation alias. If omitted, defaults to screenshot"
                    },
                    "output_format": {
                        "type": "string",
                        "enum": ["png", "jpg", "jpeg"],
                        "description": "Optional output image format. If provided and output_path has an extension, they must match. (jpeg is accepted as an alias for jpg)"
                    },
                    "output_path": {
                        "type": "string",
                        "description": "Optional path where the screenshot will be saved. If omitted, Gestura will generate a default artifact path"
                    },
                    "return": {
                        "type": "object",
                        "description": "Controls how the result is returned. PREFER mode='path' (default) — the GUI displays the full image from the file path. Use inline_base64 only when you need the image data in the response text; it produces a small JPEG thumbnail (≤128px) and may still fail for very large captures.",
                        "properties": {
                            "mode": {
                                "type": "string",
                                "enum": ["path", "inline_base64"],
                                "description": "Return mode. 'path' (RECOMMENDED) = metadata + file path, displayed natively in the GUI. 'inline_base64' = metadata + a small JPEG thumbnail (strict size limits; may be iteratively downsized)."
                            },
                            "inline": {
                                "type": "object",
                                "description": "Bounds for inline_base64 mode. Values above hard safety caps may be clamped.",
                                "properties": {
                                    "max_width": {"type": "integer", "description": "Max thumbnail width in pixels (optional)"},
                                    "max_height": {"type": "integer", "description": "Max thumbnail height in pixels (optional)"},
                                    "max_base64_chars": {"type": "integer", "description": "Max characters allowed in inline base64 payload"},
                                    "max_result_chars": {"type": "integer", "description": "Max characters allowed in the full tool JSON result (must stay <= pipeline truncation)"}
                                },
                                "additionalProperties": false
                            }
                        },
                        "additionalProperties": false
                    },
                    "region": {
                        "type": "object",
                        "description": "Optional region to capture (x, y, width, height)",
                        "properties": {
                            "x": {"type": "integer", "description": "X coordinate"},
                            "y": {"type": "integer", "description": "Y coordinate"},
                            "width": {"type": "integer", "description": "Width in pixels"},
                            "height": {"type": "integer", "description": "Height in pixels"}
                        },
                        "required": ["x", "y", "width", "height"]
                    },
                    "display": {
                        "type": "integer",
                        "description": "Optional display number (0 = primary)"
                    }
                },
                "additionalProperties": false
            }),
        ),
        "screen_record" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["start", "stop"],
                        "description": "Operation to perform: 'start' to begin recording, 'stop' to end recording"
                    },
                    "output_format": {
                        "type": "string",
                        "enum": ["mp4", "mov"],
                        "description": "Output video container format (optional, for 'start'). If provided and output_path has an extension, they must match."
                    },
                    "output_path": {
                        "type": "string",
                        "description": "Path where the recording will be saved (optional, for 'start'). If omitted, Gestura will generate a default artifact path"
                    },
                    "recording_id": {
                        "type": "string",
                        "description": "Recording ID to stop. REQUIRED when operation='stop'"
                    },
                    "region": {
                        "type": "object",
                        "description": "Optional region to record (for 'start')",
                        "properties": {
                            "x": {"type": "integer", "description": "X coordinate"},
                            "y": {"type": "integer", "description": "Y coordinate"},
                            "width": {"type": "integer", "description": "Width in pixels"},
                            "height": {"type": "integer", "description": "Height in pixels"}
                        },
                        "required": ["x", "y", "width", "height"]
                    },
                    "display": {
                        "type": "integer",
                        "description": "Optional display number (0 = primary, for 'start')"
                    }
                },
                "required": ["operation"],
                "additionalProperties": false
            }),
        ),
        "gui_control" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["toggle_view_mode", "open_explorer", "close_explorer", "open_chat", "close_chat", "navigate_config"],
                        "description": "The GUI action to perform"
                    },
                    "target": {
                        "type": "string",
                        "description": "Optional target argument for the action (if applicable)"
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        ),
        "mcp" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["search", "evaluate", "install", "enable", "disable", "list", "remove", "info"],
                        "description": "The MCP manager operation to perform. Use 'search' to find servers in the registry, 'evaluate'/'info' to inspect a server's details and install requirements, 'install' to add a server to .mcp.json, 'enable'/'disable' to toggle a configured server, 'list' to see all configured servers, 'remove' to delete an entry."
                    },
                    "query": {
                        "type": "string",
                        "description": "Search keyword for operation=search (searches server names in the registry)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results to return for operation=search (default 20, max 50)",
                        "default": 20
                    },
                    "server_id": {
                        "type": "string",
                        "description": "Registry server identifier (e.g. 'io.github.modelcontextprotocol/server-filesystem') for evaluate, install, and info operations"
                    },
                    "name": {
                        "type": "string",
                        "description": "Local alias for the server in .mcp.json (for install, enable, disable, remove). Defaults to last path segment of server_id."
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["project", "user"],
                        "description": "Config scope: 'project' writes .mcp.json in the current directory, 'user' writes to ~/.mcp.json. Default: project.",
                        "default": "project"
                    },
                    "transport": {
                        "type": "string",
                        "enum": ["stdio", "http"],
                        "description": "Override the auto-detected transport type for install"
                    },
                    "command": {
                        "type": "string",
                        "description": "Override the launch command for stdio install (e.g. 'npx', 'uvx', 'docker')"
                    },
                    "args": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Override the command args array for stdio install"
                    },
                    "url": {
                        "type": "string",
                        "description": "Override the remote URL for http install"
                    },
                    "env": {
                        "type": "object",
                        "additionalProperties": {"type": "string"},
                        "description": "Environment variables to embed in the .mcp.json entry (e.g. API keys). Use for install."
                    }
                },
                "required": ["operation"],
                "additionalProperties": false
            }),
        ),
        // Not yet supported in the runtime tool executor.
        "a2a" | "permissions" => return None,
        _ => return None,
    };

    let openai = serde_json::json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": input_schema
        }
    });

    let anthropic = serde_json::json!({
        "name": name,
        "description": description,
        "input_schema": input_schema
    });

    let gemini = serde_json::json!({
        "name": name,
        "description": description,
        "parameters": input_schema
    });

    Some((openai, anthropic, gemini))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::find_tool;

    #[test]
    fn builds_shell_schema_for_all_providers() {
        let shell = find_tool("shell").unwrap();
        let schemas = build_provider_tool_schemas(&[shell]);
        assert_eq!(schemas.openai.len(), 1);
        assert_eq!(schemas.anthropic.len(), 1);
        assert_eq!(schemas.gemini.len(), 1);

        // OpenAI format: {type:"function", function:{name, description, parameters}}
        assert_eq!(schemas.openai[0]["function"]["name"], "shell");
        assert!(
            schemas.openai[0]["function"]["parameters"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "command")
        );

        // Gemini format: {name, description, parameters}
        assert_eq!(schemas.gemini[0]["name"], "shell");
        assert!(
            schemas.gemini[0]["parameters"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "command")
        );
    }
}
