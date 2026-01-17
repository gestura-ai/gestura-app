//! Modern TUI for the chat command
//!
//! This module provides a sophisticated terminal user interface with:
//! - Fixed window layout (header, content, input, status)
//! - Stateful scrollable message list
//! - Real-time streaming responses
//! - Multi-line input with cursor control
//! - Vim-style modes (Normal, Insert, Command)
//! - Tab navigation between views
//! - Help overlay
//!
//! ## Terminal Compatibility
//!
//! This TUI uses `crossterm` for cross-platform terminal support and has been
//! designed to work with:
//!
//! - **macOS**: Terminal.app, iTerm2, Alacritty, Kitty
//! - **Linux**: gnome-terminal, Alacritty, Kitty, xterm, Konsole
//! - **Windows**: Windows Terminal, PowerShell, cmd.exe (with limitations)
//!
//! ### Requirements
//! - Minimum terminal size: 40x12 characters
//! - Recommended: 80x24 or larger for optimal experience
//! - True color support recommended for best theme rendering
//!
//! ### Known Limitations
//! - Some older terminals may not support all Unicode characters
//! - Mouse support may be limited in some terminal emulators
//! - SSH sessions with high latency may experience delayed rendering

mod app;
mod events;
mod ui;

pub use app::{Action, TuiApp, TuiMode};

use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use gestura_core::{AppConfig, CancellationToken, StreamChunk, start_streaming};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use super::{ChatOptions, ChatSession, Result};

/// Streaming state for async LLM responses
struct StreamingState {
    /// Receiver for stream chunks
    receiver: mpsc::Receiver<StreamChunk>,
    /// Cancellation token to stop the stream
    cancel_token: CancellationToken,
    /// Accumulated content from streaming
    content: String,
}

/// Run the TUI chat interface
pub fn run_tui(opts: ChatOptions<'_>) -> Result<()> {
    // Load or create session
    let session = if opts.resume {
        if let Some(id) = opts.session {
            ChatSession::load(id)?
        } else {
            ChatSession::load_last()?.unwrap_or_else(|| ChatSession::new(opts.model.map(String::from)))
        }
    } else {
        ChatSession::new(opts.model.map(String::from))
    };

    // Load config
    let mut config = AppConfig::load();
    if let Some(m) = opts.model.or(session.model.as_deref())
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

    // Create app state
    let mut app = TuiApp::new(session, config, opts.system.map(String::from));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create tokio runtime for async LLM calls
    let rt = tokio::runtime::Runtime::new()?;

    // Run the main loop
    let result = run_main_loop(&mut terminal, &mut app, &rt);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Save session on exit
    app.session.save()?;

    result
}

