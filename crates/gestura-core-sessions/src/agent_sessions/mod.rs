//! Agent session model + persistence.
//!
//! This module provides a **single** shared agent-session representation used by
//! both the CLI and GUI layers.

use std::path::PathBuf;

mod legacy_gui_migration;
mod store;
mod types;

const GESTURA_HOME_DIR_ENV: &str = "GESTURA_HOME_DIR";

fn gestura_home_dir() -> PathBuf {
    std::env::var_os(GESTURA_HOME_DIR_ENV)
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub use legacy_gui_migration::*;
pub use store::*;
pub use types::*;
