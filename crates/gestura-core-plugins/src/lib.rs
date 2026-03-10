//! Plugin discovery, lifecycle management, and permission-aware execution.
//!
//! `gestura-core-plugins` provides the workspace plugin domain: manifest-backed
//! discovery, lifecycle state management, plugin permission metadata, and a
//! shared plugin manager that can be consumed by higher-level runtime code.
//!
//! ## Main concepts
//!
//! - `PluginMetadata`: manifest-level identity, versioning, and descriptive data
//! - `PluginDependency`: declared plugin dependency edges
//! - `PluginPermission`: requested capability set for the plugin
//! - `PluginState`: lifecycle state such as loaded, running, or failed
//! - `PluginManager`: discovery, load/unload, start/stop, command execution, and
//!   event broadcast coordination
//!
//! ## Architecture role
//!
//! This crate owns the plugin domain model and manager implementation. It does
//! not try to turn plugins into a separate presentation or orchestration layer;
//! instead, it provides a core-owned substrate that other application surfaces
//! can use consistently.
//!
//! ## Stable import paths
//!
//! Most application code should import through `gestura_core::plugin_system::*`.

mod plugin_system;

pub use plugin_system::*;
