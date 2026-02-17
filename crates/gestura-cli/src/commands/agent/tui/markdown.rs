//! Minimal Markdown → ratatui text renderer for the CLI TUI.
//!
//! The TUI currently receives some content as Markdown (notably the tools catalog
//! emitted by `gestura-core::tools::*`). ratatui does not render Markdown, so we
//! convert a small, known subset into styled [`ratatui::text::Text`].
//!
//! Supported:
//! - `**bold**`
//! - `*italic*`
//! - `` `inline code` ``
//! - Bullet lines starting with `• ` or `- `
//! - Links: `[label](url)` (renders as `label (url)` with link styling)
//! - Auto-links bare `http(s)://...` URLs
//! - Headings starting with `# ` through `###### `
//! - GitHub-flavored markdown tables (basic): header row + separator row
//! - Fenced code blocks using triple backticks (fence lines are hidden)

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
};

use super::app::Theme;

/// Convert a small Markdown subset to styled ratatui `Text`.
#[allow(dead_code)]
pub(crate) fn markdown_to_text(markdown: &str, theme: &Theme) -> Text<'static> {
    let base = Style::default().fg(theme.header_fg);
    let bullet = Style::default()
        .fg(theme.system_msg)
        .add_modifier(Modifier::DIM);
    let heading = Style::default()
        .fg(theme.assistant_msg)
        .add_modifier(Modifier::BOLD);
    markdown_to_text_impl(markdown, theme, base, bullet, heading)
}

/// Like [`markdown_to_text`], but uses the provided `base` style for normal text.
///
/// This is useful when rendering markdown-like content inside role-colored
/// transcript messages.
pub(crate) fn markdown_to_text_with_base(
    markdown: &str,
    theme: &Theme,
    base: Style,
) -> Text<'static> {
    let bullet = base.add_modifier(Modifier::DIM);
    let heading = base.add_modifier(Modifier::BOLD);
    markdown_to_text_impl(markdown, theme, base, bullet, heading)
}

fn markdown_to_text_impl(
    markdown: &str,
    theme: &Theme,
    base: Style,
    bullet: Style,
    heading: Style,
) -> Text<'static> {
    let code = Style::default().fg(theme.code_fg).bg(theme.code_bg);

    // Link styling: underline + accent for the label, dim for the URL.
    let link_label = Style::default()
        .fg(theme.assistant_msg)
        .add_modifier(Modifier::UNDERLINED);
    let link_url = Style::default()
        .fg(theme.system_msg)
        .add_modifier(Modifier::DIM);

    let table_border = Style::default()
        .fg(theme.system_msg)
        .add_modifier(Modifier::DIM);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_fence = false;

    let src_lines: Vec<&str> = markdown.lines().collect();
    let mut i = 0usize;
    while i < src_lines.len() {
        let raw = src_lines[i];
        let line = raw.trim_end_matches(['\r', '\n']);

        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            i += 1;
            continue;
        }

        if in_fence {
            lines.push(Line::from(Span::styled(line.to_string(), code)));
            i += 1;
            continue;
        }

        if trimmed.is_empty() {
            lines.push(Line::from(""));
            i += 1;
            continue;
        }

        // Tables (basic GFM): header line + separator line.
        if line.contains('|')
            && i + 1 < src_lines.len()
            && is_markdown_table_separator_line(src_lines[i + 1])
        {
            let header_line = line;
            let sep_line = src_lines[i + 1];
            i += 2;

            let mut row_lines: Vec<&str> = Vec::new();
            while i < src_lines.len() {
                let rl = src_lines[i];
                if rl.trim().is_empty() {
                    break;
                }
                if !rl.contains('|') {
                    break;
                }
                row_lines.push(rl);
                i += 1;
            }

            lines.extend(render_markdown_table(
                header_line,
                sep_line,
                &row_lines,
                TableRenderStyles {
                    base,
                    code_style: code,
                    link_label,
                    link_url,
                    border_style: table_border,
                },
            ));
            continue;
        }

        // Headings: #..######
        if let Some((level, rest)) = parse_heading(line) {
            let heading_style = heading_style_for_level(heading, level);
            lines.push(Line::from(parse_inline(
                rest,
                heading_style,
                code,
                heading_style
                    .fg(theme.assistant_msg)
                    .add_modifier(Modifier::UNDERLINED),
                link_url,
            )));
            i += 1;
            continue;
        }

        // Bullets (normalize "- " to "• ").
        let bullet_line = line.trim_start();
        if let Some(rest) = bullet_line
            .strip_prefix("• ")
            .or_else(|| bullet_line.strip_prefix("- "))
        {
            let mut spans = Vec::new();
            spans.push(Span::styled("• ".to_string(), bullet));
            spans.extend(parse_inline(rest, base, code, link_label, link_url));
            lines.push(Line::from(spans));
            i += 1;
            continue;
        }

        lines.push(Line::from(parse_inline(
            line, base, code, link_label, link_url,
        )));
        i += 1;
    }

    Text::from(lines)
}

