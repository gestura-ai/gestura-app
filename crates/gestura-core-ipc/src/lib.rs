//! Hotkey-oriented inter-process communication primitives for Gestura.
//!
//! `gestura-core-ipc` provides the lightweight IPC layer used to signal hotkey
//! activity between processes, especially between CLI sessions and helper
//! processes that need to discover or trigger them.
//!
//! ## Main entry points
//!
//! - `CliHotkeyEndpoint`: discovery payload describing the running CLI endpoint
//! - `default_cli_hotkey_port_file`: canonical discovery-file location
//! - `start_cli_hotkey_server`: start the CLI hotkey server and publish discovery info
//! - `try_send_hotkey_trigger_to_cli`: best-effort trigger delivery to a running CLI session
//! - `CliHotkeyServerGuard`: lifecycle guard for cleanup and task shutdown
//!
//! ## Architecture role
//!
//! This crate owns the IPC protocol primitives themselves. It does not decide
//! what a hotkey means in the broader application; higher-level hotkey behavior
//! stays in CLI/GUI orchestration code.
//!
//! ## Stable import paths
//!
//! Most code should import through `gestura_core::hotkey_ipc::*`.

mod hotkey_ipc;

pub use hotkey_ipc::*;
