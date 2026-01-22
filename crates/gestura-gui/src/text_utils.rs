//! Small shared text utilities.

/// Returns a UTF-8 safe truncated string with an ellipsis.
///
/// This avoids panics from slicing strings on non-char boundaries.
///
/// ## Parameters
/// - `s`: Input string.
/// - `max_chars`: Maximum number of characters to keep before appending an ellipsis.
///
/// ## Returns
/// A string containing at most `max_chars` characters (plus a trailing ellipsis if truncated).
pub fn truncate_utf8(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('\u{2026}');
            return out;
        }
        out.push(ch);
    }
    out
}
