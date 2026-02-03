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
    /// This matches the canonical secure-storage key names used across the
    /// application (and the `AppConfig` secret-migration logic).
    pub const fn storage_key(self) -> &'static str {
        match self {
            Self::OpenAi => "gestura_llm_openai_api_key",
            Self::VoiceOpenAi => "gestura_voice_openai_api_key",
            Self::Anthropic => "gestura_llm_anthropic_api_key",
            Self::Grok => "gestura_llm_grok_api_key",
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
        let canonical_key = key.storage_key();

        // 1) Canonical key
        match self.storage.get_secret(canonical_key).await {
            Ok(Some(v)) if !v.is_empty() => return Some(v),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    storage_key = canonical_key,
                    error = %e,
                    "Failed to read secret from secure storage"
                );
            }
        }

        // 2) Legacy key fallback + self-heal
        let legacy_key = key.legacy_storage_key()?;
        match self.storage.get_secret(legacy_key).await {
            Ok(Some(v)) if !v.is_empty() => {
                if let Err(e) = self.storage.store_secret(canonical_key, &v).await {
                    tracing::warn!(
                        canonical_key,
                        legacy_key,
                        error = %e,
                        "Failed to self-heal secret from legacy key to canonical key"
                    );
                }
                Some(v)
            }
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(
                    storage_key = legacy_key,
                    error = %e,
                    "Failed to read secret from secure storage (legacy key)"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{MockSecureStorage, SecureStorage, SecureStorageError};
    use std::sync::Arc;

    /// Wrapper so we can both:
    /// - give `SecureStorageSecretProvider` an owned `Box<dyn SecureStorage>`
    /// - keep a handle to the underlying mock storage for assertions
    #[derive(Clone)]
    struct SharedMockStorage(Arc<MockSecureStorage>);

    #[async_trait::async_trait]
    impl SecureStorage for SharedMockStorage {
        async fn store_secret(&self, key: &str, value: &str) -> Result<(), SecureStorageError> {
            self.0.store_secret(key, value).await
        }

        async fn get_secret(&self, key: &str) -> Result<Option<String>, SecureStorageError> {
            self.0.get_secret(key).await
        }

        async fn delete_secret(&self, key: &str) -> Result<(), SecureStorageError> {
            self.0.delete_secret(key).await
        }
    }

    #[tokio::test]
    async fn reads_legacy_key_and_self_heals_to_canonical() {
        let inner = Arc::new(MockSecureStorage::new());
        inner
            .store_secret("gestura_api_key_openai", "sk-legacy")
            .await
            .unwrap();

        let provider = SecureStorageSecretProvider::new(Box::new(SharedMockStorage(inner.clone())));

        let got = provider.get_secret(SecretKey::OpenAi).await;
        assert_eq!(got.as_deref(), Some("sk-legacy"));

        let canonical = inner
            .get_secret("gestura_llm_openai_api_key")
            .await
            .unwrap();
        assert_eq!(canonical.as_deref(), Some("sk-legacy"));
    }

    #[tokio::test]
    async fn canonical_key_wins_over_legacy_key() {
        let inner = Arc::new(MockSecureStorage::new());
        inner
            .store_secret("gestura_llm_openai_api_key", "sk-canonical")
            .await
            .unwrap();
        inner
            .store_secret("gestura_api_key_openai", "sk-legacy")
            .await
            .unwrap();

        let provider = SecureStorageSecretProvider::new(Box::new(SharedMockStorage(inner.clone())));

        let got = provider.get_secret(SecretKey::OpenAi).await;
        assert_eq!(got.as_deref(), Some("sk-canonical"));
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
