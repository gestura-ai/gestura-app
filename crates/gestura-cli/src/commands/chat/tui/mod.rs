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
    SpeechProcessorCoreExt, StreamChunk, chat_sessions::MessageSource, get_speech_processor,
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use super::{ChatOptions, Result};

const KNOWN_LLM_PROVIDERS: [&str; 5] = ["openai", "anthropic", "grok", "gemini", "ollama"];

fn is_known_llm_provider(provider: &str) -> bool {
    KNOWN_LLM_PROVIDERS
        .iter()
        .any(|p| p.eq_ignore_ascii_case(provider.trim()))
}

/// Parse a CLI/TUI-style selector string into a session override.
///
/// This is intentionally legacy-aware:
/// - `provider:model` => provider+model
/// - `provider` (if matches known providers) => provider-only
/// - otherwise => model-only
fn parse_cli_model_selector_legacy_aware(
    spec: &str,
) -> Option<gestura_core::chat_sessions::SessionLlmConfig> {
    let s = spec.trim();
    if s.is_empty() {
        return None;
    }

    if s.contains(':') {
        return gestura_core::llm_overrides::session_llm_config_from_cli_model_arg(s);
    }

    if is_known_llm_provider(s) {
        return Some(gestura_core::chat_sessions::SessionLlmConfig {
            provider: Some(s.to_ascii_lowercase()),
            model: None,
        });
    }

    gestura_core::llm_overrides::session_llm_config_from_cli_model_arg(s)
}

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
        .and_then(parse_cli_model_selector_legacy_aware)
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
    let session_llm = resolve_session_llm_override(app);
    let mut config = app.config.clone();
    let effective = gestura_core::llm_overrides::apply_cli_session_llm_overrides(
        &mut config,
        session_llm.as_ref(),
    );
    (effective.provider, effective.model)
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
        // Skip providers that are not configured (cloud providers need an API key;
        // ollama just needs a config section).  This mirrors the filtering in
        // `run_list_models` (gestura model list).
        if !is_provider_configured(&app.config, provider) {
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
                .unwrap_or_else(|_| Vec::new());
            app.cached_model_lists
                .insert(provider.to_string(), fetched.clone());
            fetched
        };

        if models.is_empty() {
            // Fallback: always show at least the provider default model by using core overrides.
            let provider_only = gestura_core::chat_sessions::SessionLlmConfig {
                provider: Some(provider.to_string()),
                model: None,
            };
            let mut tmp = app.config.clone();
            let effective = gestura_core::llm_overrides::apply_cli_session_llm_overrides(
                &mut tmp,
                Some(&provider_only),
            );
            let model = effective.model.trim().to_string();
            if !model.is_empty() {
                let active = provider == active_provider && model == active_model;
                let prefix = if active { "● " } else { "  " };
                items.push(app::ModelPickerItem {
                    label: format!("{prefix}{provider}:{model}"),
                    provider: provider.to_string(),
                    model,
                });
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

    // Parse `spec` into an override, then let core resolve defaults / ensure model is non-empty.
    let mut selected = if let Some((p, m)) = spec.split_once(':') {
        let p = p.trim().to_string();
        let m = m.trim();
        if m.is_empty() {
            gestura_core::chat_sessions::SessionLlmConfig {
                provider: Some(p),
                model: None,
            }
        } else {
            gestura_core::chat_sessions::SessionLlmConfig {
                provider: Some(p),
                model: Some(m.to_string()),
            }
        }
    } else if is_known_llm_provider(spec) {
        gestura_core::chat_sessions::SessionLlmConfig {
            provider: Some(spec.to_ascii_lowercase()),
            model: None,
        }
    } else {
        // Model-only. Prefer inferred provider, else keep the current provider.
        let inferred =
            gestura_core::llm_validation::infer_provider_from_model_id(spec).map(|p| p.to_string());

        let current_provider = app
            .session
            .state
            .llm_config
            .as_ref()
            .and_then(|c| c.provider.as_deref())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| app.config.llm.primary.clone());

        gestura_core::chat_sessions::SessionLlmConfig {
            provider: Some(inferred.unwrap_or(current_provider)),
            model: Some(spec.to_string()),
        }
    };

    // Validate explicit model selections before we persist any session change.
    let provider = selected
        .provider
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    if provider.is_empty() {
        app.set_error("Model selection is missing provider".to_string());
        return Ok(());
    }
    if let Some(model) = selected
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Err(msg) =
            gestura_core::llm_validation::validate_model_for_provider(&provider, model)
        {
            app.set_error(msg);
            return Ok(());
        }
    } else {
        selected.model = None;
    }

    // Resolve to a concrete provider+model using the canonical override helper.
    let mut tmp = app.config.clone();
    let effective =
        gestura_core::llm_overrides::apply_cli_session_llm_overrides(&mut tmp, Some(&selected));
    if effective.model.trim().is_empty() {
        app.set_error(format!(
            "Could not resolve a default model for provider '{provider}'"
        ));
        return Ok(());
    }

    app.session.state.llm_config = Some(gestura_core::chat_sessions::SessionLlmConfig {
        provider: Some(effective.provider.clone()),
        model: Some(effective.model.clone()),
    });
    // Keep the legacy hint in sync for compatibility across CLI modes.
    app.session.model = Some(format!("{}:{}", effective.provider, effective.model));

    super::save_cli_session(&app.session)?;
    app.mode = TuiMode::Insert;
    app.set_status(format!(
        "Model set: {}:{}",
        effective.provider, effective.model
    ));
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

/// In-flight voice capture (record + transcribe) state.
///
/// Voice capture must never block the TUI event loop; we run the capture on the Tokio runtime and
/// poll the receiver from the main loop (similar to streaming responses).
struct VoiceCaptureState {
    /// Receiver for the transcription result.
    receiver: mpsc::Receiver<std::result::Result<String, String>>,
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
    let mut session_changed = super::ensure_session_tool_settings(&mut session, &config);

    // Apply startup override for session permission level.
    session_changed |=
        super::apply_permission_level_override(&mut session, opts.permission_level_override);

