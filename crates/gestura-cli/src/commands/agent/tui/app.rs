//! TUI Application state and mode management
//!
//! This module contains the core application state machine for the TUI,
//! including mode management, message handling, and state transitions.

use std::collections::HashMap;

use chrono::Utc;
use gestura_core::AppConfig;
use gestura_core::agent_sessions::MessageSource;
use gestura_core::platform::detect_system_dark_mode;
use ratatui::style::Color;
use ratatui::widgets::ListState;

use super::super::{AgentMessage, AgentSession};

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
        // Default to Gestura's GUI-matched palette so the CLI TUI aligns with the desktop app.
        Self::gestura()
    }
}

impl Theme {
    /// Select an initial TUI theme based on the configured UI theme mode.
    ///
    /// Gestura’s config stores a *mode* (`"system" | "light" | "dark"`) rather than a
    /// specific palette name. For the TUI we default to the **Gestura** theme so the
    /// terminal UI matches the GUI styling (using the GUI token palette).
    ///
    /// Note: this function doesn't receive an `accent`, so it falls back to the GUI's
    /// default accent (`blue`). For accent-aware initialization prefer
    /// [`Theme::from_config`].
    #[allow(dead_code)]
    pub fn from_theme_mode(theme_mode: &str) -> Self {
        Self::gestura_for(theme_mode, Some("blue"))
    }

    /// Create an initial theme from the full application config.
    ///
    /// This allows the TUI to respect both `theme_mode` and `accent`, matching the GUI.
    pub fn from_config(config: &AppConfig) -> Self {
        Self::gestura_for(&config.ui.theme_mode, config.ui.accent.as_deref())
    }

    fn is_dark_theme_mode(theme_mode: &str) -> bool {
        match theme_mode.trim().to_ascii_lowercase().as_str() {
            "light" => false,
            "dark" => true,
            "system" => detect_system_dark_mode(),
            // Default to dark: keeps terminals readable when the user provides an
            // unknown value.
            _ => true,
        }
    }

    fn accent_color(is_dark: bool, accent: Option<&str>) -> Color {
        let accent = accent.unwrap_or("blue").trim().to_ascii_lowercase();
        match (accent.as_str(), is_dark) {
            // Keep in sync with `crates/gestura-gui/frontend/src/app/ThemeController.tsx`
            ("blue", false) => Color::Rgb(37, 99, 235),
            ("blue", true) => Color::Rgb(96, 165, 250),
            ("emerald", false) => Color::Rgb(16, 185, 129),
            ("emerald", true) => Color::Rgb(52, 211, 153),
            ("amber", false) => Color::Rgb(245, 158, 11),
            ("amber", true) => Color::Rgb(251, 191, 36),
            ("purple", false) => Color::Rgb(139, 92, 246),
            ("purple", true) => Color::Rgb(167, 139, 250),
            ("rose", false) => Color::Rgb(244, 63, 94),
            ("rose", true) => Color::Rgb(251, 113, 133),
            // Unknown accent -> GUI default.
            (_, false) => Color::Rgb(37, 99, 235),
            (_, true) => Color::Rgb(96, 165, 250),
        }
    }

    /// Catppuccin theme — Mocha (dark) or Latte (light).
    pub fn catppuccin(is_dark: bool) -> Self {
        if is_dark {
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
        } else {
            // Catppuccin Latte (official light palette)
            Self {
                name: "Catppuccin Latte",
                header_bg: Color::Rgb(239, 241, 245),    // base
                header_fg: Color::Rgb(76, 79, 105),      // text
                user_msg: Color::Rgb(64, 160, 43),       // green
                assistant_msg: Color::Rgb(30, 102, 245), // blue
                system_msg: Color::Rgb(223, 142, 29),    // yellow
                error_msg: Color::Rgb(210, 15, 57),      // red
                streaming: Color::Rgb(114, 135, 253),    // lavender
                border: Color::Rgb(172, 176, 190),       // surface2
                border_focused: Color::Rgb(30, 102, 245),
                status_bg: Color::Rgb(239, 241, 245),
                status_fg: Color::Rgb(92, 95, 119), // subtext0
                mode_normal: Color::Rgb(30, 102, 245),
                mode_insert: Color::Rgb(64, 160, 43),
                mode_command: Color::Rgb(223, 142, 29),
                tab_active: Color::Rgb(30, 102, 245),
                tab_inactive: Color::Rgb(172, 176, 190),
                selection_bg: Color::Rgb(220, 224, 232), // surface1
                code_bg: Color::Rgb(230, 233, 239),      // mantle
                code_fg: Color::Rgb(76, 79, 105),
                code_keyword: Color::Rgb(136, 57, 239), // mauve
                code_string: Color::Rgb(64, 160, 43),
                code_comment: Color::Rgb(124, 127, 147), // overlay1 (deeper for ≥3:1 on mantle)
                code_number: Color::Rgb(254, 100, 11),   // peach
                code_function: Color::Rgb(30, 102, 245),
                code_lang_label: Color::Rgb(223, 142, 29),
            }
        }
    }

    /// Catppuccin Mocha (dark) — convenience alias.
    #[allow(dead_code)]
    pub fn catppuccin_mocha() -> Self {
        Self::catppuccin(true)
    }

    /// Light theme — convenience alias for `catppuccin(false)`.
    ///
    /// The standalone "Light" theme is now the Catppuccin Latte palette so that
    /// every named theme has a coherent identity in both modes.
    #[allow(dead_code)]
    pub fn light() -> Self {
        Self::catppuccin(false)
    }

