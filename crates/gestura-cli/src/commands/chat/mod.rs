//! Interactive chat command

use super::Result;
use colored::Colorize;
use gestura_core::{
    AgentPipeline, AgentRequest, AppConfig, AppConfigSecurityExt, AudioCaptureConfig,
    CancellationToken, PermissionLevel, RequestSource, SessionToolSettings, SpeechProcessorCoreExt,
    StreamChunk,
    chat_sessions::{
        ChatSessionStore, FileChatSessionStore, MessageSource, SessionToolSettingsConfigExt,
    },
    get_speech_processor,
    tool_confirmation::{TOOL_CONFIRMATIONS, ToolConfirmationDecision},
};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

mod markdown_ansi;
mod tui;

/// Options for the chat command
#[derive(Debug, Default)]
pub struct ChatOptions<'a> {
    pub model: Option<&'a str>,
    pub resume: bool,
    pub session: Option<&'a str>,
    pub tui: bool,
    pub voice: bool,
    pub system: Option<&'a str>,
}

/// Persisted chat message.
///
/// This is a re-export of the canonical message type from `gestura-core`, so the
/// CLI (including the TUI) does not maintain a divergent persistence model.
pub use gestura_core::chat_sessions::ConversationMessage as ChatMessage;

/// Persisted chat session.
///
/// The CLI uses the canonical core session type; all persistence is performed
/// via the core-backed `FileChatSessionStore`.
pub use gestura_core::chat_sessions::ChatSession;

/// Session listing filter options.
pub use gestura_core::chat_sessions::SessionFilter;

/// Session metadata returned by `list_sessions*`.
pub use gestura_core::chat_sessions::SessionInfo;

/// Return the CLI session store (file-backed, one JSON file per session).
fn session_store() -> FileChatSessionStore {
    FileChatSessionStore::new_default()
}

/// Create a new CLI session.
///
/// The CLI prefers using the current working directory as the session workspace
/// (so file/shell tools operate in the user's project), but falls back to a
/// sandbox workspace if the CWD cannot be determined.
fn new_cli_session(model: Option<String>) -> Result<ChatSession> {
    match std::env::current_dir() {
        Ok(cwd) => ChatSession::new_with_workspace(cwd, model)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Err(_) => {
            ChatSession::new_sandbox(model).map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
        }
    }
}

/// Prompt the user for a scoped tool-confirmation decision.
///
/// This is the CLI basic (non-TUI) adapter for Claude Code-style permission choices.
/// The actual policy enforcement (session caches + persisted allow-always) is implemented
/// in `gestura-core`; the CLI only collects user intent.
fn prompt_tool_confirmation_decision() -> ToolConfirmationDecision {
    loop {
        println!(
            "  Choose: [1] allow once  [2] allow session  [3] allow always  [4] deny once  [5] deny session"
        );
        print!("  Selection (default 4): ");
        let _ = std::io::stdout().flush();

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_err() {
            return ToolConfirmationDecision::DenyOnce;
        }

        return match input.trim() {
            "1" => ToolConfirmationDecision::AllowOnce,
            "2" => ToolConfirmationDecision::AllowSession,
            "3" => ToolConfirmationDecision::AllowAlways,
            "4" | "" => ToolConfirmationDecision::DenyOnce,
            "5" => ToolConfirmationDecision::DenySession,
            other => {
                println!("  {} Invalid selection '{other}'.", "✗".red());
                continue;
            }
        };
    }
}

/// Ensure the session has tool settings configured.
///
/// The unified session model stores tool settings inside `ChatSession.state.tool_settings`.
/// Older sessions (or shells that didn't initialize settings) may have this field missing.
///
/// Returns `true` if the session was updated.
pub(super) fn ensure_session_tool_settings(session: &mut ChatSession, config: &AppConfig) -> bool {
    if session.state.tool_settings.is_some() {
        return false;
    }

    session.state.tool_settings = Some(SessionToolSettings::from_global_config(config));
    true
}

/// Derive the effective tool execution policy for an `AgentRequest`.
///
/// - `PermissionLevel` is used for runtime gating (sandbox/restricted/full)
/// - `allowed_tools` is used for tool visibility to the LLM (empty = all tools)
pub(super) fn derive_request_policy(session: &ChatSession) -> (PermissionLevel, Vec<String>) {
    let Some(settings) = session.state.tool_settings.as_ref() else {
        // Backstop for legacy sessions: preserve existing CLI behavior.
        return (PermissionLevel::Restricted, Vec::new());
    };

    let permission_level = settings.permission_level.to_pipeline();

    let mut allowed_tools: Vec<String> = settings
        .enabled_tools
        .iter()
        .filter(|(_, enabled)| **enabled)
        .map(|(tool, _)| tool.clone())
        .collect();
    allowed_tools.sort();

    (permission_level, allowed_tools)
}

