//! Interactive agent command

use super::Result;
use colored::Colorize;
use gestura_core::{
    AgentPipeline, AgentRequest, AppConfig, AppConfigSecurityExt, AudioCaptureConfig,
    CancellationToken, PermissionLevel, RequestSource, SessionToolSettings, SpeechProcessorCoreExt,
    StreamChunk,
    agent_sessions::{
        AgentSessionStore, FileAgentSessionStore, MessageSource, SessionLlmConfig,
        SessionPermissionLevel, SessionToolSettingsConfigExt,
    },
    get_speech_processor, llm_overrides,
    tool_confirmation::{TOOL_CONFIRMATIONS, ToolConfirmationDecision},
};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::sync::mpsc;

mod live_actions;
mod markdown_ansi;
mod slash;
mod tui;

/// Options for the agent command
#[derive(Debug, Default)]
pub struct AgentOptions<'a> {
    pub model: Option<&'a str>,
    pub resume: bool,
    pub session: Option<&'a str>,
    pub tui: bool,
    pub voice: bool,
    pub system: Option<&'a str>,

    /// If set, overrides the session's tool permission level at startup and
    /// persists it to the session.
    pub permission_level_override: Option<SessionPermissionLevel>,
}

/// Persisted agent message.
///
/// This is a re-export of the canonical message type from `gestura-core`, so the
/// CLI (including the TUI) does not maintain a divergent persistence model.
pub use gestura_core::agent_sessions::ConversationMessage as AgentMessage;

/// Persisted agent session.
///
/// The CLI uses the canonical core session type; all persistence is performed
/// via the core-backed `FileAgentSessionStore`.
pub use gestura_core::agent_sessions::AgentSession;

/// Session listing filter options.
pub use gestura_core::agent_sessions::SessionFilter;

/// Session metadata returned by `list_sessions*`.
pub use gestura_core::agent_sessions::SessionInfo;

/// Return the CLI session store (file-backed, one JSON file per session).
fn session_store() -> FileAgentSessionStore {
    FileAgentSessionStore::new_default()
}