    /// High contrast theme for accessibility (adapts to light/dark).
    pub fn high_contrast_for(is_dark: bool) -> Self {
        if is_dark {
            Self {
                name: "High Contrast",
                header_bg: Color::Black,
                header_fg: Color::White,
                user_msg: Color::Green,
                assistant_msg: Color::LightBlue,
                system_msg: Color::Yellow,
                error_msg: Color::Red,
                streaming: Color::LightCyan,
                border: Color::White,
                border_focused: Color::LightBlue,
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
        } else {
            Self {
                name: "High Contrast",
                header_bg: Color::White,
                header_fg: Color::Black,
                user_msg: Color::Rgb(0, 100, 0),      // dark green
                assistant_msg: Color::Rgb(0, 0, 180), // dark blue
                system_msg: Color::Rgb(160, 100, 0),  // dark amber
                error_msg: Color::Rgb(180, 0, 0),     // dark red
                streaming: Color::Rgb(0, 0, 180),
                border: Color::Black,
                border_focused: Color::Rgb(0, 0, 180),
                status_bg: Color::White,
                status_fg: Color::Black,
                mode_normal: Color::Rgb(0, 0, 180),
                mode_insert: Color::Rgb(0, 100, 0),
                mode_command: Color::Rgb(160, 100, 0),
                tab_active: Color::Rgb(0, 0, 180),
                tab_inactive: Color::Rgb(120, 120, 120),
                selection_bg: Color::Rgb(210, 210, 210),
                code_bg: Color::Rgb(245, 245, 245),
                code_fg: Color::Black,
                code_keyword: Color::Rgb(140, 0, 140), // dark magenta
                code_string: Color::Rgb(0, 100, 0),
                code_comment: Color::Rgb(120, 120, 120),
                code_number: Color::Rgb(160, 100, 0),
                code_function: Color::Rgb(0, 120, 120), // dark cyan
                code_lang_label: Color::Rgb(160, 100, 0),
            }
        }
    }

    /// High contrast theme — convenience alias (dark variant).
    #[allow(dead_code)]
    pub fn high_contrast() -> Self {
        Self::high_contrast_for(true)
    }

    /// Dracula theme (adapts to light/dark).
    ///
    /// The light variant uses Dracula's official "Day" palette with softened
    /// versions of the signature hues on a warm-white background.
    pub fn dracula_for(is_dark: bool) -> Self {
        if is_dark {
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
        } else {
            // Dracula "Day" — warm-white bg with deeper Dracula accent hues.
            Self {
                name: "Dracula",
                header_bg: Color::Rgb(248, 248, 242),
                header_fg: Color::Rgb(40, 42, 54),
                user_msg: Color::Rgb(18, 140, 60), // deeper green
                assistant_msg: Color::Rgb(30, 130, 180), // deeper cyan
                system_msg: Color::Rgb(160, 140, 10), // deepened yellow
                error_msg: Color::Rgb(210, 50, 50), // deeper red
                streaming: Color::Rgb(130, 90, 210), // deeper purple
                border: Color::Rgb(175, 175, 190), // firmer for visibility
                border_focused: Color::Rgb(30, 130, 180),
                status_bg: Color::Rgb(248, 248, 242),
                status_fg: Color::Rgb(80, 95, 145), // deeper for readability
                mode_normal: Color::Rgb(30, 130, 180),
                mode_insert: Color::Rgb(18, 140, 60),
                mode_command: Color::Rgb(160, 140, 10),
                tab_active: Color::Rgb(30, 130, 180),
                tab_inactive: Color::Rgb(140, 145, 165), // deeper for readability
                selection_bg: Color::Rgb(220, 222, 232),
                code_bg: Color::Rgb(237, 238, 244), // slightly deeper for distinction
                code_fg: Color::Rgb(40, 42, 54),
                code_keyword: Color::Rgb(200, 70, 140), // deeper pink
                code_string: Color::Rgb(160, 140, 10),
                code_comment: Color::Rgb(105, 115, 150), // deeper for ~3.5:1 contrast on code_bg
                code_number: Color::Rgb(130, 90, 210),
                code_function: Color::Rgb(18, 140, 60),
                code_lang_label: Color::Rgb(140, 125, 10),
            }
        }
    }

    /// Dracula theme — convenience alias (dark variant).
    #[allow(dead_code)]
    pub fn dracula() -> Self {
        Self::dracula_for(true)
    }

    /// Pro / Claude-like theme (adapts to light/dark).
    ///
    /// Dark: minimalist near-black. Light: clean white with muted accents.
    pub fn pro_for(is_dark: bool) -> Self {
        if is_dark {
            let accent = Color::Rgb(137, 180, 250);
            Self {
                name: "Pro",
                header_bg: Color::Rgb(20, 20, 20),
                header_fg: Color::Rgb(200, 200, 200),
                user_msg: Color::Rgb(255, 255, 255),
                assistant_msg: accent,
                system_msg: Color::Rgb(100, 100, 100),
                error_msg: Color::Rgb(255, 95, 95),
                streaming: accent,
                border: Color::Rgb(60, 60, 60),
                border_focused: accent,
                status_bg: Color::Rgb(30, 30, 30),
                status_fg: Color::Rgb(150, 150, 150),
                mode_normal: Color::Rgb(100, 100, 100),
                mode_insert: accent,
                mode_command: Color::Rgb(255, 255, 255),
                tab_active: Color::Rgb(255, 255, 255),
                tab_inactive: Color::Rgb(80, 80, 80),
                selection_bg: Color::Rgb(40, 40, 40),
                code_bg: Color::Rgb(20, 20, 20),
                code_fg: Color::Rgb(200, 200, 200),
                code_keyword: Color::Rgb(86, 156, 214),
                code_string: Color::Rgb(206, 145, 120),
                code_comment: Color::Rgb(106, 153, 85),
                code_number: Color::Rgb(181, 206, 168),
                code_function: Color::Rgb(220, 220, 170),
                code_lang_label: Color::Rgb(80, 80, 80),
            }
        } else {
            let accent = Color::Rgb(50, 100, 200); // deeper blue for light bg
            Self {
                name: "Pro",
                header_bg: Color::Rgb(252, 252, 252),
                header_fg: Color::Rgb(50, 50, 50),
                user_msg: Color::Rgb(30, 30, 30),
                assistant_msg: accent,
                system_msg: Color::Rgb(120, 120, 120), // darker for readability
                error_msg: Color::Rgb(200, 50, 50),
                streaming: accent,
                border: Color::Rgb(195, 195, 195), // firmer for visibility
                border_focused: accent,
                status_bg: Color::Rgb(248, 248, 248),
                status_fg: Color::Rgb(110, 110, 110), // slightly deeper
                mode_normal: Color::Rgb(110, 110, 110),
                mode_insert: accent,
                mode_command: Color::Rgb(30, 30, 30),
                tab_active: Color::Rgb(30, 30, 30),
                tab_inactive: Color::Rgb(150, 150, 150), // deeper for readability
                selection_bg: Color::Rgb(225, 230, 242),
                code_bg: Color::Rgb(240, 240, 242), // clearer separation from white bg
                code_fg: Color::Rgb(50, 50, 50),
                code_keyword: Color::Rgb(0, 80, 170), // VS Code Blue (light)
                code_string: Color::Rgb(163, 21, 21), // VS Code Red-brown
                code_comment: Color::Rgb(0, 128, 0),  // VS Code Green
                code_number: Color::Rgb(9, 134, 88),
                code_function: Color::Rgb(121, 94, 38),
                code_lang_label: Color::Rgb(120, 120, 120),
            }
        }
    }

    /// Pro theme — convenience alias (dark variant).
    #[allow(dead_code)]
    pub fn pro() -> Self {
        Self::pro_for(true)
    }

    /// Gestura brand theme (blue → purple).
    ///
    /// This theme mirrors the GUI's design tokens (`--background`, `--surface`, `--border`,
    /// `--text`, `--text-secondary`) and uses the configured accent when available.
    ///
    /// Gradients are represented in the UI renderer via per-span RGB coloring.
    pub fn gestura() -> Self {
        // Default to the GUI's default accent and dark palette.
        Self::gestura_for("dark", Some("blue"))
    }

    /// Build the Gestura theme from a UI mode + accent.
    ///
    /// `theme_mode` uses the same semantics as the GUI (`system|light|dark`). For the
    /// TUI we currently treat `system` as dark (same as the previous TUI behavior).
    pub fn gestura_for(theme_mode: &str, accent: Option<&str>) -> Self {
        let is_dark = Self::is_dark_theme_mode(theme_mode);
        let accent = Self::accent_color(is_dark, accent);

        // Brand palette aligned with gestura.app website theme.
        let (background, surface, border, text, text_secondary) = if is_dark {
            (
                Color::Rgb(10, 10, 10),    // --background (near-black, website dark)
                Color::Rgb(26, 26, 26),    // --surface (dark gray, website dark)
                Color::Rgb(51, 65, 85),    // --border (slate-700)
                Color::Rgb(241, 245, 249), // --text (slate-100)
                Color::Rgb(148, 163, 184), // --text-secondary (slate-400)
            )
        } else {
            (
                Color::Rgb(255, 255, 255), // --background
                Color::Rgb(241, 245, 249), // --surface (slate-100, deeper than 50 for visible code/selection bg)
                Color::Rgb(203, 213, 225), // --border (slate-300, firmer than slate-200)
                Color::Rgb(30, 41, 59),    // --foreground (slate-800, website light)
                Color::Rgb(100, 116, 139), // --text-secondary (slate-500)
            )
        };

        // Secondary brand color from the blue→violet gradient endpoint.
        let secondary = if is_dark {
            Color::Rgb(167, 139, 250) // purple-400 (#a78bfa)
        } else {
            Color::Rgb(124, 58, 237) // violet-600 (#7c3aed)
        };

        let error = Color::Rgb(239, 68, 68); // red-500

        // Code syntax highlighting (conventional, softened to fit brand).
        let code_string = if is_dark {
            Color::Rgb(94, 234, 212) // teal-300
        } else {
            Color::Rgb(13, 148, 136) // teal-600
        };
        let code_number = if is_dark {
            Color::Rgb(251, 191, 36) // amber-400
        } else {
            Color::Rgb(217, 119, 6) // amber-600
        };

        Self {
            name: "Gestura",
            header_bg: surface,
            header_fg: text,

            // Message role styling:
            // - User messages use primary text (white/near-white in dark mode).
            // - Assistant messages use the focused-border accent (so transcript matches the
            //   input's focused border).
            // - System messages use the muted secondary text.
            user_msg: if is_dark {
                Color::Rgb(255, 255, 255)
            } else {
                text
            },
            assistant_msg: accent,
            system_msg: text_secondary,
            error_msg: error,
            streaming: accent,

            border,
            border_focused: accent,

            status_bg: background,
            status_fg: text_secondary,

            // Mode colors use brand palette (muted / blue / violet) for visual
            // distinction without relying on non-brand green or amber.
            mode_normal: text_secondary,
            mode_insert: accent,
            mode_command: secondary,

            tab_active: accent,
            tab_inactive: text_secondary,

            selection_bg: surface,

            // Code blocks: surface bg with brand-tinted syntax colors.
            code_bg: surface,
            code_fg: text,
            code_keyword: accent,
            code_string,
            code_comment: text_secondary,
            code_number,
            code_function: secondary,
            code_lang_label: text_secondary,
        }
    }

    /// Get a theme by name, adapting to the given `theme_mode`.
    ///
    /// `theme_mode` follows the same semantics as the config (`system|light|dark`).
    /// Every named theme produces a coherent palette for both light and dark modes.
    pub fn by_name_for(name: &str, theme_mode: &str) -> Self {
        let is_dark = Self::is_dark_theme_mode(theme_mode);
        match name.to_lowercase().as_str() {
            "catppuccin" | "catppuccin-mocha" | "catppuccin-latte" => Self::catppuccin(is_dark),
            "light" => Self::catppuccin(false), // explicit light always
            "high-contrast" | "highcontrast" | "high_contrast" => Self::high_contrast_for(is_dark),
            "dracula" => Self::dracula_for(is_dark),
            "gestura" | "brand" => Self::gestura_for(theme_mode, None),
            "pro" | "claude" => Self::pro_for(is_dark),
            _ => Self::catppuccin(is_dark),
        }
    }

    /// Get theme by name (dark variant — backwards-compatible convenience).
    #[allow(dead_code)]
    pub fn by_name(name: &str) -> Self {
        Self::by_name_for(name, "dark")
    }

    /// Stable theme keys used for cycling and persistence.
    ///
    /// These keys are independent of the display name (which may change with
    /// light/dark mode, e.g. "Catppuccin Mocha" vs "Catppuccin Latte").
    pub fn available_themes() -> &'static [&'static str] {
        &["catppuccin", "high-contrast", "dracula", "gestura", "pro"]
    }

