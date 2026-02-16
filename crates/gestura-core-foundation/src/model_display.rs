//! Model name display formatting utilities.
//!
//! Provides human-friendly model name formatting for display in CLI/GUI.

/// Format an OpenAI model ID to a human-readable name.
///
/// # Examples
/// - `gpt-4o` → `GPT-4o`
/// - `gpt-4o-mini` → `GPT-4o Mini`
/// - `gpt-3.5-turbo` → `GPT-3.5 Turbo`
pub fn format_openai_model_name(id: &str) -> String {
    match id {
        "gpt-4o" => "GPT-4o".to_string(),
        "gpt-4o-mini" => "GPT-4o Mini".to_string(),
        "gpt-4-turbo" => "GPT-4 Turbo".to_string(),
        "gpt-4-turbo-preview" => "GPT-4 Turbo Preview".to_string(),
        "gpt-4" => "GPT-4".to_string(),
        "gpt-3.5-turbo" => "GPT-3.5 Turbo".to_string(),
        "o1-preview" => "o1 Preview".to_string(),
        "o1-mini" => "o1 Mini".to_string(),
        "o3-mini" => "o3 Mini".to_string(),
        _ => title_case_kebab(id),
    }
}

/// Format an Anthropic model ID to a human-readable name.
///
/// Preserves the full version identifier (including date suffixes) so that
/// users can distinguish between model snapshots.
///
/// # Examples
/// - `claude-sonnet-4-20250514` → `Claude Sonnet 4 (20250514)`
/// - `claude-opus-4-20250514` → `Claude Opus 4 (20250514)`
/// - `claude-3-7-sonnet-20250219` → `Claude 3.7 Sonnet (20250219)`
/// - `claude-3-5-sonnet-20241022` → `Claude 3.5 Sonnet (20241022)`
/// - `claude-3-5-sonnet-latest` → `Claude 3.5 Sonnet (latest)`
/// - `claude-3-opus-20240229` → `Claude 3 Opus (20240229)`
pub fn format_anthropic_model_name(id: &str) -> String {
    let parts: Vec<&str> = id.split('-').collect();

    // Must start with "claude" and have at least 3 parts.
    if parts.is_empty() || parts[0] != "claude" || parts.len() < 3 {
        return title_case_kebab(id);
    }

    // Detect the naming convention by checking whether parts[1] is numeric.
    //
    // Old format (parts[1] is numeric):
    //   claude-{major}[-{minor}]-{variant}-{date|"latest"}
    //   e.g. claude-3-opus-20240229, claude-3-5-sonnet-20241022, claude-3-7-sonnet-20250219
    //
    // New format (parts[1] is a word):
    //   claude-{variant}-{major}-{date}
    //   e.g. claude-sonnet-4-20250514, claude-opus-4-20250514

    let first_is_numeric = parts[1].chars().all(|c| c.is_ascii_digit());

    if first_is_numeric {
        // Old format: claude-{major}[-{minor}]-{variant}-{suffix}
        // Gather consecutive numeric parts as the version (e.g., "3", "5" → "3.5").
        let mut version_parts: Vec<&str> = Vec::new();
        let mut idx = 1;
        while idx < parts.len() && parts[idx].chars().all(|c| c.is_ascii_digit()) {
            version_parts.push(parts[idx]);
            idx += 1;
        }
        let version = version_parts.join(".");

        // Next part is the variant name (e.g., "sonnet", "opus", "haiku").
        let variant = if idx < parts.len() {
            capitalize_first(parts[idx])
        } else {
            String::new()
        };
        idx += 1;

        // Remaining parts form the suffix (date or "latest").
        let suffix = if idx < parts.len() {
            parts[idx..].join("-")
        } else {
            String::new()
        };

        if suffix.is_empty() {
            format!("Claude {} {}", version, variant).trim().to_string()
        } else {
            format!("Claude {} {} ({})", version, variant, suffix)
                .trim()
                .to_string()
        }
    } else {
        // New format: claude-{variant}-{major}-{suffix}
        let variant = capitalize_first(parts[1]);
        let major = if parts.len() > 2 { parts[2] } else { "" };

        // Remaining parts form the suffix.
        let suffix = if parts.len() > 3 {
            parts[3..].join("-")
        } else {
            String::new()
        };

        if suffix.is_empty() {
            format!("Claude {} {}", variant, major).trim().to_string()
        } else {
            format!("Claude {} {} ({})", variant, major, suffix)
                .trim()
                .to_string()
        }
    }
}

