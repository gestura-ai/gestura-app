//! Pinned composer widget.
//!
//! The composer is the bottom input area that behaves like Claude Code: it is pinned
//! to the bottom of the terminal, grows with wrapped/multiline input up to a cap,
//! and then scrolls internally to keep the cursor visible.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::super::app::{TuiApp, TuiMode};
use super::thinking;

/// Render the pinned composer (input field) including cursor placement.
pub(crate) fn render_input(app: &TuiApp, frame: &mut Frame, area: Rect) {
    let mode_indicator = match app.mode {
        TuiMode::Normal => ("NORMAL", app.theme.mode_normal),
        TuiMode::Insert => ("INSERT", app.theme.mode_insert),
        TuiMode::Command => ("COMMAND", app.theme.mode_command),
        TuiMode::Help => ("HELP", app.theme.mode_normal),
        TuiMode::Confirm => ("CONFIRM", app.theme.error_msg),
        TuiMode::ToolConfirm => ("TOOLCONF", app.theme.error_msg),
        TuiMode::Search => ("SEARCH", app.theme.streaming),
        TuiMode::ModelPicker => ("MODEL", app.theme.mode_normal),
        TuiMode::Activity => ("ACTIVITY", app.theme.mode_normal),
        TuiMode::Settings => ("SETTINGS", app.theme.mode_normal),
        TuiMode::Workflows => ("WORKFLOWS", app.theme.mode_normal),
        TuiMode::Tools => ("TOOLS", app.theme.mode_normal),
        TuiMode::Capabilities => ("CAPABILITIES", app.theme.mode_normal),
        TuiMode::Mcp => ("MCP", app.theme.mode_normal),
        TuiMode::Knowledge => ("KNOWLEDGE", app.theme.mode_normal),
        TuiMode::Config => ("CONFIG", app.theme.mode_normal),
        TuiMode::Context => ("CONTEXT", app.theme.mode_normal),
        TuiMode::A2a => ("A2A", app.theme.mode_normal),
        TuiMode::Privacy => ("PRIVACY", app.theme.mode_normal),
        TuiMode::Hooks => ("HOOKS", app.theme.mode_normal),
        TuiMode::Agent => ("AGENT", app.theme.mode_normal),
        TuiMode::Memory => ("MEMORY", app.theme.mode_normal),
        TuiMode::Devices => ("DEVICES", app.theme.mode_normal),
        TuiMode::Permissions => ("PERMISSIONS", app.theme.mode_normal),
        TuiMode::Sessions => ("SESSIONS", app.theme.mode_normal),
        TuiMode::Tasks => ("TASKS", app.theme.mode_normal),
        TuiMode::Themes => ("THEMES", app.theme.mode_normal),
    };

    // Always use the muted border color for the separator line so it stays subtle.
    // The mode indicator label retains its bright accent color for at-a-glance feedback.
    let border_style = Style::default().fg(app.theme.border);

    // Claude-like composer: no full box. Use a single subtle top separator line.
    // That line consumes 1 row; the remaining rows are the editable area.
    let inner_width = area.width as usize;
    let inner_height = area.height.saturating_sub(1) as usize;

    let (cursor_row, cursor_col_logical) =
        wrapped_cursor_row_col(app.input.as_str(), app.cursor_pos, inner_width);

    // Keep the cursor visible by scrolling the paragraph content inside the pinned composer.
    let max_visible_row = inner_height.saturating_sub(1);
    let scroll_y = if inner_height == 0 {
        0usize
    } else {
        cursor_row.saturating_sub(max_visible_row)
    };
    let cursor_row_visible = cursor_row.saturating_sub(scroll_y);

    // Clamp column to avoid placing cursor on the border cell when the cursor is at end-of-line.
    let cursor_col = if inner_width == 0 {
        0usize
    } else {
        cursor_col_logical.min(inner_width.saturating_sub(1))
    };

    // Build the block with either the animated thinking title or the static mode title.
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(border_style.add_modifier(Modifier::DIM));

    let block = if app.is_loading {
        let spans = thinking::thinking_title_spans(app.loading_tick);
        block.title(Line::from(spans))
    } else {
        let title = format!(" {} ", mode_indicator.0.to_lowercase());
        block.title(Span::styled(
            title,
            Style::default()
                .fg(mode_indicator.1)
                .add_modifier(Modifier::DIM),
        ))
    };

    let input = Paragraph::new(app.input.as_str())
        .block(block)
        .style(Style::default().fg(app.theme.header_fg))
        .wrap(Wrap { trim: false })
        .scroll((scroll_y as u16, 0));

    frame.render_widget(input, area);

    // Show cursor in insert/command mode.
    if (app.mode == TuiMode::Insert || app.mode == TuiMode::Command) && inner_height > 0 {
        frame.set_cursor_position((
            area.x + cursor_col as u16,
            area.y + 1 + cursor_row_visible as u16,
        ));
    }
}

