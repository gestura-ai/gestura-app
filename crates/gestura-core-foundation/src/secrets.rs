//! Secret (API key) retrieval abstractions.
//!
//! These are pure abstractions with no dependency on secure storage implementations.
//! Concrete implementations that depend on OS keychain / secure storage live in
//! `gestura-core::secrets`.

/// A strongly-typed identifier for a secret used by the core.
///
/// These map to keys in secure storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretKey {
    /// The general OpenAI API key.
    OpenAi,
    /// The OpenAI API key specifically for voice/STT.
    VoiceOpenAi,
    /// The Anthropic API key.
    Anthropic,
    /// The Grok (xAI) API key.
    Grok,
    /// The Google Gemini API key.
    Gemini,
}

impl SecretKey {
    /// Returns the secure-storage key name used to store this secret.
    ///
    /// This matches the canonical secure-storage key names used across the
    /// application (and the `AppConfig` secret-migration logic).
    pub const fn storage_key(self) -> &'static str {
        match self {
            Self::OpenAi => "gestura_llm_openai_api_key",
            Self::VoiceOpenAi => "gestura_voice_openai_api_key",
            Self::Anthropic => "gestura_llm_anthropic_api_key",
            Self::Grok => "gestura_llm_grok_api_key",
            Self::Gemini => "gestura_llm_gemini_api_key",
        }
    }

    /// Legacy secure-storage key name used by older releases.
    ///
    /// New writes should always use [`SecretKey::storage_key`]. This exists only
    /// for backwards-compatible reads + optional self-heal migration.
    pub const fn legacy_storage_key(self) -> Option<&'static str> {
        match self {
            Self::OpenAi => Some("gestura_api_key_openai"),
            Self::VoiceOpenAi => Some("gestura_api_key_voice_openai"),
            Self::Anthropic => Some("gestura_api_key_anthropic"),
            Self::Grok => Some("gestura_api_key_grok"),
            Self::Gemini => Some("gestura_api_key_gemini"),
        }
    }
}

/// A source of secrets used by core provider selection and execution.
///
/// Implementations should treat missing secrets as `None` and should not panic.
///
/// Note: this is async to support implementations backed by secure storage.
#[async_trait::async_trait]
pub trait SecretProvider: Send + Sync {
    /// Retrieve a secret by key.
    ///
    /// Implementations should return `None` if the secret does not exist or is
    /// unavailable.
    async fn get_secret(&self, key: SecretKey) -> Option<String>;
}

/// A `SecretProvider` that always returns `None`.
///
/// Useful for contexts where secure storage is not configured or should not be
/// consulted.
#[derive(Debug, Default)]
pub struct NullSecretProvider;

#[async_trait::async_trait]
impl SecretProvider for NullSecretProvider {
    async fn get_secret(&self, _key: SecretKey) -> Option<String> {
        None
    }
}
