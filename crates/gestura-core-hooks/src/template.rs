//! Tiny, dependency-free template rendering for hooks.
//!
//! The template syntax is `{{key}}`. Keys are trimmed and matched exactly.
//! Unknown keys are replaced with an empty string.

use std::collections::HashMap;

/// Template variables map used for hook command rendering.
pub type TemplateVars = HashMap<String, String>;

/// Render `template` by replacing `{{key}}` placeholders with values in `vars`.
///
/// This is intentionally simple and avoids complex templating logic.
pub fn render_template(template: &str, vars: &TemplateVars) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // Find closing braces.
            let mut j = i + 2;
            while j + 1 < bytes.len() {
                if bytes[j] == b'}' && bytes[j + 1] == b'}' {
                    break;
                }
                j += 1;
            }

            if j + 1 < bytes.len() && bytes[j] == b'}' && bytes[j + 1] == b'}' {
                let key = template[i + 2..j].trim();
                if let Some(v) = vars.get(key) {
                    out.push_str(v);
                }
                i = j + 2;
                continue;
            }
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_template_replaces_known_and_blanks_unknown() {
        let mut vars = TemplateVars::new();
        vars.insert("tool".to_string(), "git".to_string());

        let rendered = render_template("run {{tool}} {{missing}}", &vars);
        assert_eq!(rendered, "run git ");
    }
}
