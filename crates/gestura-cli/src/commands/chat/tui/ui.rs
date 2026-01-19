//! UI rendering for the TUI
//!
//! This module contains all rendering functions for the TUI interface,
//! including the main layout, message list, input field, and status bar.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use super::app::{ConfirmAction, Theme, TuiApp, TuiMode};

/// Parsed segment of a message (text or code block)
#[derive(Debug)]
enum MessageSegment {
    Text(String),
    CodeBlock {
        language: Option<String>,
        code: String,
    },
}

/// Parse a message into segments (text and code blocks)
fn parse_message_segments(content: &str) -> Vec<MessageSegment> {
    let mut segments = Vec::new();
    let mut current_text = String::new();
    let mut in_code_block = false;
    let mut code_language: Option<String> = None;
    let mut code_content = String::new();

    for line in content.lines() {
        if line.starts_with("```") {
            if in_code_block {
                // End of code block
                segments.push(MessageSegment::CodeBlock {
                    language: code_language.take(),
                    code: code_content.clone(),
                });
                code_content.clear();
                in_code_block = false;
            } else {
                // Start of code block
                if !current_text.is_empty() {
                    segments.push(MessageSegment::Text(current_text.clone()));
                    current_text.clear();
                }
                // Extract language if specified
                let lang = line.trim_start_matches('`').trim();
                code_language = if lang.is_empty() {
                    None
                } else {
                    Some(lang.to_string())
                };
                in_code_block = true;
            }
        } else if in_code_block {
            if !code_content.is_empty() {
                code_content.push('\n');
            }
            code_content.push_str(line);
        } else {
            if !current_text.is_empty() {
                current_text.push('\n');
            }
            current_text.push_str(line);
        }
    }

    // Handle unclosed code block or remaining text
    if in_code_block && !code_content.is_empty() {
        segments.push(MessageSegment::CodeBlock {
            language: code_language,
            code: code_content,
        });
    } else if !current_text.is_empty() {
        segments.push(MessageSegment::Text(current_text));
    }

    segments
}

