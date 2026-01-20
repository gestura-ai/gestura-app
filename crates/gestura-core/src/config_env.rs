//! Environment Variable Configuration Support
//!
//! Provides hierarchical configuration loading with precedence:
//! 1. Environment variables (GESTURA_* prefix)
//! 2. Config file (~/.gestura/config.json)
//! 3. Default values
//!
//! All environment variables use the GESTURA_ prefix and snake_case naming.

use std::env;

/// Environment variable prefix for all Gestura configuration
pub const ENV_PREFIX: &str = "GESTURA_";

/// Get an environment variable with the GESTURA_ prefix
pub fn get_env(key: &str) -> Option<String> {
    let env_key = format!("{}{}", ENV_PREFIX, key.to_uppercase());
    env::var(&env_key).ok()
}

/// Get an environment variable as a boolean
pub fn get_env_bool(key: &str) -> Option<bool> {
    get_env(key).map(|v| {
        matches!(
            v.to_lowercase().as_str(),
            "true" | "1" | "yes" | "on" | "enabled"
        )
    })
}

/// Get an environment variable as a u32
pub fn get_env_u32(key: &str) -> Option<u32> {
    get_env(key).and_then(|v| v.parse().ok())
}

/// Get an environment variable as a u64
pub fn get_env_u64(key: &str) -> Option<u64> {
    get_env(key).and_then(|v| v.parse().ok())
}

/// Get an environment variable as a usize
pub fn get_env_usize(key: &str) -> Option<usize> {
    get_env(key).and_then(|v| v.parse().ok())
}

/// Environment variable mappings for AppConfig fields
///
/// Format: (env_var_suffix, config_path, description)
pub const ENV_MAPPINGS: &[(&str, &str, &str)] = &[
    // Core settings
    (
        "HOTKEY_LISTEN",
        "hotkey_listen",
        "Global hotkey to toggle the app",
    ),
    (
        "GRACE_PERIOD_SECS",
        "grace_period_secs",
        "Agent shutdown grace period in seconds",
    ),
    ("NATS_URL", "nats_url", "NATS server URL for messaging"),
    // LLM settings
    (
        "LLM_PRIMARY",
        "llm.primary",
        "Primary LLM provider (openai, anthropic, grok, ollama)",
    ),
    ("LLM_FALLBACK", "llm.fallback", "Fallback LLM provider"),
    ("OPENAI_API_KEY", "llm.openai.api_key", "OpenAI API key"),
    (
        "OPENAI_BASE_URL",
        "llm.openai.base_url",
        "OpenAI API base URL",
    ),
    ("OPENAI_MODEL", "llm.openai.model", "OpenAI model name"),
    (
        "ANTHROPIC_API_KEY",
        "llm.anthropic.api_key",
        "Anthropic API key",
    ),
    (
        "ANTHROPIC_BASE_URL",
        "llm.anthropic.base_url",
        "Anthropic API base URL",
    ),
    (
        "ANTHROPIC_MODEL",
        "llm.anthropic.model",
        "Anthropic model name",
    ),
    ("GROK_API_KEY", "llm.grok.api_key", "Grok API key"),
    ("GROK_BASE_URL", "llm.grok.base_url", "Grok API base URL"),
    ("GROK_MODEL", "llm.grok.model", "Grok model name"),
    (
        "OLLAMA_BASE_URL",
        "llm.ollama.base_url",
        "Ollama server URL",
    ),
    ("OLLAMA_MODEL", "llm.ollama.model", "Ollama model name"),
    // Voice settings
    (
        "VOICE_PROVIDER",
        "voice.provider",
        "Voice provider (local, openai, none)",
    ),
    (
        "VOICE_LOCAL_MODEL_PATH",
        "voice.local_model_path",
        "Path to local Whisper model",
    ),
    (
        "VOICE_OPENAI_API_KEY",
        "voice.openai_api_key",
        "OpenAI API key for voice",
    ),
    (
        "VOICE_OPENAI_MODEL",
        "voice.openai_model",
        "OpenAI voice model",
    ),
    (
        "VOICE_AUDIO_DEVICE",
        "voice.audio_device",
        "Audio input device name",
    ),
    // UI settings
    (
        "UI_THEME_MODE",
        "ui.theme_mode",
        "Theme mode (system, light, dark)",
    ),
    ("UI_ACCENT", "ui.accent", "Accent color"),
    // Developer settings
    (
        "DEVELOPER_MODE",
        "developer.developer_mode",
        "Enable developer mode",
    ),
    (
        "ENABLE_SIMULATORS",
        "developer.enable_simulators",
        "Enable device simulators",
    ),
    (
        "VERBOSE_BLE_LOGGING",
        "developer.verbose_ble_logging",
        "Enable verbose BLE logging",
    ),
    // Web search settings
    (
        "WEB_SEARCH_PROVIDER",
        "web_search.provider",
        "Web search provider",
    ),
    ("SERPAPI_KEY", "web_search.serpapi_key", "SerpAPI key"),
    (
        "BRAVE_SEARCH_KEY",
        "web_search.brave_key",
        "Brave Search API key",
    ),
];

