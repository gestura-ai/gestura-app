//! IPC (hotkey/inter-process communication) primitives for Gestura.
//!
//! Provides a shared-memory / Unix-socket based IPC channel for
//! communicating hotkey events between processes.
//!
//! ## Stable import paths
//!
//! Most code should import through `gestura_core::hotkey_ipc::*`.

mod hotkey_ipc;

pub use hotkey_ipc::*;
