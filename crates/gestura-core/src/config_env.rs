//! Environment Variable Configuration Support
//!
//! Provides hierarchical configuration loading with precedence:
//! 1. Environment variables (GESTURA_* prefix)
//! 2. Config file (~/.gestura/config.yaml)
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

/// API key validation result
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeyValidation {
    /// Key is valid
    Valid,
    /// Key is empty
    Empty,
    /// Key is too short
    TooShort { min_length: usize, actual: usize },
    /// Key has invalid format
    InvalidFormat { expected: &'static str },
    /// Key has invalid prefix
    InvalidPrefix { expected: &'static str },
}

impl ApiKeyValidation {
    /// Check if validation passed
    pub fn is_valid(&self) -> bool {
        matches!(self, ApiKeyValidation::Valid)
    }

    /// Get error message if invalid
    pub fn error_message(&self) -> Option<String> {
        match self {
            ApiKeyValidation::Valid => None,
            ApiKeyValidation::Empty => Some("API key is empty".to_string()),
            ApiKeyValidation::TooShort { min_length, actual } => Some(format!(
                "API key is too short (expected at least {} characters, got {})",
                min_length, actual
            )),
            ApiKeyValidation::InvalidFormat { expected } => {
                Some(format!("Invalid API key format. Expected: {}", expected))
            }
            ApiKeyValidation::InvalidPrefix { expected } => {
                Some(format!("Invalid API key prefix. Expected: {}", expected))
            }
        }
    }
}

/// Validate an OpenAI API key
pub fn validate_openai_key(key: &str) -> ApiKeyValidation {
    if key.is_empty() {
        return ApiKeyValidation::Empty;
    }
    if key.len() < 20 {
        return ApiKeyValidation::TooShort {
            min_length: 20,
            actual: key.len(),
        };
    }
    if !key.starts_with("sk-") {
        return ApiKeyValidation::InvalidPrefix { expected: "sk-" };
    }
    ApiKeyValidation::Valid
}

/// Validate an Anthropic API key
pub fn validate_anthropic_key(key: &str) -> ApiKeyValidation {
    if key.is_empty() {
        return ApiKeyValidation::Empty;
    }
    if key.len() < 20 {
        return ApiKeyValidation::TooShort {
            min_length: 20,
            actual: key.len(),
        };
    }
    if !key.starts_with("sk-ant-") {
        return ApiKeyValidation::InvalidPrefix {
            expected: "sk-ant-",
        };
    }
    ApiKeyValidation::Valid
}

/// Validate a Grok API key
pub fn validate_grok_key(key: &str) -> ApiKeyValidation {
    if key.is_empty() {
        return ApiKeyValidation::Empty;
    }
    if key.len() < 20 {
        return ApiKeyValidation::TooShort {
            min_length: 20,
            actual: key.len(),
        };
    }
    if !key.starts_with("xai-") {
        return ApiKeyValidation::InvalidPrefix { expected: "xai-" };
    }
    ApiKeyValidation::Valid
}

/// Validate any API key by provider name
pub fn validate_api_key(provider: &str, key: &str) -> ApiKeyValidation {
    match provider.to_lowercase().as_str() {
        "openai" => validate_openai_key(key),
        "anthropic" => validate_anthropic_key(key),
        "grok" => validate_grok_key(key),
        _ => {
            // Generic validation for unknown providers
            if key.is_empty() {
                ApiKeyValidation::Empty
            } else if key.len() < 10 {
                ApiKeyValidation::TooShort {
                    min_length: 10,
                    actual: key.len(),
                }
            } else {
                ApiKeyValidation::Valid
            }
        }
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

    #[test]
    fn test_validate_openai_key() {
        assert_eq!(validate_openai_key(""), ApiKeyValidation::Empty);
        assert_eq!(
            validate_openai_key("short"),
            ApiKeyValidation::TooShort {
                min_length: 20,
                actual: 5
            }
        );
        assert_eq!(
            validate_openai_key("invalid-key-format-12345"),
            ApiKeyValidation::InvalidPrefix { expected: "sk-" }
        );
        assert_eq!(
            validate_openai_key("sk-proj-1234567890abcdefgh"),
            ApiKeyValidation::Valid
        );
    }

    #[test]
    fn test_validate_anthropic_key() {
        assert_eq!(validate_anthropic_key(""), ApiKeyValidation::Empty);
        assert_eq!(
            validate_anthropic_key("sk-12345678901234567890"),
            ApiKeyValidation::InvalidPrefix {
                expected: "sk-ant-"
            }
        );
        assert_eq!(
            validate_anthropic_key("sk-ant-api03-1234567890abcdef"),
            ApiKeyValidation::Valid
        );
    }

    #[test]
    fn test_validate_grok_key() {
        assert_eq!(validate_grok_key(""), ApiKeyValidation::Empty);
        assert_eq!(
            validate_grok_key("sk-12345678901234567890"),
            ApiKeyValidation::InvalidPrefix { expected: "xai-" }
        );
        assert_eq!(
            validate_grok_key("xai-1234567890abcdefghij"),
            ApiKeyValidation::Valid
        );
    }

    #[test]
    fn test_validate_api_key_by_provider() {
        assert!(validate_api_key("openai", "sk-proj-12345678901234567890").is_valid());
        assert!(validate_api_key("anthropic", "sk-ant-12345678901234567890").is_valid());
        assert!(validate_api_key("grok", "xai-12345678901234567890").is_valid());
        assert!(validate_api_key("unknown", "some-valid-key-here").is_valid());
        assert!(!validate_api_key("unknown", "short").is_valid());
    }

    #[test]
    fn test_api_key_validation_error_messages() {
        assert!(ApiKeyValidation::Valid.error_message().is_none());
        assert!(ApiKeyValidation::Empty.error_message().is_some());
        assert!(
            ApiKeyValidation::TooShort {
                min_length: 20,
                actual: 5
            }
            .error_message()
            .unwrap()
            .contains("too short")
        );
        assert!(
            ApiKeyValidation::InvalidPrefix { expected: "sk-" }
                .error_message()
                .unwrap()
                .contains("prefix")
        );
    }
}
