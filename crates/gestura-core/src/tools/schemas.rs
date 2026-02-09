//! Provider-specific tool schemas (OpenAI / Anthropic)
//!
//! Gestura's pipeline keeps a text prompt format for portability, but some LLM
//! providers require tool definitions to be passed **out-of-band** (as JSON
//! schema) to enable structured tool calls.
//!
//! This module converts Gestura's [`ToolDefinition`] inventory into provider-
//! specific schemas.

use crate::tools::ToolDefinition;
use serde_json::Value;

/// Provider-specific tool schema bundles.
#[derive(Debug, Clone, Default)]
pub struct ProviderToolSchemas {
    /// OpenAI-compatible `tools: [{type:"function", function:{...}}]`.
    pub openai: Vec<Value>,
    /// Anthropic `tools: [{name, description, input_schema}]`.
    pub anthropic: Vec<Value>,
}

/// Build provider tool schemas for a set of tool definitions.
///
/// Note: We intentionally only include schemas for tools that have a well-
/// defined structured interface in `gestura-core`'s pipeline.
pub fn build_provider_tool_schemas(tools: &[&'static ToolDefinition]) -> ProviderToolSchemas {
    let mut out = ProviderToolSchemas::default();

    for tool in tools {
        if let Some((openai, anthropic)) = schema_for_tool(tool.name, tool.summary) {
            out.openai.push(openai);
            out.anthropic.push(anthropic);
        }
    }

    out
}

fn schema_for_tool(name: &str, summary: &str) -> Option<(Value, Value)> {
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
                    "operation": {"type": "string", "enum": ["stats"]},
                    "path": {"type": "string", "description": "Directory to analyze (optional, default '.')"}
                },
                "required": ["operation"],
                "additionalProperties": true
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
        // Not yet supported in the runtime tool executor.
        "a2a" | "permissions" | "mcp" => return None,
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

    Some((openai, anthropic))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::find_tool;

    #[test]
    fn builds_shell_schema_for_both_providers() {
        let shell = find_tool("shell").unwrap();
        let schemas = build_provider_tool_schemas(&[shell]);
        assert_eq!(schemas.openai.len(), 1);
        assert_eq!(schemas.anthropic.len(), 1);

        assert_eq!(schemas.openai[0]["function"]["name"], "shell");
        assert!(
            schemas.openai[0]["function"]["parameters"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "command")
        );
    }
}
