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
        if tool.name == "file" {
            if let Some((openai, anthropic, gemini)) = schema_for_tool(tool.name, tool.description)
            {
                out.openai.push(openai);
                out.anthropic.push(anthropic);
                out.gemini.push(gemini);
            }

            for (name, description, input_schema) in split_file_tool_schemas() {
                let (openai, anthropic, gemini) =
                    build_provider_schema(name.as_str(), description.as_str(), input_schema);
                out.openai.push(openai);
                out.anthropic.push(anthropic);
                out.gemini.push(gemini);
            }
            continue;
        }

        if let Some((openai, anthropic, gemini)) = schema_for_tool(tool.name, tool.description) {
            out.openai.push(openai);
            out.anthropic.push(anthropic);
            out.gemini.push(gemini);
        }
    }

    out
}

fn schema_for_tool(name: &str, summary: &str) -> Option<(Value, Value, Value)> {
    // Keep schemas precise enough that models can infer required call shapes reliably.
    let (description, input_schema) = match name {
        "shell" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Shell command to run. Commands must be non-interactive: this tool cannot answer prompts or confirmations, so include unattended flags such as -y/--yes/CI=1 when needed."},
                    "cwd": {"type": "string", "description": "Working directory (optional)"},
                    "env": {"type": "object", "description": "Environment variables", "additionalProperties": {"type": "string"}},
                    "timeout_secs": {"type": "integer", "description": "Timeout in seconds (optional). Quick commands can use short timeouts, but install/build/test/scaffold commands should usually use about 300 seconds. Interactive commands are not supported; if a command may prompt, use non-interactive flags or ask the user first."},
                    "allow_long_running": {"type": "boolean", "description": "When true, active PTY-backed shell commands may continue beyond timeout_secs while output activity is still arriving. Use this for long-running builds, tests, installs, or scaffolds that should only be interrupted if they appear stalled."},
                    "stall_timeout_secs": {"type": "integer", "description": "Optional quiet-period threshold used with allow_long_running. If the command produces no shell activity for this many seconds, it is treated as stalled and interrupted."}
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
                        "description": "Inspection-oriented file operation to perform. Use `read`, `list`, `tree`, or `search`. For writes, use the strict `write_file` tool; for targeted replacements, use the strict `edit_file` tool.",
                        "enum": ["read", "list", "tree", "search"]
                    },
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Path to the target file or directory. Required for `read` and `search`. Optional for `list` and `tree`, where it defaults to '.'."
                    },
                    "pattern": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Search pattern (regex) for `search`."
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "Whether `search` should recurse into subdirectories."
                    },
                    "max_matches": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of `search` matches to return."
                    },
                    "show_hidden": {
                        "type": "boolean",
                        "description": "Whether `list` or `tree` should include hidden files and directories."
                    },
                    "max_entries": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of entries to return for `list`."
                    },
                    "max_depth": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum traversal depth for `tree`."
                    },
                    "start": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Starting line number for partial `read` (1-based)."
                    },
                    "end": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Ending line number for partial `read` (1-based, inclusive)."
                    }
                },
                "oneOf": [
                    {
                        "type": "object",
                        "description": "Read a file. REQUIRED fields: `operation`, `path`. Optional fields: `start`, `end`.",
                        "properties": {
                            "operation": {
                                "type": "string",
                                "description": "Read a file.",
                                "enum": ["read"]
                            },
                            "path": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Exact file path to read."
                            },
                            "start": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "Starting line number for a partial read (1-based)."
                            },
                            "end": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "Ending line number for a partial read (1-based, inclusive)."
                            }
                        },
                        "required": ["operation", "path"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "description": "List directory contents. REQUIRED field: `operation`. `path` defaults to '.'. Optional fields: `show_hidden`, `max_entries`.",
                        "properties": {
                            "operation": {
                                "type": "string",
                                "description": "List directory contents.",
                                "enum": ["list"]
                            },
                            "path": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Directory path. Defaults to '.'."
                            },
                            "show_hidden": {
                                "type": "boolean",
                                "description": "Whether to include hidden files and directories."
                            },
                            "max_entries": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "Maximum number of entries to return."
                            }
                        },
                        "required": ["operation"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "description": "Show a directory tree. REQUIRED field: `operation`. `path` defaults to '.'. Optional fields: `show_hidden`, `max_depth`.",
                        "properties": {
                            "operation": {
                                "type": "string",
                                "description": "Show a directory tree.",
                                "enum": ["tree"]
                            },
                            "path": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Directory path. Defaults to '.'."
                            },
                            "show_hidden": {
                                "type": "boolean",
                                "description": "Whether to include hidden files and directories."
                            },
                            "max_depth": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "Maximum directory depth to traverse."
                            }
                        },
                        "required": ["operation"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "description": "Search for text in files. REQUIRED fields: `operation`, `path`, `pattern`. Optional fields: `recursive`, `max_matches`. Use this for discovery only; do not use `pattern` as a substitute for file edits.",
                        "properties": {
                            "operation": {
                                "type": "string",
                                "description": "Search files for text or regex matches.",
                                "enum": ["search"]
                            },
                            "path": {
                                "type": "string",
                                "minLength": 1,
                                "description": "File or directory path to search within."
                            },
                            "pattern": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Search pattern (regex)."
                            },
                            "recursive": {
                                "type": "boolean",
                                "description": "Whether to search recursively in subdirectories."
                            },
                            "max_matches": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "Maximum number of matches to return."
                            }
                        },
                        "required": ["operation", "path", "pattern"],
                        "additionalProperties": false
                    }
                ],
                "examples": [
                    {"operation": "read", "path": "README.md"},
                    {"operation": "list", "path": ".", "show_hidden": false},
                    {"operation": "search", "path": "src", "pattern": "TODO"}
                ],
                "required": ["operation"],
                "additionalProperties": false
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
                "oneOf": [
                    {
                        "properties": { "operation": { "enum": ["stats", "map", "deps", "lint", "test"] } },
                        "required": ["operation"]
                    },
                    {
                        "description": "Extract symbols or an outline from a single file. REQUIRED fields: `operation`, `path`.",
                        "properties": { "operation": { "enum": ["symbols", "outline"] } },
                        "required": ["operation", "path"]
                    },
                    {
                        "description": "Find references or a definition for a symbol. REQUIRED fields: `operation`, `symbol`.",
                        "properties": { "operation": { "enum": ["references", "definition"] } },
                        "required": ["operation", "symbol"]
                    },
                    {
                        "description": "Find files matching a glob or grep pattern. REQUIRED fields: `operation`, `pattern`.",
                        "properties": { "operation": { "enum": ["glob", "grep"] } },
                        "required": ["operation", "pattern"]
                    },
                    {
                        "description": "Read multiple files in one call. REQUIRED fields: `operation`, `paths`.",
                        "properties": { "operation": { "enum": ["batch_read"] } },
                        "required": ["operation", "paths"]
                    },
                    {
                        "description": "Apply one or more exact string replacements. REQUIRED fields: `operation`, `edits`. `edits` must be an array even for one change. Each edit requires `path`, `old_str`, and `new_str`.",
                        "properties": { "operation": { "enum": ["batch_edit"] } },
                        "required": ["operation", "edits"]
                    }
                ],
                "examples": [
                    {"operation": "batch_read", "paths": ["src/lib.rs", "app/main.py"]},
                    {"operation": "batch_edit", "edits": [{"path": "src/lib.rs", "old_str": "fn greet() {}", "new_str": "fn greet() { println!(\"hello\"); }"}]},
                    {"operation": "grep", "pattern": "TODO", "path": ".", "max_results": 20},
                    {"operation": "test", "path": ".", "filter": "integration"}
                ],
                "required": ["operation"],
                "additionalProperties": false
            }),
        ),
        "code_read_files" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Exact file paths to read. This tool is file-only; do not pass directories."
                    }
                },
                "required": ["paths"],
                "additionalProperties": false,
                "examples": [
                    {"paths": ["src/lib.rs", "app/main.py"]}
                ]
            }),
        ),
        "code_edit_files" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "edits": {
                        "type": "array",
                        "description": "Strict array of exact str-replace edits. Each edit must include only `path`, `old_str`, and `new_str`.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string", "description": "Exact file path to edit. Must not be a directory."},
                                "old_str": {"type": "string", "description": "Exact string to find."},
                                "new_str": {"type": "string", "description": "Replacement string."}
                            },
                            "required": ["path", "old_str", "new_str"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["edits"],
                "additionalProperties": false,
                "examples": [
                    {"edits": [{"path": "src/lib.rs", "old_str": "fn greet() {}", "new_str": "fn greet() { println!(\"hello\"); }"}]}
                ]
            }),
        ),
        "code_outline" | "code_symbols" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Exact file path. This tool is file-only; do not pass a directory."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        "code_references" | "code_definition" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "symbol": {"type": "string", "description": "Symbol name to search for."},
                    "path": {"type": "string", "description": "Existing file or directory path to search within."}
                },
                "required": ["symbol", "path"],
                "additionalProperties": false
            }),
        ),
        "code_glob" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Existing directory path to search within."},
                    "pattern": {"type": "string", "description": "Glob pattern such as **/*.rs."},
                    "max_results": {"type": "integer", "default": 100}
                },
                "required": ["path", "pattern"],
                "additionalProperties": false
            }),
        ),
        "code_grep" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Existing directory path to search within."},
                    "pattern": {"type": "string", "description": "Regex pattern to search for."},
                    "file_glob": {"type": "string"},
                    "context_lines": {"type": "integer", "default": 2},
                    "case_sensitive": {"type": "boolean", "default": false},
                    "max_results": {"type": "integer", "default": 100}
                },
                "required": ["path", "pattern"],
                "additionalProperties": false
            }),
        ),
        "code_map" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Existing directory path to map."},
                    "max_depth": {"type": "integer", "default": 4}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        "code_stats" | "code_deps" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Existing path to inspect."}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        "code_lint" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Existing project directory path."},
                    "fix": {"type": "boolean", "default": false}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        "code_test" => (
            summary,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Existing project directory path."},
                    "filter": {"type": "string"}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        "task" | "tasks" => (
            "Create, update, list, and organize tasks for the current session. For `create`, provide `name` and let the runtime assign the `task_id`; do not send `task_id` in create calls. For `update_status`, ALWAYS provide both `task_id` and `status` in the same call. Correct example: {\"operation\":\"update_status\",\"task_id\":\"abc123\",\"status\":\"completed\"}. Invalid example: {\"operation\":\"update_status\",\"task_id\":\"abc123\"}. Do not call `update_status` just to confirm or preserve the current state; if no status changed, continue the real work instead of repeating bookkeeping.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["create", "update_status", "update", "delete", "list", "get_hierarchy"],
                        "description": "Task operation to perform. `update_status` requires BOTH `task_id` and `status`; do not call it with only `task_id`, and do not use it just to confirm the current state."
                    },
                    "task_id": {
                        "type": "string",
                        "description": "Task ID. REQUIRED for update_status, update, delete operations. For `update_status`, send this together with `status` in the same call. Omit this field entirely for create/list/get_hierarchy; do not send the string 'None' or 'null'."
                    },
                    "name": {
                        "type": "string",
                        "description": "Task name. REQUIRED for create operation. Provide plain text only; do not wrap it in XML or parameter tags. For create, send `name` and optional `description`; do not send a `task_id`."
                    },
                    "description": {
                        "type": "string",
                        "description": "Task description. Optional for create and update operations."
                    },
                    "status": {
                        "type": "string",
                        "enum": ["notstarted", "inprogress", "completed", "cancelled"],
                        "description": "Task status. REQUIRED for update_status operation. Use plain JSON text only: 'notstarted', 'inprogress', 'completed', or 'cancelled'. Do not wrap status in XML/parameter tags. Do not omit this field to ask the runtime to infer or preserve the current state; if no status changed, skip the task update and continue the real work."
                    },
                    "parent_id": {
                        "type": "string",
                        "description": "Parent task ID for creating subtasks. Optional for create operation. Omit this field when there is no parent; do not send the string 'None' or 'null'."
                    }
                },
                "oneOf": [
                    {
                        "properties": {
                            "operation": { "enum": ["create"] }
                        },
                        "required": ["operation", "name"]
                    },
                    {
                        "description": "Update a task's status. REQUIRED fields: `task_id` and `status`. Correct example: {\"operation\":\"update_status\",\"task_id\":\"abc123\",\"status\":\"inprogress\"}. Invalid: {\"operation\":\"update_status\",\"task_id\":\"abc123\"}.",
                        "properties": {
                            "operation": { "enum": ["update_status"] }
                        },
                        "required": ["operation", "task_id", "status"]
                    },
                    {
                        "properties": {
                            "operation": { "enum": ["update"] }
                        },
                        "required": ["operation", "task_id"]
                    },
                    {
                        "properties": {
                            "operation": { "enum": ["delete"] }
                        },
                        "required": ["operation", "task_id"]
                    },
                    {
                        "properties": {
                            "operation": { "enum": ["list"] }
                        },
                        "required": ["operation"]
                    },
                    {
                        "properties": {
                            "operation": { "enum": ["get_hierarchy"] }
                        },
                        "required": ["operation"]
                    }
                ],
                "examples": [
                    {"operation": "create", "name": "Implement feature", "description": "Add new API endpoint"},
                    {"operation": "update_status", "task_id": "abc123", "status": "inprogress"},
                    {"operation": "update_status", "task_id": "abc123", "status": "completed"},
                    {"operation": "list"}
                ],
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

    Some(build_provider_schema(name, description, input_schema))
}

