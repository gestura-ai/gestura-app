//! Tool Registry
//!
//! Provides an authoritative, deterministic inventory of Gestura's built-in
//! tools. This is used by interactive chat commands (e.g. `/tools`) and for
//! answering common "what tools do you have" questions without relying on an
//! LLM response.
//!
//! Also provides dynamic configuration summary including MCP servers, devices,
//! and current settings.

use crate::config::AppConfig;

/// A single tool definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDefinition {
    /// Tool name as referenced by users.
    pub name: &'static str,
    /// Short summary of what the tool does.
    pub summary: &'static str,
    /// Human-readable list of inputs / parameters.
    pub inputs: &'static [&'static str],
    /// Human-readable list of side effects / security implications.
    pub side_effects: &'static [&'static str],
    /// Example invocations.
    pub examples: &'static [&'static str],
}

/// Return the set of built-in tools.
pub fn all_tools() -> &'static [ToolDefinition] {
    static TOOLS: [ToolDefinition; 6] = [
        ToolDefinition {
            name: "file",
            summary: "Read/write/list files and directories (workspace & sandbox-aware)",
            inputs: &["path", "operation (read/write/list)", "content (for write)", "options"],
            side_effects: &["Reads local files", "May write/modify local files"],
            examples: &["gestura tools file read ./README.md", "gestura tools file write ./notes.txt --content \"hello\""],
        },
        ToolDefinition {
            name: "shell",
            summary: "Run shell commands in a controlled environment",
            inputs: &["command", "cwd", "env"],
            side_effects: &["Executes local processes", "May modify local state depending on command"],
            examples: &["gestura tools shell run -- command=\"ls -la\"", "gestura tools shell run -- cwd=. -- command=\"cargo test\""],
        },
        ToolDefinition {
            name: "git",
            summary: "Git status/diff/log operations for code workflows",
            inputs: &["repository path", "operation (status/diff/log/etc)", "options"],
            side_effects: &["Reads git metadata", "May change repo state for write operations"],
            examples: &["gestura tools git status", "gestura tools git diff --staged"],
        },
        ToolDefinition {
            name: "code",
            summary: "Code analysis helpers (search, summarize, inspect)",
            inputs: &["query", "paths", "options"],
            side_effects: &["Reads local code"],
            examples: &["gestura tools code search -- query=\"update_notification_settings\""],
        },
        ToolDefinition {
            name: "web",
            summary: "Fetch web pages and summarize content",
            inputs: &["url", "options"],
            side_effects: &["Performs network requests"],
            examples: &["gestura tools web fetch https://example.com"],
        },
        ToolDefinition {
            name: "permissions",
            summary: "Check/request OS-level permissions (platform-specific)",
            inputs: &["permission kind (microphone, accessibility, etc.)"],
            side_effects: &["May prompt the OS", "May open system settings"],
            examples: &["gestura tools permissions check", "gestura tools permissions request microphone"],
        },
    ];
    &TOOLS
}

/// Find a tool definition by name (case-insensitive).
pub fn find_tool(name: &str) -> Option<&'static ToolDefinition> {
    let name = name.trim().to_ascii_lowercase();
    all_tools().iter().find(|t| t.name.eq_ignore_ascii_case(&name))
}

/// Render a compact table of built-in tools (static, no config needed).
pub fn render_tools_overview() -> String {
    let mut out = String::new();
    out.push_str("**Built-in Tools:**\n\n");
    for t in all_tools() {
        out.push_str(&format!("• **{}** - {}\n", t.name, t.summary));
    }
    out.push_str("\nUse `/tools <name>` for details on a specific tool.");
    out.push_str("\nUse `/capabilities` for full system status including MCP servers and devices.");
    out
}

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
    if config.mcp_tools.is_empty() {
        out.push_str("_No MCP servers configured._\n");
    } else {
        for mcp in &config.mcp_tools {
            out.push_str(&format!("• **{}** → `{}`\n", mcp.name, mcp.endpoint));
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
        out.push_str(&format!("• **Ollama:** {} @ {}\n", ollama.model, ollama.base_url));
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
    out.push_str(&format!("• **Developer Mode:** {}\n", if config.developer.developer_mode { "enabled" } else { "disabled" }));
    out.push_str(&format!("• **Simulators:** {}\n", if config.developer.enable_simulators { "enabled" } else { "disabled" }));
    if config.developer.enable_simulators {
        out.push_str(&format!("• **Auto-discover Simulators:** {}\n", config.developer.auto_discover_simulators));
        out.push_str(&format!("• **Simulator Pattern:** {}\n", config.developer.simulator.device_name_pattern));
    }

    // Hotkey
    out.push_str("\n## System\n\n");
    out.push_str(&format!("• **Hotkey:** {}\n", config.hotkey_listen));
    out.push_str(&format!("• **Grace Period:** {}s\n", config.grace_period_secs));

    out.push_str("\n---\nUse `/tools <name>` for details on a specific tool.");
    out
}

/// Render a detailed tool description.
pub fn render_tool_detail(name: &str) -> Option<String> {
    let t = find_tool(name)?;
    let mut out = String::new();
    out.push_str(&format!("**{}**\n\n", t.name));
    out.push_str(&format!("{}\n\n", t.summary));

    if !t.inputs.is_empty() {
        out.push_str("**Inputs:**\n");
        for i in t.inputs {
            out.push_str(&format!("• {}\n", i));
        }
        out.push('\n');
    }

    if !t.side_effects.is_empty() {
        out.push_str("**Side Effects:**\n");
        for s in t.side_effects {
            out.push_str(&format!("⚠ {}\n", s));
        }
        out.push('\n');
    }

    if !t.examples.is_empty() {
        out.push_str("**Examples:**\n");
        for e in t.examples {
            out.push_str(&format!("```\n{}\n```\n", e));
        }
    }
    Some(out)
}

/// Heuristic: decide whether a user message is asking for a tool inventory.
pub fn looks_like_tools_question(input: &str) -> bool {
    let s = input.trim().to_ascii_lowercase();
    s == "tools"
        || s == "tool"
        || s == "/tools"
        || s.contains("what tools")
        || s.contains("available tools")
        || s.contains("which tools")
        || s.contains("list tools")
        || s.contains("tool list")
        || s.contains("show tools")
        || s.contains("show me tools")
}

/// Heuristic: decide whether a user message is asking for full capabilities/config.
pub fn looks_like_capabilities_question(input: &str) -> bool {
    let s = input.trim().to_ascii_lowercase();
    s == "/capabilities"
        || s == "capabilities"
        || s.contains("what can you do")
        || s.contains("what do you have access to")
        || s.contains("have access to")
        || s.contains("what are your capabilities")
        || s.contains("show capabilities")
        || s.contains("mcp servers")
        || s.contains("mcp tools")
        || s.contains("configured tools")
        || s.contains("system status")
        || s.contains("current config")
        || s.contains("device settings")
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