    /// Map a display name back to its stable theme key.
    pub fn theme_key(&self) -> &'static str {
        match self.name {
            "Catppuccin Mocha" | "Catppuccin Latte" => "catppuccin",
            "High Contrast" => "high-contrast",
            "Dracula" => "dracula",
            "Gestura" => "gestura",
            "Pro" => "pro",
            _ => "catppuccin",
        }
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
    /// Capabilities overlay is displayed (reference popup, Esc to close)
    Capabilities,
    /// MCP browser overlay — interactive list of MCP servers
    Mcp,
    /// Knowledge browser overlay — interactive list of knowledge items
    Knowledge,
    /// Hooks browser overlay — interactive hooks management
    Hooks,
    /// Agent browser overlay — agent status and configuration
    Agent,
    /// Memory browser overlay — memory bank management
    Memory,
    /// Devices browser overlay — audio device listing
    Devices,
    /// Permissions browser overlay — permission management
    Permissions,
    /// Sessions browser overlay — session management
    Sessions,
    /// Tasks browser overlay — task management
    Tasks,
    /// Themes browser overlay — theme selection
    Themes,
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
    /// Generic confirmation that executes a slash command when accepted.
    ///
    /// This is used by interactive overlays (e.g., delete actions) to request
    /// confirmation and then delegate the actual mutation to the canonical
    /// slash-command handlers.
    ExecuteCommand {
        /// Title to display in the dialog.
        title: String,
        /// Message body to display in the dialog.
        message: String,
        /// Slash command to execute on confirm (e.g. "/memory delete --confirmed ...").
        command: String,
    },
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
    /// Copy a single message's raw markdown content to the system clipboard.
    ///
    /// This is used by the per-assistant-message "copy" overlay control in the transcript.
    CopyMessageRaw(usize),
    /// Resume a previously paused streaming session
    ResumeSession,
}

/// Clickable region for the transcript "copy" overlay button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyButtonHit {
    /// Source message index in `TuiApp.messages`.
    pub message_index: usize,
    /// Screen-space rectangle to hit-test mouse clicks.
    pub rect: ratatui::layout::Rect,
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

