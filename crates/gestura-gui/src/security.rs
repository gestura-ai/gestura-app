//! Security module - thin wrapper over gestura_core::security
//!
//! This module provides re-exports from gestura-core's security module.
//! All encryption, keychain, and token management logic lives in core.

// Re-export core security types
pub use gestura_core::security::{
    McpToken, MockSecureStorage, SecureStorage, SecureStorageError, create_secure_storage,
};

#[cfg(feature = "security")]
pub use gestura_core::security::{Encryptor, KeychainStorage, SecureConfigManager};

// Re-export encryption module for backwards compatibility
#[cfg(feature = "security")]
pub mod encryption {
    pub use gestura_core::security::encryption::*;
}