fn parse_inline(
    line: &str,
    base: Style,
    code_style: Style,
    link_label: Style,
    link_url: Style,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();

    let mut bold = false;
    let mut italic = false;
    let mut code = false;

    let mut i = 0usize;
    while i < line.len() {
        let rest = &line[i..];

        // Links are parsed before other inline markers (when not in code spans).
        if !code {
            if let Some(link) = find_markdown_link_at_or_after(line, i) {
                let (start, end, label, url) = link;
                if start > i {
                    let style = current_style(base, code_style, bold, italic, code);
                    spans.push(Span::styled(line[i..start].to_string(), style));
                }

                // Render label as link-styled text (still supports inline bold/italic/code).
                spans.extend(parse_inline(
                    label, link_label, code_style, link_label, link_url,
                ));

                // Always show URL so the user can see/copy it in terminals without hyperlink support.
                spans.push(Span::styled(" (".to_string(), link_url));
                spans.push(Span::styled(url.to_string(), link_url));
                spans.push(Span::styled(")".to_string(), link_url));

                i = end;
                continue;
            }

            if let Some(link) = find_autolink_url_at_or_after(line, i) {
                let (start, end, url) = link;
                if start > i {
                    let style = current_style(base, code_style, bold, italic, code);
                    spans.push(Span::styled(line[i..start].to_string(), style));
                }
                spans.push(Span::styled(url.to_string(), link_label));
                i = end;
                continue;
            }
        }
        let next_bold = if code {
            None
        } else {
            rest.find("**").map(|o| i + o)
        };
        let next_italic = if code {
            None
        } else {
            rest.find('*').map(|o| i + o)
        };
        let next_code = rest.find('`').map(|o| i + o);

        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        enum MarkerKind {
            Bold,
            Italic,
            Code,
        }

        let mut next: Option<(usize, MarkerKind)> = None;
        let mut consider = |pos: Option<usize>, kind: MarkerKind| {
            let Some(p) = pos else { return };
            match next {
                None => next = Some((p, kind)),
                Some((best_p, best_kind)) => {
                    if p < best_p {
                        next = Some((p, kind));
                    } else if p == best_p {
                        // Prefer bold over italic when both point at the same index ("**").
                        // Otherwise keep existing.
                        if best_kind == MarkerKind::Italic && kind == MarkerKind::Bold {
                            next = Some((p, kind));
                        }
                    }
                }
            }
        };

        consider(next_bold, MarkerKind::Bold);
        consider(next_italic, MarkerKind::Italic);
        consider(next_code, MarkerKind::Code);

        let (marker_at, marker_kind) = match next {
            Some(v) => v,
            None => {
                let style = current_style(base, code_style, bold, italic, code);
                spans.push(Span::styled(rest.to_string(), style));
                break;
            }
        };

        if marker_at > i {
            let style = current_style(base, code_style, bold, italic, code);
            spans.push(Span::styled(line[i..marker_at].to_string(), style));
        }

        // Toggle style, skip marker.
        match marker_kind {
            MarkerKind::Bold => {
                bold = !bold;
                i = marker_at + 2;
            }
            MarkerKind::Italic => {
                italic = !italic;
                i = marker_at + 1;
            }
            MarkerKind::Code => {
                code = !code;
                i = marker_at + 1;
            }
        }
    }

    spans
}

fn current_style(base: Style, code_style: Style, bold: bool, italic: bool, code: bool) -> Style {
    let mut s = if code { code_style } else { base };
    if bold && !code {
        s = s.add_modifier(Modifier::BOLD);
    }
    if italic && !code {
        s = s.add_modifier(Modifier::ITALIC);
    }
    s
}

fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let mut hashes = 0usize;
    for ch in trimmed.chars() {
        if ch == '#' {
            hashes += 1;
            if hashes > 6 {
                break;
            }
        } else {
            break;
        }
    }
    if hashes == 0 || hashes > 6 {
        return None;
    }

    let after = &trimmed[hashes..];
    // Require at least one whitespace after the hashes.
    let after = after
        .strip_prefix(' ')
        .or_else(|| after.strip_prefix('\t'))
        .map(|s| s.trim())?;

    if after.is_empty() {
        return None;
    }
    Some((hashes, after))
}