/// Main event loop
fn run_main_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TuiApp,
    rt: &tokio::runtime::Runtime,
) -> Result<()> {
    // Optional streaming state
    let mut streaming: Option<StreamingState> = None;

    loop {
        // Render
        terminal.draw(|f| ui::render(app, f))?;

        // Process streaming chunks if active
        if let Some(ref mut stream_state) = streaming {
            // Try to receive chunks without blocking
            loop {
                match stream_state.receiver.try_recv() {
                    Ok(chunk) => match chunk {
                        StreamChunk::Text(text) => {
                            stream_state.content.push_str(&text);
                            app.update_last_message(&stream_state.content);
                        }
                        StreamChunk::Done => {
                            app.finalize_streaming_message();
                            app.is_loading = false;
                            app.set_status("Ready");
                            streaming = None;
                            break;
                        }
                        StreamChunk::Cancelled => {
                            // Keep partial content but mark as cancelled
                            if !stream_state.content.is_empty() {
                                app.update_last_message(&format!(
                                    "{}\n\n[Cancelled]",
                                    stream_state.content
                                ));
                                app.finalize_streaming_message();
                            } else {
                                // Remove empty streaming message
                                app.messages.pop();
                            }
                            app.is_loading = false;
                            app.set_status("Cancelled");
                            streaming = None;
                            break;
                        }
                        StreamChunk::Error(err) => {
                            app.set_error(format!("Stream error: {}", err));
                            if !stream_state.content.is_empty() {
                                app.update_last_message(&format!(
                                    "{}\n\n[Error: {}]",
                                    stream_state.content, err
                                ));
                                app.finalize_streaming_message();
                            } else {
                                app.messages.pop();
                            }
                            app.is_loading = false;
                            streaming = None;
                            break;
                        }
                    },
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        // Stream ended unexpectedly
                        if !stream_state.content.is_empty() {
                            app.finalize_streaming_message();
                        } else {
                            app.messages.pop();
                        }
                        app.is_loading = false;
                        app.set_status("Ready");
                        streaming = None;
                        break;
                    }
                }
            }
        }

        // Poll for events with timeout (allows for streaming updates)
        if event::poll(Duration::from_millis(50))? {
            let event = event::read()?;
            let action = events::handle_event(app, event);

            match action {
                Action::Quit => {
                    // Cancel any active stream before quitting
                    if let Some(ref stream_state) = streaming {
                        stream_state.cancel_token.cancel();
                    }
                    break;
                }
                Action::SendMessage(msg) => {
                    // Don't allow sending while streaming
                    if streaming.is_none() {
                        streaming = start_streaming_message(app, rt, &msg)?;
                    }
                }
                Action::ExecuteCommand(cmd) => {
                    handle_command(app, &cmd)?;
                }
                Action::SwitchTab(idx) => {
                    if idx < app.tabs.len() {
                        app.active_tab = idx;
                    }
                }
                Action::ToggleHelp => {
                    app.mode = if app.mode == TuiMode::Help {
                        TuiMode::Insert
                    } else {
                        TuiMode::Help
                    };
                }
                Action::ScrollUp => {
                    app.scroll_up();
                }
                Action::ScrollDown => {
                    app.scroll_down();
                }
                Action::ClearInput => {
                    app.clear_input();
                }
                Action::Cancel => {
                    // Cancel active streaming
                    if let Some(ref stream_state) = streaming {
                        stream_state.cancel_token.cancel();
                        app.set_status("Cancelling...");
                    }
                }
                Action::Continue => {}
            }
        }
    }

    Ok(())
}

/// Start streaming a message to the LLM (non-blocking)
fn start_streaming_message(
    app: &mut TuiApp,
    rt: &tokio::runtime::Runtime,
    message: &str,
) -> Result<Option<StreamingState>> {
    // Check for tool-related questions first (handle synchronously)
    if crate::tool_registry::looks_like_tools_question(message) {
        app.add_message("user", message);
        let response = crate::tool_registry::render_tools_overview();
        app.add_message("assistant", &response);
        return Ok(None);
    }

    // Add user message
    app.add_message("user", message);
    app.is_loading = true;
    app.set_status("Streaming...");

    // Add placeholder for streaming response
    app.add_streaming_message();

    // Build conversation context
    let mut context = String::new();
    // Use messages before the streaming placeholder (skip last 2: user msg + streaming placeholder)
    let msg_count = app.session.messages.len();
    for msg in app.session.messages.iter().take(msg_count.saturating_sub(1)).rev().take(10).rev() {
        match msg.role.as_str() {
            "user" => context.push_str(&format!("User: {}\n", msg.content)),
            "assistant" => context.push_str(&format!("Assistant: {}\n", msg.content)),
            _ => {}
        }
    }

    // Build full prompt
    let system_prompt = app.system_prompt.clone();
    let message_owned = message.to_string();

    let full_prompt = if let Some(ref sys) = system_prompt {
        if context.len() > message_owned.len() + 10 {
            format!(
                "System: {}\n\nPrevious conversation:\n{}\nRespond to the latest user message.",
                sys, context
            )
        } else {
            format!("System: {}\n\nUser: {}", sys, message_owned)
        }
    } else if context.len() > message_owned.len() + 10 {
        format!(
            "Previous conversation:\n{}\nRespond to the latest user message.",
            context
        )
    } else {
        message_owned
    };

    // Create channel and cancellation token
    let (tx, rx) = mpsc::channel::<StreamChunk>(100);
    let cancel_token = CancellationToken::new();
    let cancel_token_clone = cancel_token.clone();
    let config = app.config.clone();

    // Spawn streaming task
    rt.spawn(async move {
        if let Err(e) = start_streaming(&config, &full_prompt, tx.clone(), cancel_token_clone).await
        {
            let _ = tx.send(StreamChunk::Error(e.to_string())).await;
        }
    });

    Ok(Some(StreamingState {
        receiver: rx,
        cancel_token,
        content: String::new(),
    }))
}

