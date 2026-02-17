//! Render a small Markdown subset to ANSI-styled terminal text.
//!
//! This is used for non-TUI output (e.g. `gestura agent --basic`) so tool/capability
//! summaries emitted by `gestura-core` don't show raw Markdown markers.

use console::style;

/// Convert a small Markdown subset to an ANSI-styled string.
///
/// Supported:
/// - Headings: `# `, `## `, `### `
/// - Bullets: `- ` and `• `
/// - Inline: `**bold**`, `*italic*` (mapped to underline), `` `code` ``
/// - Fenced blocks: ``` (fence lines removed; content rendered dim)
pub(crate) fn markdown_to_ansi(markdown: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;

    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }

        if !out.is_empty() {
            out.push('\n');
        }

        if in_fence {
            out.push_str(&style(line).dim().to_string());
            continue;
        }

        let trimmed = line.trim_start();

        // Headings.
        if let Some(rest) = trimmed
            .strip_prefix("### ")
            .or_else(|| trimmed.strip_prefix("## "))
            .or_else(|| trimmed.strip_prefix("# "))
        {
            out.push_str(&style(render_inline(rest)).bold().underlined().to_string());
            continue;
        }

        // Bullets.
        if let Some(rest) = trimmed
            .strip_prefix("• ")
            .or_else(|| trimmed.strip_prefix("- "))
        {
            out.push_str(&style("• ").dim().to_string());
            out.push_str(&render_inline(rest));
            continue;
        }

        out.push_str(&render_inline(trimmed));
    }

    out
}

fn render_inline(input: &str) -> String {
    let mut out = String::new();

    let mut bold = false;
    let mut italic = false;
    let mut code = false;

    let mut buf = String::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '`' => {
                flush_inline(&mut out, &mut buf, bold, italic, code);
                code = !code;
            }
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    flush_inline(&mut out, &mut buf, bold, italic, code);
                    bold = !bold;
                } else {
                    flush_inline(&mut out, &mut buf, bold, italic, code);
                    italic = !italic;
                }
            }
            _ => buf.push(ch),
        }
    }

    flush_inline(&mut out, &mut buf, bold, italic, code);
    out
}

fn flush_inline(out: &mut String, buf: &mut String, bold: bool, italic: bool, code: bool) {
    if buf.is_empty() {
        return;
    }

    let text = std::mem::take(buf);
    let mut s = style(text);

    if code {
        // Keep code readable and copy/paste-friendly; avoid backgrounds.
        s = s.cyan();
    }
    if bold && !code {
        s = s.bold();
    }
    if italic && !code {
        // `console` doesn't guarantee italic on all terminals; underline is widely supported.
        s = s.underlined();
    }

    out.push_str(&s.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use console::strip_ansi_codes;

    fn plain(s: &str) -> String {
        strip_ansi_codes(s).to_string()
    }

    #[test]
    fn strips_markers_and_normalizes_bullets() {
        let md = "## Built-in Tools:\n- **file** - Read `path`.";
        let rendered = markdown_to_ansi(md);
        let p = plain(&rendered);

        assert!(p.contains("Built-in Tools:"));
        assert!(p.contains("• file - Read path."));
        assert!(!p.contains("**"));
        assert!(!p.contains("`"));
        assert!(!p.contains("##"));
    }

    #[test]
    fn removes_fence_lines_but_preserves_code() {
        let md = "Example:\n```bash\necho hi\n```\nDone";
        let rendered = markdown_to_ansi(md);
        let p = plain(&rendered);

        assert_eq!(p, "Example:\necho hi\nDone");
        assert!(!p.contains("```"));
    }
}
