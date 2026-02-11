//! Session-scoped LLM override resolution helpers.
//!
//! This module centralizes the business logic for applying a per-session provider/model
//! override (see [`crate::chat_sessions::SessionLlmConfig`]) to an in-memory [`crate::config::AppConfig`].
//!
//! Goals:
//! - Keep GUI/CLI thin (they provide session data + platform-specific secret lookup).
//! - Ensure provider/model precedence and compatibility checks are consistent.

use crate::chat_sessions::SessionLlmConfig;
use crate::config::{
    AnthropicConfig, AppConfig, GeminiConfig, GrokConfig, OllamaConfig, OpenAiConfig,
};
use crate::config_env;
use crate::llm_validation;

/// The effective provider/model after applying session overrides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveLlmConfig {
    /// Effective provider id (e.g. `"openai"`, `"anthropic"`, `"ollama"`).
    pub provider: String,
    /// Effective model id for the provider.
    pub model: String,
}

/// Apply CLI-provided provider and/or model overrides to an in-memory config.
///
/// This is a convenience wrapper around [`apply_session_llm_overrides`] intended for
/// thin adapters (CLI basic mode, CLI TUI) so they don't re-implement precedence,
/// provider/model compatibility checks, or provider-config creation.
///
/// The inputs map directly to CLI flags:
/// - `provider_arg`: `--provider <provider>` style argument (provider id)
/// - `model_arg`: `--model <model>` style argument (model id)
///
/// This function does **not** persist any changes to disk.
pub fn apply_cli_llm_overrides(
    cfg: &mut AppConfig,
    provider_arg: Option<&str>,
    model_arg: Option<&str>,
) -> EffectiveLlmConfig {
    let provider = provider_arg.map(str::trim).filter(|s| !s.is_empty());
    let model = model_arg.map(str::trim).filter(|s| !s.is_empty());

    let session = SessionLlmConfig {
        provider: provider.map(|s| s.to_string()),
        model: model.map(|s| s.to_string()),
    };

    apply_cli_session_llm_overrides(cfg, Some(&session))
}

/// Apply a CLI `--model` argument to the config.
///
/// The CLI supports either:
/// - `"provider:model"` (e.g. `"openai:gpt-4o"`) to override both provider and model
/// - `"model"` (e.g. `"claude-3-5-sonnet-20241022"`) to override the model for the
///   currently selected provider
///
/// This function does **not** persist any changes to disk.
pub fn apply_cli_model_arg_overrides(
    cfg: &mut AppConfig,
    model_arg: Option<&str>,
) -> EffectiveLlmConfig {
    let session = model_arg.and_then(parse_cli_model_arg);
    apply_cli_session_llm_overrides(cfg, session.as_ref())
}

/// Apply a CLI `--provider` argument to the config.
///
/// This function does **not** persist any changes to disk.
pub fn apply_cli_provider_arg_override(
    cfg: &mut AppConfig,
    provider_arg: Option<&str>,
) -> EffectiveLlmConfig {
    apply_cli_llm_overrides(cfg, provider_arg, None)
}

/// Apply session-scoped LLM overrides to an in-memory config and return the effective provider/model.
///
/// This function does **not** persist any changes to disk.
///
/// Behavior:
/// - If the session overrides the provider, set `cfg.llm.primary`.
/// - If the session overrides the model, apply it to the active provider's config.
/// - If the model override is obviously incompatible with the provider, ignore it and fall back.
///
/// `api_key_lookup` is adapter-supplied (GUI keychain, CLI env, etc.) and is only used when
/// we need to create a provider config to attach a model override.
pub fn apply_session_llm_overrides(
    cfg: &mut AppConfig,
    session_llm: Option<&SessionLlmConfig>,
    api_key_lookup: impl Fn(&str) -> Option<String>,
) -> EffectiveLlmConfig {
    if let Some(session_llm) = session_llm {
        if let Some(provider) = session_llm.provider.as_deref().map(str::trim)
            && !provider.is_empty()
        {
            cfg.llm.primary = provider.to_string();
        }

        if let Some(model) = session_llm.model.as_deref().map(str::trim)
            && !model.is_empty()
        {
            if !llm_validation::is_model_compatible_with_provider(&cfg.llm.primary, model) {
                tracing::warn!(
                    provider = %cfg.llm.primary,
                    model = %model,
                    "Ignoring incompatible session-scoped LLM model override"
                );
            } else {
                apply_model_override(cfg, model, &api_key_lookup);
            }
        }
    }

    let provider = cfg.llm.primary.clone();
    let model = get_model_for_provider(cfg, &provider).unwrap_or_default();
    EffectiveLlmConfig { provider, model }
}