/// Check if a key is a secret (should be redacted in logs)
pub fn is_secret_key(key: &str) -> bool {
    let key_lower = key.to_lowercase();
    key_lower.contains("api_key")
        || key_lower.contains("secret")
        || key_lower.contains("password")
        || key_lower.contains("token")
        || key_lower.ends_with("_key")
}

/// Redact a secret value for logging
pub fn redact_secret(value: &str) -> String {
    if value.len() <= 8 {
        "***".to_string()
    } else {
        format!("{}...{}", &value[..4], &value[value.len() - 4..])
    }
}

/// Get all environment variables that are set
///
/// Returns a list of (env_var_suffix, display_value, is_secret) tuples
pub fn get_set_env_vars() -> Vec<(String, String, bool)> {
    ENV_MAPPINGS
        .iter()
        .filter_map(|(suffix, _path, _desc)| {
            get_env(suffix).map(|value| {
                let secret = is_secret_key(suffix);
                let display_value = if secret { redact_secret(&value) } else { value };
                (suffix.to_string(), display_value, secret)
            })
        })
        .collect()
}

/// Print documentation for all environment variables
pub fn print_env_docs() {
    println!("Gestura Environment Variables");
    println!("==============================");
    println!();
    println!("All environment variables use the {} prefix.", ENV_PREFIX);
    println!();

    for (suffix, path, desc) in ENV_MAPPINGS {
        let full_name = format!("{}{}", ENV_PREFIX, suffix);
        let is_secret = is_secret_key(suffix);
        let secret_note = if is_secret { " [SECRET]" } else { "" };
        println!("  {}{}", full_name, secret_note);
        println!("    Config path: {}", path);
        println!("    {}", desc);
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_prefix() {
        assert_eq!(ENV_PREFIX, "GESTURA_");
    }

    #[test]
    fn test_is_secret_key() {
        assert!(is_secret_key("OPENAI_API_KEY"));
        assert!(is_secret_key("api_key"));
        assert!(is_secret_key("SECRET_TOKEN"));
        assert!(is_secret_key("password"));
        assert!(is_secret_key("SERPAPI_KEY"));
        assert!(!is_secret_key("LLM_PRIMARY"));
        assert!(!is_secret_key("HOTKEY_LISTEN"));
        assert!(!is_secret_key("UI_THEME_MODE"));
    }

    #[test]
    fn test_redact_secret_short() {
        assert_eq!(redact_secret("abc"), "***");
        assert_eq!(redact_secret("12345678"), "***");
    }

    #[test]
    fn test_redact_secret_long() {
        let result = redact_secret("sk-1234567890abcdef");
        assert!(result.starts_with("sk-1"));
        assert!(result.ends_with("cdef"));
        assert!(result.contains("..."));
    }

    #[test]
    fn test_get_env_bool_parsing() {
        // Test the parsing logic directly
        let parse_bool = |v: &str| -> bool {
            matches!(
                v.to_lowercase().as_str(),
                "true" | "1" | "yes" | "on" | "enabled"
            )
        };

        assert!(parse_bool("true"));
        assert!(parse_bool("TRUE"));
        assert!(parse_bool("1"));
        assert!(parse_bool("yes"));
        assert!(parse_bool("on"));
        assert!(parse_bool("enabled"));
        assert!(!parse_bool("false"));
        assert!(!parse_bool("0"));
        assert!(!parse_bool("no"));
        assert!(!parse_bool("off"));
    }

    #[test]
    fn test_env_mappings_has_core_settings() {
        // Should have at least core settings
        assert!(ENV_MAPPINGS.iter().any(|(k, _, _)| *k == "HOTKEY_LISTEN"));
        assert!(ENV_MAPPINGS.iter().any(|(k, _, _)| *k == "LLM_PRIMARY"));
        assert!(ENV_MAPPINGS.iter().any(|(k, _, _)| *k == "OPENAI_API_KEY"));
        assert!(
            ENV_MAPPINGS
                .iter()
                .any(|(k, _, _)| *k == "ANTHROPIC_API_KEY")
        );
        assert!(ENV_MAPPINGS.iter().any(|(k, _, _)| *k == "VOICE_PROVIDER"));
    }

    #[test]
    fn test_env_mappings_have_descriptions() {
        for (suffix, path, desc) in ENV_MAPPINGS {
            assert!(!suffix.is_empty(), "Suffix should not be empty");
            assert!(!path.is_empty(), "Path should not be empty for {}", suffix);
            assert!(
                !desc.is_empty(),
                "Description should not be empty for {}",
                suffix
            );
        }
    }
}
