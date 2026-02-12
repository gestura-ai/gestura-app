//! Tool Registry
//!
//! Provides an authoritative, deterministic inventory of Gestura's built-in
//! tools. This is used by interactive chat commands (e.g. `/tools`) and for
//! answering common "what tools do you have" questions without relying on an
//! LLM response.
//!
//!
//! Note: This registry is intentionally **static** and does not depend on the
//! `gestura-core` facade's `AppConfig`. Dynamic, configuration-dependent
//! capability rendering lives in the `gestura-core` facade crate.

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
    static TOOLS: [ToolDefinition; 12] = [
        ToolDefinition {
            name: "file",
            summary: "Read/write/list files and directories (workspace & sandbox-aware)",
            inputs: &[
                "path",
                "operation (read/write/list)",
                "content (for write)",
                "options",
            ],
            side_effects: &["Reads local files", "May write/modify local files"],
            examples: &[
                "gestura tools file read ./README.md",
                "gestura tools file write ./notes.txt --content \"hello\"",
            ],
        },
        ToolDefinition {
            name: "shell",
            summary: "Run shell commands in a controlled environment",
            inputs: &["command", "cwd", "env"],
            side_effects: &[
                "Executes local processes",
                "May modify local state depending on command",
            ],
            examples: &[
                "gestura tools shell run -- command=\"ls -la\"",
                "gestura tools shell run -- cwd=. -- command=\"cargo test\"",
            ],
        },
        ToolDefinition {
            name: "git",
            summary: "Git status/diff/log operations for code workflows",
            inputs: &[
                "repository path",
                "operation (status/diff/log/etc)",
                "options",
            ],
            side_effects: &[
                "Reads git metadata",
                "May change repo state for write operations",
            ],
            examples: &[
                "gestura tools git status",
                "gestura tools git diff --staged",
            ],
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
            name: "web_search",
            summary: "Search the web using configurable providers (Local/SerpAPI/DuckDuckGo/Brave)",
            inputs: &["query", "max_results", "provider (optional)"],
            side_effects: &["Performs network requests", "May use API quotas"],
            examples: &[
                "gestura tools web_search \"rust async patterns\"",
                "gestura tools web_search \"latest AI news\" --max-results 5",
            ],
        },
        ToolDefinition {
            name: "a2a",
            summary: "Agent-to-Agent protocol for delegating tasks to remote agents",
            inputs: &["agent_url", "task_message", "auth_token (optional)"],
            side_effects: &[
                "Performs network requests",
                "May execute tasks on remote agents",
            ],
            examples: &[
                "gestura tools a2a discover https://agent.example.com",
                "gestura tools a2a send https://agent.example.com \"Summarize this document\"",
            ],
        },
        ToolDefinition {
            name: "permissions",
            summary: "Check/request OS-level permissions (platform-specific)",
            inputs: &["permission kind (microphone, accessibility, etc.)"],
            side_effects: &["May prompt the OS", "May open system settings"],
            examples: &[
                "gestura tools permissions check",
                "gestura tools permissions request microphone",
            ],
        },
        ToolDefinition {
            name: "mcp",
            summary: "Model Context Protocol tools from connected MCP servers",
            inputs: &["server_name", "tool_name", "arguments"],
            side_effects: &["Depends on the specific MCP tool being invoked"],
            examples: &[
                "gestura tools mcp list",
                "gestura tools mcp call filesystem read_file --path ./README.md",
            ],
        },
        ToolDefinition {
            name: "task",
            summary: "Manage tasks for the current session: create, update status, list, organize hierarchies",
            inputs: &[
                "operation (create/update_status/update/delete/list/get_hierarchy)",
                "task_id (for update/delete)",
                "name (for create)",
                "description (optional)",
                "status (for update_status)",
                "parent_id (optional, for subtasks)",
            ],
            side_effects: &[
                "Creates/modifies/deletes task files in .gestura/tasks/",
                "Persists task state across sessions",
            ],
            examples: &[
                "task create --name 'Implement feature' --description 'Add new API endpoint'",
                "task update_status --task_id abc123 --status inprogress",
                "task list",
                "task create --name 'Write tests' --parent_id abc123",
            ],
        },
        ToolDefinition {
            name: "screenshot",
            summary: "Capture screenshots of the screen or specific regions",
            inputs: &[
                "output_format (optional: png/jpg)",
                "output_path (optional; default artifact path)",
                "return (optional: mode=path|inline_base64 + inline bounds)",
                "region (optional: x,y,width,height)",
                "display (optional: display number)",
            ],
            side_effects: &[
                "Captures screen content (privacy-sensitive)",
                "Creates image file on disk",
                "May prompt for OS screen recording permission",
            ],
            examples: &[
                "{\"output_path\":\"./screen.png\"}",
                "{}",
                "{\"output_format\":\"jpg\"}",
                "{\"output_path\":\"./region.png\",\"region\":{\"x\":0,\"y\":0,\"width\":800,\"height\":600}}",
            ],
        },
        ToolDefinition {
            name: "screen_record",
            summary: "Record screen video with start/stop controls",
            inputs: &[
                "operation (start/stop)",
                "output_format (optional for start: mp4/mov)",
                "output_path (optional for start; default artifact path)",
                "recording_id (for stop)",
                "region (optional: x,y,width,height)",
                "display (optional: display number)",
            ],
            side_effects: &[
                "Captures screen content (privacy-sensitive)",
                "Creates video file on disk",
                "May prompt for OS screen recording permission",
                "Spawns background recording process",
            ],
            examples: &[
                "{\"operation\":\"start\",\"output_path\":\"./recording.mp4\"}",
                "{\"operation\":\"start\"}",
                "{\"operation\":\"start\",\"output_format\":\"mov\"}",
                "{\"operation\":\"stop\",\"recording_id\":\"<recording_id>\"}",
            ],
        },
    ];
    &TOOLS
}

/// Find a tool definition by name (case-insensitive).
pub fn find_tool(name: &str) -> Option<&'static ToolDefinition> {
    let name = name.trim().to_ascii_lowercase();
    all_tools()
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(&name))
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
