//! TUI Application state and mode management
//!
//! This module contains the core application state machine for the TUI,
//! including mode management, message handling, and state transitions.

use chrono::Utc;
use gestura_core::AppConfig;
use gestura_core::chat_sessions::MessageSource;
use ratatui::style::Color;
use ratatui::widgets::ListState;

use super::super::{ChatMessage, ChatSession};

/// Theme configuration for the TUI
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: &'static str,
    /// Background token for legacy/alternate header styles.
    ///
    /// The current Claude-like minimal header renders without a background bar, but we retain this
    /// token for other UI variants/themes.
    #[allow(dead_code)]
    pub header_bg: Color,
    pub header_fg: Color,
    pub user_msg: Color,
    pub assistant_msg: Color,
    pub system_msg: Color,
    pub error_msg: Color,
    pub streaming: Color,
    pub border: Color,
    pub border_focused: Color,
    /// Background token for legacy/alternate status bar styles.
    ///
    /// The current minimal status line avoids heavy background blocks, but we keep this token for
    /// future/alternative renderers.
    #[allow(dead_code)]
    pub status_bg: Color,
    pub status_fg: Color,
    pub mode_normal: Color,
    pub mode_insert: Color,
    pub mode_command: Color,
    pub tab_active: Color,
    /// Color token for inactive tab labels in the full tab-bar UI.
    ///
    /// The current Claude-like header doesn't render inactive tabs.
    #[allow(dead_code)]
    pub tab_inactive: Color,
    pub selection_bg: Color,
    // Code highlighting colors
    pub code_bg: Color,
    pub code_fg: Color,
    pub code_keyword: Color,
    pub code_string: Color,
    pub code_comment: Color,
    pub code_number: Color,
    pub code_function: Color,
    pub code_lang_label: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::pro()
    }
}

impl Theme {
    /// Select an initial TUI theme based on the configured UI theme mode.
    ///
    /// Gestura’s config stores a *mode* (`"system" | "light" | "dark"`) rather than a
    /// specific palette name. For the TUI, we map that mode into one of the built-in
    /// terminal-first themes:
    ///
    /// - `light`  → [`Theme::light`]
    /// - `dark`   → [`Theme::pro`] (Claude-like, dark)
    /// - `system` → [`Theme::pro`] (we currently treat system as dark for the TUI)
    ///
    /// Any unknown value falls back to [`Theme::default`].
    pub fn from_theme_mode(theme_mode: &str) -> Self {
        match theme_mode.trim().to_lowercase().as_str() {
            "light" => Self::light(),
            "dark" | "system" => Self::pro(),
            _ => Self::default(),
        }
    }

    /// Catppuccin Mocha theme (default dark theme)
    pub fn catppuccin_mocha() -> Self {
        Self {
            name: "Catppuccin Mocha",
            header_bg: Color::Rgb(30, 30, 46),
            header_fg: Color::Rgb(205, 214, 244),
            user_msg: Color::Rgb(166, 227, 161),
            assistant_msg: Color::Rgb(137, 180, 250),
            system_msg: Color::Rgb(249, 226, 175),
            error_msg: Color::Rgb(243, 139, 168),
            streaming: Color::Rgb(180, 190, 254),
            border: Color::Rgb(88, 91, 112),
            border_focused: Color::Rgb(137, 180, 250),
            status_bg: Color::Rgb(30, 30, 46),
            status_fg: Color::Rgb(166, 173, 200),
            mode_normal: Color::Rgb(137, 180, 250),
            mode_insert: Color::Rgb(166, 227, 161),
            mode_command: Color::Rgb(249, 226, 175),
            tab_active: Color::Rgb(137, 180, 250),
            tab_inactive: Color::Rgb(88, 91, 112),
            selection_bg: Color::Rgb(49, 50, 68),
            code_bg: Color::Rgb(24, 24, 37),
            code_fg: Color::Rgb(166, 173, 200),
            code_keyword: Color::Rgb(203, 166, 247),
            code_string: Color::Rgb(166, 227, 161),
            code_comment: Color::Rgb(108, 112, 134),
            code_number: Color::Rgb(250, 179, 135),
            code_function: Color::Rgb(137, 180, 250),
            code_lang_label: Color::Rgb(249, 226, 175),
        }
    }

    /// Light theme for bright terminals
    pub fn light() -> Self {
        Self {
            name: "Light",
            header_bg: Color::Rgb(239, 241, 245),
            header_fg: Color::Rgb(76, 79, 105),
            user_msg: Color::Rgb(64, 160, 43),
            assistant_msg: Color::Rgb(30, 102, 245),
            system_msg: Color::Rgb(223, 142, 29),
            error_msg: Color::Rgb(210, 15, 57),
            streaming: Color::Rgb(114, 135, 253),
            border: Color::Rgb(172, 176, 190),
            border_focused: Color::Rgb(30, 102, 245),
            status_bg: Color::Rgb(239, 241, 245),
            status_fg: Color::Rgb(92, 95, 119),
            mode_normal: Color::Rgb(30, 102, 245),
            mode_insert: Color::Rgb(64, 160, 43),
            mode_command: Color::Rgb(223, 142, 29),
            tab_active: Color::Rgb(30, 102, 245),
            tab_inactive: Color::Rgb(172, 176, 190),
            selection_bg: Color::Rgb(220, 224, 232),
            code_bg: Color::Rgb(230, 233, 239),
            code_fg: Color::Rgb(76, 79, 105),
            code_keyword: Color::Rgb(136, 57, 239),
            code_string: Color::Rgb(64, 160, 43),
            code_comment: Color::Rgb(140, 143, 161),
            code_number: Color::Rgb(254, 100, 11),
            code_function: Color::Rgb(30, 102, 245),
            code_lang_label: Color::Rgb(223, 142, 29),
        }
    }

    /// High contrast theme for accessibility
    pub fn high_contrast() -> Self {
        Self {
            name: "High Contrast",
            header_bg: Color::Black,
            header_fg: Color::White,
            user_msg: Color::Green,
            assistant_msg: Color::Cyan,
            system_msg: Color::Yellow,
            error_msg: Color::Red,
            streaming: Color::LightCyan,
            border: Color::White,
            border_focused: Color::LightYellow,
            status_bg: Color::Black,
            status_fg: Color::White,
            mode_normal: Color::Cyan,
            mode_insert: Color::Green,
            mode_command: Color::Yellow,
            tab_active: Color::LightYellow,
            tab_inactive: Color::Gray,
            selection_bg: Color::DarkGray,
            code_bg: Color::Black,
            code_fg: Color::White,
            code_keyword: Color::Magenta,
            code_string: Color::Green,
            code_comment: Color::Gray,
            code_number: Color::Yellow,
            code_function: Color::Cyan,
            code_lang_label: Color::Yellow,
        }
    }

    /// Dracula theme
    pub fn dracula() -> Self {
        Self {
            name: "Dracula",
            header_bg: Color::Rgb(40, 42, 54),
            header_fg: Color::Rgb(248, 248, 242),
            user_msg: Color::Rgb(80, 250, 123),
            assistant_msg: Color::Rgb(139, 233, 253),
            system_msg: Color::Rgb(241, 250, 140),
            error_msg: Color::Rgb(255, 85, 85),
            streaming: Color::Rgb(189, 147, 249),
            border: Color::Rgb(68, 71, 90),
            border_focused: Color::Rgb(139, 233, 253),
            status_bg: Color::Rgb(40, 42, 54),
            status_fg: Color::Rgb(98, 114, 164),
            mode_normal: Color::Rgb(139, 233, 253),
            mode_insert: Color::Rgb(80, 250, 123),
            mode_command: Color::Rgb(241, 250, 140),
            tab_active: Color::Rgb(139, 233, 253),
            tab_inactive: Color::Rgb(68, 71, 90),
            selection_bg: Color::Rgb(68, 71, 90),
            code_bg: Color::Rgb(33, 34, 44),
            code_fg: Color::Rgb(248, 248, 242),
            code_keyword: Color::Rgb(255, 121, 198),
            code_string: Color::Rgb(241, 250, 140),
            code_comment: Color::Rgb(98, 114, 164),
            code_number: Color::Rgb(189, 147, 249),
            code_function: Color::Rgb(80, 250, 123),
            code_lang_label: Color::Rgb(241, 250, 140),
        }
    }

