//! Interactive chat command

use super::Result;
use chrono::{DateTime, Local};
use colored::Colorize;
use gestura_core::{
    AgentContext, AppConfig, AudioCaptureConfig, get_speech_processor, select_provider,
};
use indicatif::{ProgressBar, ProgressStyle};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

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

/// Message in a chat session
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
    timestamp: DateTime<Local>,
}

/// Chat session data
#[derive(Debug, Serialize, Deserialize)]
struct ChatSession {
    id: String,
    created: DateTime<Local>,
    updated: DateTime<Local>,
    model: Option<String>,
    messages: Vec<ChatMessage>,
}

impl ChatSession {
    fn new(model: Option<String>) -> Self {
        let now = Local::now();
        Self {
            id: Uuid::new_v4().to_string(),
            created: now,
            updated: now,
            model,
            messages: Vec::new(),
        }
    }

    fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            timestamp: Local::now(),
        });
        self.updated = Local::now();
    }

    fn save(&self) -> Result<()> {
        let sessions_dir = get_sessions_dir();
        fs::create_dir_all(&sessions_dir)?;
        let path = sessions_dir.join(format!("{}.json", self.id));
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    fn load(id: &str) -> Result<Self> {
        let path = get_sessions_dir().join(format!("{}.json", id));
        let json = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }

    fn load_last() -> Result<Option<Self>> {
        let sessions_dir = get_sessions_dir();
        if !sessions_dir.exists() {
            return Ok(None);
        }

        let mut entries: Vec<_> = fs::read_dir(&sessions_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .collect();

        entries.sort_by(|a, b| {
            b.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                .cmp(
                    &a.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                )
        });

        if let Some(entry) = entries.first() {
            let json = fs::read_to_string(entry.path())?;
            return Ok(Some(serde_json::from_str(&json)?));
        }

        Ok(None)
    }

    /// Delete a session by ID
    fn delete(id: &str) -> Result<bool> {
        let path = get_sessions_dir().join(format!("{}.json", id));
        if path.exists() {
            fs::remove_file(path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Export session to a file
    fn export(&self, path: &std::path::Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }
}

/// Session metadata for listing
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub created: DateTime<Local>,
    pub updated: DateTime<Local>,
    pub message_count: usize,
    pub model: Option<String>,
}

/// List all available sessions with metadata
pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    let sessions_dir = get_sessions_dir();
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in fs::read_dir(&sessions_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            if let Ok(json) = fs::read_to_string(&path) {
                if let Ok(session) = serde_json::from_str::<ChatSession>(&json) {
                    sessions.push(SessionInfo {
                        id: session.id,
                        created: session.created,
                        updated: session.updated,
                        message_count: session.messages.len(),
                        model: session.model,
                    });
                }
            }
        }
    }

    // Sort by updated time, most recent first
    sessions.sort_by(|a, b| b.updated.cmp(&a.updated));
    Ok(sessions)
}

fn get_sessions_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gestura")
        .join("sessions")
}