/// Apply syntax highlighting using theme colors
fn highlight_code_line_themed(
    line: &str,
    language: Option<&str>,
    theme: &Theme,
) -> Vec<Span<'static>> {
    // Keywords for common languages
    let keywords: &[&str] = match language {
        Some("rust") | Some("rs") => &[
            "fn", "let", "mut", "const", "pub", "struct", "enum", "impl", "trait", "use", "mod",
            "if", "else", "match", "for", "while", "loop", "return", "async", "await", "self",
            "Self", "true", "false", "Some", "None", "Ok", "Err",
        ],
        Some("python") | Some("py") => &[
            "def", "class", "if", "elif", "else", "for", "while", "return", "import", "from", "as",
            "try", "except", "finally", "with", "True", "False", "None", "and", "or", "not", "in",
            "is", "lambda", "yield", "async", "await",
        ],
        Some("javascript") | Some("js") | Some("typescript") | Some("ts") => &[
            "function",
            "const",
            "let",
            "var",
            "if",
            "else",
            "for",
            "while",
            "return",
            "class",
            "extends",
            "import",
            "export",
            "from",
            "async",
            "await",
            "try",
            "catch",
            "finally",
            "true",
            "false",
            "null",
            "undefined",
            "new",
            "this",
        ],
        Some("bash") | Some("sh") | Some("shell") => &[
            "if", "then", "else", "fi", "for", "do", "done", "while", "case", "esac", "function",
            "return", "export", "local", "echo", "exit", "cd", "pwd",
        ],
        _ => &[],
    };

    let mut spans = Vec::new();
    let mut current_word = String::new();
    let mut in_string = false;
    let mut string_char = '"';
    let mut in_comment = false;

    for ch in line.chars() {
        if in_comment {
            current_word.push(ch);
            continue;
        }

        // Check for comment start
        if ch == '#' || (ch == '/' && current_word.ends_with('/')) {
            if !current_word.is_empty() && current_word.ends_with('/') {
                current_word.pop();
                if !current_word.is_empty() {
                    spans.push(Span::styled(
                        current_word.clone(),
                        Style::default().fg(theme.code_fg).bg(theme.code_bg),
                    ));
                }
                current_word = "/".to_string();
            }
            current_word.push(ch);
            in_comment = true;
            continue;
        }

        // Handle strings
        if (ch == '"' || ch == '\'' || ch == '`') && !in_string {
            if !current_word.is_empty() {
                let style = if keywords.contains(&current_word.as_str()) {
                    Style::default()
                        .fg(theme.code_keyword)
                        .bg(theme.code_bg)
                        .add_modifier(Modifier::BOLD)
                } else if current_word.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    Style::default().fg(theme.code_number).bg(theme.code_bg)
                } else {
                    Style::default().fg(theme.code_fg).bg(theme.code_bg)
                };
                spans.push(Span::styled(current_word.clone(), style));
                current_word.clear();
            }
            in_string = true;
            string_char = ch;
            current_word.push(ch);
            continue;
        }

        if in_string {
            current_word.push(ch);
            if ch == string_char {
                spans.push(Span::styled(
                    current_word.clone(),
                    Style::default().fg(theme.code_string).bg(theme.code_bg),
                ));
                current_word.clear();
                in_string = false;
            }
            continue;
        }

        // Word boundaries
        if ch.is_whitespace()
            || ch == '('
            || ch == ')'
            || ch == '{'
            || ch == '}'
            || ch == '['
            || ch == ']'
            || ch == ','
            || ch == ';'
            || ch == ':'
            || ch == '.'
        {
            if !current_word.is_empty() {
                let style = if keywords.contains(&current_word.as_str()) {
                    Style::default()
                        .fg(theme.code_keyword)
                        .bg(theme.code_bg)
                        .add_modifier(Modifier::BOLD)
                } else if current_word.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    Style::default().fg(theme.code_number).bg(theme.code_bg)
                } else if current_word
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false)
                {
                    Style::default().fg(theme.code_function).bg(theme.code_bg)
                } else {
                    Style::default().fg(theme.code_fg).bg(theme.code_bg)
                };
                spans.push(Span::styled(current_word.clone(), style));
                current_word.clear();
            }
            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(theme.code_fg).bg(theme.code_bg),
            ));
        } else {
            current_word.push(ch);
        }
    }

    // Handle remaining content
    if !current_word.is_empty() {
        let style = if in_comment {
            Style::default()
                .fg(theme.code_comment)
                .bg(theme.code_bg)
                .add_modifier(Modifier::ITALIC)
        } else if in_string {
            Style::default().fg(theme.code_string).bg(theme.code_bg)
        } else if keywords.contains(&current_word.as_str()) {
            Style::default()
                .fg(theme.code_keyword)
                .bg(theme.code_bg)
                .add_modifier(Modifier::BOLD)
        } else if current_word.chars().all(|c| c.is_ascii_digit() || c == '.') {
            Style::default().fg(theme.code_number).bg(theme.code_bg)
        } else {
            Style::default().fg(theme.code_fg).bg(theme.code_bg)
        };
        spans.push(Span::styled(current_word, style));
    }

    spans
}

/// Minimum terminal dimensions for usable TUI
const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 12;

/// Compact mode threshold (hide tabs, use abbreviated UI)
const COMPACT_WIDTH: u16 = 60;
const COMPACT_HEIGHT: u16 = 16;

/// Word wrap text to fit within a given width
/// Returns a vector of wrapped lines, preserving original line breaks
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();

    for line in text.lines() {
        if line.is_empty() {
            result.push(String::new());
            continue;
        }

        // If line fits, add it directly
        if line.len() <= max_width {
            result.push(line.to_string());
            continue;
        }

        // Word wrap the line
        let words: Vec<&str> = line.split_whitespace().collect();
        let mut current_line = String::new();

        for word in words {
            // If word itself is longer than max_width, break it
            if word.len() > max_width {
                if !current_line.is_empty() {
                    result.push(current_line);
                    current_line = String::new();
                }
                // Break long word into chunks
                let mut remaining = word;
                while remaining.len() > max_width {
                    let (chunk, rest) = remaining.split_at(max_width.saturating_sub(1));
                    result.push(format!("{}-", chunk));
                    remaining = rest;
                }
                if !remaining.is_empty() {
                    current_line = remaining.to_string();
                }
                continue;
            }

            // Check if word fits on current line
            let new_len = if current_line.is_empty() {
                word.len()
            } else {
                current_line.len() + 1 + word.len() // +1 for space
            };

            if new_len <= max_width {
                if !current_line.is_empty() {
                    current_line.push(' ');
                }
                current_line.push_str(word);
            } else {
                // Start new line
                if !current_line.is_empty() {
                    result.push(current_line);
                }
                current_line = word.to_string();
            }
        }

        // Don't forget the last line
        if !current_line.is_empty() {
            result.push(current_line);
        }
    }

    if result.is_empty() {
        result.push(String::new());
    }

    result
}

