//! Chat session model + persistence.
//!
//! This module provides a **single** shared chat-session representation used by
//! both the CLI and GUI layers.
//!
//! ## Design goals
//! - **Core-first**: session state + persistence live in `gestura-core`.
//! - **No shell duplication**: CLI/GUI should not define their own persistent
//!   session models or file layouts.
//! - **Workspace safety**: session persistence is stored outside the session's
//!   sandbox workspace so sandboxed tools cannot overwrite session metadata.
//!
//! ## Persistence layout
//! The default file-backed store writes one JSON file per session under:
//! `AppConfig::data_dir()/chat_sessions/<session_id>.json`.

mod store;
mod types;

pub use store::*;
pub use types::*;
