//! Tool Registry
//!
//! Provides an authoritative, deterministic inventory of Gestura's built-in
//! tools. This is used by interactive chat commands (e.g. `/tools`) and for
//! answering common "what tools do you have" questions without relying on an
//! LLM response.

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
    // Keep in sync with `crates/gestura-cli/src/commands/tools/*`.
    static TOOLS: [ToolDefinition; 6] = [
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
            name: "permissions",
            summary: "Check/request OS-level permissions (platform-specific)",
            inputs: &["permission kind (microphone, accessibility, etc.)"],
            side_effects: &["May prompt the OS", "May open system settings"],
            examples: &[
                "gestura tools permissions check",
                "gestura tools permissions request microphone",
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

/// Render a compact table of tools for display in interactive chat.
pub fn render_tools_overview() -> String {
    // Calculate max name width for alignment
    let name_width = all_tools()
        .iter()
        .map(|t| t.name.len())
        .max()
        .unwrap_or(10)
        .max(10);

    let mut out = String::new();
    out.push_str("┌─ Tools ───────────────────────────────────────────────────┐\n");
    out.push_str("│                                                           │\n");
    for t in all_tools() {
        // Truncate summary if too long to fit in ~50 chars
        let summary = if t.summary.len() > 48 {
            format!("{}…", &t.summary[..47])
        } else {
            t.summary.to_string()
        };
        out.push_str(&format!(
            "│  {:width$}  │  {:<48} │\n",
            t.name,
            summary,
            width = name_width
        ));
    }
    out.push_str("│                                                           │\n");
    out.push_str("├───────────────────────────────────────────────────────────┤\n");
    out.push_str("│  /tools <name>  show details   gestura tools -h  CLI help │\n");
    out.push_str("└───────────────────────────────────────────────────────────┘\n");
    out
}

/// Render a detailed tool description in compact table style.
pub fn render_tool_detail(name: &str) -> Option<String> {
    let t = find_tool(name)?;
    let mut out = String::new();

    // Header
    out.push_str(&format!(
        "┌─ {} ─────────────────────────────────────────────────┐\n",
        t.name
    ));
    out.push_str("│                                                           │\n");

    // Summary (word-wrap if needed)
    for line in textwrap_simple(t.summary, 55) {
        out.push_str(&format!("│  {:<56}│\n", line));
    }
    out.push_str("│                                                           │\n");

    // Inputs section
    if !t.inputs.is_empty() {
        out.push_str("├─ Inputs ──────────────────────────────────────────────────┤\n");
        for i in t.inputs {
            out.push_str(&format!("│  • {:<54}│\n", i));
        }
    }

    // Side effects section
    if !t.side_effects.is_empty() {
        out.push_str("├─ Side Effects ────────────────────────────────────────────┤\n");
        for s in t.side_effects {
            out.push_str(&format!("│  ⚠ {:<54}│\n", s));
        }
    }

    // Examples section
    if !t.examples.is_empty() {
        out.push_str("├─ Examples ────────────────────────────────────────────────┤\n");
        for e in t.examples {
            // Truncate long examples
            let ex = if e.len() > 54 {
                format!("{}…", &e[..53])
            } else {
                e.to_string()
            };
            out.push_str(&format!("│  $ {:<54}│\n", ex));
        }
    }

    out.push_str("└───────────────────────────────────────────────────────────┘\n");

    Some(out)
}

/// Simple word-wrap helper (no external dep for this small use case).
fn textwrap_simple(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_line = String::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            current_line = word.to_string();
        } else if current_line.len() + 1 + word.len() <= width {
            current_line.push(' ');
            current_line.push_str(word);
        } else {
            lines.push(current_line);
            current_line = word.to_string();
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

/// Heuristic: decide whether a user message is asking for a tool inventory.
pub fn looks_like_tools_question(input: &str) -> bool {
    let s = input.trim().to_ascii_lowercase();
    s == "tools"
        || s == "tool"
        || s.contains("what tools")
        || s.contains("available tools")
        || s.contains("which tools")
        || s.contains("list tools")
        || s.contains("tool list")
}

/// Render a summary of AI capabilities for the /capabilities command.
pub fn render_capabilities() -> String {
    let mut out = String::new();
    out.push_str("┌─ Gestura AI Capabilities ────────────────────────────────┐\n");
    out.push_str("│                                                           │\n");
    out.push_str("│  🎤 Voice Input                                           │\n");
    out.push_str("│     • Real-time speech-to-text transcription              │\n");
    out.push_str("│     • Wake word detection (\"Hey Gestura\")                 │\n");
    out.push_str("│     • Continuous listening mode                           │\n");
    out.push_str("│                                                           │\n");
    out.push_str("│  🤖 AI Providers                                          │\n");
    out.push_str("│     • OpenAI (GPT-4, GPT-4o, GPT-3.5)                     │\n");
    out.push_str("│     • Anthropic (Claude 3.5, Claude 3)                    │\n");
    out.push_str("│     • Ollama (local models)                               │\n");
    out.push_str("│                                                           │\n");
    out.push_str("│  🔧 Tool Execution                                        │\n");
    out.push_str(&format!(
        "│     • {} built-in tools (file, shell, git, code, web)    │\n",
        all_tools().len()
    ));
    out.push_str("│     • MCP server integration                              │\n");
    out.push_str("│     • Sandboxed execution environment                     │\n");
    out.push_str("│                                                           │\n");
    out.push_str("│  💬 Chat Features                                         │\n");
    out.push_str("│     • Streaming responses                                 │\n");
    out.push_str("│     • Session persistence                                 │\n");
    out.push_str("│     • Multi-turn conversations                            │\n");
    out.push_str("│                                                           │\n");
    out.push_str("└───────────────────────────────────────────────────────────┘\n");
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
        assert!(!looks_like_tools_question("tell me a joke"));
    }
}
