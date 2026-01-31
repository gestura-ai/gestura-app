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
/// # Examples
/// - `claude-sonnet-4-20250514` → `Claude Sonnet 4`
/// - `claude-3-5-sonnet-20241022` → `Claude 3.5 Sonnet`
/// - `claude-3-opus-20240229` → `Claude 3 Opus`
pub fn format_anthropic_model_name(id: &str) -> String {
    match id {
        "claude-sonnet-4-20250514" => "Claude Sonnet 4".to_string(),
        "claude-3-5-sonnet-20241022" => "Claude 3.5 Sonnet".to_string(),
        "claude-3-5-sonnet-latest" => "Claude 3.5 Sonnet".to_string(),
        "claude-3-opus-20240229" => "Claude 3 Opus".to_string(),
        "claude-3-sonnet-20240229" => "Claude 3 Sonnet".to_string(),
        "claude-3-haiku-20240307" => "Claude 3 Haiku".to_string(),
        _ => {
            // Try to parse: claude-{version}-{variant}-{date}
            let parts: Vec<&str> = id.split('-').collect();
            if parts.len() >= 3 && parts[0] == "claude" {
                let version = parts[1];
                let variant = capitalize_first(parts[2]);
                format!("Claude {} {}", version, variant)
            } else {
                title_case_kebab(id)
            }
        }
    }
}

/// Format a Grok model ID to human-readable name.
///
/// # Examples
/// - `grok-4-0709` → `Grok 4`
/// - `grok-3-mini` → `Grok 3 Mini`
pub fn format_grok_model_name(id: &str) -> String {
    let parts: Vec<&str> = id.split('-').collect();
    let mut name = String::new();

    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            name.push_str("Grok");
        } else if part.chars().all(|c| c.is_numeric()) {
            if i == 1 {
                name.push_str(&format!(" {}", part));
            }
            // Skip date-like parts (4+ digits)
        } else {
            let formatted = match *part {
                "mini" => "Mini",
                "fast" => "Fast",
                "vision" => "Vision",
                "code" => "Code",
                _ => part,
            };
            name.push_str(&format!(" {}", formatted));
        }
    }

    name.trim().to_string()
}

/// Format any model name based on provider.
///
/// # Arguments
/// - `provider`: LLM provider name (openai, anthropic, grok, ollama)
/// - `model_id`: Raw model identifier
pub fn format_model_name(provider: &str, model_id: &str) -> String {
    match provider.to_lowercase().as_str() {
        "openai" => format_openai_model_name(model_id),
        "anthropic" => format_anthropic_model_name(model_id),
        "grok" => format_grok_model_name(model_id),
        "ollama" => capitalize_first(model_id),
        _ => model_id.to_string(),
    }
}

/// Check if a provider is local (no cost tracking).
pub fn is_local_provider(provider: &str) -> bool {
    matches!(
        provider.to_lowercase().as_str(),
        "ollama" | "local" | "echo"
    )
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
    fn test_anthropic_format() {
        assert_eq!(
            format_anthropic_model_name("claude-3-5-sonnet-20241022"),
            "Claude 3.5 Sonnet"
        );
    }

    #[test]
    fn test_local_provider() {
        assert!(is_local_provider("ollama"));
        assert!(is_local_provider("Ollama"));
        assert!(!is_local_provider("openai"));
    }
}

