//! Secret (API key) retrieval abstractions.
//!
//! Core business logic sometimes needs access to secrets (API keys) for provider
//! selection and request execution.
//!
//! ## Core-First rule
//! This module intentionally lives in `gestura-core` so that key-resolution
//! precedence and fallback behavior is **core-owned** (shared by GUI and CLI),
//! while presentation layers (Tauri/CLI) remain thin adapters.
//!
//! Implementations may read from:
//! - OS keychain (via `gestura_core::security::SecureStorage`)
//! - environment variables / config files (handled elsewhere, e.g. `AppConfig`)
//! - test stubs

use crate::security::SecureStorage;

/// A strongly-typed identifier for a secret used by the core.
///
/// These map to keys in `SecureStorage`.
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
}

impl SecretKey {
    /// Returns the secure-storage key name used to store this secret.
    ///
    /// This intentionally matches the GUI keychain naming convention:
    /// `gestura_api_key_{provider}`.
    pub const fn storage_key(self) -> &'static str {
        match self {
            Self::OpenAi => "gestura_api_key_openai",
            Self::VoiceOpenAi => "gestura_api_key_voice_openai",
            Self::Anthropic => "gestura_api_key_anthropic",
            Self::Grok => "gestura_api_key_grok",
        }
    }
}

/// A source of secrets used by core provider selection and execution.
///
/// Implementations should treat missing secrets as `None` and should not panic.
///
/// Note: this is async to support implementations backed by `SecureStorage`.
#[async_trait::async_trait]
pub trait SecretProvider: Send + Sync {
    /// Retrieve a secret by key.
    ///
    /// Implementations should return `None` if the secret does not exist or is
    /// unavailable.
    async fn get_secret(&self, key: SecretKey) -> Option<String>;
}

/// A `SecretProvider` backed by `gestura-core`'s `SecureStorage`.
///
/// This is the canonical implementation for GUI/desktop usage where keychain
/// storage is available behind the `security` feature.
pub struct SecureStorageSecretProvider {
    storage: Box<dyn SecureStorage>,
}

impl std::fmt::Debug for SecureStorageSecretProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `SecureStorage` is a trait object and may not be `Debug`. We still
        // implement `Debug` so this type can be logged/embedded in other debug
        // structures without exposing secrets or depending on the concrete
        // storage implementation.
        f.debug_struct("SecureStorageSecretProvider")
            .finish_non_exhaustive()
    }
}

impl SecureStorageSecretProvider {
    /// Create a new provider backed by the given secure storage.
    pub fn new(storage: Box<dyn SecureStorage>) -> Self {
        Self { storage }
    }
}

#[async_trait::async_trait]
impl SecretProvider for SecureStorageSecretProvider {
    async fn get_secret(&self, key: SecretKey) -> Option<String> {
        match self.storage.get_secret(key.storage_key()).await {
            Ok(v) => v.filter(|s| !s.is_empty()),
            Err(e) => {
                tracing::warn!(
                    storage_key = key.storage_key(),
                    error = %e,
                    "Failed to read secret from secure storage"
                );
                None
            }
        }
    }
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
