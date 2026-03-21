//! Tool Registry
//!
//! Provides an authoritative, deterministic inventory of Gestura's built-in
//! tools. This is used by interactive agent commands (e.g. `/tools`) and for
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
    /// Short summary of what the tool does (one-liner, shown in tool lists).
    pub summary: &'static str,
    /// Rich LLM-facing description used in routing prompts and provider tool schemas.
    ///
    /// Should be 2-4 sentences explaining *when* to use this tool, *what* it does,
    /// and any important caveats. More expressive than `summary`.
    pub description: &'static str,
    /// Keywords for semantic tool matching during pre-flight routing.
    ///
    /// These are lower-case tokens that strongly correlate with a user request
    /// that should trigger this tool. Used by keyword and hybrid routers.
    pub keywords: &'static [&'static str],
    /// Human-readable list of inputs / parameters.
    pub inputs: &'static [&'static str],
    /// Human-readable list of side effects / security implications.
    pub side_effects: &'static [&'static str],
    /// Example invocations.
    pub examples: &'static [&'static str],
}

/// Canonical built-in code tool names, including the legacy aggregate `code`
/// entry and the newer split code tools.
pub fn code_tool_names() -> &'static [&'static str] {
    const CODE_TOOL_NAMES: &[&str] = &[
        "code",
        "code_read_files",
        "code_edit_files",
        "code_outline",
        "code_symbols",
        "code_references",
        "code_definition",
        "code_glob",
        "code_grep",
        "code_map",
        "code_stats",
        "code_deps",
        "code_lint",
        "code_test",
    ];

    CODE_TOOL_NAMES
}

/// Return whether a tool name belongs to the code tool family.
pub fn is_code_tool_name(name: &str) -> bool {
    code_tool_names()
        .iter()
        .any(|tool_name| tool_name.eq_ignore_ascii_case(name.trim()))
}

