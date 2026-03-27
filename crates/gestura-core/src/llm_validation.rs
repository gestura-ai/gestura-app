//! LLM provider/model compatibility helpers.
//!
//! Gestura supports multiple LLM providers. Some model ids are provider-specific
//! (e.g. `grok-*` for Grok, `claude-*` for Anthropic). This module provides a
//! *best-effort* guardrail to prevent persisting obviously invalid
//! (provider, model) pairs into session-scoped configuration.
//!
//! Notes:
//! - This intentionally errs on the side of *allowing* unknown/custom model ids.
//! - Providers with user-defined model ids (e.g. Ollama) are treated as compatible.

use gestura_core_llm::openai::{
    is_openai_model_incompatible_with_agent_session, looks_like_openai_model,
    openai_agent_session_model_message,
};

/// Attempt to infer the provider a model id belongs to.
///
/// Returns `None` if the model id does not match any known provider prefix.
/// Callers should treat `None` as "unknown" and generally allow it.
///
/// Known inferences:
/// - `grok-*` → `"grok"`
/// - `claude-*` → `"anthropic"`
/// - `gemini-*` → `"gemini"`
/// - `gpt-*`, `o1-*`, `o3-*`, `o4-*`, `o5-*`, `ft:gpt-*`, `codex-*` → `"openai"`
pub fn infer_provider_from_model_id(model_id: &str) -> Option<&'static str> {
    let m = model_id.trim().to_ascii_lowercase();
    if m.is_empty() {
        return None;
    }

    if m.starts_with("grok-") {
        return Some("grok");
    }

    if m.starts_with("claude-") {
        return Some("anthropic");
    }

    if m.starts_with("gemini-") {
        return Some("gemini");
    }

    // OpenAI model ids are not exclusively `gpt-*`, but these prefixes are
    // common enough to treat as a strong signal.
    if looks_like_openai_model(&m) {
        return Some("openai");
    }

    None
}

/// Returns `true` if the `model_id` is compatible with `provider`.
///
/// Compatibility is determined via [`infer_provider_from_model_id`]. If a
/// provider cannot be inferred for the model id, this returns `true` to avoid
/// blocking legitimate custom/unknown model ids.
///
/// Providers with user-defined model ids (`ollama`) are always treated
/// as compatible.
pub fn is_model_compatible_with_provider(provider: &str, model_id: &str) -> bool {
    let p = provider.trim().to_ascii_lowercase();
    if p.is_empty() {
        return true;
    }

    if p == "ollama" {
        return true;
    }

    if p == "openai" && is_openai_model_incompatible_with_agent_session(model_id) {
        return false;
    }

    match infer_provider_from_model_id(model_id) {
        Some(inferred) => inferred == p,
        None => true,
    }
}

/// Validate a (provider, model) pair and return a user-facing error on mismatch.
pub fn validate_model_for_provider(provider: &str, model_id: &str) -> Result<(), String> {
    let normalized_provider = provider.trim().to_ascii_lowercase();
    if normalized_provider == "openai" && is_openai_model_incompatible_with_agent_session(model_id)
    {
        return Err(openai_agent_session_model_message(model_id));
    }

    if is_model_compatible_with_provider(&normalized_provider, model_id) {
        return Ok(());
    }

    let inferred = infer_provider_from_model_id(model_id).unwrap_or("unknown");
    Err(format!(
        "Invalid model for provider: provider='{}' model='{}' (looks like provider='{}')",
        provider.trim(),
        model_id.trim(),
        inferred
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_obvious_cross_provider_pairs() {
        assert!(!is_model_compatible_with_provider("openai", "grok-2"));
        assert!(!is_model_compatible_with_provider("grok", "gpt-4o"));
        assert!(!is_model_compatible_with_provider("anthropic", "grok-3"));
        assert!(!is_model_compatible_with_provider(
            "openai",
            "claude-sonnet-4-20250514"
        ));
        assert!(!is_model_compatible_with_provider(
            "openai",
            "gemini-2.0-flash"
        ));
        assert!(!is_model_compatible_with_provider("gemini", "gpt-4o"));
    }

    #[test]
    fn accepts_matching_prefixes() {
        assert!(is_model_compatible_with_provider("grok", "grok-2"));
        assert!(is_model_compatible_with_provider(
            "anthropic",
            "claude-sonnet-4-20250514"
        ));
        assert!(is_model_compatible_with_provider("openai", "gpt-4o"));
        assert!(is_model_compatible_with_provider("openai", "o1-mini"));
        assert!(is_model_compatible_with_provider("openai", "o4-mini"));
        assert!(is_model_compatible_with_provider(
            "openai",
            "codex-mini-latest"
        ));
        assert!(is_model_compatible_with_provider(
            "openai",
            "ft:gpt-4o-mini:gestura:agent-123"
        ));
        assert!(is_model_compatible_with_provider(
            "gemini",
            "gemini-2.0-flash"
        ));
        assert!(is_model_compatible_with_provider(
            "gemini",
            "gemini-1.5-pro"
        ));
    }

    #[test]
    fn allows_unknown_models_by_default() {
        assert!(is_model_compatible_with_provider(
            "openai",
            "my-custom-model"
        ));
        assert!(is_model_compatible_with_provider(
            "anthropic",
            "some-enterprise-model"
        ));
    }

    #[test]
    fn rejects_openai_completion_only_models_for_sessions() {
        for model in [
            "text-davinci-003",
            "gpt-3.5-turbo-instruct",
            "davinci-002",
            "ft:babbage-002:gestura:legacy-123",
            "davinci:ft-gestura-legacy-456",
            "gpt-4o-transcribe",
            "gpt-4o-audio-preview",
            "gpt-realtime",
            "gpt-image-1",
        ] {
            assert!(
                !is_model_compatible_with_provider("openai", model),
                "expected {model} to be rejected for OpenAI sessions"
            );
            let message = validate_model_for_provider("openai", model).unwrap_err();
            assert!(message.contains("automatically routes OpenAI agent requests"));
        }
    }

    #[test]
    fn accepts_openai_responses_models_for_sessions() {
        for model in ["gpt-5.4", "gpt-5.3-codex", "codex-1"] {
            assert!(
                is_model_compatible_with_provider("openai", model),
                "expected {model} to be accepted for OpenAI sessions"
            );
            validate_model_for_provider("openai", model).unwrap();
        }
    }

    #[test]
    fn infers_provider_for_fine_tuned_openai_chat_models() {
        assert_eq!(
            infer_provider_from_model_id("ft:gpt-4o-mini:gestura:abc123"),
            Some("openai")
        );
        assert_eq!(infer_provider_from_model_id("o5-mini"), Some("openai"));
        assert_eq!(
            infer_provider_from_model_id("codex-mini-latest"),
            Some("openai")
        );
        assert_eq!(
            infer_provider_from_model_id("gpt-5.3-codex"),
            Some("openai")
        );
    }

    #[test]
    fn ollama_is_always_compatible() {
        assert!(is_model_compatible_with_provider("ollama", "grok-2"));
        assert!(is_model_compatible_with_provider(
            "ollama",
            "claude-sonnet-4-20250514"
        ));
    }
}