fn get_history_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gestura")
        .join("chat_history.txt")
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
            match ChatSession::load(id) {
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
            match ChatSession::load_last()? {
                Some(s) => {
                    println!("{} Resuming last session {}", "→".cyan(), s.id.dimmed());
                    s
                }
                None => {
                    println!(
                        "{}",
                        "No previous session found, starting new chat.".yellow()
                    );
                    ChatSession::new(model.map(String::from))
                }
            }
        }
    } else {
        ChatSession::new(model.map(String::from))
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
            _ => {}
        }
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
    let hints = "/help commands · /tools list · Ctrl+C quit";
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
    if !chat_session.messages.is_empty() {
        let history_header = format!("┌─ History ({} messages) ", chat_session.messages.len());
        let history_padding = inner_width.saturating_sub(history_header.len()) + 3;
        println!(
            "{}{}",
            history_header.dimmed(),
            "─".repeat(history_padding).dimmed()
        );

        for msg in &chat_session.messages {
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
                let input = if input.is_empty() && voice {
                    match record_voice_input(&rt) {
                        Ok(text) => text,
                        Err(e) => {
                            eprintln!("{}: {}", "Voice error".red(), e);
                            continue;
                        }
                    }
                } else if input.is_empty() {
                    continue;
                } else {
                    input.to_string()
                };

                // Add to history
                let _ = rl.add_history_entry(&input);

                // Handle commands
                if input.starts_with('/') {
                    let mut parts = input.split_whitespace();
                    let cmd = parts.next().unwrap_or("");
                    let args: Vec<&str> = parts.collect();
                    match cmd.to_ascii_lowercase().as_str() {
                        "/exit" | "/quit" | "/q" => {
                            chat_session.save()?;
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
                                "List all tools or show detail for one".dimmed()
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
                            if let Some(name) = args.first() {
                                match crate::tool_registry::render_tool_detail(name) {
                                    Some(text) => println!("{}", text),
                                    None => println!(
                                        "{}: Unknown tool '{}'. Try /tools to list tools.",
                                        "error".red(),
                                        name
                                    ),
                                }
                            } else {
                                println!("{}", crate::tool_registry::render_tools_overview());
                            }
                            println!();
                            continue;
                        }
                        "/clear" => {
                            print!("\x1B[2J\x1B[1;1H");
                            continue;
                        }
                        "/save" => {
                            chat_session.save()?;
                            println!("{} Session saved", "✓".green());
                            continue;
                        }
                        "/history" => {
                            let user_msgs = chat_session
                                .messages
                                .iter()
                                .filter(|m| m.role == "user")
                                .count();
                            let asst_msgs = chat_session
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
                                chat_session.messages.len()
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
                            println!(
                                "{}",
                                "╰───────────────────────────────────────────────────────────────╯"
                                    .dimmed()
                            );
                            println!();
                            continue;
                        }
                        "/new" => {
                            chat_session.save()?;
                            chat_session = ChatSession::new(model.map(String::from));
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
                chat_session.add_message("user", &input);

                // Deterministic response for common tool-inventory questions.
                if crate::tool_registry::looks_like_tools_question(&input) {
                    let text = crate::tool_registry::render_tools_overview();
                    println!();
                    println!(
                        "{} {}",
                        "◆".blue().bold(),
                        "Here are the available tools:".blue()
                    );
                    println!();
                    println!("{}", text);
                    chat_session.add_message("assistant", &text);
                    continue;
                }

                // Build conversation context for the LLM
                let mut context = String::new();
                for msg in chat_session.messages.iter().rev().take(10).rev() {
                    match msg.role.as_str() {
                        "user" => context.push_str(&format!("User: {}\n", msg.content)),
                        "assistant" => context.push_str(&format!("Assistant: {}\n", msg.content)),
                        _ => {}
                    }
                }

                // ─────────────────────────────────────────────────────────────
                // AI RESPONSE: Show thinking indicator then response
                // ─────────────────────────────────────────────────────────────
                print!("{} {}", "◆".blue().bold(), "Thinking...".dimmed());
                use std::io::Write;
                let _ = std::io::stdout().flush();

                let sys_prompt = system_prompt.clone();
                let response = rt.block_on(async {
                    let provider = select_provider(&config, &AgentContext {
                        agent_id: format!("chat-{}", chat_session.id),
                    });

                    // Build prompt with optional system context
                    let full_prompt = if let Some(ref sys) = sys_prompt {
                        if chat_session.messages.len() > 1 {
                            format!("System: {}\n\nPrevious conversation:\n{}\nRespond to the latest user message.", sys, context)
                        } else {
                            format!("System: {}\n\nUser: {}", sys, input)
                        }
                    } else if chat_session.messages.len() > 1 {
                        format!("Previous conversation:\n{}\nRespond to the latest user message.", context)
                    } else {
                        input.to_string()
                    };

                    provider.call(&full_prompt).await
                });

                // Clear the "Thinking..." line
                print!("\r\x1B[K");

                match response {
                    Ok(text) => {
                        // Format the response with proper line wrapping
                        let term_width = termsize::get().map(|s| s.cols as usize).unwrap_or(80);
                        let max_width = term_width.saturating_sub(4).max(40);

                        println!("{}", "◆".blue().bold());
                        for line in text.lines() {
                            if line.len() > max_width {
                                let wrapped = textwrap::wrap(line, max_width);
                                for part in wrapped {
                                    println!("  {}", part);
                                }
                            } else {
                                println!("  {}", line);
                            }
                        }
                        chat_session.add_message("assistant", &text);
                    }
                    Err(e) => {
                        println!();
                        println!("{} {} {}", "✗".red(), "Error:".red(), e);
                    }
                }
                println!();

                // Auto-save periodically
                if chat_session.messages.len() % 5 == 0 {
                    let _ = chat_session.save();
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
                chat_session.save()?;
                break;
            }
            Err(ReadlineError::Eof) => {
                println!();
                println!("{} {}", "✓".green(), "Session saved. Goodbye!".dimmed());
                chat_session.save()?;
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
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    spinner.set_message("Listening... (speak now, silence will stop recording)");
    spinner.enable_steady_tick(Duration::from_millis(100));

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