    /// Pro / Claude-like theme (Minimalist Dark)
    pub fn pro() -> Self {
        Self {
            name: "Pro",
            header_bg: Color::Rgb(20, 20, 20),
            header_fg: Color::Rgb(200, 200, 200),
            user_msg: Color::Rgb(255, 255, 255), // White text for user
            assistant_msg: Color::Rgb(215, 186, 125), // Soft gold/yellow for AI
            system_msg: Color::Rgb(100, 100, 100),
            error_msg: Color::Rgb(255, 95, 95),
            streaming: Color::Rgb(215, 186, 125),
            border: Color::Rgb(60, 60, 60),
            border_focused: Color::Rgb(100, 100, 100),
            status_bg: Color::Rgb(30, 30, 30),
            status_fg: Color::Rgb(150, 150, 150),
            mode_normal: Color::Rgb(100, 100, 100),
            mode_insert: Color::Rgb(215, 186, 125), // Match AI color
            mode_command: Color::Rgb(255, 255, 255),
            tab_active: Color::Rgb(255, 255, 255),
            tab_inactive: Color::Rgb(80, 80, 80),
            selection_bg: Color::Rgb(40, 40, 40),
            code_bg: Color::Rgb(20, 20, 20),
            code_fg: Color::Rgb(200, 200, 200),
            code_keyword: Color::Rgb(86, 156, 214), // VS Code Blue
            code_string: Color::Rgb(206, 145, 120), // VS Code Orange
            code_comment: Color::Rgb(106, 153, 85), // VS Code Green
            code_number: Color::Rgb(181, 206, 168),
            code_function: Color::Rgb(220, 220, 170),
            code_lang_label: Color::Rgb(80, 80, 80),
        }
    }

    /// Get theme by name
    pub fn by_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "light" => Self::light(),
            "high-contrast" | "highcontrast" | "high_contrast" => Self::high_contrast(),
            "dracula" => Self::dracula(),
            "pro" | "claude" => Self::pro(),
            _ => Self::catppuccin_mocha(),
        }
    }

    /// List available theme names
    pub fn available_themes() -> &'static [&'static str] {
        &[
            "catppuccin-mocha",
            "light",
            "high-contrast",
            "dracula",
            "pro",
        ]
    }
}

/// Current mode of the TUI application
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TuiMode {
    /// Normal mode - navigation and commands
    #[default]
    Normal,
    /// Insert mode - typing a message
    Insert,
    /// Command mode - entering a slash command
    Command,
    /// Help overlay is displayed
    Help,
    /// Confirmation dialog is displayed
    Confirm,
    /// Tool confirmation dialog is displayed
    ///
    /// This is shown when the core pipeline emits a `StreamChunk::ToolConfirmationRequired`.
    /// The user must choose a scoped decision (allow/deny once/session/always).
    ToolConfirm,
    /// Search mode - searching through messages
    Search,
    /// Model picker overlay is displayed
    ModelPicker,
    /// Agent activity overlay is displayed
    Activity,
    /// Settings submenu mode - navigating settings tab
    Settings,
    /// Workflows submenu mode - navigating workflows tab
    Workflows,
    /// Tools submenu mode - viewing tools tab
    Tools,
}

/// Types of confirmation dialogs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Confirm quitting without saving
    QuitWithoutSave,
    /// Confirm clearing message history
    ClearMessages,
    /// Confirm starting a new session
    NewSession,
}

/// Pending tool confirmation request that must be resolved by the user.
///
/// This is a thin TUI adapter payload; all policy and caching is enforced in
/// `gestura-core` via `TOOL_CONFIRMATIONS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingToolConfirmation {
    /// Confirmation id emitted by the core pipeline.
    pub confirmation_id: String,
    /// Tool name that is requesting confirmation.
    pub tool_name: String,
    /// Tool arguments (stringified) to preview in the UI.
    pub tool_args: String,
    /// Human-readable description of the tool action.
    pub description: String,
    /// Risk level (0-100-ish) provided by the core tool inspection.
    pub risk_level: u8,
    /// Tool category (e.g. "filesystem", "network") provided by core inspection.
    pub category: String,
}

/// Actions that can be triggered by user input
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Continue the main loop
    Continue,
    /// Quit the application
    Quit,
    /// Send the current input as a message
    SendMessage(String),
    /// Execute a slash command
    ExecuteCommand(String),
    /// Switch to a different tab
    SwitchTab(usize),
    /// Toggle help overlay
    ToggleHelp,
    /// Scroll messages up
    ScrollUp,
    /// Scroll messages down
    ScrollDown,
    /// Scroll messages up by a page
    PageUp,
    /// Scroll messages down by a page
    PageDown,
    /// Clear the input field
    ClearInput,
    /// Cancel current operation (streaming, etc.)
    Cancel,
    /// Toggle voice recording
    ToggleRecording,
    /// Enhance the current prompt using LLM
    EnhancePrompt,
    /// Copy the currently selected message(s) to the system clipboard
    CopySelection,
}

/// Message for TUI display with additional metadata
#[derive(Debug, Clone)]
pub struct TuiMessage {
    pub role: String,
    pub content: String,
    pub thinking: Option<String>,
    pub is_streaming: bool,
    pub is_error: bool,
}

impl From<&ChatMessage> for TuiMessage {
    fn from(msg: &ChatMessage) -> Self {
        Self {
            role: msg.role.clone(),
            content: msg.content.clone(),
            thinking: msg.thinking.clone(),
            is_streaming: false,
            is_error: false,
        }
    }
}

/// Available slash commands
pub const COMMANDS: &[(&str, &str)] = &[
    ("/help", "Show help information"),
    ("/tools", "List available tools"),
    ("/tools <name>", "Show tool details"),
    ("/clear", "Clear message history"),
    ("/save", "Save current session"),
    ("/new", "Start a new session"),
    ("/history", "Show session statistics"),
    ("/quit", "Exit the application"),
    ("/settings", "Switch to settings tab"),
    ("/capabilities", "Show AI capabilities"),
    ("/model", "Select active model (interactive picker)"),
    (
        "/model <provider:model|model>",
        "Set session model (e.g. openai:gpt-4o or claude-3-5-sonnet)",
    ),
    ("/activity", "Toggle agent activity view"),
    ("/theme", "List available themes"),
    (
        "/theme <name>",
        "Change theme (catppuccin-mocha, light, high-contrast, dracula)",
    ),
    ("/search", "Enter interactive search mode"),
    ("/search <query>", "Search messages for query"),
    ("/sessions", "List all saved sessions"),
    ("/session list", "List all saved sessions"),
    ("/session load <id>", "Load/switch to a session"),
    ("/session delete <id>", "Delete a session"),
    ("/session export", "Export current session to file"),
    ("/session export <id>", "Export a session to file"),
    ("/session info", "Show current session details"),
    // --- Claude Code parity commands ---
    ("/rewind", "List session checkpoints"),
    ("/rewind <id>", "Restore session to a checkpoint"),
    ("/tasks", "Show current task list"),
    ("/hooks", "Show hooks configuration"),
    ("/permissions", "List granted tool permissions"),
    ("/permissions audit", "Show permission audit log"),
    ("/context", "Show resolved context/guardrails"),
];

