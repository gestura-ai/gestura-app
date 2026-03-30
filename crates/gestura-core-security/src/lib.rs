//! Security primitives, secure storage, sandboxing, and privacy helpers.
//!
//! `gestura-core-security` owns the core security-related functionality for the
//! workspace. It combines secret storage, optional encryption, execution
//! sandboxing, and GDPR-focused data handling into a single domain crate.
//!
//! ## Responsibilities
//!
//! - secure storage abstraction with OS-keychain and mock implementations
//! - AES-256-GCM encryption helpers behind the `security` feature
//! - secret-provider integration for runtime config and provider credentials
//! - sandbox configuration and isolation primitives
//! - GDPR support such as export, deletion, consent, and audit-oriented helpers
//!
//! ## Security model
//!
//! The workspace follows a default-deny posture for dangerous behavior. This
//! crate does not implement the full tool-permission system itself, but it
//! provides the lower-level building blocks used by higher-level orchestration:
//!
//! - secure secret storage instead of plaintext where possible
//! - explicit sandbox boundaries for untrusted execution
//! - typed privacy and token models used across protocol and tool flows
//!
//! ## Feature-gated behavior
//!
//! - `security`: enables AES-256-GCM encryption and OS keychain integration
//!
//! When the `security` feature is unavailable or keychain access is disabled,
//! the crate can fall back to mock/in-memory behavior that keeps tests and
//! reduced environments usable without pretending secrets are durably protected.
//!
//! ## Stable import paths
//!
//! Most application code should import through the facade paths exposed by
//! `gestura-core`, such as:
//!
//! - `gestura_core::security::*`
//! - `gestura_core::gdpr::*`
//! - `gestura_core::sandbox::*`

pub mod gdpr;
pub mod sandbox;
pub mod secrets;
pub mod storage;

#[cfg(feature = "security")]
pub mod encryption;

// Re-exports for convenience
pub use gdpr::*;
pub use sandbox::*;
pub use secrets::SecureStorageSecretProvider;
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

/// Check if keychain access is disabled via environment variables
///
/// Returns `true` when `GESTURA_DISABLE_KEYCHAIN=1` or
/// `GESTURA_NO_KEYCHAIN=1` is set.
///
/// Outside unit-test builds, `CI` also disables keychain access to avoid
/// non-interactive runner hangs. Unit tests intentionally ignore bare `CI`
/// because test builds already use in-memory mock secure storage.
pub fn keychain_access_disabled() -> bool {
    let explicitly_disabled = std::env::var_os("GESTURA_DISABLE_KEYCHAIN").is_some()
        || std::env::var_os("GESTURA_NO_KEYCHAIN").is_some();

    explicitly_disabled || (cfg!(not(test)) && std::env::var_os("CI").is_some())
}

/// Create the appropriate secure storage implementation based on features.
///
/// When the `security` feature is enabled, this returns a keychain-backed
/// storage unless keychain access has been explicitly disabled. Otherwise, it
/// returns an in-memory mock storage suitable for tests and constrained
/// environments.
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
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct ScopedEnvVar {
        key: &'static str,
        old: Option<OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }

        fn unset(key: &'static str) -> Self {
            let old = std::env::var_os(key);
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, old }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            if let Some(value) = &self.old {
                unsafe {
                    std::env::set_var(self.key, value);
                }
            } else {
                unsafe {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

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

    #[test]
    fn keychain_access_disabled_ignores_ci_in_unit_tests() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let _ci = ScopedEnvVar::set("CI", "1");
        let _disabled = ScopedEnvVar::unset("GESTURA_DISABLE_KEYCHAIN");
        let _no_keychain = ScopedEnvVar::unset("GESTURA_NO_KEYCHAIN");

        assert!(
            !keychain_access_disabled(),
            "unit tests should not skip mocked secure storage just because CI is set"
        );
    }

    #[test]
    fn keychain_access_disabled_still_respects_explicit_overrides() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let _ci = ScopedEnvVar::unset("CI");
        let _disabled = ScopedEnvVar::set("GESTURA_DISABLE_KEYCHAIN", "1");
        let _no_keychain = ScopedEnvVar::unset("GESTURA_NO_KEYCHAIN");

        assert!(keychain_access_disabled());
    }
}
