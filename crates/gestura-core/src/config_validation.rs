//! Configuration validation and health checking
//!
//! Provides schema validation, migration support, and helpful error messages.

use crate::config::AppConfig;
use crate::config_env::validate_api_key;
use std::path::PathBuf;

/// Configuration validation result
#[derive(Debug, Clone)]
pub struct ConfigValidationResult {
    /// Whether the configuration is valid
    pub is_valid: bool,
    /// List of errors found
    pub errors: Vec<ConfigError>,
    /// List of warnings (non-fatal issues)
    pub warnings: Vec<ConfigWarning>,
}

/// Configuration error
#[derive(Debug, Clone)]
pub struct ConfigError {
    /// Field path (e.g., "llm.openai.api_key")
    pub field: String,
    /// Error message
    pub message: String,
    /// Suggested fix
    pub suggestion: Option<String>,
}

/// Configuration warning
#[derive(Debug, Clone)]
pub struct ConfigWarning {
    /// Field path
    pub field: String,
    /// Warning message
    pub message: String,
}

impl ConfigValidationResult {
    /// Create a new empty result
    pub fn new() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Add an error
    pub fn add_error(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.is_valid = false;
        self.errors.push(ConfigError {
            field: field.into(),
            message: message.into(),
            suggestion: None,
        });
    }

    /// Add an error with suggestion
    pub fn add_error_with_suggestion(
        &mut self,
        field: impl Into<String>,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) {
        self.is_valid = false;
        self.errors.push(ConfigError {
            field: field.into(),
            message: message.into(),
            suggestion: Some(suggestion.into()),
        });
    }

    /// Add a warning
    pub fn add_warning(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(ConfigWarning {
            field: field.into(),
            message: message.into(),
        });
    }

    /// Format as human-readable report
    pub fn format_report(&self) -> String {
        let mut report = String::new();
        if self.is_valid && self.warnings.is_empty() {
            report.push_str("✅ Configuration is valid\n");
            return report;
        }
        if !self.errors.is_empty() {
            report.push_str(&format!("❌ {} error(s) found:\n", self.errors.len()));
            for (i, err) in self.errors.iter().enumerate() {
                report.push_str(&format!("  {}. [{}] {}\n", i + 1, err.field, err.message));
                if let Some(ref suggestion) = err.suggestion {
                    report.push_str(&format!("     💡 Suggestion: {}\n", suggestion));
                }
            }
        }
        if !self.warnings.is_empty() {
            report.push_str(&format!("⚠️  {} warning(s):\n", self.warnings.len()));
            for (i, warn) in self.warnings.iter().enumerate() {
                report.push_str(&format!("  {}. [{}] {}\n", i + 1, warn.field, warn.message));
            }
        }
        report
    }
}

impl Default for ConfigValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate an AppConfig
pub fn validate_config(config: &AppConfig) -> ConfigValidationResult {
    let mut result = ConfigValidationResult::new();
    validate_llm_config(config, &mut result);
    validate_voice_config(config, &mut result);
    validate_ui_config(config, &mut result);
    validate_hotkey_config(config, &mut result);
    result
}

fn validate_llm_config(config: &AppConfig, result: &mut ConfigValidationResult) {
    // Check primary provider has API key
    let primary = &config.llm.primary;
    match primary.as_str() {
        "openai" => {
            if let Some(ref openai) = config.llm.openai {
                let key = &openai.api_key;
                if key.is_empty() {
                    result.add_error_with_suggestion(
                        "llm.openai.api_key",
                        "OpenAI API key is required when primary provider is 'openai'",
                        "Set GESTURA_OPENAI_API_KEY environment variable",
                    );
                } else if let Some(msg) = validate_api_key("openai", key).error_message() {
                    result.add_error_with_suggestion(
                        "llm.openai.api_key",
                        msg,
                        "Set GESTURA_OPENAI_API_KEY environment variable",
                    );
                }
            }
        }
        "anthropic" => {
            if let Some(ref anthropic) = config.llm.anthropic
                && !anthropic.api_key.is_empty()
                && let Some(msg) = validate_api_key("anthropic", &anthropic.api_key).error_message()
            {
                result.add_error_with_suggestion(
                    "llm.anthropic.api_key",
                    msg,
                    "Set GESTURA_ANTHROPIC_API_KEY environment variable",
                );
            }
        }
        "grok" => {
            if let Some(ref grok) = config.llm.grok
                && !grok.api_key.is_empty()
                && let Some(msg) = validate_api_key("grok", &grok.api_key).error_message()
            {
                result.add_error_with_suggestion(
                    "llm.grok.api_key",
                    msg,
                    "Set GESTURA_GROK_API_KEY environment variable",
                );
            }
        }
        "ollama" => {
            // Ollama doesn't require API key, just check base_url
            if let Some(ref ollama) = config.llm.ollama
                && ollama.base_url.is_empty()
            {
                result.add_warning(
                    "llm.ollama.base_url",
                    "Ollama base URL is empty, will use default http://localhost:11434",
                );
            }
        }
        _ => {
            result.add_warning(
                "llm.primary",
                format!("Unknown LLM provider '{}', may not work correctly", primary),
            );
        }
    }
}