impl From<&AgentMessage> for TuiMessage {
    fn from(msg: &AgentMessage) -> Self {
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
        "Change theme (catppuccin-mocha, light, high-contrast, dracula, gestura, pro)",
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
    ("/tasks", "Open tasks browser (or: /tasks <subcommand>)"),
    ("/task", "Manage tasks (try: /task help)"),
    ("/hooks", "Open hooks browser (or: /hooks <subcommand>)"),
    ("/permissions", "List granted tool permissions"),
    ("/permissions audit", "Show permission audit log"),
    ("/context", "Show context manager status"),
    ("/context status", "Show context cache statistics"),
    (
        "/context analyze <request>",
        "Analyze a request (categories, tools, entities)",
    ),
    ("/context categories", "List all context categories"),
    ("/context clear", "Clear all context caches"),
    // --- CLI command parity (gestura -h) ---
    ("/mcp", "Show MCP server status"),
    ("/mcp list", "List configured MCP servers"),
    ("/mcp status", "Show MCP protocol status"),
    ("/mcp tools", "List tools from connected MCP servers"),
    ("/mcp get <name>", "Show details for an MCP server"),
    ("/mcp enable <name>", "Enable an MCP server"),
    ("/mcp disable <name>", "Disable an MCP server"),
    (
        "/mcp add <name> <cmd_or_url>",
        "Add a new MCP server (options: --transport, --scope)",
    ),
    ("/mcp remove <name>", "Remove an MCP server"),
    ("/mcp connect <name>", "Connect to an MCP server"),
    ("/mcp disconnect <name>", "Disconnect from an MCP server"),
    ("/config", "List configuration settings"),
    ("/config list", "List all configuration settings"),
    ("/config get <key>", "Get a configuration value"),
    ("/config set <key> <value>", "Set a configuration value"),
    ("/config path", "Show configuration file path"),
    ("/config reset", "Reset configuration to defaults"),
    ("/a2a", "Show A2A protocol status"),
    ("/a2a profiles", "List registered agent profiles"),
    ("/a2a agents", "List known remote agents"),
    ("/knowledge", "List knowledge items"),
    ("/knowledge search <query>", "Search knowledge items"),
    ("/knowledge categories", "List knowledge categories"),
    ("/knowledge status", "Show knowledge system status"),
    ("/agent", "Show agent status"),
    ("/agent list", "List available agents"),
    ("/agent config <name>", "Show agent configuration"),
    ("/device", "List audio input devices"),
    ("/device scan", "Scan for audio devices"),
    ("/health", "Show system health diagnostics"),
    ("/privacy", "Show data retention policy"),
    ("/privacy export", "Export all user data (GDPR)"),
    ("/memory", "List memory bank entries"),
    ("/memory save", "Save conversation to memory bank"),
    ("/memory clear", "Clear all memory bank entries"),
    ("/summarize", "Summarize conversation history"),
    (
        "/listen",
        "Toggle listening mode (Enter on empty prompt to record)",
    ),
    ("/voice", "Record one voice message"),
    ("/exec <prompt>", "Execute a single prompt inline"),
    ("/continue", "Resume a paused session"),
];

/// TUI application state
pub struct TuiApp {
    /// Current input buffer
    pub input: String,
    /// Cursor position within input
    pub cursor_pos: usize,
    /// Agent messages
    pub messages: Vec<TuiMessage>,
    /// Message list state for scrolling
    pub message_list_state: ListState,
    /// Whether user has manually scrolled (disables auto-scroll)
    pub user_scrolled: bool,
    /// Agent session for persistence
    pub session: AgentSession,
    /// Application configuration
    pub config: AppConfig,
    /// Optional system prompt
    pub system_prompt: Option<String>,
    /// Current TUI mode
    pub mode: TuiMode,
    /// Whether we're waiting for a response
    pub is_loading: bool,
    /// Whether listening mode is enabled.
    ///
    /// When enabled, pressing Enter on an empty input triggers voice capture.
    pub listening_mode: bool,
    /// Whether a voice capture workflow is currently in progress (recording/transcribing).
    pub voice_capture_in_progress: bool,
    /// Frame counter for the animated thinking spinner (incremented each render tick while loading).
    pub loading_tick: u64,
    /// Current status message
    pub status: String,
    /// Error message (if any)
    pub error: Option<String>,
    /// Timestamp when the error was set (for auto-dismiss)
    pub error_timestamp: Option<std::time::Instant>,
    /// Count of visible error messages in session (limit 2)
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
    /// Command palette: list state for scrollable viewport tracking
    pub command_list_state: ListState,
    /// Command history for up/down navigation
    pub command_history: Vec<String>,
    /// Current position in command history
    pub command_history_pos: Option<usize>,
    /// Pending confirmation action
    pub pending_confirm: Option<ConfirmAction>,
    /// Mode to restore after dismissing a confirm dialog.
    pub confirm_return_mode: Option<TuiMode>,
    /// Pending tool confirmation request (scoped allow/deny decision).
    pub pending_tool_confirmation: Option<PendingToolConfirmation>,
    /// Layout areas for mouse click detection (set during render)
    pub layout_areas: LayoutAreas,

    /// Cached per-message copy-button hit targets (recomputed each render pass).
    ///
    /// These are rendered as overlays so they are never included in `rendered_line_texts`.
    pub assistant_copy_buttons: Vec<CopyButtonHit>,

    /// Which assistant message's copy button is currently hovered by the mouse.
    ///
    /// This drives the hover-highlight styling for the per-message "copy" control.
    pub hovered_copy_button: Option<usize>,

    /// Which assistant message's copy button is currently pressed (mouse-down).
    ///
    /// This drives the pressed styling (dim while pressed) and ensures we only trigger the copy
    /// action on mouse-up if the release occurs over the same button.
    pub pressed_copy_button: Option<usize>,
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

    /// Rendered capabilities text (populated when `/capabilities` is invoked).
    pub capabilities_text: String,
    /// Scroll offset for the capabilities overlay.
    pub capabilities_scroll: usize,

    /// Agent activity state (tool-call transcript, separate from agent transcript).
    pub activity_state: ActivityState,
    /// Interactive tools list state (Tools tab).
    pub tools_state: ToolsState,
    /// Interactive MCP server browser state (overlay).
    pub mcp_browser_state: McpBrowserState,
    /// Interactive knowledge browser state (overlay).
    pub knowledge_browser_state: KnowledgeBrowserState,
    /// Hooks browser state (overlay).
    pub hooks_browser_state: GenericBrowserState,
    /// Hooks browser cached data.
    pub hooks_browser_data: HooksBrowserData,
    /// Agent browser state (overlay).
    pub agent_browser_state: GenericBrowserState,
    /// Agent browser cached data.
    pub agent_browser_data: AgentBrowserData,
    /// Memory browser state (overlay).
    pub memory_browser_state: GenericBrowserState,
    /// Memory browser cached entries.
    pub memory_browser_entries: Vec<MemoryBrowserEntry>,
    /// Devices browser state (overlay).
    pub devices_browser_state: GenericBrowserState,
    /// Devices browser cached entries.
    pub devices_browser_entries: Vec<DeviceBrowserEntry>,
    /// Permissions browser state (overlay).
    pub permissions_browser_state: GenericBrowserState,
    /// Permissions browser cached entries.
    pub permissions_browser_entries: Vec<PermissionBrowserEntry>,
    /// Sessions browser state (overlay).
    pub sessions_browser_state: GenericBrowserState,
    /// Sessions browser cached entries.
    pub sessions_browser_entries: Vec<SessionBrowserEntry>,
    /// Tasks browser state (overlay).
    pub tasks_browser_state: GenericBrowserState,
    /// Tasks browser cached entries.
    pub tasks_browser_entries: Vec<TaskBrowserEntry>,
    /// Themes browser state (overlay).
    pub themes_browser_state: GenericBrowserState,
    /// Themes browser cached names.
    pub themes_browser_names: Vec<String>,
    /// Model picker overlay state.
    pub model_picker_state: ModelPickerState,
    /// Cached dynamic model lists per provider (populated on first `/model` open).
    pub cached_model_lists: HashMap<String, Vec<gestura_core::ModelInfo>>,