/// Persist a session to disk.
fn save_cli_session(session: &ChatSession) -> Result<()> {
    session_store()
        .save(session)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

/// Load a session by ID.
fn load_cli_session(id: &str) -> Result<ChatSession> {
    session_store()
        .load(id)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

/// Load the most recently active session, if any.
fn load_last_cli_session() -> Result<Option<ChatSession>> {
    session_store()
        .load_last()
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

/// Delete a session by ID.
fn delete_cli_session(id: &str) -> Result<bool> {
    session_store()
        .delete(id)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

/// Export a session to a specific file.
fn export_cli_session(session: &ChatSession, path: &Path) -> Result<()> {
    let json = session
        .to_pretty_json()
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    fs::write(path, json)?;
    Ok(())
}

/// Handle `/tools` subcommands in basic (readline) mode.
///
/// - no args              → list tools with enabled status
/// - `<name>`             → show detail for a specific tool
/// - `enable <name>`      → enable a tool
/// - `disable <name>`     → disable a tool
fn basic_mode_tools_command(args: &[&str], session: &mut ChatSession) {
    let tools = gestura_core::tools::all_tools();
    let enabled_map = session
        .state
        .tool_settings
        .as_ref()
        .map(|s| &s.enabled_tools);

    match args {
        // enable / disable
        [verb, name, ..]
            if verb.eq_ignore_ascii_case("enable") || verb.eq_ignore_ascii_case("disable") =>
        {
            let want_enabled = verb.eq_ignore_ascii_case("enable");
            if gestura_core::tools::find_tool(name).is_some() {
                let tool_name = name.to_ascii_lowercase();
                let settings = session
                    .state
                    .tool_settings
                    .get_or_insert_with(Default::default);
                settings
                    .enabled_tools
                    .insert(tool_name.clone(), want_enabled);
                let _ = save_cli_session(session);
                let label = if want_enabled { "enabled" } else { "disabled" };
                println!("{} Tool '{}' {}", "✓".green(), tool_name, label);
            } else {
                println!(
                    "{}: Unknown tool '{}'. Try /tools to list.",
                    "error".red(),
                    name
                );
            }
        }
        // detail
        [name, ..] => match gestura_core::tools::render_tool_detail(name) {
            Some(text) => {
                let tool = gestura_core::tools::find_tool(name).unwrap();
                let is_enabled = enabled_map
                    .and_then(|m| m.get(tool.name).copied())
                    .unwrap_or(false);
                let status = if is_enabled {
                    format!("{}", "✓ enabled".green())
                } else {
                    format!("{}", "✗ disabled".red())
                };
                println!("  Status: {}", status);
                println!();
                println!("{}", markdown_ansi::markdown_to_ansi(&text));
                println!(
                    "{}",
                    format!(
                        "Use /tools {} <name> to toggle.",
                        if is_enabled { "disable" } else { "enable" }
                    )
                    .dimmed()
                );
            }
            None => println!(
                "{}: Unknown tool '{}'. Try /tools to list.",
                "error".red(),
                name
            ),
        },
        // list
        [] => {
            println!("{} {}", "◆".blue().bold(), "Built-in Tools:".blue());
            println!();
            for t in tools {
                let is_enabled = enabled_map
                    .and_then(|m| m.get(t.name).copied())
                    .unwrap_or(false);
                let indicator = if is_enabled {
                    format!("{}", "✓".green())
                } else {
                    format!("{}", "✗".red())
                };
                println!(
                    "  {} {:<16} {}",
                    indicator,
                    t.name.bold(),
                    t.summary.dimmed()
                );
            }
            println!();
            println!(
                "{}",
                "Use /tools <name> for details · /tools enable|disable <name> to toggle".dimmed()
            );
        }
    }
}

/// List all available sessions with metadata
pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    list_sessions_filtered(SessionFilter::All)
}

/// List sessions with optional date filtering
pub fn list_sessions_filtered(filter: SessionFilter) -> Result<Vec<SessionInfo>> {
    session_store()
        .list(filter)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

fn get_history_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gestura")
        .join("chat_history.txt")
}

fn model_for_provider(cfg: &AppConfig, provider: &str) -> Option<String> {
    match provider {
        "openai" => cfg.llm.openai.as_ref().map(|c| c.model.clone()),
        "anthropic" => cfg.llm.anthropic.as_ref().map(|c| c.model.clone()),
        "grok" => cfg.llm.grok.as_ref().map(|c| c.model.clone()),
        "gemini" => cfg.llm.gemini.as_ref().map(|c| c.model.clone()),
        "ollama" => cfg.llm.ollama.as_ref().map(|c| c.model.clone()),
        _ => None,
    }
}

pub fn run(opts: ChatOptions<'_>) -> Result<()> {
    // If TUI mode is requested, launch the TUI
    if opts.tui {
        return tui::run_tui(opts);
    }

    // Voice mode is handled within basic mode (voice input for prompts)
    // The voice flag enables voice-to-text for user input

    // Basic readline mode
    run_basic_mode(opts)
}

fn run_basic_mode(opts: ChatOptions<'_>) -> Result<()> {
    let ChatOptions {
        model,
        resume,
        session,
        voice,
        system,
        ..
    } = opts;

    // Print compact header (claude-code / codex style)
    println!();
    println!(
        "{} {}",
        "gestura".cyan().bold(),
        "— voice-first AI assistant".dimmed()
    );
    println!("{}", "─".repeat(50).dimmed());
    if voice {
        println!("{}", "  🎤 voice mode enabled (Enter to record)".yellow());
    }

    // Load or create session
    let mut chat_session = if resume {
        if let Some(id) = session {
            match load_cli_session(id) {
                Ok(s) => {
                    println!("{} Resuming session {}", "→".cyan(), id.dimmed());
                    s
                }
                Err(e) => {
                    eprintln!("{}: Failed to load session {}: {}", "error".red(), id, e);
                    std::process::exit(1);
                }
            }
        } else {
            match load_last_cli_session()? {
                Some(s) => {
                    println!("{} Resuming last session {}", "→".cyan(), s.id.dimmed());
                    s
                }
                None => {
                    println!(
                        "{}",
                        "No previous session found, starting new chat.".yellow()
                    );
                    new_cli_session(model.map(String::from))?
                }
            }
        }
    } else {
        new_cli_session(model.map(String::from))?
    };

    // Load config and set up provider
    let mut config = AppConfig::load();
    if let Some(m) = model.or(chat_session.model.as_deref())
        && let Some((provider, model_name)) = m.split_once(':')
    {
        config.llm.primary = provider.to_string();
        match provider {
            "openai" => {
                if let Some(ref mut openai) = config.llm.openai {
                    openai.model = model_name.to_string();
                }
            }
            "anthropic" => {
                if let Some(ref mut anthropic) = config.llm.anthropic {
                    anthropic.model = model_name.to_string();
                }
            }
            "grok" => {
                if let Some(ref mut grok) = config.llm.grok {
                    grok.model = model_name.to_string();
                }
            }
            "gemini" => {
                if let Some(ref mut gemini) = config.llm.gemini {
                    gemini.model = model_name.to_string();
                }
            }
            "ollama" => {
                if let Some(ref mut ollama) = config.llm.ollama {
                    ollama.model = model_name.to_string();
                }
            }
            _ => {}
        }
    }

    // Ensure persisted sessions have tool settings (migration / defaults).
    if ensure_session_tool_settings(&mut chat_session, &config) {
        save_cli_session(&chat_session)?;
    }

    // ─────────────────────────────────────────────────────────────────────────
    // HEADER: Full-width box with session info
    // ─────────────────────────────────────────────────────────────────────────
    let term_width = termsize::get().map(|s| s.cols as usize).unwrap_or(80);
    let inner_width = term_width.saturating_sub(4).max(40);

    println!();
    println!("{}", format!("╭{}╮", "─".repeat(inner_width + 2)).dimmed());

    // Title line
    let title = "gestura — voice-first AI assistant";
    let title_padding = inner_width.saturating_sub(title.len());
    println!(
        "{} {}{} {}",
        "│".dimmed(),
        title.cyan().bold(),
        " ".repeat(title_padding),
        "│".dimmed()
    );

    // Session info line
    let session_info = format!(
        "session {} · provider {} · model {}",
        &chat_session.id[..8],
        config.llm.primary,
        model.unwrap_or("default")
    );
    let session_padding = inner_width.saturating_sub(session_info.chars().count());
    println!(
        "{} {}{} {}",
        "│".dimmed(),
        session_info.dimmed(),
        " ".repeat(session_padding),
        "│".dimmed()
    );

    // Workspace directory line
    if let Some(workspace) = chat_session.workspace_dir() {
        let workspace_display = workspace.display().to_string();
        let workspace_line = format!("workspace: {}", workspace_display);
        let truncated_line = if workspace_line.chars().count() > inner_width {
            format!(
                "workspace: ...{}",
                &workspace_display[workspace_display.len().saturating_sub(inner_width - 16)..]
            )
        } else {
            workspace_line
        };
        let ws_padding = inner_width.saturating_sub(truncated_line.chars().count());
        println!(
            "{} {}{} {}",
            "│".dimmed(),
            truncated_line.dimmed(),
            " ".repeat(ws_padding),
            "│".dimmed()
        );
    }

    // System prompt if provided
    if let Some(sys) = system {
        let sys_display = if sys.len() > inner_width.saturating_sub(10) {
            format!("{}...", &sys[..inner_width.saturating_sub(13)])
        } else {
            sys.to_string()
        };
        let sys_line = format!("system: {}", sys_display);
        let sys_padding = inner_width.saturating_sub(sys_line.chars().count());
        println!(
            "{} {}{} {}",
            "│".dimmed(),
            sys_line.dimmed(),
            " ".repeat(sys_padding),
            "│".dimmed()
        );
    }

    println!("{}", format!("├{}┤", "─".repeat(inner_width + 2)).dimmed());

    // Help hints
    let hints = "/help commands · /tools list · /summarize history · /memory manage · Ctrl+C quit";
    let hints_padding = inner_width.saturating_sub(hints.len());
    println!(
        "{} {}{} {}",
        "│".dimmed(),
        hints.dimmed(),
        " ".repeat(hints_padding),
        "│".dimmed()
    );

    println!("{}", format!("╰{}╯", "─".repeat(inner_width + 2)).dimmed());
    println!();

    // Store system prompt for use in LLM calls
    let system_prompt = system.map(String::from);

    // ─────────────────────────────────────────────────────────────────────────
    // HISTORY: Show previous messages if resuming session
    // ─────────────────────────────────────────────────────────────────────────
    if chat_session.message_count() != 0 {
        let history_header = format!("┌─ History ({} messages) ", chat_session.message_count());
        let history_padding = inner_width.saturating_sub(history_header.len()) + 3;
        println!(
            "{}{}",
            history_header.dimmed(),
            "─".repeat(history_padding).dimmed()
        );

        for msg in &chat_session.state.messages {
            let prefix = if msg.role == "user" {
                "│ >"
            } else {
                "│ ◆"
            };
            let color = if msg.role == "user" { "green" } else { "blue" };

            // Word-wrap long messages
            let max_line_width = inner_width.saturating_sub(4);
            let lines: Vec<&str> = msg.content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if line.len() > max_line_width {
                    // Split long lines
                    let wrapped = textwrap::wrap(line, max_line_width);
                    for (j, part) in wrapped.iter().enumerate() {
                        if i == 0 && j == 0 {
                            if color == "green" {
                                println!("{} {}", prefix.green(), part);
                            } else {
                                println!("{} {}", prefix.blue(), part);
                            }
                        } else {
                            println!("{}   {}", "│".dimmed(), part);
                        }
                    }
                } else if i == 0 {
                    if color == "green" {
                        println!("{} {}", prefix.green(), line);
                    } else {
                        println!("{} {}", prefix.blue(), line);
                    }
                } else {
                    println!("{}   {}", "│".dimmed(), line);
                }
            }
        }

        let history_footer = format!("└{}", "─".repeat(inner_width + 2));
        println!("{}", history_footer.dimmed());
        println!();
    }

    // Set up readline
    let mut rl =
        DefaultEditor::new().map_err(|e| format!("Failed to initialize readline: {}", e))?;

    // Load history
    let history_path = get_history_path();
    if let Some(parent) = history_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = rl.load_history(&history_path);

    // Create tokio runtime for async LLM calls
    let rt = tokio::runtime::Runtime::new()?;

    // Main chat loop
    loop {
        // Minimal prompt (claude-code style: just ">")
        let prompt = if voice {
            format!("{} ", "🎤 >".green())
        } else {
            format!("{} ", ">".green().bold())
        };

        match rl.readline(&prompt) {
            Ok(line) => {
                let input = line.trim();

                // In voice mode, empty input triggers voice recording
                let (input, input_source) = if input.is_empty() && voice {
                    match record_voice_input(&rt) {
                        Ok(text) => (text, MessageSource::Voice),
                        Err(e) => {
                            eprintln!("{}: {}", "Voice error".red(), e);
                            continue;
                        }
                    }
                } else if input.is_empty() {
                    continue;
                } else {
                    (input.to_string(), MessageSource::Text)
                };

                // Add to history
                let _ = rl.add_history_entry(&input);

                // Handle /exec by stripping the prefix so it goes to the LLM
                let input = if let Some(rest) = input.strip_prefix("/exec ") {
                    rest.to_string()
                } else {
                    input
                };

                // Handle commands
                if input.starts_with('/') {
                    let mut parts = input.split_whitespace();
                    let cmd = parts.next().unwrap_or("");
                    let args: Vec<&str> = parts.collect();
                    match cmd.to_ascii_lowercase().as_str() {
                        "/exit" | "/quit" | "/q" => {
                            save_cli_session(&chat_session)?;
                            println!();
                            println!(
                                "{} {} {}",
                                "✓".green(),
                                "Session saved.".dimmed(),
                                "Goodbye!".cyan()
                            );
                            println!();
                            break;
                        }
                        "/voice" => {
                            // Explicit voice command
                            match record_voice_input(&rt) {
                                Ok(text) => {
                                    if !text.is_empty() {
                                        println!("{} {}", "Transcribed:".cyan(), text);
                                        // Continue to process as regular input
                                        // (fall through to LLM call below)
                                    }
                                }
                                Err(e) => {
                                    eprintln!("{}: {}", "Voice error".red(), e);
                                }
                            }
                            continue;
                        }
                        "/help" => {
                            println!();
                            println!(
                                "{}",
                                "╭─ Commands ─────────────────────────────────────────────────╮"
                                    .dimmed()
                            );
                            println!(
                                "{}  {}   {}",
                                "│".dimmed(),
                                "/q, /quit, /exit".green(),
                                "Exit and save the current session".dimmed()
                            );
                            println!(
                                "{}  {}              {}",
                                "│".dimmed(),
                                "/help".green(),
                                "Show this help message".dimmed()
                            );
                            println!(
                                "{}  {}             {}",
                                "│".dimmed(),
                                "/clear".green(),
                                "Clear the terminal screen".dimmed()
                            );
                            println!(
                                "{}  {}      {}",
                                "│".dimmed(),
                                "/tools [name]".green(),
                                "List tools, show detail, enable/disable".dimmed()
                            );
                            println!(
                                "{}  {}        {}",
                                "│".dimmed(),
                                "/summarize".green(),
                                "Summarize conversation history".dimmed()
                            );
                            println!(
                                "{}  {}  {}",
                                "│".dimmed(),
                                "/memory [list|save|clear]".green(),
                                "Manage memory bank".dimmed()
                            );
                            println!(
                                "{}  {}              {}",
                                "│".dimmed(),
                                "/save".green(),
                                "Save session to disk immediately".dimmed()
                            );
                            println!(
                                "{}  {}           {}",
                                "│".dimmed(),
                                "/history".green(),
                                "Show message count in session".dimmed()
                            );
                            println!(
                                "{}  {}               {}",
                                "│".dimmed(),
                                "/new".green(),
                                "Start a fresh chat session".dimmed()
                            );
                            if voice {
                                println!(
                                    "{}  {}             {}",
                                    "│".dimmed(),
                                    "/voice".green(),
                                    "Record voice input via microphone".dimmed()
                                );
                            }
                            println!(
                                "{}",
                                "├─ System ───────────────────────────────────────────────────┤"
                                    .dimmed()
                            );
                            println!(
                                "{}  {} {}",
                                "│".dimmed(),
                                "/mcp [status|list|tools|get|add|remove|enable|disable|connect|disconnect]".green(),
                                "".dimmed()
                            );
                            println!(
                                "{}  {}   {}",
                                "│".dimmed(),
                                "".dimmed(),
                                "MCP server management".dimmed()
                            );
                            println!(
                                "{}  {}  {}",
                                "│".dimmed(),
                                "/config [list|get|set|path|reset]".green(),
                                "Configuration management".dimmed()
                            );
                            println!(
                                "{}  {} {}",
                                "│".dimmed(),
                                "/session [info|list|delete|export]".green(),
                                "Session management".dimmed()
                            );
                            println!(
                                "{}  {}  {}",
                                "│".dimmed(),
                                "/context [status|analyze|categories|clear]".green(),
                                "Context system".dimmed()
                            );
                            println!(
                                "{}  {}  {}",
                                "│".dimmed(),
                                "/model [provider:model]".green(),
                                "View or switch LLM model".dimmed()
                            );
                            println!(
                                "{}  {}   {}",
                                "│".dimmed(),
                                "/exec <prompt>".green(),
                                "Execute prompt (bypass slash-cmd detection)".dimmed()
                            );
                            println!(
                                "{}  {} {}",
                                "│".dimmed(),
                                "/a2a [status|profiles|agents]".green(),
                                "A2A protocol status".dimmed()
                            );
                            println!(
                                "{}  {}    {}",
                                "│".dimmed(),
                                "/knowledge [list|search]".green(),
                                "Browse knowledge base".dimmed()
                            );
                            println!(
                                "{}  {}    {}",
                                "│".dimmed(),
                                "/agent [status|config]".green(),
                                "Agent info and LLM config".dimmed()
                            );
                            println!(
                                "{}  {}        {}",
                                "│".dimmed(),
                                "/device [list]".green(),
                                "Audio input devices".dimmed()
                            );
                            println!(
                                "{}  {}            {}",
                                "│".dimmed(),
                                "/health".green(),
                                "System health diagnostics".dimmed()
                            );
                            println!(
                                "{}  {}  {}",
                                "│".dimmed(),
                                "/privacy [status|policy]".green(),
                                "Privacy & GDPR tools".dimmed()
                            );
                            println!(
                                "{}  {}            {}",
                                "│".dimmed(),
                                "/listen".green(),
                                "Voice input status".dimmed()
                            );
                            println!(
                                "{}",
                                "├─ Keyboard ─────────────────────────────────────────────────┤"
                                    .dimmed()
                            );
                            println!(
                                "{}  {}            {}",
                                "│".dimmed(),
                                "Ctrl+C".yellow(),
                                "Quit immediately (session saved)".dimmed()
                            );
                            println!(
                                "{}  {}            {}",
                                "│".dimmed(),
                                "Ctrl+D".yellow(),
                                "End of input (same as /quit)".dimmed()
                            );
                            println!(
                                "{}  {}             {}",
                                "│".dimmed(),
                                "Enter".yellow(),
                                "Send message to assistant".dimmed()
                            );
                            println!(
                                "{}",
                                "╰─────────────────────────────────────────────────────────────╯"
                                    .dimmed()
                            );
                            println!();
                            continue;
                        }
                        "/tools" => {
                            println!();
                            basic_mode_tools_command(&args, &mut chat_session);
                            println!();
                            continue;
                        }
                        "/summarize" => {
                            println!();
                            // Get conversation history
                            let history: Vec<String> = chat_session
                                .state
                                .messages
                                .iter()
                                .map(|msg| msg.content.clone())
                                .collect();

                            if history.is_empty() {
                                println!(
                                    "{} {}",
                                    "◆".yellow().bold(),
                                    "No conversation history to summarize.".yellow()
                                );
                            } else {
                                // Use context manager to summarize
                                use gestura_core::context::ContextManager;
                                let context_manager = ContextManager::new();
                                let summary = context_manager.summarize_history(&history);

                                println!(
                                    "{} {}",
                                    "◆".blue().bold(),
                                    "Conversation Summary:".blue()
                                );
                                println!();
                                println!("{}", summary);
                                println!();
                                println!(
                                    "{}",
                                    format!("Summarized {} messages", history.len()).dimmed()
                                );

                                // Add summary to session
                                chat_session.add_assistant_message(
                                    &format!(
                                        "## Conversation Summary\n\n{}\n\n---\n\n*Summarized {} messages*",
                                        summary,
                                        history.len()
                                    ),
                                    Some("Summarizing conversation history (no LLM call)...".to_string()),
                                );
                            }
                            println!();
                            continue;
                        }
                        cmd if cmd.starts_with("/memory") => {
                            println!();
                            // Parse subcommand
                            let parts: Vec<&str> = cmd.split_whitespace().collect();
                            let subcommand = parts.get(1).unwrap_or(&"list");

                            match *subcommand {
                                "list" => {
                                    // List all memory bank entries
                                    if let Some(workspace_dir) = chat_session.workspace_dir() {
                                        let result = tokio::runtime::Runtime::new()
                                            .unwrap()
                                            .block_on(gestura_core::memory_bank::list_memory_bank(
                                                workspace_dir,
                                            ));
                                        match result {
                                            Ok(entries) if !entries.is_empty() => {
                                                println!(
                                                    "{} {}",
                                                    "◆".blue().bold(),
                                                    format!(
                                                        "Memory Bank Entries ({} total):",
                                                        entries.len()
                                                    )
                                                    .blue()
                                                );
                                                println!();
                                                for entry in entries {
                                                    println!(
                                                        "  {} {} (Session: {})",
                                                        "•".dimmed(),
                                                        entry
                                                            .timestamp
                                                            .format("%Y-%m-%d %H:%M UTC"),
                                                        entry.session_id.dimmed()
                                                    );
                                                    println!("    {}", entry.summary);
                                                    if let Some(path) = entry.file_path {
                                                        println!(
                                                            "    File: {}",
                                                            path.display().to_string().dimmed()
                                                        );
                                                    }
                                                    println!();
                                                }
                                            }
                                            Ok(_) => {
                                                println!(
                                                    "{} {}",
                                                    "◆".yellow().bold(),
                                                    "No memory bank entries found.".yellow()
                                                );
                                            }
                                            Err(e) => {
                                                println!(
                                                    "{} {}",
                                                    "✗".red().bold(),
                                                    format!("Error listing memory bank: {}", e)
                                                        .red()
                                                );
                                            }
                                        }
                                    } else {
                                        println!(
                                            "{} {}",
                                            "✗".red().bold(),
                                            "No workspace directory configured. Cannot access memory bank.".red()
                                        );
                                    }
                                }
                                "save" => {
                                    // Save current context to memory bank
                                    if let Some(workspace_dir) = chat_session.workspace_dir() {
                                        let history: Vec<String> = chat_session
                                            .state
                                            .messages
                                            .iter()
                                            .map(|msg| msg.content.clone())
                                            .collect();

                                        if history.is_empty() {
                                            println!(
                                                "{} {}",
                                                "◆".yellow().bold(),
                                                "No conversation history to save.".yellow()
                                            );
                                        } else {
                                            use gestura_core::context::ContextManager;
                                            let context_manager = ContextManager::new();
                                            let summary =
                                                context_manager.summarize_history(&history);
                                            let content = history.join("\n\n");

                                            let entry =
                                                gestura_core::memory_bank::MemoryBankEntry {
                                                    timestamp: chrono::Utc::now(),
                                                    session_id: chat_session.id.clone(),
                                                    summary: summary.clone(),
                                                    content,
                                                    file_path: None,
                                                };

                                            let result =
                                                tokio::runtime::Runtime::new().unwrap().block_on(
                                                    gestura_core::memory_bank::save_to_memory_bank(
                                                        workspace_dir,
                                                        &entry,
                                                    ),
                                                );
                                            match result {
                                                Ok(path) => {
                                                    println!(
                                                        "{} {}",
                                                        "✓".green().bold(),
                                                        format!(
                                                            "Saved {} messages to memory bank",
                                                            history.len()
                                                        )
                                                        .green()
                                                    );
                                                    println!("  File: {}", path.display());
                                                    println!("  Summary: {}", summary.dimmed());
                                                }
                                                Err(e) => {
                                                    println!(
                                                        "{} {}",
                                                        "✗".red().bold(),
                                                        format!(
                                                            "Error saving to memory bank: {}",
                                                            e
                                                        )
                                                        .red()
                                                    );
                                                }
                                            }
                                        }
                                    } else {
                                        println!(
                                            "{} {}",
                                            "✗".red().bold(),
                                            "No workspace directory configured. Cannot save to memory bank.".red()
                                        );
                                    }
                                }
                                "clear" => {
                                    // Clear all memory bank entries
                                    if let Some(workspace_dir) = chat_session.workspace_dir() {
                                        let memory_dir =
                                            workspace_dir.join(".gestura").join("memory");
                                        match std::fs::remove_dir_all(&memory_dir) {
                                            Ok(_) => {
                                                // Recreate the directory
                                                let _ = std::fs::create_dir_all(&memory_dir);
                                                println!(
                                                    "{} {}",
                                                    "✓".green().bold(),
                                                    "Cleared all memory bank entries.".green()
                                                );
                                            }
                                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                                println!(
                                                    "{} {}",
                                                    "◆".yellow().bold(),
                                                    "Memory bank is already empty.".yellow()
                                                );
                                            }
                                            Err(e) => {
                                                println!(
                                                    "{} {}",
                                                    "✗".red().bold(),
                                                    format!("Error clearing memory bank: {}", e)
                                                        .red()
                                                );
                                            }
                                        }
                                    } else {
                                        println!(
                                            "{} {}",
                                            "✗".red().bold(),
                                            "No workspace directory configured. Cannot clear memory bank.".red()
                                        );
                                    }
                                }
                                _ => {
                                    println!(
                                        "{} {}",
                                        "✗".red().bold(),
                                        format!("Unknown /memory subcommand: '{}'", subcommand)
                                            .red()
                                    );
                                    println!();
                                    println!("Usage:");
                                    println!(
                                        "  {} - Show all memory bank entries",
                                        "/memory list".green()
                                    );
                                    println!(
                                        "  {} - Save current conversation to memory bank",
                                        "/memory save".green()
                                    );
                                    println!(
                                        "  {} - Delete all memory bank entries",
                                        "/memory clear".green()
                                    );
                                }
                            }
                            println!();
                            continue;
                        }
                        "/clear" => {
                            print!("\x1B[2J\x1B[1;1H");
                            continue;
                        }
                        "/save" => {
                            save_cli_session(&chat_session)?;
                            println!("{} Session saved", "✓".green());
                            continue;
                        }
                        "/history" => {
                            let user_msgs = chat_session
                                .state
                                .messages
                                .iter()
                                .filter(|m| m.role == "user")
                                .count();
                            let asst_msgs = chat_session
                                .state
                                .messages
                                .iter()
                                .filter(|m| m.role == "assistant")
                                .count();
                            println!();
                            println!(
                                "{}",
                                "╭─ Session Statistics ─────────────────────────────────────────╮"
                                    .dimmed()
                            );
                            println!(
                                "{}  {} {}",
                                "│".dimmed(),
                                "Session ID:".dimmed(),
                                chat_session.id
                            );
                            println!(
                                "{}  {} {}",
                                "│".dimmed(),
                                "Total Messages:".dimmed(),
                                chat_session.message_count()
                            );
                            println!(
                                "{}  {} {}",
                                "│".dimmed(),
                                "Your Messages:".dimmed(),
                                user_msgs
                            );
                            println!(
                                "{}  {} {}",
                                "│".dimmed(),
                                "AI Responses:".dimmed(),
                                asst_msgs
                            );
                            if let Some(workspace) = chat_session.workspace_dir() {
                                println!(
                                    "{}  {} {}",
                                    "│".dimmed(),
                                    "Workspace:".dimmed(),
                                    workspace.display()
                                );
                            }
                            println!(
                                "{}",
                                "╰───────────────────────────────────────────────────────────────╯"
                                    .dimmed()
                            );
                            println!();
                            continue;
                        }
                        "/new" => {
                            save_cli_session(&chat_session)?;
                            chat_session = new_cli_session(model.map(String::from))?;
                            println!();
                            println!(
                                "{} {} {}",
                                "✓".green(),
                                "New session started:".dimmed(),
                                chat_session.id
                            );
                            println!();
                            continue;
                        }
                        "/mcp" => {
                            println!();
                            basic_mode_mcp_command(&args);
                            println!();
                            continue;
                        }
                        "/a2a" => {
                            println!();
                            basic_mode_a2a_command(&args);
                            println!();
                            continue;
                        }
                        "/knowledge" => {
                            println!();
                            basic_mode_knowledge_command(&args);
                            println!();
                            continue;
                        }
                        "/agent" => {
                            println!();
                            basic_mode_agent_command(&args, &config, &chat_session);
                            println!();
                            continue;
                        }
                        "/device" => {
                            println!();
                            basic_mode_device_command();
                            println!();
                            continue;
                        }
                        "/health" => {
                            println!();
                            basic_mode_health_command(&config);
                            println!();
                            continue;
                        }
                        "/privacy" => {
                            println!();
                            basic_mode_privacy_command(&args);
                            println!();
                            continue;
                        }
                        "/listen" => {
                            println!();
                            basic_mode_listen_command();
                            println!();
                            continue;
                        }
                        "/config" => {
                            println!();
                            basic_mode_config_command(&args);
                            println!();
                            continue;
                        }
                        "/session" | "/sessions" => {
                            println!();
                            basic_mode_session_command(&args, &chat_session);
                            println!();
                            continue;
                        }
                        "/context" => {
                            println!();
                            basic_mode_context_command(&args);
                            println!();
                            continue;
                        }
                        "/model" => {
                            println!();
                            if let Some(new_model) =
                                basic_mode_model_command(&args, &config, &chat_session)
                            {
                                chat_session.model = Some(new_model);
                            }
                            println!();
                            continue;
                        }
                        _ => {
                            println!();
                            println!("{} {} {}", "✗".red(), "Unknown command:".dimmed(), cmd);
                            println!(
                                "  {} /help {}",
                                "Tip:".dimmed(),
                                "for available commands".dimmed()
                            );
                            println!();
                            continue;
                        }
                    }
                }

                // Add user message to session
                chat_session.add_user_message(&input, input_source);

                // Handle explicit /tools command only (not natural language questions)
                if input.trim().starts_with("/tools") {
                    let parts: Vec<&str> = input.split_whitespace().collect();
                    println!();
                    basic_mode_tools_command(&parts[1..], &mut chat_session);
                    println!();
                    continue;
                }

                // Handle /summarize command - summarize conversation history without calling LLM
                if input.trim().starts_with("/summarize") {
                    println!();
                    // Get conversation history
                    let history: Vec<String> = chat_session
                        .state
                        .messages
                        .iter()
                        .map(|msg| msg.content.clone())
                        .collect();

                    if history.is_empty() {
                        println!(
                            "{} {}",
                            "◆".yellow().bold(),
                            "No conversation history to summarize.".yellow()
                        );
                    } else {
                        // Use context manager to summarize
                        use gestura_core::context::ContextManager;
                        let context_manager = ContextManager::new();
                        let summary = context_manager.summarize_history(&history);

                        println!("{} {}", "◆".blue().bold(), "Conversation Summary:".blue());
                        println!();
                        println!("{}", summary);
                        println!();
                        println!(
                            "{}",
                            format!("Summarized {} messages", history.len()).dimmed()
                        );

                        // Add summary to session
                        chat_session.add_assistant_message(
                            &format!(
                                "## Conversation Summary\n\n{}\n\n---\n\n*Summarized {} messages*",
                                summary,
                                history.len()
                            ),
                            Some("Summarizing conversation history (no LLM call)...".to_string()),
                        );
                    }
                    println!();
                    continue;
                }

                // Handle /memory command - manage memory bank without calling LLM
                if input.trim().starts_with("/memory") {
                    println!();
                    // Parse subcommand
                    let parts: Vec<&str> = input.split_whitespace().collect();
                    let subcommand = parts.get(1).unwrap_or(&"list");

                    match *subcommand {
                        "list" => {
                            // List all memory bank entries
                            if let Some(workspace_dir) = chat_session.workspace_dir() {
                                let result = tokio::runtime::Runtime::new().unwrap().block_on(
                                    gestura_core::memory_bank::list_memory_bank(workspace_dir),
                                );
                                match result {
                                    Ok(entries) if !entries.is_empty() => {
                                        println!(
                                            "{} {}",
                                            "◆".blue().bold(),
                                            format!(
                                                "Memory Bank Entries ({} total):",
                                                entries.len()
                                            )
                                            .blue()
                                        );
                                        println!();
                                        for entry in entries {
                                            println!(
                                                "  {} {} (Session: {})",
                                                "•".dimmed(),
                                                entry.timestamp.format("%Y-%m-%d %H:%M UTC"),
                                                entry.session_id.dimmed()
                                            );
                                            println!("    {}", entry.summary);
                                            if let Some(path) = entry.file_path {
                                                println!(
                                                    "    File: {}",
                                                    path.display().to_string().dimmed()
                                                );
                                            }
                                            println!();
                                        }
                                    }
                                    Ok(_) => {
                                        println!(
                                            "{} {}",
                                            "◆".yellow().bold(),
                                            "No memory bank entries found.".yellow()
                                        );
                                    }
                                    Err(e) => {
                                        println!(
                                            "{} {}",
                                            "✗".red().bold(),
                                            format!("Error listing memory bank: {}", e).red()
                                        );
                                    }
                                }
                            } else {
                                println!(
                                    "{} {}",
                                    "✗".red().bold(),
                                    "No workspace directory configured. Cannot access memory bank."
                                        .red()
                                );
                            }
                        }
                        "save" => {
                            // Save current context to memory bank
                            if let Some(workspace_dir) = chat_session.workspace_dir() {
                                let history: Vec<String> = chat_session
                                    .state
                                    .messages
                                    .iter()
                                    .map(|msg| msg.content.clone())
                                    .collect();

                                if history.is_empty() {
                                    println!(
                                        "{} {}",
                                        "◆".yellow().bold(),
                                        "No conversation history to save.".yellow()
                                    );
                                } else {
                                    use gestura_core::context::ContextManager;
                                    let context_manager = ContextManager::new();
                                    let summary = context_manager.summarize_history(&history);
                                    let content = history.join("\n\n");

                                    let entry = gestura_core::memory_bank::MemoryBankEntry {
                                        timestamp: chrono::Utc::now(),
                                        session_id: chat_session.id.clone(),
                                        summary: summary.clone(),
                                        content,
                                        file_path: None,
                                    };

                                    let result = tokio::runtime::Runtime::new().unwrap().block_on(
                                        gestura_core::memory_bank::save_to_memory_bank(
                                            workspace_dir,
                                            &entry,
                                        ),
                                    );
                                    match result {
                                        Ok(path) => {
                                            println!(
                                                "{} {}",
                                                "✓".green().bold(),
                                                format!(
                                                    "Saved {} messages to memory bank",
                                                    history.len()
                                                )
                                                .green()
                                            );
                                            println!("  File: {}", path.display());
                                            println!("  Summary: {}", summary.dimmed());
                                        }
                                        Err(e) => {
                                            println!(
                                                "{} {}",
                                                "✗".red().bold(),
                                                format!("Error saving to memory bank: {}", e).red()
                                            );
                                        }
                                    }
                                }
                            } else {
                                println!(
                                    "{} {}",
                                    "✗".red().bold(),
                                    "No workspace directory configured. Cannot save to memory bank.".red()
                                );
                            }
                        }
                        "clear" => {
                            // Clear all memory bank entries
                            if let Some(workspace_dir) = chat_session.workspace_dir() {
                                let memory_dir = workspace_dir.join(".gestura").join("memory");
                                match std::fs::remove_dir_all(&memory_dir) {
                                    Ok(_) => {
                                        // Recreate the directory
                                        let _ = std::fs::create_dir_all(&memory_dir);
                                        println!(
                                            "{} {}",
                                            "✓".green().bold(),
                                            "Cleared all memory bank entries.".green()
                                        );
                                    }
                                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                        println!(
                                            "{} {}",
                                            "◆".yellow().bold(),
                                            "Memory bank is already empty.".yellow()
                                        );
                                    }
                                    Err(e) => {
                                        println!(
                                            "{} {}",
                                            "✗".red().bold(),
                                            format!("Error clearing memory bank: {}", e).red()
                                        );
                                    }
                                }
                            } else {
                                println!(
                                    "{} {}",
                                    "✗".red().bold(),
                                    "No workspace directory configured. Cannot clear memory bank."
                                        .red()
                                );
                            }
                        }
                        _ => {
                            println!(
                                "{} {}",
                                "✗".red().bold(),
                                format!("Unknown /memory subcommand: '{}'", subcommand).red()
                            );
                            println!();
                            println!("Usage:");
                            println!(
                                "  {} - Show all memory bank entries",
                                "/memory list".green()
                            );
                            println!(
                                "  {} - Save current conversation to memory bank",
                                "/memory save".green()
                            );
                            println!(
                                "  {} - Delete all memory bank entries",
                                "/memory clear".green()
                            );
                        }
                    }
                    println!();
                    continue;
                }

                // Build conversation history for the AgentPipeline
                let history: Vec<gestura_core::Message> =
                    chat_session.to_pipeline_messages_limited(10);

                // ─────────────────────────────────────────────────────────────
                // AI RESPONSE: Show thinking indicator then response
                // ─────────────────────────────────────────────────────────────
                // Build the agent request with workspace sandboxing
                let mut request = AgentRequest::new(&input)
                    .with_streaming(true)
                    .with_source(RequestSource::CliBasic)
                    .with_history(history);

                // Set workspace directory for sandboxed operations
                if let Some(workspace) = chat_session.workspace_dir() {
                    request = request.with_workspace(workspace.clone());
                }

                // Add system prompt if available
                if let Some(ref sys) = system_prompt {
                    request = request.with_system_prompt(sys.clone());
                }

                // Ensure the agent is aware of the effective environment for this session.
                // Note: this is informational metadata used by the system prompt.
                let provider_name = config.llm.primary.clone();
                let model_name = chat_session
                    .model
                    .clone()
                    .or_else(|| model_for_provider(&config, &provider_name))
                    .unwrap_or_default();
                let (permission_level, allowed_tools) = derive_request_policy(&chat_session);
                request = request
                    .with_session_llm_config(provider_name, model_name)
                    .with_permission_level(permission_level);
                if !allowed_tools.is_empty() {
                    request = request.with_allowed_tools(allowed_tools);
                }

                // Stream response chunks as they arrive (CLI basic mode should feel interactive).
                println!();
                println!("{}", "◆".blue().bold());
                print!("  ");
                let _ = std::io::stdout().flush();

                // Clone the session id into the async streaming scope so we can resolve
                // tool confirmations against the correct session.
                let session_id_for_tool_confirm = chat_session.id.clone();

                let config_clone = config.clone();
                let response: Result<gestura_core::AgentResponse> = rt.block_on(async move {
                    let (tx, mut rx) = mpsc::channel::<StreamChunk>(100);
                    let cancel_token = CancellationToken::new();
                    let cancel_for_task = cancel_token.clone();

                    let stream_task = tokio::spawn(async move {
                        let pipeline = AgentPipeline::with_provider_optimized_config(config_clone);
                        pipeline
                            .process_streaming(request, tx, cancel_for_task)
                            .await
                    });

                    let mut saw_done = false;
                    while let Some(chunk) = rx.recv().await {
                        match chunk {
                            StreamChunk::Status { message } => {
                                println!();
                                println!("  {} {}", "ℹ".cyan(), message.dimmed());
                                print!("  ");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::Text(t) => {
                                // Maintain indentation across newlines.
                                let rendered = t.replace("\n", "\n  ");
                                print!("{}", rendered);
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::Thinking(_) => {
                                // Thinking is stored in the final AgentResponse; we don't print it by default.
                            }
                            StreamChunk::ToolCallStart { name, .. } => {
                                println!();
                                println!("  {} {}", "→".cyan(), format!("tool: {name}").dimmed());
                                print!("  ");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::ToolCallEnd => {
                                // Tool call specification ended, execution starting
                            }
                            StreamChunk::ToolCallArgs(_) => {}
                            StreamChunk::ToolCallResult {
                                name,
                                success,
                                output,
                                duration_ms,
                            } => {
                                if success {
                                    println!(
                                        "  {} {} ({}ms)",
                                        "✓".green(),
                                        name.dimmed(),
                                        duration_ms
                                    );
                                    // Show output with pretty printing for JSON
                                    if !output.is_empty() {
                                        let formatted_output = format_tool_output(&output);
                                        println!("{}", formatted_output.dimmed());
                                    }
                                } else {
                                    println!(
                                        "  {} {} failed ({}ms):",
                                        "✗".red(),
                                        name,
                                        duration_ms
                                    );
                                    // Show error output with pretty printing
                                    let formatted_output = format_tool_output(&output);
                                    println!("{}", formatted_output.red());
                                }
                                print!("  ");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::RetryAttempt {
                                attempt,
                                max_attempts,
                                delay_ms,
                                error_message,
                            } => {
                                println!();
                                println!(
                                    "  {} Retry {}/{} in {}ms: {}",
                                    "⟳".yellow(),
                                    attempt,
                                    max_attempts,
                                    delay_ms,
                                    error_message.dimmed()
                                );
                                print!("  ");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::ContextCompacted {
                                messages_before,
                                messages_after,
                                tokens_saved,
                                summary,
                            } => {
                                println!();
                                println!(
                                    "  {} Context compacted: {} → {} messages ({} tokens saved)",
                                    "📦".cyan(),
                                    messages_before,
                                    messages_after,
                                    tokens_saved
                                );
                                if !summary.is_empty() {
                                    println!("     {}", summary.dimmed());
                                }
                                print!("  ");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::MemoryBankSaved {
                                file_path,
                                session_id,
                                summary,
                                messages_saved,
                            } => {
                                println!();
                                println!(
                                    "  {} Memory bank saved: {} messages",
                                    "💾".cyan(),
                                    messages_saved
                                );
                                println!("     File: {}", file_path.dimmed());
                                if !summary.is_empty() {
                                    println!("     Summary: {}", summary.dimmed());
                                }
                                println!("     Session: {}", session_id.dimmed());
                                print!("  ");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::Done(_) => {
                                saw_done = true;
                                break;
                            }
                            StreamChunk::ConfigRequest { key, value, .. } => {
                                // In CLI, just show config request as info
                                if let Some(v) = value {
                                    println!("\n📋 Config request: {} → {}", key, v);
                                } else {
                                    println!("\n📋 Config query: {}", key);
                                }
                            }
                            StreamChunk::ToolConfirmationRequired {
                                confirmation_id,
                                tool_name,
                                description,
                                risk_level,
                                category,
                                ..
                            } => {
                                println!();
                                println!(
                                    "  {} Tool '{}' requires confirmation (risk {}/10, {}): {}",
                                    "⚠️".yellow(),
                                    tool_name,
                                    risk_level,
                                    category,
                                    description
                                );

                                let decision = prompt_tool_confirmation_decision();
                                if let Err(err) = TOOL_CONFIRMATIONS.resolve_decision(
                                    &confirmation_id,
                                    Some(session_id_for_tool_confirm.as_str()),
                                    decision,
                                ) {
                                    println!(
                                        "  {} Failed to resolve confirmation: {}",
                                        "✗".red(),
                                        err
                                    );
                                }

                                print!("  ");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::ToolBlocked { tool_name, reason } => {
                                println!();
                                println!(
                                    "  {} Tool '{}' blocked: {}",
                                    "🚫".red(),
                                    tool_name,
                                    reason
                                );
                                print!("  ");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::TokenUsageUpdate {
                                estimated,
                                limit,
                                percentage,
                                status,
                                estimated_cost,
                            } => {
                                // Display token usage inline
                                let status_icon = match status {
                                    gestura_core::streaming::TokenUsageStatus::Green => "🟢",
                                    gestura_core::streaming::TokenUsageStatus::Yellow => "🟡",
                                    gestura_core::streaming::TokenUsageStatus::Red => "🔴",
                                };
                                println!();
                                println!(
                                    "  {} Tokens: {}/{} ({}%) - Est. cost: ${:.4}",
                                    status_icon, estimated, limit, percentage, estimated_cost
                                );
                                print!("  ");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::AgentLoopIteration { iteration } => {
                                if iteration > 0 {
                                    println!();
                                    println!("  {} {}", "◆".cyan(), "Reviewing results…".dimmed());
                                    print!("  ");
                                    let _ = std::io::stdout().flush();
                                }
                            }
                            StreamChunk::ShellOutput { data, .. } => {
                                print!("{data}");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::ShellLifecycle {
                                state,
                                exit_code,
                                command,
                                ..
                            } => {
                                println!();
                                let label = format!("{state:?}");
                                if let Some(code) = exit_code {
                                    println!(
                                        "  {} shell {}: {} (exit {})",
                                        "⚙".dimmed(),
                                        label.dimmed(),
                                        command.dimmed(),
                                        code
                                    );
                                } else {
                                    println!(
                                        "  {} shell {}: {}",
                                        "⚙".dimmed(),
                                        label.dimmed(),
                                        command.dimmed()
                                    );
                                }
                                print!("  ");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::Paused => {
                                println!();
                                println!("  {} {}", "⏸".yellow(), "Session paused".dimmed());
                                break;
                            }
                            StreamChunk::Cancelled => break,
                            StreamChunk::Error(e) => {
                                return Err(std::io::Error::other(e).into());
                            }
                        }
                    }

                    let agent_response = stream_task.await.map_err(|e| {
                        std::io::Error::other(format!("Streaming task failed: {e}"))
                    })??;
                    if !saw_done {
                        // The channel can close without an explicit Done; still return whatever we have.
                    }
                    Ok(agent_response)
                });

                match response {
                    Ok(agent_response) => {
                        println!();

                        // Show token usage if available
                        if let Some(usage) = &agent_response.usage {
                            println!(
                                "  {} tokens: {} in / {} out",
                                "ℹ".dimmed(),
                                usage.input_tokens.to_string().dimmed(),
                                usage.output_tokens.to_string().dimmed()
                            );
                        }

                        chat_session.add_assistant_message(
                            &agent_response.content,
                            agent_response.thinking,
                        );
                    }
                    Err(e) => {
                        println!();
                        println!("{} {} {}", "✗".red(), "Error:".red(), e);
                    }
                }
                println!();

                // Auto-save periodically
                if chat_session.message_count() % 5 == 0 {
                    let _ = save_cli_session(&chat_session);
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!();
                println!();
                println!(
                    "{}",
                    "╭─────────────────────────────────────────────────────────────╮".dimmed()
                );
                println!(
                    "{} {} {}",
                    "│".dimmed(),
                    "Session interrupted. Your conversation has been saved.".yellow(),
                    "│".dimmed()
                );
                println!(
                    "{} {} {}{}",
                    "│".dimmed(),
                    "Session ID:".dimmed(),
                    chat_session.id,
                    " ".repeat(24) + "│"
                );
                println!(
                    "{}",
                    "╰─────────────────────────────────────────────────────────────╯".dimmed()
                );
                save_cli_session(&chat_session)?;
                break;
            }
            Err(ReadlineError::Eof) => {
                println!();
                println!("{} {}", "✓".green(), "Session saved. Goodbye!".dimmed());
                save_cli_session(&chat_session)?;
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    // Save history
    let _ = rl.save_history(&history_path);

    Ok(())
}

/// Record voice input and return transcribed text
fn record_voice_input(rt: &tokio::runtime::Runtime) -> Result<String> {
    // Show recording indicator
    let spinner =
        super::spinner::brand_spinner("Listening... (speak now, silence will stop recording)");

    let result = rt.block_on(async {
        let speech_processor = get_speech_processor();
        let audio_config = AudioCaptureConfig {
            device_name: None, // Use default device
            silence_threshold: 0.01,
            silence_timeout_secs: 1.5,
            max_recording_secs: 30,
            wait_for_speech_timeout_secs: 10,
        };

        // Record audio
        let (duration, audio_path) = speech_processor
            .record_audio_to_file(audio_config.device_name.clone())
            .await
            .map_err(|e| format!("Recording failed: {}", e))?;

        if duration < 0.5 {
            return Err("Recording too short - no audio captured".to_string());
        }

        // Transcribe using speech processor
        match speech_processor.transcribe_audio(&audio_path).await {
            Ok(result) => Ok(result.text),
            Err(e) => {
                // Fallback: return placeholder if transcription not available
                tracing::warn!(
                    "Transcription failed: {}, audio saved to {:?}",
                    e,
                    audio_path
                );
                Err(format!(
                    "Transcription failed: {} (audio saved to {:?})",
                    e, audio_path
                ))
            }
        }
    });

    spinner.finish_and_clear();

    match result {
        Ok(text) => {
            println!("{} {}", "🎤".green(), text.dimmed());
            Ok(text)
        }
        Err(e) => Err(e.into()),
    }
}

/// Format tool output with pretty printing for JSON and smart truncation
fn format_tool_output(output: &str) -> String {
    // Try to parse as JSON and pretty print
    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(output) {
        // Pretty print JSON with indentation
        if let Ok(pretty) = serde_json::to_string_pretty(&json_value) {
            // Truncate if too long, but show more for JSON (1000 chars instead of 100)
            if pretty.len() > 1000 {
                let truncated = &pretty[..1000];
                // Try to truncate at a line boundary
                if let Some(last_newline) = truncated.rfind('\n') {
                    format!(
                        "    {}\n    ... (truncated, {} more chars)",
                        &pretty[..last_newline],
                        pretty.len() - last_newline
                    )
                } else {
                    format!(
                        "    {}...\n    (truncated, {} more chars)",
                        truncated,
                        pretty.len() - 1000
                    )
                }
            } else {
                // Indent each line for better readability
                pretty
                    .lines()
                    .map(|line| format!("    {}", line))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        } else {
            // Fallback to regular truncation if pretty printing fails
            truncate_output(output, 100)
        }
    } else {
        // Not JSON, use regular truncation
        truncate_output(output, 100)
    }
}

/// Truncate output to a maximum length
fn truncate_output(output: &str, max_len: usize) -> String {
    if output.len() > max_len {
        format!(
            "    {}...\n    (truncated, {} more chars)",
            &output[..max_len],
            output.len() - max_len
        )
    } else {
        // Indent for consistency
        output
            .lines()
            .map(|line| format!("    {}", line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Basic mode `/mcp` slash command handler.
fn basic_mode_mcp_command(args: &[&str]) {
    use gestura_core::config::{McpScope, McpServerEntry, McpTransportType};

    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None | Some("status") => {
            let config = AppConfig::load();
            println!("{}", "MCP Server Status".bold().cyan());
            println!("{}", "═".repeat(50));
            let total = config.mcp_servers.len();
            let enabled = config.mcp_servers.iter().filter(|s| s.enabled).count();
            println!(
                "{}: {} configured ({} enabled)",
                "Servers".bold(),
                total,
                enabled
            );
            for server in &config.mcp_servers {
                let status = if server.enabled {
                    "✓".green()
                } else {
                    "○".dimmed()
                };
                let endpoint = match server.transport {
                    McpTransportType::Stdio => {
                        let cmd = server.command.as_deref().unwrap_or("");
                        let cmd_args = server.args.join(" ");
                        format!("{} {}", cmd, cmd_args).trim().to_string()
                    }
                    _ => server.url.clone().unwrap_or_default(),
                };
                println!(
                    "  {} {} [{}] {}",
                    status,
                    server.name.cyan(),
                    format!("{}", server.transport).dimmed(),
                    endpoint.dimmed()
                );
            }
            // Also show connected servers
            let rt = tokio::runtime::Runtime::new().unwrap();
            let registry = gestura_core::get_mcp_client_registry();
            let connected = rt.block_on(registry.connected_servers());
            if !connected.is_empty() {
                println!();
                println!("{}", "Connected:".bold().yellow());
                for name in &connected {
                    println!("  {} {}", "✓".green(), name);
                }
            }
        }
        Some("list") => {
            let config = AppConfig::load();
            if config.mcp_servers.is_empty() {
                println!("{}", "No MCP servers configured.".dimmed());
                println!(
                    "Add one with: {}",
                    "/mcp add <name> <command_or_url>".cyan()
                );
            } else {
                println!("{}", "MCP Servers".bold().cyan());
                println!("{}", "═".repeat(60));
                println!(
                    "{:20} {:8} {:8} {}",
                    "NAME".underline(),
                    "TYPE".underline(),
                    "SCOPE".underline(),
                    "ENDPOINT / COMMAND".underline()
                );
                for srv in &config.mcp_servers {
                    let status = if srv.enabled { "✓" } else { "○" };
                    let endpoint = match srv.transport {
                        McpTransportType::Stdio => {
                            let cmd = srv.command.as_deref().unwrap_or("");
                            let cmd_args = srv.args.join(" ");
                            format!("{} {}", cmd, cmd_args).trim().to_string()
                        }
                        _ => srv.url.clone().unwrap_or_default(),
                    };
                    println!(
                        "{} {:18} {:8} {:8} {}",
                        status,
                        srv.name.cyan(),
                        format!("{}", srv.transport).dimmed(),
                        format!("{}", srv.scope).dimmed(),
                        endpoint.dimmed()
                    );
                }
                println!();
                println!("Total: {} server(s)", config.mcp_servers.len());
            }
        }
        Some("tools") => {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let registry = gestura_core::get_mcp_client_registry();
            let server_filter = args.get(1).map(|s| s.to_string());
            let all = rt.block_on(registry.all_tools());
            let filtered: Vec<_> = if let Some(ref filter) = server_filter {
                all.into_iter().filter(|(name, _)| name == filter).collect()
            } else {
                all
            };
            if filtered.is_empty() {
                println!(
                    "{}",
                    "No MCP tools available. Connect a server first.".dimmed()
                );
            } else {
                println!("{}", "MCP Tools".bold().cyan());
                println!("{}", "═".repeat(50));
                let mut total = 0;
                for (server_name, server_tools) in &filtered {
                    println!(
                        "\n{} ({} tools):",
                        server_name.bold(),
                        server_tools.len()
                    );
                    for tool in server_tools {
                        let desc = tool.description.as_deref().unwrap_or("(no description)");
                        println!("  {} {} — {}", "•".cyan(), tool.name, desc.dimmed());
                    }
                    total += server_tools.len();
                }
                println!();
                println!(
                    "Total: {} tool(s) across {} server(s)",
                    total,
                    filtered.len()
                );
            }
        }
        Some("get") => {
            if let Some(name) = args.get(1) {
                let config = AppConfig::load();
                match config.mcp_servers.iter().find(|t| t.name == *name) {
                    Some(srv) => {
                        println!("{}", srv.name.bold().cyan());
                        println!("{}", "═".repeat(50));
                        println!(
                            "  {} {}",
                            "Transport:".dimmed(),
                            format!("{}", srv.transport).cyan()
                        );
                        println!(
                            "  {} {}",
                            "Enabled:".dimmed(),
                            if srv.enabled {
                                "yes".green()
                            } else {
                                "no".red()
                            }
                        );
                        println!("  {} {}", "Scope:".dimmed(), srv.scope);
                        println!("  {} {}s", "Timeout:".dimmed(), srv.timeout_secs);
                        println!(
                            "  {} {}",
                            "Auto-reconnect:".dimmed(),
                            srv.auto_reconnect
                        );
                        match srv.transport {
                            McpTransportType::Stdio => {
                                println!(
                                    "  {} {}",
                                    "Command:".dimmed(),
                                    srv.command.as_deref().unwrap_or("(none)")
                                );
                                if !srv.args.is_empty() {
                                    println!("  {} {:?}", "Args:".dimmed(), srv.args);
                                }
                                if !srv.env.is_empty() {
                                    println!("  {}", "Env:".dimmed());
                                    for (k, v) in &srv.env {
                                        println!("    {}={}", k, v);
                                    }
                                }
                            }
                            _ => {
                                println!(
                                    "  {} {}",
                                    "URL:".dimmed(),
                                    srv.url.as_deref().unwrap_or("(none)")
                                );
                                if !srv.headers.is_empty() {
                                    println!("  {}", "Headers:".dimmed());
                                    for (k, v) in &srv.headers {
                                        println!("    {}: {}", k, v);
                                    }
                                }
                            }
                        }
                    }
                    None => {
                        println!("{} MCP server '{}' not found", "✗".red(), name);
                    }
                }
            } else {
                println!("{} Usage: /mcp get <name>", "✗".red());
            }
        }
        Some("enable") => {
            if let Some(name) = args.get(1) {
                let mut config = AppConfig::load();
                match config.mcp_servers.iter_mut().find(|t| t.name == *name) {
                    Some(srv) => {
                        srv.enabled = true;
                        if let Err(e) = config.save() {
                            println!("{} Failed to save config: {}", "✗".red(), e);
                        } else {
                            println!("{} Enabled MCP server: {}", "✓".green(), name.cyan());
                        }
                    }
                    None => {
                        println!("{} MCP server '{}' not found", "✗".red(), name);
                    }
                }
            } else {
                println!("{} Usage: /mcp enable <name>", "✗".red());
            }
        }
        Some("disable") => {
            if let Some(name) = args.get(1) {
                let mut config = AppConfig::load();
                match config.mcp_servers.iter_mut().find(|t| t.name == *name) {
                    Some(srv) => {
                        srv.enabled = false;
                        if let Err(e) = config.save() {
                            println!("{} Failed to save config: {}", "✗".red(), e);
                        } else {
                            println!("{} Disabled MCP server: {}", "✓".green(), name.cyan());
                        }
                    }
                    None => {
                        println!("{} MCP server '{}' not found", "✗".red(), name);
                    }
                }
            } else {
                println!("{} Usage: /mcp disable <name>", "✗".red());
            }
        }
        Some("add") => {
            // /mcp add <name> <command_or_url> [--transport stdio|http|sse] [--scope user|project|local]
            if args.len() < 3 {
                println!("{} Usage: /mcp add <name> <command_or_url> [--transport stdio|http|sse] [--scope user|project|local]", "✗".red());
                return;
            }
            let name = args[1].to_string();
            let command_or_url = args[2].to_string();
            let mut transport_str = "stdio".to_string();
            let mut scope_str = "user".to_string();

            // Parse optional flags
            let mut i = 3;
            while i < args.len() {
                match args[i] {
                    "--transport" | "-t" => {
                        if let Some(val) = args.get(i + 1) {
                            transport_str = val.to_string();
                            i += 1;
                        }
                    }
                    "--scope" | "-s" => {
                        if let Some(val) = args.get(i + 1) {
                            scope_str = val.to_string();
                            i += 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }

            let transport_type: McpTransportType = match transport_str.parse() {
                Ok(t) => t,
                Err(e) => {
                    println!("{} {}", "✗".red(), e);
                    return;
                }
            };
            let scope_val: McpScope = match scope_str.parse() {
                Ok(s) => s,
                Err(e) => {
                    println!("{} {}", "✗".red(), e);
                    return;
                }
            };

            let mut config = AppConfig::load();
            if config.mcp_servers.iter().any(|t| t.name == name) {
                println!(
                    "{} MCP server '{}' already exists. Use {} first.",
                    "✗".red(),
                    name,
                    "/mcp remove".cyan()
                );
                return;
            }

            let entry = match transport_type {
                McpTransportType::Stdio => McpServerEntry {
                    name: name.clone(),
                    transport: transport_type,
                    enabled: true,
                    command: Some(command_or_url.clone()),
                    scope: scope_val,
                    ..McpServerEntry::default()
                },
                McpTransportType::Http | McpTransportType::Sse => McpServerEntry {
                    name: name.clone(),
                    transport: transport_type,
                    enabled: true,
                    url: Some(command_or_url.clone()),
                    scope: scope_val,
                    ..McpServerEntry::default()
                },
            };

            config.mcp_servers.push(entry);
            if let Err(e) = config.save() {
                println!("{} Failed to save config: {}", "✗".red(), e);
            } else {
                println!(
                    "{} Added MCP server: {} ({})",
                    "✓".green(),
                    name.cyan(),
                    transport_str
                );
            }
        }
        Some("remove") => {
            if let Some(name) = args.get(1) {
                let mut config = AppConfig::load();
                let original_len = config.mcp_servers.len();
                config.mcp_servers.retain(|t| t.name != *name);
                if config.mcp_servers.len() == original_len {
                    println!("{} MCP server '{}' not found", "✗".red(), name);
                } else if let Err(e) = config.save() {
                    println!("{} Failed to save config: {}", "✗".red(), e);
                } else {
                    println!("{} Removed MCP server: {}", "✓".green(), name.cyan());
                }
            } else {
                println!("{} Usage: /mcp remove <name>", "✗".red());
            }
        }
        Some("connect") => {
            if let Some(name) = args.get(1) {
                let config = AppConfig::load();
                if let Some(srv) = config.mcp_servers.iter().find(|s| s.name == *name) {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let registry = gestura_core::get_mcp_client_registry();
                    match rt.block_on(registry.connect(srv)) {
                        Ok(tools) => {
                            println!(
                                "{} Connected to MCP server: {} ({} tools discovered)",
                                "✓".green(),
                                name.cyan(),
                                tools.len()
                            );
                            for t in &tools {
                                println!("  {} {}", "•".cyan(), t.name);
                                if let Some(desc) = &t.description {
                                    println!("    {}", desc.dimmed());
                                }
                            }
                        }
                        Err(e) => {
                            println!(
                                "{} Failed to connect to '{}': {}",
                                "✗".red(),
                                name,
                                e
                            );
                        }
                    }
                } else {
                    println!("{} MCP server '{}' not found in config", "✗".red(), name);
                }
            } else {
                println!("{} Usage: /mcp connect <name>", "✗".red());
            }
        }
        Some("disconnect") => {
            if let Some(name) = args.get(1) {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let registry = gestura_core::get_mcp_client_registry();
                rt.block_on(registry.disconnect(name));
                println!(
                    "{} Disconnected from MCP server: {}",
                    "✓".green(),
                    name.cyan()
                );
            } else {
                println!("{} Usage: /mcp disconnect <name>", "✗".red());
            }
        }
        Some(other) => {
            println!(
                "{} Unknown /mcp subcommand: '{}'",
                "✗".red(),
                other
            );
            println!();
            println!("Usage:");
            println!("  {} - Show MCP server status", "/mcp".green());
            println!("  {} - List configured servers", "/mcp list".green());
            println!("  {} - List tools from connected servers", "/mcp tools".green());
            println!(
                "  {} - Show server details",
                "/mcp get <name>".green()
            );
            println!(
                "  {} - Add a new MCP server",
                "/mcp add <name> <cmd_or_url>".green()
            );
            println!(
                "  {} - Remove an MCP server",
                "/mcp remove <name>".green()
            );
            println!(
                "  {} - Enable an MCP server",
                "/mcp enable <name>".green()
            );
            println!(
                "  {} - Disable an MCP server",
                "/mcp disable <name>".green()
            );
            println!(
                "  {} - Connect to an MCP server",
                "/mcp connect <name>".green()
            );
            println!(
                "  {} - Disconnect from an MCP server",
                "/mcp disconnect <name>".green()
            );
        }
    }
}

/// Basic mode `/a2a` slash command handler.
fn basic_mode_a2a_command(args: &[&str]) {
    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None | Some("status") => {
            println!("{}", "A2A Protocol Status".bold().cyan());
            println!("{}", "═".repeat(50));
            println!();
            println!("{}: Agent2Agent (A2A)", "Protocol".bold());
            println!("{}: 0.3.0", "Version".bold());
            println!("{}: Linux Foundation", "Governance".bold());
            println!();
            println!("{}", "Features".bold().yellow());
            println!("  {} Agent discovery via Agent Cards", "✓".green());
            println!("  {} Task-based communication", "✓".green());
            println!("  {} JSON-RPC 2.0 protocol", "✓".green());
            println!("  {} Bearer token authentication", "✓".green());
            println!("  {} Profile propagation", "✓".green());
            println!("  {} SSE streaming support", "✓".green());
        }
        Some("profiles") => {
            println!("{}", "No A2A profiles registered yet.".dimmed());
            println!(
                "Use {} to register a new profile.",
                "gestura a2a register".cyan()
            );
        }
        Some("agents") => {
            println!("{}", "No remote agents discovered yet.".dimmed());
            println!(
                "Use {} to discover a remote agent.",
                "gestura a2a discover <url>".cyan()
            );
        }
        Some("discover") => {
            if let Some(url) = args.get(1) {
                println!(
                    "Use {} from CLI for agent discovery.",
                    format!("gestura a2a discover {url}").cyan()
                );
            } else {
                println!("{} Usage: /a2a discover <url>", "✗".red());
            }
        }
        Some(other) => {
            println!(
                "{} Unknown /a2a subcommand: {}. Try: status, profiles, agents, discover",
                "✗".red(),
                other
            );
        }
    }
}

/// Basic mode `/knowledge` slash command handler.
fn basic_mode_knowledge_command(args: &[&str]) {
    use gestura_core::knowledge::{KnowledgeQuery, KnowledgeStore, register_builtin_knowledge};

    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    let store = KnowledgeStore::with_default_dir();
    register_builtin_knowledge(&store);

    match subcommand.as_deref() {
        None | Some("list") => {
            let items = store.list();
            if items.is_empty() {
                println!("{}", "No knowledge items registered.".dimmed());
            } else {
                println!("{}", "Knowledge Base".bold().cyan());
                println!("{}", "═".repeat(50));
                for item in &items {
                    println!(
                        "  {} [{}] {} — {}",
                        "•".cyan(),
                        item.category,
                        item.name.bold(),
                        item.description.dimmed()
                    );
                }
                println!();
                println!("{}", format!("{} items total", items.len()).dimmed());
            }
        }
        Some("search") => {
            let query_text = args.get(1..).unwrap_or_default().join(" ");
            if query_text.is_empty() {
                println!("{} Usage: /knowledge search <query>", "✗".red());
            } else {
                let query = KnowledgeQuery {
                    query: query_text.clone(),
                    limit: Some(10),
                    ..Default::default()
                };
                let matches = store.find(&query);
                if matches.is_empty() {
                    println!("{}", format!("No results for '{query_text}'.").dimmed());
                } else {
                    println!("{}", format!("Search: '{query_text}'").bold().cyan());
                    println!("{}", "═".repeat(50));
                    for m in &matches {
                        println!(
                            "  {} {} (score: {:.2}) — {}",
                            "•".cyan(),
                            m.item.name.bold(),
                            m.score,
                            m.item.description.dimmed()
                        );
                    }
                    println!();
                    println!("{}", format!("{} result(s)", matches.len()).dimmed());
                }
            }
        }
        Some("categories") => {
            let cats = store.categories();
            if cats.is_empty() {
                println!("{}", "No knowledge categories found.".dimmed());
            } else {
                println!("{}", "Knowledge Categories".bold().cyan());
                println!("{}", "═".repeat(50));
                for cat in &cats {
                    let count = store.list_by_category(cat).len();
                    println!("  {} {} ({} items)", "•".cyan(), cat, count);
                }
            }
        }
        Some("status") => {
            println!("{}", "Knowledge Base Status".bold().cyan());
            println!("{}", "═".repeat(50));
            println!("Total items: {}", store.count());
            println!("Categories: {}", store.categories().len());
            println!("Base directory: {}", store.base_dir().display());
        }
        Some(other) => {
            println!(
                "{} Unknown /knowledge subcommand: {}. Try: list, search, categories, status",
                "✗".red(),
                other
            );
        }
    }
}

/// Basic mode `/agent` slash command handler.
fn basic_mode_agent_command(args: &[&str], config: &AppConfig, session: &ChatSession) {
    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None | Some("status") => {
            println!("{}", "Agent Status".bold().cyan());
            println!("{}", "═".repeat(50));
            println!("{}: {}", "Version".bold(), gestura_core::VERSION);
            println!("{}: {}", "Primary LLM".bold(), config.llm.primary);
            println!(
                "{}: {}",
                "Model".bold(),
                session.model.as_deref().unwrap_or("(default)")
            );
            println!("{}: {}", "Session".bold(), &session.id[..8]);
            println!("{}: {}", "Messages".bold(), session.message_count());
            println!();
            println!("{}", "Provider Status:".bold().yellow());
            let has_openai = std::env::var("OPENAI_API_KEY").is_ok()
                || config
                    .llm
                    .openai
                    .as_ref()
                    .is_some_and(|o| !o.api_key.is_empty());
            let has_anthropic = std::env::var("ANTHROPIC_API_KEY").is_ok()
                || config
                    .llm
                    .anthropic
                    .as_ref()
                    .is_some_and(|a| !a.api_key.is_empty());
            println!(
                "  {} OpenAI",
                if has_openai {
                    "✓".green()
                } else {
                    "○".dimmed()
                }
            );
            println!(
                "  {} Anthropic",
                if has_anthropic {
                    "✓".green()
                } else {
                    "○".dimmed()
                }
            );
        }
        Some("config") => {
            println!("{}", "Agent Configuration".bold().cyan());
            println!("{}", "═".repeat(50));
            println!("{}: {}", "Primary".bold(), config.llm.primary);
            if let Some(ref openai) = config.llm.openai {
                println!("OpenAI model: {}", openai.model);
            }
            if let Some(ref anthropic) = config.llm.anthropic {
                println!("Anthropic model: {}", anthropic.model);
            }
            if let Some(ref grok) = config.llm.grok {
                println!("Grok model: {}", grok.model);
            }
            if let Some(ref ollama) = config.llm.ollama {
                println!("Ollama model: {}", ollama.model);
                println!("Ollama base URL: {}", ollama.base_url);
            }
        }
        Some(other) => {
            println!(
                "{} Unknown /agent subcommand: {}. Try: status, config",
                "✗".red(),
                other
            );
        }
    }
}

/// Basic mode `/device` slash command handler.
fn basic_mode_device_command() {
    let devices = gestura_core::list_audio_input_devices();
    let mic_available = gestura_core::is_microphone_available();

    println!("{}", "Audio Devices".bold().cyan());
    println!("{}", "═".repeat(50));
    println!(
        "Microphone available: {}",
        if mic_available {
            "✓ yes".green()
        } else {
            "✗ no".red()
        }
    );
    println!();
    if devices.is_empty() {
        println!("{}", "No audio input devices found.".dimmed());
    } else {
        println!("{} device(s) detected:", devices.len());
        for dev in &devices {
            let marker = if dev.is_default { " (default)" } else { "" };
            println!("  {} {}{}", "•".cyan(), dev.name, marker);
        }
    }
}

/// Basic mode `/health` slash command handler.
fn basic_mode_health_command(config: &AppConfig) {
    println!("{}", "System Health".bold().cyan());
    println!("{}", "═".repeat(50));

    println!("{} Gestura v{}", "✓".green(), gestura_core::VERSION);

    let config_path = AppConfig::default_path();
    let config_ok = config_path.exists();
    println!(
        "{} Config: {}",
        if config_ok {
            "✓".green()
        } else {
            "○".dimmed()
        },
        config_path.display()
    );

    println!();
    println!("{}", "LLM Providers:".bold().yellow());
    let has_openai = std::env::var("OPENAI_API_KEY").is_ok()
        || config
            .llm
            .openai
            .as_ref()
            .is_some_and(|o| !o.api_key.is_empty());
    let has_anthropic = std::env::var("ANTHROPIC_API_KEY").is_ok()
        || config
            .llm
            .anthropic
            .as_ref()
            .is_some_and(|a| !a.api_key.is_empty());
    let has_grok = std::env::var("XAI_API_KEY").is_ok()
        || config
            .llm
            .grok
            .as_ref()
            .is_some_and(|g| !g.api_key.is_empty());
    let has_ollama = config.llm.ollama.is_some();

    println!(
        "  {} OpenAI",
        if has_openai {
            "✓".green()
        } else {
            "○".dimmed()
        }
    );
    println!(
        "  {} Anthropic",
        if has_anthropic {
            "✓".green()
        } else {
            "○".dimmed()
        }
    );
    println!(
        "  {} Grok",
        if has_grok {
            "✓".green()
        } else {
            "○".dimmed()
        }
    );
    println!(
        "  {} Ollama",
        if has_ollama {
            "✓".green()
        } else {
            "○".dimmed()
        }
    );

    println!();
    println!("{}", "Audio:".bold().yellow());
    let mic = gestura_core::is_microphone_available();
    let devices = gestura_core::list_audio_input_devices();
    println!(
        "  {} Microphone",
        if mic { "✓".green() } else { "○".dimmed() }
    );
    println!("  {} device(s) detected", devices.len());

    println!();
    println!("{}", "MCP:".bold().yellow());
    let mcp_count = config.mcp_servers.len();
    let mcp_enabled = config.mcp_servers.iter().filter(|s| s.enabled).count();
    println!(
        "  {} server(s) configured ({} enabled)",
        mcp_count, mcp_enabled
    );
}

/// Basic mode `/privacy` slash command handler.
fn basic_mode_privacy_command(args: &[&str]) {
    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None | Some("status") => {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let report = rt.block_on(async {
                let manager = gestura_core::get_gdpr_manager().await;
                manager.generate_privacy_report().await
            });
            println!("{}", "Privacy Report".bold().cyan());
            println!("{}", "═".repeat(50));
            if let Ok(pretty) = serde_json::to_string_pretty(&report) {
                println!("{pretty}");
            } else {
                println!("{report:?}");
            }
        }
        Some("policy") => {
            println!("{}", "Data Retention Policy".bold().cyan());
            println!("{}", "═".repeat(50));
            println!();
            println!("Gestura respects user privacy and GDPR compliance:");
            println!();
            println!(
                "  {} Voice recordings: Temporary only, deleted after transcription",
                "•".cyan()
            );
            println!(
                "  {} Chat sessions: Stored locally in workspace",
                "•".cyan()
            );
            println!(
                "  {} API keys: Stored in local config file only",
                "•".cyan()
            );
            println!(
                "  {} Memory bank: Stored locally in .gestura/memory/",
                "•".cyan()
            );
            println!(
                "  {} No data is sent to third parties except configured LLM providers",
                "•".cyan()
            );
            println!();
            println!(
                "Use {} for a full GDPR data export.",
                "gestura privacy export".cyan()
            );
            println!(
                "Use {} to exercise right to erasure.",
                "gestura privacy delete".cyan()
            );
        }
        Some("export") => {
            println!(
                "{}: Use {} from CLI for data export.",
                "Note".yellow(),
                "gestura privacy export".cyan()
            );
        }
        Some(other) => {
            println!(
                "{} Unknown /privacy subcommand: {}. Try: status, policy, export",
                "✗".red(),
                other
            );
        }
    }
}

/// Basic mode `/listen` slash command handler.
fn basic_mode_listen_command() {
    let mic_available = gestura_core::is_microphone_available();
    let is_recording = gestura_core::is_speech_recording();

    println!("{}", "Voice Input".bold().cyan());
    println!("{}", "═".repeat(50));
    println!(
        "Microphone: {}",
        if mic_available {
            "✓ available".green()
        } else {
            "✗ not available".red()
        }
    );
    println!(
        "Recording: {}",
        if is_recording {
            "active".yellow()
        } else {
            "idle".dimmed()
        }
    );
    println!();
    println!(
        "For full voice input, use {} from CLI.",
        "gestura listen".cyan()
    );
}

/// Basic mode `/config` slash command handler.
fn basic_mode_config_command(args: &[&str]) {
    use gestura_core::config_env::{is_secret_key, redact_secret};

    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None | Some("list") => {
            let config = AppConfig::load();
            println!("{}", "Configuration".bold().cyan());
            println!("{}", "═".repeat(60));
            let keys = [
                "llm.primary",
                "voice.provider",
                "voice.local_model_path",
                "voice.audio_device",
                "ui.theme_mode",
                "hotkey_listen",
                "nats_url",
                "pipeline.max_history_messages",
                "pipeline.auto_compact_threshold_percent",
                "pipeline.compaction_strategy",
                "pipeline.max_context_tokens",
                "pipeline.log_token_usage",
            ];
            for key in keys {
                let value = basic_mode_get_config_value(&config, key);
                println!(
                    "  {:42} {}",
                    key.dimmed(),
                    value.unwrap_or_else(|| "(unset)".to_string())
                );
            }
            println!();
            println!("{}", "Config file:".dimmed());
            println!("  {}", AppConfig::default_path().display());
        }
        Some("get") => {
            if let Some(key) = args.get(1) {
                let config = AppConfig::load();
                match basic_mode_get_config_value(&config, key) {
                    Some(v) => println!("{} = {}", key.cyan(), v),
                    None => println!("{} Unknown config key: {}", "✗".red(), key),
                }
            } else {
                println!("{} Usage: /config get <key>", "✗".red());
            }
        }
        Some("set") => {
            if args.len() < 3 {
                println!(
                    "{} Usage: /config set <key> <value>",
                    "✗".red()
                );
                return;
            }
            let key = args[1];
            let value = args[2..].join(" ");
            let mut config = AppConfig::load();
            if basic_mode_set_config_value(&mut config, key, &value) {
                if let Err(e) = config.save() {
                    println!("{} Failed to save config: {}", "✗".red(), e);
                } else {
                    let display_value = if is_secret_key(key) {
                        redact_secret(&value)
                    } else {
                        value.to_string()
                    };
                    println!("{} {} = {}", "✓".green(), key.cyan(), display_value);
                }
            } else {
                println!(
                    "{} Unknown or read-only config key: {}",
                    "✗".red(),
                    key
                );
            }
        }
        Some("path") => {
            println!(
                "{}: {}",
                "Config file".dimmed(),
                AppConfig::default_path().display()
            );
        }
        Some("reset") => {
            let config = AppConfig::default();
            if let Err(e) = config.save() {
                println!("{} Failed to save config: {}", "✗".red(), e);
            } else {
                println!("{} Configuration reset to defaults", "✓".green());
            }
        }
        Some(other) => {
            println!(
                "{} Unknown /config subcommand: '{}'. Try: list, get, set, path, reset",
                "✗".red(),
                other
            );
        }
    }
}

/// Get a config value by key (mirrors `commands/config.rs` logic).
fn basic_mode_get_config_value(config: &AppConfig, key: &str) -> Option<String> {
    use gestura_core::config_env::redact_secret;
    match key {
        "llm.primary" => Some(config.llm.primary.clone()),
        "voice.provider" => Some(config.voice.provider.clone()),
        "voice.local_model_path" => Some(config.voice.local_model_path.clone().unwrap_or_default()),
        "voice.audio_device" => Some(config.voice.audio_device.clone().unwrap_or_default()),
        "ui.theme_mode" => Some(config.ui.theme_mode.clone()),
        "hotkey_listen" => Some(config.hotkey_listen.clone()),
        "nats_url" => Some(config.nats_url.clone()),
        "pipeline.max_history_messages" => Some(config.pipeline.max_history_messages.to_string()),
        "pipeline.auto_compact_threshold_percent" => {
            Some(config.pipeline.auto_compact_threshold_percent.to_string())
        }
        "pipeline.compaction_strategy" => {
            Some(format!("{:?}", config.pipeline.compaction_strategy))
        }
        "pipeline.max_context_tokens" => Some(config.pipeline.max_context_tokens.to_string()),
        "pipeline.log_token_usage" => Some(config.pipeline.log_token_usage.to_string()),
        "llm.openai.api_key" => config
            .llm
            .openai
            .as_ref()
            .map(|c| redact_secret(&c.api_key)),
        "llm.anthropic.api_key" => config
            .llm
            .anthropic
            .as_ref()
            .map(|c| redact_secret(&c.api_key)),
        "llm.grok.api_key" => config.llm.grok.as_ref().map(|c| redact_secret(&c.api_key)),
        _ => None,
    }
}

/// Set a config value by key (mirrors `commands/config.rs` logic).
fn basic_mode_set_config_value(config: &mut AppConfig, key: &str, value: &str) -> bool {
    match key {
        "llm.primary" => {
            config.llm.primary = value.to_string();
            true
        }
        "voice.provider" => {
            config.voice.provider = value.to_string();
            true
        }
        "voice.local_model_path" => {
            config.voice.local_model_path = Some(value.to_string());
            true
        }
        "voice.audio_device" => {
            config.voice.audio_device = Some(value.to_string());
            true
        }
        "ui.theme_mode" => {
            config.ui.theme_mode = value.to_string();
            true
        }
        "hotkey_listen" => {
            config.hotkey_listen = value.to_string();
            true
        }
        "nats_url" => {
            config.nats_url = value.to_string();
            true
        }
        "pipeline.max_history_messages" => {
            if let Ok(val) = value.parse::<usize>() {
                config.pipeline.max_history_messages = val;
                true
            } else {
                false
            }
        }
        "pipeline.auto_compact_threshold_percent" => {
            if let Ok(val) = value.parse::<u8>() {
                if val <= 100 {
                    config.pipeline.auto_compact_threshold_percent = val;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        }
        "pipeline.compaction_strategy" => {
            use gestura_core::pipeline::CompactionStrategy;
            config.pipeline.compaction_strategy = CompactionStrategy::parse(value);
            true
        }
        "pipeline.max_context_tokens" => {
            if let Ok(val) = value.parse::<usize>() {
                config.pipeline.max_context_tokens = val;
                true
            } else {
                false
            }
        }
        "pipeline.log_token_usage" => {
            if let Ok(val) = value.parse::<bool>() {
                config.pipeline.log_token_usage = val;
                true
            } else {
                false
            }
        }
        "llm.openai.api_key" => {
            config.llm.openai.get_or_insert(Default::default()).api_key = value.to_string();
            true
        }
        "llm.anthropic.api_key" => {
            config
                .llm
                .anthropic
                .get_or_insert(Default::default())
                .api_key = value.to_string();
            true
        }
        "llm.grok.api_key" => {
            config.llm.grok.get_or_insert(Default::default()).api_key = value.to_string();
            true
        }
        _ => false,
    }
}

/// Basic mode `/session` slash command handler.
fn basic_mode_session_command(args: &[&str], current: &ChatSession) {
    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None | Some("info") => {
            println!("{}", "Current Session".bold().cyan());
            println!("{}", "═".repeat(50));
            println!("  {} {}", "ID:".dimmed(), current.id);
            println!("  {} {}", "Title:".dimmed(), current.title);
            println!("  {} {}", "Created:".dimmed(), current.created_at);
            println!("  {} {}", "Last active:".dimmed(), current.last_active);
            println!("  {} {}", "Messages:".dimmed(), current.message_count());
            println!(
                "  {} {}",
                "Model:".dimmed(),
                current.model.as_deref().unwrap_or("(default)")
            );
            if let Some(ref ws) = current.state.workspace_dir {
                println!("  {} {}", "Workspace:".dimmed(), ws.display());
            }
        }
        Some("list") => {
            let store = session_store();
            match store.list(gestura_core::chat_sessions::SessionFilter::All) {
                Ok(sessions) => {
                    if sessions.is_empty() {
                        println!("{}", "No sessions found.".dimmed());
                    } else {
                        println!("{}", "Chat Sessions".bold().cyan());
                        println!("{}", "═".repeat(60));
                        println!(
                            "{:38} {:6} {}",
                            "SESSION ID".underline(),
                            "MSGS".underline(),
                            "LAST ACTIVE".underline()
                        );
                        for info in sessions.iter().take(20) {
                            let active_str = {
                                let elapsed = chrono::Utc::now()
                                    .signed_duration_since(info.last_active);
                                let secs = elapsed.num_seconds();
                                if secs < 60 {
                                    "just now".to_string()
                                } else if secs < 3600 {
                                    format!("{} min ago", secs / 60)
                                } else if secs < 86400 {
                                    format!("{} hours ago", secs / 3600)
                                } else {
                                    format!("{} days ago", secs / 86400)
                                }
                            };
                            let marker = if info.id == current.id {
                                "▸".green()
                            } else {
                                " ".normal()
                            };
                            println!(
                                "{} {:36} {:6} {}",
                                marker,
                                info.id[..info.id.len().min(36)].cyan(),
                                info.message_count,
                                active_str.dimmed()
                            );
                        }
                        println!();
                        println!("Total: {} session(s)", sessions.len());
                    }
                }
                Err(e) => println!("{} Failed to list sessions: {}", "✗".red(), e),
            }
        }
        Some("delete") => {
            if let Some(id) = args.get(1) {
                if *id == current.id {
                    println!("{} Cannot delete the current active session", "✗".red());
                    return;
                }
                match delete_cli_session(id) {
                    Ok(true) => println!("{} Deleted session: {}", "✓".green(), id.cyan()),
                    Ok(false) => println!("{} Session '{}' not found", "✗".red(), id),
                    Err(e) => println!("{} Failed to delete: {}", "✗".red(), e),
                }
            } else {
                println!("{} Usage: /session delete <id>", "✗".red());
            }
        }
        Some("export") => {
            let current_id_str = current.id.as_str();
            let target_id = args.get(1).unwrap_or(&current_id_str);
            let session_to_export = if *target_id == current.id {
                current.clone()
            } else {
                match load_cli_session(target_id) {
                    Ok(s) => s,
                    Err(e) => {
                        println!("{} Failed to load session: {}", "✗".red(), e);
                        return;
                    }
                }
            };
            let filename = format!("session-{}.json", &session_to_export.id[..8]);
            match serde_json::to_string_pretty(&session_to_export) {
                Ok(json) => match std::fs::write(&filename, &json) {
                    Ok(()) => {
                        println!("{} Exported to: {}", "✓".green(), filename.cyan());
                    }
                    Err(e) => println!("{} Failed to write: {}", "✗".red(), e),
                },
                Err(e) => println!("{} Failed to serialize: {}", "✗".red(), e),
            }
        }
        Some(other) => {
            println!(
                "{} Unknown /session subcommand: '{}'. Try: info, list, delete, export",
                "✗".red(),
                other
            );
        }
    }
}

/// Basic mode `/context` slash command handler.
fn basic_mode_context_command(args: &[&str]) {
    use gestura_core::context::{ContextCategory, ContextManager, RequestAnalyzer};

    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None | Some("status") => {
            let manager = ContextManager::new();
            let stats = manager.cache_stats();
            println!("{}", "Context Manager Status".bold().cyan());
            println!("{}", "═".repeat(50));
            println!();
            println!("{}", "Cache Statistics".yellow());
            println!(
                "  Context Cache: {} / {} entries",
                stats.context_cache.size, stats.context_cache.max_size
            );
            println!(
                "  File Cache:    {} / {} entries",
                stats.file_cache.size, stats.file_cache.max_size
            );
            println!(
                "  History Cache: {} / {} entries",
                stats.history_cache.size, stats.history_cache.max_size
            );
            println!();
            println!("{}", "Features".yellow());
            println!("  {} Request analysis without LLM", "✓".green());
            println!("  {} Category-based tool filtering", "✓".green());
            println!("  {} Smart context caching with TTL", "✓".green());
            println!("  {} Entity extraction (paths, URLs)", "✓".green());
            println!("  {} Follow-up detection", "✓".green());
        }
        Some("analyze") => {
            if args.len() < 2 {
                println!("{} Usage: /context analyze <request text>", "✗".red());
                return;
            }
            let request = args[1..].join(" ");
            let analyzer = RequestAnalyzer::new();
            let analysis = analyzer.analyze(&request);

            println!("{}", "Request Analysis".bold().cyan());
            println!("{}", "═".repeat(60));
            println!("{}: {}", "Request".dimmed(), request);
            println!();

            println!("{}", "Detected Categories".yellow());
            if analysis.categories.is_empty() {
                println!("  {}", "(none)".dimmed());
            } else {
                for cat in &analysis.categories {
                    let icon = context_category_icon(*cat);
                    println!("  {} {:?}", icon, cat);
                }
            }
            println!();

            println!("{}", "Suggested Tools".yellow());
            if analysis.suggested_tools.is_empty() {
                println!("  {}", "(none — general conversation)".dimmed());
            } else {
                for tool in &analysis.suggested_tools {
                    println!("  ● {}", tool);
                }
            }
            println!();

            if !analysis.entities.is_empty() {
                println!("{}", "Extracted Entities".yellow());
                for entity in &analysis.entities {
                    println!(
                        "  → [{:?}]: {}",
                        entity.entity_type, entity.value
                    );
                }
                println!();
            }

            println!("{}", "Analysis Flags".yellow());
            let needs_tools = if analysis.needs_tools {
                "✓".green()
            } else {
                "✗".red()
            };
            let is_followup = if analysis.is_followup {
                "✓".green()
            } else {
                "✗".red()
            };
            println!("  Needs Tools: {}", needs_tools);
            println!("  Is Follow-up: {}", is_followup);
            println!(
                "  Confidence: {}%",
                (analysis.confidence * 100.0) as u32
            );
        }
        Some("categories") => {
            println!("{}", "Context Categories".bold().cyan());
            println!("{}", "═".repeat(50));
            println!();
            let categories = [
                (ContextCategory::FileSystem, "File system operations (read, write, edit)"),
                (ContextCategory::Shell, "Shell command execution"),
                (ContextCategory::Git, "Git version control operations"),
                (ContextCategory::Code, "Code analysis (symbols, references)"),
                (ContextCategory::Web, "Web fetching and search"),
                (ContextCategory::Voice, "Voice and audio processing"),
                (ContextCategory::Config, "Configuration management"),
                (ContextCategory::Session, "Session and history"),
                (ContextCategory::Tools, "Tool introspection"),
                (ContextCategory::Agent, "Agent orchestration"),
                (ContextCategory::Mcp, "MCP protocol operations"),
                (ContextCategory::A2a, "A2A protocol operations"),
                (ContextCategory::General, "General conversation (no tools)"),
            ];
            for (cat, desc) in categories {
                let icon = context_category_icon(cat);
                println!("{} {:?}", icon, cat);
                println!("  {}", desc.dimmed());
            }
        }
        Some("clear") => {
            let manager = ContextManager::new();
            manager.clear_caches();
            println!("{} All context caches cleared", "✓".green());
        }
        Some(other) => {
            println!(
                "{} Unknown /context subcommand: '{}'. Try: status, analyze, categories, clear",
                "✗".red(),
                other
            );
        }
    }
}

/// Icon for a context category (used by `/context` handler).
fn context_category_icon(cat: gestura_core::context::ContextCategory) -> &'static str {
    use gestura_core::context::ContextCategory;
    match cat {
        ContextCategory::FileSystem => "📁",
        ContextCategory::Shell => "🖥️",
        ContextCategory::Git => "🔀",
        ContextCategory::Code => "💻",
        ContextCategory::Web => "🌐",
        ContextCategory::Voice => "🎤",
        ContextCategory::Config => "⚙️",
        ContextCategory::Session => "📜",
        ContextCategory::Tools => "🔧",
        ContextCategory::Agent => "🤖",
        ContextCategory::Mcp => "🔌",
        ContextCategory::A2a => "🔗",
        ContextCategory::General => "💬",
    }
}

/// Basic mode `/model` slash command handler.
///
/// Returns `Some(new_model)` if the user changed the session model so the
/// caller can update `chat_session.model`.
fn basic_mode_model_command(
    args: &[&str],
    config: &AppConfig,
    session: &ChatSession,
) -> Option<String> {
    if args.is_empty() {
        // Show current model info
        let provider = session
            .state
            .llm_config
            .as_ref()
            .and_then(|c| c.provider.as_deref())
            .unwrap_or(&config.llm.primary);
        let model = session
            .state
            .llm_config
            .as_ref()
            .and_then(|c| c.model.as_deref())
            .or_else(|| basic_mode_model_for_provider(config, provider));
        println!("{}", "Active Model".bold().cyan());
        println!("{}", "═".repeat(40));
        println!("  {} {}", "Provider:".dimmed(), provider);
        println!(
            "  {} {}",
            "Model:".dimmed(),
            model.unwrap_or("(provider default)")
        );
        if let Some(legacy) = session.model.as_deref() {
            println!("  {} {}", "Session hint:".dimmed(), legacy);
        }
        println!();
        println!(
            "{} /model <provider:model>  e.g. /model openai:gpt-4o",
            "Set:".dimmed()
        );
        println!(
            "  {} openai, anthropic, grok, gemini, ollama",
            "Providers:".dimmed()
        );
        return None;
    }

    let spec = args.join(" ");
    let spec = spec.trim();

    // Parse spec — supports `provider:model`, provider-only, or model-only
    let (provider, model) = if let Some((p, m)) = spec.split_once(':') {
        let p = p.trim().to_string();
        let m = m.trim().to_string();
        (p, if m.is_empty() { None } else { Some(m) })
    } else {
        match spec.to_ascii_lowercase().as_str() {
            "openai" | "anthropic" | "grok" | "gemini" | "ollama" => {
                let p = spec.to_string();
                let m = basic_mode_model_for_provider(config, &p).map(|s| s.to_string());
                (p, m)
            }
            _ => {
                // Model-only — infer provider
                let inferred = gestura_core::llm_validation::infer_provider_from_model_id(spec)
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| config.llm.primary.clone());
                (inferred, Some(spec.to_string()))
            }
        }
    };

    // Validate compatibility
    if let Some(ref m) = model
        && let Err(err) =
            gestura_core::llm_validation::validate_model_for_provider(&provider, m)
    {
        println!("{} {}", "✗".red(), err);
        return None;
    }

    let display_model = model.as_deref().unwrap_or("(provider default)");
    println!(
        "{} Model set to {} ({})",
        "✓".green(),
        display_model.cyan(),
        provider.dimmed()
    );

    // Return spec for caller to set on session
    let result = if let Some(ref m) = model {
        format!("{}:{}", provider, m)
    } else {
        provider.clone()
    };
    Some(result)
}

/// Resolve the default model for a provider from config.
fn basic_mode_model_for_provider<'a>(config: &'a AppConfig, provider: &str) -> Option<&'a str> {
    match provider {
        "openai" => config.llm.openai.as_ref().map(|c| c.model.as_str()),
        "anthropic" => config.llm.anthropic.as_ref().map(|c| c.model.as_str()),
        "grok" => config.llm.grok.as_ref().map(|c| c.model.as_str()),
        "gemini" => config.llm.gemini.as_ref().map(|c| c.model.as_str()),
        "ollama" => config.llm.ollama.as_ref().map(|c| c.model.as_str()),
        _ => None,
    }
}