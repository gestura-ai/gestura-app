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
//! ## Configuration model
//!
//! Gestura persists user configuration primarily as YAML at
//! `~/.gestura/config.yaml`. The typed structures in this crate are designed to
//! keep that file human-readable while still supporting richer runtime behavior.
//!
//! High-signal areas in the config model include:
//!
//! - LLM provider selection and provider-specific settings
//! - voice/STT provider settings and local-model paths
//! - pipeline limits, compaction, and reflection settings
//! - permissions, hooks, UI preferences, and developer options
//!
//! ## Precedence and portability
//!
//! This crate owns the *pure* parts of configuration behavior:
//!
//! - defaults embedded in Rust types
//! - file serialization/deserialization
//! - environment-variable overrides via `GESTURA_*`
//! - validation and config-key discovery
//!
//! Because these behaviors are side-effect-light and portable, they can be used
//! consistently from the CLI, GUI, tests, and tooling.
//!
//! ## Boundary with `gestura-core`
//!
//! This crate avoids runtime integrations that require security-sensitive or
//! platform-specific bridges. Keychain hydration, secure secret migration,
//! legacy on-disk migration wiring, and other security-dependent extensions
//! remain in the `gestura-core::config` facade layer.
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
