//! Tool Registry
//!
//! Provides an authoritative, deterministic inventory of Gestura's built-in
//! tools. This is used by interactive agent commands (e.g. `/tools`) and for
//! answering common "what tools do you have" questions without relying on an
//! LLM response.
//!
//! Also provides dynamic configuration summary including MCP servers, devices,
//! and current settings.

use crate::config::AppConfig;

// Static tool inventory is owned by the tools domain crate.
// We re-export it here so callers can keep using
// `gestura_core::tools::registry::{all_tools, find_tool, ...}`.
pub use gestura_core_tools::registry::{
    ToolDefinition, all_tools, code_tool_names, find_tool, is_code_tool_name,
    looks_like_capabilities_question, looks_like_tools_question, render_tool_detail,
    render_tools_overview,
};

/// Render a comprehensive capabilities overview including dynamic config.
/// This shows built-in tools, MCP servers/tools, device status, and settings.
pub fn render_capabilities(config: &AppConfig) -> String {
    let mut out = String::new();

    // Built-in tools
    out.push_str("## Built-in Tools\n\n");
    for t in all_tools() {
        out.push_str(&format!("• **{}** - {}\n", t.name, t.summary));
    }

    // MCP Servers & Tools
    out.push_str("\n## MCP Servers & Tools\n\n");
    if config.mcp_servers.is_empty() {
        out.push_str("_No MCP servers configured._\n");
    } else {
        for srv in &config.mcp_servers {
            let status = if srv.enabled { "✓" } else { "○" };
            out.push_str(&format!(
                "• {} **{}** ({}) → `{}`\n",
                status,
                srv.name,
                srv.transport,
                srv.effective_uri()
            ));
        }
    }

    // MDH Pointers (data resources)
    if !config.mdh_pointers.is_empty() {
        out.push_str("\n## MDH Data Resources\n\n");
        for (alias, uri) in &config.mdh_pointers {
            out.push_str(&format!("• **{}** → `{}`\n", alias, uri));
        }
    }

    // LLM Configuration
    out.push_str("\n## LLM Configuration\n\n");
    out.push_str(&format!("• **Primary Provider:** {}\n", config.llm.primary));
    if let Some(ref openai) = config.llm.openai {
        out.push_str(&format!("• **OpenAI Model:** {}\n", openai.model));
    }
    if let Some(ref anthropic) = config.llm.anthropic {
        out.push_str(&format!("• **Anthropic Model:** {}\n", anthropic.model));
    }
    if let Some(ref grok) = config.llm.grok {
        out.push_str(&format!("• **Grok Model:** {}\n", grok.model));
    }
    if let Some(ref ollama) = config.llm.ollama {
        out.push_str(&format!(
            "• **Ollama:** {} @ {}\n",
            ollama.model, ollama.base_url
        ));
    }

    // Voice Configuration
    out.push_str("\n## Voice Configuration\n\n");
    out.push_str(&format!("• **Provider:** {}\n", config.voice.provider));
    if let Some(ref device) = config.voice.audio_device {
        out.push_str(&format!("• **Audio Device:** {}\n", device));
    }
    if let Some(ref model_path) = config.voice.local_model_path {
        out.push_str(&format!("• **Local Model:** {}\n", model_path));
    }

    // Device/Simulator Settings
    out.push_str("\n## Device & Simulator Settings\n\n");
    out.push_str(&format!(
        "• **Developer Mode:** {}\n",
        if config.developer.developer_mode {
            "enabled"
        } else {
            "disabled"
        }
    ));
    out.push_str(&format!(
        "• **Simulators:** {}\n",
        if config.developer.enable_simulators {
            "enabled"
        } else {
            "disabled"
        }
    ));
    if config.developer.enable_simulators {
        out.push_str(&format!(
            "• **Auto-discover Simulators:** {}\n",
            config.developer.auto_discover_simulators
        ));
        out.push_str(&format!(
            "• **Simulator Pattern:** {}\n",
            config.developer.simulator.device_name_pattern
        ));
    }

    // Hotkey
    out.push_str("\n## System\n\n");
    out.push_str(&format!("• **Hotkey:** {}\n", config.hotkey_listen));
    out.push_str(&format!(
        "• **Grace Period:** {}s\n",
        config.grace_period_secs
    ));

    out.push_str("\n---\nUse `/tools <name>` for details on a specific tool.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_expected_tools() {
        assert!(find_tool("file").is_some());
        assert!(find_tool("shell").is_some());
        assert!(find_tool("git").is_some());
    }

    #[test]
    fn looks_like_tools_question_matches_common_phrases() {
        assert!(looks_like_tools_question("what tools do you have?"));
        assert!(looks_like_tools_question("list tools"));
        assert!(looks_like_tools_question("/tools"));
        assert!(!looks_like_tools_question("tell me a joke"));
    }
}
