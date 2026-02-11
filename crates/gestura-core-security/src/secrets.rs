//! Secret (API key) retrieval backed by secure storage.
//!
//! Pure abstractions (`SecretKey`, `SecretProvider`, `NullSecretProvider`) live in
//! `gestura-core-foundation::secrets` and are re-exported from the crate root.
//!
//! This module adds the `SecureStorageSecretProvider` implementation that depends
//! on [`SecureStorage`](crate::SecureStorage) (OS keychain).

use crate::SecureStorage;
use gestura_core_foundation::secrets::{SecretKey, SecretProvider};

/// A `SecretProvider` backed by `SecureStorage`.
///
/// This is the canonical implementation for GUI/desktop usage where keychain
/// storage is available behind the `security` feature.
pub struct SecureStorageSecretProvider {
    storage: Box<dyn SecureStorage>,
}

impl std::fmt::Debug for SecureStorageSecretProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
    use crate::{MockSecureStorage, SecureStorageError};
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
