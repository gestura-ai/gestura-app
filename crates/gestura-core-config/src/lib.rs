//! Configuration domain crate for Gestura
//!
//! This crate owns all configuration **type definitions**, pure loading/saving
//! helpers, environment-variable support, validation, and file-watching.
//!
//! Security-dependent operations (keychain hydration, secret migration,
//! sanitization) remain in `gestura-core` as bridge code.

pub mod config_env;
pub mod hooks_types;
pub mod types;
pub mod validation;
pub mod watcher;

// Re-export everything at crate root for convenience
pub use config_env::*;
pub use hooks_types::*;
pub use types::*;
pub use validation::*;
pub use watcher::*;