    // ========== Scrolling & Selection ==========
    /// Total number of rendered lines in the last frame (set by `render_messages`).
    ///
    /// This count reflects the flattened line count (one `ListItem` per line) rather
    /// than the message count, so scroll bounds match the visual list.
    pub rendered_line_count: usize,
    /// Mapping from rendered-line index → source message index (set by `render_messages`).
    pub line_to_message_map: Vec<usize>,
    /// Plain-text content of each rendered line (set by `render_messages`).
    ///
    /// Used by the in-app selection/copy feature so we can copy the exact visible text of
    /// selected lines rather than whole message content.
    pub rendered_line_texts: Vec<String>,
    /// Start of the current mouse-drag text selection (rendered-line index).
    pub selection_anchor: Option<usize>,
    /// End of the current mouse-drag text selection (rendered-line index).
    pub selection_end: Option<usize>,

    /// When true, the next render pass should snap the transcript scroll position to the bottom.
    ///
    /// This is used to implement reliable follow-tail scrolling during streaming updates where
    /// new content changes the rendered line count.
    pub pending_scroll_to_bottom: bool,
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

/// State for the interactive tools list.
#[derive(Debug, Clone, Default)]
pub struct ToolsState {
    /// Currently selected tool index in the list.
    pub selected_index: usize,
    /// Whether we are viewing the detail pane for the selected tool.
    pub detail_mode: bool,
    /// Selection state used by ratatui's stateful list widget.
    pub list_state: ListState,
}

impl ToolsState {
    /// Move the selection up.
    pub fn select_prev(&mut self, tool_count: usize) {
        if tool_count == 0 {
            return;
        }
        self.selected_index = if self.selected_index == 0 {
            tool_count - 1
        } else {
            self.selected_index - 1
        };
        self.list_state.select(Some(self.selected_index));
    }

    /// Move the selection down.
    pub fn select_next(&mut self, tool_count: usize) {
        if tool_count == 0 {
            return;
        }
        self.selected_index = (self.selected_index + 1) % tool_count;
        self.list_state.select(Some(self.selected_index));
    }

    /// Select a specific index (clamped to bounds).
    pub fn select(&mut self, index: usize, tool_count: usize) {
        if tool_count == 0 {
            return;
        }
        self.selected_index = index.min(tool_count - 1);
        self.list_state.select(Some(self.selected_index));
    }
}

/// Cached snapshot of an MCP server for the interactive browser.
#[derive(Debug, Clone)]
pub struct McpBrowserEntry {
    /// The server configuration entry.
    pub entry: gestura_core::config::McpServerEntry,
    /// Whether this server is currently connected (resolved at open time).
    pub connected: bool,
}

/// State for the interactive MCP server browser overlay.
#[derive(Debug, Clone, Default)]
pub struct McpBrowserState {
    /// Cached list of MCP servers (populated when overlay opens).
    pub servers: Vec<McpBrowserEntry>,
    /// Currently selected index in the list.
    pub selected_index: usize,
    /// Whether we are viewing the detail pane for the selected server.
    pub detail_mode: bool,
    /// Selection state used by ratatui's stateful list widget.
    pub list_state: ListState,
}

impl McpBrowserState {
    /// Move the selection up.
    pub fn select_prev(&mut self) {
        let count = self.servers.len();
        if count == 0 {
            return;
        }
        self.selected_index = if self.selected_index == 0 {
            count - 1
        } else {
            self.selected_index - 1
        };
        self.list_state.select(Some(self.selected_index));
    }

    /// Move the selection down.
    pub fn select_next(&mut self) {
        let count = self.servers.len();
        if count == 0 {
            return;
        }
        self.selected_index = (self.selected_index + 1) % count;
        self.list_state.select(Some(self.selected_index));
    }
}

/// State for the interactive knowledge browser overlay.
#[derive(Debug, Clone, Default)]
pub struct KnowledgeBrowserState {
    /// Cached list of knowledge items (populated when overlay opens).
    pub items: Vec<gestura_core::knowledge::KnowledgeItem>,
    /// Currently selected index in the list.
    pub selected_index: usize,
    /// Whether we are viewing the detail pane for the selected item.
    pub detail_mode: bool,
    /// Selection state used by ratatui's stateful list widget.
    pub list_state: ListState,
}

impl KnowledgeBrowserState {
    /// Move the selection up.
    pub fn select_prev(&mut self) {
        let count = self.items.len();
        if count == 0 {
            return;
        }
        self.selected_index = if self.selected_index == 0 {
            count - 1
        } else {
            self.selected_index - 1
        };
        self.list_state.select(Some(self.selected_index));
    }

    /// Move the selection down.
    pub fn select_next(&mut self) {
        let count = self.items.len();
        if count == 0 {
            return;
        }
        self.selected_index = (self.selected_index + 1) % count;
        self.list_state.select(Some(self.selected_index));
    }
}

/// Generic browser state with a selectable list and detail mode.
///
/// Used by Hooks, Agent, Devices, Permissions, Sessions, Tasks, and Themes browsers.
/// The `items` field is intentionally left out — each browser stores its domain data
/// alongside this state.
#[derive(Debug, Clone, Default)]
pub struct GenericBrowserState {
    /// Currently selected index in the list.
    pub selected_index: usize,
    /// Whether we are viewing the detail pane for the selected item.
    pub detail_mode: bool,
    /// Selection state used by ratatui's stateful list widget.
    pub list_state: ListState,
    /// Total number of items (set when opening the browser).
    pub item_count: usize,
}

impl GenericBrowserState {
    /// Move the selection up.
    pub fn select_prev(&mut self) {
        if self.item_count == 0 {
            return;
        }
        self.selected_index = if self.selected_index == 0 {
            self.item_count - 1
        } else {
            self.selected_index - 1
        };
        self.list_state.select(Some(self.selected_index));
    }