/// TUI application state
pub struct TuiApp {
    /// Current input buffer
    pub input: String,
    /// Cursor position within input
    pub cursor_pos: usize,
    /// Chat messages
    pub messages: Vec<TuiMessage>,
    /// Message list state for scrolling
    pub message_list_state: ListState,
    /// Whether user has manually scrolled (disables auto-scroll)
    pub user_scrolled: bool,
    /// Chat session for persistence
    pub session: ChatSession,
    /// Application configuration
    pub config: AppConfig,
    /// Optional system prompt
    pub system_prompt: Option<String>,
    /// Current TUI mode
    pub mode: TuiMode,
    /// Whether we're waiting for a response
    pub is_loading: bool,
    /// Current status message
    pub status: String,
    /// Error message (if any)
    pub error: Option<String>,
    /// Timestamp when the error was set (for auto-dismiss)
    pub error_timestamp: Option<std::time::Instant>,
    /// Count of visible error messages in chat (limit 2)
    pub error_message_count: usize,
    /// If true, skip saving the session when the TUI exits.
    ///
    /// This is used for explicit "quit without saving" flows.
    pub skip_save_on_exit: bool,
    /// Active tab index
    pub active_tab: usize,
    /// Available tabs
    pub tabs: Vec<&'static str>,
    /// Available workflows (filename, description)
    pub workflows: Vec<(String, String)>,
    /// State for the settings tab
    pub settings_state: SettingsState,
    /// Command palette: filtered commands based on input
    pub command_suggestions: Vec<(String, String)>,
    /// Command palette: selected suggestion index
    pub command_selection: usize,
    /// Command history for up/down navigation
    pub command_history: Vec<String>,
    /// Current position in command history
    pub command_history_pos: Option<usize>,
    /// Pending confirmation action
    pub pending_confirm: Option<ConfirmAction>,
    /// Pending tool confirmation request (scoped allow/deny decision).
    pub pending_tool_confirmation: Option<PendingToolConfirmation>,
    /// Layout areas for mouse click detection (set during render)
    pub layout_areas: LayoutAreas,
    /// Current theme
    pub theme: Theme,
    /// Search query
    pub search_query: String,
    /// Search matches: (message_index, character_ranges)
    pub search_matches: Vec<(usize, Vec<std::ops::Range<usize>>)>,
    /// Current match index for n/N navigation
    pub current_match_idx: usize,
    /// Whether to filter to show only matching messages
    pub search_filter_mode: bool,
    /// Session token usage (input tokens)
    pub session_input_tokens: u64,
    /// Session token usage (output tokens)
    pub session_output_tokens: u64,
    /// Session estimated cost in USD
    pub session_cost_usd: f64,
    /// Original prompt before enhancement (for undo with Cmd+Z)
    pub original_prompt: Option<String>,

    /// Agent activity state (tool-call transcript, separate from chat transcript).
    pub activity_state: ActivityState,
    /// Model picker overlay state.
    pub model_picker_state: ModelPickerState,

    // ========== Scrolling & Selection ==========
    /// Total number of rendered lines in the last frame (set by `render_messages`).
    ///
    /// This count reflects the flattened line count (one `ListItem` per line) rather
    /// than the message count, so scroll bounds match the visual list.
    pub rendered_line_count: usize,
    /// Mapping from rendered-line index → source message index (set by `render_messages`).
    pub line_to_message_map: Vec<usize>,
    /// Start of the current mouse-drag text selection (rendered-line index).
    pub selection_anchor: Option<usize>,
    /// End of the current mouse-drag text selection (rendered-line index).
    pub selection_end: Option<usize>,
}

/// A single line in the agent activity transcript.
#[derive(Debug, Clone)]
pub struct ActivityEntry {
    /// Rendered activity text.
    pub text: String,
    /// Whether the entry represents an error.
    pub is_error: bool,
}

/// State for the agent activity overlay.
#[derive(Debug, Clone, Default)]
pub struct ActivityState {
    /// Activity entries in chronological order.
    pub entries: Vec<ActivityEntry>,
    /// Selection state used to implement scrolling in the activity view.
    pub list_state: ListState,
    /// Whether the user has manually scrolled (disables auto-scroll).
    pub user_scrolled: bool,
}

impl ActivityState {
    /// Append a new activity entry.
    pub fn push(&mut self, entry: ActivityEntry) {
        self.entries.push(entry);
        if !self.user_scrolled {
            self.scroll_to_bottom();
        }
    }

    /// Clear all activity entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.list_state.select(None);
        self.user_scrolled = false;
    }

    /// Scroll to the bottom of activity entries.
    pub fn scroll_to_bottom(&mut self) {
        if !self.entries.is_empty() {
            self.list_state
                .select(Some(self.entries.len().saturating_sub(1)));
        }
        self.user_scrolled = false;
    }

    /// Scroll up in the activity list.
    pub fn scroll_up(&mut self) {
        self.user_scrolled = true;
        let current = self.list_state.selected().unwrap_or(0);
        if current > 0 {
            self.list_state.select(Some(current - 1));
        }
    }

    /// Scroll down in the activity list.
    pub fn scroll_down(&mut self) {
        let current = self.list_state.selected().unwrap_or(0);
        let max = self.entries.len().saturating_sub(1);
        if current < max {
            self.list_state.select(Some(current + 1));
        }
        if self.is_at_bottom() {
            self.user_scrolled = false;
        }
    }

    /// Returns true if the activity view is currently at the bottom.
    pub fn is_at_bottom(&self) -> bool {
        let selected = self.list_state.selected().unwrap_or(0);
        selected >= self.entries.len().saturating_sub(1)
    }
}

/// A model picker option.
#[derive(Debug, Clone)]
pub struct ModelPickerItem {
    /// Display label (typically `provider:model`).
    pub label: String,
    /// Provider id (e.g. `openai`).
    pub provider: String,
    /// Model id (provider-specific).
    pub model: String,
}

/// State for the model picker overlay.
#[derive(Debug, Clone, Default)]
pub struct ModelPickerState {
    /// User-entered filter string.
    pub query: String,
    /// All available picker options.
    pub items: Vec<ModelPickerItem>,
    /// Indices into `items` representing the filtered view.
    pub filtered: Vec<usize>,
    /// Selection state for the filtered list.
    pub list_state: ListState,
}

impl ModelPickerState {
    /// Reset the picker query and selection.
    pub fn reset(&mut self) {
        self.query.clear();
        self.filtered = (0..self.items.len()).collect();
        self.list_state.select(self.filtered.first().copied());
    }

    /// Recompute `filtered` based on the current query.
    pub fn refilter(&mut self) {
        let q = self.query.trim().to_ascii_lowercase();
        if q.is_empty() {
            self.filtered = (0..self.items.len()).collect();
        } else {
            self.filtered = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(idx, item)| {
                    let hay = item.label.to_ascii_lowercase();
                    hay.contains(&q).then_some(idx)
                })
                .collect();
        }

        // Clamp selection into the filtered list.
        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else {
            let selected = self.list_state.selected().unwrap_or(0);
            let clamped = selected.min(self.filtered.len() - 1);
            self.list_state.select(Some(clamped));
        }
    }

    /// Move selection up (within the filtered list).
    pub fn select_prev(&mut self) {
        let current = self.list_state.selected().unwrap_or(0);
        if current > 0 {
            self.list_state.select(Some(current - 1));
        }
    }

    /// Move selection down (within the filtered list).
    pub fn select_next(&mut self) {
        let current = self.list_state.selected().unwrap_or(0);
        let max = self.filtered.len().saturating_sub(1);
        if current < max {
            self.list_state.select(Some(current + 1));
        }
    }

    /// Return the currently selected picker item.
    pub fn selected_item(&self) -> Option<&ModelPickerItem> {
        let selected = self.list_state.selected()?;
        let idx = *self.filtered.get(selected)?;
        self.items.get(idx)
    }
}

/// State for the interactive settings editor
#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    /// Currently selected field index
    pub selected_field: usize,
    /// Whether we are currently editing the field
    pub is_editing: bool,
    /// Temporary value buffer while editing
    pub edit_buffer: String,
}

/// Settings fields available for editing
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SettingsField {
    Provider = 0,
    Model = 1,
    SystemPrompt = 2,
    Temperature = 3,
}

impl SettingsField {
    pub const COUNT: usize = 4;

    pub fn from_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(Self::Provider),
            1 => Some(Self::Model),
            2 => Some(Self::SystemPrompt),
            3 => Some(Self::Temperature),
            _ => None,
        }
    }
}

/// Cached layout areas for mouse click detection
#[derive(Debug, Clone, Default)]
pub struct LayoutAreas {
    /// Tab header area
    pub tabs: Option<ratatui::layout::Rect>,
    /// Message list area
    pub messages: Option<ratatui::layout::Rect>,
    /// Input field area
    pub input: Option<ratatui::layout::Rect>,
}