/// Handle slash commands
fn handle_command(app: &mut TuiApp, command: &str) -> Result<()> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let cmd = parts.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let args = &parts[1..];

    match cmd.as_str() {
        "/quit" | "/q" | "/exit" => {
            // This will be handled by returning Quit action
            // For now, just save
            app.session.save()?;
            app.set_status("Session saved. Use Ctrl+C or q to quit.");
        }
        "/help" | "/?" => {
            app.mode = TuiMode::Help;
        }
        "/tools" => {
            if let Some(name) = args.first() {
                if let Some(detail) = crate::tool_registry::render_tool_detail(name) {
                    app.messages.push(app::TuiMessage {
                        role: "system".to_string(),
                        content: detail,
                        is_streaming: false,
                        is_error: false,
                    });
                } else {
                    app.set_error(format!("Unknown tool: {}", name));
                }
            } else {
                app.active_tab = 1; // Switch to tools tab
            }
        }
        "/clear" => {
            app.messages.clear();
            app.message_list_state.select(None);
            app.set_status("Messages cleared");
        }
        "/save" => {
            app.session.save()?;
            app.set_status("Session saved");
        }
        "/new" => {
            app.session.save()?;
            app.session = ChatSession::new(app.session.model.clone());
            app.messages.clear();
            app.message_list_state.select(None);
            app.set_status(format!("New session: {}", &app.session.id[..8]));
        }
        "/history" => {
            let user_count = app.messages.iter().filter(|m| m.role == "user").count();
            let asst_count = app.messages.iter().filter(|m| m.role == "assistant").count();
            app.set_status(format!(
                "Session: {} | Messages: {} (you: {}, AI: {})",
                &app.session.id[..8],
                app.messages.len(),
                user_count,
                asst_count
            ));
        }
        "/settings" => {
            app.active_tab = 2; // Switch to settings tab
        }
        "/capabilities" => {
            let caps = crate::tool_registry::render_capabilities();
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: caps,
                is_streaming: false,
                is_error: false,
            });
        }
        "/theme" => {
            if let Some(name) = args.first() {
                app.set_theme(name);
            } else {
                // Show available themes
                let themes = app::Theme::available_themes().join(", ");
                app.set_status(format!("Available themes: {} (Ctrl+T to cycle)", themes));
            }
        }
        "/sessions" | "/session" => {
            handle_session_command(app, args)?;
        }
        _ => {
            app.set_error(format!("Unknown command: {}", cmd));
        }
    }

    Ok(())
}