/// Render the entire TUI
pub fn render(app: &mut TuiApp, frame: &mut Frame) {
    let area = frame.area();

    // Check for minimum terminal size
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small_message(app, frame, area);
        return;
    }

    // Determine layout mode based on terminal size
    let is_compact = area.width < COMPACT_WIDTH || area.height < COMPACT_HEIGHT;

    // Adaptive layout based on terminal size
    let chunks = if is_compact {
        // Compact layout: smaller header, minimal chrome
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Minimal header (just mode indicator)
                Constraint::Min(4),    // Content area
                Constraint::Length(3), // Input field
                Constraint::Length(1), // Status bar
            ])
            .split(area)
    } else {
        // Standard layout
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header with tabs
                Constraint::Min(8),    // Content area
                Constraint::Length(3), // Input field
                Constraint::Length(1), // Status bar
            ])
            .split(area)
    };

    // Store layout areas for mouse click detection
    app.layout_areas.tabs = Some(chunks[0]);
    app.layout_areas.messages = Some(chunks[1]);
    app.layout_areas.input = Some(chunks[2]);

    if is_compact {
        render_compact_header(app, frame, chunks[0]);
    } else {
        render_header(app, frame, chunks[0]);
    }
    render_content(app, frame, chunks[1]);
    render_input(app, frame, chunks[2]);
    render_status_bar(app, frame, chunks[3]);

    // Render overlays
    if app.mode == TuiMode::Help {
        render_help_overlay(app, frame, area);
    } else if app.mode == TuiMode::Confirm {
        render_confirm_dialog(app, frame, area);
    } else if app.mode == TuiMode::Command && !app.command_suggestions.is_empty() {
        // Render command palette above the input field
        render_command_palette(app, frame, chunks[2]);
    } else if app.mode == TuiMode::Search {
        // Render search bar above the input field
        render_search_bar(app, frame, chunks[2]);
    }
}

/// Render a message when terminal is too small
fn render_too_small_message(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let message = format!(
        "Terminal too small!\n\nCurrent: {}x{}\nMinimum: {}x{}\n\nPlease resize your terminal.",
        area.width, area.height, MIN_WIDTH, MIN_HEIGHT
    );

    let paragraph = Paragraph::new(message)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.error_msg))
                .title(" Gestura "),
        )
        .style(Style::default().fg(app.theme.error_msg))
        .alignment(ratatui::layout::Alignment::Center)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

/// Render compact header for small terminals (single line, no tabs)
fn render_compact_header(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let mode_str = match app.mode {
        TuiMode::Normal => "N",
        TuiMode::Insert => "I",
        TuiMode::Command => ":",
        TuiMode::Help => "?",
        TuiMode::Confirm => "!",
        TuiMode::Search => "/",
    };

    let tab_str = match app.active_tab {
        0 => "Chat",
        1 => "Tools",
        2 => "Settings",
        _ => "?",
    };

    let header_text = format!(
        " [{}] {} │ {} │ {}x{}",
        mode_str,
        tab_str,
        &app.session.id[..8.min(app.session.id.len())],
        area.width,
        area.height
    );

    let paragraph = Paragraph::new(header_text).style(
        Style::default()
            .fg(app.theme.header_fg)
            .bg(app.theme.status_bg),
    );

    frame.render_widget(paragraph, area);
}