impl TuiApp {
    /// Create a new TUI application
    pub fn new(session: ChatSession, config: AppConfig, system_prompt: Option<String>) -> Self {
        let messages: Vec<TuiMessage> = session
            .state
            .messages
            .iter()
            .map(TuiMessage::from)
            .collect();

        let initial_theme = Theme::from_theme_mode(&config.ui.theme_mode);

        let mut message_list_state = ListState::default();
        // Select the last message if any exist
        if !messages.is_empty() {
            message_list_state.select(Some(messages.len().saturating_sub(1)));
        }

        Self {
            input: String::new(),
            cursor_pos: 0,
            messages,
            message_list_state,
            user_scrolled: false,
            session,
            config,
            system_prompt,
            mode: TuiMode::Insert, // Start in insert mode for immediate typing
            is_loading: false,
            status: "Ready".to_string(),
            error: None,
            error_timestamp: None,
            error_message_count: 0,
            skip_save_on_exit: false,
            active_tab: 0,
            tabs: vec!["Chat", "Workflows", "Tools", "Settings", "Help"],
            workflows: Vec::new(),
            settings_state: SettingsState::default(),
            command_suggestions: Vec::new(),
            command_selection: 0,
            command_history: Vec::new(),
            command_history_pos: None,
            pending_confirm: None,
            pending_tool_confirmation: None,
            layout_areas: LayoutAreas::default(),
            theme: initial_theme,
            search_query: String::new(),
            search_matches: Vec::new(),
            current_match_idx: 0,
            search_filter_mode: false,
            session_input_tokens: 0,
            session_output_tokens: 0,
            session_cost_usd: 0.0,
            original_prompt: None,

            activity_state: ActivityState::default(),
            model_picker_state: ModelPickerState::default(),

            rendered_line_count: 0,
            line_to_message_map: Vec::new(),
            selection_anchor: None,
            selection_end: None,
        }
    }

    /// Record token usage from an LLM call
    pub fn record_token_usage(
        &mut self,
        input_tokens: u32,
        output_tokens: u32,
        cost_usd: Option<f64>,
    ) {
        self.session_input_tokens += input_tokens as u64;
        self.session_output_tokens += output_tokens as u64;
        self.session_cost_usd += cost_usd.unwrap_or(0.0);
    }

    /// Get compact formatted token usage for status bar (format: "1.2K|$0.01")
    /// Used when terminal width is limited (< 80 columns)
    #[allow(dead_code)]
    pub fn format_token_usage_compact(&self) -> String {
        let total = self.session_input_tokens + self.session_output_tokens;
        if total == 0 {
            return String::new();
        }

        let formatted_total = if total >= 1_000_000 {
            format!("{:.1}M", total as f64 / 1_000_000.0)
        } else if total >= 1_000 {
            format!("{:.1}K", total as f64 / 1_000.0)
        } else {
            total.to_string()
        };

        if self.session_cost_usd > 0.001 {
            format!("{}|${:.2}", formatted_total, self.session_cost_usd)
        } else {
            formatted_total
        }
    }

    /// Set the theme by name
    pub fn set_theme(&mut self, name: &str) {
        self.theme = Theme::by_name(name);
        self.set_status(format!("Theme changed to: {}", self.theme.name));
    }

    /// Cycle to the next theme
    pub fn cycle_theme(&mut self) {
        let themes = Theme::available_themes();
        let current_idx = themes
            .iter()
            .position(|&t| t == self.theme.name.to_lowercase().replace(' ', "-"))
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % themes.len();
        self.set_theme(themes[next_idx]);
    }

    /// Show a confirmation dialog
    pub fn show_confirm(&mut self, action: ConfirmAction) {
        self.pending_confirm = Some(action);
        self.mode = TuiMode::Confirm;
    }

    /// Show a tool confirmation overlay.
    ///
    /// The decision itself is resolved via `gestura-core` once the user chooses.
    pub fn show_tool_confirmation(&mut self, pending: PendingToolConfirmation) {
        self.pending_tool_confirmation = Some(pending);
        self.mode = TuiMode::ToolConfirm;
    }

    /// Cancel the confirmation dialog
    pub fn cancel_confirm(&mut self) {
        self.pending_confirm = None;
        self.mode = TuiMode::Insert;
    }

    /// Get the pending confirmation action and clear it
    pub fn take_confirm(&mut self) -> Option<ConfirmAction> {
        self.mode = TuiMode::Insert;
        self.pending_confirm.take()
    }

    /// Get the pending tool confirmation and clear it.
    ///
    /// This returns the pending payload and returns the UI to Insert mode.
    pub fn take_tool_confirmation(&mut self) -> Option<PendingToolConfirmation> {
        self.mode = TuiMode::Insert;
        self.pending_tool_confirmation.take()
    }

    /// Return the command token (the first whitespace-delimited segment) from a command spec.
    ///
    /// For example, `"/tools <name>"` becomes `"/tools"`.
    fn command_token(cmd: &str) -> &str {
        cmd.split_whitespace().next().unwrap_or(cmd)
    }

    /// Return the byte range of the command token currently being edited.
    ///
    /// If the cursor is already past the first whitespace (i.e. editing arguments), this
    /// returns `None` so the command palette can be hidden.
    fn command_token_range(&self) -> Option<std::ops::Range<usize>> {
        if self.input.is_empty() {
            return None;
        }

        let token_end = self
            .input
            .find(|c: char| c.is_whitespace())
            .unwrap_or(self.input.len());

        // If the cursor is past the token, we consider the user to be typing args.
        if self.cursor_pos > token_end {
            return None;
        }

        Some(0..token_end)
    }

    /// Update command suggestions based on the current command token.
    ///
    /// UX goals:
    /// - Prefer prefix matches (e.g. `/h` ranks `/help` above `/history`).
    /// - Allow description matches as a fallback.
    /// - Hide suggestions once the user is typing arguments (after a space).
    pub fn update_command_suggestions(&mut self) {
        let Some(range) = self.command_token_range() else {
            self.command_suggestions.clear();
            self.command_selection = 0;
            return;
        };

        let query = self.input[range].to_lowercase();
        if !query.starts_with('/') {
            self.command_suggestions.clear();
            self.command_selection = 0;
            return;
        }

        let query_no_slash = query.trim_start_matches('/');
        let show_all = query_no_slash.is_empty();

        let mut scored: Vec<(i32, String, String)> = Vec::new();
        for (cmd, desc) in COMMANDS.iter() {
            let cmd_token = Self::command_token(cmd).to_lowercase();
            let desc_lc = desc.to_lowercase();

            let score = if show_all || cmd_token == query {
                Some(0)
            } else if cmd_token.starts_with(&query) {
                // Prefer shorter completions when both are prefix matches.
                Some(10 + (cmd_token.len().saturating_sub(query.len()) as i32))
            } else if let Some(pos) = cmd_token.find(&query) {
                Some(100 + (pos as i32))
            } else if desc_lc.contains(query_no_slash) {
                Some(200)
            } else {
                None
            };

            if let Some(score) = score {
                scored.push((score, (*cmd).to_string(), (*desc).to_string()));
            }
        }

        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        self.command_suggestions = scored
            .into_iter()
            .map(|(_, cmd, desc)| (cmd, desc))
            .collect();

        // Clamp selection to new list.
        if self.command_selection >= self.command_suggestions.len() {
            self.command_selection = 0;
        }
    }

    /// Select next command suggestion
    pub fn next_command_suggestion(&mut self) {
        if !self.command_suggestions.is_empty() {
            self.command_selection = (self.command_selection + 1) % self.command_suggestions.len();
        }
    }

    /// Select previous command suggestion
    pub fn prev_command_suggestion(&mut self) {
        if !self.command_suggestions.is_empty() {
            self.command_selection = if self.command_selection == 0 {
                self.command_suggestions.len() - 1
            } else {
                self.command_selection - 1
            };
        }
    }

    /// Apply selected command suggestion to input
    pub fn apply_command_suggestion(&mut self) {
        let Some((cmd, _)) = self.command_suggestions.get(self.command_selection) else {
            return;
        };
        let Some(range) = self.command_token_range() else {
            return;
        };

        // Extract just the command token (before any <arg> template).
        let cmd_token = Self::command_token(cmd);
        let suffix = &self.input[range.end..];
        let new_suffix = if suffix.is_empty() {
            " ".to_string()
        } else {
            suffix.to_string()
        };

        self.input = format!("{}{}", cmd_token, new_suffix);
        self.cursor_pos = self.input.len();
    }

    /// Add command to history
    pub fn add_to_command_history(&mut self, cmd: &str) {
        // Don't add duplicates of the last command
        if self.command_history.last().map(|s| s.as_str()) != Some(cmd) {
            self.command_history.push(cmd.to_string());
        }
        self.command_history_pos = None;
    }

    /// Navigate to previous command in history
    pub fn prev_command_history(&mut self) {
        if self.command_history.is_empty() {
            return;
        }
        let new_pos = match self.command_history_pos {
            None => self.command_history.len() - 1,
            Some(0) => 0,
            Some(p) => p - 1,
        };
        self.command_history_pos = Some(new_pos);
        if let Some(cmd) = self.command_history.get(new_pos) {
            self.input = cmd.clone();
            self.cursor_pos = self.input.len();
        }
    }