/// Handle session management commands
fn handle_session_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    let subcommand = args.first().map(|s| s.to_lowercase());

    match subcommand.as_deref() {
        None | Some("list") => {
            // List all sessions
            match super::list_sessions() {
                Ok(sessions) => {
                    if sessions.is_empty() {
                        app.set_status("No saved sessions found");
                    } else {
                        let mut content = String::from("📁 Saved Sessions:\n\n");
                        for (i, session) in sessions.iter().take(10).enumerate() {
                            let is_current = session.id == app.session.id;
                            let marker = if is_current { "▶ " } else { "  " };
                            let model_info = session.model.as_deref().unwrap_or("default");
                            content.push_str(&format!(
                                "{}{}. {} ({} msgs, {})\n   Updated: {}\n\n",
                                marker,
                                i + 1,
                                &session.id[..8],
                                session.message_count,
                                model_info,
                                session.updated.format("%Y-%m-%d %H:%M")
                            ));
                        }
                        content.push_str("\nCommands: /session load <id> | /session delete <id> | /session export <id>");
                        app.messages.push(app::TuiMessage {
                            role: "system".to_string(),
                            content,
                            is_streaming: false,
                            is_error: false,
                        });
                        app.scroll_to_bottom();
                    }
                }
                Err(e) => {
                    app.set_error(format!("Failed to list sessions: {}", e));
                }
            }
        }
        Some("load") | Some("switch") | Some("resume") => {
            if let Some(id) = args.get(1) {
                // Save current session first
                if let Err(e) = app.session.save() {
                    app.set_error(format!("Failed to save current session: {}", e));
                    return Ok(());
                }

                // Try to load the session (support partial ID matching)
                match find_session_by_prefix(id) {
                    Ok(Some(full_id)) => {
                        match ChatSession::load(&full_id) {
                            Ok(session) => {
                                // Convert session messages to TUI messages
                                app.messages = session
                                    .messages
                                    .iter()
                                    .map(|m| app::TuiMessage {
                                        role: m.role.clone(),
                                        content: m.content.clone(),
                                        is_streaming: false,
                                        is_error: false,
                                    })
                                    .collect();
                                app.session = session;
                                app.message_list_state.select(None);
                                app.scroll_to_bottom();
                                app.set_status(format!("Loaded session: {}", &full_id[..8]));
                            }
                            Err(e) => {
                                app.set_error(format!("Failed to load session: {}", e));
                            }
                        }
                    }
                    Ok(None) => {
                        app.set_error(format!("Session not found: {}", id));
                    }
                    Err(e) => {
                        app.set_error(format!("Error finding session: {}", e));
                    }
                }
            } else {
                app.set_error("Usage: /session load <session_id>");
            }
        }
        Some("delete") | Some("rm") => {
            if let Some(id) = args.get(1) {
                // Don't allow deleting current session
                if app.session.id.starts_with(*id) {
                    app.set_error("Cannot delete current session. Use /new first.");
                    return Ok(());
                }

                match find_session_by_prefix(id) {
                    Ok(Some(full_id)) => {
                        match ChatSession::delete(&full_id) {
                            Ok(true) => {
                                app.set_status(format!("Deleted session: {}", &full_id[..8]));
                            }
                            Ok(false) => {
                                app.set_error(format!("Session not found: {}", id));
                            }
                            Err(e) => {
                                app.set_error(format!("Failed to delete session: {}", e));
                            }
                        }
                    }
                    Ok(None) => {
                        app.set_error(format!("Session not found: {}", id));
                    }
                    Err(e) => {
                        app.set_error(format!("Error finding session: {}", e));
                    }
                }
            } else {
                app.set_error("Usage: /session delete <session_id>");
            }
        }
        Some("export") => {
            if let Some(id) = args.get(1) {
                let export_path = args.get(2).map(|s| std::path::PathBuf::from(s));

                match find_session_by_prefix(id) {
                    Ok(Some(full_id)) => {
                        match ChatSession::load(&full_id) {
                            Ok(session) => {
                                let path = export_path.unwrap_or_else(|| {
                                    std::path::PathBuf::from(format!("session_{}.json", &full_id[..8]))
                                });
                                match session.export(&path) {
                                    Ok(()) => {
                                        app.set_status(format!("Exported to: {}", path.display()));
                                    }
                                    Err(e) => {
                                        app.set_error(format!("Failed to export: {}", e));
                                    }
                                }
                            }
                            Err(e) => {
                                app.set_error(format!("Failed to load session: {}", e));
                            }
                        }
                    }
                    Ok(None) => {
                        app.set_error(format!("Session not found: {}", id));
                    }
                    Err(e) => {
                        app.set_error(format!("Error finding session: {}", e));
                    }
                }
            } else {
                // Export current session
                let path = std::path::PathBuf::from(format!("session_{}.json", &app.session.id[..8]));
                match app.session.export(&path) {
                    Ok(()) => {
                        app.set_status(format!("Exported current session to: {}", path.display()));
                    }
                    Err(e) => {
                        app.set_error(format!("Failed to export: {}", e));
                    }
                }
            }
        }
        Some("info") => {
            let session = &app.session;
            let user_count = app.messages.iter().filter(|m| m.role == "user").count();
            let asst_count = app.messages.iter().filter(|m| m.role == "assistant").count();
            let content = format!(
                "📋 Current Session Info:\n\n\
                 ID: {}\n\
                 Created: {}\n\
                 Updated: {}\n\
                 Model: {}\n\
                 Messages: {} (you: {}, AI: {})",
                session.id,
                session.created.format("%Y-%m-%d %H:%M:%S"),
                session.updated.format("%Y-%m-%d %H:%M:%S"),
                session.model.as_deref().unwrap_or("default"),
                app.messages.len(),
                user_count,
                asst_count
            );
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content,
                is_streaming: false,
                is_error: false,
            });
            app.scroll_to_bottom();
        }
        Some(unknown) => {
            app.set_error(format!(
                "Unknown session command: {}. Try: list, load, delete, export, info",
                unknown
            ));
        }
    }

    Ok(())
}

/// Find a session by ID prefix
fn find_session_by_prefix(prefix: &str) -> Result<Option<String>> {
    let sessions = super::list_sessions()?;
    for session in sessions {
        if session.id.starts_with(prefix) {
            return Ok(Some(session.id));
        }
    }
    Ok(None)
}

