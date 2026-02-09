//! UI rendering for the TUI
//!
//! This module contains all rendering functions for the TUI interface,
//! including the main layout, message list, input field, and status bar.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use super::app::{ConfirmAction, Theme, TuiApp, TuiMode};
use super::markdown;
use super::widgets::composer;

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

/// Word-wrap already-styled spans to fit within `max_width`.
///
/// This uses the same simple whitespace-collapsing approach as [`wrap_text`]
/// (i.e., it wraps on `split_whitespace()` boundaries).
fn wrap_spans(spans: &[Span<'static>], max_width: usize) -> Vec<Vec<Span<'static>>> {
    if max_width == 0 {
        return vec![spans.to_vec()];
    }

    #[derive(Clone, Debug)]
    struct Token {
        text: String,
        style: Style,
    }

    let mut tokens: Vec<Token> = Vec::new();
    for span in spans {
        let style = span.style;
        for w in span.content.as_ref().split_whitespace() {
            tokens.push(Token {
                text: w.to_string(),
                style,
            });
        }
    }

    // Preserve blank lines.
    if tokens.is_empty() {
        return vec![Vec::new()];
    }

    let mut result: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut current_len = 0usize;
    let mut last_style = Style::default();

    for token in tokens {
        // If token itself is longer than max_width, break it (best effort).
        if token.text.len() > max_width {
            if !current.is_empty() {
                result.push(current);
                current = Vec::new();
                current_len = 0;
            }

            // If the available width is too small to safely hyphenate, just
            // emit the long token as-is to avoid infinite loops.
            if max_width <= 1 {
                result.push(vec![Span::styled(token.text, token.style)]);
                continue;
            }

            let mut remaining = token.text.as_str();
            while remaining.len() > max_width {
                let take = max_width.saturating_sub(1).max(1);
                let (chunk, rest) = remaining.split_at(take);
                result.push(vec![Span::styled(format!("{}-", chunk), token.style)]);
                remaining = rest;
            }
            if !remaining.is_empty() {
                current = vec![Span::styled(remaining.to_string(), token.style)];
                current_len = remaining.len();
                last_style = token.style;
            }
            continue;
        }

        let new_len = if current.is_empty() {
            token.text.len()
        } else {
            current_len + 1 + token.text.len()
        };

        if new_len <= max_width {
            if !current.is_empty() {
                current.push(Span::styled(" ".to_string(), last_style));
                current_len += 1;
            }
            let token_len = token.text.len();
            current.push(Span::styled(token.text, token.style));
            current_len += token_len;
            last_style = token.style;
        } else {
            if !current.is_empty() {
                result.push(current);
            }
            let token_len = token.text.len();
            current = vec![Span::styled(token.text, token.style)];
            current_len = token_len;
            last_style = token.style;
        }
    }

    if !current.is_empty() {
        result.push(current);
    }

    result
}

fn find_query_ranges(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }

    let text_lower = text.to_lowercase();
    let query_lower = query.to_lowercase();

    let mut ranges = Vec::new();
    let mut pos = 0usize;
    while let Some(found) = text_lower[pos..].find(&query_lower) {
        let start = pos + found;
        ranges.push(start..start + query_lower.len());
        pos = start + query_lower.len();
    }

    ranges
}

fn apply_highlight_ranges_to_spans(
    spans: &[Span<'static>],
    ranges: &[std::ops::Range<usize>],
    theme: &Theme,
) -> Vec<Span<'static>> {
    if ranges.is_empty() {
        return spans.to_vec();
    }

    let mut out: Vec<Span<'static>> = Vec::new();
    let mut offset = 0usize;

    for span in spans {
        let content = span.content.as_ref();
        let span_len = content.len();
        let span_start = offset;
        let span_end = offset + span_len;

        let mut last = 0usize;
        for range in ranges {
            if range.end <= span_start || range.start >= span_end {
                continue;
            }

            let overlap_start = range.start.saturating_sub(span_start).min(span_len);
            let overlap_end = range.end.saturating_sub(span_start).min(span_len);

            if overlap_start > last {
                out.push(Span::styled(
                    content[last..overlap_start].to_string(),
                    span.style,
                ));
            }

            if overlap_end > overlap_start {
                out.push(Span::styled(
                    content[overlap_start..overlap_end].to_string(),
                    span.style
                        .fg(theme.header_fg)
                        .bg(theme.streaming)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            last = overlap_end;
        }

        if last < span_len {
            out.push(Span::styled(content[last..].to_string(), span.style));
        }

        offset += span_len;
    }

    out
}

// Note: composer sizing + cursor mapping helpers live in `widgets::composer`.

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

    // Claude Code UI reads like a terminal transcript; keep header minimal.
    // We still render one dim line for session context.
    let header_height: u16 = 1;
    let content_min: u16 = if is_compact { 4 } else { 8 };
    let status_height: u16 = 1;

    // Keep the composer from consuming the whole screen. In practice this yields a
    // multi-line editor feel: it grows until max, then scrolls internally.
    let min_input_height: u16 = 3;
    let max_input_height: u16 = 10.min(
        area.height
            .saturating_sub(header_height)
            .saturating_sub(status_height)
            .saturating_sub(content_min)
            .max(min_input_height),
    );
    let input_height = composer::composer_height_for_input(
        &app.input,
        area.width,
        min_input_height,
        max_input_height,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(content_min),
            Constraint::Length(input_height),
            Constraint::Length(status_height),
        ])
        .split(area);

    // Store layout areas for mouse click detection
    app.layout_areas.tabs = Some(chunks[0]);
    app.layout_areas.messages = Some(chunks[1]);
    app.layout_areas.input = Some(chunks[2]);

    render_header(app, frame, chunks[0]);
    render_content(app, frame, chunks[1]);
    composer::render_input(app, frame, chunks[2]);
    render_status_bar(app, frame, chunks[3]);

    // Render overlays
    if app.mode == TuiMode::Help {
        render_help_overlay(app, frame, area);
    } else if app.mode == TuiMode::Confirm {
        render_confirm_dialog(app, frame, area);
    } else if app.mode == TuiMode::ToolConfirm {
        render_tool_confirm_overlay(app, frame, area);
    } else if app.mode == TuiMode::ModelPicker {
        render_model_picker_overlay(app, frame, area);
    } else if app.mode == TuiMode::Activity {
        render_activity_overlay(app, frame, area);
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

    // Keep the message minimal (Claude-like): no boxes, no heavy chrome.
    let paragraph = Paragraph::new(message)
        .style(
            Style::default()
                .fg(app.theme.error_msg)
                .add_modifier(Modifier::DIM),
        )
        .alignment(ratatui::layout::Alignment::Center)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

/// Render compact header for small terminals (single line, no tabs)
///
/// Displays a Claude Code-like header with:
/// - Left-aligned content with bounding box
/// - Formatted model name (GPT-4o, Claude 3.5 Sonnet, etc.)
/// - Voice model when enabled
fn render_compact_header(app: &TuiApp, frame: &mut Frame, area: Rect) {
    use ratatui::widgets::{Block, Borders};

    // Get provider and model info (respects session overrides)
    let provider = app.provider_name();
    let model_id = app.model_name();
    let formatted_model = gestura_core::format_model_name(&provider, &model_id);

    // Build header content parts
    let mut parts: Vec<String> = vec![formatted_model];

    // Add voice model if enabled
    let voice_provider = app.config.voice.provider.as_str();
    if voice_provider != "none" {
        let voice_label = match voice_provider {
            "openai" => {
                let model = app
                    .config
                    .voice
                    .openai_model
                    .as_deref()
                    .unwrap_or("whisper-1");
                format!("Voice: {}", model)
            }
            "local" => "Voice: Local".to_string(),
            other => format!("Voice: {}", other),
        };
        parts.push(voice_label);
    }

    // Add mode indicator
    let mode_str = match app.mode {
        TuiMode::Normal => "NORMAL",
        TuiMode::Insert => "INSERT",
        TuiMode::Command => "COMMAND",
        TuiMode::Help => "HELP",
        TuiMode::Confirm => "CONFIRM",
        TuiMode::ToolConfirm => "CONFIRM",
        TuiMode::Search => "SEARCH",
        TuiMode::ModelPicker => "MODEL",
        TuiMode::Activity => "ACTIVITY",
        TuiMode::Settings => "SETTINGS",
        TuiMode::Workflows => "WORKFLOWS",
        TuiMode::Tools => "TOOLS",
    };
    parts.push(mode_str.to_string());

    let header_text = format!(" {} ", parts.join(" │ "));

    // Create block with bottom border (Claude Code-like bounding box)
    let block = Block::default().borders(Borders::BOTTOM).border_style(
        Style::default()
            .fg(app.theme.border)
            .add_modifier(Modifier::DIM),
    );

    let paragraph = Paragraph::new(header_text)
        .style(
            Style::default()
                .fg(app.theme.header_fg)
                .add_modifier(Modifier::BOLD),
        )
        .block(block);

    frame.render_widget(paragraph, area);
}

/// Render the header.
///
/// For Claude Code visual parity we render a single-line, dim header.
fn render_header(app: &TuiApp, frame: &mut Frame, area: Rect) {
    render_compact_header(app, frame, area);
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

    // Claude Code-style default view: when there are no messages yet, render a
    // minimal home screen with tips and current model.
    if app.messages.is_empty() {
        render_empty_chat_view(app, frame, area);
        return;
    }

    // Calculate available width for text wrapping.
    // Transcript prefixes are at most 2 columns (e.g. "> ", "# ", "! ").
    let wrap_width = area.width.saturating_sub(2) as usize;

    // Filter messages if in filter mode
    let message_indices: Vec<usize> = if app.search_filter_mode && has_search {
        app.search_matches.iter().map(|(idx, _)| *idx).collect()
    } else {
        (0..app.messages.len()).collect()
    };

    // Build the flat list of rendered lines and a parallel mapping from each
    // rendered line back to its source message index.
    let mut all_items: Vec<ListItem> = Vec::new();
    let mut line_to_message: Vec<usize> = Vec::new();

    for &msg_idx in &message_indices {
        let msg = &app.messages[msg_idx];
        let match_ranges = if has_search {
            app.get_match_ranges(msg_idx)
        } else {
            None
        };
        let should_highlight = match_ranges.is_some() && has_search;

        let (prefix, base_style) = match msg.role.as_str() {
            "user" => ("> ", Style::default().fg(theme.user_msg)),
            "assistant" => {
                if msg.is_streaming {
                    (
                        "",
                        Style::default()
                            .fg(theme.streaming)
                            .add_modifier(Modifier::ITALIC),
                    )
                } else {
                    ("", Style::default().fg(theme.assistant_msg))
                }
            }
            "system" => ("# ", Style::default().fg(theme.system_msg)),
            _ => ("", Style::default()),
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

        // Render Thinking if present (with word wrapping) using transcript style.
        if let Some(thinking) = &msg.thinking
            && !thinking.is_empty()
        {
            items.push(ListItem::new(Line::from(Span::styled(
                "... thinking",
                Style::default()
                    .fg(theme.code_comment)
                    .add_modifier(Modifier::ITALIC | Modifier::DIM),
            ))));

            // Wrap thinking text (indent 2 spaces)
            let thinking_wrap_width = wrap_width.saturating_sub(2);
            let wrapped_thinking = wrap_text(thinking, thinking_wrap_width);
            for line in wrapped_thinking {
                items.push(ListItem::new(Line::from(Span::styled(
                    format!("  {}", line),
                    Style::default()
                        .fg(theme.code_comment)
                        .add_modifier(Modifier::ITALIC | Modifier::DIM),
                ))));
            }

            items.push(ListItem::new(Line::from("")));
        }
        let mut is_first = true;

        for segment in segments {
            match segment {
                MessageSegment::Text(text) => {
                    // Render markdown styling first, then wrap styled spans.
                    let rendered = markdown::markdown_to_text_with_base(&text, theme, base_style);

                    for rendered_line in rendered.lines {
                        let wrapped = wrap_spans(&rendered_line.spans, wrap_width);
                        for wrapped_spans in wrapped {
                            let display_prefix = if is_first {
                                is_first = false;
                                prefix
                            } else if prefix.is_empty() {
                                ""
                            } else {
                                "  "
                            };

                            let mut spans: Vec<Span<'static>> = Vec::new();
                            spans.push(Span::styled(display_prefix.to_string(), base_style));

                            if should_highlight {
                                let visible: String =
                                    wrapped_spans.iter().map(|s| s.content.as_ref()).collect();
                                let ranges = find_query_ranges(&visible, search_query);
                                spans.extend(apply_highlight_ranges_to_spans(
                                    &wrapped_spans,
                                    &ranges,
                                    theme,
                                ));
                            } else {
                                spans.extend(wrapped_spans);
                            }

                            items.push(ListItem::new(Line::from(spans)));
                        }
                    }
                }
                MessageSegment::CodeBlock { language, code } => {
                    // Transcript-style fenced code blocks (avoid box-drawing chrome).
                    let lang_label = language.as_deref().unwrap_or("");
                    let fence_open = if lang_label.is_empty() {
                        "```".to_string()
                    } else {
                        format!("```{}", lang_label)
                    };
                    items.push(ListItem::new(Line::from(Span::styled(
                        fence_open,
                        Style::default()
                            .fg(theme.code_lang_label)
                            .add_modifier(Modifier::DIM),
                    ))));

                    // Add highlighted code lines (indented)
                    for code_line in code.lines() {
                        let mut spans = vec![Span::styled(
                            "    ".to_string(),
                            Style::default()
                                .fg(theme.code_lang_label)
                                .add_modifier(Modifier::DIM),
                        )];
                        spans.extend(highlight_code_line_themed(
                            code_line,
                            language.as_deref(),
                            theme,
                        ));
                        items.push(ListItem::new(Line::from(spans)));
                    }

                    items.push(ListItem::new(Line::from(Span::styled(
                        "```",
                        Style::default()
                            .fg(theme.code_lang_label)
                            .add_modifier(Modifier::DIM),
                    ))));
                    items.push(ListItem::new(Line::from("")));
                    is_first = false;
                }
            }
        }

        let final_items = if items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                prefix.to_string(),
                base_style,
            )))]
        } else {
            items
        };

        // Record the mapping: each rendered line maps back to this message.
        for _ in &final_items {
            line_to_message.push(msg_idx);
        }
        all_items.extend(final_items);
    }

    // Persist the line count and mapping so scroll logic uses the correct bounds.
    app.rendered_line_count = all_items.len();
    app.line_to_message_map = line_to_message;

    let messages_block = Block::default().borders(Borders::NONE);

    let list = List::new(all_items).block(messages_block).highlight_style(
        Style::default()
            .bg(theme.selection_bg)
            .add_modifier(Modifier::BOLD),
    );

    // IMPORTANT: pass the real `message_list_state` — NOT a clone — so ratatui's
    // viewport offset updates are persisted between frames.
    frame.render_stateful_widget(list, area, &mut app.message_list_state);
}

