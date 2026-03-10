//! Configuration types, validation, environment loading, and file watching.
//!
//! `gestura-core-config` is the source of truth for Gestura configuration data
//! structures and pure configuration workflows.
//!
//! ## Responsibilities
//!
//! - `AppConfig` and nested configuration structs/enums
//! - environment-variable loading and override helpers
//! - validation rules and config sanity checks
//! - file watching for live reload flows
//! - hook and plugin configuration models
//!
//! ## Boundary with `gestura-core`
//!
//! This crate avoids runtime integrations that require security-sensitive or
//! platform-specific bridges. Keychain hydration, secure secret migration, and
//! other security-dependent extensions remain in the `gestura-core::config`
//! facade layer.
//!
//! For most consumers, the stable public import paths remain
//! `gestura_core::config::*` and `gestura_core::config_env::*`.

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
