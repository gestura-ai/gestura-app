//! Security module for encryption, keychain integration, and token management
//!
//! This module provides secure storage abstractions and AES encryption utilities
//! for protecting sensitive data in the Gestura ecosystem.
//!
//! # Features
//!
//! - `security`: Enables AES-256-GCM encryption and OS keychain integration
//!
//! # Example
//!
//! ```rust,ignore
//! use gestura_core::security::{create_secure_storage, SecureStorage};
//!
//! let storage = create_secure_storage();
//! storage.store_secret("api_key", "secret_value").await?;
//! let value = storage.get_secret("api_key").await?;
//! ```

mod storage;

#[cfg(feature = "security")]
pub mod encryption;

pub use storage::{MockSecureStorage, SecureStorage, SecureStorageError};

#[cfg(feature = "security")]
pub use encryption::{Encryptor, SecureConfigManager};

#[cfg(feature = "security")]
pub use storage::KeychainStorage;

/// Token for MCP authentication
///
/// Represents an authentication token with optional expiration and scope restrictions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpToken {
    /// The token string
    pub token: String,
    /// Optional expiration timestamp
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Scopes this token is authorized for
    pub scopes: Vec<String>,
}

impl McpToken {
    /// Create a new MCP token
    pub fn new(token: String) -> Self {
        Self {
            token,
            expires_at: None,
            scopes: Vec::new(),
        }
    }

    /// Create a token with expiration
    pub fn with_expiry(token: String, expires_at: chrono::DateTime<chrono::Utc>) -> Self {
        Self {
            token,
            expires_at: Some(expires_at),
            scopes: Vec::new(),
        }
    }

    /// Add scopes to the token
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }

    /// Check if the token is expired
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| exp < chrono::Utc::now())
            .unwrap_or(false)
    }

    /// Check if token has a specific scope
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

/// Create the appropriate secure storage implementation based on features
///
/// When the `security` feature is enabled, returns a keychain-backed storage.
/// Otherwise, returns an in-memory mock storage suitable for testing.
///
/// ## Non-interactive / CI behavior
///
/// Some OS keychain providers may block in headless or non-interactive contexts
/// (e.g. CI, integration tests). To avoid hangs, you can disable keychain usage
/// at runtime by setting `GESTURA_DISABLE_KEYCHAIN=1` (or `GESTURA_NO_KEYCHAIN=1`).
///
/// When disabled, Gestura will behave as if secure storage is unavailable.
pub fn keychain_access_disabled() -> bool {
    std::env::var_os("GESTURA_DISABLE_KEYCHAIN").is_some()
        || std::env::var_os("GESTURA_NO_KEYCHAIN").is_some()
        || std::env::var_os("CI").is_some()
}

pub fn create_secure_storage() -> Box<dyn SecureStorage> {
    #[cfg(all(feature = "security", not(test)))]
    {
        if keychain_access_disabled() {
            Box::new(MockSecureStorage::default())
        } else {
            Box::new(KeychainStorage)
        }
    }
    #[cfg(any(not(feature = "security"), test))]
    {
        Box::new(MockSecureStorage::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_token_creation() {
        let token = McpToken::new("test_token".to_string());
        assert_eq!(token.token, "test_token");
        assert!(token.expires_at.is_none());
        assert!(token.scopes.is_empty());
    }

    #[test]
    fn test_mcp_token_with_scopes() {
        let token = McpToken::new("test".to_string())
            .with_scopes(vec!["read".to_string(), "write".to_string()]);
        assert!(token.has_scope("read"));
        assert!(token.has_scope("write"));
        assert!(!token.has_scope("admin"));
    }

    #[test]
    fn test_mcp_token_expiry() {
        let future = chrono::Utc::now() + chrono::Duration::hours(1);
        let token = McpToken::with_expiry("test".to_string(), future);
        assert!(!token.is_expired());

        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        let expired_token = McpToken::with_expiry("test".to_string(), past);
        assert!(expired_token.is_expired());
    }
}