/// Format a Grok model ID to human-readable name.
///
/// Preserves numeric version suffixes (e.g., release dates) in parentheses so
/// that distinct model snapshots remain distinguishable.
///
/// # Examples
/// - `grok-4-0709` → `Grok 4 (0709)`
/// - `grok-3` → `Grok 3`
/// - `grok-3-mini` → `Grok 3 Mini`
/// - `grok-3-mini-fast` → `Grok 3 Mini Fast`
/// - `grok-2-1212` → `Grok 2 (1212)`
/// - `grok-2-vision-1212` → `Grok 2 Vision (1212)`
pub fn format_grok_model_name(id: &str) -> String {
    let parts: Vec<&str> = id.split('-').collect();
    let mut name = String::new();

    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            name.push_str("Grok");
        } else if part.chars().all(|c| c.is_ascii_digit()) {
            if i == 1 {
                // Primary version number (e.g., the "3" in grok-3)
                name.push_str(&format!(" {}", part));
            } else {
                // Secondary numeric part — release/version suffix (e.g., "1212", "0709")
                name.push_str(&format!(" ({})", part));
            }
        } else {
            let formatted = match *part {
                "mini" => "Mini",
                "fast" => "Fast",
                "vision" => "Vision",
                "code" => "Code",
                "beta" => "Beta",
                _ => part,
            };
            name.push_str(&format!(" {}", formatted));
        }
    }

    name.trim().to_string()
}

/// Format a Gemini model ID to a human-readable name.
///
/// # Examples
/// - `gemini-2.0-flash` → `Gemini 2.0 Flash`
/// - `gemini-2.0-flash-lite` → `Gemini 2.0 Flash Lite`
/// - `gemini-1.5-pro` → `Gemini 1.5 Pro`
pub fn format_gemini_model_name(id: &str) -> String {
    let parts: Vec<&str> = id.split('-').collect();
    let mut name = String::new();

    for (i, part) in parts.iter().enumerate() {
        if i == 0 && part.eq_ignore_ascii_case("gemini") {
            name.push_str("Gemini");
        } else {
            let formatted = match *part {
                "pro" => "Pro",
                "flash" => "Flash",
                "lite" => "Lite",
                "ultra" => "Ultra",
                "nano" => "Nano",
                "exp" => "Experimental",
                "latest" => "(Latest)",
                other => {
                    // Numeric version segments (e.g. "2.0", "1.5") are kept as-is.
                    name.push(' ');
                    name.push_str(other);
                    continue;
                }
            };
            name.push(' ');
            name.push_str(formatted);
        }
    }

    name.trim().to_string()
}

/// Format any model name based on provider.
///
/// # Arguments
/// - `provider`: LLM provider name (openai, anthropic, grok, gemini, ollama)
/// - `model_id`: Raw model identifier
pub fn format_model_name(provider: &str, model_id: &str) -> String {
    match provider.to_lowercase().as_str() {
        "openai" => format_openai_model_name(model_id),
        "anthropic" => format_anthropic_model_name(model_id),
        "grok" => format_grok_model_name(model_id),
        "gemini" => format_gemini_model_name(model_id),
        "ollama" => capitalize_first(model_id),
        _ => model_id.to_string(),
    }
}

/// Check if a provider is local (no cost tracking).
pub fn is_local_provider(provider: &str) -> bool {
    matches!(provider.to_lowercase().as_str(), "ollama" | "local")
}

