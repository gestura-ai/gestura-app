//! Secret (API key) retrieval abstractions.
//!
//! Pure abstractions live in foundation, implementation in security crate.
//! Re-exported here for stable import paths.

pub use gestura_core_foundation::secrets::*;
pub use gestura_core_security::secrets::SecureStorageSecretProvider;