/// Render a Claude Code-like home/default view when there are no messages.
///
/// This keeps the main transcript window visually "empty" (no heavy chrome)
/// while still guiding first-time users toward the key commands.
fn render_empty_chat_view(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let model_label = effective_model_label(app);
    let lines = vec![
        Line::from(Span::styled(
            "Gestura",
            Style::default()
                .fg(app.theme.header_fg)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("model: {}", model_label),
            Style::default()
                .fg(app.theme.status_fg)
                .add_modifier(Modifier::DIM),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "tips",
            Style::default()
                .fg(app.theme.status_fg)
                .add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "  /model     select model",
            Style::default().fg(app.theme.code_comment),
        )),
        Line::from(Span::styled(
            "  /activity  agent activity (tool calls)",
            Style::default().fg(app.theme.code_comment),
        )),
        Line::from(Span::styled(
            "  /help      keys & commands",
            Style::default().fg(app.theme.code_comment),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "type to chat (insert mode) • press / to run a command",
            Style::default()
                .fg(app.theme.status_fg)
                .add_modifier(Modifier::DIM),
        )),
    ];

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .alignment(ratatui::layout::Alignment::Center)
        .wrap(Wrap { trim: true });

    frame.render_widget(p, area);
}

/// Compute a short `provider:model` label for display.
///
/// Prefers session-scoped overrides, then CLI session `model` hint, then config.
fn effective_model_label(app: &TuiApp) -> String {
    if let Some(cfg) = app.session.state.llm_config.as_ref() {
        let provider = cfg.provider.as_deref().unwrap_or("").trim();
        let model = cfg.model.as_deref().unwrap_or("").trim();
        if !provider.is_empty() && !model.is_empty() {
            return format!("{}:{}", provider, model);
        }
        if !provider.is_empty() {
            return provider.to_string();
        }
    }

    if let Some(m) = app.session.model.as_deref() {
        let m = m.trim();
        if !m.is_empty() {
            return m.to_string();
        }
    }

    let provider = app.config.llm.primary.trim();
    if provider.is_empty() {
        "default".to_string()
    } else {
        provider.to_string()
    }
}

