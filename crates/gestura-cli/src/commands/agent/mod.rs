#![allow(clippy::type_complexity)]
//! Interactive agent command

use super::Result;
use colored::Colorize;
use gestura_core::config::AgentTelemetryTraceExportProtocol;
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
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::sync::mpsc;

mod catalog;
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
    pub prompt: Option<&'a str>,
    pub prompt_file: Option<&'a Path>,
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

pub(super) fn effective_session_reflection_enabled(
    config: &AppConfig,
    session: &AgentSession,
) -> bool {
    gestura_core::agent_sessions::effective_session_reflection_enabled(&session.state, config)
}

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

fn resolve_session_active_task_id(session: &AgentSession) -> Option<String> {
    let session_task_id = session
        .state
        .working_memory
        .active_task_id
        .as_deref()
        .map(str::trim)
        .filter(|task_id| !task_id.is_empty())
        .map(str::to_string);

    let session_id = session.id.trim();
    if session_id.is_empty() {
        return session_task_id;
    }

    gestura_core::get_global_task_manager()
        .get_current_task_id(session_id)
        .ok()
        .flatten()
        .map(|task_id| task_id.trim().to_string())
        .filter(|task_id| !task_id.is_empty())
        .or(session_task_id)
}

fn sync_session_active_task_id(session: &mut AgentSession) -> bool {
    let resolved = resolve_session_active_task_id(session);
    if session.state.working_memory.active_task_id == resolved {
        return false;
    }

    session.state.working_memory.active_task_id = resolved;
    true
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

enum BasicModeCommandOutcome {
    Continue,
    Break,
    Submit {
        input: String,
        input_source: MessageSource,
    },
}

struct BasicModeRuntime<'a> {
    model_hint: Option<&'a str>,
    config: &'a mut AppConfig,
    agent_session: &'a mut AgentSession,
    voice: &'a mut bool,
    system_prompt: Option<&'a str>,
    rt: &'a tokio::runtime::Runtime,
}

impl BasicModeRuntime<'_> {
    fn run_input(&mut self, input: String, input_source: MessageSource) -> Result<bool> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }

        let mut input = trimmed.to_string();
        let mut input_source = input_source;

        if let Some(rest) = input.strip_prefix("/exec ") {
            input = rest.to_string();
        }

        if let Some(outcome) = handle_basic_mode_slash_command(
            input.clone(),
            input_source,
            self.model_hint,
            self.config,
            self.agent_session,
            self.voice,
            self.rt,
        )? {
            match outcome {
                BasicModeCommandOutcome::Continue => return Ok(false),
                BasicModeCommandOutcome::Break => return Ok(true),
                BasicModeCommandOutcome::Submit {
                    input: next_input,
                    input_source: next_source,
                } => {
                    input = next_input;
                    input_source = next_source;
                }
            }
        }

        execute_basic_mode_turn(
            self.agent_session,
            input,
            input_source,
            self.config,
            self.system_prompt,
            self.rt,
        )?;
        Ok(false)
    }
}