fn heading_style_for_level(base_heading: Style, level: usize) -> Style {
    match level {
        1 => base_heading.add_modifier(Modifier::UNDERLINED),
        2..=4 => base_heading,
        5 | 6 => base_heading.add_modifier(Modifier::DIM),
        _ => base_heading,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableAlign {
    Left,
    Right,
    Center,
}

#[derive(Clone, Copy, Debug)]
struct TableRenderStyles {
    base: Style,
    code_style: Style,
    link_label: Style,
    link_url: Style,
    border_style: Style,
}

fn is_markdown_table_separator_line(line: &str) -> bool {
    let src = line.trim();
    if src.is_empty() || !src.contains('|') || !src.contains('-') {
        return false;
    }

    let parts = split_markdown_table_row(src);
    if parts.is_empty() {
        return false;
    }

    for p in parts {
        let p = p.trim();
        if p.is_empty() {
            return false;
        }
        let mut body = p;
        if let Some(rest) = body.strip_prefix(':') {
            body = rest;
        }
        if let Some(rest) = body.strip_suffix(':') {
            body = rest;
        }
        let body = body.trim();
        // Must be 3+ dashes (ignoring whitespace).
        let dash_count = body.chars().filter(|c| *c == '-').count();
        if dash_count < 3 {
            return false;
        }
        if !body.chars().all(|c| c == '-' || c.is_whitespace()) {
            return false;
        }
    }
    true
}

fn split_markdown_table_row(line: &str) -> Vec<String> {
    let mut src = line.trim();
    if let Some(rest) = src.strip_prefix('|') {
        src = rest;
    }
    if let Some(rest) = src.strip_suffix('|') {
        src = rest;
    }

    let mut cells: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut in_code = false;
    let mut escaped = false;

    for ch in src.chars() {
        if escaped {
            cell.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '`' {
            in_code = !in_code;
            cell.push(ch);
            continue;
        }
        if ch == '|' && !in_code {
            cells.push(cell.trim().to_string());
            cell.clear();
            continue;
        }
        cell.push(ch);
    }
    cells.push(cell.trim().to_string());
    cells
}

fn parse_markdown_table_alignments(sep_line: &str, col_count: usize) -> Vec<TableAlign> {
    let parts = split_markdown_table_row(sep_line);
    let mut out = Vec::with_capacity(col_count);
    for idx in 0..col_count {
        let p = parts.get(idx).map(|s| s.trim()).unwrap_or("");
        let starts = p.starts_with(':');
        let ends = p.ends_with(':');
        let a = if starts && ends {
            TableAlign::Center
        } else if ends {
            TableAlign::Right
        } else {
            TableAlign::Left
        };
        out.push(a);
    }
    out
}

fn render_markdown_table(
    header_line: &str,
    sep_line: &str,
    row_lines: &[&str],
    styles: TableRenderStyles,
) -> Vec<Line<'static>> {
    let TableRenderStyles {
        base,
        code_style,
        link_label,
        link_url,
        border_style,
    } = styles;

    let headers = split_markdown_table_row(header_line);
    let col_count = headers.len().max(1);
    let aligns = parse_markdown_table_alignments(sep_line, col_count);

    // Parse cells into spans and plain strings so we can compute column widths.
    let mut header_cells: Vec<(Vec<Span<'static>>, String)> = Vec::with_capacity(col_count);
    for idx in 0..col_count {
        let raw = headers.get(idx).map(|s| s.as_str()).unwrap_or("");
        let cell_base = base.add_modifier(Modifier::BOLD);
        let cell_link = cell_base
            .add_modifier(Modifier::UNDERLINED)
            .fg(link_label.fg.unwrap_or_default());
        let spans = parse_inline(raw, cell_base, code_style, cell_link, link_url);
        let plain: String = spans.iter().map(|s| s.content.as_ref()).collect();
        header_cells.push((spans, plain));
    }

    let mut body_cells: Vec<Vec<(Vec<Span<'static>>, String)>> = Vec::new();
    for rl in row_lines {
        let parts = split_markdown_table_row(rl);
        let mut row: Vec<(Vec<Span<'static>>, String)> = Vec::with_capacity(col_count);
        for idx in 0..col_count {
            let raw = parts.get(idx).map(|s| s.as_str()).unwrap_or("");
            let spans = parse_inline(raw, base, code_style, link_label, link_url);
            let plain: String = spans.iter().map(|s| s.content.as_ref()).collect();
            row.push((spans, plain));
        }
        body_cells.push(row);
    }

    let mut widths = vec![0usize; col_count];
    for (idx, (_spans, plain)) in header_cells.iter().enumerate() {
        widths[idx] = widths[idx].max(plain.chars().count());
    }
    for row in &body_cells {
        for (idx, (_spans, plain)) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(plain.chars().count());
        }
    }

    let border_line = |left: char, mid: char, right: char| -> Line<'static> {
        let mut s = String::new();
        s.push(left);
        for (idx, w) in widths.iter().enumerate() {
            s.push_str(&"─".repeat(w + 2));
            if idx + 1 < widths.len() {
                s.push(mid);
            }
        }
        s.push(right);
        Line::from(Span::styled(s, border_style))
    };

    let render_row = |cells: &[(Vec<Span<'static>>, String)], is_header: bool| -> Line<'static> {
        let mut out: Vec<Span<'static>> = Vec::new();
        out.push(Span::styled("│".to_string(), border_style));

        for (idx, (cell_spans, plain)) in cells.iter().enumerate() {
            let width = widths[idx];
            let len = plain.chars().count();
            let diff = width.saturating_sub(len);

            let (left_pad, right_pad) = match aligns[idx] {
                TableAlign::Left => (0, diff),
                TableAlign::Right => (diff, 0),
                TableAlign::Center => (diff / 2, diff - (diff / 2)),
            };

            out.push(Span::styled(" ".to_string(), base));
            if left_pad > 0 {
                out.push(Span::styled(" ".repeat(left_pad), base));
            }
            if is_header {
                // Ensure header text is bold even if inline parsing emits multiple spans.
                for sp in cell_spans {
                    out.push(Span::styled(
                        sp.content.to_string(),
                        sp.style.add_modifier(Modifier::BOLD),
                    ));
                }
            } else {
                out.extend(cell_spans.iter().cloned());
            }
            if right_pad > 0 {
                out.push(Span::styled(" ".repeat(right_pad), base));
            }
            out.push(Span::styled(" ".to_string(), base));
            out.push(Span::styled("│".to_string(), border_style));
        }
        Line::from(out)
    };

    let mut out_lines: Vec<Line<'static>> = Vec::new();
    out_lines.push(border_line('┌', '┬', '┐'));
    out_lines.push(render_row(
        &header_cells
            .iter()
            .map(|(s, p)| (s.clone(), p.clone()))
            .collect::<Vec<_>>(),
        true,
    ));
    out_lines.push(border_line('├', '┼', '┤'));
    for row in body_cells {
        out_lines.push(render_row(&row, false));
    }
    out_lines.push(border_line('└', '┴', '┘'));
    out_lines
}

fn find_markdown_link_at_or_after(line: &str, start: usize) -> Option<(usize, usize, &str, &str)> {
    let rest = &line[start..];
    let rel = rest.find('[')?;
    let open = start + rel;
    let after_open = open + 1;

    let close_bracket_rel = line[after_open..].find(']')?;
    let close_bracket = after_open + close_bracket_rel;
    if close_bracket + 1 >= line.len() || &line[close_bracket + 1..close_bracket + 2] != "(" {
        return None;
    }
    let url_start = close_bracket + 2;
    let close_paren_rel = line[url_start..].find(')')?;
    let close_paren = url_start + close_paren_rel;
    if close_paren <= url_start {
        return None;
    }

    let label = &line[after_open..close_bracket];
    let url = line[url_start..close_paren].trim();
    if url.is_empty() {
        return None;
    }
    Some((open, close_paren + 1, label, url))
}

fn find_autolink_url_at_or_after(line: &str, start: usize) -> Option<(usize, usize, &str)> {
    let rest = &line[start..];
    let http_rel = rest.find("http://");
    let https_rel = rest.find("https://");
    let rel = match (http_rel, https_rel) {
        (None, None) => return None,
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (Some(a), Some(b)) => a.min(b),
    };

    let url_start = start + rel;

    // Avoid linking things like "foohttps://...".
    if url_start > 0
        && let Some(prev) = line[..url_start].chars().next_back()
        && (prev.is_ascii_alphanumeric() || prev == '_' || prev == '@')
    {
        return None;
    }

    let mut end = line.len();
    for (off, ch) in line[url_start..].char_indices() {
        if ch.is_whitespace() || ch == '<' || ch == '>' || ch == '(' || ch == ')' {
            end = url_start + off;
            break;
        }
    }

    let mut url_end = end;
    while url_end > url_start {
        let last = line[..url_end].chars().next_back()?;
        if [',', '.', ';', ':', '!', '?'].contains(&last) {
            url_end -= last.len_utf8();
        } else {
            break;
        }
    }

    if url_end <= url_start {
        return None;
    }
    Some((url_start, url_end, &line[url_start..url_end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_bold_and_bullets_strip_markers() {
        let theme = Theme::pro();
        let md = "**Built-in Tools:**\n• **file** - Read files\nUse `/tools <name>` for details.";
        let text = markdown_to_text(md, &theme);

        assert_eq!(text.lines.len(), 3);

        let l0 = &text.lines[0];
        let l0_str: String = l0.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(l0_str, "Built-in Tools:");
        // At least one span should already be bold (adding bold shouldn't change it).
        assert!(
            l0.spans
                .iter()
                .any(|s| s.style.add_modifier(Modifier::BOLD) == s.style)
        );

        let l1 = &text.lines[1];
        let l1_str: String = l1.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(l1_str, "• file - Read files");
        assert!(l1.spans.iter().any(
            |s| s.content.as_ref() == "file" && s.style.add_modifier(Modifier::BOLD) == s.style
        ));

        let l2 = &text.lines[2];
        let l2_str: String = l2.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(l2_str.contains("Use /tools <name>"));
    }

    #[test]
    fn markdown_italic_strips_markers() {
        let theme = Theme::pro();
        let md = "Use *italics* and **bold** and `code`.";
        let text = markdown_to_text(md, &theme);

        let l0 = &text.lines[0];
        let l0_str: String = l0.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(l0_str, "Use italics and bold and code.");

        assert!(l0.spans.iter().any(|s| s.content.as_ref() == "italics"
            && s.style.add_modifier(Modifier::ITALIC) == s.style));
        assert!(l0.spans.iter().any(
            |s| s.content.as_ref() == "bold" && s.style.add_modifier(Modifier::BOLD) == s.style
        ));
        // `code` is rendered without backticks.
        assert!(l0.spans.iter().any(|s| s.content.as_ref() == "code"));
    }

    #[test]
    fn markdown_links_render_label_and_url_and_strip_markers() {
        let theme = Theme::pro();
        let md = "See [Docs](https://example.com) and **bold**.";
        let text = markdown_to_text(md, &theme);

        let l0 = &text.lines[0];
        let l0_str: String = l0.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(l0_str, "See Docs (https://example.com) and bold.");

        // Link label should be underlined.
        assert!(l0.spans.iter().any(|s| {
            s.content.as_ref() == "Docs" && s.style.add_modifier(Modifier::UNDERLINED) == s.style
        }));
    }

    #[test]
    fn markdown_autolinks_bare_urls_without_trailing_punctuation() {
        let theme = Theme::pro();
        let md = "Visit https://example.com.";
        let text = markdown_to_text(md, &theme);

        let l0 = &text.lines[0];
        let l0_str: String = l0.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(l0_str, "Visit https://example.com.");
        assert!(l0.spans.iter().any(|s| {
            s.content.as_ref() == "https://example.com"
                && s.style.add_modifier(Modifier::UNDERLINED) == s.style
        }));
    }

    #[test]
    fn markdown_headings_support_h1_through_h6() {
        let theme = Theme::pro();
        let md = "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6";
        let text = markdown_to_text(md, &theme);
        assert_eq!(text.lines.len(), 6);

        let l0 = &text.lines[0];
        assert_eq!(
            l0.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>(),
            "H1"
        );
        assert!(
            l0.spans
                .iter()
                .any(|s| s.style.add_modifier(Modifier::UNDERLINED) == s.style)
        );

        let l5 = &text.lines[5];
        assert_eq!(
            l5.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>(),
            "H6"
        );
        assert!(
            l5.spans
                .iter()
                .any(|s| s.style.add_modifier(Modifier::DIM) == s.style)
        );
    }

    #[test]
    fn markdown_tables_render_ascii_with_alignment() {
        let theme = Theme::pro();
        let md = "| Name | Value |\n| --- | ---: |\n| a | 1 |\n| long | 22 |";
        let text = markdown_to_text(md, &theme);

        let rendered: Vec<String> = text
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();

        assert_eq!(rendered.len(), 6);
        assert_eq!(rendered[0], "┌──────┬───────┐");
        assert_eq!(rendered[1], "│ Name │ Value │");
        assert_eq!(rendered[2], "├──────┼───────┤");
        assert_eq!(rendered[3], "│ a    │     1 │");
        assert_eq!(rendered[4], "│ long │    22 │");
        assert_eq!(rendered[5], "└──────┴───────┘");
    }
}