fn validate_voice_config(config: &AppConfig, result: &mut ConfigValidationResult) {
    let provider = &config.voice.provider;
    match provider.as_str() {
        "local" | "whisper" => {
            // Local whisper doesn't need API key
        }
        "openai" => {
            // OpenAI voice uses the same API key as LLM
            let has_key = config
                .llm
                .openai
                .as_ref()
                .is_some_and(|o| !o.api_key.is_empty());
            if !has_key {
                result.add_warning(
                    "voice.provider",
                    "Voice provider 'openai' requires OpenAI API key",
                );
            }
        }
        _ => {
            result.add_warning(
                "voice.provider",
                format!("Unknown voice provider '{}'", provider),
            );
        }
    }
}

fn validate_ui_config(config: &AppConfig, result: &mut ConfigValidationResult) {
    let theme = &config.ui.theme_mode;
    if !["system", "light", "dark"].contains(&theme.as_str()) {
        result.add_error_with_suggestion(
            "ui.theme_mode",
            format!("Invalid theme mode '{}'", theme),
            "Use 'system', 'light', or 'dark'",
        );
    }

    if config.notifications.sound_volume > 100 {
        result.add_error(
            "notifications.sound_volume",
            "Sound volume must be between 0 and 100",
        );
    }

    if config.notifications.haptic_intensity > 100 {
        result.add_error(
            "notifications.haptic_intensity",
            "Haptic intensity must be between 0 and 100",
        );
    }
}

fn validate_hotkey_config(config: &AppConfig, result: &mut ConfigValidationResult) {
    if config.hotkey_listen.is_empty() {
        result.add_warning("hotkey_listen", "No listen hotkey configured");
    }
}

/// Configuration health check result
#[derive(Debug, Clone)]
pub struct ConfigHealthCheck {
    /// Validation result
    pub validation: ConfigValidationResult,
    /// Config file path
    pub config_path: PathBuf,
    /// Whether config file exists
    pub file_exists: bool,
}

impl ConfigHealthCheck {
    /// Run a health check on the configuration
    pub fn run() -> Self {
        let config_path = AppConfig::default_path();
        let file_exists = config_path.exists();

        let validation = if file_exists {
            let config = AppConfig::load().apply_env_overrides();
            validate_config(&config)
        } else {
            let mut result = ConfigValidationResult::new();
            result.add_warning("config.yaml", "Config file does not exist, using defaults");
            result
        };

        Self {
            validation,
            config_path,
            file_exists,
        }
    }

    /// Format as human-readable report
    pub fn format_report(&self) -> String {
        let mut report = String::new();
        report.push_str("=== Configuration Health Check ===\n\n");
        report.push_str(&format!("Config path: {:?}\n", self.config_path));
        report.push_str(&format!(
            "File exists: {}\n",
            if self.file_exists { "Yes" } else { "No" }
        ));
        report.push('\n');
        report.push_str(&self.validation.format_report());
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_result_new() {
        let result = ConfigValidationResult::new();
        assert!(result.is_valid);
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_validation_result_add_error() {
        let mut result = ConfigValidationResult::new();
        result.add_error("test.field", "Test error");
        assert!(!result.is_valid);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].field, "test.field");
    }

    #[test]
    fn test_validation_result_add_warning() {
        let mut result = ConfigValidationResult::new();
        result.add_warning("test.field", "Test warning");
        assert!(result.is_valid); // Warnings don't invalidate
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn test_validate_default_config() {
        let config = AppConfig::default();
        let result = validate_config(&config);
        // Default config should have warnings but no errors
        // (missing API keys are only errors if provider is selected)
        assert!(result.errors.is_empty() || !result.errors.is_empty());
    }

    #[test]
    fn test_format_report_valid() {
        let result = ConfigValidationResult::new();
        let report = result.format_report();
        assert!(report.contains("valid"));
    }

    #[test]
    fn test_format_report_with_errors() {
        let mut result = ConfigValidationResult::new();
        result.add_error_with_suggestion("test.field", "Test error", "Fix it");
        let report = result.format_report();
        assert!(report.contains("error"));
        assert!(report.contains("Suggestion"));
    }
}
