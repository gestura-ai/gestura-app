//! Hooks system for Gestura
//!
//! This crate provides a small, **safe-by-default** hooks engine inspired by
//! Claude Code's hooks model:
//!
//! - Hooks are configured as command templates, tied to an event.
//! - Hooks are **disabled by default** and require explicit allow-listing of
//!   programs before anything is executed.
//! - The pipeline integration lives in a separate Phase (see task list). This
//!   crate focuses on the data model + execution engine + unit tests.

mod engine;
mod executor;
mod template;
mod types;

pub use engine::{HookEngine, HookExecutionRecord};
pub use executor::{HookExecutor, ProcessHookExecutor};
pub use template::{TemplateVars, render_template};
pub use types::{HookCommandTemplate, HookContext, HookDefinition, HookEvent, HooksSettings};
