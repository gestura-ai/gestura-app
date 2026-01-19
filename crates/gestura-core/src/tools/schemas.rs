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
                        "description": "File operation",
                        "enum": ["read", "write", "edit", "list", "tree", "search"]
                    },
                    "path": {"type": "string", "description": "Path to file or directory"},
                    "content": {"type": "string", "description": "Content to write (for write)"},
                    "old": {"type": "string", "description": "Old string to replace (for edit)"},
                    "new": {"type": "string", "description": "New string (for edit)"},
                    "pattern": {"type": "string", "description": "Search pattern (for search)"},
                    "recursive": {"type": "boolean", "description": "Recursive search (for search)"},
                    "max_matches": {"type": "integer", "description": "Limit matches (for search)"},
                    "show_hidden": {"type": "boolean", "description": "Include dotfiles (for list/tree)"},
                    "max_entries": {"type": "integer", "description": "Limit entries (for list)"},
                    "max_depth": {"type": "integer", "description": "Max depth (for tree)"},
                    "start": {"type": "integer", "description": "Start line (for read)"},
                    "end": {"type": "integer", "description": "End line (for read)"}
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
                    "operation": {"type": "string", "enum": ["fetch", "search"]},
                    "url": {"type": "string", "description": "URL to fetch (for fetch)"},
                    "query": {"type": "string", "description": "Search query (for search)"},
                    "num_results": {"type": "integer", "description": "Number of results (for search)"},
                    "max_results": {"type": "integer", "description": "Alias for num_results"}
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
