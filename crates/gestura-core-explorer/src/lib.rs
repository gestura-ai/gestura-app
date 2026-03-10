//! Workspace-bounded file-system exploration utilities for Gestura.
//!
//! `gestura-core-explorer` provides lightweight exploration helpers used by the
//! agent pipeline and presentation layers to inspect a workspace safely.
//!
//! ## Responsibilities
//!
//! - directory listing with stable sorting and truncation reporting
//! - root-relative path handling suitable for UI and agent consumption
//! - workspace-bounded resolution to avoid accidental path escape
//! - related status helpers such as git-aware inspection surfaces
//!
//! ## Safety model
//!
//! Explorer operations are designed around a workspace root. Canonicalization
//! and root-bounded resolution help ensure that symlinks or path traversal do
//! not silently escape the intended workspace. Entries that resolve outside the
//! workspace are omitted rather than exposed.
//!
//! ## Stable import paths
//!
//! Most code should import through `gestura_core::explorer::*`.

mod explorer;

pub use explorer::*;