    /// Navigate to next command in history
    pub fn next_command_history(&mut self) {
        if self.command_history.is_empty() {
            return;
        }
        match self.command_history_pos {
            None => {}
            Some(p) if p >= self.command_history.len() - 1 => {
                self.command_history_pos = None;
                self.input.clear();
                self.cursor_pos = 0;
            }
            Some(p) => {
                self.command_history_pos = Some(p + 1);
                if let Some(cmd) = self.command_history.get(p + 1) {
                    self.input = cmd.clone();
                    self.cursor_pos = self.input.len();
                }
            }
        }
    }

    /// Get the effective model name for display (respects session overrides).
    ///
    /// This delegates to the same precedence logic used by the pipeline to ensure
    /// the UI always shows the model that will actually be used for inference.
    ///
    /// Precedence:
    /// 1) `session.state.llm_config` (session-scoped overrides via `/model`)
    /// 2) legacy `session.model` hint (supports `provider:model`)
    /// 3) `config.llm.primary` + provider default model
    pub fn model_name(&self) -> String {
        let (_provider, model) = super::effective_provider_model_for_ui(self);
        model
    }

    /// Get the effective provider name for display (respects session overrides).
    ///
    /// This delegates to the same precedence logic used by the pipeline to ensure
    /// the UI always shows the provider that will actually be used for inference.
    ///
    /// Precedence:
    /// 1) `session.state.llm_config.provider` (session-scoped overrides via `/model`)
    /// 2) legacy `session.model` hint (supports `provider:model`)
    /// 3) `config.llm.primary`
    pub fn provider_name(&self) -> String {
        let (provider, _model) = super::effective_provider_model_for_ui(self);
        provider
    }

    /// Add a message to the chat
    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(TuiMessage {
            role: role.to_string(),
            content: content.to_string(),
            thinking: None,
            is_streaming: false,
            is_error: false,
        });

        match role {
            "user" => self.session.add_user_message(content, MessageSource::Text),
            "assistant" => self.session.add_assistant_message(content, None),
            other => {
                // Best-effort persistence for uncommon roles.
                self.session.state.messages.push(ChatMessage {
                    role: other.to_string(),
                    content: content.to_string(),
                    tool_call_id: None,
                    thinking: None,
                    timestamp: Utc::now(),
                    source: MessageSource::System,
                });
            }
        }

