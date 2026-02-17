//! Memory Bank - Persistent context storage for conversation history
//!
//! This crate implements the Memory Bank concept inspired by Kilo Code's approach.
//! It provides persistent storage of conversation context in human-readable markdown
//! files that can be searched and retrieved across sessions.
//!
//! ## Stable import paths
//!
//! Most code should import through `gestura_core::memory_bank::*`.

mod memory_bank;

pub use memory_bank::*;
