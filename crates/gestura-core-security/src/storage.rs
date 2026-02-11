//! Secure storage implementations for secrets management
//!
//! Provides abstractions for storing and retrieving sensitive data
//! using either OS keychain integration or in-memory mock storage.

use std::collections::HashMap;
use thiserror::Error;

/// Error type for secure storage operations
#[derive(Debug, Error)]
pub enum SecureStorageError {
    /// Storage backend error
    #[error("Storage error: {0}")]
    Storage(String),

    /// Key not found
    #[error("Key not found: {0}")]
    NotFound(String),

    /// Lock poisoned
    #[error("Lock poisoned")]
    LockPoisoned,

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<SecureStorageError> for gestura_core_foundation::AppError {
    fn from(err: SecureStorageError) -> Self {
        gestura_core_foundation::AppError::Io(std::io::Error::other(err.to_string()))
    }
}

/// Secure storage interface for sensitive data
///
/// Implementations of this trait provide secure storage for secrets
/// such as API keys, tokens, and encryption keys.
#[async_trait::async_trait]
pub trait SecureStorage: Send + Sync {
    /// Store a secret value with the given key
    async fn store_secret(&self, key: &str, value: &str) -> Result<(), SecureStorageError>;

    /// Retrieve a secret value by key
    async fn get_secret(&self, key: &str) -> Result<Option<String>, SecureStorageError>;

    /// Delete a secret by key
    async fn delete_secret(&self, key: &str) -> Result<(), SecureStorageError>;

    /// Check if a secret exists
    async fn has_secret(&self, key: &str) -> Result<bool, SecureStorageError> {
        Ok(self.get_secret(key).await?.is_some())
    }
}

/// Mock secure storage for testing
///
/// Stores secrets in memory without persistence. Suitable for testing
/// and development environments where OS keychain is not available.
pub struct MockSecureStorage {
    data: std::sync::RwLock<HashMap<String, String>>,
}

impl Default for MockSecureStorage {
    fn default() -> Self {
        Self {
            data: std::sync::RwLock::new(HashMap::new()),
        }
    }
}

impl MockSecureStorage {
    /// Create a new mock storage instance
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a mock storage pre-populated with secrets
    pub fn with_secrets(secrets: HashMap<String, String>) -> Self {
        Self {
            data: std::sync::RwLock::new(secrets),
        }
    }
}

#[async_trait::async_trait]
impl SecureStorage for MockSecureStorage {
    async fn store_secret(&self, key: &str, value: &str) -> Result<(), SecureStorageError> {
        let mut data = self
            .data
            .write()
            .map_err(|_| SecureStorageError::LockPoisoned)?;
        data.insert(key.to_string(), value.to_string());
        Ok(())
    }

    async fn get_secret(&self, key: &str) -> Result<Option<String>, SecureStorageError> {
        let data = self
            .data
            .read()
            .map_err(|_| SecureStorageError::LockPoisoned)?;
        Ok(data.get(key).cloned())
    }

    async fn delete_secret(&self, key: &str) -> Result<(), SecureStorageError> {
        let mut data = self
            .data
            .write()
            .map_err(|_| SecureStorageError::LockPoisoned)?;
        data.remove(key);
        Ok(())
    }
}

/// OS keychain integration (when security feature enabled)
///
/// Uses the operating system's secure credential storage:
/// - macOS: Keychain
/// - Windows: Credential Manager
/// - Linux: Secret Service (via libsecret)
#[cfg(feature = "security")]
pub struct KeychainStorage;

#[cfg(feature = "security")]
#[async_trait::async_trait]
impl SecureStorage for KeychainStorage {
    async fn store_secret(&self, key: &str, value: &str) -> Result<(), SecureStorageError> {
        let entry = keyring::Entry::new("gestura", key)
            .map_err(|e| SecureStorageError::Storage(e.to_string()))?;
        entry
            .set_password(value)
            .map_err(|e| SecureStorageError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn get_secret(&self, key: &str) -> Result<Option<String>, SecureStorageError> {
        let entry = keyring::Entry::new("gestura", key)
            .map_err(|e| SecureStorageError::Storage(e.to_string()))?;
        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecureStorageError::Storage(e.to_string())),
        }
    }

    async fn delete_secret(&self, key: &str) -> Result<(), SecureStorageError> {
        let entry = keyring::Entry::new("gestura", key)
            .map_err(|e| SecureStorageError::Storage(e.to_string()))?;
        match entry.delete_password() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecureStorageError::Storage(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_storage_store_and_get() {
        let storage = MockSecureStorage::new();
        storage
            .store_secret("test_key", "test_value")
            .await
            .unwrap();
        let value = storage.get_secret("test_key").await.unwrap();
        assert_eq!(value, Some("test_value".to_string()));
    }

    #[tokio::test]
    async fn test_mock_storage_delete() {
        let storage = MockSecureStorage::new();
        storage.store_secret("key", "value").await.unwrap();
        storage.delete_secret("key").await.unwrap();
        let value = storage.get_secret("key").await.unwrap();
        assert!(value.is_none());
    }

    #[tokio::test]
    async fn test_mock_storage_has_secret() {
        let storage = MockSecureStorage::new();
        assert!(!storage.has_secret("key").await.unwrap());
        storage.store_secret("key", "value").await.unwrap();
        assert!(storage.has_secret("key").await.unwrap());
    }
}