/// Create a new CLI session.
///
/// The CLI prefers using the current working directory as the session workspace
/// (so file/shell tools operate in the user's project), but falls back to a
/// sandbox workspace if the CWD cannot be determined.
fn new_cli_session(model: Option<String>) -> Result<AgentSession> {
    match std::env::current_dir() {
        Ok(cwd) => AgentSession::new_with_workspace(cwd, model)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>),
        Err(_) => {
            AgentSession::new_sandbox(model).map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
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
/// The unified session model stores tool settings inside `AgentSession.state.tool_settings`.
/// Older sessions (or shells that didn't initialize settings) may have this field missing.
///
/// Returns `true` if the session was updated.
pub(super) fn ensure_session_tool_settings(session: &mut AgentSession, config: &AppConfig) -> bool {
    if session.state.tool_settings.is_some() {
        return false;
    }

    session.state.tool_settings = Some(SessionToolSettings::from_global_config(config));
    true
}

fn apply_permission_level_override(
    session: &mut AgentSession,
    override_level: Option<SessionPermissionLevel>,
) -> bool {
    let Some(level) = override_level else {
        return false;
    };

    let settings = session
        .state
        .tool_settings
        .get_or_insert_with(Default::default);
    let changed = settings.permission_level != level;
    settings.permission_level = level;
    changed
}

/// Derive the effective tool execution policy for an `AgentRequest`.
///
/// - `PermissionLevel` is used for runtime gating (sandbox/restricted/full)
/// - `allowed_tools` is used for tool visibility to the LLM (empty = all tools)
pub(super) fn derive_request_policy(session: &AgentSession) -> (PermissionLevel, Vec<String>) {
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
fn save_cli_session(session: &AgentSession) -> Result<()> {
    session_store()
        .save(session)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

/// Load a session by ID.
fn load_cli_session(id: &str) -> Result<AgentSession> {
    session_store()
        .load(id)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

/// Load the most recently active session, if any.
fn load_last_cli_session() -> Result<Option<AgentSession>> {
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
fn export_cli_session(session: &AgentSession, path: &Path) -> Result<()> {
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
fn basic_mode_tools_command(args: &[&str], session: &mut AgentSession) {
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
        .join("agent_history.txt")
}

pub fn run(opts: AgentOptions<'_>) -> Result<()> {
    // If TUI mode is requested, launch the TUI
    if opts.tui {
        return tui::run_tui(opts);
    }

    // Voice mode is handled within basic mode (voice input for prompts)
    // The voice flag enables voice-to-text for user input

    // Basic readline mode
    run_basic_mode(opts)
}

fn run_basic_mode(opts: AgentOptions<'_>) -> Result<()> {
    let AgentOptions {
        model,
        resume,
        session,
        voice,
        system,
        permission_level_override,
        ..
    } = opts;

    // Voice mode is mutable at runtime (toggled via `/listen`).
    let mut voice = voice;

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
    let mut agent_session = if resume {
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
                        "No previous session found, starting new session.".yellow()
                    );
                    new_cli_session(model.map(String::from))?
                }
            }
        }
    } else {
        new_cli_session(model.map(String::from))?
    };

    // Load config.
    //
    // IMPORTANT: do not hand-roll provider/model mutation here. We rely on the
    // canonical core override helpers so provider configs are materialized and
    // the effective model is never empty.
    let mut config = AppConfig::load();

    // Normalize the session's provider/model override early so:
    // - `/model` and header display are consistent
    // - pipeline metadata never ends up with an empty model
    // - legacy `session.model` strings are migrated to `provider:model`
    match llm_overrides::normalize_session_llm_override(&config, &mut agent_session, model) {
        Ok(true) => {
            save_cli_session(&agent_session)?;
        }
        Ok(false) => {}
        Err(msg) => {
            println!("{} {msg}", "✗".red());
        }
    }

    // Ensure persisted sessions have tool settings (migration / defaults).
    if ensure_session_tool_settings(&mut agent_session, &config) {
        save_cli_session(&agent_session)?;
    }

    // Apply startup override for session permission level.
    if apply_permission_level_override(&mut agent_session, permission_level_override) {
        save_cli_session(&agent_session)?;
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
    let (_, effective) =
        llm_overrides::apply_basic_mode_session_llm_overrides(&config, &agent_session);
    let session_info = format!(
        "session {} · provider {} · model {}",
        &agent_session.id[..8],
        effective.provider,
        effective.model
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
    if let Some(workspace) = agent_session.workspace_dir() {
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
    if agent_session.message_count() != 0 {
        let history_header = format!("┌─ History ({} messages) ", agent_session.message_count());
        let history_padding = inner_width.saturating_sub(history_header.len()) + 3;
        println!(
            "{}{}",
            history_header.dimmed(),
            "─".repeat(history_padding).dimmed()
        );

        for msg in &agent_session.state.messages {
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

    // Main agent loop
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
                let (mut input, mut input_source) = if input.is_empty() && voice {
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

                // Handle /exec by stripping the prefix so it goes to the LLM.
                if let Some(rest) = input.strip_prefix("/exec ") {
                    input = rest.to_string();
                }

                // Handle commands
                if input.starts_with('/') {
                    let mut parts = input.split_whitespace();
                    let cmd = parts.next().unwrap_or("");
                    let args: Vec<&str> = parts.collect();
                    match cmd.to_ascii_lowercase().as_str() {
                        "/exit" | "/quit" | "/q" => {
                            save_cli_session(&agent_session)?;
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
                                        // Treat transcription as the user input (falls through
                                        // to the LLM call below).
                                        input = text;
                                        input_source = MessageSource::Voice;
                                    } else {
                                        // No transcription; do not send an empty message.
                                        continue;
                                    }
                                }
                                Err(e) => {
                                    eprintln!("{}: {}", "Voice error".red(), e);
                                    continue;
                                }
                            }
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
                                "/memory [list|search|save|clear|delete]".green(),
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
                                "Start a fresh session".dimmed()
                            );
                            println!(
                                "{}  {}             {}",
                                "│".dimmed(),
                                "/voice".green(),
                                "Record voice input via microphone".dimmed()
                            );
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
                                "Toggle listening mode (Enter on empty prompt to record)".dimmed()
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
                            basic_mode_tools_command(&args, &mut agent_session);
                            println!();
                            continue;
                        }
                        "/summarize" => {
                            println!();
                            // Get conversation history
                            let history: Vec<String> = agent_session
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
                                agent_session.add_assistant_message(
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
                        "/memory" => {
                            println!();
                            basic_mode_memory_command(&args, &agent_session);
                            println!();
                            continue;
                        }
                        "/clear" => {
                            print!("\x1B[2J\x1B[1;1H");
                            continue;
                        }
                        "/save" => {
                            save_cli_session(&agent_session)?;
                            println!("{} Session saved", "✓".green());
                            continue;
                        }
                        "/history" => {
                            let user_msgs = agent_session
                                .state
                                .messages
                                .iter()
                                .filter(|m| m.role == "user")
                                .count();
                            let asst_msgs = agent_session
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
                                agent_session.id
                            );
                            println!(
                                "{}  {} {}",
                                "│".dimmed(),
                                "Total Messages:".dimmed(),
                                agent_session.message_count()
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
                            if let Some(workspace) = agent_session.workspace_dir() {
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
                            save_cli_session(&agent_session)?;
                            agent_session = new_cli_session(model.map(String::from))?;
                            println!();
                            println!(
                                "{} {} {}",
                                "✓".green(),
                                "New session started:".dimmed(),
                                agent_session.id
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
                            basic_mode_knowledge_command(&args, &agent_session);
                            println!();
                            continue;
                        }
                        "/agent" => {
                            println!();
                            basic_mode_agent_command(&args, &config, &agent_session);
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
                            // Toggle listening mode for basic CLI.
                            if !voice {
                                if !gestura_core::is_microphone_available() {
                                    println!(
                                        "{} {}",
                                        "✗".red(),
                                        "Microphone not available; cannot enable listening mode"
                                            .dimmed()
                                    );
                                    voice = false;
                                } else {
                                    voice = true;
                                    println!(
                                        "{} {}",
                                        "🎤".green(),
                                        "Listening mode enabled (press Enter on an empty prompt to record)"
                                            .dimmed()
                                    );
                                }
                            } else {
                                voice = false;
                                println!(
                                    "{} {}",
                                    "🔇".yellow(),
                                    "Listening mode disabled".dimmed()
                                );
                            }

                            basic_mode_listen_command(voice);
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
                            basic_mode_session_command(&args, &agent_session);
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
                            if let Some(new_llm) =
                                basic_mode_model_command(&args, &config, &agent_session)
                            {
                                // Persist canonical override + legacy hint.
                                let provider = new_llm.provider.clone().unwrap_or_default();
                                let model = new_llm.model.clone().unwrap_or_default();
                                if !provider.trim().is_empty() && !model.trim().is_empty() {
                                    agent_session.state.llm_config = Some(new_llm);
                                    agent_session.model = Some(format!("{}:{}", provider, model));
                                    save_cli_session(&agent_session)?;
                                }
                            }
                            println!();
                            continue;
                        }
                        "/hooks" | "/hook" => {
                            println!();
                            if args.is_empty() {
                                basic_mode_hooks_command(&config);
                            } else {
                                let mut cfg = config.clone();
                                match slash::apply_hooks_subcommand(&args, &mut cfg) {
                                    Ok(outcome) => {
                                        let changed = outcome.changed();
                                        for line in outcome.into_lines() {
                                            println!("{line}");
                                        }
                                        if changed {
                                            if let Err(e) = cfg.save() {
                                                println!(
                                                    "{} Failed to save config: {}",
                                                    "✗".red(),
                                                    e
                                                );
                                            } else {
                                                config = cfg;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        println!("{} {}", "✗".red(), e);
                                        println!();
                                        if let Ok(outcome) =
                                            slash::apply_hooks_subcommand(&["help"], &mut cfg)
                                        {
                                            for line in outcome.into_lines() {
                                                println!("{line}");
                                            }
                                        }
                                    }
                                }
                            }
                            println!();
                            continue;
                        }
                        "/permissions" | "/permission" => {
                            println!();
                            if args.is_empty() {
                                basic_mode_permissions_command();
                            } else {
                                match slash::run_permissions_subcommand(&args, &mut agent_session) {
                                    Ok(outcome) => {
                                        for line in outcome.lines {
                                            println!("{line}");
                                        }
                                        if outcome.changed_permissions {
                                            println!("{} Permissions updated.", "✓".green());
                                        }

                                        if outcome.session_changed
                                            && let Err(e) = save_cli_session(&agent_session)
                                        {
                                            println!("{} Failed to save session: {}", "✗".red(), e);
                                        }
                                    }
                                    Err(e) => {
                                        println!("{} {}", "✗".red(), e);
                                        println!();
                                        if let Ok(outcome) = slash::run_permissions_subcommand(
                                            &["help"],
                                            &mut agent_session,
                                        ) {
                                            for line in outcome.lines {
                                                println!("{line}");
                                            }
                                        }
                                    }
                                }
                            }
                            println!();
                            continue;
                        }
                        "/tasks" => {
                            println!();
                            if args.is_empty() {
                                basic_mode_tasks_command(&agent_session, &rt);
                            } else {
                                use gestura_core::tasks::TaskManager;

                                let task_manager = TaskManager::new(
                                    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")),
                                );
                                match slash::run_tasks_subcommand(
                                    &args,
                                    &task_manager,
                                    &agent_session.id,
                                    agent_session.workspace_dir().map(|path| path.as_path()),
                                ) {
                                    Ok(out) => {
                                        let lines = match out.live_action {
                                            Some(act) => {
                                                match slash::execute_tasks_live_action(&rt, act) {
                                                    Ok(lines) => lines,
                                                    Err(e) => {
                                                        println!("{} {}", "✗".red(), e);
                                                        Vec::new()
                                                    }
                                                }
                                            }
                                            None => out.lines,
                                        };
                                        for line in lines {
                                            println!("{line}");
                                        }
                                    }
                                    Err(e) => {
                                        println!("{} {}", "✗".red(), e);
                                    }
                                }
                            }
                            println!();
                            continue;
                        }

                        "/task" => {
                            println!();
                            // `/task` is the subcommand-oriented interface, but users often type it
                            // when they really want the interactive task browser. Treat `/task`
                            // (no args) as an alias for `/tasks`.
                            if args.is_empty() {
                                basic_mode_tasks_command(&agent_session, &rt);
                            } else {
                                use gestura_core::tasks::TaskManager;

                                let task_manager = TaskManager::new(
                                    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")),
                                );
                                match slash::run_tasks_subcommand(
                                    &args,
                                    &task_manager,
                                    &agent_session.id,
                                    agent_session.workspace_dir().map(|path| path.as_path()),
                                ) {
                                    Ok(out) => {
                                        let lines = match out.live_action {
                                            Some(act) => {
                                                match slash::execute_tasks_live_action(&rt, act) {
                                                    Ok(lines) => lines,
                                                    Err(e) => {
                                                        println!("{} {}", "✗".red(), e);
                                                        Vec::new()
                                                    }
                                                }
                                            }
                                            None => out.lines,
                                        };
                                        for line in lines {
                                            println!("{line}");
                                        }
                                    }
                                    Err(e) => {
                                        println!("{} {}", "✗".red(), e);
                                    }
                                }
                            }
                            println!();
                            continue;
                        }
                        "/theme" | "/themes" => {
                            println!();
                            basic_mode_themes_command();
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
                agent_session.add_user_message(&input, input_source);

                // Handle explicit /tools command only (not natural language questions)
                if input.trim().starts_with("/tools") {
                    let parts: Vec<&str> = input.split_whitespace().collect();
                    println!();
                    basic_mode_tools_command(&parts[1..], &mut agent_session);
                    println!();
                    continue;
                }

                // Handle /summarize command - summarize conversation history without calling LLM
                if input.trim().starts_with("/summarize") {
                    println!();
                    // Get conversation history
                    let history: Vec<String> = agent_session
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
                        agent_session.add_assistant_message(
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

                // Build conversation history for the AgentPipeline
                let history: Vec<gestura_core::Message> =
                    agent_session.to_pipeline_messages_limited(10);

                // ─────────────────────────────────────────────────────────────
                // AI RESPONSE: Show thinking indicator then response
                // ─────────────────────────────────────────────────────────────
                // Build the agent request with workspace sandboxing
                let mut request = AgentRequest::new(&input)
                    .with_streaming(true)
                    .with_source(RequestSource::CliBasic)
                    .with_history(history);

                // Set workspace directory for sandboxed operations
                if let Some(workspace) = agent_session.workspace_dir() {
                    request = request.with_workspace(workspace.clone());
                }

                // Add system prompt if available
                if let Some(ref sys) = system_prompt {
                    request = request.with_system_prompt(sys.clone());
                }

                // Compute and apply the effective provider/model for this session.
                //
                // IMPORTANT: we apply overrides to the *pipeline config* so the underlying LLM
                // call matches what `/model` says, and so provider configs are materialized.
                let (config_for_pipeline, effective) =
                    llm_overrides::apply_basic_mode_session_llm_overrides(&config, &agent_session);
                let provider_name = effective.provider;
                let model_name = effective.model;
                let (permission_level, allowed_tools) = derive_request_policy(&agent_session);
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
                let session_id_for_tool_confirm = agent_session.id.clone();

                let config_clone = config_for_pipeline;
                let response: Result<gestura_core::AgentResponse> = rt.block_on(async move {
                    let (tx, mut rx) = mpsc::channel::<StreamChunk>(100);
                    let cancel_token = CancellationToken::new();
                    let cancel_for_task = cancel_token.clone();

                    let stream_task = tokio::spawn(async move {
                        let pipeline = AgentPipeline::with_provider_optimized_config(config_clone)
                            .with_knowledge(get_knowledge_store(), get_knowledge_settings());
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
                            StreamChunk::ReflectionStarted { reason } => {
                                println!();
                                println!(
                                    "  {} {}",
                                    "↺".magenta(),
                                    format!("reflection: {reason}").dimmed()
                                );
                                print!("  ");
                                let _ = std::io::stdout().flush();
                            }
                            StreamChunk::ReflectionComplete {
                                summary,
                                stored,
                                promoted,
                            } => {
                                println!();
                                println!(
                                    "  {} {}{}{}",
                                    "🧠".magenta(),
                                    summary.dimmed(),
                                    if stored { " · stored" } else { "" },
                                    if promoted { " · promoted" } else { "" },
                                );
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

                        agent_session.add_assistant_message(
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
                if agent_session.message_count() % 5 == 0 {
                    let _ = save_cli_session(&agent_session);
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
                    agent_session.id,
                    " ".repeat(24) + "│"
                );
                println!(
                    "{}",
                    "╰─────────────────────────────────────────────────────────────╯".dimmed()
                );
                save_cli_session(&agent_session)?;
                break;
            }
            Err(ReadlineError::Eof) => {
                println!();
                println!("{} {}", "✓".green(), "Session saved. Goodbye!".dimmed());
                save_cli_session(&agent_session)?;
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

/// Interactive MCP server form wizard (shared by Add and Edit flows).
///
/// When `existing` is `None`, this is a new-server wizard; when `Some`, it
/// pre-populates every field from the existing entry (name is immutable on
/// edit).  Returns `Some(entry)` on success, `None` if the user cancels.
fn mcp_server_form(
    existing: Option<&gestura_core::config::McpServerEntry>,
) -> Option<gestura_core::config::McpServerEntry> {
    use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
    use gestura_core::config::{McpScope, McpServerEntry, McpTransportType};

    let theme = ColorfulTheme::default();
    let is_edit = existing.is_some();

    // ── Name ──────────────────────────────────────────────────
    let name: String = if let Some(srv) = existing {
        println!("  Name: {}", srv.name.bold().cyan());
        srv.name.clone()
    } else {
        let n: String = Input::with_theme(&theme)
            .with_prompt("Server name")
            .validate_with(|input: &String| -> std::result::Result<(), &str> {
                if input.trim().is_empty() {
                    Err("Name cannot be empty")
                } else {
                    Ok(())
                }
            })
            .interact_text()
            .ok()?;
        if n.trim().is_empty() {
            return None;
        }
        // Check uniqueness
        let config = AppConfig::load();
        if config.mcp_servers.iter().any(|s| s.name == n) {
            println!(
                "{} Server '{}' already exists. Use {} to modify it.",
                "✗".red(),
                n,
                "Edit".cyan()
            );
            return None;
        }
        n
    };

    // ── Transport type ────────────────────────────────────────
    let transport_labels = [
        "stdio — Local child process",
        "http  — Streamable HTTP (recommended for remote)",
        "sse   — Server-Sent Events (legacy)",
    ];
    let transport_default = match existing.map(|s| &s.transport) {
        Some(McpTransportType::Http) => 1,
        Some(McpTransportType::Sse) => 2,
        _ => 0,
    };
    let transport_idx = Select::with_theme(&theme)
        .with_prompt("Transport type")
        .items(&transport_labels)
        .default(transport_default)
        .interact_opt()
        .ok()
        .flatten()?;
    let transport = match transport_idx {
        1 => McpTransportType::Http,
        2 => McpTransportType::Sse,
        _ => McpTransportType::Stdio,
    };

    // ── Transport-specific fields ─────────────────────────────
    let (command, args_vec, env_map, url, headers_map) = match transport {
        McpTransportType::Stdio => {
            let def_cmd = existing.and_then(|s| s.command.clone()).unwrap_or_default();
            let cmd: String = Input::with_theme(&theme)
                .with_prompt("Command (e.g. npx, uvx, docker)")
                .default(def_cmd)
                .allow_empty(false)
                .interact_text()
                .ok()?;

            let def_args = existing.map(|s| s.args.join("\n")).unwrap_or_default();
            let args_raw: String = Input::with_theme(&theme)
                .with_prompt("Arguments (comma-separated, empty to skip)")
                .default(def_args.replace('\n', ", "))
                .allow_empty(true)
                .interact_text()
                .ok()?;
            let parsed_args: Vec<String> = args_raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let def_env = existing
                .map(|s| {
                    s.env
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, v))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let env_raw: String = Input::with_theme(&theme)
                .with_prompt("Environment variables (KEY=VALUE comma-separated, empty to skip)")
                .default(def_env)
                .allow_empty(true)
                .interact_text()
                .ok()?;
            let mut env = std::collections::HashMap::new();
            for pair in env_raw.split(',') {
                let pair = pair.trim();
                if let Some((k, v)) = pair.split_once('=') {
                    let k = k.trim().to_string();
                    let v = v.trim().to_string();
                    if !k.is_empty() {
                        env.insert(k, v);
                    }
                }
            }

            (
                Some(cmd),
                parsed_args,
                env,
                None,
                std::collections::HashMap::new(),
            )
        }
        McpTransportType::Http | McpTransportType::Sse => {
            let def_url = existing.and_then(|s| s.url.clone()).unwrap_or_default();
            let url_val: String = Input::with_theme(&theme)
                .with_prompt("Server URL")
                .default(def_url)
                .allow_empty(false)
                .interact_text()
                .ok()?;

            let def_headers = existing
                .map(|s| {
                    s.headers
                        .iter()
                        .map(|(k, v)| format!("{}: {}", k, v))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let headers_raw: String = Input::with_theme(&theme)
                .with_prompt("Headers (Key: Value comma-separated, empty to skip)")
                .default(def_headers)
                .allow_empty(true)
                .interact_text()
                .ok()?;
            let mut headers = std::collections::HashMap::new();
            for pair in headers_raw.split(',') {
                let pair = pair.trim();
                if let Some((k, v)) = pair.split_once(':') {
                    let k = k.trim().to_string();
                    let v = v.trim().to_string();
                    if !k.is_empty() {
                        headers.insert(k, v);
                    }
                }
            }

            (
                None,
                Vec::new(),
                std::collections::HashMap::new(),
                Some(url_val),
                headers,
            )
        }
    };

    // ── Scope ─────────────────────────────────────────────────
    let scope_labels = [
        "user    — Available across all projects",
        "project — Shared via .mcp.json",
        "local   — Local override, not committed",
    ];
    let scope_default = match existing.map(|s| &s.scope) {
        Some(McpScope::Project) => 1,
        Some(McpScope::Local) => 2,
        _ => 0,
    };
    let scope_idx = Select::with_theme(&theme)
        .with_prompt("Scope")
        .items(&scope_labels)
        .default(scope_default)
        .interact_opt()
        .ok()
        .flatten()?;
    let scope = match scope_idx {
        1 => McpScope::Project,
        2 => McpScope::Local,
        _ => McpScope::User,
    };

    // ── Timeout ───────────────────────────────────────────────
    let def_timeout = existing.map(|s| s.timeout_secs).unwrap_or(30);
    let timeout_str: String = Input::with_theme(&theme)
        .with_prompt("Timeout (seconds)")
        .default(def_timeout.to_string())
        .interact_text()
        .ok()?;
    let timeout_secs: u64 = timeout_str.parse().unwrap_or(30);

    // ── Auto-reconnect ────────────────────────────────────────
    let def_reconnect = existing.map(|s| s.auto_reconnect).unwrap_or(true);
    let auto_reconnect = Confirm::with_theme(&theme)
        .with_prompt("Auto-reconnect on failure?")
        .default(def_reconnect)
        .interact_opt()
        .ok()
        .flatten()?;

    // ── Enabled ───────────────────────────────────────────────
    let def_enabled = existing.map(|s| s.enabled).unwrap_or(true);
    let enabled = Confirm::with_theme(&theme)
        .with_prompt("Enable this server?")
        .default(def_enabled)
        .interact_opt()
        .ok()
        .flatten()?;

    // ── Confirm ───────────────────────────────────────────────
    println!();
    println!("{}", "─── Server Summary ───".dimmed());
    println!("  Name:           {}", name.cyan());
    println!("  Transport:      {}", format!("{}", transport).cyan());
    match transport {
        McpTransportType::Stdio => {
            println!(
                "  Command:        {}",
                command.as_deref().unwrap_or("(none)")
            );
            if !args_vec.is_empty() {
                println!("  Arguments:      {}", args_vec.join(", "));
            }
            if !env_map.is_empty() {
                println!("  Env vars:       {}", env_map.len());
            }
        }
        _ => {
            println!("  URL:            {}", url.as_deref().unwrap_or("(none)"));
            if !headers_map.is_empty() {
                println!("  Headers:        {}", headers_map.len());
            }
        }
    }
    println!("  Scope:          {}", scope);
    println!("  Timeout:        {}s", timeout_secs);
    println!(
        "  Auto-reconnect: {}",
        if auto_reconnect { "yes" } else { "no" }
    );
    println!(
        "  Enabled:        {}",
        if enabled {
            "yes".green().to_string()
        } else {
            "no".red().to_string()
        }
    );

    let confirm = Confirm::with_theme(&theme)
        .with_prompt(if is_edit {
            "Save changes?"
        } else {
            "Add this server?"
        })
        .default(true)
        .interact_opt()
        .ok()
        .flatten()?;
    if !confirm {
        println!("{}", "Cancelled.".dimmed());
        return None;
    }

    Some(McpServerEntry {
        name,
        transport,
        enabled,
        command,
        args: args_vec,
        env: env_map,
        url,
        headers: headers_map,
        scope,
        timeout_secs,
        auto_reconnect,
        session_default_enabled: true,
    })
}

/// Basic mode `/mcp` slash command handler.
fn basic_mode_mcp_command(args: &[&str]) {
    use dialoguer::{Select, theme::ColorfulTheme};
    use gestura_core::config::{McpServerEntry, McpTransportType};

    fn build_mcp_args_from_entry(sub: &str, entry: &McpServerEntry, is_edit: bool) -> Vec<String> {
        let mut out = vec![sub.to_string(), entry.name.clone()];

        // Make edits "exact" (wizard provides full state).
        if is_edit {
            out.push("--clear-args".to_string());
            out.push("--clear-env".to_string());
            out.push("--clear-headers".to_string());
        }

        out.push("--transport".to_string());
        out.push(format!("{}", entry.transport));
        out.push("--scope".to_string());
        out.push(format!("{}", entry.scope));
        out.push("--timeout".to_string());
        out.push(entry.timeout_secs.to_string());

        if entry.auto_reconnect {
            out.push("--auto-reconnect".to_string());
        } else {
            out.push("--no-auto-reconnect".to_string());
        }

        if entry.enabled {
            out.push("--enabled".to_string());
        } else {
            out.push("--disabled".to_string());
        }

        match entry.transport {
            McpTransportType::Stdio => {
                if let Some(cmd) = &entry.command {
                    out.push("--command".to_string());
                    out.push(cmd.clone());
                }
                for a in &entry.args {
                    out.push("--arg".to_string());
                    out.push(a.clone());
                }
                for (k, v) in &entry.env {
                    out.push("--env".to_string());
                    out.push(format!("{k}={v}"));
                }
            }
            McpTransportType::Http | McpTransportType::Sse => {
                if let Some(url) = &entry.url {
                    out.push("--url".to_string());
                    out.push(url.clone());
                }
                for (k, v) in &entry.headers {
                    out.push("--header".to_string());
                    out.push(format!("{k}: {v}"));
                }
            }
        }

        out
    }

    fn execute_mcp_live_action(
        rt: &tokio::runtime::Runtime,
        cfg: &AppConfig,
        act: slash::McpLiveAction,
    ) -> Vec<String> {
        let registry = gestura_core::get_mcp_client_registry();

        match act {
            slash::McpLiveAction::Status => {
                let connected = rt.block_on(registry.connected_servers());

                let mut lines = vec!["MCP Server Status".to_string(), "═".repeat(50)];
                lines.push(format!("Servers: {} configured", cfg.mcp_servers.len()));
                lines.push(format!(
                    "Enabled: {}",
                    cfg.mcp_servers.iter().filter(|s| s.enabled).count()
                ));
                lines.push(String::new());

                for server in &cfg.mcp_servers {
                    let status = if server.enabled { "✓" } else { "○" };
                    let endpoint = match server.transport {
                        McpTransportType::Stdio => {
                            let cmd = server.command.as_deref().unwrap_or("");
                            let cmd_args = server.args.join(" ");
                            format!("{} {}", cmd, cmd_args).trim().to_string()
                        }
                        _ => server.url.clone().unwrap_or_default(),
                    };
                    let conn = if connected.contains(&server.name) {
                        "●"
                    } else {
                        "○"
                    };
                    lines.push(format!(
                        "  {status} {conn} {} [{}] {}",
                        server.name, server.transport, endpoint
                    ));
                }

                if !connected.is_empty() {
                    lines.push(String::new());
                    lines.push("Connected:".to_string());
                    for name in connected {
                        lines.push(format!("  ✓ {name}"));
                    }
                }

                lines
            }
            slash::McpLiveAction::Tools { server } => {
                let all = rt.block_on(registry.all_tools());
                let filtered: Vec<_> = if let Some(ref filter) = server {
                    all.into_iter().filter(|(name, _)| name == filter).collect()
                } else {
                    all
                };

                if filtered.is_empty() {
                    return vec!["No MCP tools available. Connect a server first.".to_string()];
                }

                let mut lines = vec!["MCP Tools".to_string(), "═".repeat(50)];
                let mut total = 0usize;
                for (srv, tools) in &filtered {
                    lines.push(String::new());
                    lines.push(format!("{srv} ({} tools):", tools.len()));
                    for tool in tools {
                        let desc = tool.description.as_deref().unwrap_or("(no description)");
                        lines.push(format!("  • {} — {desc}", tool.name));
                    }
                    total += tools.len();
                }
                lines.push(String::new());
                lines.push(format!(
                    "Total: {total} tool(s) across {} server(s)",
                    filtered.len()
                ));
                lines
            }
            slash::McpLiveAction::Connect { name } => {
                let Some(srv) = cfg.mcp_servers.iter().find(|s| s.name == name) else {
                    return vec![format!("MCP server not found in config: {name}")];
                };

                match rt.block_on(registry.connect(srv)) {
                    Ok(tools) => {
                        let mut lines = vec![format!(
                            "Connected to MCP server: {name} ({} tools discovered)",
                            tools.len()
                        )];
                        for t in &tools {
                            lines.push(format!("  • {}", t.name));
                            if let Some(desc) = &t.description {
                                lines.push(format!("    {desc}"));
                            }
                        }
                        lines
                    }
                    Err(e) => vec![format!("Failed to connect to '{name}': {e}")],
                }
            }
            slash::McpLiveAction::Disconnect { name } => {
                rt.block_on(registry.disconnect(&name));
                vec![format!("Disconnected from MCP server: {name}")]
            }
        }
    }

    let rt = tokio::runtime::Runtime::new().unwrap();

    let run_canonical = |cmd_args: &[&str]| {
        let mut cfg = AppConfig::load();
        match slash::run_mcp_subcommand(cmd_args, &mut cfg) {
            Ok(out) => {
                if out.changed
                    && let Err(e) = cfg.save()
                {
                    println!("{} Failed to save config: {e}", "✗".red());
                    return;
                }

                let lines = if let Some(act) = out.live_action {
                    execute_mcp_live_action(&rt, &cfg, act)
                } else {
                    out.lines
                };

                for l in lines {
                    println!("{l}");
                }
            }
            Err(e) => {
                println!("{} {e}", "✗".red());
            }
        }
    };

    if !args.is_empty() {
        // Explicit subcommand mode: delegate to canonical slash handler.
        run_canonical(args);
        return;
    }

    // Interactive MCP browser (mirrors GUI management panel)
    let registry = gestura_core::get_mcp_client_registry();
    loop {
        let config = AppConfig::load();
        let connected = rt.block_on(registry.connected_servers());
        let labels: Vec<String> = config
            .mcp_servers
            .iter()
            .map(|srv| {
                let status = if srv.enabled { "✓" } else { "✗" };
                let conn = if connected.contains(&srv.name) {
                    "●"
                } else {
                    "○"
                };
                format!(
                    "{} {} {:<20} [{}] {}",
                    status, conn, srv.name, srv.transport, srv.scope
                )
            })
            .collect();

        let mut menu_items: Vec<String> = labels;
        menu_items.push("+ Add Server".green().bold().to_string());
        menu_items.push("← Back to agent".to_string());

        println!();
        let sel = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("MCP Servers")
            .items(&menu_items)
            .default(0)
            .interact_opt();

        let Some(idx) = sel.ok().flatten() else {
            break;
        };

        let server_count = config.mcp_servers.len();
        if idx == server_count {
            // "+ Add Server"
            println!();
            println!("{}", "Add MCP Server".bold().cyan());
            println!("{}", "═".repeat(40));
            if let Some(entry) = mcp_server_form(None) {
                let args_strings = build_mcp_args_from_entry("add", &entry, false);
                let args_refs: Vec<&str> = args_strings.iter().map(|s| s.as_str()).collect();
                run_canonical(&args_refs);
            }
            continue;
        }
        if idx > server_count {
            break; // "Back to agent"
        }

        let srv = &config.mcp_servers[idx];
        // Show detail + action menu for selected server
        println!();
        println!("{}", srv.name.bold().cyan());
        println!("{}", "─".repeat(40));
        println!("  Transport:      {}", format!("{}", srv.transport).cyan());
        println!(
            "  Enabled:        {}",
            if srv.enabled {
                "yes".green().to_string()
            } else {
                "no".red().to_string()
            }
        );
        println!("  Scope:          {}", srv.scope);
        match srv.transport {
            McpTransportType::Stdio => {
                println!(
                    "  Command:        {}",
                    srv.command.as_deref().unwrap_or("(none)")
                );
                if !srv.args.is_empty() {
                    println!("  Arguments:      {}", srv.args.join(", "));
                }
                if !srv.env.is_empty() {
                    println!("  Env vars:");
                    for (k, v) in &srv.env {
                        println!("    {}={}", k, v);
                    }
                }
            }
            _ => {
                println!(
                    "  URL:            {}",
                    srv.url.as_deref().unwrap_or("(none)")
                );
                if !srv.headers.is_empty() {
                    println!("  Headers:");
                    for (k, v) in &srv.headers {
                        println!("    {}: {}", k, v);
                    }
                }
            }
        }
        println!(
            "  Connected:      {}",
            if connected.contains(&srv.name) {
                "yes".green().to_string()
            } else {
                "no".dimmed().to_string()
            }
        );
        println!("  Timeout:        {}s", srv.timeout_secs);
        println!(
            "  Auto-reconnect: {}",
            if srv.auto_reconnect { "yes" } else { "no" }
        );

        let toggle_label = if srv.enabled {
            "Disable this server"
        } else {
            "Enable this server"
        };
        let conn_label = if connected.contains(&srv.name) {
            "Disconnect"
        } else {
            "Connect"
        };

        let actions = ["Edit", toggle_label, conn_label, "Remove", "← Back to list"];
        let action = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Action")
            .items(&actions)
            .default(0)
            .interact_opt();

        match action.ok().flatten() {
            Some(0) => {
                // Edit
                let srv_clone = srv.clone();
                println!();
                println!("{}", format!("Edit: {}", srv_clone.name).bold().cyan());
                println!("{}", "═".repeat(40));
                if let Some(updated) = mcp_server_form(Some(&srv_clone)) {
                    let args_strings = build_mcp_args_from_entry("edit", &updated, true);
                    let args_refs: Vec<&str> = args_strings.iter().map(|s| s.as_str()).collect();
                    run_canonical(&args_refs);
                }
            }
            Some(1) => {
                // Toggle enable/disable
                let name = srv.name.clone();
                let subcmd = if srv.enabled { "disable" } else { "enable" };
                run_canonical(&[subcmd, &name]);
            }
            Some(2) => {
                // Connect/disconnect
                let name = srv.name.clone();
                let subcmd = if connected.contains(&srv.name) {
                    "disconnect"
                } else {
                    "connect"
                };
                run_canonical(&[subcmd, &name]);
            }
            Some(3) => {
                // Remove
                let name = srv.name.clone();
                run_canonical(&["remove", &name]);
            }
            _ => {} // Back to list — loop continues
        }
        // Loop continues: config reloaded at top
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
///
/// Uses [`KnowledgeSettingsManager`] for session-scoped enable/disable persistence.
fn basic_mode_knowledge_command(args: &[&str], session: &AgentSession) {
    use dialoguer::{Select, theme::ColorfulTheme};
    use gestura_core::knowledge::{KnowledgeQuery, KnowledgeStore, register_builtin_knowledge};

    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    let store = KnowledgeStore::with_default_dir();
    register_builtin_knowledge(&store);
    let settings_mgr = gestura_core::knowledge::KnowledgeSettingsManager::new(
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
    );
    let session_id = &session.id;

    /// Overlay per-session enabled state onto a list of knowledge items.
    fn apply_session_enabled(
        items: &mut [gestura_core::knowledge::KnowledgeItem],
        mgr: &gestura_core::knowledge::KnowledgeSettingsManager,
        sid: &str,
    ) {
        if let Ok(enabled_ids) = mgr.get_enabled_knowledge(sid) {
            for item in items.iter_mut() {
                item.enabled = enabled_ids.contains(&item.id);
            }
        }
    }

    match subcommand.as_deref() {
        None => {
            // Interactive knowledge browser
            let mut items = store.list();
            items.sort_by(|a, b| a.name.cmp(&b.name));
            apply_session_enabled(&mut items, &settings_mgr, session_id);
            if items.is_empty() {
                println!("{}", "No knowledge items registered.".dimmed());
                return;
            }

            loop {
                let labels: Vec<String> = items
                    .iter()
                    .map(|item| {
                        let status = if item.enabled { "✓" } else { "✗" };
                        let desc_short = if item.description.len() > 40 {
                            format!("{}…", &item.description[..39])
                        } else {
                            item.description.clone()
                        };
                        format!(
                            "{} {:<24} [{}] {}",
                            status, item.name, item.category, desc_short
                        )
                    })
                    .collect();

                let mut menu_items: Vec<String> = labels;
                menu_items.push("← Back to agent".to_string());

                println!();
                let sel = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Knowledge Base")
                    .items(&menu_items)
                    .default(0)
                    .interact_opt();

                let Some(idx) = sel.ok().flatten() else {
                    break;
                };

                if idx >= items.len() {
                    break; // "Back to agent"
                }

                let item = &items[idx];
                // Show detail
                println!();
                println!("{}", item.name.bold().cyan());
                println!("{}", "─".repeat(40));
                println!("  Category: {}", item.category.cyan());
                println!(
                    "  Enabled:  {}",
                    if item.enabled {
                        "yes".green().to_string()
                    } else {
                        "no".red().to_string()
                    }
                );
                println!("  Priority: {}", item.priority);
                println!();
                println!("  {}", item.description);
                if !item.triggers.is_empty() {
                    println!();
                    println!("  {}", "Triggers:".bold());
                    for trigger in &item.triggers {
                        println!("    • {}", trigger);
                    }
                }
                if !item.core_content.is_empty() {
                    println!();
                    println!("  {}", "Content Preview:".bold());
                    for line in item.core_content.lines().take(8) {
                        println!("    {}", line.dimmed());
                    }
                    let total = item.core_content.lines().count();
                    if total > 8 {
                        println!("    {}", format!("... ({} more lines)", total - 8).dimmed());
                    }
                }

                let toggle_label = if item.enabled {
                    "Disable this item"
                } else {
                    "Enable this item"
                };
                let actions = [toggle_label, "← Back to list"];
                let action = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Action")
                    .items(&actions)
                    .default(0)
                    .interact_opt();

                if let Some(0) = action.ok().flatten() {
                    let new_enabled = !item.enabled;
                    let _ = settings_mgr.set_knowledge_enabled(session_id, &item.id, new_enabled);
                    let label = if new_enabled { "enabled" } else { "disabled" };
                    println!("{} Knowledge '{}' {}", "✓".green(), item.name.cyan(), label);
                    // Refresh items list for next iteration
                    items = store.list();
                    items.sort_by(|a, b| a.name.cmp(&b.name));
                    apply_session_enabled(&mut items, &settings_mgr, session_id);
                }
                // Loop back to list
            }
        }
        Some("list") => {
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

/// Basic mode `/memory` slash command handler — interactive memory bank browser.
///
/// When called with no subcommand, shows a dialoguer-based interactive menu.
/// Also supports explicit subcommands: `list`, `save`, `search <query>`, `clear`, `delete <path>`.
fn basic_mode_memory_command(args: &[&str], session: &AgentSession) {
    if args.is_empty() {
        match tokio::runtime::Runtime::new() {
            Ok(rt) => {
                if let Err(error) =
                    rt.block_on(crate::commands::memory::browse_session_memory(session))
                {
                    println!("{} {}", "✗".red().bold(), format!("{error}").red());
                }
            }
            Err(error) => {
                println!("{} {}", "✗".red().bold(), format!("{error}").red());
            }
        }
        return;
    }

    use dialoguer::{Confirm, Select, theme::ColorfulTheme};
    use live_actions::{MemoryExecOutput, execute_memory_live_action};

    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    let workspace_dir = match session.workspace_dir() {
        Some(dir) => dir.to_path_buf(),
        None => {
            println!(
                "{} {}",
                "✗".red().bold(),
                "No workspace directory configured. Cannot access memory bank.".red()
            );
            return;
        }
    };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            println!(
                "{} {}",
                "✗".red().bold(),
                format!("Failed to create Tokio runtime: {e}").red()
            );
            return;
        }
    };

    let print_help = || {
        if let Ok(out) = slash::run_memory_subcommand(&["help"], session) {
            for l in out.lines {
                println!("{l}");
            }
        }
    };

    let run_canonical = |cmd_args: &[&str]| {
        match slash::run_memory_subcommand(cmd_args, session) {
            Ok(out) => {
                let Some(act) = out.live_action else {
                    for l in out.lines {
                        println!("{l}");
                    }
                    return;
                };

                // Execute live action using the single runtime for this handler.
                match execute_memory_live_action(&rt, &workspace_dir, act) {
                    Ok(MemoryExecOutput::Listed(entries)) => {
                        if entries.is_empty() {
                            println!(
                                "{} {}",
                                "◆".yellow().bold(),
                                "No memory bank entries found.".yellow()
                            );
                        } else {
                            println!(
                                "{} {}",
                                "◆".blue().bold(),
                                format!("Memory Bank Entries ({} total):", entries.len()).blue()
                            );
                            println!();
                            for entry in &entries {
                                println!(
                                    "  {} {} (Session: {})",
                                    "•".dimmed(),
                                    entry.timestamp.format("%Y-%m-%d %H:%M UTC"),
                                    entry.session_id.dimmed()
                                );
                                println!("    {}", entry.summary);
                                println!();
                            }
                        }
                    }
                    Ok(MemoryExecOutput::Searched { query, results }) => {
                        if results.is_empty() {
                            println!("{}", format!("No results for '{query}'.").dimmed());
                        } else {
                            println!(
                                "{} {}",
                                "◆".blue().bold(),
                                format!("Search: '{}' — {} result(s)", query, results.len()).blue()
                            );
                            for r in &results {
                                println!(
                                    "  {} {} — {}",
                                    "•".dimmed(),
                                    r.timestamp.format("%Y-%m-%d %H:%M UTC"),
                                    r.summary
                                );
                            }
                        }
                    }
                    Ok(MemoryExecOutput::Saved(path)) => {
                        println!(
                            "{} {}",
                            "✓".green().bold(),
                            "Saved conversation to memory bank".green()
                        );
                        println!("  File: {}", path.display());
                    }
                    Ok(MemoryExecOutput::Cleared(0)) => {
                        println!(
                            "{} {}",
                            "◆".yellow().bold(),
                            "Memory bank is already empty.".yellow()
                        );
                    }
                    Ok(MemoryExecOutput::Cleared(count)) => {
                        println!(
                            "{} {}",
                            "✓".green().bold(),
                            format!("Cleared {} memory bank entries.", count).green()
                        );
                    }
                    Ok(MemoryExecOutput::Deleted) => {
                        println!("{} {}", "✓".green().bold(), "Deleted memory entry".green());
                    }
                    Err(e) => {
                        println!(
                            "{} {}",
                            "✗".red().bold(),
                            format!("Memory operation failed: {e}").red()
                        );
                    }
                }
            }
            Err(e) => {
                println!("{} {}", "✗".red().bold(), e.red());
                print_help();
            }
        }
    };

    let load_entries = || -> Vec<gestura_core::memory_bank::MemoryBankEntry> {
        let out = slash::run_memory_subcommand(&["list"], session).ok();
        let Some(out) = out else {
            return Vec::new();
        };
        let Some(act) = out.live_action else {
            return Vec::new();
        };
        match execute_memory_live_action(&rt, &workspace_dir, act) {
            Ok(MemoryExecOutput::Listed(entries)) => entries,
            Ok(_) | Err(_) => Vec::new(),
        }
    };

    match subcommand.as_deref() {
        None => {
            // Interactive browser
            loop {
                let entries = load_entries();
                let count = entries.len();

                // Build menu: entries + action items
                let mut menu_items: Vec<String> = entries
                    .iter()
                    .map(|e| {
                        let summary_short = if e.summary.len() > 45 {
                            format!("{}…", &e.summary[..44])
                        } else {
                            e.summary.clone()
                        };
                        format!(
                            "  {} {} — {}",
                            e.timestamp.format("%Y-%m-%d %H:%M"),
                            e.session_id.chars().take(8).collect::<String>().dimmed(),
                            summary_short
                        )
                    })
                    .collect();

                menu_items.push(format!("{} Save conversation", "💾"));
                menu_items.push(format!("{} Search entries", "🔍"));
                if count > 0 {
                    menu_items.push(format!("{} Clear all ({} entries)", "🗑", count));
                }
                menu_items.push("← Back to agent".to_string());

                println!();
                let sel = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt(format!("Memory Bank ({} entries)", count))
                    .items(&menu_items)
                    .default(0)
                    .interact_opt();

                let Some(idx) = sel.ok().flatten() else {
                    break;
                };

                if idx < entries.len() {
                    // Show entry detail
                    let entry = &entries[idx];
                    println!();
                    println!("{}", "Memory Bank Entry".bold().cyan());
                    println!("{}", "─".repeat(50));
                    println!(
                        "  Timestamp:  {}",
                        entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
                    );
                    println!("  Session:    {}", entry.session_id.cyan());
                    println!("  Summary:    {}", entry.summary);
                    if let Some(ref path) = entry.file_path {
                        println!("  File:       {}", path.display().to_string().dimmed());
                    }
                    println!();
                    println!("  {}", "Content Preview:".bold());
                    for line in entry.content.lines().take(10) {
                        println!("    {}", line.dimmed());
                    }
                    let total = entry.content.lines().count();
                    if total > 10 {
                        println!(
                            "    {}",
                            format!("... ({} more lines)", total - 10).dimmed()
                        );
                    }

                    // Entry detail actions
                    let actions = ["← Back to list", "🗑 Delete entry"];
                    let action = Select::with_theme(&ColorfulTheme::default())
                        .with_prompt("Action")
                        .items(&actions)
                        .default(0)
                        .interact_opt();

                    if action.ok().flatten() == Some(1) {
                        let Some(path) = entry.file_path.as_ref() else {
                            println!(
                                "{} {}",
                                "✗".red().bold(),
                                "This entry has no file path; cannot delete.".red()
                            );
                            continue;
                        };

                        let path_str = path.display().to_string();
                        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
                            .with_prompt(format!(
                                "Delete memory entry file '{}' ? (This is destructive)",
                                path_str
                            ))
                            .default(false)
                            .interact()
                            .unwrap_or(false);
                        if confirmed {
                            run_canonical(&["delete", "--confirmed", &path_str]);
                        }
                    }

                    // Loop back to list
                    continue;
                }

                // Action items
                let action_offset = entries.len();
                let action_idx = idx - action_offset;

                if action_idx == 0 {
                    // Save
                    run_canonical(&["save"]);
                } else if action_idx == 1 {
                    // Search
                    let query: String = dialoguer::Input::with_theme(&ColorfulTheme::default())
                        .with_prompt("Search query")
                        .allow_empty(true)
                        .interact_text()
                        .unwrap_or_default();
                    if !query.is_empty() {
                        run_canonical(&["search", &query, "--limit", "10"]);
                    }
                } else if action_idx == 2 && count > 0 {
                    // Clear
                    let confirmed = Confirm::with_theme(&ColorfulTheme::default())
                        .with_prompt(format!("Delete all {} memory entries?", count))
                        .default(false)
                        .interact()
                        .unwrap_or(false);
                    if confirmed {
                        run_canonical(&["clear", "--confirmed"]);
                    }
                } else {
                    break; // Back to agent
                }
            }
        }
        Some("clear") => {
            // In basic mode, prefer to prompt instead of forcing the user to type --confirmed.
            let confirmed = args.contains(&"--confirmed")
                || Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Clear all memory entries? (This is destructive)")
                    .default(false)
                    .interact()
                    .unwrap_or(false);
            if confirmed {
                run_canonical(&["clear", "--confirmed"]);
            }
        }
        Some("delete") => {
            let has_confirmed = args.contains(&"--confirmed");
            let path_arg = args.iter().skip(1).find(|a| **a != "--confirmed").copied();

            let Some(path_str) = path_arg else {
                println!("{} Usage: /memory delete <path>", "✗".red());
                return;
            };

            if !has_confirmed {
                let confirmed = Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt(format!(
                        "Delete memory entry file '{path_str}'? (This is destructive)"
                    ))
                    .default(false)
                    .interact()
                    .unwrap_or(false);
                if !confirmed {
                    return;
                }
            }

            run_canonical(&["delete", "--confirmed", path_str]);
        }
        Some(_) => {
            // list/save/search and any other subcommands route through canonical parsing.
            run_canonical(args);
        }
    }
}

/// Basic mode `/agent` slash command handler.
fn basic_mode_agent_command(args: &[&str], config: &AppConfig, session: &AgentSession) {
    use dialoguer::{Select, theme::ColorfulTheme};

    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None => {
            // Interactive agent dashboard
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

            let mut rows: Vec<(String, String)> = vec![
                ("Version".into(), gestura_core::VERSION.to_string()),
                ("Primary LLM".into(), config.llm.primary.clone()),
                (
                    "Model".into(),
                    session.model.as_deref().unwrap_or("(default)").to_string(),
                ),
                ("Session".into(), session.id[..8].to_string()),
                ("Messages".into(), session.message_count().to_string()),
                (
                    "OpenAI".into(),
                    if has_openai {
                        "✓ configured"
                    } else {
                        "○ not configured"
                    }
                    .to_string(),
                ),
                (
                    "Anthropic".into(),
                    if has_anthropic {
                        "✓ configured"
                    } else {
                        "○ not configured"
                    }
                    .to_string(),
                ),
            ];
            if let Some(ref openai) = config.llm.openai {
                rows.push(("OpenAI model".into(), openai.model.clone()));
            }
            if let Some(ref anthropic) = config.llm.anthropic {
                rows.push(("Anthropic model".into(), anthropic.model.clone()));
            }

            loop {
                let labels: Vec<String> = rows
                    .iter()
                    .map(|(k, v)| format!("{:<20} {}", k, v))
                    .collect();
                let mut items = labels.clone();
                items.push("← Back to agent".to_string());

                let sel = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Agent Status")
                    .items(&items)
                    .default(0)
                    .interact_opt()
                    .ok()
                    .flatten();
                match sel {
                    Some(i) if i < rows.len() => {
                        let (k, v) = &rows[i];
                        println!("\n  {} = {}\n", k.bold().cyan(), v);
                    }
                    _ => break,
                }
            }
        }
        Some("status") => {
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
    use dialoguer::{Select, theme::ColorfulTheme};

    let devices = gestura_core::list_audio_input_devices();
    let mic_available = gestura_core::is_microphone_available();

    if devices.is_empty() {
        println!(
            "Microphone available: {}",
            if mic_available {
                "✓ yes".green()
            } else {
                "✗ no".red()
            }
        );
        println!("{}", "No audio input devices found.".dimmed());
        return;
    }

    loop {
        let labels: Vec<String> = devices
            .iter()
            .map(|d| {
                let badge = if d.is_default { " ★ (default)" } else { "" };
                format!("  {}{}", d.name, badge)
            })
            .collect();
        let mut items = labels;
        items.push("← Back to agent".to_string());

        let sel = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!(
                "Audio Devices ({}, mic {})",
                devices.len(),
                if mic_available { "✓" } else { "✗" }
            ))
            .items(&items)
            .default(0)
            .interact_opt()
            .ok()
            .flatten();
        match sel {
            Some(i) if i < devices.len() => {
                let dev = &devices[i];
                println!("\n  {}", "Device Details".bold().cyan());
                println!("  Name:    {}", dev.name);
                println!("  Default: {}", if dev.is_default { "Yes" } else { "No" });
                println!("  Type:    Audio Input\n");
            }
            _ => break,
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
                "  {} Agent sessions: Stored locally in workspace",
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
fn basic_mode_listen_command(listening_enabled: bool) {
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

    println!(
        "Listening mode: {}",
        if listening_enabled {
            "enabled".green()
        } else {
            "disabled".dimmed()
        }
    );
    println!();

    if listening_enabled {
        println!(
            "{}",
            "Tip: press Enter on an empty prompt to record. Use /voice for one-shot recording."
                .dimmed()
        );
    } else {
        println!("{}", "Tip: run /listen to enable voice mode.".dimmed());
    }
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
                println!("{} Usage: /config set <key> <value>", "✗".red());
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
                println!("{} Unknown or read-only config key: {}", "✗".red(), key);
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
fn basic_mode_session_command(args: &[&str], current: &AgentSession) {
    use dialoguer::{Select, theme::ColorfulTheme};

    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None => {
            // Interactive session browser
            match list_sessions_filtered(SessionFilter::All) {
                Ok(sessions) if !sessions.is_empty() => loop {
                    let labels: Vec<String> = sessions
                        .iter()
                        .map(|s| {
                            let marker = if s.id == current.id { "▸" } else { " " };
                            let model = s.model.as_deref().unwrap_or("default");
                            format!(
                                "{} {}  {:>4} msgs  {}  {}",
                                marker,
                                &s.id[..s.id.len().min(8)],
                                s.message_count,
                                model,
                                s.last_active
                                    .with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M")
                            )
                        })
                        .collect();
                    let mut items = labels;
                    items.push("← Back to agent".to_string());

                    let sel = Select::with_theme(&ColorfulTheme::default())
                        .with_prompt("Sessions")
                        .items(&items)
                        .default(0)
                        .interact_opt()
                        .ok()
                        .flatten();
                    match sel {
                        Some(i) if i < sessions.len() => {
                            let s = &sessions[i];
                            let is_current = s.id == current.id;
                            println!("\n  {}", "Session Details".bold().cyan());
                            println!("  ID:          {}", s.id);
                            println!("  Model:       {}", s.model.as_deref().unwrap_or("default"));
                            println!("  Messages:    {}", s.message_count);
                            println!(
                                "  Created:     {}",
                                s.created_at
                                    .with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M")
                            );
                            println!(
                                "  Last active: {}",
                                s.last_active
                                    .with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M")
                            );
                            if is_current {
                                println!("  (current session)");
                            }
                            println!();
                        }
                        _ => break,
                    }
                },
                Ok(_) => println!("{}", "No sessions found.".dimmed()),
                Err(e) => println!("{} Failed to list sessions: {}", "✗".red(), e),
            }
        }
        Some("info") => {
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
            match store.list(gestura_core::agent_sessions::SessionFilter::All) {
                Ok(sessions) => {
                    if sessions.is_empty() {
                        println!("{}", "No sessions found.".dimmed());
                    } else {
                        println!("{}", "Agent Sessions".bold().cyan());
                        println!("{}", "═".repeat(60));
                        println!(
                            "{:38} {:6} {}",
                            "SESSION ID".underline(),
                            "MSGS".underline(),
                            "LAST ACTIVE".underline()
                        );
                        for info in sessions.iter().take(20) {
                            let active_str = {
                                let elapsed =
                                    chrono::Utc::now().signed_duration_since(info.last_active);
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
                    println!("  → [{:?}]: {}", entity.entity_type, entity.value);
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
            println!("  Confidence: {}%", (analysis.confidence * 100.0) as u32);
        }
        Some("categories") => {
            println!("{}", "Context Categories".bold().cyan());
            println!("{}", "═".repeat(50));
            println!();
            let categories = [
                (
                    ContextCategory::FileSystem,
                    "File system operations (read, write, edit)",
                ),
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
                (ContextCategory::Task, "Task management for current session"),
                (
                    ContextCategory::Screen,
                    "Screen capture and recording (screenshot, screen_record)",
                ),
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
        ContextCategory::Task => "✅",
        ContextCategory::Screen => "🎥",
        ContextCategory::General => "💬",
    }
}

/// Basic mode `/model` slash command handler.
///
/// Returns a canonical [`SessionLlmConfig`] if the user changed the session model.
fn basic_mode_model_command(
    args: &[&str],
    config: &AppConfig,
    session: &AgentSession,
) -> Option<SessionLlmConfig> {
    if args.is_empty() {
        // Show current effective provider/model info.
        let (_, effective) = llm_overrides::apply_basic_mode_session_llm_overrides(config, session);
        println!("{}", "Active Model".bold().cyan());
        println!("{}", "═".repeat(40));
        println!("  {} {}", "Provider:".dimmed(), effective.provider);
        println!("  {} {}", "Model:".dimmed(), effective.model);
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

    // Parse spec — supports `provider:model`, provider-only, or model-only.
    let (provider, model) = if let Some((p, m)) = spec.split_once(':') {
        let p = p.trim().to_string();
        let m = m.trim().to_string();
        if m.is_empty() {
            (p, None)
        } else {
            (p, Some(m))
        }
    } else if llm_overrides::is_known_llm_provider(spec) {
        (spec.to_ascii_lowercase(), None)
    } else {
        // Model-only — infer provider when possible, else keep current primary.
        let inferred = gestura_core::llm_validation::infer_provider_from_model_id(spec)
            .map(|p| p.to_string())
            .unwrap_or_else(|| config.llm.primary.clone());
        (inferred, Some(spec.to_string()))
    };

    let resolved = if let Some(model) = model {
        if let Err(err) =
            gestura_core::llm_validation::validate_model_for_provider(&provider, &model)
        {
            println!("{} {err}", "✗".red());
            return None;
        }
        SessionLlmConfig {
            provider: Some(provider.clone()),
            model: Some(model.clone()),
        }
    } else {
        // Provider-only: resolve the provider's default model via core overrides.
        let mut tmp = config.clone();
        let provider_only = SessionLlmConfig {
            provider: Some(provider.clone()),
            model: None,
        };
        let effective =
            llm_overrides::apply_cli_session_llm_overrides(&mut tmp, Some(&provider_only));
        if effective.model.trim().is_empty() {
            println!(
                "{} Could not resolve a default model for provider '{provider}'",
                "✗".red()
            );
            return None;
        }
        SessionLlmConfig {
            provider: Some(effective.provider),
            model: Some(effective.model),
        }
    };

    let provider_disp = resolved.provider.as_deref().unwrap_or("");
    let model_disp = resolved.model.as_deref().unwrap_or("");
    println!(
        "{} Model set to {} ({})",
        "✓".green(),
        model_disp.cyan(),
        provider_disp.dimmed()
    );

    Some(resolved)
}

/// Basic mode `/hooks` slash command handler — interactive browser.
fn basic_mode_hooks_command(config: &AppConfig) {
    use dialoguer::{Select, theme::ColorfulTheme};

    let hooks = &config.hooks;
    println!(
        "Hooks: {} | Timeout: {}ms | Max output: {} bytes",
        if hooks.enabled {
            "enabled".green().to_string()
        } else {
            "disabled".red().to_string()
        },
        hooks.timeout_ms,
        hooks.max_output_bytes
    );

    if !hooks.allowed_programs.is_empty() {
        println!(
            "Allowed programs: {}",
            hooks.allowed_programs.join(", ").cyan()
        );
    }

    if hooks.hooks.is_empty() {
        println!("{}", "No hooks configured.".dimmed());
        return;
    }

    loop {
        let labels: Vec<String> = hooks
            .hooks
            .iter()
            .map(|h| {
                format!(
                    "  {:<20} {:?}  → {} {}",
                    h.name,
                    h.event,
                    h.command.program,
                    h.command.args.join(" ")
                )
            })
            .collect();
        let mut items = labels;
        items.push("← Back to agent".to_string());

        let sel = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Hooks ({})", hooks.hooks.len()))
            .items(&items)
            .default(0)
            .interact_opt()
            .ok()
            .flatten();
        match sel {
            Some(i) if i < hooks.hooks.len() => {
                let h = &hooks.hooks[i];
                println!("\n  {}", "Hook Details".bold().cyan());
                println!("  Name:    {}", h.name);
                println!("  Event:   {:?}", h.event);
                println!("  Program: {}", h.command.program);
                if !h.command.args.is_empty() {
                    println!("  Args:    {}", h.command.args.join(" "));
                }
                println!();
            }
            _ => break,
        }
    }
}

/// Basic mode `/permissions` slash command handler — interactive browser.
fn basic_mode_permissions_command() {
    use dialoguer::{Select, theme::ColorfulTheme};
    use gestura_core::PermissionManager;

    let manager = PermissionManager::new();
    let perms = match manager.list() {
        Ok(p) => p,
        Err(e) => {
            println!("{} Failed to load permissions: {}", "✗".red(), e);
            return;
        }
    };

    if perms.is_empty() {
        println!("{}", "No permissions granted.".dimmed());
        println!(
            "Grant permissions with: {}",
            "/permissions grant <tool> <action>".cyan()
        );
        return;
    }

    loop {
        let labels: Vec<String> = perms
            .iter()
            .map(|p| {
                let scope_str = match &p.scope {
                    gestura_core::PermissionScope::Global => "Global".to_string(),
                    gestura_core::PermissionScope::Path(s) => format!("Path({})", s),
                    gestura_core::PermissionScope::Command(s) => format!("Cmd({})", s),
                };
                let expiry = p
                    .expires_at
                    .map(|e| e.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "never".to_string());
                format!(
                    "  {}:{} [{}] expires {}",
                    p.tool, p.action, scope_str, expiry
                )
            })
            .collect();
        let mut items = labels;
        items.push("← Back to agent".to_string());

        let sel = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Permissions ({})", perms.len()))
            .items(&items)
            .default(0)
            .interact_opt()
            .ok()
            .flatten();
        match sel {
            Some(i) if i < perms.len() => {
                let p = &perms[i];
                println!("\n  {}", "Permission Details".bold().cyan());
                println!("  Tool:       {}", p.tool);
                println!("  Action:     {}", p.action);
                println!("  Scope:      {:?}", p.scope);
                println!("  Granted at: {}", p.granted_at.format("%Y-%m-%d %H:%M"));
                println!(
                    "  Expires:    {}",
                    p.expires_at
                        .map(|e| e.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "never".to_string())
                );
                println!();
            }
            _ => break,
        }
    }
}

/// Basic mode `/tasks` slash command handler — interactive browser.
fn basic_mode_tasks_command(session: &AgentSession, rt: &tokio::runtime::Runtime) {
    use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
    use gestura_core::tasks::{TaskManager, TaskStatus};

    #[derive(Clone)]
    struct Entry {
        id: String,
        name: String,
        description: String,
        status: TaskStatus,
        parent_id: Option<String>,
        source: String,
    }

    fn status_icon(status: TaskStatus) -> &'static str {
        match status {
            TaskStatus::NotStarted => "[ ]",
            TaskStatus::Blocked => "[!]",
            TaskStatus::InProgress => "[/]",
            TaskStatus::Completed => "[x]",
            TaskStatus::Cancelled => "[-]",
        }
    }

    fn next_status(status: TaskStatus) -> &'static str {
        match status {
            TaskStatus::NotStarted => "in_progress",
            TaskStatus::Blocked => "in_progress",
            TaskStatus::InProgress => "completed",
            TaskStatus::Completed => "cancelled",
            TaskStatus::Cancelled => "not_started",
        }
    }

    let theme = ColorfulTheme::default();
    let task_manager = TaskManager::new(dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")));

    let run_canonical = |args: &[&str]| {
        match slash::run_tasks_subcommand(
            args,
            &task_manager,
            &session.id,
            session.workspace_dir().map(|path| path.as_path()),
        ) {
            Ok(out) => {
                let lines = match out.live_action {
                    Some(act) => match slash::execute_tasks_live_action(rt, act) {
                        Ok(lines) => lines,
                        Err(e) => {
                            println!("{} {e}", "✗".red());
                            Vec::new()
                        }
                    },
                    None => out.lines,
                };
                for line in lines {
                    println!("{line}");
                }
            }
            Err(e) => {
                println!("{} {e}", "✗".red());
                // Print usage to guide recovery.
                if let Ok(out) = slash::run_tasks_subcommand(
                    &["help"],
                    &task_manager,
                    &session.id,
                    session.workspace_dir().map(|path| path.as_path()),
                ) {
                    for line in out.lines {
                        println!("{line}");
                    }
                }
            }
        }
    };

    let prompt_create_name = || -> Option<String> {
        loop {
            let input: String = Input::with_theme(&theme)
                .with_prompt("Task name (single token)")
                .interact_text()
                .ok()?;
            let name = input.trim();
            if name.is_empty() {
                println!("{} Name cannot be empty.", "✗".red());
                continue;
            }
            if name.split_whitespace().count() != 1 {
                println!(
                    "{} Create requires a single-token name (no spaces).",
                    "✗".red()
                );
                continue;
            }
            return Some(name.to_string());
        }
    };

    loop {
        let hierarchy = match task_manager.get_hierarchy(&session.id) {
            Ok(h) => h,
            Err(e) => {
                println!("{} Failed to load tasks: {}", "✗".red(), e);
                return;
            }
        };

        let current_task_id = task_manager.get_current_task_id(&session.id).ok().flatten();

        // Flatten hierarchy into UI entries (root tasks then subtasks).
        let mut entries: Vec<Entry> = Vec::new();
        for (root, subtasks) in &hierarchy {
            entries.push(Entry {
                id: root.id.clone(),
                name: root.name.clone(),
                description: root.description.clone(),
                status: root.status,
                parent_id: None,
                source: format!("{:?}", root.source),
            });
            for sub in subtasks {
                entries.push(Entry {
                    id: sub.id.clone(),
                    name: sub.name.clone(),
                    description: sub.description.clone(),
                    status: sub.status,
                    parent_id: sub.parent_id.clone(),
                    source: format!("{:?}", sub.source),
                });
            }
        }

        // Main menu: tasks + actions.
        let mut items: Vec<String> = Vec::new();
        if entries.is_empty() {
            items.push("(no tasks yet)".dimmed().to_string());
        } else {
            items.extend(entries.iter().map(|e| {
                let indent = if e.parent_id.is_some() { "    " } else { "  " };
                let cur = if current_task_id.as_deref() == Some(e.id.as_str()) {
                    " (current)"
                } else {
                    ""
                };
                format!(
                    "{}{} {}{} [{}]",
                    indent,
                    status_icon(e.status),
                    e.name,
                    cur,
                    e.source
                )
            }));
        }

        let create_idx = items.len();
        items.push("＋ Create new task".to_string());

        let clear_current_idx = items.len();
        if current_task_id.is_some() {
            items.push("⨯ Clear current task".to_string());
        }

        items.push("← Back to agent".to_string());

        let sel = Select::with_theme(&theme)
            .with_prompt("Tasks")
            .items(&items)
            .default(0)
            .interact_opt()
            .ok()
            .flatten();

        let Some(sel) = sel else {
            break;
        };

        if sel == create_idx {
            let Some(name) = prompt_create_name() else {
                continue;
            };
            let desc: String = Input::with_theme(&theme)
                .with_prompt("Description (optional)")
                .allow_empty(true)
                .interact_text()
                .unwrap_or_default();

            let mut args: Vec<String> = vec!["create".to_string(), name];
            if !desc.trim().is_empty() {
                args.extend(desc.split_whitespace().map(|s| s.to_string()));
            }
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            println!();
            run_canonical(&arg_refs);
            println!();
            continue;
        }

        if current_task_id.is_some() && sel == clear_current_idx {
            println!();
            run_canonical(&["current", "clear"]);
            println!();
            continue;
        }

        // Back to agent.
        if sel >= items.len() - 1 {
            break;
        }

        // If there are no tasks, ignore selection.
        if entries.is_empty() {
            continue;
        }

        // Selected a task entry.
        if sel >= entries.len() {
            continue;
        }
        let entry = entries[sel].clone();

        println!("\n  {}", "Task Details".bold().cyan());
        println!("  Name:        {}", entry.name);
        println!("  Status:      {}", status_icon(entry.status));
        println!("  Source:      {}", entry.source);
        println!("  ID:          {}", &entry.id[..entry.id.len().min(8)]);
        if let Some(pid) = entry.parent_id.as_deref() {
            println!("  Parent:      {}", &pid[..pid.len().min(8)]);
        }
        if !entry.description.is_empty() {
            println!("  Description: {}", entry.description);
        }
        if current_task_id.as_deref() == Some(entry.id.as_str()) {
            println!("  Current:     {}", "yes".green());
        }
        println!();

        let actions = [
            "← Back",
            "Cycle status",
            "Set as current",
            "Edit name",
            "Edit description",
            "Create subtask",
            "Add dependency (blocked by)",
            "Delete task",
        ];

        let action = Select::with_theme(&theme)
            .with_prompt("Action")
            .items(&actions)
            .default(0)
            .interact_opt()
            .ok()
            .flatten();

        let Some(action) = action else {
            continue;
        };

        match action {
            0 => {}
            1 => {
                println!();
                run_canonical(&["status", entry.id.as_str(), next_status(entry.status)]);
                println!();
            }
            2 => {
                println!();
                run_canonical(&["current", "set", entry.id.as_str()]);
                println!();
            }
            3 => {
                let new_name: String = Input::with_theme(&theme)
                    .with_prompt("New name")
                    .interact_text()
                    .unwrap_or_default();
                if new_name.trim().is_empty() {
                    println!("{} Name cannot be empty.", "✗".red());
                    continue;
                }
                let mut args: Vec<String> =
                    vec!["update".to_string(), entry.id.clone(), "name".to_string()];
                args.extend(new_name.split_whitespace().map(|s| s.to_string()));
                let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                println!();
                run_canonical(&arg_refs);
                println!();
            }
            4 => {
                let new_desc: String = Input::with_theme(&theme)
                    .with_prompt("New description")
                    .interact_text()
                    .unwrap_or_default();
                if new_desc.trim().is_empty() {
                    println!("{} Description cannot be empty.", "✗".red());
                    continue;
                }
                let mut args: Vec<String> =
                    vec!["update".to_string(), entry.id.clone(), "desc".to_string()];
                args.extend(new_desc.split_whitespace().map(|s| s.to_string()));
                let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                println!();
                run_canonical(&arg_refs);
                println!();
            }
            5 => {
                let Some(sub_name) = prompt_create_name() else {
                    continue;
                };
                let sub_desc: String = Input::with_theme(&theme)
                    .with_prompt("Description (optional)")
                    .allow_empty(true)
                    .interact_text()
                    .unwrap_or_default();

                let mut args: Vec<String> =
                    vec!["create-sub".to_string(), entry.id.clone(), sub_name];
                if !sub_desc.trim().is_empty() {
                    args.extend(sub_desc.split_whitespace().map(|s| s.to_string()));
                }
                let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                println!();
                run_canonical(&arg_refs);
                println!();
            }
            6 => {
                let tasks = match task_manager.list_tasks(&session.id) {
                    Ok(t) => t,
                    Err(e) => {
                        println!("{} Failed to list tasks: {e}", "✗".red());
                        continue;
                    }
                };
                let mut candidates: Vec<(String, String)> = tasks
                    .into_iter()
                    .filter(|t| t.id != entry.id)
                    .map(|t| (t.id, t.name))
                    .collect();
                if candidates.is_empty() {
                    println!("{} No other tasks available to depend on.", "ℹ".cyan());
                    continue;
                }
                candidates.sort_by(|a, b| a.1.cmp(&b.1));

                let labels: Vec<String> = candidates
                    .iter()
                    .map(|(tid, tname)| format!("  {} ({})", tname, &tid[..tid.len().min(8)]))
                    .collect();
                let dep_sel = Select::with_theme(&theme)
                    .with_prompt("Blocked by")
                    .items(&labels)
                    .default(0)
                    .interact_opt()
                    .ok()
                    .flatten();
                let Some(dep_sel) = dep_sel else {
                    continue;
                };
                let blocked_by_id = candidates[dep_sel].0.clone();
                println!();
                run_canonical(&["dep", "add", entry.id.as_str(), blocked_by_id.as_str()]);
                println!();
            }
            7 => {
                let ok = Confirm::with_theme(&theme)
                    .with_prompt("Delete this task? This cannot be undone.")
                    .default(false)
                    .interact()
                    .unwrap_or(false);
                if !ok {
                    continue;
                }
                println!();
                run_canonical(&["delete", "--confirmed", entry.id.as_str()]);
                println!();
            }
            _ => {}
        }
    }
}

/// Basic mode `/theme` slash command handler — interactive browser.
fn basic_mode_themes_command() {
    use dialoguer::{Select, theme::ColorfulTheme};

    // Use the same stable keys as `Theme::available_themes()`.
    let themes = ["catppuccin", "high-contrast", "dracula", "gestura", "pro"];

    loop {
        let labels: Vec<String> = themes.iter().map(|t| format!("  {}", t)).collect();
        let mut items = labels;
        items.push("← Back to agent".to_string());

        let sel = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Themes")
            .items(&items)
            .default(0)
            .interact_opt()
            .ok()
            .flatten();
        match sel {
            Some(i) if i < themes.len() => {
                println!(
                    "\n  {} Theme '{}' selected. Use TUI mode for live theme switching.\n",
                    "ℹ".cyan(),
                    themes[i]
                );
            }
            _ => break,
        }
    }
}

// ── Knowledge store (G6) ────────────────────────────────────────────────────
// Module-level singletons so the pipeline can always be wired with knowledge,
// regardless of which code path constructs it.  The TUI sub-module accesses
// these via `super::get_knowledge_store()` / `super::get_knowledge_settings()`.

/// Global knowledge store for all CLI agent pipelines.
static KNOWLEDGE_STORE: OnceLock<gestura_core::KnowledgeStore> = OnceLock::new();

/// Global knowledge settings manager for all CLI agent pipelines.
static KNOWLEDGE_SETTINGS: OnceLock<gestura_core::KnowledgeSettingsManager> = OnceLock::new();

/// Get or initialize the module-level knowledge store.
pub(super) fn get_knowledge_store() -> &'static gestura_core::KnowledgeStore {
    KNOWLEDGE_STORE.get_or_init(|| {
        let store = gestura_core::KnowledgeStore::with_default_dir();
        gestura_core::register_builtin_knowledge(&store);
        if let Err(e) = store.load_user_items() {
            tracing::warn!(error = %e, "Failed to load persisted user knowledge (continuing)");
        }
        store
    })
}

/// Get or initialize the module-level knowledge settings manager.
pub(super) fn get_knowledge_settings() -> &'static gestura_core::KnowledgeSettingsManager {
    KNOWLEDGE_SETTINGS.get_or_init(|| {
        let base_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        gestura_core::KnowledgeSettingsManager::new(base_dir)
    })
}
