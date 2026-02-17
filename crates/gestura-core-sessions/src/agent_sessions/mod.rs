//! Agent session model + persistence.
//!
//! This module provides a **single** shared agent-session representation used by
//! both the CLI and GUI layers.

mod legacy_gui_migration;
mod store;
mod types;

pub use legacy_gui_migration::*;
pub use store::*;
pub use types::*;