        // Auto-scroll to bottom unless user has scrolled up
        if !self.user_scrolled {
            self.scroll_to_bottom();
        }
    }

    /// Add a streaming message (placeholder that will be updated)
    pub fn add_streaming_message(&mut self) {
        self.messages.push(TuiMessage {
            role: "assistant".to_string(),
            content: String::new(),
            thinking: None,
            is_streaming: true,
            is_error: false,
        });
        if !self.user_scrolled {
            self.scroll_to_bottom();
        }
    }

    /// Update the last message (for streaming)
    pub fn update_last_message(&mut self, content: &str) {
        if let Some(last) = self.messages.last_mut() {
            last.content = content.to_string();
        }
    }

    /// Mark the last message as an error (used when the stream fails).
    pub fn mark_last_message_error(&mut self) {
        if let Some(last) = self.messages.last_mut() {
            last.is_error = true;
            last.is_streaming = false;
        }
    }

    /// Push a non-persisted system error message into the UI.
    ///
    /// This does not write into the persisted `ChatSession` history.
    /// Limited to 2 visible error messages in the session to avoid clutter.
    /// Critical errors (connection failures, API quota exceeded) are shown as chat messages.
    pub fn push_error_message(&mut self, content: impl Into<String>) {
        const MAX_ERROR_MESSAGES: usize = 2;

        // Check if we've hit the limit
        if self.error_message_count >= MAX_ERROR_MESSAGES {
            // Remove the oldest error message to make room
            if let Some(idx) = self.messages.iter().position(|m| m.is_error) {
                self.messages.remove(idx);
                self.error_message_count = self.error_message_count.saturating_sub(1);
            }
        }

        self.messages.push(TuiMessage {
            role: "system".to_string(),
            content: content.into(),
            thinking: None,
            is_streaming: false,
            is_error: true,
        });
        self.error_message_count += 1;

        if !self.user_scrolled {
            self.scroll_to_bottom();
        }
    }

    /// Update the last message thinking content
    pub fn update_last_message_thinking(&mut self, thinking: &str) {
        if let Some(last) = self.messages.last_mut() {
            last.thinking = Some(thinking.to_string());
        }
    }

    /// Finalize the streaming message
    pub fn finalize_streaming_message(&mut self) {
        if let Some(last) = self.messages.last_mut() {
            last.is_streaming = false;

            // Also save to session
            match last.role.as_str() {
                "assistant" => self
                    .session
                    .add_assistant_message(&last.content, last.thinking.clone()),
                "user" => self
                    .session
                    .add_user_message(&last.content, MessageSource::Text),
                other => {
                    self.session.state.messages.push(ChatMessage {
                        role: other.to_string(),
                        content: last.content.clone(),
                        tool_call_id: None,
                        thinking: last.thinking.clone(),
                        timestamp: Utc::now(),
                        source: MessageSource::System,
                    });
                }
            }
        }
    }

    /// Scroll to the bottom of messages.
    ///
    /// Uses the rendered line count (from the last frame) when available so the
    /// selection lands on the actual last visible line, not just the last message.
    pub fn scroll_to_bottom(&mut self) {
        let max = if self.rendered_line_count > 0 {
            self.rendered_line_count
        } else {
            self.messages.len()
        };
        if max > 0 {
            self.message_list_state.select(Some(max.saturating_sub(1)));
        }
        self.user_scrolled = false;
    }

    /// Scroll up by one line in the rendered message list.
    pub fn scroll_up(&mut self) {
        self.user_scrolled = true;
        let current = self.message_list_state.selected().unwrap_or(0);
        if current > 0 {
            self.message_list_state.select(Some(current - 1));
        }
    }

    /// Scroll down by one line in the rendered message list.
    pub fn scroll_down(&mut self) {
        let current = self.message_list_state.selected().unwrap_or(0);
        let max = self.max_scroll_index();
        if current < max {
            self.message_list_state.select(Some(current + 1));
        }
        // If we're at the bottom, re-enable auto-scroll
        if self.is_at_bottom() {
            self.user_scrolled = false;
        }
    }

    /// Scroll up by a page (roughly one screenful of lines).
    pub fn page_up(&mut self, page_size: usize) {
        self.user_scrolled = true;
        let current = self.message_list_state.selected().unwrap_or(0);
        let target = current.saturating_sub(page_size);
        self.message_list_state.select(Some(target));
    }

    /// Scroll down by a page (roughly one screenful of lines).
    pub fn page_down(&mut self, page_size: usize) {
        let current = self.message_list_state.selected().unwrap_or(0);
        let max = self.max_scroll_index();
        let target = (current + page_size).min(max);
        self.message_list_state.select(Some(target));
        if self.is_at_bottom() {
            self.user_scrolled = false;
        }
    }

    /// Maximum valid scroll index (last rendered line, or last message as fallback).
    fn max_scroll_index(&self) -> usize {
        if self.rendered_line_count > 0 {
            self.rendered_line_count.saturating_sub(1)
        } else {
            self.messages.len().saturating_sub(1)
        }
    }

    /// Insert a character at cursor position
    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor_pos, c);
        self.cursor_pos += 1;
    }

    /// Delete the character before cursor
    pub fn delete_char_before(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            self.input.remove(self.cursor_pos);
        }
    }

    /// Delete the character after cursor
    pub fn delete_char_after(&mut self) {
        if self.cursor_pos < self.input.len() {
            self.input.remove(self.cursor_pos);
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        if self.cursor_pos < self.input.len() {
            self.cursor_pos += 1;
        }
    }

    /// Move cursor to start
    pub fn cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to end
    pub fn cursor_end(&mut self) {
        self.cursor_pos = self.input.len();
    }

    /// Move cursor to next word (vim 'w' motion)
    pub fn cursor_word_forward(&mut self) {
        let chars: Vec<char> = self.input.chars().collect();
        let len = chars.len();
        if self.cursor_pos >= len {
            return;
        }

        // Skip current word (non-whitespace)
        while self.cursor_pos < len && !chars[self.cursor_pos].is_whitespace() {
            self.cursor_pos += 1;
        }
        // Skip whitespace
        while self.cursor_pos < len && chars[self.cursor_pos].is_whitespace() {
            self.cursor_pos += 1;
        }
    }

    /// Move cursor to previous word (vim 'b' motion)
    pub fn cursor_word_backward(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }

        let chars: Vec<char> = self.input.chars().collect();

        // Move back one to start
        self.cursor_pos -= 1;

        // Skip whitespace
        while self.cursor_pos > 0 && chars[self.cursor_pos].is_whitespace() {
            self.cursor_pos -= 1;
        }
        // Skip to start of word
        while self.cursor_pos > 0 && !chars[self.cursor_pos - 1].is_whitespace() {
            self.cursor_pos -= 1;
        }
    }

    /// Delete word before cursor (vim 'db' or Ctrl+W)
    pub fn delete_word_before(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }

        let chars: Vec<char> = self.input.chars().collect();
        let original_pos = self.cursor_pos;

        // Skip whitespace
        while self.cursor_pos > 0 && chars[self.cursor_pos - 1].is_whitespace() {
            self.cursor_pos -= 1;
        }
        // Skip to start of word
        while self.cursor_pos > 0 && !chars[self.cursor_pos - 1].is_whitespace() {
            self.cursor_pos -= 1;
        }

        // Remove the characters
        self.input = chars[..self.cursor_pos]
            .iter()
            .chain(chars[original_pos..].iter())
            .collect();
    }

    /// Delete to end of line (vim 'D' or Ctrl+K)
    pub fn delete_to_end(&mut self) {
        self.input.truncate(self.cursor_pos);
    }

    /// Clear the input field
    pub fn clear_input(&mut self) {
        self.input.clear();
        self.cursor_pos = 0;
    }

    /// Take the current input (clears the input buffer)
    pub fn take_input(&mut self) -> String {
        let input = std::mem::take(&mut self.input);
        self.cursor_pos = 0;
        input
    }

    /// Set status message
    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
        self.error = None;
    }

    /// Set error message with timestamp for auto-dismiss
    pub fn set_error(&mut self, error: impl Into<String>) {
        let err = error.into();
        self.status = format!("Error: {}", &err);
        self.error = Some(err);
        self.error_timestamp = Some(std::time::Instant::now());
    }

    /// Clear error after successful operation or timeout
    pub fn clear_error(&mut self) {
        self.error = None;
        self.error_timestamp = None;
        self.status = "Ready".to_string();
    }

    /// Check if error should be auto-dismissed (15 second timeout)
    /// Returns true if error was cleared
    pub fn check_error_timeout(&mut self) -> bool {
        const AUTO_DISMISS_SECS: u64 = 15;
        if let Some(timestamp) = self.error_timestamp
            && timestamp.elapsed().as_secs() >= AUTO_DISMISS_SECS
        {
            self.clear_error();
            return true;
        }
        false
    }

    /// Get the scroll position indicator (e.g., "5/23").
    ///
    /// Uses the rendered line count when available so the denominator reflects
    /// the actual number of visual lines, not just the message count.
    pub fn scroll_indicator(&self) -> String {
        let current = self.message_list_state.selected().unwrap_or(0) + 1;
        let total = if self.rendered_line_count > 0 {
            self.rendered_line_count
        } else {
            self.messages.len()
        };
        if total == 0 {
            "0/0".to_string()
        } else {
            format!("{}/{}", current, total)
        }
    }

    /// Check if we're at the bottom of the message list.
    ///
    /// Compares against the rendered line count so this remains correct even
    /// when messages expand to many visual lines.
    pub fn is_at_bottom(&self) -> bool {
        let current = self.message_list_state.selected().unwrap_or(0);
        let total = if self.rendered_line_count > 0 {
            self.rendered_line_count
        } else {
            self.messages.len()
        };
        current + 1 >= total
    }

    // ========== Search Methods ==========

    /// Start search mode
    pub fn start_search(&mut self) {
        self.mode = TuiMode::Search;
        self.search_query.clear();
        self.search_matches.clear();
        self.current_match_idx = 0;
    }

    /// Update search query and find matches
    ///
    /// Programmatic search update - sets query and finds matches.
    /// Used by `/search <query>` command and Ctrl+Shift+F (search from clipboard).
    pub fn update_search(&mut self, query: &str) {
        self.search_query = query.to_string();
        self.find_matches();
        self.current_match_idx = 0;
        // Jump to first match if any
        if !self.search_matches.is_empty() {
            self.jump_to_match(0);
        }
    }

    /// Add a character to search query
    pub fn search_insert_char(&mut self, c: char) {
        self.search_query.push(c);
        self.find_matches();
        // Jump to first match if any
        if !self.search_matches.is_empty() && self.current_match_idx == 0 {
            self.jump_to_match(0);
        }
    }

    /// Remove last character from search query
    pub fn search_backspace(&mut self) {
        self.search_query.pop();
        self.find_matches();
        self.current_match_idx = 0;
        if !self.search_matches.is_empty() {
            self.jump_to_match(0);
        }
    }

    /// Find all matches in messages (case-insensitive)
    fn find_matches(&mut self) {
        self.search_matches.clear();

        if self.search_query.is_empty() {
            return;
        }

        let query_lower = self.search_query.to_lowercase();

        for (msg_idx, msg) in self.messages.iter().enumerate() {
            let content_lower = msg.content.to_lowercase();
            let mut ranges = Vec::new();

            // Find all occurrences
            let mut start = 0;
            while let Some(pos) = content_lower[start..].find(&query_lower) {
                let abs_pos = start + pos;
                ranges.push(abs_pos..abs_pos + query_lower.len());
                start = abs_pos + 1;
            }

            if !ranges.is_empty() {
                self.search_matches.push((msg_idx, ranges));
            }
        }

        // Update status with match count
        let total_matches: usize = self.search_matches.iter().map(|(_, r)| r.len()).sum();
        if total_matches > 0 {
            self.status = format!(
                "Found {} match{} in {} message{}",
                total_matches,
                if total_matches == 1 { "" } else { "es" },
                self.search_matches.len(),
                if self.search_matches.len() == 1 {
                    ""
                } else {
                    "s"
                }
            );
        } else if !self.search_query.is_empty() {
            self.status = "No matches found".to_string();
        }
    }

    /// Jump to a specific match by index
    fn jump_to_match(&mut self, match_idx: usize) {
        if match_idx < self.search_matches.len() {
            let (msg_idx, _) = &self.search_matches[match_idx];
            self.message_list_state.select(Some(*msg_idx));
            self.user_scrolled = true;
        }
    }

    /// Go to next search match
    pub fn next_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        self.current_match_idx = (self.current_match_idx + 1) % self.search_matches.len();
        self.jump_to_match(self.current_match_idx);
        self.status = format!(
            "Match {}/{}",
            self.current_match_idx + 1,
            self.search_matches.len()
        );
    }

    /// Go to previous search match
    pub fn prev_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        if self.current_match_idx == 0 {
            self.current_match_idx = self.search_matches.len() - 1;
        } else {
            self.current_match_idx -= 1;
        }
        self.jump_to_match(self.current_match_idx);
        self.status = format!(
            "Match {}/{}",
            self.current_match_idx + 1,
            self.search_matches.len()
        );
    }

    /// Toggle filter mode (show only matching messages)
    pub fn toggle_search_filter(&mut self) {
        self.search_filter_mode = !self.search_filter_mode;
        if self.search_filter_mode {
            self.status = "Filter mode: showing only matches".to_string();
        } else {
            self.status = "Filter mode: off".to_string();
        }
    }

    /// Cancel search and return to normal mode
    pub fn cancel_search(&mut self) {
        self.mode = TuiMode::Normal;
        self.search_query.clear();
        self.search_matches.clear();
        self.search_filter_mode = false;
        self.status = "Ready".to_string();
    }

    /// Confirm search and return to normal mode (keep highlights)
    pub fn confirm_search(&mut self) {
        self.mode = TuiMode::Normal;
        if self.search_matches.is_empty() {
            self.status = "No matches found".to_string();
        } else {
            self.status = format!(
                "Found {} matches - n/N to navigate, Esc to clear",
                self.search_matches.len()
            );
        }
    }

    /// Clear search results (called when pressing Esc in normal mode with active search)
    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.search_matches.clear();
        self.current_match_idx = 0;
        self.search_filter_mode = false;
        self.status = "Search cleared".to_string();
    }

    /// Get match ranges for a specific message
    pub fn get_match_ranges(&self, msg_idx: usize) -> Option<&Vec<std::ops::Range<usize>>> {
        self.search_matches
            .iter()
            .find(|(idx, _)| *idx == msg_idx)
            .map(|(_, ranges)| ranges)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::chat::new_cli_session;

    /// Helper to create a test app with a mock session
    fn create_test_app() -> TuiApp {
        let session = new_cli_session(None).unwrap();
        let config = AppConfig::default();
        TuiApp::new(session, config, None)
    }

    // ==================== State Transition Tests ====================

    #[test]
    fn test_initial_state() {
        let app = create_test_app();
        // App starts in Insert mode for immediate typing
        assert_eq!(app.mode, TuiMode::Insert);
        assert!(app.input.is_empty());
        assert_eq!(app.cursor_pos, 0);
        assert!(app.messages.is_empty());
    }

    #[test]
    fn test_mode_transitions() {
        let mut app = create_test_app();

        // Start in Insert mode, switch to Normal
        app.mode = TuiMode::Normal;
        assert_eq!(app.mode, TuiMode::Normal);

        // Normal -> Insert
        app.mode = TuiMode::Insert;
        assert_eq!(app.mode, TuiMode::Insert);

        // Insert -> Command
        app.mode = TuiMode::Command;
        assert_eq!(app.mode, TuiMode::Command);

        // Command -> Help
        app.mode = TuiMode::Help;
        assert_eq!(app.mode, TuiMode::Help);

        // Help -> Search
        app.mode = TuiMode::Search;
        assert_eq!(app.mode, TuiMode::Search);

        // Search -> Confirm
        app.mode = TuiMode::Confirm;
        assert_eq!(app.mode, TuiMode::Confirm);
    }

    // ==================== Input Handling Tests ====================

    #[test]
    fn test_insert_char() {
        let mut app = create_test_app();

        app.insert_char('H');
        app.insert_char('e');
        app.insert_char('l');
        app.insert_char('l');
        app.insert_char('o');

        assert_eq!(app.input, "Hello");
        assert_eq!(app.cursor_pos, 5);
    }

    #[test]
    fn test_delete_char_before() {
        let mut app = create_test_app();
        app.input = "Hello".to_string();
        app.cursor_pos = 5;

        app.delete_char_before();
        assert_eq!(app.input, "Hell");
        assert_eq!(app.cursor_pos, 4);

        // Delete at position 0 should do nothing
        app.cursor_pos = 0;
        app.delete_char_before();
        assert_eq!(app.input, "Hell");
    }

    #[test]
    fn test_delete_char_after() {
        let mut app = create_test_app();
        app.input = "Hello".to_string();
        app.cursor_pos = 0;

        app.delete_char_after();
        assert_eq!(app.input, "ello");
        assert_eq!(app.cursor_pos, 0);

        // Delete at end should do nothing
        app.cursor_pos = 4;
        app.delete_char_after();
        assert_eq!(app.input, "ello");
    }

    #[test]
    fn test_cursor_movement() {
        let mut app = create_test_app();
        app.input = "Hello".to_string();
        app.cursor_pos = 5;

        // Move left
        app.cursor_left();
        assert_eq!(app.cursor_pos, 4);

        // Move left multiple times
        app.cursor_left();
        app.cursor_left();
        assert_eq!(app.cursor_pos, 2);

        // Move right
        app.cursor_right();
        assert_eq!(app.cursor_pos, 3);

        // Move to start
        app.cursor_home();
        assert_eq!(app.cursor_pos, 0);

        // Move to end
        app.cursor_end();
        assert_eq!(app.cursor_pos, 5);

        // Can't move left past 0
        app.cursor_pos = 0;
        app.cursor_left();
        assert_eq!(app.cursor_pos, 0);

        // Can't move right past end
        app.cursor_pos = 5;
        app.cursor_right();
        assert_eq!(app.cursor_pos, 5);
    }

    #[test]
    fn test_insert_at_cursor_position() {
        let mut app = create_test_app();
        app.input = "Hllo".to_string();
        app.cursor_pos = 1;

        app.insert_char('e');
        assert_eq!(app.input, "Hello");
        assert_eq!(app.cursor_pos, 2);
    }

    // ==================== Message Management Tests ====================

    #[test]
    fn test_add_message() {
        let mut app = create_test_app();

        app.add_message("user", "Hello");
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, "user");
        assert_eq!(app.messages[0].content, "Hello");

        app.add_message("assistant", "Hi there!");
        assert_eq!(app.messages.len(), 2);
        assert_eq!(app.messages[1].role, "assistant");
    }

    #[test]
    fn test_scroll_bounds() {
        let mut app = create_test_app();

        // Add 10 messages
        for i in 0..10 {
            app.add_message("user", &format!("Message {}", i));
        }

        // Scroll up from bottom
        app.scroll_to_bottom();
        let _initial_selection = app.message_list_state.selected();

        app.scroll_up();
        let after_up = app.message_list_state.selected();
        assert!(after_up.is_some());

        // Scroll down
        app.scroll_down();
        let after_down = app.message_list_state.selected();
        assert!(after_down.is_some());

        // Scroll to bottom
        app.scroll_to_bottom();
        assert_eq!(app.message_list_state.selected(), Some(9));
    }

    #[test]
    fn test_scroll_empty_messages() {
        let mut app = create_test_app();

        // Should not panic with empty messages
        app.scroll_up();
        app.scroll_down();
        app.scroll_to_bottom();

        assert!(app.message_list_state.selected().is_none());
    }

    // ==================== Search Functionality Tests ====================

    #[test]
    fn test_search_messages() {
        let mut app = create_test_app();

        app.add_message("user", "Hello world");
        app.add_message("assistant", "Hi there, world!");
        app.add_message("user", "How are you?");

        // Use update_search which sets query and finds matches
        app.update_search("world");

        assert_eq!(app.search_query, "world");
        assert_eq!(app.search_matches.len(), 2); // Two messages contain "world"
    }

    #[test]
    fn test_search_case_insensitive() {
        let mut app = create_test_app();

        app.add_message("user", "Hello WORLD");
        app.add_message("assistant", "world is great");

        app.update_search("World");

        assert_eq!(app.search_matches.len(), 2);
    }

    #[test]
    fn test_search_navigation() {
        let mut app = create_test_app();

        app.add_message("user", "First target");
        app.add_message("assistant", "No hit here");
        app.add_message("user", "Second target");
        app.add_message("assistant", "Third target");

        app.update_search("target");
        assert_eq!(app.search_matches.len(), 3);
        assert_eq!(app.current_match_idx, 0);

        app.next_match();
        assert_eq!(app.current_match_idx, 1);

        app.next_match();
        assert_eq!(app.current_match_idx, 2);

        // Wrap around
        app.next_match();
        assert_eq!(app.current_match_idx, 0);

        // Previous
        app.prev_match();
        assert_eq!(app.current_match_idx, 2);
    }

    #[test]
    fn test_clear_search() {
        let mut app = create_test_app();

        app.add_message("user", "Hello world");
        app.update_search("world");
        assert!(!app.search_query.is_empty());

        app.clear_search();
        assert!(app.search_query.is_empty());
        assert!(app.search_matches.is_empty());
        assert_eq!(app.current_match_idx, 0);
    }

    // ==================== Theme Tests ====================

    #[test]
    fn test_theme_switching() {
        let mut app = create_test_app();

        // Default theme is "Pro" (display name, not slug)
        let initial_theme = app.theme.name;
        assert_eq!(initial_theme, "Pro");

        app.set_theme("light");
        assert_eq!(app.theme.name, "Light");

        app.set_theme("high-contrast");
        assert_eq!(app.theme.name, "High Contrast");

        app.set_theme("dracula");
        assert_eq!(app.theme.name, "Dracula");

        // Invalid theme falls back to catppuccin-mocha (the fallback in by_name)
        app.set_theme("nonexistent");
        assert_eq!(app.theme.name, "Catppuccin Mocha");
    }

    #[test]
    fn test_theme_cycling() {
        let mut app = create_test_app();

        // Get initial theme name and verify it's the default (Pro)
        assert_eq!(app.theme.name, "Pro");

        // Cycle to next theme - Pro is at index 4, so cycling wraps to index 0
        app.cycle_theme();
        // After cycling from pro (index 4), should wrap to catppuccin-mocha (index 0)
        assert_eq!(app.theme.name, "Catppuccin Mocha");

        // Cycle again
        app.cycle_theme();
        assert_eq!(app.theme.name, "Light");

        // Cycle again
        app.cycle_theme();
        assert_eq!(app.theme.name, "High Contrast");

        // Cycle again
        app.cycle_theme();
        assert_eq!(app.theme.name, "Dracula");

        // Cycle wraps back to Pro
        app.cycle_theme();
        assert_eq!(app.theme.name, "Pro");
    }

    // ==================== Command Suggestion Tests ====================

    #[test]
    fn test_command_suggestions_prefix_ranking() {
        let mut app = create_test_app();

        // Both /help and /history should match, but /help should rank first (shorter prefix match).
        app.input = "/h".to_string();
        app.update_command_suggestions();

        assert!(!app.command_suggestions.is_empty());
        let first = &app.command_suggestions[0].0;
        assert!(first.starts_with("/help"));
    }

    #[test]
    fn test_command_suggestions_hide_when_typing_args() {
        let mut app = create_test_app();
        app.input = "/tools ".to_string();
        app.cursor_pos = app.input.len();
        app.update_command_suggestions();
        assert!(app.command_suggestions.is_empty());
    }

    #[test]
    fn test_apply_command_suggestion_completes_token_and_adds_space() {
        let mut app = create_test_app();

        app.input = "/to".to_string();
        app.cursor_pos = app.input.len();
        app.update_command_suggestions();

        let idx = app
            .command_suggestions
            .iter()
            .position(|(cmd, _)| cmd.contains("/tools <name>"))
            .expect("expected /tools <name> to be suggested");
        app.command_selection = idx;

        app.apply_command_suggestion();
        assert_eq!(app.input, "/tools ");
        assert_eq!(app.cursor_pos, app.input.len());
    }

    #[test]
    fn test_command_suggestion_navigation() {
        let mut app = create_test_app();

        app.input = "/".to_string();
        app.update_command_suggestions();

        let total = app.command_suggestions.len();
        assert!(total > 0);

        app.next_command_suggestion();
        assert_eq!(app.command_selection, 1);

        app.prev_command_suggestion();
        assert_eq!(app.command_selection, 0);

        // Wrap around
        app.prev_command_suggestion();
        assert_eq!(app.command_selection, total - 1);
    }

    // ==================== Status and Error Tests ====================

    #[test]
    fn test_set_status() {
        let mut app = create_test_app();

        app.set_status("Loading...");
        assert_eq!(app.status, "Loading...");
    }

    #[test]
    fn test_set_error() {
        let mut app = create_test_app();

        app.set_error("Something went wrong");
        assert!(app.error.is_some());
        assert_eq!(app.error.as_ref().unwrap(), "Something went wrong");
    }

    // ==================== Tab Navigation Tests ====================

    #[test]
    fn test_tab_navigation() {
        let mut app = create_test_app();

        // 5 tabs: Chat, Workflows, Tools, Settings, Help (indices 0-4)
        assert_eq!(app.tabs.len(), 5);
        assert_eq!(app.active_tab, 0);

        // Simulate tab switching (done via direct field access in events.rs)
        app.active_tab = (app.active_tab + 1) % app.tabs.len();
        assert_eq!(app.active_tab, 1);

        app.active_tab = (app.active_tab + 1) % app.tabs.len();
        assert_eq!(app.active_tab, 2);

        app.active_tab = (app.active_tab + 1) % app.tabs.len();
        assert_eq!(app.active_tab, 3);

        app.active_tab = (app.active_tab + 1) % app.tabs.len();
        assert_eq!(app.active_tab, 4);

        // Wrap around from 4 to 0
        app.active_tab = (app.active_tab + 1) % app.tabs.len();
        assert_eq!(app.active_tab, 0);

        // Previous tab (from 0 wraps to 4)
        app.active_tab = if app.active_tab == 0 {
            app.tabs.len() - 1
        } else {
            app.active_tab - 1
        };
        assert_eq!(app.active_tab, 4);
    }

    // ==================== Vim Motion Tests ====================

    #[test]
    fn test_word_forward_motion() {
        let mut app = create_test_app();
        app.input = "hello world test".to_string();
        app.cursor_pos = 0;

        app.cursor_word_forward();
        // Should skip to after "hello " (position 6)
        assert!(app.cursor_pos > 0);
    }

    #[test]
    fn test_word_backward_motion() {
        let mut app = create_test_app();
        app.input = "hello world test".to_string();
        app.cursor_pos = 12; // In "test"

        app.cursor_word_backward();
        // Should move back to start of "world" or "test"
        assert!(app.cursor_pos < 12);
    }

    #[test]
    fn test_delete_word_before() {
        let mut app = create_test_app();
        app.input = "hello world".to_string();
        app.cursor_pos = 11; // At end

        app.delete_word_before();
        // Should delete "world"
        assert!(!app.input.contains("world") || app.input.len() < 11);
    }

    #[test]
    fn test_delete_to_end() {
        let mut app = create_test_app();
        app.input = "hello world".to_string();
        app.cursor_pos = 5; // After "hello"

        app.delete_to_end();
        assert_eq!(app.input, "hello");
    }

    // ==================== Scroll Indicator Tests ====================

    #[test]
    fn test_scroll_indicator() {
        let mut app = create_test_app();

        // Empty messages
        assert_eq!(app.scroll_indicator(), "0/0");

        // Add messages
        app.add_message("user", "Message 1");
        app.add_message("user", "Message 2");
        app.add_message("user", "Message 3");

        app.scroll_to_bottom();
        assert_eq!(app.scroll_indicator(), "3/3");

        app.scroll_up();
        assert_eq!(app.scroll_indicator(), "2/3");
    }

    // ==================== Performance Tests ====================

    #[test]
    fn test_large_message_history() {
        let mut app = create_test_app();

        // Add 1000+ messages to test performance
        for i in 0..1000 {
            app.add_message("user", &format!("User message {}", i));
            app.add_message("assistant", &format!("Assistant response {}", i));
        }

        assert_eq!(app.messages.len(), 2000);

        // Verify scrolling still works
        app.scroll_to_bottom();
        assert_eq!(app.message_list_state.selected(), Some(1999));

        app.scroll_up();
        assert_eq!(app.message_list_state.selected(), Some(1998));

        // Verify scroll indicator
        assert_eq!(app.scroll_indicator(), "1999/2000");
    }

    #[test]
    fn test_rapid_input() {
        let mut app = create_test_app();

        // Simulate rapid typing
        let test_string = "The quick brown fox jumps over the lazy dog. ".repeat(10);
        for c in test_string.chars() {
            app.insert_char(c);
        }

        assert_eq!(app.input.len(), test_string.len());
        assert_eq!(app.cursor_pos, test_string.len());

        // Rapid deletion
        for _ in 0..100 {
            app.delete_char_before();
        }

        assert_eq!(app.input.len(), test_string.len() - 100);
    }

    #[test]
    fn test_search_large_history() {
        let mut app = create_test_app();

        // Add many messages with searchable content
        for i in 0..500 {
            app.add_message("user", &format!("Message number {} with keyword", i));
            app.add_message("assistant", &format!("Response {} without the word", i));
        }

        // Search should find all user messages
        app.update_search("keyword");
        assert_eq!(app.search_matches.len(), 500);

        // Navigate through matches
        for _ in 0..10 {
            app.next_match();
        }
        assert_eq!(app.current_match_idx, 10);
    }

    #[test]
    fn test_cursor_operations_long_input() {
        let mut app = create_test_app();

        // Create a very long input
        app.input = "a".repeat(10000);
        app.cursor_pos = 5000;

        // Test cursor operations on long input
        app.cursor_left();
        assert_eq!(app.cursor_pos, 4999);

        app.cursor_right();
        assert_eq!(app.cursor_pos, 5000);

        app.cursor_home();
        assert_eq!(app.cursor_pos, 0);

        app.cursor_end();
        assert_eq!(app.cursor_pos, 10000);

        // Word navigation on long input
        app.cursor_pos = 5000;
        app.cursor_word_forward();
        // Should move to end since it's all 'a' characters
        assert!(app.cursor_pos > 5000);
    }
}