/// Parse the CLI `--model` argument into a session-style provider/model override.
///
/// Returns `None` if the argument is empty/whitespace.
fn parse_cli_model_arg(model_arg: &str) -> Option<SessionLlmConfig> {
    let arg = model_arg.trim();
    if arg.is_empty() {
        return None;
    }

    if let Some((provider, model)) = arg.split_once(':') {
        let provider = provider.trim();
        let model = model.trim();
        Some(SessionLlmConfig {
            provider: (!provider.is_empty()).then(|| provider.to_string()),
            model: (!model.is_empty()).then(|| model.to_string()),
        })
    } else {
        Some(SessionLlmConfig {
            provider: None,
            model: Some(arg.to_string()),
        })
    }
}

/// Parse a CLI-style model selector (e.g. `"provider:model"` or `"model"`) into a
/// [`SessionLlmConfig`].
///
/// This is a small, shared helper for thin adapters (CLI basic mode, CLI TUI, GUI)
/// that want to persist a session-scoped override using the same parsing rules as
/// [`apply_cli_model_arg_overrides`].
///
/// Returns `None` for empty/whitespace-only input.
pub fn session_llm_config_from_cli_model_arg(model_arg: &str) -> Option<SessionLlmConfig> {
    parse_cli_model_arg(model_arg)
}

/// Apply session-style LLM overrides for CLI adapters.
///
/// CLI adapters don't have access to GUI keychain storage, so we resolve API keys
/// from (1) already-loaded config values and (2) environment variables.
/// Apply session-style LLM overrides for CLI adapters.
///
/// CLI adapters don't have access to GUI keychain storage, so we resolve API keys
/// from (1) already-loaded config values and (2) environment variables.
///
/// This is a convenience wrapper around [`apply_session_llm_overrides`] that provides
/// a standard CLI API-key lookup strategy.
pub fn apply_cli_session_llm_overrides(
    cfg: &mut AppConfig,
    session_llm: Option<&SessionLlmConfig>,
) -> EffectiveLlmConfig {
    let openai_key = cfg.llm.openai.as_ref().map(|c| c.api_key.clone());
    let anthropic_key = cfg.llm.anthropic.as_ref().map(|c| c.api_key.clone());
    let gemini_key = cfg.llm.gemini.as_ref().map(|c| c.api_key.clone());
    let grok_key = cfg.llm.grok.as_ref().map(|c| c.api_key.clone());

    let api_key_lookup = move |provider: &str| match provider {
        "openai" => openai_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| config_env::get_env("OPENAI_API_KEY")),
        "anthropic" => anthropic_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| config_env::get_env("ANTHROPIC_API_KEY")),
        "gemini" => gemini_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| config_env::get_env("GEMINI_API_KEY")),
        "grok" => grok_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| config_env::get_env("GROK_API_KEY")),
        _ => None,
    };

    apply_session_llm_overrides(cfg, session_llm, api_key_lookup)
}