fn run_basic_mode(opts: AgentOptions<'_>) -> Result<()> {
    let AgentOptions {
        model,
        resume,
        session,
        prompt,
        prompt_file,
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

    // Create tokio runtime for async LLM calls
    let rt = tokio::runtime::Runtime::new()?;

    if let Some(prompt) = load_one_shot_prompt(prompt, prompt_file)? {
        return run_basic_mode_one_shot(
            model,
            &mut config,
            &mut agent_session,
            &mut voice,
            system_prompt.as_deref(),
            prompt,
            &rt,
        );
    }

    if !io::stdin().is_terminal() {
        return run_basic_mode_noninteractive(
            model,
            &mut config,
            &mut agent_session,
            &mut voice,
            system_prompt.as_deref(),
            &rt,
        );
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

                if let Some(outcome) = handle_basic_mode_slash_command(
                    input.clone(),
                    input_source,
                    model,
                    &mut config,
                    &mut agent_session,
                    &mut voice,
                    &rt,
                )? {
                    match outcome {
                        BasicModeCommandOutcome::Continue => continue,
                        BasicModeCommandOutcome::Break => break,
                        BasicModeCommandOutcome::Submit {
                            input: next_input,
                            input_source: next_source,
                        } => {
                            input = next_input;
                            input_source = next_source;
                        }
                    }
                }

                execute_basic_mode_turn(
                    &mut agent_session,
                    input,
                    input_source,
                    &config,
                    system_prompt.as_deref(),
                    &rt,
                )?;
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

fn load_one_shot_prompt(
    prompt: Option<&str>,
    prompt_file: Option<&Path>,
) -> Result<Option<String>> {
    match (prompt, prompt_file) {
        (Some(prompt), None) => Ok(Some(prompt.to_string())),
        (None, Some(path)) => fs::read_to_string(path).map(Some).map_err(|error| {
            format!("Failed to read prompt file {}: {error}", path.display()).into()
        }),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Err("--prompt and --prompt-file cannot be used together".into()),
    }
}

fn run_basic_mode_one_shot(
    model_hint: Option<&str>,
    config: &mut AppConfig,
    agent_session: &mut AgentSession,
    voice: &mut bool,
    system_prompt: Option<&str>,
    prompt: String,
    rt: &tokio::runtime::Runtime,
) -> Result<()> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Err("One-shot prompt cannot be empty".into());
    }

    let should_exit = BasicModeRuntime {
        model_hint,
        config,
        agent_session,
        voice,
        system_prompt,
        rt,
    }
    .run_input(trimmed.to_string(), MessageSource::Text)?;

    if should_exit {
        return Ok(());
    }

    println!();
    println!("{} {}", "✓".green(), "Session saved. Goodbye!".dimmed());
    save_cli_session(agent_session)?;
    Ok(())
}

fn run_basic_mode_noninteractive(
    model_hint: Option<&str>,
    config: &mut AppConfig,
    agent_session: &mut AgentSession,
    voice: &mut bool,
    system_prompt: Option<&str>,
    rt: &tokio::runtime::Runtime,
) -> Result<()> {
    let mut script = String::new();
    io::stdin().read_to_string(&mut script)?;
    {
        let mut runtime = BasicModeRuntime {
            model_hint,
            config,
            agent_session,
            voice,
            system_prompt,
            rt,
        };

        for raw_line in script.lines() {
            if runtime.run_input(raw_line.to_string(), MessageSource::Text)? {
                return Ok(());
            }
        }
    }

    println!();
    println!("{} {}", "✓".green(), "Session saved. Goodbye!".dimmed());
    save_cli_session(agent_session)?;
    Ok(())
}

fn handle_basic_mode_slash_command(
    mut input: String,
    mut input_source: MessageSource,
    model_hint: Option<&str>,
    config: &mut AppConfig,
    agent_session: &mut AgentSession,
    voice: &mut bool,
    rt: &tokio::runtime::Runtime,
) -> Result<Option<BasicModeCommandOutcome>> {
    if !input.starts_with('/') {
        return Ok(None);
    }

    let mut parts = input.split_whitespace();
    let raw_cmd = parts.next().unwrap_or("").to_ascii_lowercase();
    let cmd = catalog::canonical_command(&raw_cmd);
    let args: Vec<&str> = parts.collect();

    let outcome = match cmd {
        "/quit" => {
            save_cli_session(agent_session)?;
            println!();
            println!(
                "{} {} {}",
                "✓".green(),
                "Session saved.".dimmed(),
                "Goodbye!".cyan()
            );
            println!();
            BasicModeCommandOutcome::Break
        }
        "/voice" => match record_voice_input(rt) {
            Ok(text) if !text.is_empty() => {
                println!("{} {}", "Transcribed:".cyan(), text);
                input = text;
                input_source = MessageSource::Voice;
                BasicModeCommandOutcome::Submit {
                    input,
                    input_source,
                }
            }
            Ok(_) => BasicModeCommandOutcome::Continue,
            Err(e) => {
                eprintln!("{}: {}", "Voice error".red(), e);
                BasicModeCommandOutcome::Continue
            }
        },
        "/help" => {
            print_basic_mode_help();
            BasicModeCommandOutcome::Continue
        }
        "/tools" => {
            println!();
            basic_mode_tools_command(&args, agent_session);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/summarize" => {
            println!();
            summarize_session_history(agent_session);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/memory" => {
            println!();
            basic_mode_memory_command(&args, agent_session);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/clear" => {
            print!("\x1B[2J\x1B[1;1H");
            BasicModeCommandOutcome::Continue
        }
        "/save" => {
            save_cli_session(agent_session)?;
            println!("{} Session saved", "✓".green());
            BasicModeCommandOutcome::Continue
        }
        "/history" => {
            println!();
            print_session_statistics(agent_session);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/new" => {
            save_cli_session(agent_session)?;
            *agent_session = new_cli_session(model_hint.map(String::from))?;
            println!();
            println!(
                "{} {} {}",
                "✓".green(),
                "New session started:".dimmed(),
                agent_session.id
            );
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/mcp" => {
            println!();
            basic_mode_mcp_command(&args);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/a2a" => {
            println!();
            basic_mode_a2a_command(&args);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/knowledge" => {
            println!();
            basic_mode_knowledge_command(&args, agent_session);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/agent" => {
            println!();
            basic_mode_agent_command(&args, config, agent_session);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/device" => {
            println!();
            basic_mode_device_command(&args, config, agent_session);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/health" => {
            println!();
            basic_mode_health_command(config);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/privacy" => {
            println!();
            basic_mode_privacy_command(&args);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/listen" => {
            println!();
            if !*voice {
                if !gestura_core::is_microphone_available() {
                    println!(
                        "{} {}",
                        "✗".red(),
                        "Microphone not available; cannot enable listening mode".dimmed()
                    );
                    *voice = false;
                } else {
                    *voice = true;
                    println!(
                        "{} {}",
                        "🎤".green(),
                        "Listening mode enabled (press Enter on an empty prompt to record)"
                            .dimmed()
                    );
                }
            } else {
                *voice = false;
                println!("{} {}", "🔇".yellow(), "Listening mode disabled".dimmed());
            }

            basic_mode_listen_command(*voice);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/config" => {
            println!();
            basic_mode_config_command(&args);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/session" => {
            println!();
            basic_mode_session_command(&args, agent_session);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/context" => {
            println!();
            basic_mode_context_command(&args);
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/workflow" => {
            println!();
            if let Some(workflow_prompt) = basic_mode_workflow_command(&args) {
                BasicModeCommandOutcome::Submit {
                    input: workflow_prompt,
                    input_source,
                }
            } else {
                println!();
                BasicModeCommandOutcome::Continue
            }
        }
        "/init" => {
            println!();
            match crate::commands::init::run() {
                Ok(()) => println!(),
                Err(error) => println!("{} {}", "✗".red(), error),
            }
            BasicModeCommandOutcome::Continue
        }
        "/model" => {
            println!();
            if let Some(new_llm) = basic_mode_model_command(&args, config, agent_session) {
                let provider = new_llm.provider.clone().unwrap_or_default();
                let model_name = new_llm.model.clone().unwrap_or_default();
                if !provider.trim().is_empty() && !model_name.trim().is_empty() {
                    agent_session.state.llm_config = Some(new_llm);
                    agent_session.model = Some(format!("{}:{}", provider, model_name));
                    save_cli_session(agent_session)?;
                }
            }
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/hooks" => {
            println!();
            if args.is_empty() {
                basic_mode_hooks_command(config);
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
                                println!("{} Failed to save config: {}", "✗".red(), e);
                            } else {
                                *config = cfg;
                            }
                        }
                    }
                    Err(e) => {
                        println!("{} {}", "✗".red(), e);
                        println!();
                        if let Ok(outcome) = slash::apply_hooks_subcommand(&["help"], &mut cfg) {
                            for line in outcome.into_lines() {
                                println!("{line}");
                            }
                        }
                    }
                }
            }
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/permissions" => {
            println!();
            if args.is_empty() {
                basic_mode_permissions_command();
            } else {
                match slash::run_permissions_subcommand(&args, agent_session) {
                    Ok(outcome) => {
                        for line in outcome.lines {
                            println!("{line}");
                        }
                        if outcome.changed_permissions {
                            println!("{} Permissions updated.", "✓".green());
                        }

                        if outcome.session_changed
                            && let Err(e) = save_cli_session(agent_session)
                        {
                            println!("{} Failed to save session: {}", "✗".red(), e);
                        }
                    }
                    Err(e) => {
                        println!("{} {}", "✗".red(), e);
                        println!();
                        if let Ok(outcome) =
                            slash::run_permissions_subcommand(&["help"], agent_session)
                        {
                            for line in outcome.lines {
                                println!("{line}");
                            }
                        }
                    }
                }
            }
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/tasks" => {
            println!();
            if args.is_empty() {
                basic_mode_tasks_command(agent_session, rt);
            } else {
                let task_manager = gestura_core::get_global_task_manager();
                match slash::run_tasks_subcommand(
                    &args,
                    task_manager,
                    &agent_session.id,
                    agent_session.workspace_dir().map(|path| path.as_path()),
                ) {
                    Ok(out) => {
                        let task_context_changed = if out.changed {
                            sync_session_active_task_id(agent_session)
                        } else {
                            false
                        };
                        let lines = match out.live_action {
                            Some(act) => match slash::execute_tasks_live_action(rt, act) {
                                Ok(lines) => lines,
                                Err(e) => {
                                    println!("{} {}", "✗".red(), e);
                                    Vec::new()
                                }
                            },
                            None => out.lines,
                        };
                        for line in lines {
                            println!("{line}");
                        }
                        if task_context_changed && let Err(e) = save_cli_session(agent_session) {
                            println!("{} Failed to save session: {}", "✗".red(), e);
                        }
                    }
                    Err(e) => {
                        println!("{} {}", "✗".red(), e);
                    }
                }
            }
            println!();
            BasicModeCommandOutcome::Continue
        }
        "/theme" => {
            println!();
            basic_mode_themes_command();
            println!();
            BasicModeCommandOutcome::Continue
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
            BasicModeCommandOutcome::Continue
        }
    };

    Ok(Some(outcome))
}