/// Return the set of built-in tools.
pub fn all_tools() -> &'static [ToolDefinition] {
    static TOOLS: &[ToolDefinition] = &[
        ToolDefinition {
            name: "file",
            summary: "Read/write/list files and directories (workspace & sandbox-aware)",
            description: "Read, write, edit, list, search, and navigate files and directories in \
                the workspace. Use this tool whenever the user wants to open, create, modify, \
                delete, or inspect file or directory contents. Workspace-sandboxed to prevent \
                accidental access outside the project root.",
            keywords: &[
                "file",
                "read",
                "write",
                "edit",
                "create",
                "delete",
                "save",
                "directory",
                "folder",
                "path",
                "list",
                "search",
                "content",
                "text",
                "document",
                "open",
                "load",
                "cat",
                "find",
                "ls",
                "tree",
            ],
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
            description: "Execute shell commands, scripts, and programs in a controlled \
                environment. Use this tool to run build systems (cargo, npm, make), run tests, \
                lint code, install packages, or perform any other command-line operation. \
                Runs in the workspace directory by default.",
            keywords: &[
                "shell", "run", "execute", "command", "terminal", "bash", "script", "process",
                "npm", "cargo", "make", "build", "test", "lint", "install", "start", "stop",
                "restart", "deploy", "compile", "launch",
            ],
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
            description: "Interact with Git repositories: show status, view diffs, read commit \
                history, manage branches, stage and commit changes, stash, and resolve conflicts. \
                Use this tool whenever the user asks about code changes, version history, \
                branches, or anything related to source control.",
            keywords: &[
                "git",
                "commit",
                "branch",
                "diff",
                "status",
                "log",
                "merge",
                "push",
                "pull",
                "repository",
                "version",
                "history",
                "staged",
                "stash",
                "rebase",
                "checkout",
                "blame",
                "conflict",
                "tag",
                "remote",
            ],
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
            summary: "Analyze, search, edit, lint, and test code with a compatibility-friendly aggregate interface",
            description: "Inspect and operate on source code using the legacy aggregate code tool interface. Use this when the model or session configuration expects a single `code` tool that can route to operations like stats, map, symbols, grep, batch_read, batch_edit, lint, and test. The runtime also exposes stricter split code tools, but this compatibility entry remains available for existing prompts, policies, and sessions.",
            keywords: &[
                "code",
                "source",
                "read",
                "edit",
                "refactor",
                "search",
                "grep",
                "symbols",
                "definition",
                "references",
                "outline",
                "map",
                "stats",
                "deps",
                "lint",
                "test",
                "batch",
                "files",
            ],
            inputs: &[
                "operation",
                "path",
                "symbol / pattern (operation-specific)",
                "paths / edits / options (operation-specific)",
            ],
            side_effects: &[
                "Reads local code files",
                "May write local code files for batch_edit",
                "May execute local subprocesses for lint/test",
            ],
            examples: &[
                "Code stats: {operation:stats, path:.}",
                "Read files: {operation:batch_read, paths:[src/main.rs, src/lib.rs]}",
                "Edit files: {operation:batch_edit, edits:[{path:src/lib.rs, old_str:old, new_str:new}]}",
            ],
        },
        ToolDefinition {
            name: "code_read_files",
            summary: "Read one or more exact source files with a strict file-only contract",
            description: "Read exact file contents for known source files. Use this after you know the specific file paths you need. Pass `paths`; do not pass directory roots or edit-style arguments.",
            keywords: &[
                "code", "read", "file", "source", "contents", "open", "inspect",
            ],
            inputs: &["paths"],
            side_effects: &["Reads local code files"],
            examples: &["Read files: {paths:[src/main.rs, src/lib.rs]}"],
        },
        ToolDefinition {
            name: "code_edit_files",
            summary: "Apply exact str-replace edits across one or more files with a strict schema",
            description: "Edit existing files using an `edits` array. Each edit must include only `path`, `old_str`, and `new_str`. This tool is strict: do not pass directories, `pattern`, `symbol`, or read-style fields.",
            keywords: &["code", "edit", "replace", "refactor", "rewrite", "modify"],
            inputs: &["edits"],
            side_effects: &["Writes local code files"],
            examples: &[
                "Edit files: {edits:[{path:src/lib.rs, old_str:old_name, new_str:new_name}]}",
            ],
        },
        ToolDefinition {
            name: "code_outline",
            summary: "Return a structured outline for one exact source file",
            description: "Extract the outline of a single file, including functions, structs, enums, and impls. Requires an exact file path.",
            keywords: &["code", "outline", "api", "file", "structure", "symbols"],
            inputs: &["path"],
            side_effects: &["Reads local code files"],
            examples: &["Outline a file: {path:crates/my-crate/src/lib.rs}"],
        },
        ToolDefinition {
            name: "code_symbols",
            summary: "Extract top-level symbols from one exact source file",
            description: "List the top-level symbols from a file. Requires an exact file path.",
            keywords: &["code", "symbols", "file", "function", "struct", "enum"],
            inputs: &["path"],
            side_effects: &["Reads local code files"],
            examples: &["Symbols: {path:crates/my-crate/src/lib.rs}"],
        },
        ToolDefinition {
            name: "code_references",
            summary: "Find references to a symbol across an existing code path",
            description: "Find references for a symbol inside a file or directory path.",
            keywords: &["code", "references", "symbol", "usage", "find"],
            inputs: &["symbol", "path"],
            side_effects: &["Reads local code files"],
            examples: &["References: {symbol:MyStruct, path:.}"],
        },
        ToolDefinition {
            name: "code_definition",
            summary: "Jump to the first definition of a symbol across an existing code path",
            description: "Find the first definition for a symbol inside a file or directory path.",
            keywords: &["code", "definition", "symbol", "declare", "where"],
            inputs: &["symbol", "path"],
            side_effects: &["Reads local code files"],
            examples: &["Definition: {symbol:execute_tool, path:.}"],
        },
        ToolDefinition {
            name: "code_glob",
            summary: "Find files by glob pattern inside a directory",
            description: "Search for files using a glob pattern relative to a directory root.",
            keywords: &["code", "glob", "files", "match", "pattern", "discover"],
            inputs: &["path", "pattern", "max_results"],
            side_effects: &["Reads local code metadata"],
            examples: &["Glob files: {path:., pattern:**/*.rs}"],
        },
        ToolDefinition {
            name: "code_grep",
            summary: "Regex search code contents inside a directory",
            description: "Search file contents with a regex pattern, optional file glob filtering, and context lines.",
            keywords: &["code", "grep", "search", "regex", "text", "pattern"],
            inputs: &[
                "path",
                "pattern",
                "file_glob",
                "context_lines",
                "max_results",
            ],
            side_effects: &["Reads local code files"],
            examples: &[
                "Grep code: {path:., pattern:fn handle_request, file_glob:*.rs, context_lines:3}",
            ],
        },
        ToolDefinition {
            name: "code_map",
            summary: "Summarize repository structure for a directory",
            description: "Build a repository map for a directory root.",
            keywords: &["code", "map", "repo", "structure", "directory", "overview"],
            inputs: &["path", "max_depth"],
            side_effects: &["Reads local code metadata"],
            examples: &["Map repo: {path:., max_depth:3}"],
        },
        ToolDefinition {
            name: "code_stats",
            summary: "Compute line and language statistics for an existing path",
            description: "Compute language and line-count statistics for a file or directory path.",
            keywords: &["code", "stats", "lines", "languages", "count"],
            inputs: &["path"],
            side_effects: &["Reads local code metadata"],
            examples: &["Code stats: {path:.}"],
        },
        ToolDefinition {
            name: "code_deps",
            summary: "Inspect Cargo dependencies for an existing path",
            description: "Inspect Cargo dependencies for a manifest or project path.",
            keywords: &["code", "deps", "dependencies", "cargo", "manifest"],
            inputs: &["path"],
            side_effects: &["Reads local manifests"],
            examples: &["Dependencies: {path:.}"],
        },
        ToolDefinition {
            name: "code_lint",
            summary: "Run linting for a project directory",
            description: "Run cargo clippy for a project directory. This executes a subprocess.",
            keywords: &["code", "lint", "clippy", "cargo", "verify"],
            inputs: &["path", "fix"],
            side_effects: &[
                "Executes local lint subprocesses",
                "May modify files when fix=true",
            ],
            examples: &["Lint: {path:.}"],
        },
        ToolDefinition {
            name: "code_test",
            summary: "Run tests for a project directory",
            description: "Run cargo test for a project directory, optionally filtered.",
            keywords: &["code", "test", "cargo", "verify", "unit", "integration"],
            inputs: &["path", "filter"],
            side_effects: &["Executes local test subprocesses"],
            examples: &["Test: {path:., filter:my_test_name}"],
        },
        ToolDefinition {
            name: "web",
            summary: "Fetch web pages and summarize content",
            description: "Fetch and read content from web pages, converting HTML to readable \
                text. Use this tool when the user provides a URL or asks to retrieve information \
                from a specific website, read documentation, check a webpage, or download content \
                from the internet. Handles redirects and extracts main body text.",
            keywords: &[
                "web",
                "fetch",
                "url",
                "page",
                "website",
                "http",
                "https",
                "browse",
                "download",
                "documentation",
                "internet",
                "online",
                "link",
                "site",
                "visit",
                "open",
                "retrieve",
                "get",
                "html",
                "content",
            ],
            inputs: &["url", "options"],
            side_effects: &["Performs network requests"],
            examples: &["gestura tools web fetch https://example.com"],
        },
        ToolDefinition {
            name: "web_search",
            summary: "Search the web using configurable providers (Local/SerpAPI/DuckDuckGo/Brave)",
            description: "Search the internet using configurable search providers (DuckDuckGo, \
                Brave, SerpAPI). Use this tool when the user wants to find information online, \
                look up a topic, research a question, find recent news or events, or discover \
                relevant web resources. Returns ranked result snippets with URLs.",
            keywords: &[
                "search",
                "google",
                "find",
                "lookup",
                "query",
                "results",
                "internet",
                "online",
                "discover",
                "research",
                "news",
                "information",
                "duckduckgo",
                "brave",
                "look up",
                "what is",
                "how to",
                "latest",
            ],
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
            description: "Delegate tasks to remote AI agents using the Agent-to-Agent (A2A) \
                protocol. Use this tool when the user wants to offload a subtask to a specialized \
                remote agent, discover what remote agents are available, or orchestrate multi-agent \
                workflows where different agents collaborate on a larger goal.",
            keywords: &[
                "agent",
                "a2a",
                "delegate",
                "remote",
                "orchestrate",
                "multi-agent",
                "protocol",
                "worker",
                "supervisor",
                "task",
                "handoff",
                "subagent",
                "collaborate",
            ],
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
            description: "Check and request OS-level permissions such as microphone access, \
                accessibility, screen recording, and camera. Use this tool when a feature \
                requires a system permission that may not yet be granted, or when the user \
                asks what permissions the app has or needs.",
            keywords: &[
                "permission",
                "permissions",
                "microphone",
                "accessibility",
                "access",
                "privacy",
                "camera",
                "system",
                "grant",
                "request",
                "check",
                "screen recording",
                "allow",
            ],
            inputs: &["permission kind (microphone, accessibility, etc.)"],
            side_effects: &["May prompt the OS", "May open system settings"],
            examples: &[
                "gestura tools permissions check",
                "gestura tools permissions request microphone",
            ],
        },
        ToolDefinition {
            name: "mcp",
            summary: "Search, evaluate, install, enable, disable, and manage MCP servers from the official registry",
            description: "Discover and manage Model Context Protocol (MCP) servers from the official \
                registry at registry.modelcontextprotocol.io. Use this tool when the user wants to \
                find new MCP servers by keyword, evaluate a specific server's capabilities and \
                requirements, install a server into .mcp.json, enable or disable configured servers, \
                list installed servers, or remove a server. Always start with operation=search to \
                find candidates, then operation=evaluate to review details, then operation=install \
                to add the server — the tool provides LLM workflow guidance at each step.",
            keywords: &[
                "mcp",
                "install",
                "search",
                "discover",
                "registry",
                "manage",
                "configure",
                "browse",
                "npm",
                "npx",
                "enable",
                "disable",
                "remove",
                "server",
                "protocol",
                "extension",
                "plugin",
                "external",
                "connect",
                "capability",
                "model context",
                "integration",
                "list",
                "add",
                "setup",
                "pypi",
                "docker",
                "oci",
                "stdio",
                "http",
                "tool",
            ],
            inputs: &[
                "operation (search/evaluate/install/enable/disable/list/remove/info)",
                "query (for search)",
                "limit (for search, default 20)",
                "server_id (for evaluate/install/info)",
                "name (local alias for install/enable/disable/remove)",
                "scope (project|user, default project)",
                "transport (stdio|http, for install override)",
                "command (for install stdio override)",
                "args (array, for install stdio override)",
                "url (for install http override)",
                "env (object, env vars for install)",
            ],
            side_effects: &[
                "search/evaluate/info: network request to registry.modelcontextprotocol.io",
                "install/enable/disable/remove: modifies .mcp.json on disk",
            ],
            examples: &[
                "{\"operation\":\"search\",\"query\":\"filesystem\"}",
                "{\"operation\":\"search\",\"query\":\"github\",\"limit\":10}",
                "{\"operation\":\"evaluate\",\"server_id\":\"io.github.modelcontextprotocol/server-filesystem\"}",
                "{\"operation\":\"install\",\"server_id\":\"io.github.modelcontextprotocol/server-filesystem\",\"name\":\"filesystem\",\"scope\":\"project\"}",
                "{\"operation\":\"install\",\"server_id\":\"io.github.exa/exa\",\"env\":{\"EXA_API_KEY\":\"<key>\"}}",
                "{\"operation\":\"list\",\"scope\":\"project\"}",
                "{\"operation\":\"enable\",\"name\":\"filesystem\"}",
                "{\"operation\":\"disable\",\"name\":\"filesystem\"}",
                "{\"operation\":\"remove\",\"name\":\"filesystem\",\"scope\":\"project\"}",
                "{\"operation\":\"info\",\"server_id\":\"io.github.modelcontextprotocol/server-github\"}",
            ],
        },
        ToolDefinition {
            name: "task",
            summary: "Manage tasks for the current session: create, update status, list, organize hierarchies",
            description: "Create, update, list, and organize tasks and work items for the \
                current session. Use this tool when the user wants to track progress on work, \
                create a to-do list, mark items as done, build subtask hierarchies, or manage \
                any kind of structured checklist or work breakdown. For `update_status`, ALWAYS \
                provide both `task_id` and `status` in the same call; do not call `update_status` \
                with only `task_id`, and do not use it just to confirm/preserve the current state. \
                If no status changed, skip the task update and continue the real work.",
            keywords: &[
                "task",
                "todo",
                "work",
                "track",
                "checklist",
                "reminder",
                "subtask",
                "status",
                "create",
                "list",
                "progress",
                "organize",
                "mark",
                "done",
                "complete",
                "in progress",
                "workflow",
                "breakdown",
            ],
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
                "{\"operation\":\"update_status\",\"task_id\":\"abc123\",\"status\":\"completed\"}",
                "task list",
                "task create --name 'Write tests' --parent_id abc123",
            ],
        },
        ToolDefinition {
            name: "screenshot",
            summary: "Capture screenshots of the screen or specific regions",
            description: "Capture screenshots of the full screen, a specific display, or a \
                defined region. Use this tool when the user wants to take a picture of their \
                screen, capture a UI state, snap a region, or save a visual record. Returns \
                a file path or inline base64 image. Requires screen recording permission.",
            keywords: &[
                "screenshot",
                "capture",
                "screen",
                "image",
                "photo",
                "snap",
                "grab",
                "display",
                "picture",
                "png",
                "jpg",
                "snapshot",
                "screengrab",
                "take a picture",
                "show me",
            ],
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
            description: "Record the screen as a video file (MP4 or MOV). Start a recording \
                session with optional region and display selection, then stop it to produce a \
                file. Use this tool when the user wants to create a screencast, demonstrate \
                a workflow, record a tutorial, or capture video of any on-screen activity.",
            keywords: &[
                "record",
                "recording",
                "video",
                "screen",
                "capture",
                "screencast",
                "demonstration",
                "demo",
                "tutorial",
                "mp4",
                "mov",
                "film",
                "show",
                "create a video",
                "make a video",
            ],
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
        ToolDefinition {
            name: "gui_control",
            summary: "Drive the application GUI for self-demonstrations",
            description: "Control the Gestura application GUI programmatically: toggle view \
                modes, open or close the file explorer, navigate to the chat panel, or open \
                configuration screens. Use this tool when the agent needs to demonstrate its \
                own interface or navigate UI panels as part of a self-demonstration workflow.",
            keywords: &[
                "gui",
                "interface",
                "view",
                "mode",
                "explorer",
                "editor",
                "chat",
                "navigate",
                "toggle",
                "window",
                "ui",
                "panel",
                "open",
                "close",
                "config",
                "settings",
            ],
            inputs: &[
                "action (toggle_view_mode/open_explorer/close_explorer/open_chat/close_chat/navigate_config)",
                "target (optional argument depending on action)",
            ],
            side_effects: &["Changes the physical view in the user's GUI"],
            examples: &[
                "{\"action\":\"toggle_view_mode\"}",
                "{\"action\":\"open_explorer\"}",
            ],
        },
    ];
    TOOLS
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
        assert!(find_tool("code").is_some());
    }

    #[test]
    fn code_tool_helpers_cover_legacy_and_split_names() {
        assert!(is_code_tool_name("code"));
        assert!(is_code_tool_name("code_edit_files"));
        assert!(is_code_tool_name("code_read_files"));
        assert!(!is_code_tool_name("shell"));
    }

    #[test]
    fn looks_like_tools_question_matches_common_phrases() {
        assert!(looks_like_tools_question("what tools do you have?"));
        assert!(looks_like_tools_question("list tools"));
        assert!(looks_like_tools_question("/tools"));
        assert!(!looks_like_tools_question("tell me a joke"));
    }
}