/// Render the header with tabs
fn render_header(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let block = Block::default().style(Style::default().bg(app.theme.header_bg));
    frame.render_widget(block, area);

    // Render "Gestura" logo and context
    let logo_style = Style::default()
        .fg(app.theme.header_fg)
        .bg(app.theme.header_bg)
        .add_modifier(Modifier::BOLD);

    let normal_style = Style::default()
        .fg(app.theme.header_fg)
        .bg(app.theme.header_bg);
    let arrow = Span::styled(" › ", normal_style);

    let mut spans = vec![
        Span::styled(" GESTURA ", logo_style),
        arrow.clone(),
        Span::styled(
            format!("{} ", app.session.id.get(0..8).unwrap_or("")),
            normal_style,
        ),
    ];

    // Add Tabs
    for (i, title) in app.tabs.iter().enumerate() {
        spans.push(arrow.clone());
        let style = if i == app.active_tab {
            Style::default()
                .fg(app.theme.tab_active)
                .bg(app.theme.header_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(app.theme.tab_inactive)
                .bg(app.theme.header_bg)
        };
        spans.push(Span::styled(*title, style));
    }

    let p = Paragraph::new(Line::from(spans));
    frame.render_widget(p, area);
}

/// Render the main content area (messages or other tab content)
fn render_content(app: &mut TuiApp, frame: &mut Frame, area: Rect) {
    match app.active_tab {
        0 => render_messages(app, frame, area),
        1 => render_workflows_tab(app, frame, area),
        2 => render_tools_tab(app, frame, area),
        3 => render_settings_tab(app, frame, area),
        4 => render_help_tab(app, frame, area),
        _ => render_messages(app, frame, area),
    }
}

/// Render the message list with syntax highlighting for code blocks
fn render_messages(app: &mut TuiApp, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let search_query = &app.search_query;
    let has_search = !search_query.is_empty();

    // Calculate available width for text wrapping
    // Subtract borders (2) and prefix space (8 for "▶ You: " or "◆ AI: ")
    let wrap_width = area.width.saturating_sub(10) as usize;

    // Filter messages if in filter mode
    let message_indices: Vec<usize> = if app.search_filter_mode && has_search {
        app.search_matches.iter().map(|(idx, _)| *idx).collect()
    } else {
        (0..app.messages.len()).collect()
    };

    let messages: Vec<ListItem> = message_indices
        .iter()
        .flat_map(|&msg_idx| {
            let msg = &app.messages[msg_idx];
            let match_ranges = if has_search {
                app.get_match_ranges(msg_idx)
            } else {
                None
            };

            // Check if this message has search matches for highlighting
            let has_match = has_search && app.message_has_match(msg_idx);

            let (prefix, base_style) = match msg.role.as_str() {
                "user" => {
                    // Add search indicator (🔍) for messages with matches
                    let pfx = if has_match {
                        "🔍▶ You: "
                    } else {
                        "▶ You: "
                    };
                    (pfx, Style::default().fg(theme.user_msg))
                }
                "assistant" => {
                    if msg.is_streaming {
                        (
                            "◆ AI: ",
                            Style::default()
                                .fg(theme.streaming)
                                .add_modifier(Modifier::ITALIC),
                        )
                    } else {
                        let pfx = if has_match {
                            "🔍◆ AI: "
                        } else {
                            "◆ AI: "
                        };
                        (pfx, Style::default().fg(theme.assistant_msg))
                    }
                }
                "system" => {
                    let pfx = if has_match {
                        "🔍⚙ System: "
                    } else {
                        "⚙ System: "
                    };
                    (pfx, Style::default().fg(theme.system_msg))
                }
                _ => ("• ", Style::default()),
            };

            let base_style = if msg.is_error {
                base_style.fg(theme.error_msg).add_modifier(Modifier::BOLD)
            } else {
                base_style
            };

            let content = if msg.is_streaming {
                // Determine streaming state
                if msg.thinking.is_some() {
                    // We are thinking
                    if msg.content.is_empty() {
                        String::new() // Don't show text if only thinking
                    } else {
                        format!("{}▌", msg.content)
                    }
                } else {
                    format!("{}▌", msg.content)
                }
            } else {
                msg.content.clone()
            };

            // Parse message for code blocks
            let segments = parse_message_segments(&content);
            let mut items = Vec::new();

            // Render Thinking if present (with word wrapping)
            if let Some(thinking) = &msg.thinking
                && !thinking.is_empty()
            {
                items.push(ListItem::new(Line::from(Span::styled(
                    "  ┌─ Thinking ",
                    Style::default()
                        .fg(theme.code_comment)
                        .add_modifier(Modifier::ITALIC),
                ))));

                // Wrap thinking text (subtract 4 for "  │ " prefix)
                let thinking_wrap_width = wrap_width.saturating_sub(4);
                let wrapped_thinking = wrap_text(thinking, thinking_wrap_width);
                for line in wrapped_thinking {
                    items.push(ListItem::new(Line::from(Span::styled(
                        format!("  │ {}", line),
                        Style::default()
                            .fg(theme.code_comment)
                            .add_modifier(Modifier::ITALIC),
                    ))));
                }

                items.push(ListItem::new(Line::from(Span::styled(
                    "  └───────────",
                    Style::default()
                        .fg(theme.code_comment)
                        .add_modifier(Modifier::ITALIC),
                ))));
            }
            let mut is_first = true;
            let mut char_offset = 0usize; // Track position in original content

            for segment in segments {
                match segment {
                    MessageSegment::Text(text) => {
                        // Apply word wrapping to the text
                        let wrapped_lines = wrap_text(&text, wrap_width);

                        for wrapped_line in wrapped_lines {
                            let display_prefix = if is_first {
                                is_first = false;
                                prefix
                            } else {
                                "  "
                            };

                            // Build spans with search highlighting
                            let spans = if let Some(ranges) = match_ranges {
                                highlight_search_matches(
                                    display_prefix,
                                    &wrapped_line,
                                    char_offset,
                                    ranges,
                                    base_style,
                                    theme,
                                )
                            } else {
                                vec![Span::styled(
                                    format!("{}{}", display_prefix, wrapped_line),
                                    base_style,
                                )]
                            };

                            items.push(ListItem::new(Line::from(spans)));
                            char_offset += wrapped_line.len() + 1; // +1 for newline
                        }
                    }
                    MessageSegment::CodeBlock { language, code } => {
                        // Add language label if present
                        let lang_label = language.as_deref().unwrap_or("code");
                        let header = format!("  ┌─ {} ─", lang_label);
                        items.push(ListItem::new(Line::from(Span::styled(
                            header,
                            Style::default().fg(theme.code_lang_label),
                        ))));

                        // Add highlighted code lines
                        for code_line in code.lines() {
                            let mut spans = vec![Span::styled(
                                "  │ ".to_string(),
                                Style::default().fg(theme.code_lang_label),
                            )];
                            spans.extend(highlight_code_line_themed(
                                code_line,
                                language.as_deref(),
                                theme,
                            ));
                            items.push(ListItem::new(Line::from(spans)));
                        }

                        // Add closing border
                        items.push(ListItem::new(Line::from(Span::styled(
                            "  └────────",
                            Style::default().fg(theme.code_lang_label),
                        ))));
                        is_first = false;
                        // Update char_offset for code block (including markers)
                        char_offset += code.len() + 10; // Approximate for code block markers
                    }
                }
            }

            if items.is_empty() {
                vec![ListItem::new(Line::from(Span::styled(
                    prefix.to_string(),
                    base_style,
                )))]
            } else {
                items
            }
        })
        .collect();

    let scroll_info = app.scroll_indicator();
    let search_info = if has_search {
        format!(" [{}]", app.search_query)
    } else {
        String::new()
    };
    let title = format!(
        " Messages ({}){} {} ",
        app.messages.len(),
        search_info,
        if app.user_scrolled {
            scroll_info
        } else {
            "".to_string()
        }
    );

    let messages_block = Block::default()
        // No borders for cleaner look
        .borders(Borders::NONE)
        .title(Span::styled(
            title,
            Style::default().fg(app.theme.header_fg),
        ));

    let list = List::new(messages).block(messages_block).highlight_style(
        Style::default()
            .bg(theme.selection_bg)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(list, area, &mut app.message_list_state.clone());
}

/// Highlight search matches in a line of text
fn highlight_search_matches(
    prefix: &str,
    text: &str,
    char_offset: usize,
    ranges: &[std::ops::Range<usize>],
    base_style: Style,
    theme: &super::app::Theme,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    spans.push(Span::styled(prefix.to_string(), base_style));

    let text_start = char_offset;
    let text_end = char_offset + text.len();

    // Find ranges that overlap with this line
    let mut last_end = 0usize;
    for range in ranges {
        // Check if range overlaps with this line
        if range.end <= text_start || range.start >= text_end {
            continue;
        }

        // Calculate overlap within this line
        let overlap_start = range.start.saturating_sub(text_start).min(text.len());
        let overlap_end = (range.end - text_start).min(text.len());

        // Add text before the match
        if overlap_start > last_end {
            spans.push(Span::styled(
                text[last_end..overlap_start].to_string(),
                base_style,
            ));
        }

        // Add highlighted match
        if overlap_end > overlap_start {
            spans.push(Span::styled(
                text[overlap_start..overlap_end].to_string(),
                Style::default()
                    .fg(theme.header_fg)
                    .bg(theme.streaming)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        last_end = overlap_end;
    }

    // Add remaining text after last match
    if last_end < text.len() {
        spans.push(Span::styled(text[last_end..].to_string(), base_style));
    }

    // If no matches were found in this line, just return the whole text
    if spans.len() == 1 {
        spans.push(Span::styled(text.to_string(), base_style));
    }

    spans
}

/// Render the tools tab
fn render_tools_tab(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let tools_text = crate::tool_registry::render_tools_overview();
    let paragraph = Paragraph::new(tools_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.border))
                .title(" Tools "),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// Render the help tab
fn render_help_tab(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let help_text = r#"Keyboard Shortcuts:

Navigation:
  j/↓         Scroll down
  k/↑         Scroll up
  g           Go to top
  G           Go to bottom
  Tab         Next tab
  Shift+Tab   Previous tab
  1-4         Switch to tab

Modes:
  i/a         Enter insert mode
  Esc         Exit to normal mode
  /           Enter command mode

Input:
  Enter       Send message
  Shift+Enter New line
  Ctrl+U      Clear input
  Ctrl+T      Cycle theme

Commands:
  /help       Show help
  /tools      List tools
  /clear      Clear messages
  /quit       Exit application
  /save       Save session
  /theme <n>  Change theme"#;

    let paragraph = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.border))
                .title(" Help "),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// Render the input field
fn render_input(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let mode_indicator = match app.mode {
        TuiMode::Normal => ("NORMAL", app.theme.mode_normal),
        TuiMode::Insert => ("INSERT", app.theme.mode_insert),
        TuiMode::Command => ("COMMAND", app.theme.mode_command),
        TuiMode::Help => ("HELP", app.theme.mode_normal),
        TuiMode::Confirm => ("CONFIRM", app.theme.error_msg),
        TuiMode::Search => ("SEARCH", app.theme.streaming),
    };

    let title = format!(
        " {} │ {} ",
        mode_indicator.0,
        if app.is_loading {
            "Waiting for response..."
        } else {
            "Type a message"
        }
    );

    let border_style = if app.mode == TuiMode::Insert || app.mode == TuiMode::Command {
        Style::default().fg(mode_indicator.1)
    } else {
        Style::default().fg(app.theme.border)
    };

    let input = Paragraph::new(app.input.as_str())
        .block(
            Block::default()
                .borders(Borders::TOP) // Only top border to separate from messages
                .border_style(border_style)
                .title(Span::styled(title, Style::default().fg(mode_indicator.1))),
        )
        .style(Style::default().fg(app.theme.header_fg))
        .wrap(Wrap { trim: false }); // Enable text wrapping for input

    frame.render_widget(input, area);

    // Show cursor in insert/command mode
    // Note: With wrapping, cursor position calculation becomes complex for multi-line input
    // For now, we show the cursor at the end of visible content
    if app.mode == TuiMode::Insert || app.mode == TuiMode::Command {
        let input_width = area.width.saturating_sub(2) as usize; // Account for borders
        let cursor_line = if input_width > 0 {
            app.cursor_pos / input_width
        } else {
            0
        };
        let cursor_col = if input_width > 0 {
            app.cursor_pos % input_width
        } else {
            app.cursor_pos
        };
        frame.set_cursor_position((
            area.x + cursor_col as u16 + 1,
            area.y + 1 + cursor_line as u16,
        ));
    }
}

/// Render the status bar with adaptive compact format for narrow terminals
fn render_status_bar(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let is_compact = area.width < 80;

    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50), // Left status
            Constraint::Percentage(50), // Right stats
        ])
        .split(area);

    // Left: Status message
    let status_style = if app.error.is_some() {
        Style::default()
            .fg(app.theme.error_msg)
            .bg(app.theme.status_bg)
    } else {
        Style::default()
            .fg(app.theme.status_fg)
            .bg(app.theme.status_bg)
    };

    let status_text = if let Some(e) = &app.error {
        if is_compact {
            format!(" ERR: {}", &e[..e.len().min(20)])
        } else {
            format!(" ERROR: {}", e)
        }
    } else if is_compact {
        format!(" {:?}", app.mode)
    } else {
        format!(" {} [{:?}]", app.status, app.mode)
    };

    frame.render_widget(Paragraph::new(status_text).style(status_style), layout[0]);

    // Right: Stats - use compact format for narrow terminals
    let stats_style = Style::default()
        .fg(app.theme.status_fg)
        .bg(app.theme.status_bg);

    let model = app.model_name();

    let stats_text = if is_compact {
        // Compact format: "1.2K|$0.01 | gpt-4"
        let compact_tokens = app.format_token_usage_compact();
        let short_model = if model.len() > 10 {
            &model[..10]
        } else {
            model
        };
        if compact_tokens.is_empty() {
            format!("{} ", short_model)
        } else {
            format!("{} | {} ", compact_tokens, short_model)
        }
    } else {
        // Verbose format: "Tokens: 1.2K | Cost: $0.0100 | Model: gpt-4"
        let token_total = app.session_input_tokens + app.session_output_tokens;
        let cost = app.session_cost_usd;
        format!(
            "Tokens: {} | Cost: ${:.4} | Model: {} ",
            gestura_core::token_tracker::format_token_count(token_total),
            cost,
            model
        )
    };

    frame.render_widget(
        Paragraph::new(stats_text)
            .style(stats_style)
            .alignment(ratatui::layout::Alignment::Right),
        layout[1],
    );
}

/// Render workflows tab
fn render_workflows_tab(app: &TuiApp, frame: &mut Frame, area: Rect) {
    if app.workflows.is_empty() {
        let p = Paragraph::new("\n  No workflows found in .agent/workflows/\n  Create .md files there to define workflows.")
             .style(Style::default().fg(app.theme.code_comment));

        frame.render_widget(p.block(Block::default().borders(Borders::NONE)), area);
        return;
    }

    let items: Vec<ListItem> = app
        .workflows
        .iter()
        .map(|(name, desc)| {
            let content = vec![
                Line::from(vec![
                    Span::styled(
                        format!("  {}  ", name),
                        Style::default()
                            .fg(app.theme.header_fg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(desc, Style::default().fg(app.theme.status_fg)),
                ]),
                Line::from(""), // spacer
            ];
            ListItem::new(content)
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::NONE).title(" Workflows "));

    frame.render_widget(list, area);
}

/// Render the interactive settings tab
fn render_settings_tab(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let mut items = Vec::new();

    let selected_style = Style::default()
        .fg(app.theme.header_fg)
        .bg(app.theme.selection_bg);
    let normal_style = Style::default().fg(app.theme.status_fg);

    // Helper to render a field
    let render_field = |idx: usize, label: &str, value: &str| {
        let is_selected = app.settings_state.selected_field == idx;
        let style = if is_selected {
            selected_style
        } else {
            normal_style
        };
        let prefix = if is_selected { " > " } else { "   " };

        // If editing this field, show edit buffer
        let display_value = if is_selected && app.settings_state.is_editing {
            format!("{}█", app.settings_state.edit_buffer)
        } else {
            value.to_string()
        };

        ListItem::new(Line::from(vec![
            Span::styled(format!("{}{:<15}", prefix, label), style),
            Span::styled(
                display_value,
                if is_selected {
                    style.add_modifier(Modifier::BOLD)
                } else {
                    style
                },
            ),
        ]))
    };

    // 1. Provider
    items.push(render_field(0, "Provider", &app.config.llm.primary));

    // 2. Model
    let model = app.model_name();
    items.push(render_field(1, "Model", model));

    // 3. System Prompt
    let sys_prompt = app.system_prompt.as_deref().unwrap_or("Default");
    let sys_display = if sys_prompt.len() > 40 {
        format!("{}...", &sys_prompt[..37])
    } else {
        sys_prompt.to_string()
    };
    items.push(render_field(2, "System Prompt", &sys_display));

    // 4. Temperature (Placeholder)
    items.push(render_field(3, "Temperature", "0.7"));

    let info_text = if app.settings_state.is_editing {
        "Press Enter to save, Esc to cancel"
    } else {
        "Arrow keys to navigate, Enter to edit, Tab to switch tabs"
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::NONE)
            .title(format!(" Settings ({}) ", info_text)),
    );

    frame.render_widget(list, area);
}

/// Render the help overlay (modal)
fn render_help_overlay(app: &TuiApp, frame: &mut Frame, area: Rect) {
    // Calculate centered popup area
    let popup_width = 60.min(area.width.saturating_sub(4));
    let popup_height = 20.min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let help_text = r#"
  Gestura TUI Help
  ─────────────────────────────────────

  Modes:
    Normal    Navigate and execute commands
    Insert    Type messages
    Command   Enter slash commands
    Search    Find text in messages

  Keys (Normal Mode):
    i/a       Enter insert mode
    j/k       Scroll up/down
    g/G       Top/bottom of messages
    Tab       Switch tabs
    Ctrl+F    Search messages
    n/N       Next/prev search match
    ?         Toggle this help
    q         Quit
    Ctrl+T    Cycle theme

  Keys (Insert Mode):
    Enter     Send message
    Esc       Exit to normal mode
    Ctrl+U    Clear input
    Ctrl+F    Search messages
    Ctrl+T    Cycle theme

  Keys (Search Mode):
    Enter     Confirm search
    Esc       Cancel search
    Tab/↓     Next match
    ↑         Previous match
    Ctrl+G    Toggle filter mode

  Press Esc or ? to close
"#;

    let paragraph = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.border_focused))
                .title(Span::styled(
                    " Help ",
                    Style::default()
                        .fg(app.theme.header_fg)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .style(Style::default().fg(app.theme.header_fg));

    frame.render_widget(paragraph, popup_area);
}

/// Render the command palette popup above the input field
fn render_command_palette(app: &TuiApp, frame: &mut Frame, input_area: Rect) {
    let suggestions = &app.command_suggestions;
    if suggestions.is_empty() {
        return;
    }

    // Calculate popup size and position (above input field)
    let height = (suggestions.len() as u16 + 2).min(10); // +2 for borders, max 10 lines
    let width = input_area.width.saturating_sub(4).min(60);
    let x = input_area.x + 2;
    let y = input_area.y.saturating_sub(height);

    let popup_area = Rect::new(x, y, width, height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    // Build list items
    let items: Vec<ListItem> = suggestions
        .iter()
        .enumerate()
        .map(|(i, (cmd, desc))| {
            let style = if i == app.command_selection {
                Style::default()
                    .fg(app.theme.tab_active)
                    .add_modifier(Modifier::BOLD)
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.header_fg)
            };
            let content = format!("{:<15} {}", cmd, desc);
            ListItem::new(Line::from(Span::styled(content, style)))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(app.theme.mode_command))
            .title(Span::styled(
                " Commands (Tab to complete, ↑↓ to navigate) ",
                Style::default().fg(app.theme.mode_command),
            )),
    );

    frame.render_widget(list, popup_area);
}

/// Render a confirmation dialog
fn render_confirm_dialog(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let (title, message) = match &app.pending_confirm {
        Some(ConfirmAction::QuitWithoutSave) => (
            " Quit Without Saving? ",
            "You have unsaved changes. Are you sure you want to quit?\n\n  [Y] Yes, quit    [N] No, cancel",
        ),
        Some(ConfirmAction::ClearMessages) => (
            " Clear Messages? ",
            "This will clear all messages in the current session.\n\n  [Y] Yes, clear    [N] No, cancel",
        ),
        Some(ConfirmAction::NewSession) => (
            " Start New Session? ",
            "This will save the current session and start a new one.\n\n  [Y] Yes, continue    [N] No, cancel",
        ),
        None => return,
    };

    // Center the popup
    let popup_width = 50;
    let popup_height = 7;
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let paragraph = Paragraph::new(message)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.error_msg))
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(app.theme.error_msg)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .style(Style::default().fg(app.theme.header_fg))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, popup_area);
}

/// Render the search bar overlay
fn render_search_bar(app: &TuiApp, frame: &mut Frame, input_area: Rect) {
    // Position search bar above the input area
    let search_height = 3;
    let search_area = Rect::new(
        input_area.x,
        input_area.y.saturating_sub(search_height),
        input_area.width,
        search_height,
    );

    // Clear the area
    frame.render_widget(Clear, search_area);

    // Build search prompt with match count
    let match_info = if app.search_matches.is_empty() {
        if app.search_query.is_empty() {
            String::new()
        } else {
            " (no matches)".to_string()
        }
    } else {
        format!(
            " ({}/{})",
            app.current_match_idx + 1,
            app.search_matches.len()
        )
    };

    let title = format!(" Search{} ", match_info);

    // Create search input display
    let search_text = format!("{}_", app.search_query);

    let paragraph = Paragraph::new(search_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.border_focused))
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(app.theme.streaming)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .style(Style::default().fg(app.theme.header_fg));

    frame.render_widget(paragraph, search_area);
}
