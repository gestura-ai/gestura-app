//! Provider-specific tool schemas (OpenAI / Anthropic / Gemini)
//!
//! Gestura's pipeline keeps a text prompt format for portability, but some LLM
//! providers require tool definitions to be passed **out-of-band** (as JSON
//! schema) to enable structured tool calls.
//!
//! This module converts Gestura's built-in tool inventory into provider-specific
//! schemas.
//!
//! Built-in tool schemas are owned by the `gestura-core-tools` domain crate; we
//! re-export them here to preserve stable public paths.

pub use gestura_core_tools::schemas::{ProviderToolSchemas, build_provider_tool_schemas};

/// Build provider tool schemas from dynamically-discovered MCP tools.
///
/// Each MCP `Tool` already carries a JSON Schema `input_schema`, so we wrap it
/// in the provider-specific envelope. The tool name is namespaced as
/// `mcp__<server>__<tool>` so the pipeline can route calls back to the correct
/// MCP server.
pub fn build_mcp_tool_schemas(
    server_tools: &[(String, Vec<crate::mcp::types::Tool>)],
) -> ProviderToolSchemas {
    let mut out = ProviderToolSchemas::default();

    for (server_name, tools) in server_tools {
        for tool in tools {
            let namespaced = format!("mcp__{}__{}", server_name, tool.name);
            let description = tool
                .description
                .clone()
                .unwrap_or_else(|| format!("MCP tool {}/{}", server_name, tool.name));

            let openai = serde_json::json!({
                "type": "function",
                "function": {
                    "name": namespaced,
                    "description": description,
                    "parameters": tool.input_schema
                }
            });
            let anthropic = serde_json::json!({
                "name": namespaced,
                "description": description,
                "input_schema": tool.input_schema
            });
            let gemini = serde_json::json!({
                "name": namespaced,
                "description": description,
                "parameters": tool.input_schema
            });

            out.openai.push(openai);
            out.anthropic.push(anthropic);
            out.gemini.push(gemini);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::find_tool;

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
