//! OpenAI model capability and endpoint routing helpers.
//!
//! Gestura supports both the legacy Chat Completions endpoint and the modern
//! Responses endpoint for OpenAI. This module centralizes the model-id
//! heuristics that determine whether a model is suitable for agent sessions and
//! which endpoint should be used.

/// OpenAI inference API selected for a given model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiApi {
    /// `/v1/chat/completions`
    ChatCompletions,
    /// `/v1/responses`
    Responses,
}

const OPENAI_AGENT_MODEL_PREFIXES: &[&str] =
    &["gpt-", "o1-", "o3-", "o4-", "o5-", "chatgpt-4o-", "codex-"];
const OPENAI_FINE_TUNED_AGENT_MODEL_PREFIXES: &[&str] = &[
    "ft:gpt-",
    "ft:o1-",
    "ft:o3-",
    "ft:o4-",
    "ft:o5-",
    "ft:codex-",
];
const OPENAI_NON_AGENT_MARKERS: &[&str] = &[
    "instruct",
    "transcribe",
    "audio",
    "realtime",
    "tts",
    "moderation",
    "embedding",
];
const OPENAI_LEGACY_COMPLETION_MODELS: &[&str] = &[
    "ada",
    "babbage",
    "curie",
    "davinci",
    "babbage-002",
    "davinci-002",
    "gpt-3.5-turbo-instruct",
];

fn normalize_model_id(model_id: &str) -> String {
    model_id.trim().to_ascii_lowercase()
}

fn starts_with_any(value: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| value.starts_with(prefix))
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

/// Returns `true` when the model id is obviously part of the OpenAI family.
pub fn looks_like_openai_model(model_id: &str) -> bool {
    let model_id = normalize_model_id(model_id);
    !model_id.is_empty()
        && (starts_with_any(&model_id, OPENAI_AGENT_MODEL_PREFIXES)
            || starts_with_any(&model_id, OPENAI_FINE_TUNED_AGENT_MODEL_PREFIXES)
            || model_id.starts_with("text-")
            || model_id.starts_with("code-"))
}

/// Returns `true` when the model id is a known legacy completion / non-agent model.
pub fn is_known_openai_legacy_completion_model(model_id: &str) -> bool {
    let model_id = normalize_model_id(model_id);
    if model_id.is_empty() {
        return false;
    }

    if model_id.starts_with("text-")
        || model_id.starts_with("code-")
        || model_id.starts_with("gpt-image-")
    {
        return true;
    }

    if OPENAI_LEGACY_COMPLETION_MODELS.contains(&model_id.as_str()) {
        return true;
    }

    OPENAI_LEGACY_COMPLETION_MODELS.iter().any(|base| {
        model_id.starts_with(&format!("ft:{base}:")) || model_id.starts_with(&format!("{base}:ft-"))
    })
}

/// Returns `true` when a model should be routed to `/v1/responses`.
pub fn is_openai_responses_api_model(model_id: &str) -> bool {
    let model_id = normalize_model_id(model_id);
    if model_id.is_empty() {
        return false;
    }

    let is_chat_compatible_codex = model_id.starts_with("codex-mini-");
    let is_responses_codex = ((model_id.starts_with("codex-") && !is_chat_compatible_codex)
        || model_id.contains("-codex"))
        && !is_chat_compatible_codex;

    is_responses_codex || model_id.starts_with("gpt-5") || model_id.starts_with("o5-")
}

/// Select the OpenAI inference API to use for the supplied model id.
pub fn openai_api_for_model(model_id: &str) -> OpenAiApi {
    if is_openai_responses_api_model(model_id) {
        OpenAiApi::Responses
    } else {
        OpenAiApi::ChatCompletions
    }
}

/// Returns `true` when the model is suitable for Gestura agent sessions.
pub fn is_agent_capable_openai_model(model_id: &str) -> bool {
    let model_id = normalize_model_id(model_id);
    if model_id.is_empty() || is_known_openai_legacy_completion_model(&model_id) {
        return false;
    }

    let has_supported_prefix = starts_with_any(&model_id, OPENAI_AGENT_MODEL_PREFIXES)
        || starts_with_any(&model_id, OPENAI_FINE_TUNED_AGENT_MODEL_PREFIXES);

    has_supported_prefix
        && !contains_any(&model_id, OPENAI_NON_AGENT_MARKERS)
        && !model_id.starts_with("gpt-image-")
}

/// Returns `true` when the model should be rejected for agent sessions.
pub fn is_openai_model_incompatible_with_agent_session(model_id: &str) -> bool {
    let model_id = normalize_model_id(model_id);
    if model_id.is_empty() {
        return false;
    }

    if is_known_openai_legacy_completion_model(&model_id) {
        return true;
    }

    if looks_like_openai_model(&model_id) {
        return !is_agent_capable_openai_model(&model_id);
    }

    false
}

/// Build a user-facing error describing why the model cannot be used for sessions.
pub fn openai_agent_session_model_message(model_id: &str) -> String {
    format!(
        "OpenAI model '{}' is not compatible with Gestura agent sessions. Gestura automatically routes OpenAI agent requests to /v1/chat/completions or /v1/responses depending on model capabilities, so choose an agent/tool-capable model such as gpt-4o, gpt-4.1, o3, o4-mini, gpt-5.4, gpt-5.3-codex, or codex-mini-latest.",
        model_id.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_modern_models_to_responses() {
        assert_eq!(openai_api_for_model("gpt-5.4"), OpenAiApi::Responses);
        assert_eq!(openai_api_for_model("gpt-5.3-codex"), OpenAiApi::Responses);
        assert_eq!(openai_api_for_model("codex-1"), OpenAiApi::Responses);
        assert_eq!(openai_api_for_model("o5-preview"), OpenAiApi::Responses);
        assert_eq!(openai_api_for_model("gpt-4o"), OpenAiApi::ChatCompletions);
        assert_eq!(
            openai_api_for_model("codex-mini-latest"),
            OpenAiApi::ChatCompletions
        );
    }

    #[test]
    fn recognizes_agent_capable_models() {
        for model in [
            "gpt-4o",
            "gpt-4.1",
            "o4-mini",
            "gpt-5.4",
            "gpt-5.3-codex",
            "codex-1",
            "codex-mini-latest",
        ] {
            assert!(
                is_agent_capable_openai_model(model),
                "expected {model} to be supported"
            );
        }

        for model in [
            "text-davinci-003",
            "gpt-4o-transcribe",
            "gpt-4o-audio-preview",
            "gpt-realtime",
            "gpt-image-1",
        ] {
            assert!(
                is_openai_model_incompatible_with_agent_session(model),
                "expected {model} to be rejected"
            );
        }
    }
}