fn execute_basic_mode_turn(
    agent_session: &mut AgentSession,
    input: String,
    input_source: MessageSource,
    config: &AppConfig,
    system_prompt: Option<&str>,
    rt: &tokio::runtime::Runtime,
) -> Result<()> {
    agent_session.add_user_message(&input, input_source);

    if input.trim().starts_with("/tools") {
        let parts: Vec<&str> = input.split_whitespace().collect();
        println!();
        basic_mode_tools_command(&parts[1..], agent_session);
        println!();
        return Ok(());
    }

    if input.trim().starts_with("/summarize") {
        println!();
        summarize_session_history(agent_session);
        println!();
        return Ok(());
    }

    let history: Vec<gestura_core::Message> = agent_session.to_pipeline_messages_limited(10);

    let mut request = AgentRequest::new(&input)
        .with_streaming(true)
        .with_source(RequestSource::CliBasic)
        .with_session(agent_session.id.clone())
        .with_history(history);

    if let Some(workspace) = agent_session.workspace_dir() {
        request = request.with_workspace(workspace.clone());
    }

    if let Some(sys) = system_prompt {
        request = request.with_system_prompt(sys.to_string());
    }

    let (config_for_pipeline, effective) =
        llm_overrides::apply_basic_mode_session_llm_overrides(config, agent_session);
    let provider_name = effective.provider;
    let model_name = effective.model;
    let (permission_level, allowed_tools) = derive_request_policy(agent_session);
    let active_task_id = resolve_session_active_task_id(agent_session);
    if sync_session_active_task_id(agent_session) {
        let _ = save_cli_session(agent_session);
    }
    request = request
        .with_session_llm_config(provider_name, model_name)
        .with_reflection_enabled(effective_session_reflection_enabled(config, agent_session))
        .with_permission_level(permission_level);
    if let Some(task_id) = active_task_id {
        request = request.with_task(task_id);
    }
    if !allowed_tools.is_empty() {
        request = request.with_allowed_tools(allowed_tools);
    }

    println!();
    println!("{}", "◆".blue().bold());
    print!("  ");
    let _ = std::io::stdout().flush();

    let session_id_for_tool_confirm = agent_session.id.clone();
    let config_clone = config_for_pipeline;
    let response: Result<(
        gestura_core::AgentResponse,
        Vec<gestura_core::agent_sessions::SessionToolCall>,
        Vec<(String, String)>,
    )> = rt.block_on(async move {
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
        let mut current_tool_call: Option<(String, String, String)> = None;
        let mut completed_tool_calls = Vec::new();
        let mut tool_result_messages = Vec::new();
        while let Some(chunk) = rx.recv().await {
            match chunk {
                StreamChunk::Status { message } => {
                    println!();
                    println!("  {} {}", "ℹ".cyan(), message.dimmed());
                    print!("  ");
                    let _ = std::io::stdout().flush();
                }
                StreamChunk::Narration { narration, .. } => {
                    println!();
                    println!("  {} {}", "◇".cyan(), narration.message.dimmed());
                    print!("  ");
                    let _ = std::io::stdout().flush();
                }
                StreamChunk::Text(t) => {
                    let rendered = t.replace("\n", "\n  ");
                    print!("{rendered}");
                    let _ = std::io::stdout().flush();
                }
                StreamChunk::Thinking(_) => {}
                StreamChunk::TaskRuntimeSnapshot { snapshot } => {
                    println!();
                    println!("  {} {}", "☰".cyan(), snapshot.status_message.dimmed());
                    if let Some(current_task) = snapshot.current_task {
                        println!(
                            "  {} current: {} [{}]",
                            "•".cyan(),
                            current_task.name.dimmed(),
                            current_task.status
                        );
                    }
                    print!("  ");
                    let _ = std::io::stdout().flush();
                }
                StreamChunk::ToolCallStart { id, name } => {
                    current_tool_call = Some((id, name.clone(), String::new()));
                    println!();
                    println!("  {} {}", "→".cyan(), format!("tool: {name}").dimmed());
                    print!("  ");
                    let _ = std::io::stdout().flush();
                }
                StreamChunk::ToolCallEnd => {}
                StreamChunk::ToolCallArgs(args) => {
                    if let Some((_, _, ref mut acc)) = current_tool_call {
                        acc.push_str(&args);
                    }
                }
                StreamChunk::ToolCallResult {
                    name,
                    success,
                    output,
                    duration_ms,
                } => {
                    if let Some((tool_call_id, tool_name, arguments)) = current_tool_call.take() {
                        completed_tool_calls.push(gestura_core::agent_sessions::SessionToolCall {
                            id: tool_call_id.clone(),
                            name: tool_name,
                            arguments,
                            result: output.clone(),
                            success,
                            duration_ms,
                            timestamp: chrono::Utc::now(),
                        });
                        if !output.trim().is_empty() {
                            tool_result_messages.push((tool_call_id, output.clone()));
                        }
                    }
                    if success {
                        println!("  {} {} ({}ms)", "✓".green(), name.dimmed(), duration_ms);
                        if !output.is_empty() {
                            let formatted_output = format_tool_output(&output);
                            println!("{}", formatted_output.dimmed());
                        }
                    } else {
                        println!("  {} {} failed ({}ms):", "✗".red(), name, duration_ms);
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
                        println!("  {} Failed to resolve confirmation: {}", "✗".red(), err);
                    }

                    print!("  ");
                    let _ = std::io::stdout().flush();
                }
                StreamChunk::ToolBlocked { tool_name, reason } => {
                    println!();
                    println!("  {} Tool '{}' blocked: {}", "🚫".red(), tool_name, reason);
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
                StreamChunk::ShellSessionLifecycle {
                    shell_session_id,
                    state,
                    cwd,
                    active_command,
                    available_for_reuse,
                    ..
                } => {
                    println!();
                    let cwd = cwd.unwrap_or_else(|| "<unknown cwd>".to_string());
                    let reuse = if available_for_reuse {
                        "reusable"
                    } else {
                        "reserved"
                    };
                    if let Some(command) = active_command {
                        println!(
                            "  {} shell session {} {:?} ({}, cwd: {}, active: {})",
                            "🖥".dimmed(),
                            shell_session_id.dimmed(),
                            state,
                            reuse.dimmed(),
                            cwd.dimmed(),
                            command.dimmed()
                        );
                    } else {
                        println!(
                            "  {} shell session {} {:?} ({}, cwd: {})",
                            "🖥".dimmed(),
                            shell_session_id.dimmed(),
                            state,
                            reuse.dimmed(),
                            cwd.dimmed()
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

        let agent_response = stream_task
            .await
            .map_err(|e| std::io::Error::other(format!("Streaming task failed: {e}")))??;
        if !saw_done {
            // The channel can close without an explicit Done; still return whatever we have.
        }
        Ok((agent_response, completed_tool_calls, tool_result_messages))
    });

    match response {
        Ok((agent_response, completed_tool_calls, tool_result_messages)) => {
            println!();
            if let Some(usage) = &agent_response.usage {
                println!(
                    "  {} tokens: {} in / {} out",
                    "ℹ".dimmed(),
                    usage.input_tokens.to_string().dimmed(),
                    usage.output_tokens.to_string().dimmed()
                );
            }

            for tool_call in completed_tool_calls {
                agent_session.state.record_tool_call(tool_call);
            }
            for (tool_call_id, content) in tool_result_messages {
                agent_session
                    .state
                    .add_tool_message(&tool_call_id, &content);
            }
            agent_session.add_assistant_message(&agent_response.content, agent_response.thinking);
            let _ = save_cli_session(agent_session);
        }
        Err(e) => {
            println!();
            println!("{} {} {}", "✗".red(), "Error:".red(), e);
        }
    }
    println!();

    if sync_session_active_task_id(agent_session) {
        let _ = save_cli_session(agent_session);
    }

    if agent_session.message_count().is_multiple_of(5) {
        let _ = save_cli_session(agent_session);
    }

    Ok(())
}

fn print_session_statistics(agent_session: &AgentSession) {
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
    println!(
        "{}",
        "╭─ Session Statistics ─────────────────────────────────────────╮".dimmed()
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
        "╰───────────────────────────────────────────────────────────────╯".dimmed()
    );
}

fn summarize_session_history(agent_session: &mut AgentSession) {
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
        return;
    }

    use gestura_core::context::ContextManager;
    let context_manager = ContextManager::new();
    let summary = context_manager.summarize_history(&history);

    println!("{} {}", "◆".blue().bold(), "Conversation Summary:".blue());
    println!();
    println!("{summary}");
    println!();
    println!(
        "{}",
        format!("Summarized {} messages", history.len()).dimmed()
    );

    agent_session.add_assistant_message(
        &format!(
            "## Conversation Summary\n\n{}\n\n---\n\n*Summarized {} messages*",
            summary,
            history.len()
        ),
        Some("Summarizing conversation history (no LLM call)...".to_string()),
    );
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
            for line in slash::a2a_status_lines() {
                println!("{line}");
            }
        }
        Some("profiles") => {
            for line in slash::a2a_profiles_lines() {
                println!("{line}");
            }
        }
        Some("agents") => {
            for line in slash::a2a_agents_lines() {
                println!("{line}");
            }
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

fn dispatch_basic_mode_managed_command(
    command: &str,
    config: &AppConfig,
    session: &AgentSession,
) -> bool {
    let mut parts = command.split_whitespace();
    let Some(cmd) = parts.next() else {
        return false;
    };
    let args: Vec<&str> = parts.collect();
    match cmd {
        "/agent" => {
            basic_mode_agent_command(&args, config, session);
            true
        }
        "/device" => {
            basic_mode_device_command(&args, config, session);
            true
        }
        "/knowledge" => {
            basic_mode_knowledge_command(&args, session);
            true
        }
        "/config" => {
            basic_mode_config_command(&args);
            true
        }
        "/model" => {
            let _ = basic_mode_model_command(&args, config, session);
            true
        }
        _ => false,
    }
}

fn basic_mode_managed_browser(
    prompt: &str,
    entries: &[tui::ManagedCommandEntry],
    config: &AppConfig,
    session: &AgentSession,
) {
    use dialoguer::{Select, theme::ColorfulTheme};
    use tui::ManagedCommandAction;

    loop {
        let labels: Vec<String> = entries
            .iter()
            .map(|entry| format!("{:<24} {}", entry.title, entry.summary))
            .collect();
        let mut items = labels;
        items.push("← Back to agent".to_string());

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(prompt)
            .items(&items)
            .default(0)
            .interact_opt()
            .ok()
            .flatten();

        let Some(idx) = selection else {
            break;
        };
        if idx >= entries.len() {
            break;
        }

        let entry = &entries[idx];
        println!();
        println!("{}", entry.title.bold().cyan());
        println!("{}", "─".repeat(60));
        for line in &entry.detail {
            if line.is_empty() {
                println!();
            } else {
                println!("  {line}");
            }
        }
        println!();
        println!("  {} {}", "Command:".dimmed(), entry.command.cyan());

        let (action_items, show_index) = match &entry.action {
            ManagedCommandAction::Execute(_) => (
                vec!["Run command", "Show command only", "← Back to list"],
                1,
            ),
            ManagedCommandAction::Prefill(_) => {
                (vec!["Show suggested command", "← Back to list"], 0)
            }
            ManagedCommandAction::Confirm { .. } => (
                vec!["Show command requiring confirmation", "← Back to list"],
                0,
            ),
        };

        let action = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Action")
            .items(&action_items)
            .default(0)
            .interact_opt()
            .ok()
            .flatten();

        match (&entry.action, action) {
            (ManagedCommandAction::Execute(command), Some(0)) => {
                if !dispatch_basic_mode_managed_command(command, config, session) {
                    println!("{} Run manually: {}", "ℹ".blue(), command.cyan());
                }
            }
            (ManagedCommandAction::Execute(command), Some(idx)) if idx == show_index => {
                println!("{} {}", "Suggested command:".dimmed(), command.cyan());
            }
            (ManagedCommandAction::Prefill(command), Some(0))
            | (ManagedCommandAction::Confirm { command, .. }, Some(0)) => {
                println!("{} {}", "Suggested command:".dimmed(), command.cyan());
            }
            _ => {}
        }
    }
}

/// Basic mode `/knowledge` slash command handler.
///
/// Uses [`KnowledgeSettingsManager`] for session-scoped enable/disable persistence.
fn basic_mode_knowledge_command(args: &[&str], session: &AgentSession) {
    use dialoguer::{Input, Select, theme::ColorfulTheme};

    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    if subcommand.is_some() {
        match slash::run_knowledge_subcommand(args, session) {
            Ok(lines) => {
                for line in lines {
                    println!("{line}");
                }
            }
            Err(error) => println!("{} {}", "✗".red(), error),
        }
        return;
    }

    let settings_mgr = gestura_core::knowledge::KnowledgeSettingsManager::new(
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
    );
    let session_id = &session.id;
    let mut category_filter: Option<String> = None;
    let mut search_query = String::new();

    loop {
        let items = slash::load_session_knowledge_items(session_id);
        if items.is_empty() {
            println!("{}", slash::knowledge_empty_message());
            return;
        }

        let mut categories: Vec<String> = items.iter().map(|item| item.category.clone()).collect();
        categories.sort();
        categories.dedup();

        let lowered_query = search_query.to_ascii_lowercase();
        let visible_indices: Vec<usize> = items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                let category_matches = category_filter
                    .as_deref()
                    .is_none_or(|category| item.category == category);
                let query_matches = lowered_query.is_empty()
                    || item.name.to_ascii_lowercase().contains(&lowered_query)
                    || item
                        .description
                        .to_ascii_lowercase()
                        .contains(&lowered_query)
                    || item
                        .triggers
                        .iter()
                        .any(|trigger| trigger.to_ascii_lowercase().contains(&lowered_query));
                (category_matches && query_matches).then_some(idx)
            })
            .collect();

        let mut menu_items = vec![format!(
            "Filter category: {}",
            category_filter.as_deref().unwrap_or("all")
        )];
        menu_items.push(format!(
            "Search text: {}",
            if search_query.is_empty() {
                "(none)".to_string()
            } else {
                search_query.clone()
            }
        ));
        menu_items.push("Clear filters/search".to_string());
        menu_items.extend(visible_indices.iter().map(|idx| {
            let item = &items[*idx];
            let status = if item.enabled { "✓" } else { "✗" };
            format!(
                "{} {:<24} [{}] {}",
                status,
                item.name,
                item.category,
                item.description.chars().take(42).collect::<String>()
            )
        }));
        menu_items.push("← Back to agent".to_string());

        let prompt = format!(
            "Knowledge Base ({} shown of {})",
            visible_indices.len(),
            items.len()
        );
        let Some(selection) = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(prompt)
            .items(&menu_items)
            .default(0)
            .interact_opt()
            .ok()
            .flatten()
        else {
            break;
        };

        match selection {
            0 => {
                let mut category_items = vec!["All categories".to_string()];
                category_items.extend(categories.iter().cloned());
                if let Some(choice) = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Choose category filter")
                    .items(&category_items)
                    .default(0)
                    .interact_opt()
                    .ok()
                    .flatten()
                {
                    category_filter = if choice == 0 {
                        None
                    } else {
                        categories.get(choice - 1).cloned()
                    };
                }
            }
            1 => {
                if let Ok(query) = Input::<String>::with_theme(&ColorfulTheme::default())
                    .with_prompt("Search knowledge items")
                    .allow_empty(true)
                    .with_initial_text(search_query.clone())
                    .interact_text()
                {
                    search_query = query.trim().to_string();
                }
            }
            2 => {
                category_filter = None;
                search_query.clear();
            }
            selected if selected == menu_items.len() - 1 => break,
            selected => {
                let idx = visible_indices[selected - 3];
                let item = &items[idx];
                println!();
                for line in slash::knowledge_detail_lines(item, false) {
                    println!("{line}");
                }
                println!();

                let toggle_label = if item.enabled {
                    "Disable this item"
                } else {
                    "Enable this item"
                };
                let actions = [toggle_label, "Show canonical command", "← Back to list"];
                let action = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Action")
                    .items(&actions)
                    .default(0)
                    .interact_opt()
                    .ok()
                    .flatten();

                match action {
                    Some(0) => {
                        let new_enabled = !item.enabled;
                        let _ =
                            settings_mgr.set_knowledge_enabled(session_id, &item.id, new_enabled);
                        println!(
                            "{} Knowledge '{}' {}",
                            "✓".green(),
                            item.name.cyan(),
                            if new_enabled { "enabled" } else { "disabled" }
                        );
                    }
                    Some(1) => println!(
                        "{} {}",
                        "Suggested command:".dimmed(),
                        format!("/knowledge show {}", item.id).cyan()
                    ),
                    _ => {}
                }
            }
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
    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None => basic_mode_managed_browser(
            "Agent Console",
            &slash::agent_browser_entries(config, session),
            config,
            session,
        ),
        Some(_) => match slash::run_agent_subcommand(args, config, session) {
            Ok(lines) => {
                for line in lines {
                    println!("{line}");
                }
            }
            Err(error) => {
                println!("{} {}", "✗".red(), error);
            }
        },
    }
}

/// Basic mode `/device` slash command handler.
fn basic_mode_device_command(args: &[&str], config: &AppConfig, session: &AgentSession) {
    if !args.is_empty() {
        match slash::run_device_subcommand(args) {
            Ok(lines) => {
                for line in lines {
                    println!("{line}");
                }
            }
            Err(error) => {
                println!("{} {}", "✗".red(), error);
            }
        }
        return;
    }

    basic_mode_managed_browser(
        "Device Console",
        &slash::device_browser_entries(config),
        config,
        session,
    );
}

/// Basic mode `/health` slash command handler.
fn basic_mode_health_command(config: &AppConfig) {
    for line in slash::health_diagnostic_lines(config) {
        println!("{line}");
    }
}

/// Basic mode `/privacy` slash command handler.
fn basic_mode_privacy_command(args: &[&str]) {
    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None | Some("status") | Some("report") => {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let report = rt.block_on(async {
                let manager = gestura_core::get_gdpr_manager().await;
                manager.generate_privacy_report().await
            });
            let pretty =
                serde_json::to_string_pretty(&report).unwrap_or_else(|_| format!("{report:?}"));
            for line in slash::privacy_report_lines(pretty) {
                println!("{line}");
            }
        }
        Some("policy") => {
            for line in slash::privacy_policy_lines() {
                println!("{line}");
            }
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
                "{} Unknown /privacy subcommand: {}. Try: status, report, policy, export",
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

fn print_basic_mode_help() {
    println!();
    println!(
        "{}",
        "╭─ Commands ─────────────────────────────────────────────────╮".dimmed()
    );
    println!(
        "{}  {}   {}",
        "│".dimmed(),
        "/q, /quit, /exit".green(),
        "Exit and save the current session".dimmed()
    );

    for section in catalog::HELP_SECTION_ORDER {
        println!(
            "{}",
            format!(
                "├─ {} ────────────────────────────────────────────────┤",
                catalog::section_title(*section)
            )
            .dimmed()
        );

        for spec in catalog::SLASH_COMMANDS
            .iter()
            .filter(|spec| spec.help_section == *section)
        {
            println!(
                "{}  {}  {}",
                "│".dimmed(),
                spec.command.green(),
                spec.description.dimmed()
            );
        }
    }

    println!(
        "{}",
        "├─ Keyboard ─────────────────────────────────────────────────┤".dimmed()
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
        "╰─────────────────────────────────────────────────────────────╯".dimmed()
    );
    println!();
}

fn basic_mode_workflow_command(args: &[&str]) -> Option<String> {
    let manager = gestura_core::WorkflowManager::new();

    if args.is_empty() {
        match manager.list_workflows() {
            Ok(workflows) if workflows.is_empty() => {
                println!("{} No workflows available", "ℹ".blue());
                None
            }
            Ok(workflows) => {
                let items: Vec<String> = workflows
                    .iter()
                    .map(|workflow| format!("{} — {}", workflow.name, workflow.description))
                    .collect();

                match dialoguer::Select::new()
                    .with_prompt("Open workflow shell")
                    .items(&items)
                    .default(0)
                    .interact()
                {
                    Ok(index) => {
                        let workflow = &workflows[index];
                        match manager.load_workflow(&workflow.name) {
                            Ok(loaded) => {
                                println!("{} Loaded workflow: {}", "✓".green(), loaded.name.cyan());
                                Some(loaded.content)
                            }
                            Err(error) => {
                                println!("{} {}", "✗".red(), error);
                                None
                            }
                        }
                    }
                    Err(error) => {
                        println!("{} {}", "✗".red(), error);
                        None
                    }
                }
            }
            Err(error) => {
                println!("{} {}", "✗".red(), error);
                None
            }
        }
    } else {
        match args[0] {
            "list" => {
                match manager.list_workflows() {
                    Ok(workflows) if workflows.is_empty() => {
                        println!("{} No workflows available", "ℹ".blue())
                    }
                    Ok(workflows) => {
                        println!("{} Available workflows:", "🧩".cyan());
                        for workflow in workflows {
                            println!(
                                "  • {} — {}",
                                workflow.name.green(),
                                workflow.description.dimmed()
                            );
                        }
                    }
                    Err(error) => println!("{} {}", "✗".red(), error),
                }
                None
            }
            "run" => {
                let Some(name) = args.get(1) else {
                    println!("{} Usage: /workflow run <name>", "ℹ".blue());
                    return None;
                };

                match manager.load_workflow(name) {
                    Ok(workflow) => {
                        println!("{} Loaded workflow: {}", "✓".green(), workflow.name.cyan());
                        Some(workflow.content)
                    }
                    Err(error) => {
                        println!("{} {}", "✗".red(), error);
                        None
                    }
                }
            }
            _ => {
                println!("{} Usage: /workflow [list|run <name>]", "ℹ".blue());
                None
            }
        }
    }
}

/// Basic mode `/config` slash command handler.
fn basic_mode_config_command(args: &[&str]) {
    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None | Some("list") => {
            let config = AppConfig::load();
            for line in slash::config_list_lines(&config) {
                println!("{line}");
            }
        }
        Some("get") => {
            if let Some(key) = args.get(1) {
                let config = AppConfig::load();
                match slash::config_get_line(&config, key) {
                    Some(line) => println!("{line}"),
                    None => println!("{} Unknown config key: {}", "✗".red(), key),
                }
            } else {
                println!("{} Usage: /config get <key>", "✗".red());
            }
        }
        Some("keys") => {
            for line in slash::config_keys_lines() {
                println!("{line}");
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
                    println!(
                        "{} {}",
                        "✓".green(),
                        slash::config_updated_message(key, &value)
                    );
                }
            } else {
                println!("{} Unknown or read-only config key: {}", "✗".red(), key);
            }
        }
        Some("path") => {
            println!("{}", slash::config_path_line());
        }
        Some("reset") => {
            let config = AppConfig::default();
            if let Err(e) = config.save() {
                println!("{} Failed to save config: {}", "✗".red(), e);
            } else {
                println!("{} {}", "✓".green(), slash::config_reset_message());
            }
        }
        Some(other) => {
            println!(
                "{} Unknown /config subcommand: '{}'. Try: list, get, keys, set, path, reset",
                "✗".red(),
                other
            );
        }
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
        "pipeline.agent_telemetry.enabled" => {
            if let Ok(val) = value.parse::<bool>() {
                config.pipeline.agent_telemetry.enabled = val;
                true
            } else {
                false
            }
        }
        "pipeline.agent_telemetry.trace_export.enabled" => {
            if let Ok(val) = value.parse::<bool>() {
                config.pipeline.agent_telemetry.trace_export.enabled = val;
                true
            } else {
                false
            }
        }
        "pipeline.agent_telemetry.trace_export.protocol" => {
            if let Some(protocol) = AgentTelemetryTraceExportProtocol::parse(value) {
                config.pipeline.agent_telemetry.trace_export.protocol = protocol;
                true
            } else {
                false
            }
        }
        "pipeline.agent_telemetry.trace_export.endpoint" => {
            config.pipeline.agent_telemetry.trace_export.endpoint = value.to_string();
            true
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

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        basic_mode_set_config_value, resolve_session_active_task_id, sync_session_active_task_id,
    };
    use gestura_core::AppConfig;

    #[test]
    fn basic_mode_set_config_value_supports_agent_telemetry_fields() {
        let mut config = AppConfig::default();

        assert!(basic_mode_set_config_value(
            &mut config,
            "pipeline.agent_telemetry.enabled",
            "true"
        ));
        assert!(basic_mode_set_config_value(
            &mut config,
            "pipeline.agent_telemetry.trace_export.enabled",
            "true"
        ));
        assert!(basic_mode_set_config_value(
            &mut config,
            "pipeline.agent_telemetry.trace_export.protocol",
            "http"
        ));
        assert!(basic_mode_set_config_value(
            &mut config,
            "pipeline.agent_telemetry.trace_export.endpoint",
            "http://127.0.0.1:4318/v1/traces"
        ));

        assert!(config.pipeline.agent_telemetry.enabled);
        assert!(config.pipeline.agent_telemetry.trace_export.enabled);
        assert_eq!(
            config
                .pipeline
                .agent_telemetry
                .trace_export
                .protocol
                .as_str(),
            "http"
        );
        assert_eq!(
            config.pipeline.agent_telemetry.trace_export.endpoint,
            "http://127.0.0.1:4318/v1/traces"
        );
    }

    #[test]
    fn resolve_session_active_task_id_falls_back_to_session_working_memory() {
        let workspace = std::env::temp_dir().join(format!(
            "gestura-cli-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let mut session = super::AgentSession::new_with_workspace(workspace, None).unwrap();
        session.state.working_memory.active_task_id = Some("task-123".to_string());

        assert_eq!(
            resolve_session_active_task_id(&session),
            Some("task-123".to_string())
        );
    }

    #[test]
    fn sync_session_active_task_id_normalizes_empty_values_to_none() {
        let workspace = std::env::temp_dir().join(format!(
            "gestura-cli-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let mut session = super::AgentSession::new_with_workspace(workspace, None).unwrap();
        session.state.working_memory.active_task_id = Some("   ".to_string());

        assert!(sync_session_active_task_id(&mut session));
        assert_eq!(session.state.working_memory.active_task_id, None);
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
            for line in slash::session_info_lines(current) {
                println!("{line}");
            }
        }
        Some("list") => {
            let (filter, filter_label) = slash::parse_session_list_filter(args.get(1).copied());
            match list_sessions_filtered(filter) {
                Ok(sessions) => {
                    if sessions.is_empty() {
                        println!("{}", slash::session_empty_message(&filter_label));
                    } else {
                        for line in slash::session_list_lines(
                            &sessions,
                            &current.id,
                            &filter_label,
                            20,
                            false,
                        ) {
                            println!("{line}");
                        }
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
    use gestura_core::context::{ContextManager, RequestAnalyzer};

    let subcommand = args.first().map(|s| s.to_ascii_lowercase());
    match subcommand.as_deref() {
        None | Some("status") => {
            let manager = ContextManager::new();
            let stats = manager.cache_stats();
            for line in slash::context_status_lines(&stats) {
                println!("{line}");
            }
        }
        Some("analyze") => {
            if args.len() < 2 {
                println!("{} Usage: /context analyze <request text>", "✗".red());
                return;
            }
            let request = args[1..].join(" ");
            let analyzer = RequestAnalyzer::new();
            let analysis = analyzer.analyze(&request);
            for line in slash::context_analysis_lines(&request, &analysis) {
                println!("{line}");
            }
        }
        Some("categories") => {
            for line in slash::context_categories_lines() {
                println!("{line}");
            }
        }
        Some("clear") => {
            let manager = ContextManager::new();
            manager.clear_caches();
            println!("{} {}", "✓".green(), slash::context_clear_message());
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
    use gestura_core::tasks::TaskStatus;

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
    let task_manager = gestura_core::get_global_task_manager();

    let run_canonical = |args: &[&str]| {
        match slash::run_tasks_subcommand(
            args,
            task_manager,
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
                    task_manager,
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