fn apply_model_override(
    cfg: &mut AppConfig,
    model: &str,
    api_key_lookup: &impl Fn(&str) -> Option<String>,
) {
    match cfg.llm.primary.as_str() {
        "openai" => {
            let openai = cfg.llm.openai.get_or_insert_with(|| OpenAiConfig {
                api_key: api_key_lookup("openai").unwrap_or_default(),
                model: model.to_string(),
                base_url: None,
            });
            openai.model = model.to_string();
        }
        "anthropic" => {
            let anthropic = cfg.llm.anthropic.get_or_insert_with(|| AnthropicConfig {
                api_key: api_key_lookup("anthropic").unwrap_or_default(),
                model: model.to_string(),
                base_url: None,
                thinking_budget_tokens: None,
            });
            anthropic.model = model.to_string();
        }
        "grok" => {
            let grok = cfg.llm.grok.get_or_insert_with(|| GrokConfig {
                api_key: api_key_lookup("grok").unwrap_or_default(),
                model: model.to_string(),
                base_url: None,
            });
            grok.model = model.to_string();
        }
        "gemini" => {
            let gemini = cfg.llm.gemini.get_or_insert_with(|| GeminiConfig {
                api_key: api_key_lookup("gemini").unwrap_or_default(),
                model: model.to_string(),
                base_url: None,
            });
            gemini.model = model.to_string();
        }
        "ollama" => {
            let ollama = cfg.llm.ollama.get_or_insert_with(|| OllamaConfig {
                base_url: "http://localhost:11434".into(),
                model: model.to_string(),
            });
            ollama.model = model.to_string();
        }
        _ => {}
    }
}

fn get_model_for_provider(cfg: &AppConfig, provider: &str) -> Option<String> {
    match provider {
        "openai" => cfg.llm.openai.as_ref().map(|c| c.model.clone()),
        "anthropic" => cfg.llm.anthropic.as_ref().map(|c| c.model.clone()),
        "gemini" => cfg.llm.gemini.as_ref().map(|c| c.model.clone()),
        "grok" => cfg.llm.grok.as_ref().map(|c| c.model.clone()),
        "ollama" => cfg.llm.ollama.as_ref().map(|c| c.model.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_model_arg_parses_provider_and_model() {
        let parsed = parse_cli_model_arg("openai:gpt-4o").expect("should parse");
        assert_eq!(parsed.provider.as_deref(), Some("openai"));
        assert_eq!(parsed.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn parse_cli_model_arg_parses_model_only() {
        let parsed = parse_cli_model_arg("claude-3-5-sonnet").expect("should parse");
        assert_eq!(parsed.provider.as_deref(), None);
        assert_eq!(parsed.model.as_deref(), Some("claude-3-5-sonnet"));
    }

    #[test]
    fn apply_cli_model_arg_overrides_ignores_empty() {
        let mut cfg = AppConfig::default();
        let eff = apply_cli_model_arg_overrides(&mut cfg, Some("  "));
        assert_eq!(eff.provider, cfg.llm.primary);
    }

    #[test]
    fn provider_override_applies_and_model_falls_back() {
        let mut cfg = AppConfig::default();
        let session = SessionLlmConfig {
            provider: Some("ollama".into()),
            model: None,
        };

        let eff = apply_session_llm_overrides(&mut cfg, Some(&session), |_| None);
        assert_eq!(eff.provider, "ollama");
        assert!(!eff.model.is_empty());
        assert_eq!(cfg.llm.primary, "ollama");
    }

    #[test]
    fn incompatible_model_override_is_ignored() {
        let mut cfg = AppConfig::default();
        // Use a provider with no config in default (anthropic is default primary, config None)
        let session = SessionLlmConfig {
            provider: Some("openai".into()),
            model: Some("grok-2".into()),
        };

        let eff = apply_session_llm_overrides(&mut cfg, Some(&session), |_| None);
        assert_eq!(eff.provider, "openai");
        // Because openai config isn't created when model override is rejected, model falls back to empty.
        assert_eq!(eff.model, "");
        assert!(cfg.llm.openai.is_none());
    }

    #[test]
    fn model_override_creates_provider_config_and_sets_model() {
        let mut cfg = AppConfig::default();
        let session = SessionLlmConfig {
            provider: Some("openai".into()),
            model: Some("gpt-4o".into()),
        };

        let eff = apply_session_llm_overrides(&mut cfg, Some(&session), |_| Some("k".into()));
        assert_eq!(eff.provider, "openai");
        assert_eq!(eff.model, "gpt-4o");
        assert_eq!(cfg.llm.primary, "openai");
        assert_eq!(cfg.llm.openai.as_ref().unwrap().model, "gpt-4o");
    }
}