fn build_provider_schema(
    name: &str,
    description: &str,
    input_schema: Value,
) -> (Value, Value, Value) {
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

    (openai, anthropic, gemini)
}

fn split_file_tool_schemas() -> Vec<(String, String, Value)> {
    vec![
        (
            "read_file".to_string(),
            "Read one exact file with an optional line range. Strict file-only contract; do not pass directory, search, or edit fields.".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Exact file path to read. Must not be a directory."
                    },
                    "start": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Starting line number for a partial read (1-based)."
                    },
                    "end": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Ending line number for a partial read (1-based, inclusive)."
                    }
                },
                "required": ["path"],
                "additionalProperties": false,
                "examples": [
                    {"path": "src/main.rs"},
                    {"path": "src/main.rs", "start": 1, "end": 80}
                ]
            }),
        ),
        (
            "write_file".to_string(),
            "Write one exact file using the full replacement content. Strict full-document contract; include `path` and canonical `content` only.".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Destination file path."
                    },
                    "content": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Full replacement file content. Use this canonical field only; do not substitute `pattern`, `text`, or `contents`."
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false,
                "examples": [
                    {"path": "docs/summary.txt", "content": "Build completed successfully.\n"}
                ]
            }),
        ),
        (
            "edit_file".to_string(),
            "Apply one exact string replacement inside one existing file. Strict edit contract; include only `path`, `old`, and `new`.".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Existing file path to edit."
                    },
                    "old": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Exact existing text to replace."
                    },
                    "new": {
                        "type": "string",
                        "description": "Replacement text."
                    }
                },
                "required": ["path", "old", "new"],
                "additionalProperties": false,
                "examples": [
                    {"path": "src/lib.rs", "old": "fn greet() { println!(\"hi\"); }", "new": "fn greet() { println!(\"hello\"); }"}
                ]
            }),
        ),
    ]
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

        let command_description =
            schemas.openai[0]["function"]["parameters"]["properties"]["command"]["description"]
                .as_str()
                .expect("shell command description should exist");
        assert!(command_description.contains("non-interactive"));
    }

    #[test]
    fn task_schema_requires_name_for_create_operation() {
        let task = find_tool("task").unwrap();
        let schemas = build_provider_tool_schemas(&[task]);

        let branches = schemas.openai[0]["function"]["parameters"]["oneOf"]
            .as_array()
            .expect("task schema should define oneOf branches");

        let create_branch = branches
            .iter()
            .find(|branch| branch["properties"]["operation"]["enum"][0] == "create")
            .expect("missing create branch");

        let required = create_branch["required"]
            .as_array()
            .expect("create branch should have required fields");

        assert!(required.iter().any(|value| value == "name"));
    }

    #[test]
    fn task_schema_warns_against_missing_status_noop_updates() {
        let task = find_tool("task").unwrap();
        let schemas = build_provider_tool_schemas(&[task]);

        let function_description = schemas.openai[0]["function"]["description"]
            .as_str()
            .expect("task function description should exist");
        assert!(function_description.contains("ALWAYS provide both `task_id` and `status`"));
        assert!(function_description.contains("Invalid example"));

        let operation_description =
            schemas.openai[0]["function"]["parameters"]["properties"]["operation"]["description"]
                .as_str()
                .expect("task operation description should exist");
        assert!(operation_description.contains("requires BOTH `task_id` and `status`"));
        assert!(operation_description.contains("do not use it just to confirm the current state"));

        let status_description =
            schemas.openai[0]["function"]["parameters"]["properties"]["status"]["description"]
                .as_str()
                .expect("task status description should exist");
        assert!(status_description.contains("Do not omit this field"));
        assert!(status_description.contains("skip the task update and continue the real work"));

        let update_status_branch = schemas.openai[0]["function"]["parameters"]["oneOf"]
            .as_array()
            .expect("task schema should define oneOf branches")
            .iter()
            .find(|branch| branch["properties"]["operation"]["enum"][0] == "update_status")
            .expect("missing update_status branch");
        let branch_description = update_status_branch["description"]
            .as_str()
            .expect("update_status branch description should exist");
        assert!(branch_description.contains("Correct example"));
        assert!(branch_description.contains("Invalid"));

        let examples = schemas.openai[0]["function"]["parameters"]["examples"]
            .as_array()
            .expect("task schema should include examples");
        assert!(examples.iter().any(|example| {
            example["operation"] == "update_status"
                && example["task_id"] == "abc123"
                && example["status"] == "inprogress"
        }));
    }

    #[test]
    fn file_schema_requires_content_for_write_operation() {
        let file = find_tool("file").unwrap();
        let schemas = build_provider_tool_schemas(&[file]);

        let write_file_schema = schemas
            .openai
            .iter()
            .find(|schema| schema["function"]["name"] == "write_file")
            .expect("missing write_file schema");
        let parameters = &write_file_schema["function"]["parameters"];
        let required = parameters["required"]
            .as_array()
            .expect("write_file schema should have required fields");

        assert!(required.iter().any(|value| value == "path"));
        assert!(required.iter().any(|value| value == "content"));

        let examples = parameters["examples"]
            .as_array()
            .expect("write_file schema should include examples");
        assert!(examples.iter().any(|example| {
            example["path"] == "docs/summary.txt" && example["content"].is_string()
        }));

        let branch_properties = parameters["properties"]
            .as_object()
            .expect("write_file schema should define properties");
        assert!(parameters["additionalProperties"] == serde_json::json!(false));
        assert!(!branch_properties.contains_key("pattern"));
        assert!(!branch_properties.contains_key("start"));

        let description = write_file_schema["function"]["description"]
            .as_str()
            .expect("write_file description should exist");
        assert!(description.contains("canonical `content` only"));
        assert!(!description.contains("contents"));
        assert!(!description.contains("text"));
    }

    #[test]
    fn file_schema_uses_strict_operation_specific_branches() {
        let file = find_tool("file").unwrap();
        let schemas = build_provider_tool_schemas(&[file]);

        let parameters = &schemas.openai[0]["function"]["parameters"];
        let branches = parameters["oneOf"]
            .as_array()
            .expect("file schema should define oneOf branches");

        assert_eq!(branches.len(), 4);
        let root_properties = parameters["properties"]
            .as_object()
            .expect("file schema should expose top-level properties");
        assert!(parameters["additionalProperties"] == serde_json::json!(false));
        assert!(root_properties.contains_key("operation"));
        assert!(root_properties.contains_key("path"));
        assert!(root_properties.contains_key("pattern"));
        assert!(root_properties.contains_key("start"));
        assert!(!root_properties.contains_key("content"));
        assert!(!root_properties.contains_key("old"));
        assert!(!root_properties.contains_key("new"));

        let search_branch = branches
            .iter()
            .find(|branch| branch["properties"]["operation"]["enum"][0] == "search")
            .expect("missing search branch");
        let search_properties = search_branch["properties"]
            .as_object()
            .expect("search branch should define properties");

        assert!(search_branch["additionalProperties"] == serde_json::json!(false));
        assert!(search_properties.contains_key("pattern"));
        assert!(!search_properties.contains_key("content"));

        let read_branch = branches
            .iter()
            .find(|branch| branch["properties"]["operation"]["enum"][0] == "read")
            .expect("missing read branch");
        let read_properties = read_branch["properties"]
            .as_object()
            .expect("read branch should define properties");

        assert!(read_branch["additionalProperties"] == serde_json::json!(false));
        assert!(read_properties.contains_key("start"));
        assert!(read_properties.contains_key("end"));
        assert!(!read_properties.contains_key("content"));
    }

    #[test]
    fn file_provider_schemas_split_mutations_from_inspection_tool() {
        let file = find_tool("file").unwrap();
        let schemas = build_provider_tool_schemas(&[file]);

        let tool_names = schemas
            .openai
            .iter()
            .filter_map(|schema| schema["function"]["name"].as_str())
            .collect::<Vec<_>>();

        assert!(tool_names.contains(&"file"));
        assert!(tool_names.contains(&"read_file"));
        assert!(tool_names.contains(&"write_file"));
        assert!(tool_names.contains(&"edit_file"));

        let edit_file_schema = schemas
            .openai
            .iter()
            .find(|schema| schema["function"]["name"] == "edit_file")
            .expect("missing edit_file schema");

        let required = edit_file_schema["function"]["parameters"]["required"]
            .as_array()
            .expect("edit_file required fields");
        assert!(required.iter().any(|value| value == "path"));
        assert!(required.iter().any(|value| value == "old"));
        assert!(required.iter().any(|value| value == "new"));
    }

    #[test]
    fn code_schema_requires_edits_for_batch_edit_operation() {
        let code = find_tool("code_edit_files").unwrap();
        let schemas = build_provider_tool_schemas(&[code]);

        let parameters = &schemas.openai[0]["function"]["parameters"];
        let required = parameters["required"]
            .as_array()
            .expect("code_edit_files schema should have required fields");

        assert!(required.iter().any(|value| value == "edits"));

        let description = parameters["properties"]["edits"]["description"]
            .as_str()
            .expect("edits description should exist");
        assert!(description.contains("Strict array of exact str-replace edits"));

        let examples = parameters["examples"]
            .as_array()
            .expect("code_edit_files schema should include examples");
        assert!(examples.iter().any(|example| {
            example["edits"].is_array() && example["edits"][0]["old_str"].is_string()
        }));
    }
}
