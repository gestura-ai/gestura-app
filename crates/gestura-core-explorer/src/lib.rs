//! File system explorer utility for Gestura.
//!
//! Provides file system exploration (listing directories, resolving paths,
//! git status) for the agent pipeline.
//!
//! ## Stable import paths
//!
//! Most code should import through `gestura_core::explorer::*`.

mod explorer;

pub use explorer::*;
