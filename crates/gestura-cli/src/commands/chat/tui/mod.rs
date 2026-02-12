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
mod markdown;
mod ui;
mod widgets;

pub use app::{Action, TuiApp, TuiMode};

use app::{ConfirmAction, PendingToolConfirmation};

use std::io;
use std::time::Duration;

use chrono::Local;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use gestura_core::{
    AgentPipeline, AgentRequest, AppConfig, AppConfigSecurityExt, CancellationToken, RequestSource,
    StreamChunk,
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use super::{ChatOptions, Result};

/// Resolve the session-scoped LLM override for the current TUI session.
///
/// Precedence:
/// 1) `app.session.state.llm_config` (canonical persisted override)
/// 2) legacy `app.session.model` (CLI-style selector string)
fn resolve_session_llm_override(
    app: &TuiApp,
) -> Option<gestura_core::chat_sessions::SessionLlmConfig> {
    if let Some(cfg) = app.session.state.llm_config.as_ref() {
        return Some(cfg.clone());
    }

    app.session
        .model
        .as_deref()
        .and_then(gestura_core::llm_overrides::session_llm_config_from_cli_model_arg)
}

/// Return true if the token looks like a CLI flag (e.g. `--confirmed`).
fn is_flag_token(tok: &str) -> bool {
    tok.trim_start().starts_with("--")
}

/// Get the first non-flag argument from a slice of parsed command args.
fn first_non_flag_arg<'a>(args: &'a [&'a str]) -> Option<&'a str> {
    args.iter().copied().find(|a| !is_flag_token(a))
}

/// Compute the best-effort provider+model selection for the current TUI session.
///
/// Precedence:
/// 1) `session.state.llm_config` (session-scoped overrides)
/// 2) legacy `session.model` hint (supports `provider:model`)
/// 3) `app.config.llm.primary` + provider default model
fn effective_provider_model_for_ui(app: &TuiApp) -> (String, String) {
    if let Some(cfg) = app.session.state.llm_config.as_ref() {
        let provider = cfg.provider.as_deref().unwrap_or("").trim().to_string();
        let model = cfg.model.as_deref().unwrap_or("").trim().to_string();
        if !provider.is_empty() {
            let effective_model = if !model.is_empty() {
                model
            } else {
                model_for_provider(&app.config, &provider).unwrap_or_default()
            };
            return (provider, effective_model);
        }
    }

    if let Some(m) = app.session.model.as_deref() {
        let m = m.trim();
        if let Some((p, model)) = m.split_once(':') {
            let p = p.trim();
            let model = model.trim();
            if !p.is_empty() {
                return (p.to_string(), model.to_string());
            }
        }
    }

    let provider = app.config.llm.primary.clone();
    let model = model_for_provider(&app.config, &provider).unwrap_or_default();
    (provider, model)
}

/// Populate and open the model picker overlay.
///
/// Fetches the dynamic model list for each configured provider (cached after
/// the first call) and builds picker items for every available model. The
/// currently active model is marked with a `●` prefix.
fn open_model_picker(app: &mut TuiApp, rt: &tokio::runtime::Runtime) {
    let primary = app.config.llm.primary.clone();
    let mut items: Vec<app::ModelPickerItem> = Vec::new();

    // Keep ordering predictable (primary provider first).
    let mut providers: Vec<&str> = Vec::new();
    providers.push(primary.as_str());
    for p in ["openai", "anthropic", "grok", "gemini", "ollama"] {
        if p != primary.as_str() {
            providers.push(p);
        }
    }
    providers.dedup();

    // Determine the currently active provider+model so we can mark it.
    let (active_provider, active_model) = effective_provider_model_for_ui(app);

    for provider in providers {
        // Only show providers that have a config section.
        if model_for_provider(&app.config, provider).is_none() {
            continue;
        }

        // Fetch or reuse cached model list for this provider.
        let models = if let Some(cached) = app.cached_model_lists.get(provider) {
            cached.clone()
        } else {
            let (api_key, base_url) = tui_api_key_and_base_url(&app.config, provider);
            let key_ref = api_key.as_deref();
            let url_ref = base_url.as_deref();
            let fetched = rt
                .block_on(gestura_core::list_models_for_provider(
                    provider, key_ref, url_ref,
                ))
                .unwrap_or_else(|_| gestura_core::static_models_for_provider(provider));
            app.cached_model_lists
                .insert(provider.to_string(), fetched.clone());
            fetched
        };

        if models.is_empty() {
            // Fallback: show at least the configured default model.
            if let Some(model) = model_for_provider(&app.config, provider) {
                let model = model.trim().to_string();
                if !model.is_empty() {
                    let active = provider == active_provider && model == active_model;
                    let prefix = if active { "● " } else { "  " };
                    items.push(app::ModelPickerItem {
                        label: format!("{prefix}{provider}:{model}"),
                        provider: provider.to_string(),
                        model,
                    });
                }
            }
            continue;
        }

        for m in &models {
            let active = provider == active_provider && m.id == active_model;
            let prefix = if active { "● " } else { "  " };
            items.push(app::ModelPickerItem {
                label: format!("{prefix}{provider}:{}", m.id),
                provider: provider.to_string(),
                model: m.id.clone(),
            });
        }
    }

    app.model_picker_state.items = items;
    app.model_picker_state.reset();

    // Preselect the effective provider/model when possible.
    if let Some(pos) = app.model_picker_state.filtered.iter().position(|idx| {
        app.model_picker_state
            .items
            .get(*idx)
            .is_some_and(|it| it.provider == active_provider && it.model == active_model)
    }) {
        app.model_picker_state.list_state.select(Some(pos));
    }

    app.mode = TuiMode::ModelPicker;
    app.set_status("Model picker: type to filter, Enter to select, Esc to close");
}

/// Apply a `/model` selection to the current session and persist it.
///
/// Accepted forms:
/// - `provider:model` (explicit)
/// - `provider` (provider only; selects provider default model)
/// - `model` (model id only; will infer provider from model prefix when possible)
fn apply_model_selection(app: &mut TuiApp, spec: &str, rt: &tokio::runtime::Runtime) -> Result<()> {
    let spec = spec.trim();
    if spec.is_empty() {
        open_model_picker(app, rt);
        return Ok(());
    }

    let (mut provider, mut model) = if let Some((p, m)) = spec.split_once(':') {
        (p.trim().to_string(), m.trim().to_string())
    } else {
        // Distinguish provider-only vs model-only.
        match spec.to_ascii_lowercase().as_str() {
            "openai" | "anthropic" | "grok" | "gemini" | "ollama" => {
                let p = spec.trim().to_string();
                let m = model_for_provider(&app.config, &p).unwrap_or_default();
                (p, m)
            }
            _ => {
                // Model-only. Prefer inferred provider, else keep the current provider.
                let inferred = gestura_core::llm_validation::infer_provider_from_model_id(spec)
                    .map(|p| p.to_string());

                let current_provider = app
                    .session
                    .state
                    .llm_config
                    .as_ref()
                    .and_then(|c| c.provider.as_deref())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| app.config.llm.primary.clone());

                (inferred.unwrap_or(current_provider), spec.to_string())
            }
        }
    };

    provider = provider.trim().to_string();
    model = model.trim().to_string();
    if provider.is_empty() {
        app.set_error("Model selection is missing provider".to_string());
        return Ok(());
    }
    if model.is_empty() {
        app.set_error("Model selection is missing model".to_string());
        return Ok(());
    }

    if let Err(msg) = gestura_core::llm_validation::validate_model_for_provider(&provider, &model) {
        app.set_error(msg);
        return Ok(());
    }

    app.session.state.llm_config = Some(gestura_core::chat_sessions::SessionLlmConfig {
        provider: Some(provider.clone()),
        model: Some(model.clone()),
    });
    // Keep the legacy hint in sync for compatibility across CLI modes.
    app.session.model = Some(format!("{}:{}", provider, model));

    super::save_cli_session(&app.session)?;
    app.mode = TuiMode::Insert;
    app.set_status(format!("Model set: {}:{}", provider, model));
    Ok(())
}

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
    /// Original user input for pause-state capture.
    original_input: String,
    /// Conversation history snapshot for pause-state capture.
    history: Vec<gestura_core::Message>,
    /// System prompt in effect for pause-state capture.
    system_prompt: Option<String>,
    /// Completed tool calls tracked for pause-state capture.
    completed_tool_calls: Vec<gestura_core::ToolCallRecord>,
    /// In-progress tool call being built (id, name, accumulated args).
    current_tool_call: Option<(String, String, String)>,
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

/// Truncate a string by *character count* for safe UI previews.
///
/// This avoids panics from slicing UTF-8 strings on non-char boundaries.
fn truncate_for_preview(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (count, ch) in s.chars().enumerate() {
        if count >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

/// Append an entry to the agent activity transcript.
///
/// This is used to keep non-response streaming events (tool calls, retries, token usage, etc.)
/// out of the main assistant transcript while still exposing them to the user.
fn push_activity_entry(tui: &mut TuiApp, text: impl Into<String>, is_error: bool) {
    tui.activity_state.push(app::ActivityEntry {
        text: text.into(),
        is_error,
    });
}

/// Append an informational activity entry.
fn push_activity_info(tui: &mut TuiApp, text: impl Into<String>) {
    push_activity_entry(tui, text, false);
}

/// Append an error activity entry.
fn push_activity_error(tui: &mut TuiApp, text: impl Into<String>) {
    push_activity_entry(tui, text, true);
}

/// Format a token usage line suitable for both the status bar and the activity transcript.
fn format_token_usage_line(
    estimated: usize,
    limit: usize,
    percentage: u8,
    status: gestura_core::streaming::TokenUsageStatus,
    estimated_cost: f64,
) -> String {
    let status_icon = match status {
        gestura_core::streaming::TokenUsageStatus::Green => "🟢",
        gestura_core::streaming::TokenUsageStatus::Yellow => "🟡",
        gestura_core::streaming::TokenUsageStatus::Red => "🔴",
    };

    format!(
        "{} Tokens: {}/{} ({}%) - ${:.4}",
        status_icon, estimated, limit, percentage, estimated_cost
    )
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
    // Enable mouse capture so we receive wheel-scroll and click/drag events (used for in-app
    // transcript scrolling and text selection). Bracketed paste is also enabled so terminals
    // emit `Event::Paste(...)` for reliable paste support in raw mode.
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
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
        DisableMouseCapture,
        DisableBracketedPaste
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

        // Advance the thinking-spinner animation counter each frame while loading.
        if app.is_loading {
            app.loading_tick = app.loading_tick.wrapping_add(1);
        }

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
                        StreamChunk::Status { message } => {
                            let msg = message;
                            app.set_status(msg.clone());
                            push_activity_info(app, format!("ℹ️ {}", msg));
                        }
                        StreamChunk::Text(text) => {
                            stream_state.content.push_str(&text);
                            app.update_last_message(&stream_state.content);
                        }
                        StreamChunk::ToolCallStart { id, name } => {
                            // Track in-progress tool call for pause-state capture.
                            stream_state.current_tool_call =
                                Some((id, name.clone(), String::new()));
                            push_activity_info(app, format!("🔧 Using tool: {}", name));
                            app.set_status(format!("Using tool: {}", name));
                        }
                        StreamChunk::ToolCallArgs(args) => {
                            // Accumulate args for pause-state capture.
                            if let Some((_, _, ref mut acc)) = stream_state.current_tool_call {
                                acc.push_str(&args);
                            }
                            let preview = truncate_for_preview(&args, 160);
                            push_activity_info(app, format!("  ⏳ Args: {}", preview));
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
                            // Record completed tool call for pause-state capture.
                            let (tc_id, tc_name, tc_args) =
                                stream_state.current_tool_call.take().unwrap_or_default();
                            let result = if success {
                                gestura_core::ToolResult::Success(output.clone())
                            } else {
                                gestura_core::ToolResult::Error(output.clone())
                            };
                            stream_state
                                .completed_tool_calls
                                .push(gestura_core::ToolCallRecord {
                                    id: tc_id,
                                    name: tc_name,
                                    arguments: tc_args,
                                    result,
                                    duration_ms,
                                });

                            let formatted = format_tool_output_tui(&output);
                            let (prefix, is_error) = if success {
                                ("✅", false)
                            } else {
                                ("❌", true)
                            };
                            push_activity_entry(
                                app,
                                format!(
                                    "  {} {} ({}ms):\n{}",
                                    prefix, name, duration_ms, formatted
                                ),
                                is_error,
                            );
                            app.set_status(format!(
                                "Tool {}: {}",
                                if success { "ok" } else { "failed" },
                                name
                            ));
                        }
                        StreamChunk::RetryAttempt {
                            attempt,
                            max_attempts,
                            delay_ms,
                            error_message,
                        } => {
                            let preview = truncate_for_preview(&error_message, 200);
                            push_activity_info(
                                app,
                                format!(
                                    "🔁 Retry {}/{} in {}ms: {}",
                                    attempt, max_attempts, delay_ms, preview
                                ),
                            );
                            app.set_status(format!("Retrying ({}/{})", attempt, max_attempts));
                        }
                        StreamChunk::ContextCompacted {
                            messages_before,
                            messages_after,
                            tokens_saved,
                            summary,
                        } => {
                            let mut line = format!(
                                "📦 Context compacted: {} → {} messages ({} tokens saved)",
                                messages_before, messages_after, tokens_saved
                            );
                            if !summary.trim().is_empty() {
                                let preview =
                                    truncate_for_preview(&summary.replace('\n', " "), 240);
                                line.push_str(&format!("\n  Summary: {}", preview));
                            }
                            push_activity_info(app, line);
                            app.set_status("Context compacted".to_string());
                        }
                        StreamChunk::MemoryBankSaved {
                            file_path,
                            session_id,
                            summary,
                            messages_saved,
                        } => {
                            let mut line = format!(
                                "💾 Memory bank saved: {} messages\n  File: {}",
                                messages_saved, file_path
                            );
                            if !summary.trim().is_empty() {
                                let preview =
                                    truncate_for_preview(&summary.replace('\n', " "), 240);
                                line.push_str(&format!("\n  Summary: {}", preview));
                            }
                            push_activity_info(app, line);
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
                            app.set_status(msg.clone());
                            push_activity_info(app, msg);
                        }
                        StreamChunk::ToolConfirmationRequired {
                            confirmation_id,
                            tool_name,
                            tool_args,
                            description,
                            risk_level,
                            category,
                            ..
                        } => {
                            let pending = PendingToolConfirmation {
                                confirmation_id,
                                tool_name,
                                tool_args,
                                description,
                                risk_level,
                                category,
                            };

                            app.activity_state.push(app::ActivityEntry {
                                text: format!(
                                    "⚠️ Tool '{}' requires confirmation: {}",
                                    pending.tool_name, pending.description
                                ),
                                is_error: false,
                            });
                            app.set_status(format!("Confirmation required: {}", pending.tool_name));
                            app.show_tool_confirmation(pending);
                        }
                        StreamChunk::ToolBlocked { tool_name, reason } => {
                            app.activity_state.push(app::ActivityEntry {
                                text: format!("🚫 Tool '{}' blocked: {}", tool_name, reason),
                                is_error: true,
                            });
                            app.set_status(format!("Tool blocked: {}", tool_name));
                        }
                        StreamChunk::AgentLoopIteration { iteration } => {
                            if iteration > 0 {
                                push_activity_info(
                                    app,
                                    format!(
                                        "◆ Iteration {} — reviewing tool results…",
                                        iteration + 1
                                    ),
                                );
                                app.set_status("Reviewing tool results…".to_string());
                            }
                        }
                        StreamChunk::ShellOutput { data, .. } => {
                            // Show streaming shell output in the activity pane.
                            let preview = truncate_for_preview(&data, 200);
                            push_activity_info(app, format!("  📟 {}", preview));
                        }
                        StreamChunk::ShellLifecycle {
                            state,
                            command,
                            exit_code,
                            ..
                        } => {
                            let msg = if let Some(code) = exit_code {
                                format!("⚙ shell {:?}: {} (exit {})", state, command, code)
                            } else {
                                format!("⚙ shell {:?}: {}", state, command)
                            };
                            push_activity_info(app, msg);
                        }
                        StreamChunk::Cancelled | StreamChunk::Paused => {
                            let is_paused = matches!(chunk, StreamChunk::Paused);

                            // Capture pause state so the session can be resumed later.
                            let paused_state = gestura_core::PausedExecutionState {
                                original_input: stream_state.original_input.clone(),
                                system_prompt: stream_state.system_prompt.clone(),
                                history: stream_state.history.clone(),
                                partial_content: stream_state.content.clone(),
                                partial_thinking: if stream_state.thinking_content.is_empty() {
                                    None
                                } else {
                                    Some(stream_state.thinking_content.clone())
                                },
                                completed_tool_calls: stream_state.completed_tool_calls.clone(),
                                iteration: 0,
                                source: gestura_core::RequestSource::CliTui,
                                session_id: Some(app.session.id.clone()),
                                workspace_dir: app.session.workspace_dir().cloned(),
                                model_snapshot: None,
                                paused_at: chrono::Utc::now(),
                            };
                            app.session.state.paused_execution = Some(paused_state);
                            // Persist so the pause state survives restarts.
                            let _ = super::save_cli_session(&app.session);

                            let label = if is_paused { "Paused" } else { "Cancelled" };
                            push_activity_info(
                                app,
                                format!("⏸ {} — use @continue to resume", label),
                            );
                            if !stream_state.content.is_empty() {
                                app.update_last_message(&format!(
                                    "{}\n\n[{} — use @continue to resume]",
                                    stream_state.content, label
                                ));
                                app.finalize_streaming_message();
                            } else {
                                app.messages.pop();
                            }
                            app.is_loading = false;
                            app.set_status(format!("{} — type @continue to resume", label));
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
                            let line = format_token_usage_line(
                                estimated,
                                limit,
                                percentage,
                                status,
                                estimated_cost,
                            );
                            app.set_status(line.clone());
                            push_activity_info(app, line);
                        }
                        StreamChunk::Error(err) => {
                            let error_msg = format!("Stream error: {}", err);
                            app.set_error(&error_msg);
                            push_activity_error(app, format!("❌ {}", error_msg));

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
                    // Intercept @continue to resume a paused session.
                    if msg.trim().eq_ignore_ascii_case("@continue") {
                        if streaming.is_none() {
                            streaming = start_resume_streaming(app, rt)?;
                        } else {
                            app.set_status(
                                "Response streaming; wait for completion (Esc to cancel) first",
                            );
                        }
                    } else if streaming.is_none() {
                        // Clear any stale pause state when the user sends a new message.
                        if app.session.state.paused_execution.is_some() {
                            app.session.state.paused_execution = None;
                            let _ = super::save_cli_session(&app.session);
                        }
                        streaming = start_streaming_message(app, rt, &msg)?;
                    } else {
                        // Preserve the user's input so it isn't lost while a stream is active.
                        app.input = msg;
                        app.cursor_pos = app.input.len();
                        app.set_status(
							"Response streaming; wait for completion (Esc to cancel), then press Enter",
						);
                    }
                }
                Action::ResumeSession => {
                    if streaming.is_none() {
                        streaming = start_resume_streaming(app, rt)?;
                    } else {
                        app.set_status(
                            "Response streaming; wait for completion (Esc to cancel) first",
                        );
                    }
                }
                Action::ExecuteCommand(cmd) => {
                    if streaming.is_some() && !command_allowed_while_streaming(&cmd) {
                        app.set_status(
                            "Still streaming — press Esc to cancel before running commands"
                                .to_string(),
                        );
                    } else if let Some(action) = handle_command(app, &cmd, rt)? {
                        match action {
                            Action::Quit => break,
                            Action::ResumeSession => {
                                if streaming.is_none() {
                                    streaming = start_resume_streaming(app, rt)?;
                                }
                            }
                            _ => {}
                        }
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
                Action::PageUp => {
                    let page_size = app
                        .layout_areas
                        .messages
                        .map(|r| r.height as usize)
                        .unwrap_or(20);
                    app.page_up(page_size);
                }
                Action::PageDown => {
                    let page_size = app
                        .layout_areas
                        .messages
                        .map(|r| r.height as usize)
                        .unwrap_or(20);
                    app.page_down(page_size);
                }
                Action::CopySelection => {
                    // Determine which rendered line(s) to copy.
                    // If there's a mouse selection range, use that; otherwise
                    // fall back to the currently selected line.
                    let (start, end) =
                        if let (Some(a), Some(e)) = (app.selection_anchor, app.selection_end) {
                            (a.min(e), a.max(e))
                        } else {
                            let sel = app.message_list_state.selected().unwrap_or(0);
                            (sel, sel)
                        };

                    // Join the plain-text content of each rendered line in the range.
                    let line_count = end.saturating_sub(start) + 1;
                    let text: String = (start..=end)
                        .filter_map(|idx| app.rendered_line_texts.get(idx))
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n");

                    if !text.is_empty() {
                        match arboard::Clipboard::new() {
                            Ok(mut clipboard) => match clipboard.set_text(&text) {
                                Ok(()) => {
                                    app.set_status(format!(
                                        "Copied {} line(s) to clipboard",
                                        line_count
                                    ));
                                }
                                Err(e) => {
                                    app.set_status(format!("Clipboard error: {e}"));
                                }
                            },
                            Err(e) => {
                                app.set_status(format!("Clipboard unavailable: {e}"));
                            }
                        }
                    }

                    // Clear selection after copy
                    app.selection_anchor = None;
                    app.selection_end = None;
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
    // Redirect /tools to the interactive tools tab (handled the same as :tools command).
    if message.trim().starts_with("/tools") {
        let parts: Vec<&str> = message.split_whitespace().collect();
        open_tools_tab(app, &parts[1..]);
        return Ok(None);
    }

    // Add user message
    app.add_message("user", message);
    app.is_loading = true;
    app.loading_tick = 0;
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

    // Compute and apply the effective provider/model for this session.
    //
    // IMPORTANT: we apply overrides to the *pipeline config* so the underlying LLM call
    // matches what the UI shows (/model picker), instead of only changing labels.
    let session_llm = resolve_session_llm_override(app);
    let mut config = app.config.clone();
    let effective = gestura_core::llm_overrides::apply_cli_session_llm_overrides(
        &mut config,
        session_llm.as_ref(),
    );
    let provider_name = effective.provider;
    let model_name = if !effective.model.trim().is_empty() {
        effective.model
    } else {
        model_for_provider(&config, &provider_name).unwrap_or_default()
    };
    let (permission_level, allowed_tools) = super::derive_request_policy(&app.session);
    request = request
        .with_session_llm_config(provider_name, model_name)
        .with_permission_level(permission_level);
    if !allowed_tools.is_empty() {
        request = request.with_allowed_tools(allowed_tools);
    }

    // Snapshot history and input for pause-state capture before ownership moves.
    let history_snapshot = history.clone();
    let input_snapshot = message.to_string();
    let system_prompt_snapshot = app.system_prompt.clone();

    // Add conversation history
    request = request.with_history(history);

    // Create channel and cancellation token
    let (tx, rx) = mpsc::channel::<StreamChunk>(100);
    let cancel_token = CancellationToken::new();
    let cancel_token_clone = cancel_token.clone();
    // Spawn streaming task using AgentPipeline
    rt.spawn(async move {
        let pipeline = AgentPipeline::with_provider_optimized_config(config);
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
        original_input: input_snapshot,
        history: history_snapshot,
        system_prompt: system_prompt_snapshot,
        completed_tool_calls: Vec::new(),
        current_tool_call: None,
    }))
}

/// Resume a previously paused streaming session.
///
/// Returns `None` if there is no paused execution state to resume.
fn start_resume_streaming(
    app: &mut TuiApp,
    rt: &tokio::runtime::Runtime,
) -> Result<Option<StreamingState>> {
    let paused = match app.session.state.paused_execution.take() {
        Some(p) => p,
        None => {
            app.set_error("No paused session to resume");
            return Ok(None);
        }
    };

    // Show resume indicator in chat
    app.add_message("system", "⏵ Resuming paused session…");
    app.is_loading = true;
    app.loading_tick = 0;
    app.set_status("Resuming paused session…");
    app.add_streaming_message();

    // Snapshot for the new StreamingState (used if the resumed stream is paused again).
    let input_snapshot = paused.original_input.clone();
    let history_snapshot = paused.history.clone();
    let system_prompt_snapshot = paused.system_prompt.clone();

    // Build the resume request through the core pipeline.
    let mut request = AgentRequest::new(&paused.original_input)
        .with_streaming(true)
        .with_source(gestura_core::RequestSource::CliTui)
        .with_resume_state(paused);

    if let Some(ref sys) = app.system_prompt {
        request = request.with_system_prompt(sys.clone());
    }
    request = request.with_session(app.session.id.clone());
    if let Some(ws) = app.session.workspace_dir() {
        request = request.with_workspace(ws.clone());
    }

    let session_llm = resolve_session_llm_override(app);
    let mut config = app.config.clone();
    let effective = gestura_core::llm_overrides::apply_cli_session_llm_overrides(
        &mut config,
        session_llm.as_ref(),
    );
    let provider_name = effective.provider;
    let model_name = if !effective.model.trim().is_empty() {
        effective.model
    } else {
        model_for_provider(&config, &provider_name).unwrap_or_default()
    };
    let (permission_level, allowed_tools) = super::derive_request_policy(&app.session);
    request = request
        .with_session_llm_config(provider_name, model_name)
        .with_permission_level(permission_level);
    if !allowed_tools.is_empty() {
        request = request.with_allowed_tools(allowed_tools);
    }

    let (tx, rx) = mpsc::channel::<StreamChunk>(100);
    let cancel_token = CancellationToken::new();
    let cancel_token_clone = cancel_token.clone();
    rt.spawn(async move {
        let pipeline = AgentPipeline::with_provider_optimized_config(config);
        if let Err(e) = pipeline
            .process_streaming(request, tx.clone(), cancel_token_clone)
            .await
        {
            let _ = tx.send(StreamChunk::Error(e.to_string())).await;
        }
    });

    // Clear the persisted pause state now that we've resumed.
    let _ = super::save_cli_session(&app.session);

    Ok(Some(StreamingState {
        receiver: rx,
        cancel_token,
        content: String::new(),
        thinking_content: String::new(),
        original_input: input_snapshot,
        history: history_snapshot,
        system_prompt: system_prompt_snapshot,
        completed_tool_calls: Vec::new(),
        current_tool_call: None,
    }))
}

/// Open the interactive tools tab.
///
/// Supports the following argument patterns:
/// - no args        → open list view
/// - `<name>`       → open detail view for that tool
/// - `enable <name>`  / `disable <name>` → toggle and stay on list
fn open_tools_tab(app: &mut TuiApp, args: &[&str]) {
    let tools = gestura_core::tools::all_tools();

    match args {
        // `/tools enable <name>` or `/tools disable <name>`
        [verb, name, ..]
            if verb.eq_ignore_ascii_case("enable") || verb.eq_ignore_ascii_case("disable") =>
        {
            let want_enabled = verb.eq_ignore_ascii_case("enable");
            if let Some(idx) = tools.iter().position(|t| t.name.eq_ignore_ascii_case(name)) {
                let tool_name = tools[idx].name.to_string();
                let settings = app
                    .session
                    .state
                    .tool_settings
                    .get_or_insert_with(Default::default);
                settings
                    .enabled_tools
                    .insert(tool_name.clone(), want_enabled);
                let _ = super::save_cli_session(&app.session);

                // Switch to tools tab with the toggled item selected.
                app.tools_state.select(idx, tools.len());
                app.active_tab = 2;
                app.mode = TuiMode::Tools;
                let label = if want_enabled { "enabled" } else { "disabled" };
                app.set_status(format!("Tool '{}' {}", tool_name, label));
            } else {
                app.set_error(format!("Unknown tool: {}", name));
            }
        }
        // `/tools <name>` → detail view
        [name, ..] => {
            if let Some(idx) = tools.iter().position(|t| t.name.eq_ignore_ascii_case(name)) {
                app.tools_state.select(idx, tools.len());
                app.tools_state.detail_mode = true;
                app.active_tab = 2;
                app.mode = TuiMode::Tools;
                app.set_status(format!(
                    "Tool: {} — Space to toggle, Esc to go back",
                    tools[idx].name
                ));
            } else {
                app.set_error(format!("Unknown tool: {}", name));
            }
        }
        // `/tools` → list view
        [] => {
            app.tools_state.detail_mode = false;
            // Initialise selection if not already set.
            if app.tools_state.list_state.selected().is_none() && !tools.is_empty() {
                app.tools_state.select(0, tools.len());
            }
            app.active_tab = 2;
            app.mode = TuiMode::Tools;
            app.set_status("Tools: ↑/↓ navigate, Enter details, Space toggle, Esc close");
        }
    }
}

/// Handle slash commands.
///
/// Returns an optional `Action` for commands that should affect the main loop.
fn handle_command(
    app: &mut TuiApp,
    command: &str,
    rt: &tokio::runtime::Runtime,
) -> Result<Option<Action>> {
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
        "/activity" => {
            if app.mode == TuiMode::Activity {
                app.mode = TuiMode::Insert;
                app.set_status("Activity closed");
            } else {
                app.activity_state.scroll_to_bottom();
                app.mode = TuiMode::Activity;
                app.set_status("Activity: ↑/↓ to scroll, Esc to close");
            }
        }
        "/model" => {
            if let Some(spec) = first_non_flag_arg(args) {
                apply_model_selection(app, spec, rt)?;
            } else {
                open_model_picker(app, rt);
            }
        }
        "/tools" => {
            open_tools_tab(app, args);
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
            app.mode = TuiMode::Settings;
            app.set_status("Settings: ↑/↓ to navigate, Enter to edit, Esc to return to chat");
        }
        "/capabilities" => {
            let caps = gestura_core::tools::render_capabilities(&app.config);
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
            handle_workflow_command(app, args)?;
        }
        "/config" => {
            handle_config_command(app, args)?;
        }
        "/rewind" => {
            handle_rewind_command(app, args)?;
        }
        "/tasks" => {
            handle_tasks_command(app)?;
        }
        "/hooks" => {
            handle_hooks_command(app)?;
        }
        "/permissions" => {
            handle_permissions_command(app, args)?;
        }
        "/context" => {
            handle_context_command(app)?;
        }
        "/continue" | "/resume" => {
            if app.session.state.paused_execution.is_some() {
                return Ok(Some(Action::ResumeSession));
            } else {
                app.set_error("No paused session to resume");
            }
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

/// Get the global workflow manager instance
fn get_workflow_manager() -> gestura_core::WorkflowManager {
    gestura_core::WorkflowManager::new()
}

fn model_for_provider(cfg: &gestura_core::AppConfig, provider: &str) -> Option<String> {
    match provider {
        "openai" => cfg.llm.openai.as_ref().map(|c| c.model.clone()),
        "anthropic" => cfg.llm.anthropic.as_ref().map(|c| c.model.clone()),
        "grok" => cfg.llm.grok.as_ref().map(|c| c.model.clone()),
        "gemini" => cfg.llm.gemini.as_ref().map(|c| c.model.clone()),
        "ollama" => cfg.llm.ollama.as_ref().map(|c| c.model.clone()),
        _ => None,
    }
}

/// Extract the API key and optional base URL for a provider from config.
///
/// Used by the model picker to pass credentials to `list_models_for_provider`.
fn tui_api_key_and_base_url(
    config: &gestura_core::AppConfig,
    provider: &str,
) -> (Option<String>, Option<String>) {
    match provider {
        "openai" => {
            let cfg = config.llm.openai.as_ref();
            (
                cfg.map(|c| c.api_key.clone()).filter(|k| !k.is_empty()),
                cfg.and_then(|c| c.base_url.clone()),
            )
        }
        "anthropic" => {
            let cfg = config.llm.anthropic.as_ref();
            (
                cfg.map(|c| c.api_key.clone()).filter(|k| !k.is_empty()),
                cfg.and_then(|c| c.base_url.clone()),
            )
        }
        "gemini" => {
            let cfg = config.llm.gemini.as_ref();
            (
                cfg.map(|c| c.api_key.clone()).filter(|k| !k.is_empty()),
                cfg.and_then(|c| c.base_url.clone()),
            )
        }
        "grok" => {
            let cfg = config.llm.grok.as_ref();
            (
                cfg.map(|c| c.api_key.clone()).filter(|k| !k.is_empty()),
                cfg.and_then(|c| c.base_url.clone()),
            )
        }
        "ollama" => {
            let cfg = config.llm.ollama.as_ref();
            (None, cfg.map(|c| c.base_url.clone()))
        }
        _ => (None, None),
    }
}

/// Load workflows from core WorkflowManager into TUI app state
fn load_workflows(app: &mut TuiApp) {
    let manager = get_workflow_manager();
    app.workflows.clear();

    match manager.list_workflows() {
        Ok(workflows) => {
            for workflow in workflows {
                app.workflows.push((workflow.name, workflow.description));
            }
        }
        Err(e) => {
            tracing::warn!("Failed to load workflows: {}", e);
        }
    }
}

/// Handle /workflow slash command - delegates to core WorkflowManager
fn handle_workflow_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    let manager = get_workflow_manager();

    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("list") => {
            load_workflows(app);
            app.active_tab = 1; // Switch to workflows tab
            app.mode = TuiMode::Workflows;
            app.set_status("Workflows: ↑/↓ to navigate, Enter to run, Esc to return to chat");
        }
        Some("run") => {
            if let Some(name) = args.get(1) {
                // Delegate to core WorkflowManager for loading
                match manager.load_workflow(name) {
                    Ok(workflow) => {
                        app.set_status(format!("Running workflow: {}", workflow.name));
                        app.active_tab = 0; // Switch to chat

                        // Inject workflow content as user input
                        // User can review and press Enter to send
                        app.input = workflow.content;
                        app.cursor_pos = app.input.len();
                    }
                    Err(gestura_core::WorkflowError::NotFound(_)) => {
                        app.set_error(format!("Workflow not found: {}", name));
                    }
                    Err(e) => {
                        app.set_error(format!("Failed to load workflow: {}", e));
                    }
                }
            } else {
                app.set_error("Usage: /workflow run <name>");
            }
        }
        Some(cmd) => app.set_error(format!("Unknown workflow command: {}", cmd)),
    }
    Ok(())
}

/// Handle /config slash command for session-only config viewing.
///
/// Subcommands:
/// - `/config` or `/config list` - Show current configuration values
/// - `/config get <key>` - Get a specific config value
/// - `/config keys` - List available config keys
fn handle_config_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    let subcommand = args.first().map(|s| s.to_lowercase());

    match subcommand.as_deref() {
        None | Some("list") => {
            // Show current config summary
            let config = &app.config;
            let mut lines = vec![
                "━━━ Current Configuration ━━━".to_string(),
                String::new(),
                "LLM Settings:".to_string(),
                format!("  primary: {}", config.llm.primary),
            ];

            // Show provider-specific settings if configured
            if let Some(ref openai) = config.llm.openai
                && !openai.model.is_empty()
            {
                lines.push(format!("  openai.model: {}", openai.model));
            }
            if let Some(ref anthropic) = config.llm.anthropic
                && !anthropic.model.is_empty()
            {
                lines.push(format!("  anthropic.model: {}", anthropic.model));
            }
            if let Some(ref grok) = config.llm.grok
                && !grok.model.is_empty()
            {
                lines.push(format!("  grok.model: {}", grok.model));
            }
            if let Some(ref ollama) = config.llm.ollama
                && !ollama.model.is_empty()
            {
                lines.push(format!("  ollama.model: {}", ollama.model));
            }

            lines.push(String::new());
            lines.push("Voice Settings:".to_string());
            lines.push(format!("  provider: {}", config.voice.provider));

            lines.push(String::new());
            lines.push("Pipeline Settings:".to_string());
            lines.push(format!(
                "  max_history_messages: {}",
                config.pipeline.max_history_messages
            ));
            lines.push(format!(
                "  auto_compact_threshold: {}%",
                config.pipeline.auto_compact_threshold_percent
            ));

            lines.push(String::new());
            lines.push("UI:".to_string());
            lines.push(format!("  theme_mode: {}", config.ui.theme_mode));

            lines.push(String::new());
            lines.push("Use /config get <key> for specific values".to_string());
            lines.push("Use /config keys to list all available keys".to_string());

            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
        }
        Some("get") => {
            if let Some(key) = args.get(1) {
                // Delegate to core AppConfig::get() for single source of truth
                if let Some(value) = app.config.get(key) {
                    app.set_status(format!("{} = {}", key, value));
                } else {
                    app.set_error(format!("Unknown config key: {}", key));
                }
            } else {
                app.set_error("Usage: /config get <key>");
            }
        }
        Some("keys") => {
            // Delegate to core AppConfig::list_keys() for single source of truth
            let keys = gestura_core::AppConfig::list_keys();
            let content = format!(
                "━━━ Available Config Keys ━━━\n\n{}\n\nUse /config get <key> to view a value",
                keys.join("\n")
            );
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content,
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
        }
        Some(cmd) => {
            app.set_error(format!(
                "Unknown config subcommand: {}. Use: list, get, keys",
                cmd
            ));
        }
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

// --- Claude Code parity slash command handlers ---

/// Handle `/rewind` command - list and restore checkpoints.
///
/// - `/rewind` or `/rewind list` - List session checkpoints
/// - `/rewind <id>` - Restore session to checkpoint with that id prefix
fn handle_rewind_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    use gestura_core::chat_sessions::FileChatSessionStore;
    use gestura_core::checkpoints::{
        CheckpointManager, CheckpointRetentionPolicy, FileCheckpointStore,
    };
    use gestura_core::tasks::TaskManager;

    let manager = CheckpointManager::new(
        FileCheckpointStore::new_default(),
        CheckpointRetentionPolicy::default(),
    );

    let subcommand = args.first().map(|s| s.to_lowercase());
    match subcommand.as_deref() {
        None | Some("list") => {
            let checkpoints = manager
                .list_session_checkpoints(&app.session.id)
                .unwrap_or_default();
            if checkpoints.is_empty() {
                app.messages.push(app::TuiMessage {
                    role: "system".to_string(),
                    content: "No checkpoints found for this session.\n\nCheckpoints are created automatically before write operations.".to_string(),
                    thinking: None,
                    is_streaming: false,
                    is_error: false,
                });
            } else {
                let mut lines = vec!["━━━ Session Checkpoints ━━━".to_string(), String::new()];
                for cp in &checkpoints {
                    let id_short = &cp.id.to_string()[..8];
                    let label = cp.label.as_deref().unwrap_or("-");
                    let ts = cp.created_at.format("%Y-%m-%d %H:%M:%S UTC");
                    lines.push(format!("  {} | {} | {}", id_short, ts, label));
                }
                lines.push(String::new());
                lines.push("Use /rewind <id> to restore to a checkpoint.".to_string());
                app.messages.push(app::TuiMessage {
                    role: "system".to_string(),
                    content: lines.join("\n"),
                    thinking: None,
                    is_streaming: false,
                    is_error: false,
                });
            }
        }
        Some(id_prefix) => {
            // Find checkpoint by id prefix
            let checkpoints = manager
                .list_session_checkpoints(&app.session.id)
                .unwrap_or_default();
            let found: Vec<_> = checkpoints
                .iter()
                .filter(|cp| cp.id.to_string().starts_with(id_prefix))
                .collect();

            match found.len() {
                0 => {
                    app.set_error(format!("No checkpoint found with id prefix: {}", id_prefix));
                }
                1 => {
                    let cp_id = found[0].id;
                    let session_store = FileChatSessionStore::default();
                    let task_manager = TaskManager::new(
                        dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
                    );
                    match manager.apply_session_checkpoint(&cp_id, &session_store, &task_manager) {
                        Ok(payload) => {
                            // Update app state with restored session
                            app.session = payload.session;
                            app.messages = app
                                .session
                                .state
                                .messages
                                .iter()
                                .map(app::TuiMessage::from)
                                .collect();
                            app.set_status(format!(
                                "Restored checkpoint: {}",
                                &cp_id.to_string()[..8]
                            ));
                        }
                        Err(e) => {
                            app.set_error(format!("Failed to restore checkpoint: {}", e));
                        }
                    }
                }
                _ => {
                    app.set_error(format!(
                        "Ambiguous id prefix '{}': matches {} checkpoints. Use more characters.",
                        id_prefix,
                        found.len()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Handle `/tasks` command - show current task list.
fn handle_tasks_command(app: &mut TuiApp) -> Result<()> {
    use gestura_core::tasks::TaskManager;

    let task_manager =
        TaskManager::new(dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from(".")));
    match task_manager.get_hierarchy(&app.session.id) {
        Ok(hierarchy) => {
            if hierarchy.is_empty() {
                app.messages.push(app::TuiMessage {
                    role: "system".to_string(),
                    content: "No tasks found for this session.\n\nTasks are created by the AI agent during complex workflows.".to_string(),
                    thinking: None,
                    is_streaming: false,
                    is_error: false,
                });
            } else {
                let mut lines = vec!["━━━ Task List ━━━".to_string(), String::new()];
                for (root, subtasks) in &hierarchy {
                    let status_icon = match root.status {
                        gestura_core::TaskStatus::NotStarted => "[ ]",
                        gestura_core::TaskStatus::InProgress => "[/]",
                        gestura_core::TaskStatus::Completed => "[x]",
                        gestura_core::TaskStatus::Cancelled => "[-]",
                    };
                    lines.push(format!("{} {}", status_icon, root.name));
                    for sub in subtasks {
                        let sub_icon = match sub.status {
                            gestura_core::TaskStatus::NotStarted => "[ ]",
                            gestura_core::TaskStatus::InProgress => "[/]",
                            gestura_core::TaskStatus::Completed => "[x]",
                            gestura_core::TaskStatus::Cancelled => "[-]",
                        };
                        lines.push(format!("  {} {}", sub_icon, sub.name));
                    }
                }
                app.messages.push(app::TuiMessage {
                    role: "system".to_string(),
                    content: lines.join("\n"),
                    thinking: None,
                    is_streaming: false,
                    is_error: false,
                });
            }
        }
        Err(e) => {
            app.set_error(format!("Failed to load tasks: {}", e));
        }
    }
    Ok(())
}

/// Handle `/hooks` command - show hooks configuration.
fn handle_hooks_command(app: &mut TuiApp) -> Result<()> {
    let hooks = &app.config.hooks;
    let mut lines = vec!["━━━ Hooks Configuration ━━━".to_string(), String::new()];
    lines.push(format!(
        "Enabled: {}",
        if hooks.enabled { "yes" } else { "no" }
    ));
    lines.push(format!("Timeout: {} ms", hooks.timeout_ms));
    lines.push(format!("Max output: {} bytes", hooks.max_output_bytes));
    lines.push(String::new());

    if hooks.allowed_programs.is_empty() {
        lines.push("Allowed programs: (none)".to_string());
    } else {
        lines.push("Allowed programs:".to_string());
        for prog in &hooks.allowed_programs {
            lines.push(format!("  - {}", prog));
        }
    }
    lines.push(String::new());

    if hooks.hooks.is_empty() {
        lines.push("Configured hooks: (none)".to_string());
    } else {
        lines.push("Configured hooks:".to_string());
        for hook in &hooks.hooks {
            lines.push(format!("  {} ({:?})", hook.name, hook.event));
            lines.push(format!(
                "    cmd: {} {}",
                hook.command.program,
                hook.command.args.join(" ")
            ));
        }
    }

    app.messages.push(app::TuiMessage {
        role: "system".to_string(),
        content: lines.join("\n"),
        thinking: None,
        is_streaming: false,
        is_error: false,
    });
    Ok(())
}

/// Handle `/permissions` command - list granted permissions and audit log.
///
/// - `/permissions` or `/permissions list` - List granted permissions
/// - `/permissions audit` - Show permission audit log
fn handle_permissions_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    use gestura_core::tools::permissions::PermissionManager;

    let manager = PermissionManager::new();
    let subcommand = args.first().map(|s| s.to_lowercase());

    match subcommand.as_deref() {
        None | Some("list") => match manager.list() {
            Ok(perms) => {
                if perms.is_empty() {
                    app.messages.push(app::TuiMessage {
                            role: "system".to_string(),
                            content: "No tool permissions have been granted.\n\nGrant permissions with 'AllowAlways' when prompted for tool confirmation.".to_string(),
                            thinking: None,
                            is_streaming: false,
                            is_error: false,
                        });
                } else {
                    let mut lines = vec!["━━━ Granted Permissions ━━━".to_string(), String::new()];
                    for perm in &perms {
                        let scope_str = match &perm.scope {
                            gestura_core::PermissionScope::Global => "global".to_string(),
                            gestura_core::PermissionScope::Path(p) => format!("path:{}", p),
                            gestura_core::PermissionScope::Command(c) => format!("cmd:{}", c),
                        };
                        let expires = perm
                            .expires_at
                            .map(|e| e.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "never".to_string());
                        lines.push(format!(
                            "  {}:{} [{}] expires: {}",
                            perm.tool, perm.action, scope_str, expires
                        ));
                    }
                    lines.push(String::new());
                    lines.push("Use /permissions audit to see check history.".to_string());
                    app.messages.push(app::TuiMessage {
                        role: "system".to_string(),
                        content: lines.join("\n"),
                        thinking: None,
                        is_streaming: false,
                        is_error: false,
                    });
                }
            }
            Err(e) => {
                app.set_error(format!("Failed to list permissions: {}", e));
            }
        },
        Some("audit") => {
            match manager.audit_log() {
                Ok(log) => {
                    if log.is_empty() {
                        app.messages.push(app::TuiMessage {
                            role: "system".to_string(),
                            content: "Permission audit log is empty.".to_string(),
                            thinking: None,
                            is_streaming: false,
                            is_error: false,
                        });
                    } else {
                        let mut lines =
                            vec!["━━━ Permission Audit Log ━━━".to_string(), String::new()];
                        // Show last 20 entries
                        for entry in log.iter().rev().take(20) {
                            let status = if entry.allowed { "✓" } else { "✗" };
                            let res = entry.resource.as_deref().unwrap_or("-");
                            lines.push(format!(
                                "  {} {}:{} [{}] - {}",
                                status, entry.tool, entry.action, res, entry.reason
                            ));
                        }
                        if log.len() > 20 {
                            lines.push(format!("  ... and {} more entries", log.len() - 20));
                        }
                        app.messages.push(app::TuiMessage {
                            role: "system".to_string(),
                            content: lines.join("\n"),
                            thinking: None,
                            is_streaming: false,
                            is_error: false,
                        });
                    }
                }
                Err(e) => {
                    app.set_error(format!("Failed to load audit log: {}", e));
                }
            }
        }
        Some(other) => {
            app.set_error(format!("Unknown /permissions subcommand: {}", other));
        }
    }
    Ok(())
}

/// Handle `/context` command - show resolved context and guardrails.
fn handle_context_command(app: &mut TuiApp) -> Result<()> {
    let mut lines = vec!["━━━ Session Context ━━━".to_string(), String::new()];

    // Session info
    lines.push(format!("Session ID: {}", &app.session.id[..8]));
    lines.push(format!(
        "Model: {}",
        app.session.model.as_deref().unwrap_or("(default)")
    ));

    // Workspace
    if let Some(workspace) = app.session.workspace_dir() {
        lines.push(format!("Workspace: {}", workspace.display()));

        // Check for guardrails
        let agents_md = workspace.join("AGENTS.md");
        let gestura_guardrails = workspace.join(".gestura").join("guardrails");
        if gestura_guardrails.exists() {
            lines.push("Guardrails: .gestura/guardrails ✓".to_string());
        } else if agents_md.exists() {
            lines.push("Guardrails: AGENTS.md ✓".to_string());
        } else {
            lines.push("Guardrails: (none found)".to_string());
        }
    } else {
        lines.push("Workspace: (not set)".to_string());
    }

    lines.push(String::new());

    // Message history
    let user_msgs = app
        .session
        .state
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .count();
    let asst_msgs = app
        .session
        .state
        .messages
        .iter()
        .filter(|m| m.role == "assistant")
        .count();
    lines.push(format!(
        "Messages: {} user, {} assistant",
        user_msgs, asst_msgs
    ));

    // Pipeline settings
    lines.push(String::new());
    lines.push("Pipeline:".to_string());
    lines.push(format!(
        "  Max history: {} messages",
        app.config.pipeline.max_history_messages
    ));
    lines.push(format!(
        "  Max context: {} tokens",
        app.config.pipeline.max_context_tokens
    ));
    lines.push(format!(
        "  Auto-compact: {}%",
        app.config.pipeline.auto_compact_threshold_percent
    ));

    app.messages.push(app::TuiMessage {
        role: "system".to_string(),
        content: lines.join("\n"),
        thinking: None,
        is_streaming: false,
        is_error: false,
    });
    Ok(())
}