    /// Move the selection down.
    pub fn select_next(&mut self) {
        if self.item_count == 0 {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.item_count;
        self.list_state.select(Some(self.selected_index));
    }

    /// Reset the state for a new list of items.
    pub fn reset(&mut self, count: usize) {
        self.item_count = count;
        self.selected_index = 0;
        self.detail_mode = false;
        if count > 0 {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }
}

/// Cached hooks data for the hooks browser overlay.
#[derive(Debug, Clone, Default)]
pub struct HooksBrowserData {
    /// Whether hooks are globally enabled.
    pub enabled: bool,
    /// Timeout in ms.
    pub timeout_ms: u64,
    /// Max output bytes.
    pub max_output_bytes: usize,
    /// Allowed programs.
    pub allowed_programs: Vec<String>,
    /// Configured hooks (name, event, program, args).
    pub hooks: Vec<(String, String, String, String)>,
}

/// Cached agent data for the agent browser overlay.
#[derive(Debug, Clone, Default)]
pub struct AgentBrowserData {
    /// Display rows for the agent dashboard: (label, value).
    pub rows: Vec<(String, String)>,
}

/// Cached memory entries for the memory browser overlay.
#[derive(Debug, Clone)]
pub struct MemoryBrowserEntry {
    /// Timestamp string.
    pub timestamp: String,
    /// Optional category.
    pub category: Option<String>,
    /// Summary.
    pub summary: String,
    /// Full content.
    pub content: String,
    /// Session ID.
    pub session_id: String,
    /// Entry file path (stored as a workspace-relative path string when possible).
    ///
    /// This is used for safe delete operations via `/memory delete`.
    pub file_path: Option<String>,
}

/// Cached device data for the device browser overlay.
#[derive(Debug, Clone)]
pub struct DeviceBrowserEntry {
    /// Device name.
    pub name: String,
    /// Whether this is the default device.
    pub is_default: bool,
}

/// Cached permission data for the permissions browser overlay.
#[derive(Debug, Clone)]
pub struct PermissionBrowserEntry {
    /// Tool name.
    pub tool: String,
    /// Action name.
    pub action: String,
    /// Scope description.
    pub scope: String,
    /// Expiry description.
    pub expires: String,
}

/// Cached session info for the sessions browser overlay.
#[derive(Debug, Clone)]
pub struct SessionBrowserEntry {
    /// Session ID.
    pub id: String,
    /// Model used.
    pub model: String,
    /// Message count.
    pub message_count: usize,
    /// Created timestamp string.
    pub created: String,
    /// Last active timestamp string.
    pub last_active: String,
    /// Whether this is the current session.
    pub is_current: bool,
}

/// Cached task data for the tasks browser overlay.
#[derive(Debug, Clone)]
pub struct TaskBrowserEntry {
    /// Task ID.
    pub id: String,
    /// Task name.
    pub name: String,
    /// Task description.
    pub description: String,
    /// Status string.
    pub status: String,
    /// Status icon ([ ], [/], [x], [-]).
    pub status_icon: String,
    /// Parent task ID (for subtasks).
    pub parent_id: Option<String>,
    /// Source (User, Agent, Orchestrator).
    pub source: String,
    /// Created timestamp string.
    pub created: String,
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
    pub fn new(session: AgentSession, config: AppConfig, system_prompt: Option<String>) -> Self {
        let messages: Vec<TuiMessage> = session
            .state
            .messages
            .iter()
            .map(TuiMessage::from)
            .collect();

        let has_initial_messages = !messages.is_empty();

        let initial_theme = Theme::from_config(&config);

        let mut message_list_state = ListState::default();
        // Select the last message if any exist (best-effort; we will snap to the true bottom
        // after first render when wrapped line counts are known).
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
            listening_mode: false,
            voice_capture_in_progress: false,
            loading_tick: 0,
            status: "Ready".to_string(),
            error: None,
            error_timestamp: None,
            error_message_count: 0,
            skip_save_on_exit: false,
            active_tab: 0,
            tabs: vec!["Agent", "Workflows", "Tools", "Settings", "Help"],
            workflows: Vec::new(),
            settings_state: SettingsState::default(),
            command_suggestions: Vec::new(),
            command_selection: 0,
            command_list_state: ListState::default(),
            command_history: Vec::new(),
            command_history_pos: None,
            pending_confirm: None,
            confirm_return_mode: None,
            pending_tool_confirmation: None,
            layout_areas: LayoutAreas::default(),
            assistant_copy_buttons: Vec::new(),
            hovered_copy_button: None,
            pressed_copy_button: None,
            theme: initial_theme,
            search_query: String::new(),
            search_matches: Vec::new(),
            current_match_idx: 0,
            search_filter_mode: false,
            session_input_tokens: 0,
            session_output_tokens: 0,
            session_cost_usd: 0.0,
            original_prompt: None,

            capabilities_text: String::new(),
            capabilities_scroll: 0,

            activity_state: ActivityState::default(),
            tools_state: ToolsState::default(),
            mcp_browser_state: McpBrowserState::default(),
            knowledge_browser_state: KnowledgeBrowserState::default(),
            hooks_browser_state: GenericBrowserState::default(),
            hooks_browser_data: HooksBrowserData::default(),
            agent_browser_state: GenericBrowserState::default(),
            agent_browser_data: AgentBrowserData::default(),
            memory_browser_state: GenericBrowserState::default(),
            memory_browser_entries: Vec::new(),
            devices_browser_state: GenericBrowserState::default(),
            devices_browser_entries: Vec::new(),
            permissions_browser_state: GenericBrowserState::default(),
            permissions_browser_entries: Vec::new(),
            sessions_browser_state: GenericBrowserState::default(),
            sessions_browser_entries: Vec::new(),
            tasks_browser_state: GenericBrowserState::default(),
            tasks_browser_entries: Vec::new(),
            themes_browser_state: GenericBrowserState::default(),
            themes_browser_names: Vec::new(),
            model_picker_state: ModelPickerState::default(),
            cached_model_lists: HashMap::new(),

            rendered_line_count: 0,
            line_to_message_map: Vec::new(),
            rendered_line_texts: Vec::new(),
            selection_anchor: None,
            selection_end: None,

            pending_scroll_to_bottom: has_initial_messages,
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

    /// Set the theme by name, respecting the current `theme_mode`.
    ///
    /// The Gestura theme additionally forwards the configured accent color.
    pub fn set_theme(&mut self, name: &str) {
        let normalized = name.trim().to_ascii_lowercase();
        let mode = &self.config.ui.theme_mode;
        if normalized == "gestura" || normalized == "brand" {
            self.theme = Theme::gestura_for(mode, self.config.ui.accent.as_deref());
        } else {
            self.theme = Theme::by_name_for(name, mode);
        }
        self.set_status(format!("Theme changed to: {}", self.theme.name));
    }

    /// Cycle to the next theme (stable key-based).
    pub fn cycle_theme(&mut self) {
        let themes = Theme::available_themes();
        let current_key = self.theme.theme_key();
        let current_idx = themes.iter().position(|&t| t == current_key).unwrap_or(0);
        let next_idx = (current_idx + 1) % themes.len();
        self.set_theme(themes[next_idx]);
    }

    /// Show a confirmation dialog
    pub fn show_confirm(&mut self, action: ConfirmAction) {
        // Remember where we came from so overlays can return to their previous mode.
        if self.mode != TuiMode::Confirm {
            self.confirm_return_mode = Some(self.mode);
        }
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
        self.mode = self.confirm_return_mode.take().unwrap_or(TuiMode::Insert);
    }

    /// Get the pending confirmation action and clear it
    pub fn take_confirm(&mut self) -> Option<ConfirmAction> {
        self.mode = self.confirm_return_mode.take().unwrap_or(TuiMode::Insert);
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
        self.sync_command_list_state();
    }

    /// Select next command suggestion
    pub fn next_command_suggestion(&mut self) {
        if !self.command_suggestions.is_empty() {
            self.command_selection = (self.command_selection + 1) % self.command_suggestions.len();
            self.sync_command_list_state();
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
            self.sync_command_list_state();
        }
    }

    /// Sync the `command_list_state` with the current `command_selection` so
    /// ratatui's stateful `List` widget scrolls the viewport to keep the
    /// selected item visible.
    fn sync_command_list_state(&mut self) {
        if self.command_suggestions.is_empty() {
            self.command_list_state.select(None);
        } else {
            self.command_list_state.select(Some(self.command_selection));
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

    /// Add a message to the agent transcript
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
                self.session.state.messages.push(AgentMessage {
                    role: other.to_string(),
                    content: content.to_string(),
                    tool_call_id: None,
                    thinking: None,
                    timestamp: Utc::now(),
                    source: MessageSource::System,
                });
            }
        }

        // A new message (especially from the user) is a strong signal that the
        // conversation should follow-tail.  Unconditionally re-enable auto-scroll
        // so the view snaps to the bottom on the next render.
        if role == "user" {
            self.user_scrolled = false;
        }
        self.pending_scroll_to_bottom = true;
    }

    /// Add a user message with an explicit source (text vs voice).
    ///
    /// This keeps the UI transcript and persisted `AgentSession` in sync.
    pub fn add_user_message_with_source(
        &mut self,
        content: &str,
        source: gestura_core::agent_sessions::MessageSource,
    ) {
        self.messages.push(TuiMessage {
            role: "user".to_string(),
            content: content.to_string(),
            thinking: None,
            is_streaming: false,
            is_error: false,
        });

        self.session.add_user_message(content, source);

        // A user message is a strong signal that the view should follow-tail.
        self.user_scrolled = false;
        self.pending_scroll_to_bottom = true;
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
        // A streaming placeholder always follows a user message, so auto-scroll
        // should already be re-enabled.  Set the flag unconditionally to be safe.
        self.pending_scroll_to_bottom = true;
    }

    /// Update the last message (for streaming)
    pub fn update_last_message(&mut self, content: &str) {
        if let Some(last) = self.messages.last_mut() {
            last.content = content.to_string();
        }

        // Streaming updates can change the rendered line count due to wrapping. Defer the actual
        // scroll-to-bottom until after the next render computes the updated line count.
        if !self.user_scrolled {
            self.pending_scroll_to_bottom = true;
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
    /// This does not write into the persisted `AgentSession` history.
    /// Limited to 2 visible error messages in the session to avoid clutter.
    /// Critical errors (connection failures, API quota exceeded) are shown as agent messages.
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

        // Error messages are important — always scroll to make them visible.
        self.pending_scroll_to_bottom = true;
    }

    /// Update the last message thinking content
    pub fn update_last_message_thinking(&mut self, thinking: &str) {
        if let Some(last) = self.messages.last_mut() {
            last.thinking = Some(thinking.to_string());
        }

        if !self.user_scrolled {
            self.pending_scroll_to_bottom = true;
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
                    self.session.state.messages.push(AgentMessage {
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

    /// Clamp the cursor to a valid UTF-8 character boundary within the current input.
    fn clamp_cursor_to_char_boundary(&mut self) {
        self.cursor_pos = self.cursor_pos.min(self.input.len());
        while self.cursor_pos > 0 && !self.input.is_char_boundary(self.cursor_pos) {
            self.cursor_pos = self.cursor_pos.saturating_sub(1);
        }
    }

    /// Return the previous UTF-8 character boundary at or before `idx`.
    fn prev_char_boundary(&self, idx: usize) -> usize {
        if idx == 0 {
            return 0;
        }

        let mut i = idx.min(self.input.len());
        // If `idx` is a boundary, moving left should go to the previous char.
        i = i.saturating_sub(1);
        while i > 0 && !self.input.is_char_boundary(i) {
            i = i.saturating_sub(1);
        }
        i
    }

    /// Return the next UTF-8 character boundary strictly after `idx`.
    fn next_char_boundary(&self, idx: usize) -> usize {
        let len = self.input.len();
        if idx >= len {
            return len;
        }

        let mut i = (idx + 1).min(len);
        while i < len && !self.input.is_char_boundary(i) {
            i += 1;
        }
        i
    }

    /// Insert a character at the cursor position.
    ///
    /// `cursor_pos` is treated as a **byte offset** into the UTF-8 string.
    pub fn insert_char(&mut self, c: char) {
        self.clamp_cursor_to_char_boundary();
        self.input.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
    }

    /// Insert a string at the cursor position (used for bracketed paste).
    ///
    /// `cursor_pos` is treated as a **byte offset** into the UTF-8 string.
    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.clamp_cursor_to_char_boundary();
        self.input.insert_str(self.cursor_pos, s);
        self.cursor_pos = self.cursor_pos.saturating_add(s.len());
    }

    /// Delete the character before cursor
    pub fn delete_char_before(&mut self) {
        self.clamp_cursor_to_char_boundary();
        if self.cursor_pos == 0 {
            return;
        }

        let prev = self.prev_char_boundary(self.cursor_pos);
        self.input.remove(prev);
        self.cursor_pos = prev;
    }

    /// Delete the character after cursor
    pub fn delete_char_after(&mut self) {
        self.clamp_cursor_to_char_boundary();
        if self.cursor_pos < self.input.len() {
            self.input.remove(self.cursor_pos);
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        self.clamp_cursor_to_char_boundary();
        self.cursor_pos = self.prev_char_boundary(self.cursor_pos);
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        self.clamp_cursor_to_char_boundary();
        self.cursor_pos = self.next_char_boundary(self.cursor_pos);
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
        self.clamp_cursor_to_char_boundary();
        let len = self.input.len();
        if self.cursor_pos >= len {
            return;
        }

        let mut i = self.cursor_pos;

        // Skip current word (non-whitespace)
        while i < len {
            let ch = self.input[i..].chars().next().unwrap_or(' ');
            if ch.is_whitespace() {
                break;
            }
            i += ch.len_utf8();
        }

        // Skip whitespace
        while i < len {
            let ch = self.input[i..].chars().next().unwrap_or(' ');
            if !ch.is_whitespace() {
                break;
            }
            i += ch.len_utf8();
        }

        self.cursor_pos = i;
    }

    /// Move cursor to previous word (vim 'b' motion)
    pub fn cursor_word_backward(&mut self) {
        self.clamp_cursor_to_char_boundary();
        if self.cursor_pos == 0 {
            return;
        }

        let mut i = self.prev_char_boundary(self.cursor_pos);

        // Skip whitespace
        while i > 0 {
            let ch = self.input[i..].chars().next().unwrap_or(' ');
            if !ch.is_whitespace() {
                break;
            }
            i = self.prev_char_boundary(i);
        }

        // Skip to start of word
        while i > 0 {
            let prev = self.prev_char_boundary(i);
            let ch = self.input[prev..].chars().next().unwrap_or(' ');
            if ch.is_whitespace() {
                break;
            }
            i = prev;
        }

        self.cursor_pos = i;
    }

    /// Delete word before cursor (vim 'db' or Ctrl+W)
    pub fn delete_word_before(&mut self) {
        self.clamp_cursor_to_char_boundary();
        if self.cursor_pos == 0 {
            return;
        }

        let original_pos = self.cursor_pos;
        let mut i = self.cursor_pos;

        // Skip whitespace
        while i > 0 {
            let prev = self.prev_char_boundary(i);
            let ch = self.input[prev..].chars().next().unwrap_or(' ');
            if !ch.is_whitespace() {
                break;
            }
            i = prev;
        }

        // Skip to start of word
        while i > 0 {
            let prev = self.prev_char_boundary(i);
            let ch = self.input[prev..].chars().next().unwrap_or(' ');
            if ch.is_whitespace() {
                break;
            }
            i = prev;
        }

        self.input.replace_range(i..original_pos, "");
        self.cursor_pos = i;
    }

    /// Delete to end of line (vim 'D' or Ctrl+K)
    pub fn delete_to_end(&mut self) {
        self.clamp_cursor_to_char_boundary();
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
    use crate::commands::agent::new_cli_session;

    /// Helper to create a test app with a mock session.
    ///
    /// Uses `theme_mode = "dark"` for deterministic theme resolution (avoids
    /// OS-dependent system detection in tests).
    fn create_test_app() -> TuiApp {
        let session = new_cli_session(None).unwrap();
        let mut config = AppConfig::default();
        config.ui.theme_mode = "dark".to_string();
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

        // Default theme is "Gestura" (GUI-matched)
        let initial_theme = app.theme.name;
        assert_eq!(initial_theme, "Gestura");

        // "light" is an explicit-light alias → Catppuccin Latte
        app.set_theme("light");
        assert_eq!(app.theme.name, "Catppuccin Latte");

        app.set_theme("high-contrast");
        assert_eq!(app.theme.name, "High Contrast");

        app.set_theme("dracula");
        assert_eq!(app.theme.name, "Dracula");

        app.set_theme("gestura");
        assert_eq!(app.theme.name, "Gestura");

        // Invalid theme falls back to catppuccin for the current mode (dark)
        app.set_theme("nonexistent");
        assert_eq!(app.theme.name, "Catppuccin Mocha");
    }

    #[test]
    fn test_theme_cycling() {
        let mut app = create_test_app();

        // Default theme is Gestura (key: "gestura", index 3 in available_themes).
        assert_eq!(app.theme.name, "Gestura");
        assert_eq!(app.theme.theme_key(), "gestura");

        // Cycle order: catppuccin(0) → high-contrast(1) → dracula(2) → gestura(3) → pro(4)
        // From gestura(3) → pro(4)
        app.cycle_theme();
        assert_eq!(app.theme.name, "Pro");

        // pro(4) wraps → catppuccin(0) — dark mode → Catppuccin Mocha
        app.cycle_theme();
        assert_eq!(app.theme.name, "Catppuccin Mocha");

        // catppuccin(0) → high-contrast(1)
        app.cycle_theme();
        assert_eq!(app.theme.name, "High Contrast");

        // high-contrast(1) → dracula(2)
        app.cycle_theme();
        assert_eq!(app.theme.name, "Dracula");

        // dracula(2) → gestura(3)
        app.cycle_theme();
        assert_eq!(app.theme.name, "Gestura");

        // gestura(3) → pro(4)
        app.cycle_theme();
        assert_eq!(app.theme.name, "Pro");
    }

    #[test]
    fn test_gestura_theme_respects_config_accent() {
        let session = new_cli_session(None).unwrap();
        let mut config = AppConfig::default();
        config.ui.theme_mode = "dark".to_string();
        config.ui.accent = Some("emerald".to_string());

        let app = TuiApp::new(session, config, None);

        assert_eq!(app.theme.name, "Gestura");
        assert_eq!(app.theme.border_focused, Color::Rgb(52, 211, 153));
        assert_eq!(app.theme.streaming, Color::Rgb(52, 211, 153));
        // User messages use primary text (white) in dark mode.
        assert_eq!(app.theme.user_msg, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn test_is_dark_theme_mode_variants() {
        assert!(!Theme::is_dark_theme_mode("light"));
        assert!(Theme::is_dark_theme_mode("dark"));
        // "system" delegates to OS detection — just verify no panic.
        let _ = Theme::is_dark_theme_mode("system");
        // Unknown values default to dark.
        assert!(Theme::is_dark_theme_mode("bogus"));
        assert!(Theme::is_dark_theme_mode(""));
    }

    #[test]
    fn test_by_name_for_light_and_dark() {
        // Dark mode should give Catppuccin Mocha.
        let dark = Theme::by_name_for("catppuccin", "dark");
        assert_eq!(dark.name, "Catppuccin Mocha");

        // Light mode should give Catppuccin Latte.
        let light = Theme::by_name_for("catppuccin", "light");
        assert_eq!(light.name, "Catppuccin Latte");

        // Other themes keep same name but adapt palette.
        let hc_dark = Theme::by_name_for("high-contrast", "dark");
        let hc_light = Theme::by_name_for("high-contrast", "light");
        assert_eq!(hc_dark.name, "High Contrast");
        assert_eq!(hc_light.name, "High Contrast");
        // Light should have a visibly different background from dark.
        assert_ne!(hc_dark.header_bg, hc_light.header_bg);
    }

    #[test]
    fn test_theme_key_round_trip() {
        // Every theme produced by by_name_for should map back to its stable key.
        for key in Theme::available_themes() {
            let dark = Theme::by_name_for(key, "dark");
            assert_eq!(dark.theme_key(), *key, "dark round-trip failed for {key}");

            let light = Theme::by_name_for(key, "light");
            assert_eq!(light.theme_key(), *key, "light round-trip failed for {key}");
        }
    }

    #[test]
    fn test_light_mode_theme_switching() {
        let session = new_cli_session(None).unwrap();
        let mut config = AppConfig::default();
        config.ui.theme_mode = "light".to_string();
        let mut app = TuiApp::new(session, config, None);

        // Default is Gestura (light variant).
        assert_eq!(app.theme.name, "Gestura");

        app.set_theme("catppuccin");
        assert_eq!(app.theme.name, "Catppuccin Latte");

        app.set_theme("pro");
        assert_eq!(app.theme.name, "Pro");
        // Pro light should have a near-white header.
        assert_eq!(app.theme.header_bg, Color::Rgb(252, 252, 252));
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

        // 5 tabs: Agent, Workflows, Tools, Settings, Help (indices 0-4)
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