/// Render the model picker overlay.
///
/// This is a Claude Code-like overlay: type-to-filter, arrow keys to select,
/// Enter to apply, Esc to close.
fn render_model_picker_overlay(app: &mut TuiApp, frame: &mut Frame, area: Rect) {
    let popup_width = 70.min(area.width.saturating_sub(4));
    let popup_height = 18.min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(popup_area);

    let title = Span::styled(
        " Model ",
        Style::default()
            .fg(app.theme.header_fg)
            .add_modifier(Modifier::BOLD),
    );

    let filter_line = format!(" filter: {}", app.model_picker_state.query);
    let filter = Paragraph::new(filter_line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.border))
                .title(title),
        )
        .style(
            Style::default()
                .fg(app.theme.status_fg)
                .add_modifier(Modifier::DIM),
        );
    frame.render_widget(filter, chunks[0]);

    let items: Vec<ListItem> = app
        .model_picker_state
        .filtered
        .iter()
        .filter_map(|idx| app.model_picker_state.items.get(*idx))
        .map(|it| {
            ListItem::new(Line::from(Span::styled(
                it.label.clone(),
                Style::default().fg(app.theme.assistant_msg),
            )))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(
            Style::default()
                .fg(app.theme.tab_active)
                .bg(app.theme.selection_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");

    frame.render_stateful_widget(list, chunks[1], &mut app.model_picker_state.list_state);

    let hint = Paragraph::new(" type to filter • ↑↓ to select • Enter apply • Esc close ")
        .style(
            Style::default()
                .fg(app.theme.status_fg)
                .add_modifier(Modifier::DIM),
        )
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(hint, chunks[2]);
}

/// Render the agent activity overlay.
///
/// This displays a scrollable transcript of tool calls (name, args preview, result).
fn render_activity_overlay(app: &mut TuiApp, frame: &mut Frame, area: Rect) {
    let popup_width = 80.min(area.width.saturating_sub(4));
    let popup_height = area.height.saturating_sub(4).clamp(10, 24);
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let title = Span::styled(
        format!(" Activity ({}) ", app.activity_state.entries.len()),
        Style::default()
            .fg(app.theme.header_fg)
            .add_modifier(Modifier::BOLD),
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border))
        .title(title);

    let items: Vec<ListItem> = app
        .activity_state
        .entries
        .iter()
        .map(|e| {
            let style = if e.is_error {
                Style::default().fg(app.theme.error_msg)
            } else {
                Style::default().fg(app.theme.status_fg)
            };
            ListItem::new(Text::from(e.text.clone())).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_symbol("› ")
        .highlight_style(
            Style::default()
                .fg(app.theme.tab_active)
                .bg(app.theme.selection_bg),
        );

    frame.render_stateful_widget(list, popup_area, &mut app.activity_state.list_state);
}

/// Render the tools tab
fn render_tools_tab(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let tools_markdown = gestura_core::tools::render_tools_overview();
    let tools_text = markdown::markdown_to_text(&tools_markdown, &app.theme);
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
            .add_modifier(Modifier::DIM)
    } else {
        Style::default()
            .fg(app.theme.status_fg)
            .add_modifier(Modifier::DIM)
    };

    let status_text = if let Some(e) = &app.error {
        if is_compact {
            format!("! {}", &e[..e.len().min(20)])
        } else {
            format!("! {}", e)
        }
    } else if is_compact {
        format!("{:?}", app.mode)
    } else {
        format!("{} [{:?}]", app.status, app.mode)
    };

    frame.render_widget(Paragraph::new(status_text).style(status_style), layout[0]);

    // Right: Stats - use compact format for narrow terminals
    let stats_style = Style::default()
        .fg(app.theme.status_fg)
        .add_modifier(Modifier::DIM);

    // Use effective provider/model (respects session overrides via /model)
    let provider = app.provider_name();
    let model = app.model_name();
    let formatted_model = gestura_core::format_model_name(&provider, &model);
    let is_local = gestura_core::is_local_provider(&provider);

    let mut stats_text = if is_compact {
        // Compact format for narrow terminals
        let token_total = app.session_input_tokens + app.session_output_tokens;
        let short_model = if formatted_model.len() > 12 {
            &formatted_model[..12]
        } else {
            &formatted_model
        };
        if token_total == 0 {
            format!("{} ", short_model)
        } else if is_local {
            // Local providers: show only tokens, no cost
            format!(
                "{} | {} ",
                gestura_core::token_tracker::format_token_count(token_total),
                short_model
            )
        } else {
            // Remote providers: show tokens and cost
            let cost = app.session_cost_usd;
            format!(
                "{}|${:.2} | {} ",
                gestura_core::token_tracker::format_token_count(token_total),
                cost,
                short_model
            )
        }
    } else {
        // Verbose format for wide terminals
        let token_total = app.session_input_tokens + app.session_output_tokens;
        if is_local {
            // Local providers: show only tokens, no cost
            format!(
                "Tokens: {} | Model: {} ",
                gestura_core::token_tracker::format_token_count(token_total),
                formatted_model
            )
        } else {
            // Remote providers: show tokens and cost
            let cost = app.session_cost_usd;
            format!(
                "Tokens: {} | Cost: ${:.4} | Model: {} ",
                gestura_core::token_tracker::format_token_count(token_total),
                cost,
                formatted_model
            )
        }
    };

    // When the user has scrolled away from the bottom, show a subtle position indicator.
    // This keeps the default “Claude-like” chrome minimal, while still providing orientation.
    if !app.is_at_bottom() {
        stats_text = format!("{} | {} ", app.scroll_indicator(), stats_text.trim_end());
    }

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

    // 1. Provider (show effective provider, respects session overrides)
    let provider = app.provider_name();
    items.push(render_field(0, "Provider", &provider));

    // 2. Model (show effective model, respects session overrides)
    let model = app.model_name();
    items.push(render_field(1, "Model", &model));

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
                " Commands (Tab to complete, Enter to run, ↑↓ to select) ",
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

/// Render a tool confirmation overlay (modal).
///
/// This is displayed when the core pipeline requires a scoped decision for a tool call.
fn render_tool_confirm_overlay(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let Some(pending) = app.pending_tool_confirmation.as_ref() else {
        return;
    };

    // Center the popup
    let popup_width = 74.min(area.width.saturating_sub(4));
    let popup_height = 12.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    /// Truncate a string to a maximum character count for compact UI previews.
    fn truncate_preview(s: &str, max_chars: usize) -> String {
        let mut out: String = s.chars().take(max_chars).collect();
        if s.chars().count() > max_chars {
            out.push('…');
        }
        out
    }

    let args_preview = truncate_preview(&pending.tool_args.replace('\n', " "), 140);

    let message = format!(
        "Tool: {tool}\nCategory: {cat}   Risk: {risk}\n\n{desc}\n\nArgs: {args}\n\n[1] Allow once  [2] Allow session  [3] Allow always\n[4] Deny once   [5] Deny session   [Esc] Deny once",
        tool = pending.tool_name,
        cat = pending.category,
        risk = pending.risk_level,
        desc = pending.description,
        args = args_preview
    );

    let paragraph = Paragraph::new(message)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.error_msg))
                .title(Span::styled(
                    " Tool Confirmation Required ",
                    Style::default()
                        .fg(app.theme.error_msg)
                        .add_modifier(Modifier::BOLD),
                )),
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_message_segments_returns_text_when_no_fences() {
        let segs = parse_message_segments("hello\nworld");
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            MessageSegment::Text(t) => assert_eq!(t, "hello\nworld"),
            _ => panic!("expected Text segment"),
        }
    }

    #[test]
    fn parse_message_segments_splits_text_and_code_blocks() {
        let input = "before\n```rust\nfn main() {}\n```\nafter";
        let segs = parse_message_segments(input);
        assert_eq!(segs.len(), 3);

        match &segs[0] {
            MessageSegment::Text(t) => assert_eq!(t, "before"),
            _ => panic!("expected first segment to be Text"),
        }
        match &segs[1] {
            MessageSegment::CodeBlock { language, code } => {
                assert_eq!(language.as_deref(), Some("rust"));
                assert_eq!(code, "fn main() {}");
            }
            _ => panic!("expected second segment to be CodeBlock"),
        }
        match &segs[2] {
            MessageSegment::Text(t) => assert_eq!(t, "after"),
            _ => panic!("expected third segment to be Text"),
        }
    }

    #[test]
    fn parse_message_segments_keeps_unclosed_code_block() {
        let input = "```\nline1\nline2";
        let segs = parse_message_segments(input);
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            MessageSegment::CodeBlock { language, code } => {
                assert!(language.is_none());
                assert_eq!(code, "line1\nline2");
            }
            _ => panic!("expected CodeBlock segment"),
        }
    }

    #[test]
    fn wrap_text_preserves_blank_lines() {
        let lines = wrap_text("hello\n\nworld", 80);
        assert_eq!(
            lines,
            vec!["hello".to_string(), "".to_string(), "world".to_string()]
        );
    }

    #[test]
    fn wrap_text_wraps_by_words() {
        let lines = wrap_text("hello world", 5);
        assert_eq!(lines, vec!["hello".to_string(), "world".to_string()]);
    }

    #[test]
    fn wrap_text_hyphenates_long_words() {
        let lines = wrap_text("abcdefghij", 5);
        assert_eq!(
            lines,
            vec!["abcd-".to_string(), "efgh-".to_string(), "ij".to_string()]
        );
    }
}