    if session_changed {
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

    // Start local hotkey IPC server so the GUI global shortcut can route here.
    //
    // This is best-effort: if it fails (e.g., temp dir issues), the TUI should
    // still run normally.
    let (hotkey_tx, mut hotkey_rx) = mpsc::unbounded_channel::<()>();
    let _hotkey_guard = match rt.block_on(gestura_core::hotkey_ipc::start_cli_hotkey_server(
        hotkey_tx,
        gestura_core::hotkey_ipc::default_cli_hotkey_port_file(),
    )) {
        Ok(g) => Some(g),
        Err(e) => {
            tracing::warn!("Failed to start CLI hotkey IPC server: {e}");
            None
        }
    };

    // Load initial workflows
    load_workflows(&mut app);

    // Run the main loop
    let result = run_main_loop(&mut terminal, &mut app, &rt, &mut hotkey_rx);

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
    hotkey_rx: &mut mpsc::UnboundedReceiver<()>,
) -> Result<()> {
    // Optional streaming state
    let mut streaming: Option<StreamingState> = None;
    // Optional prompt enhancement state
    let mut prompt_enhancement: Option<PromptEnhancementState> = None;
    // Optional voice capture state
    let mut voice_capture: Option<VoiceCaptureState> = None;

    loop {
        // Handle triggers from the GUI global hotkey (routed via local IPC).
        while hotkey_rx.try_recv().is_ok() {
            handle_toggle_recording_action(app, &streaming, &mut voice_capture, rt);
        }

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

        // Process voice capture results (non-blocking)
        if let Some(ref mut capture_state) = voice_capture {
            let mut completed = false;
            match capture_state.receiver.try_recv() {
                Ok(Ok(transcript)) => {
                    let transcript = transcript.trim().to_string();
                    if transcript.is_empty() {
                        app.set_status("No speech detected".to_string());
                    } else if streaming.is_none() {
                        // Clear any stale pause state when the user sends a new message.
                        if app.session.state.paused_execution.is_some() {
                            app.session.state.paused_execution = None;
                            let _ = super::save_cli_session(&app.session);
                        }

                        streaming =
                            start_streaming_message(app, rt, &transcript, MessageSource::Voice)?;
                    } else {
                        // Should not happen (we block starting voice while streaming), but keep the UI safe.
                        app.input = transcript;
                        app.cursor_pos = app.input.len();
                        app.set_status(
                            "Response streaming; voice transcript inserted — press Enter after stream completes"
                                .to_string(),
                        );
                    }
                    completed = true;
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    let msg_lc = msg.to_ascii_lowercase();
                    if msg_lc.contains("cancel") {
                        app.set_status("Recording cancelled".to_string());
                    } else {
                        app.set_error(format!("Voice capture failed: {}", msg));
                    }
                    completed = true;
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    app.set_error("Voice capture task ended unexpectedly".to_string());
                    completed = true;
                }
            }

            if completed {
                app.voice_capture_in_progress = false;
                voice_capture = None;
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
                    if app.voice_capture_in_progress {
                        gestura_core::request_stop_recording();
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
                        streaming = start_streaming_message(app, rt, &msg, MessageSource::Text)?;
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
                            Action::SendMessage(msg) => {
                                if streaming.is_none() {
                                    if app.session.state.paused_execution.is_some() {
                                        app.session.state.paused_execution = None;
                                        let _ = super::save_cli_session(&app.session);
                                    }
                                    streaming = start_streaming_message(
                                        app,
                                        rt,
                                        &msg,
                                        MessageSource::Text,
                                    )?;
                                } else {
                                    app.input = msg;
                                    app.cursor_pos = app.input.len();
                                    app.set_status(
                                        "Response streaming; wait for completion (Esc to cancel), then press Enter"
                                            .to_string(),
                                    );
                                }
                            }
                            Action::ToggleRecording => {
                                handle_toggle_recording_action(
                                    app,
                                    &streaming,
                                    &mut voice_capture,
                                    rt,
                                );
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
                Action::CopyMessageRaw(msg_idx) => {
                    let text = match app.messages.get(msg_idx) {
                        Some(msg) if !msg.content.is_empty() => msg.content.clone(),
                        Some(_) => {
                            app.set_status("Message is empty".to_string());
                            String::new()
                        }
                        None => {
                            app.set_status("Message not found".to_string());
                            String::new()
                        }
                    };

                    if !text.is_empty() {
                        match arboard::Clipboard::new() {
                            Ok(mut clipboard) => match clipboard.set_text(&text) {
                                Ok(()) => {
                                    app.set_status("Copied message to clipboard".to_string());
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

                    // Clear selection after copy (consistent with CopySelection behavior).
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

                    if app.voice_capture_in_progress {
                        gestura_core::request_stop_recording();
                        app.set_status("Stopping recording...".to_string());
                    }
                }
                Action::ToggleRecording => {
                    handle_toggle_recording_action(app, &streaming, &mut voice_capture, rt);
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
    source: MessageSource,
) -> Result<Option<StreamingState>> {
    // Redirect /tools to the interactive tools tab (handled the same as :tools command).
    if message.trim().starts_with("/tools") {
        let parts: Vec<&str> = message.split_whitespace().collect();
        open_tools_tab(app, &parts[1..]);
        return Ok(None);
    }

    // Add user message (persist with explicit source)
    app.add_user_message_with_source(message, source);
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
    let model_name = effective.model;
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
    let model_name = effective.model;
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

/// Open the interactive MCP server browser overlay.
///
/// Loads the current MCP server configuration, resolves which servers are
/// currently connected via the global [`McpClientRegistry`], and populates
/// [`McpBrowserState`] before switching to [`TuiMode::Mcp`].
fn open_mcp_browser(app: &mut TuiApp, rt: &tokio::runtime::Runtime) {
    // Use the in-memory config (kept in sync with on-disk saves) so the overlay
    // reflects mutations performed within the TUI.
    let config = &app.config;
    let registry = gestura_core::mcp::client::get_mcp_client_registry();
    let connected_names = rt.block_on(registry.connected_servers());

    let servers: Vec<app::McpBrowserEntry> = config
        .mcp_servers
        .iter()
        .map(|entry| app::McpBrowserEntry {
            entry: entry.clone(),
            connected: connected_names.contains(&entry.name),
        })
        .collect();

    let count = servers.len();
    app.mcp_browser_state.servers = servers;
    app.mcp_browser_state.detail_mode = false;
    app.mcp_browser_state.selected_index = 0;
    if count > 0 {
        app.mcp_browser_state.list_state.select(Some(0));
    } else {
        app.mcp_browser_state.list_state.select(None);
    }
    app.mode = TuiMode::Mcp;
    app.set_status(
	        "MCP: ↑/↓ navigate  Enter details  n add  Space toggle  c connect  d disconnect  x remove  Esc close",
	    );
}

/// Open the interactive knowledge browser overlay.
///
/// Loads all registered knowledge items (including builtins) and populates
/// [`KnowledgeBrowserState`] before switching to [`TuiMode::Knowledge`].
///
/// The `enabled` state on each item is derived from the current session's
/// persisted settings via [`KnowledgeSettingsManager`], not from the item's
/// default `enabled` field.
fn open_knowledge_browser(app: &mut TuiApp) {
    let store = gestura_core::knowledge::KnowledgeStore::with_default_dir();
    gestura_core::knowledge::register_builtin_knowledge(&store);
    let mut items = store.list();
    items.sort_by(|a, b| a.name.cmp(&b.name));

    // Overlay per-session enabled state from KnowledgeSettingsManager.
    let session_id = &app.session.id;
    let settings_mgr = gestura_core::knowledge::KnowledgeSettingsManager::new(
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
    );
    if let Ok(enabled_ids) = settings_mgr.get_enabled_knowledge(session_id) {
        for item in &mut items {
            item.enabled = enabled_ids.contains(&item.id);
        }
    }

    let count = items.len();
    app.knowledge_browser_state.items = items;
    app.knowledge_browser_state.detail_mode = false;
    app.knowledge_browser_state.selected_index = 0;
    if count > 0 {
        app.knowledge_browser_state.list_state.select(Some(0));
    } else {
        app.knowledge_browser_state.list_state.select(None);
    }
    app.mode = TuiMode::Knowledge;
    app.set_status("Knowledge: ↑/↓ navigate  Enter details  Space toggle  Esc close");
}

/// Open the interactive hooks browser overlay.
fn open_hooks_browser(app: &mut TuiApp) {
    let hooks = &app.config.hooks;
    let data = app::HooksBrowserData {
        enabled: hooks.enabled,
        timeout_ms: hooks.timeout_ms,
        max_output_bytes: hooks.max_output_bytes,
        allowed_programs: hooks.allowed_programs.clone(),
        hooks: hooks
            .hooks
            .iter()
            .map(|h| {
                (
                    h.name.clone(),
                    format!("{:?}", h.event),
                    h.command.program.clone(),
                    h.command.args.join(" "),
                )
            })
            .collect(),
    };
    let count = data.hooks.len().max(1); // at least 1 for the empty state
    app.hooks_browser_data = data;
    app.hooks_browser_state.reset(count);
    app.mode = TuiMode::Hooks;
    app.set_status(
        "Hooks: ↑/↓ navigate  Enter details  Space toggle  n new  e edit  x delete  a allow+  r allow-  t timeout  m max  Esc close",
    );
}

/// Open the interactive agent browser overlay.
fn open_agent_browser(app: &mut TuiApp) {
    let config = &app.config;
    let mut rows: Vec<(String, String)> = vec![
        ("Version".to_string(), gestura_core::VERSION.to_string()),
        ("Primary LLM".to_string(), config.llm.primary.clone()),
        (
            "Model".to_string(),
            app.session
                .model
                .as_deref()
                .unwrap_or("(default)")
                .to_string(),
        ),
        (
            "Session".to_string(),
            app.session.id[..app.session.id.len().min(8)].to_string(),
        ),
        (
            "Messages".to_string(),
            app.session.message_count().to_string(),
        ),
    ];

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
    rows.push((
        "OpenAI".to_string(),
        if has_openai {
            "✓ configured"
        } else {
            "○ not configured"
        }
        .to_string(),
    ));
    rows.push((
        "Anthropic".to_string(),
        if has_anthropic {
            "✓ configured"
        } else {
            "○ not configured"
        }
        .to_string(),
    ));

    if let Some(ref openai) = config.llm.openai {
        rows.push(("OpenAI model".to_string(), openai.model.clone()));
    }
    if let Some(ref anthropic) = config.llm.anthropic {
        rows.push(("Anthropic model".to_string(), anthropic.model.clone()));
    }

    let count = rows.len();
    app.agent_browser_data = app::AgentBrowserData { rows };
    app.agent_browser_state.reset(count);
    app.mode = TuiMode::Agent;
    app.set_status("Agent: ↑/↓ navigate  Enter details  Esc close");
}

/// Open the interactive memory browser overlay.
fn open_memory_browser(app: &mut TuiApp, rt: &tokio::runtime::Runtime) {
    use super::live_actions::{MemoryExecOutput, execute_memory_live_action};

    let Some(workspace_dir) = app.session.workspace_dir().cloned() else {
        app.set_error("No workspace directory set for this session.");
        return;
    };

    let out = match super::slash::run_memory_subcommand(&["list"], &app.session) {
        Ok(out) => out,
        Err(e) => {
            app.set_error(e);
            return;
        }
    };

    let Some(act) = out.live_action else {
        app.set_error("Internal error: /memory list produced no live action");
        return;
    };

    match execute_memory_live_action(rt, &workspace_dir, act) {
        Ok(MemoryExecOutput::Listed(entries)) => {
            let browser_entries: Vec<app::MemoryBrowserEntry> = entries
                .iter()
                .map(|e| {
                    let file_path = e.file_path.as_ref().and_then(|p| {
                        if let Ok(rel) = p.strip_prefix(&workspace_dir) {
                            Some(rel.to_string_lossy().to_string())
                        } else {
                            p.file_name().map(|name| {
                                std::path::PathBuf::from(".gestura/memory")
                                    .join(name)
                                    .to_string_lossy()
                                    .to_string()
                            })
                        }
                    });

                    app::MemoryBrowserEntry {
                        timestamp: e.timestamp.format("%Y-%m-%d %H:%M").to_string(),
                        category: e.category.clone(),
                        summary: e.summary.clone(),
                        content: e.content.clone(),
                        session_id: e.session_id.clone(),
                        file_path,
                    }
                })
                .collect();

            // Keep a stable selection model even when empty.
            let count = browser_entries.len().max(1);
            app.memory_browser_entries = browser_entries;
            app.memory_browser_state.reset(count);
            app.mode = TuiMode::Memory;
            app.set_status("Memory: ↑/↓ navigate  Enter details  s save  x delete  Esc close");
        }
        Ok(other) => {
            app.set_error(format!("Unexpected /memory list output: {other:?}"));
        }
        Err(e) => {
            app.set_error(format!("Failed to read memory bank: {e}"));
        }
    }
}

/// Open the interactive devices browser overlay.
fn open_devices_browser(app: &mut TuiApp) {
    let devices = gestura_core::list_audio_input_devices();
    let entries: Vec<app::DeviceBrowserEntry> = devices
        .iter()
        .map(|d| app::DeviceBrowserEntry {
            name: d.name.clone(),
            is_default: d.is_default,
        })
        .collect();
    let count = entries.len();
    app.devices_browser_entries = entries;
    app.devices_browser_state.reset(count);
    app.mode = TuiMode::Devices;
    app.set_status("Devices: ↑/↓ navigate  Enter details  Esc close");
}

/// Open the interactive permissions browser overlay.
fn open_permissions_browser(app: &mut TuiApp) {
    use crate::commands::tools::permissions::permission_manager;

    match permission_manager().list() {
        Ok(perms) => {
            let entries: Vec<app::PermissionBrowserEntry> = perms
                .iter()
                .map(|p| {
                    let scope_str = match &p.scope {
                        gestura_core::PermissionScope::Global => "global".to_string(),
                        gestura_core::PermissionScope::Path(path) => format!("path:{}", path),
                        gestura_core::PermissionScope::Command(cmd) => format!("cmd:{}", cmd),
                    };
                    let expires = p
                        .expires_at
                        .map(|e| e.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "never".to_string());
                    app::PermissionBrowserEntry {
                        tool: p.tool.clone(),
                        action: p.action.clone(),
                        scope: scope_str,
                        expires,
                    }
                })
                .collect();
            let count = entries.len().max(1); // at least 1 for the empty state
            app.permissions_browser_entries = entries;
            app.permissions_browser_state.reset(count);
            app.mode = TuiMode::Permissions;
            app.set_status(
                "Permissions: ↑/↓ navigate  Enter details  g grant  x revoke  r reset  l level  Esc close",
            );
        }
        Err(e) => {
            app.set_error(format!("Failed to list permissions: {}", e));
        }
    }
}

/// Open the interactive sessions browser overlay.
fn open_sessions_browser(app: &mut TuiApp) {
    match super::list_sessions_filtered(super::SessionFilter::All) {
        Ok(sessions) => {
            let current_id = app.session.id.clone();
            let entries: Vec<app::SessionBrowserEntry> = sessions
                .iter()
                .map(|s| app::SessionBrowserEntry {
                    id: s.id.clone(),
                    model: s.model.as_deref().unwrap_or("default").to_string(),
                    message_count: s.message_count,
                    created: s
                        .created_at
                        .with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string(),
                    last_active: s
                        .last_active
                        .with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string(),
                    is_current: s.id == current_id,
                })
                .collect();
            let count = entries.len();
            app.sessions_browser_entries = entries;
            app.sessions_browser_state.reset(count);
            app.mode = TuiMode::Sessions;
            app.set_status(
                "Sessions: ↑/↓ navigate  Enter details  l load  x delete  e export  Esc close",
            );
        }
        Err(e) => {
            app.set_error(format!("Failed to list sessions: {}", e));
        }
    }
}

/// Open the interactive tasks browser overlay.
fn open_tasks_browser(app: &mut TuiApp) {
    use gestura_core::tasks::TaskManager;

    let task_manager =
        TaskManager::new(dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from(".")));
    match task_manager.get_hierarchy(&app.session.id) {
        Ok(hierarchy) => {
            let mut entries: Vec<app::TaskBrowserEntry> = Vec::new();
            for (root, subtasks) in &hierarchy {
                let status_icon = match root.status {
                    gestura_core::tasks::TaskStatus::NotStarted => "[ ]",
                    gestura_core::tasks::TaskStatus::InProgress => "[/]",
                    gestura_core::tasks::TaskStatus::Completed => "[x]",
                    gestura_core::tasks::TaskStatus::Cancelled => "[-]",
                };
                entries.push(app::TaskBrowserEntry {
                    id: root.id.clone(),
                    name: root.name.clone(),
                    description: root.description.clone(),
                    status: format!("{:?}", root.status),
                    status_icon: status_icon.to_string(),
                    parent_id: None,
                    source: format!("{:?}", root.source),
                    created: root.created_at.format("%Y-%m-%d %H:%M").to_string(),
                });
                for sub in subtasks {
                    let sub_icon = match sub.status {
                        gestura_core::tasks::TaskStatus::NotStarted => "[ ]",
                        gestura_core::tasks::TaskStatus::InProgress => "[/]",
                        gestura_core::tasks::TaskStatus::Completed => "[x]",
                        gestura_core::tasks::TaskStatus::Cancelled => "[-]",
                    };
                    entries.push(app::TaskBrowserEntry {
                        id: sub.id.clone(),
                        name: sub.name.clone(),
                        description: sub.description.clone(),
                        status: format!("{:?}", sub.status),
                        status_icon: sub_icon.to_string(),
                        parent_id: sub.parent_id.clone(),
                        source: format!("{:?}", sub.source),
                        created: sub.created_at.format("%Y-%m-%d %H:%M").to_string(),
                    });
                }
            }
            let count = entries.len().max(1); // at least 1 for the empty state
            app.tasks_browser_entries = entries;
            app.tasks_browser_state.reset(count);
            app.mode = TuiMode::Tasks;
            app.set_status(
                "Tasks: ↑/↓ navigate  Enter details  n new  e name  d desc  s sub  a dep  Space status  c current  u clear  x delete  Esc close",
	            );
        }
        Err(e) => {
            app.set_error(format!("Failed to list tasks: {}", e));
        }
    }
}

/// Open the interactive themes browser overlay.
fn open_themes_browser(app: &mut TuiApp) {
    let themes: Vec<String> = app::Theme::available_themes()
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let count = themes.len();
    app.themes_browser_names = themes;
    app.themes_browser_state.reset(count);
    app.mode = TuiMode::Themes;
    app.set_status("Themes: ↑/↓ navigate  Enter apply  Esc close");
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
            app.capabilities_text = gestura_core::tools::render_capabilities(&app.config);
            app.capabilities_scroll = 0;
            app.mode = TuiMode::Capabilities;
        }
        "/theme" => {
            if args.is_empty() {
                open_themes_browser(app);
            } else {
                app.set_theme(args[0]);
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
            if args.is_empty() {
                open_sessions_browser(app);
            } else {
                handle_session_command(app, args)?;
            }
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
            if args.is_empty() {
                open_tasks_browser(app);
            } else {
                handle_tasks_command(app, args)?;
            }
        }
        "/task" => {
            // `/task` is the subcommand-oriented interface, but treat `/task` (no args)
            // as an alias for `/tasks` to avoid surprising users with usage output.
            if args.is_empty() {
                open_tasks_browser(app);
            } else {
                handle_tasks_command(app, args)?;
            }
        }
        "/hooks" | "/hook" => {
            if args.is_empty() {
                open_hooks_browser(app);
            } else {
                handle_hooks_command(app, args)?;
            }
        }
        "/permissions" | "/permission" => {
            if args.is_empty() {
                open_permissions_browser(app);
            } else {
                handle_permissions_command(app, args)?;
            }
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
        "/mcp" => {
            if args.is_empty() {
                open_mcp_browser(app, rt);
            } else {
                handle_mcp_command(app, args, rt)?;
            }
        }
        "/a2a" => {
            handle_a2a_command(app, args)?;
        }
        "/knowledge" => {
            if args.is_empty() {
                open_knowledge_browser(app);
            } else {
                handle_knowledge_command(app, args)?;
            }
        }
        "/agent" => {
            if args.is_empty() {
                open_agent_browser(app);
            } else {
                handle_agent_command(app, args)?;
            }
        }
        "/device" => {
            if args.is_empty() {
                open_devices_browser(app);
            } else {
                handle_device_command(app, args)?;
            }
        }
        "/health" => {
            handle_health_command(app)?;
        }
        "/privacy" => {
            handle_privacy_command(app, args, rt)?;
        }
        "/memory" => {
            if args.is_empty() {
                open_memory_browser(app, rt);
            } else {
                handle_memory_command(app, args, rt)?;
            }
        }
        "/summarize" => {
            handle_summarize_command(app)?;
        }
        "/listen" => {
            if app.voice_capture_in_progress {
                app.set_status("Recording in progress; press Esc to cancel".to_string());
            } else if app.listening_mode {
                app.listening_mode = false;
                app.set_status("Listening mode disabled".to_string());
            } else if !gestura_core::is_microphone_available() {
                app.set_error("No microphone available".to_string());
            } else {
                app.listening_mode = true;
                app.set_status(
                    "Listening mode enabled: press Enter on an empty prompt to record".to_string(),
                );
            }
        }
        "/voice" => {
            if !gestura_core::is_microphone_available() {
                app.set_error("No microphone available".to_string());
            } else {
                return Ok(Some(Action::ToggleRecording));
            }
        }
        "/exec" => {
            if args.is_empty() {
                app.set_error("Usage: /exec <prompt>");
            } else {
                let prompt = args.join(" ");
                return Ok(Some(Action::SendMessage(prompt)));
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

/// Check whether a provider is usable (has credentials for cloud providers,
/// or has a config section for local providers like ollama).
///
/// This is the single source of truth for "should this provider appear in the
/// model picker / model list" and mirrors the logic in `run_list_models`.
fn is_provider_configured(config: &gestura_core::AppConfig, provider: &str) -> bool {
    match provider {
        "openai" => config
            .llm
            .openai
            .as_ref()
            .is_some_and(|c| !c.api_key.trim().is_empty()),
        "anthropic" => config
            .llm
            .anthropic
            .as_ref()
            .is_some_and(|c| !c.api_key.trim().is_empty()),
        "gemini" => config
            .llm
            .gemini
            .as_ref()
            .is_some_and(|c| !c.api_key.trim().is_empty()),
        "grok" => config
            .llm
            .grok
            .as_ref()
            .is_some_and(|c| !c.api_key.trim().is_empty()),
        "ollama" => config.llm.ollama.is_some(),
        _ => false,
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

/// Handle `/task`/`/tasks <subcommand...>` command - manage tasks for the current session.
fn handle_tasks_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    use gestura_core::tasks::TaskManager;

    let task_manager =
        TaskManager::new(dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from(".")));

    match super::slash::run_tasks_subcommand(args, &task_manager, &app.session.id) {
        Ok(out) => {
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: out.lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
            app.scroll_to_bottom();

            // If the tasks overlay is open, refresh its contents after mutations.
            if out.changed && app.mode == TuiMode::Tasks {
                open_tasks_browser(app);
            }
        }
        Err(e) => {
            app.set_error(&e);
            if let Ok(out) =
                super::slash::run_tasks_subcommand(&["help"], &task_manager, &app.session.id)
            {
                app.messages.push(app::TuiMessage {
                    role: "system".to_string(),
                    content: out.lines.join("\n"),
                    thinking: None,
                    is_streaming: false,
                    is_error: false,
                });
                app.scroll_to_bottom();
            }
        }
    }

    Ok(())
}

/// Handle `/hooks <subcommand...>` command - manage hook settings & definitions.
fn handle_hooks_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    let mut cfg = app.config.clone();

    match super::slash::apply_hooks_subcommand(args, &mut cfg) {
        Ok(outcome) => {
            let changed = outcome.changed();
            let lines = outcome.into_lines();

            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
            app.scroll_to_bottom();

            if changed {
                match cfg.save() {
                    Ok(()) => {
                        app.config = cfg;

                        // If the hooks overlay is open, refresh its contents.
                        if app.mode == TuiMode::Hooks {
                            open_hooks_browser(app);
                        }
                    }
                    Err(e) => {
                        app.set_error(format!("Failed to save config: {}", e));
                    }
                }
            }
        }
        Err(e) => {
            app.set_error(&e);
            if let Ok(outcome) = super::slash::apply_hooks_subcommand(&["help"], &mut cfg) {
                app.messages.push(app::TuiMessage {
                    role: "system".to_string(),
                    content: outcome.into_lines().join("\n"),
                    thinking: None,
                    is_streaming: false,
                    is_error: false,
                });
                app.scroll_to_bottom();
            }
        }
    }

    Ok(())
}

/// Handle `/permissions` command - list granted permissions and audit log.
///
/// - `/permissions` or `/permissions list` - List granted permissions
/// - `/permissions audit` - Show permission audit log
fn handle_permissions_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    match super::slash::run_permissions_subcommand(args, &mut app.session) {
        Ok(out) => {
            if !out.lines.is_empty() {
                app.messages.push(app::TuiMessage {
                    role: "system".to_string(),
                    content: out.lines.join("\n"),
                    thinking: None,
                    is_streaming: false,
                    is_error: false,
                });
                app.scroll_to_bottom();
            }

            if out.session_changed
                && let Err(e) = super::save_cli_session(&app.session)
            {
                app.set_error(format!("Failed to save session: {e}"));
            }

            // If the permissions overlay is open, refresh it after mutations.
            if out.changed_permissions && app.mode == TuiMode::Permissions {
                open_permissions_browser(app);
            }
        }
        Err(e) => {
            app.set_error(&e);
            if let Ok(help) = super::slash::run_permissions_subcommand(&["help"], &mut app.session)
            {
                app.messages.push(app::TuiMessage {
                    role: "system".to_string(),
                    content: help.lines.join("\n"),
                    thinking: None,
                    is_streaming: false,
                    is_error: false,
                });
                app.scroll_to_bottom();
            }
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

/// Handle `/mcp` command - MCP server management.
///
/// Subcommands:
/// - `/mcp` or `/mcp status` - Show MCP protocol status
/// - `/mcp list` - List configured servers
/// - `/mcp tools` - List tools from connected servers
/// - `/mcp connect <name>` - Connect to a server
/// - `/mcp disconnect <name>` - Disconnect from a server
fn handle_mcp_command(app: &mut TuiApp, args: &[&str], rt: &tokio::runtime::Runtime) -> Result<()> {
    fn execute_mcp_live_action(
        rt: &tokio::runtime::Runtime,
        cfg: &AppConfig,
        act: super::slash::McpLiveAction,
    ) -> Vec<String> {
        use gestura_core::config::McpTransportType;

        let registry = gestura_core::mcp::client::get_mcp_client_registry();

        match act {
            super::slash::McpLiveAction::Status => {
                let connected = rt.block_on(registry.connected_servers());

                let mut lines = vec!["━━━ MCP Server Status ━━━".to_string(), String::new()];
                lines.push(format!("Servers: {} configured", cfg.mcp_servers.len()));
                lines.push(format!(
                    "Enabled: {}",
                    cfg.mcp_servers.iter().filter(|s| s.enabled).count()
                ));

                if !cfg.mcp_servers.is_empty() {
                    lines.push(String::new());
                    for srv in &cfg.mcp_servers {
                        let status = if srv.enabled { "✓" } else { "○" };
                        let conn = if connected.contains(&srv.name) {
                            "●"
                        } else {
                            "○"
                        };
                        let endpoint = match srv.transport {
                            McpTransportType::Stdio => {
                                let cmd = srv.command.as_deref().unwrap_or("");
                                let cmd_args = srv.args.join(" ");
                                format!("{} {}", cmd, cmd_args).trim().to_string()
                            }
                            _ => srv.url.clone().unwrap_or_default(),
                        };
                        lines.push(format!(
                            "  {status} {conn} {} [{}] {endpoint}",
                            srv.name, srv.transport
                        ));
                    }
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
            super::slash::McpLiveAction::Tools { server } => {
                let all = rt.block_on(registry.all_tools());
                let filtered: Vec<_> = if let Some(ref filter) = server {
                    all.into_iter().filter(|(name, _)| name == filter).collect()
                } else {
                    all
                };

                if filtered.is_empty() {
                    return vec!["No MCP tools discovered. Connect to a server first.".to_string()];
                }

                let mut lines = vec!["━━━ MCP Tools ━━━".to_string(), String::new()];
                let mut total = 0usize;
                for (srv, tools) in &filtered {
                    lines.push(format!("  {srv} ({} tools)", tools.len()));
                    for tool in tools {
                        lines.push(format!(
                            "    • {} — {}",
                            tool.name,
                            tool.description.as_deref().unwrap_or("(no description)")
                        ));
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
            super::slash::McpLiveAction::Connect { name } => {
                let Some(srv) = cfg.mcp_servers.iter().find(|s| s.name == name) else {
                    return vec![format!("MCP server not found in config: {name}")];
                };
                match rt.block_on(registry.connect(srv)) {
                    Ok(tools) => vec![format!(
                        "Connected to MCP server '{name}' ({} tools discovered)",
                        tools.len()
                    )],
                    Err(e) => vec![format!("Failed to connect to '{name}': {e}")],
                }
            }
            super::slash::McpLiveAction::Disconnect { name } => {
                rt.block_on(registry.disconnect(&name));
                vec![format!("Disconnected from MCP server '{name}'")]
            }
        }
    }

    let mut cfg = app.config.clone();
    match super::slash::run_mcp_subcommand(args, &mut cfg) {
        Ok(out) => {
            let changed = out.changed;

            // Persist config changes first.
            if changed {
                match cfg.save() {
                    Ok(()) => {
                        app.config = cfg.clone();
                    }
                    Err(e) => {
                        app.set_error(format!("Failed to save config: {e}"));
                        return Ok(());
                    }
                }
            }

            let mut lines = if let Some(act) = out.live_action {
                execute_mcp_live_action(rt, &cfg, act)
            } else {
                out.lines
            };

            // Always show something for /mcp actions (even ones that primarily set status).
            if lines.is_empty() {
                lines.push("(no output)".to_string());
            }

            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
            app.scroll_to_bottom();

            // If the MCP overlay is open, refresh its cached list after mutations or
            // connection state changes.
            if app.mode == TuiMode::Mcp {
                open_mcp_browser(app, rt);
            }
        }
        Err(e) => {
            app.set_error(&e);
            let mut help_cfg = app.config.clone();
            if let Ok(help) = super::slash::run_mcp_subcommand(&["help"], &mut help_cfg) {
                app.messages.push(app::TuiMessage {
                    role: "system".to_string(),
                    content: help.lines.join("\n"),
                    thinking: None,
                    is_streaming: false,
                    is_error: false,
                });
                app.scroll_to_bottom();
            }
        }
    }

    Ok(())
}

/// Handle `/a2a` command - Agent-to-Agent protocol.
///
/// Subcommands:
/// - `/a2a` or `/a2a status` - Show A2A protocol status
/// - `/a2a profiles` - List registered profiles
/// - `/a2a agents` - List known agents
/// - `/a2a discover <url>` - Discover a remote agent
fn handle_a2a_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    let subcommand = args.first().map(|s| s.to_lowercase());

    match subcommand.as_deref() {
        None | Some("status") => {
            let lines = vec![
                "━━━ A2A Protocol Status ━━━".to_string(),
                String::new(),
                "Protocol: Agent2Agent (A2A)".to_string(),
                "Version: 0.3.0".to_string(),
                "Governance: Linux Foundation".to_string(),
                "License: Apache 2.0".to_string(),
                String::new(),
                "Features:".to_string(),
                "  ✓ Agent discovery via Agent Cards".to_string(),
                "  ✓ Task-based communication".to_string(),
                "  ✓ JSON-RPC 2.0 protocol".to_string(),
                "  ✓ Bearer token authentication".to_string(),
                "  ✓ Profile propagation".to_string(),
                "  ✓ SSE streaming support".to_string(),
                String::new(),
                "Endpoints:".to_string(),
                "  • agent/discover".to_string(),
                "  • task/create".to_string(),
                "  • task/status".to_string(),
                "  • task/cancel".to_string(),
                "  • profile/register".to_string(),
                "  • profile/validate".to_string(),
            ];
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
        }
        Some("profiles") => {
            app.set_status(
                "No A2A profiles registered yet. Use 'gestura a2a register' to add one.",
            );
        }
        Some("agents") => {
            app.set_status(
                "No remote agents discovered yet. Use '/a2a discover <url>' to find one.",
            );
        }
        Some("discover") => {
            if let Some(url) = args.get(1) {
                app.set_status(format!(
                    "Agent discovery requires async network call. Use 'gestura a2a discover {}' from CLI.",
                    url
                ));
            } else {
                app.set_error("Usage: /a2a discover <url>");
            }
        }
        Some(other) => {
            app.set_error(format!(
                "Unknown /a2a subcommand: {}. Try: status, profiles, agents, discover",
                other
            ));
        }
    }
    Ok(())
}

/// Handle `/knowledge` command - Knowledge base management.
///
/// Subcommands:
/// - `/knowledge` or `/knowledge list` - List all knowledge items
/// - `/knowledge search <query>` - Search knowledge
/// - `/knowledge categories` - List categories
/// - `/knowledge status` - Show knowledge base status
/// - `/knowledge show <id>` - Show a specific item
fn handle_knowledge_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    use gestura_core::knowledge::{KnowledgeQuery, KnowledgeStore, register_builtin_knowledge};

    let subcommand = args.first().map(|s| s.to_lowercase());
    let store = KnowledgeStore::with_default_dir();
    register_builtin_knowledge(&store);

    match subcommand.as_deref() {
        None | Some("list") => {
            let items = store.list();
            if items.is_empty() {
                app.set_status("No knowledge items registered.");
            } else {
                let mut lines = vec![
                    format!("━━━ Knowledge Base ({} items) ━━━", items.len()),
                    String::new(),
                ];
                for item in &items {
                    lines.push(format!(
                        "  • [{}] {} — {}",
                        item.category, item.name, item.description
                    ));
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
        Some("search") => {
            let query_text = args.get(1..).unwrap_or_default().join(" ");
            if query_text.is_empty() {
                app.set_error("Usage: /knowledge search <query>");
            } else {
                let query = KnowledgeQuery {
                    query: query_text.clone(),
                    limit: Some(10),
                    ..Default::default()
                };
                let matches = store.find(&query);
                if matches.is_empty() {
                    app.set_status(format!("No knowledge items match '{}'.", query_text));
                } else {
                    let mut lines = vec![
                        format!("━━━ Knowledge Search: '{}' ━━━", query_text),
                        String::new(),
                    ];
                    for m in &matches {
                        lines.push(format!(
                            "  • {} (score: {:.2}) — {}",
                            m.item.name, m.score, m.item.description
                        ));
                    }
                    lines.push(String::new());
                    lines.push(format!("{} result(s)", matches.len()));
                    app.messages.push(app::TuiMessage {
                        role: "system".to_string(),
                        content: lines.join("\n"),
                        thinking: None,
                        is_streaming: false,
                        is_error: false,
                    });
                }
            }
        }
        Some("categories") => {
            let cats = store.categories();
            if cats.is_empty() {
                app.set_status("No knowledge categories found.");
            } else {
                let mut lines = vec!["━━━ Knowledge Categories ━━━".to_string(), String::new()];
                for cat in &cats {
                    let count = store.list_by_category(cat).len();
                    lines.push(format!("  • {} ({} items)", cat, count));
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
        Some("status") => {
            let mut lines = vec!["━━━ Knowledge Base Status ━━━".to_string(), String::new()];
            lines.push(format!("Total items: {}", store.count()));
            lines.push(format!("Categories: {}", store.categories().len()));
            lines.push(format!("Base directory: {}", store.base_dir().display()));
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
        }
        Some("show") => {
            if let Some(id) = args.get(1) {
                if let Some(item) = store.get(id) {
                    let mut lines = vec![
                        format!("━━━ {} ━━━", item.name),
                        String::new(),
                        format!("ID: {}", item.id),
                        format!("Category: {}", item.category),
                        format!("Description: {}", item.description),
                        format!("Triggers: {}", item.triggers.join(", ")),
                        String::new(),
                    ];
                    if !item.core_content.is_empty() {
                        lines.push("Content:".to_string());
                        lines.push(item.core_content.clone());
                    }
                    app.messages.push(app::TuiMessage {
                        role: "system".to_string(),
                        content: lines.join("\n"),
                        thinking: None,
                        is_streaming: false,
                        is_error: false,
                    });
                } else {
                    app.set_error(format!("Knowledge item '{}' not found.", id));
                }
            } else {
                app.set_error("Usage: /knowledge show <id>");
            }
        }
        Some(other) => {
            app.set_error(format!(
                "Unknown /knowledge subcommand: {}. Try: list, search, categories, status, show",
                other
            ));
        }
    }
    Ok(())
}

/// Handle `/agent` command - Agent status and configuration.
///
/// Subcommands:
/// - `/agent` or `/agent status` - Show agent status
/// - `/agent config` - Show LLM provider configuration
fn handle_agent_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    let subcommand = args.first().map(|s| s.to_lowercase());

    match subcommand.as_deref() {
        None | Some("status") => {
            let config = &app.config;
            let mut lines = vec!["━━━ Agent Status ━━━".to_string(), String::new()];
            lines.push(format!("Version: {}", gestura_core::VERSION));
            lines.push(format!("Primary LLM: {}", config.llm.primary));
            lines.push(format!(
                "Model: {}",
                app.session.model.as_deref().unwrap_or("(default)")
            ));
            lines.push(format!("Session: {}", &app.session.id[..8]));
            lines.push(format!("Messages: {}", app.session.message_count()));

            // Check provider status
            lines.push(String::new());
            lines.push("Provider Status:".to_string());
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
            lines.push(format!("  {} OpenAI", if has_openai { "✓" } else { "○" }));
            lines.push(format!(
                "  {} Anthropic",
                if has_anthropic { "✓" } else { "○" }
            ));

            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
        }
        Some("config") => {
            let config = &app.config;
            let mut lines = vec!["━━━ Agent Configuration ━━━".to_string(), String::new()];
            lines.push(format!("Primary: {}", config.llm.primary));
            if let Some(ref openai) = config.llm.openai {
                lines.push(format!("OpenAI model: {}", openai.model));
            }
            if let Some(ref anthropic) = config.llm.anthropic {
                lines.push(format!("Anthropic model: {}", anthropic.model));
            }
            if let Some(ref grok) = config.llm.grok {
                lines.push(format!("Grok model: {}", grok.model));
            }
            if let Some(ref ollama) = config.llm.ollama {
                lines.push(format!("Ollama model: {}", ollama.model));
                lines.push(format!("Ollama base URL: {}", ollama.base_url));
            }
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
        }
        Some(other) => {
            app.set_error(format!(
                "Unknown /agent subcommand: {}. Try: status, config",
                other
            ));
        }
    }
    Ok(())
}

/// Handle `/device` command - Audio device listing.
///
/// Subcommands:
/// - `/device` or `/device list` - List audio input devices
fn handle_device_command(app: &mut TuiApp, args: &[&str]) -> Result<()> {
    let subcommand = args.first().map(|s| s.to_lowercase());

    match subcommand.as_deref() {
        None | Some("list") | Some("scan") => {
            let devices = gestura_core::list_audio_input_devices();
            let mic_available = gestura_core::is_microphone_available();

            let mut lines = vec!["━━━ Audio Devices ━━━".to_string(), String::new()];
            lines.push(format!(
                "Microphone available: {}",
                if mic_available { "✓ yes" } else { "✗ no" }
            ));
            lines.push(String::new());

            if devices.is_empty() {
                lines.push("No audio input devices found.".to_string());
            } else {
                lines.push(format!("{} device(s) detected:", devices.len()));
                for dev in &devices {
                    let marker = if dev.is_default { " (default)" } else { "" };
                    lines.push(format!("  • {}{}", dev.name, marker));
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
        Some(other) => {
            app.set_error(format!(
                "Unknown /device subcommand: {}. Try: list, scan",
                other
            ));
        }
    }
    Ok(())
}

/// Handle `/health` command - System health diagnostics.
fn handle_health_command(app: &mut TuiApp) -> Result<()> {
    let config = &app.config;
    let mut lines = vec!["━━━ System Health ━━━".to_string(), String::new()];

    // Version
    lines.push(format!("✓ Gestura v{}", gestura_core::VERSION));

    // Config path
    let config_path = AppConfig::default_path();
    let config_ok = config_path.exists();
    lines.push(format!(
        "{} Config: {}",
        if config_ok { "✓" } else { "○" },
        config_path.display()
    ));

    // LLM Providers
    lines.push(String::new());
    lines.push("LLM Providers:".to_string());

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

    lines.push(format!("  {} OpenAI", if has_openai { "✓" } else { "○" }));
    lines.push(format!(
        "  {} Anthropic",
        if has_anthropic { "✓" } else { "○" }
    ));
    lines.push(format!("  {} Grok", if has_grok { "✓" } else { "○" }));
    lines.push(format!("  {} Ollama", if has_ollama { "✓" } else { "○" }));

    // Audio
    lines.push(String::new());
    lines.push("Audio:".to_string());
    let mic = gestura_core::is_microphone_available();
    let devices = gestura_core::list_audio_input_devices();
    lines.push(format!("  {} Microphone", if mic { "✓" } else { "○" }));
    lines.push(format!("  {} device(s) detected", devices.len()));

    // MCP
    lines.push(String::new());
    lines.push("MCP:".to_string());
    let mcp_count = config.mcp_servers.len();
    let mcp_enabled = config.mcp_servers.iter().filter(|s| s.enabled).count();
    lines.push(format!(
        "  {} server(s) configured ({} enabled)",
        mcp_count, mcp_enabled
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

/// Handle `/privacy` command - GDPR and privacy tools.
///
/// Subcommands:
/// - `/privacy` or `/privacy status` - Generate privacy report
/// - `/privacy policy` - Show data retention policy
/// - `/privacy export` - Guidance on data export
fn handle_privacy_command(
    app: &mut TuiApp,
    args: &[&str],
    rt: &tokio::runtime::Runtime,
) -> Result<()> {
    let subcommand = args.first().map(|s| s.to_lowercase());

    match subcommand.as_deref() {
        None | Some("status") => {
            let report = rt.block_on(async {
                let manager = gestura_core::get_gdpr_manager().await;
                manager.generate_privacy_report().await
            });
            let pretty =
                serde_json::to_string_pretty(&report).unwrap_or_else(|_| format!("{report:?}"));
            let mut lines = vec!["━━━ Privacy Report ━━━".to_string(), String::new()];
            lines.push(pretty);
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
        }
        Some("policy") => {
            let lines = vec![
                "━━━ Data Retention Policy ━━━".to_string(),
                String::new(),
                "Gestura respects user privacy and GDPR compliance:".to_string(),
                String::new(),
                "• Voice recordings: Temporary only, deleted after transcription".to_string(),
                "• Chat sessions: Stored locally in workspace".to_string(),
                "• API keys: Stored in local config file only".to_string(),
                "• Memory bank: Stored locally in .gestura/memory/".to_string(),
                "• No data is sent to third parties except configured LLM providers".to_string(),
                String::new(),
                "Use 'gestura privacy export' for a full GDPR data export.".to_string(),
                "Use 'gestura privacy delete' to exercise right to erasure.".to_string(),
            ];
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
        }
        Some("export") => {
            app.set_status("Data export requires file I/O. Use 'gestura privacy export' from CLI.");
        }
        Some(other) => {
            app.set_error(format!(
                "Unknown /privacy subcommand: {}. Try: status, policy, export",
                other
            ));
        }
    }
    Ok(())
}

/// Handle `/memory` command - Memory bank management.
///
/// Subcommands:
/// - `/memory` or `/memory list` - List memory bank entries
/// - `/memory save` - Save current conversation to memory bank
/// - `/memory clear` - Clear the memory bank
/// - `/memory delete <path>` - Delete a memory bank entry (requires confirmation)
fn handle_memory_command(
    app: &mut TuiApp,
    args: &[&str],
    rt: &tokio::runtime::Runtime,
) -> Result<()> {
    use super::live_actions::{MemoryExecOutput, execute_memory_live_action};

    let sub = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    // Destructive confirmation UX: prompt via confirm modal, then re-run with --confirmed.
    if sub == "clear" && !args.contains(&"--confirmed") {
        app.show_confirm(app::ConfirmAction::ExecuteCommand {
            title: "Clear Memory Bank?".to_string(),
            message: "This will permanently delete ALL memory entries.\n\n  [Y] Yes, clear all    [N] No, cancel"
                .to_string(),
            command: "/memory clear --confirmed".to_string(),
        });
        return Ok(());
    }

    if sub == "delete" {
        let mut confirmed = false;
        let mut path_arg: Option<&str> = None;
        for a in args.iter().skip(1).copied() {
            if a == "--confirmed" {
                confirmed = true;
            } else {
                path_arg = Some(a);
            }
        }

        let Some(path_str) = path_arg else {
            app.set_error("Usage: /memory delete <path> [--confirmed]");
            return Ok(());
        };

        if !confirmed {
            app.show_confirm(app::ConfirmAction::ExecuteCommand {
                title: "Delete Memory Entry?".to_string(),
                message: format!(
                    "This will permanently delete the memory entry file:\n\n  {path_str}\n\n  [Y] Yes, delete    [N] No, cancel"
                ),
                command: format!("/memory delete --confirmed {path_str}"),
            });
            return Ok(());
        }
    }

    let Some(workspace_dir) = app.session.workspace_dir().cloned() else {
        app.set_error("No workspace directory set for this session.");
        return Ok(());
    };

    let out = match super::slash::run_memory_subcommand(args, &app.session) {
        Ok(out) => out,
        Err(e) => {
            app.set_error(e);
            return Ok(());
        }
    };

    if let Some(first_line) = out.lines.first() {
        app.set_status(first_line.clone());
    }

    let Some(act) = out.live_action else {
        if !out.lines.is_empty() {
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: out.lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
        }
        return Ok(());
    };

    match execute_memory_live_action(rt, &workspace_dir, act) {
        Ok(MemoryExecOutput::Listed(entries)) => {
            if entries.is_empty() {
                app.set_status(
                    "Memory bank is empty. Use '/memory save' to store conversation context.",
                );
            } else {
                let mut lines = vec![
                    format!("━━━ Memory Bank ({} entries) ━━━", entries.len()),
                    String::new(),
                ];
                for entry in &entries {
                    lines.push(format!(
                        "  • [{}] {}",
                        entry.timestamp.format("%Y-%m-%d %H:%M"),
                        entry.summary
                    ));
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
        Ok(MemoryExecOutput::Searched { query, results }) => {
            let mut lines = vec![
                format!("━━━ Memory Search: '{query}' ({}) ━━━", results.len()),
                String::new(),
            ];
            for r in &results {
                lines.push(format!(
                    "  • [{}] {}",
                    r.timestamp.format("%Y-%m-%d %H:%M"),
                    r.summary
                ));
            }
            app.messages.push(app::TuiMessage {
                role: "system".to_string(),
                content: lines.join("\n"),
                thinking: None,
                is_streaming: false,
                is_error: false,
            });
        }
        Ok(MemoryExecOutput::Saved(path)) => {
            app.set_status(format!("Saved to memory bank: {}", path.display()));
        }
        Ok(MemoryExecOutput::Cleared(count)) => {
            app.set_status(format!("Cleared {count} memory bank entries."));
        }
        Ok(MemoryExecOutput::Deleted) => {
            app.set_status("Deleted memory entry".to_string());
        }
        Err(e) => {
            app.set_error(format!("Memory operation failed: {e}"));
        }
    }

    if out.changed && app.mode == TuiMode::Memory {
        open_memory_browser(app, rt);
    }

    Ok(())
}

/// Handle `/summarize` command - Summarize current conversation.
fn handle_summarize_command(app: &mut TuiApp) -> Result<()> {
    let history: Vec<String> = app
        .session
        .state
        .messages
        .iter()
        .map(|msg| msg.content.clone())
        .collect();

    if history.is_empty() {
        app.set_error("No conversation to summarize.");
    } else {
        use gestura_core::context::ContextManager;
        let context_manager = ContextManager::new();
        let summary = context_manager.summarize_history(&history);

        let mut lines = vec!["━━━ Conversation Summary ━━━".to_string(), String::new()];
        lines.push(summary);
        app.messages.push(app::TuiMessage {
            role: "system".to_string(),
            content: lines.join("\n"),
            thinking: None,
            is_streaming: false,
            is_error: false,
        });
    }
    Ok(())
}

fn handle_toggle_recording_action(
    app: &mut TuiApp,
    streaming: &Option<StreamingState>,
    voice_capture: &mut Option<VoiceCaptureState>,
    rt: &tokio::runtime::Runtime,
) {
    if streaming.is_some() {
        app.set_status("Still streaming… press Esc to cancel before recording".to_string());
        return;
    }

    if app.voice_capture_in_progress {
        app.set_status("Recording already in progress".to_string());
        return;
    }

    if !gestura_core::is_microphone_available() {
        app.set_error("No microphone available".to_string());
        return;
    }

    app.voice_capture_in_progress = true;
    app.set_status("Listening… speak now (silence stops recording; Esc cancels)".to_string());
    *voice_capture = Some(spawn_voice_capture(rt));
}

/// Spawn a non-blocking voice capture (record + transcribe) task.
fn spawn_voice_capture(rt: &tokio::runtime::Runtime) -> VoiceCaptureState {
    let (tx, rx) = mpsc::channel::<std::result::Result<String, String>>(1);

    rt.spawn(async move {
        // Ensure a previous cancellation doesn't immediately abort a new recording.
        gestura_core::reset_stop_flag();

        let speech_processor = get_speech_processor();

        let (duration, audio_path) = match speech_processor
            .record_audio_to_file(None)
            .await
            .map_err(|e| e.to_string())
        {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(Err(e)).await;
                return;
            }
        };

        if duration < 0.5 {
            let _ = tokio::fs::remove_file(&audio_path).await;
            let _ = tx
                .send(Err("Recording too short - no audio captured".to_string()))
                .await;
            return;
        }

        let res = match speech_processor.transcribe_audio(&audio_path).await {
            Ok(result) => {
                let _ = tokio::fs::remove_file(&audio_path).await;
                Ok(result.text)
            }
            Err(e) => Err(format!(
                "Transcription failed: {} (audio saved to {:?})",
                e, audio_path
            )),
        };

        let _ = tx.send(res).await;
    });

    VoiceCaptureState { receiver: rx }
}
