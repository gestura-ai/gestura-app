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
//! - Headings starting with `## ` or `# `
//! - Fenced code blocks using triple backticks (fence lines are hidden)

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
};

use super::app::Theme;

/// Convert a small Markdown subset to styled ratatui `Text`.
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

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_fence = false;

    for raw in markdown.lines() {
        let line = raw.trim_end_matches(['\r', '\n']);

        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }

        if in_fence {
            lines.push(Line::from(Span::styled(line.to_string(), code)));
            continue;
        }

        if trimmed.is_empty() {
            lines.push(Line::from(""));
            continue;
        }

        if let Some(rest) = line.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(rest.to_string(), heading)));
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(rest.to_string(), heading)));
            continue;
        }

        if let Some(rest) = line.strip_prefix("• ") {
            let mut spans = Vec::new();
            spans.push(Span::styled("• ".to_string(), bullet));
            spans.extend(parse_inline(rest, base, code));
            lines.push(Line::from(spans));
            continue;
        }
        if let Some(rest) = line.strip_prefix("- ") {
            let mut spans = Vec::new();
            spans.push(Span::styled("• ".to_string(), bullet));
            spans.extend(parse_inline(rest, base, code));
            lines.push(Line::from(spans));
            continue;
        }

        lines.push(Line::from(parse_inline(line, base, code)));
    }

    Text::from(lines)
}

fn parse_inline(line: &str, base: Style, code_style: Style) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();

    let mut bold = false;
    let mut italic = false;
    let mut code = false;

    let mut i = 0usize;
    while i < line.len() {
        let rest = &line[i..];
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
}
