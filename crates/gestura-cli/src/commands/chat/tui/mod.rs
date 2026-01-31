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

use app::ConfirmAction;

use std::io;
use std::time::Duration;

use chrono::Local;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use gestura_core::{
    AgentPipeline, AgentRequest, AppConfig, CancellationToken, RequestSource, StreamChunk,
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use super::{ChatOptions, Result};

/// Streaming state for async LLM responses
struct StreamingState {
    /// Receiver for stream chunks
    receiver: mpsc::Receiver<StreamChunk>,
    /// Cancellation token to stop the stream
    cancel_token: CancellationToken,
    /// Accumulated content from streaming
    content: String,
    /// Accumulated thinking content
    thinking_content: String,
}

/// In-flight prompt enhancement state.
///
/// Prompt enhancement must never block the TUI event loop; we run the LLM call on the Tokio
/// runtime and poll the receiver from the main loop (similar to streaming responses).
struct PromptEnhancementState {
    /// Receiver for the enhancement result.
    receiver: mpsc::Receiver<std::result::Result<String, String>>,
    /// The original input captured when enhancement started (used for undo).
    original_input: String,
}

/// Whether a command should be allowed to run while a stream is in progress.
///
/// Some commands mutate the message list/session state (e.g., `/clear`, `/new`) and can corrupt
/// the UI while streaming updates are still being applied. We allow only safe, user-initiated exit
/// commands during active streaming.
fn command_allowed_while_streaming(command: &str) -> bool {
    let cmd = command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();

    matches!(cmd.as_str(), "/quit" | "/q" | "/exit" | "/quit!")
}

/// Run the TUI chat interface
pub fn run_tui(opts: ChatOptions<'_>) -> Result<()> {
    // Load or create session
    let mut session = if opts.resume {
        if let Some(id) = opts.session {
            super::load_cli_session(id)?
        } else if let Some(last) = super::load_last_cli_session()? {
            last
        } else {
            super::new_cli_session(opts.model.map(String::from))?
        }
    } else {
        super::new_cli_session(opts.model.map(String::from))?
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

    // Ensure persisted sessions have tool settings (migration / defaults).
    if super::ensure_session_tool_settings(&mut session, &config) {
        super::save_cli_session(&session)?;
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

    // Load initial workflows
    load_workflows(&mut app);

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

    // Save session on exit unless explicitly suppressed ("quit without save").
    if !app.skip_save_on_exit {
        super::save_cli_session(&app.session)?;
    }

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
    // Optional prompt enhancement state
    let mut prompt_enhancement: Option<PromptEnhancementState> = None;

    loop {
        // Check for auto-dismiss of transient errors (15 second timeout)
        // Skip if user is actively interacting (in Insert or Command mode)
        if app.mode != TuiMode::Insert && app.mode != TuiMode::Command {
            app.check_error_timeout();
        }

        // Render
        terminal.draw(|f| ui::render(app, f))?;

        // Process streaming chunks if active
        if let Some(ref mut stream_state) = streaming {
            // Try to receive chunks without blocking
            loop {
                match stream_state.receiver.try_recv() {
                    Ok(chunk) => match chunk {
                        StreamChunk::Thinking(text) => {
                            stream_state.thinking_content.push_str(&text);
                            app.update_last_message_thinking(&stream_state.thinking_content);
                        }
                        StreamChunk::Text(text) => {
                            stream_state.content.push_str(&text);
                            app.update_last_message(&stream_state.content);
                        }
                        StreamChunk::ToolCallStart { id: _, name } => {
                            stream_state
                                .content
                                .push_str(&format!("\n\n🔧 *Using tool: {}*\n", name));
                            app.update_last_message(&stream_state.content);
                        }
                        StreamChunk::ToolCallArgs(args) => {
                            // Accumulate tool arguments (could display progress)
                            stream_state.content.push_str(&format!(
                                "  ⏳ Args: {}...\n",
                                &args[..args.len().min(50)]
                            ));
                            app.update_last_message(&stream_state.content);
                        }
                        StreamChunk::ToolCallEnd => {
                            // Tool specification ended, waiting for result
                        }
                        StreamChunk::ToolCallResult {
                            name,
                            success,
                            output,
                            duration_ms,
                        } => {
                            if success {
                                let formatted = format_tool_output_tui(&output);
                                stream_state.content.push_str(&format!(
                                    "  ✅ {} ({}ms):\n{}\n",
                                    name, duration_ms, formatted
                                ));
                            } else {
                                let formatted = format_tool_output_tui(&output);
                                stream_state.content.push_str(&format!(
                                    "  ❌ {} failed ({}ms):\n{}\n",
                                    name, duration_ms, formatted
                                ));
                            }
                            app.update_last_message(&stream_state.content);
                        }
                        StreamChunk::RetryAttempt {
                            attempt,
                            max_attempts,
                            delay_ms,
                            error_message,
                        } => {
                            // Show retry notification in the stream
                            stream_state.content.push_str(&format!(
                                "\n⟳ Retry {}/{} in {}ms: {}\n",
                                attempt, max_attempts, delay_ms, error_message
                            ));
                            app.update_last_message(&stream_state.content);
                            app.set_status(format!("Retrying ({}/{})", attempt, max_attempts));
                        }
                        StreamChunk::ContextCompacted {
                            messages_before,
                            messages_after,
                            tokens_saved,
                            summary,
                        } => {
                            // Show compaction notification in the stream
                            stream_state.content.push_str(&format!(
                                "\n📦 Context compacted: {} → {} messages ({} tokens saved)\n",
                                messages_before, messages_after, tokens_saved
                            ));
                            if !summary.is_empty() {
                                stream_state.content.push_str(&format!("   {}\n", summary));
                            }
                            app.update_last_message(&stream_state.content);
                            app.set_status("Context compacted".to_string());
                        }
                        StreamChunk::MemoryBankSaved {
                            file_path,
                            session_id,
                            summary,
                            messages_saved,
                        } => {
                            // Show memory bank save notification in the stream
                            stream_state.content.push_str(&format!(
                                "\n💾 Memory bank saved: {} messages\n",
                                messages_saved
                            ));
                            stream_state
                                .content
                                .push_str(&format!("   File: {}\n", file_path));
                            if !summary.is_empty() {
                                stream_state
                                    .content
                                    .push_str(&format!("   Summary: {}\n", summary));
                            }
                            app.update_last_message(&stream_state.content);
                            app.set_status(format!("Memory saved (session: {})", session_id));
                        }
                        StreamChunk::Done(usage) => {
                            // Record token usage if available
                            if let Some(usage) = usage {
                                app.record_token_usage(
                                    usage.input_tokens,
                                    usage.output_tokens,
                                    usage.estimated_cost_usd,
                                );
                            }
                            app.finalize_streaming_message();
                            app.is_loading = false;
                            // Clear any previous error on successful completion
                            app.clear_error();
                            streaming = None;
                            break;
                        }
                        StreamChunk::ConfigRequest { key, value, .. } => {
                            // In CLI TUI, just show config request as info
                            let msg = if let Some(v) = value {
                                format!("📋 Config request: {} → {}", key, v)
                            } else {
                                format!("📋 Config query: {}", key)
                            };
                            app.set_status(msg);
                        }
                        StreamChunk::ToolConfirmationRequired {
                            tool_name,
                            description,
                            ..
                        } => {
                            // In CLI TUI, show confirmation request as status
                            stream_state.content.push_str(&format!(
                                "\n⚠️ Tool '{}' requires confirmation: {}\n",
                                tool_name, description
                            ));
                            app.update_last_message(&stream_state.content);
                            app.set_status(format!("Confirmation required: {}", tool_name));
                        }
                        StreamChunk::ToolBlocked { tool_name, reason } => {
                            // In CLI TUI, show blocked tool as error
                            stream_state.content.push_str(&format!(
                                "\n🚫 Tool '{}' blocked: {}\n",
                                tool_name, reason
                            ));
                            app.update_last_message(&stream_state.content);
                            app.set_status(format!("Tool blocked: {}", tool_name));
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
                        StreamChunk::TokenUsageUpdate {
                            estimated,
                            limit,
                            percentage,
                            status,
                            estimated_cost,
                        } => {
                            // Display token usage in status bar
                            let status_icon = match status {
                                gestura_core::streaming::TokenUsageStatus::Green => "🟢",
                                gestura_core::streaming::TokenUsageStatus::Yellow => "🟡",
                                gestura_core::streaming::TokenUsageStatus::Red => "🔴",
                            };
                            app.set_status(format!(
                                "{} Tokens: {}/{} ({}%) - ${:.4}",
                                status_icon, estimated, limit, percentage, estimated_cost
                            ));
                        }
                        StreamChunk::Error(err) => {
                            let error_msg = format!("Stream error: {}", err);
                            app.set_error(&error_msg);

                            // Push critical error as visible message in chat
                            // (connection failures, API quota exceeded, etc.)
                            app.push_error_message(format!("⚠️ {}", error_msg));

                            if !stream_state.content.is_empty() {
                                app.update_last_message(&format!(
                                    "{}\n\n[Error: {}]",
                                    stream_state.content, err
                                ));
                                app.mark_last_message_error();
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

        // Process prompt enhancement results (non-blocking)
        if let Some(ref mut enhance_state) = prompt_enhancement {
            let mut completed = false;
            match enhance_state.receiver.try_recv() {
                Ok(Ok(enhanced)) => {
                    // Store original for undo
                    app.original_prompt = Some(enhance_state.original_input.clone());
                    // Replace with enhanced prompt
                    app.input = enhanced;
                    app.cursor_pos = app.input.len();
                    app.set_status(format!(
                        "✨ Prompt enhanced! (was {} chars, now {} chars) - Press Cmd+Z to undo",
                        enhance_state.original_input.len(),
                        app.input.len()
                    ));
                    completed = true;
                }
                Ok(Err(e)) => {
                    app.set_error(format!("Enhancement failed: {}", e));
                    completed = true;
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    app.set_error("Enhancement task ended unexpectedly".to_string());
                    completed = true;
                }
            }

            if completed {
                prompt_enhancement = None;
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
                    if streaming.is_some() && !command_allowed_while_streaming(&cmd) {
                        app.set_status(
                            "Still streaming — press Esc to cancel before running commands"
                                .to_string(),
                        );
                    } else if let Some(Action::Quit) = handle_command(app, &cmd)? {
                        break;
                    }
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
                Action::ToggleRecording => {
                    // Voice recording toggle - not implemented in CLI TUI
                    app.set_status("Voice recording not available in CLI mode");
                }
                Action::EnhancePrompt => {
                    // Don't enhance while streaming, while enhancement is already running, or if input is empty.
                    if streaming.is_some() {
                        app.set_status(
                            "Can't enhance prompt while streaming (press Esc to cancel)"
                                .to_string(),
                        );
                    } else if prompt_enhancement.is_some() {
                        app.set_status("Enhancement already in progress...".to_string());
                    } else if !app.input.is_empty() {
                        let original_input = app.input.clone();
                        let original_input_for_state = original_input.clone();
                        app.set_status("Enhancing prompt...".to_string());

                        // Build context from session history
                        let session_history: Vec<(String, String)> = app
                            .session
                            .state
                            .messages
                            .iter()
                            .rev()
                            .take(5) // Last 5 messages to avoid token overflow
                            .rev()
                            .map(|msg| (msg.role.clone(), msg.content.clone()))
                            .collect();

                        let (tx, rx) = mpsc::channel::<std::result::Result<String, String>>(1);
                        rt.spawn(async move {
                            use gestura_core::prompt_enhancement::{
                                PromptContext, enhance_prompt_with_llm,
                            };

                            let cfg = gestura_core::config::AppConfig::load_async().await;
                            let context = if session_history.is_empty() {
                                None
                            } else {
                                Some(PromptContext::new().with_session_history(session_history))
                            };

                            let res = enhance_prompt_with_llm(&original_input, &cfg, context)
                                .await
                                .map_err(|e| e.to_string());
                            let _ = tx.send(res).await;
                        });

                        prompt_enhancement = Some(PromptEnhancementState {
                            receiver: rx,
                            original_input: original_input_for_state,
                        });
                    } else {
                        app.set_status("Please enter a prompt first".to_string());
                    }
                }
                Action::Continue => {}
            }
        }
    }

    Ok(())
}

/// Start streaming a message to the LLM (non-blocking) via AgentPipeline
fn start_streaming_message(
    app: &mut TuiApp,
    rt: &tokio::runtime::Runtime,
    message: &str,
) -> Result<Option<StreamingState>> {
    // Handle explicit /tools command only (not natural language questions)
    // Natural language questions should go through the LLM for dynamic, session-aware responses
    if message.trim().starts_with("/tools") {
        app.add_message("user", message);
        let response = crate::tool_registry::render_tools_overview();
        app.add_message("assistant", &response);
        return Ok(None);
    }

    // Add user message
    app.add_message("user", message);
    app.is_loading = true;
    app.set_status("Streaming via AgentPipeline...");

    // Add placeholder for streaming response
    app.add_streaming_message();

    // Build conversation history for the pipeline
    let msg_count = app.session.state.messages.len();
    let history: Vec<gestura_core::Message> = app
        .session
        .state
        .messages
        .iter()
        .take(msg_count.saturating_sub(1))
        .rev()
        .take(10)
        .rev()
        .map(|msg| msg.to_pipeline_message())
        .collect();

    // Build the agent request
    let mut request = AgentRequest::new(message)
        .with_streaming(true)
        .with_source(RequestSource::CliTui);

    // Add system prompt if available
    if let Some(ref sys) = app.system_prompt {
        request = request.with_system_prompt(sys.clone());
    }

    // Attach session environment metadata for agent awareness.
    request = request.with_session(app.session.id.clone());
    if let Some(ws) = app.session.workspace_dir() {
        request = request.with_workspace(ws.clone());
    }
    let provider_name = app.config.llm.primary.clone();
    let model_name = app
        .session
        .model
        .clone()
        .or_else(|| model_for_provider(&app.config, &provider_name))
        .unwrap_or_default();
    let (permission_level, allowed_tools) = super::derive_request_policy(&app.session);
    request = request
        .with_session_llm_config(provider_name, model_name)
        .with_permission_level(permission_level);
    if !allowed_tools.is_empty() {
        request = request.with_allowed_tools(allowed_tools);
    }

    // Add conversation history
    request = request.with_history(history);

    // Create channel and cancellation token
    let (tx, rx) = mpsc::channel::<StreamChunk>(100);
    let cancel_token = CancellationToken::new();
    let cancel_token_clone = cancel_token.clone();
    let config = app.config.clone();

    // Spawn streaming task using AgentPipeline
    rt.spawn(async move {
        let pipeline = AgentPipeline::new(config);
        if let Err(e) = pipeline
            .process_streaming(request, tx.clone(), cancel_token_clone)
            .await
        {
            let _ = tx.send(StreamChunk::Error(e.to_string())).await;
        }
    });

    Ok(Some(StreamingState {
        receiver: rx,
        cancel_token,
        content: String::new(),
        thinking_content: String::new(),
    }))
}

/// Handle slash commands.
///
/// Returns an optional `Action` for commands that should affect the main loop.
fn handle_command(app: &mut TuiApp, command: &str) -> Result<Option<Action>> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let cmd = parts.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let args = &parts[1..];
    let confirmed = args.contains(&"--confirmed");

    match cmd.as_str() {
        "/quit" | "/q" | "/exit" => {
            // Quit via command (save-on-exit handled by run_tui)
            return Ok(Some(Action::Quit));
        }
        "/quit!" => {
            // Explicit quit without save requires confirmation.
            app.show_confirm(ConfirmAction::QuitWithoutSave);
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
                        thinking: None,
                        is_streaming: false,
                        is_error: false,
                    });
                } else {
                    app.set_error(format!("Unknown tool: {}", name));
                }
            } else {
                app.active_tab = 2; // Switch to tools tab
            }
        }
        "/clear" => {
            app.show_confirm(ConfirmAction::ClearMessages);
        }
        "/save" => {
            super::save_cli_session(&app.session)?;
            app.set_status("Session saved");
        }
        "/new" => {
            if confirmed {
                super::save_cli_session(&app.session)?;
                let mut new_session = super::new_cli_session(app.session.model.clone())?;
                if super::ensure_session_tool_settings(&mut new_session, &app.config) {
                    super::save_cli_session(&new_session)?;
                }
                app.session = new_session;
                app.messages.clear();
                app.message_list_state.select(None);
                app.set_status(format!("New session: {}", &app.session.id[..8]));
            } else {
                app.show_confirm(ConfirmAction::NewSession);
            }
        }
        "/history" => {
            let user_count = app.messages.iter().filter(|m| m.role == "user").count();
            let asst_count = app
                .messages
                .iter()
                .filter(|m| m.role == "assistant")
                .count();
            app.set_status(format!(
                "Session: {} | Messages: {} (you: {}, AI: {})",
                &app.session.id[..8],
                app.messages.len(),
                user_count,
                asst_count
            ));
        }
        "/settings" => {
            app.active_tab = 3; // Switch to settings tab
        }
        "/capabilities" => {
            let caps = crate::tool_registry::render_capabilities();
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: caps,
                thinking: None,
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
        "/search" | "/find" => {
            if args.is_empty() {
                // Enter interactive search mode
                app.start_search();
            } else {
                // Programmatic search with query
                let query = args.join(" ");
                app.update_search(&query);
                app.mode = TuiMode::Search;
                let match_count = app.search_matches.len();
                if match_count > 0 {
                    app.set_status(format!("Found {} matches for '{}'", match_count, query));
                } else {
                    app.set_status(format!("No matches for '{}'", query));
                }
            }
        }
        "/sessions" | "/session" => {
            handle_session_command(app, args)?;
        }
        "/workflow" | "/workflows" => {
            handle_workflow_command(app, args, &get_workflows_dir())?;
        }
        _ => {
            app.set_error(format!("Unknown command: {}", cmd));
        }
    }

    Ok(None)
}

/// Handle session management commands
fn handle_session_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    let subcommand = args.first().map(|s| s.to_lowercase());

    match subcommand.as_deref() {
        None | Some("list") => {
            // Check for date filter: /session list today|week|month
            let filter = args.get(1).map(|s| s.to_lowercase());
            let session_filter = match filter.as_deref() {
                Some("today") => super::SessionFilter::Today,
                Some("week") | Some("thisweek") => super::SessionFilter::ThisWeek,
                Some("month") | Some("thismonth") => super::SessionFilter::ThisMonth,
                _ => super::SessionFilter::All,
            };

            let filter_label = match &session_filter {
                super::SessionFilter::All => String::new(),
                super::SessionFilter::Today => " (today)".to_string(),
                super::SessionFilter::ThisWeek => " (this week)".to_string(),
                super::SessionFilter::ThisMonth => " (this month)".to_string(),
                super::SessionFilter::DateRange { from, to } => match (from, to) {
                    (Some(f), Some(t)) => {
                        format!(" ({} to {})", f.format("%Y-%m-%d"), t.format("%Y-%m-%d"))
                    }
                    (Some(f), None) => format!(" (from {})", f.format("%Y-%m-%d")),
                    (None, Some(t)) => format!(" (until {})", t.format("%Y-%m-%d")),
                    (None, None) => String::new(),
                },
            };

            // List sessions with filter
            match super::list_sessions_filtered(session_filter) {
                Ok(sessions) => {
                    if sessions.is_empty() {
                        app.set_status(format!("No saved sessions found{}", filter_label));
                    } else {
                        let mut content = format!("📁 Saved Sessions{}:\n\n", filter_label);
                        for (i, session) in sessions.iter().take(10).enumerate() {
                            let is_current = session.id == app.session.id;
                            let marker = if is_current { "▶ " } else { "  " };
                            let model_info = session.model.as_deref().unwrap_or("default");
                            content.push_str(&format!(
                                "{}{}. {} ({} msgs, {})\n   Created: {} | Updated: {}\n\n",
                                marker,
                                i + 1,
                                &session.id[..8],
                                session.message_count,
                                model_info,
                                session
                                    .created_at
                                    .with_timezone(&Local)
                                    .format("%Y-%m-%d %H:%M"),
                                session
                                    .last_active
                                    .with_timezone(&Local)
                                    .format("%Y-%m-%d %H:%M")
                            ));
                        }
                        content.push_str("\nFilters: /session list today|week|month");
                        content.push_str("\nCommands: /session load <id> | /session delete <id> | /session export <id>");
                        app.messages.push(app::TuiMessage {
                            role: "system".to_string(),
                            content,
                            thinking: None,
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
                if let Err(e) = super::save_cli_session(&app.session) {
                    app.set_error(format!("Failed to save current session: {}", e));
                    return Ok(());
                }

                // Try to load the session (support partial ID matching)
                match find_session_by_prefix(id) {
                    Ok(Some(full_id)) => {
                        match super::load_cli_session(&full_id) {
                            Ok(mut session) => {
                                if super::ensure_session_tool_settings(&mut session, &app.config) {
                                    super::save_cli_session(&session)?;
                                }
                                // Convert session messages to TUI messages
                                app.messages = session
                                    .state
                                    .messages
                                    .iter()
                                    .map(|m| app::TuiMessage {
                                        role: m.role.clone(),
                                        content: m.content.clone(),
                                        thinking: m.thinking.clone(),
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
                    Ok(Some(full_id)) => match super::delete_cli_session(&full_id) {
                        Ok(true) => {
                            app.set_status(format!("Deleted session: {}", &full_id[..8]));
                        }
                        Ok(false) => {
                            app.set_error(format!("Session not found: {}", id));
                        }
                        Err(e) => {
                            app.set_error(format!("Failed to delete session: {}", e));
                        }
                    },
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
                let export_path = args.get(2).map(std::path::PathBuf::from);

                match find_session_by_prefix(id) {
                    Ok(Some(full_id)) => match super::load_cli_session(&full_id) {
                        Ok(session) => {
                            let path = export_path.unwrap_or_else(|| {
                                std::path::PathBuf::from(format!("session_{}.json", &full_id[..8]))
                            });
                            match super::export_cli_session(&session, &path) {
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
                    },
                    Ok(None) => {
                        app.set_error(format!("Session not found: {}", id));
                    }
                    Err(e) => {
                        app.set_error(format!("Error finding session: {}", e));
                    }
                }
            } else {
                // Export current session
                let path =
                    std::path::PathBuf::from(format!("session_{}.json", &app.session.id[..8]));
                match super::export_cli_session(&app.session, &path) {
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
            let asst_count = app
                .messages
                .iter()
                .filter(|m| m.role == "assistant")
                .count();
            let content = format!(
                "📋 Current Session Info:\n\n\
                 ID: {}\n\
                 Created: {}\n\
                 Updated: {}\n\
                 Model: {}\n\
                 Messages: {} (you: {}, AI: {})",
                session.id,
                session
                    .created_at
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S"),
                session
                    .last_active
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S"),
                session.model.as_deref().unwrap_or("default"),
                app.messages.len(),
                user_count,
                asst_count
            );
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content,
                thinking: None,
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

fn get_workflows_dir() -> std::path::PathBuf {
    // Check .agent/workflows in current directory
    let current = std::path::PathBuf::from(".agent/workflows");
    if current.exists() {
        return current;
    }
    // Fallback to gestura config dir
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("gestura")
        .join("workflows")
}

fn model_for_provider(cfg: &gestura_core::AppConfig, provider: &str) -> Option<String> {
    match provider {
        "openai" => cfg.llm.openai.as_ref().map(|c| c.model.clone()),
        "anthropic" => cfg.llm.anthropic.as_ref().map(|c| c.model.clone()),
        "grok" => cfg.llm.grok.as_ref().map(|c| c.model.clone()),
        "ollama" => cfg.llm.ollama.as_ref().map(|c| c.model.clone()),
        _ => None,
    }
}

fn load_workflows(app: &mut TuiApp) {
    let dir = get_workflows_dir();
    app.workflows.clear();

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "md")
                && let Some(name) = path.file_stem().and_then(|s| s.to_str())
            {
                // Try to read description from frontmatter
                let desc = if let Ok(content) = std::fs::read_to_string(&path) {
                    content
                        .lines()
                        .find(|l| l.starts_with("description:"))
                        .map(|l| l.trim_start_matches("description:").trim().to_string())
                        .unwrap_or_else(|| "No description".to_string())
                } else {
                    "No description".to_string()
                };
                app.workflows.push((name.to_string(), desc));
            }
        }
    }
    app.workflows.sort_by(|a, b| a.0.cmp(&b.0));
}

fn handle_workflow_command(app: &mut TuiApp, args: &[&str], dir: &std::path::Path) -> Result<()> {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("list") => {
            load_workflows(app);
            app.active_tab = 1; // Switch to workflows tab
            app.set_status(format!("Found {} workflows", app.workflows.len()));
        }
        Some("run") => {
            if let Some(name) = args.get(1) {
                let filename = if name.ends_with(".md") {
                    name.to_string()
                } else {
                    format!("{}.md", name)
                };
                let path = dir.join(&filename);
                if path.exists() {
                    match std::fs::read_to_string(path) {
                        Ok(content) => {
                            // Strip frontmatter if present
                            let prompt = if let Some(stripped) = content.strip_prefix("---") {
                                if let Some(end_idx) = stripped.find("---") {
                                    stripped[end_idx + 3..].trim().to_string()
                                } else {
                                    content
                                }
                            } else {
                                content
                            };

                            app.set_status(format!("Running workflow: {}", name));
                            app.active_tab = 0; // Switch to chat

                            // Inject as user message
                            // Note: Real workflow engine might do more, but for now we treat it as a prompt template
                            app.input = prompt;
                            // Trigger send automatically (simulated)
                            // We can't easily trigger Action::SendMessage here as we are inside handle_command
                            // So we just leave it in input for user to press Enter?
                            // Or better: Let's populate input and user can verify before running.
                            app.cursor_pos = app.input.len();
                        }
                        Err(e) => app.set_error(format!("Failed to read workflow: {}", e)),
                    }
                } else {
                    app.set_error(format!("Workflow not found: {}", name));
                }
            } else {
                app.set_error("Usage: /workflow run <name>");
            }
        }
        Some(cmd) => app.set_error(format!("Unknown workflow command: {}", cmd)),
    }
    Ok(())
}

/// Format tool output for TUI with pretty printing for JSON
fn format_tool_output_tui(output: &str) -> String {
    if output.is_empty() {
        return "     (completed)".to_string();
    }

    // Try to parse as JSON and pretty print
    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(output) {
        if let Ok(pretty) = serde_json::to_string_pretty(&json_value) {
            // For TUI, show more content (500 chars) since it has scrolling
            if pretty.len() > 500 {
                let truncated = &pretty[..500];
                if let Some(last_newline) = truncated.rfind('\n') {
                    format!(
                        "     {}\n     ... ({} more chars)",
                        pretty[..last_newline].replace('\n', "\n     "),
                        pretty.len() - last_newline
                    )
                } else {
                    format!(
                        "     {}...\n     ({} more chars)",
                        truncated.replace('\n', "\n     "),
                        pretty.len() - 500
                    )
                }
            } else {
                // Indent each line for better readability
                format!("     {}", pretty.replace('\n', "\n     "))
            }
        } else {
            truncate_output_tui(output, 200)
        }
    } else {
        truncate_output_tui(output, 200)
    }
}

/// Truncate output for TUI
fn truncate_output_tui(output: &str, max_len: usize) -> String {
    if output.len() > max_len {
        format!(
            "     {}...\n     ({} more chars)",
            output[..max_len].replace('\n', "\n     "),
            output.len() - max_len
        )
    } else {
        format!("     {}", output.replace('\n', "\n     "))
    }
}