/// Compute the composer block height (including borders) for a pinned input area.
///
/// The composer grows with input up to `max_height`. Beyond that, it stays pinned and
/// the input paragraph scrolls internally.
pub(crate) fn composer_height_for_input(
    input: &str,
    area_width: u16,
    min_height: u16,
    max_height: u16,
) -> u16 {
    // Borders::TOP does not consume horizontal space.
    let inner_width = area_width as usize;
    let lines = wrapped_line_count(input, inner_width).max(1);
    let desired = (lines as u16).saturating_add(1); // +1 for top separator line
    desired.clamp(min_height, max_height)
}

/// Count how many visual lines `text` will occupy when soft-wrapped to `width` columns.
///
/// Notes:
/// - `width == 0` returns `1`.
/// - Explicit newlines always start a new visual line.
fn wrapped_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }

    let mut row = 0usize;
    let mut col = 0usize;

    for ch in text.chars() {
        if ch == '\n' {
            row += 1;
            col = 0;
            continue;
        }

        if col == width {
            row += 1;
            col = 0;
        }

        col += 1;
    }

    // Number of rows is 0-based.
    row + 1
}

/// Convert a byte cursor index into (row, col) coordinates for a soft-wrapped editor.
///
/// The returned column is a *logical* column; it may equal `width` when the cursor is
/// positioned at the end of a full line. Callers rendering a terminal cursor should
/// clamp the column into `[0, width.saturating_sub(1)]`.
fn wrapped_cursor_row_col(text: &str, cursor_pos: usize, width: usize) -> (usize, usize) {
    if width == 0 {
        return (0, 0);
    }

    let cursor_pos = cursor_pos.min(text.len());
    let mut row = 0usize;
    let mut col = 0usize;

    for (idx, ch) in text.char_indices() {
        if idx >= cursor_pos {
            break;
        }

        if ch == '\n' {
            row += 1;
            col = 0;
            continue;
        }

        if col == width {
            row += 1;
            col = 0;
        }

        col += 1;
    }

    (row, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_line_count_handles_newlines_and_wrap() {
        assert_eq!(wrapped_line_count("", 10), 1);
        assert_eq!(wrapped_line_count("a\n", 10), 2);
        assert_eq!(wrapped_line_count("hello", 5), 1);
        assert_eq!(wrapped_line_count("helloo", 5), 2);
    }

    #[test]
    fn wrapped_cursor_row_col_tracks_row_and_col() {
        // Width 5, cursor after 6 chars should be on row 1, col 1 (wrap before 6th char).
        let (r, c) = wrapped_cursor_row_col("helloo", 6, 5);
        assert_eq!((r, c), (1, 1));

        // Newline resets column and increments row.
        let (r, c) = wrapped_cursor_row_col("a\nb", 2, 10);
        assert_eq!((r, c), (1, 0));
    }

    #[test]
    fn composer_height_respects_min_and_max() {
        // Empty input should still occupy min height.
        assert_eq!(composer_height_for_input("", 80, 3, 10), 3);

        // A long input should clamp to max.
        let long = "x".repeat(100);
        assert_eq!(composer_height_for_input(&long, 10, 3, 5), 5);
    }
}