/// Convert kebab-case to Title Case.
fn title_case_kebab(s: &str) -> String {
    s.split('-')
        .map(capitalize_first)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Capitalize the first character of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_format() {
        assert_eq!(format_openai_model_name("gpt-4o"), "GPT-4o");
        assert_eq!(format_openai_model_name("gpt-4o-mini"), "GPT-4o Mini");
    }

    #[test]
    fn test_anthropic_format_old_single_version() {
        // Old format: claude-{major}-{variant}-{date}
        assert_eq!(
            format_anthropic_model_name("claude-3-opus-20240229"),
            "Claude 3 Opus (20240229)"
        );
        assert_eq!(
            format_anthropic_model_name("claude-3-sonnet-20240229"),
            "Claude 3 Sonnet (20240229)"
        );
        assert_eq!(
            format_anthropic_model_name("claude-3-haiku-20240307"),
            "Claude 3 Haiku (20240307)"
        );
    }

    #[test]
    fn test_anthropic_format_old_double_version() {
        // Old format: claude-{major}-{minor}-{variant}-{date}
        assert_eq!(
            format_anthropic_model_name("claude-3-5-sonnet-20241022"),
            "Claude 3.5 Sonnet (20241022)"
        );
        assert_eq!(
            format_anthropic_model_name("claude-3-5-haiku-20241022"),
            "Claude 3.5 Haiku (20241022)"
        );
        assert_eq!(
            format_anthropic_model_name("claude-3-7-sonnet-20250219"),
            "Claude 3.7 Sonnet (20250219)"
        );
    }

    #[test]
    fn test_anthropic_format_new_convention() {
        // New format: claude-{variant}-{major}-{date}
        assert_eq!(
            format_anthropic_model_name("claude-sonnet-4-20250514"),
            "Claude Sonnet 4 (20250514)"
        );
        assert_eq!(
            format_anthropic_model_name("claude-opus-4-20250514"),
            "Claude Opus 4 (20250514)"
        );
    }

    #[test]
    fn test_anthropic_format_latest_alias() {
        assert_eq!(
            format_anthropic_model_name("claude-3-5-sonnet-latest"),
            "Claude 3.5 Sonnet (latest)"
        );
    }

    #[test]
    fn test_grok_format_with_version_suffix() {
        assert_eq!(format_grok_model_name("grok-3"), "Grok 3");
        assert_eq!(format_grok_model_name("grok-3-mini"), "Grok 3 Mini");
        assert_eq!(
            format_grok_model_name("grok-3-mini-fast"),
            "Grok 3 Mini Fast"
        );
        assert_eq!(format_grok_model_name("grok-4-0709"), "Grok 4 (0709)");
        assert_eq!(format_grok_model_name("grok-2-1212"), "Grok 2 (1212)");
        assert_eq!(
            format_grok_model_name("grok-2-vision-1212"),
            "Grok 2 Vision (1212)"
        );
        assert_eq!(format_grok_model_name("grok-beta"), "Grok Beta");
    }

    #[test]
    fn test_gemini_format() {
        assert_eq!(
            format_gemini_model_name("gemini-2.0-flash"),
            "Gemini 2.0 Flash"
        );
        assert_eq!(
            format_gemini_model_name("gemini-2.0-flash-lite"),
            "Gemini 2.0 Flash Lite"
        );
        assert_eq!(format_gemini_model_name("gemini-1.5-pro"), "Gemini 1.5 Pro");
        assert_eq!(
            format_gemini_model_name("gemini-1.5-flash"),
            "Gemini 1.5 Flash"
        );
    }

    #[test]
    fn test_format_model_name_gemini_dispatch() {
        assert_eq!(
            format_model_name("gemini", "gemini-2.0-flash"),
            "Gemini 2.0 Flash"
        );
    }

    #[test]
    fn test_local_provider() {
        assert!(is_local_provider("ollama"));
        assert!(is_local_provider("Ollama"));
        assert!(!is_local_provider("openai"));
    }
}
