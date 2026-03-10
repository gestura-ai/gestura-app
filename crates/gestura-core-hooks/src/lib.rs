//! Safe-by-default hooks engine for event-driven command templates.
//!
//! `gestura-core-hooks` provides a small hooks system for wiring specific agent
//! or application events to command templates, while keeping execution tightly
//! constrained and auditable.
//!
//! ## Safety model
//!
//! Hooks are intentionally conservative:
//!
//! - hook execution is opt-in
//! - configured programs must appear in `allowed_programs`
//! - templates are rendered from explicit `HookContext` values
//! - execution records are captured for inspection and testing
//!
//! This crate focuses on the hooks data model and execution engine rather than
//! broader pipeline orchestration.
//!
//! ## Main entry points
//!
//! - `HookDefinition` and `HookCommandTemplate`: the declarative hook model
//! - `HookEvent`: supported events that can trigger hooks
//! - `HooksSettings`: engine configuration including allow-listed programs
//! - `HookEngine`: safe dispatcher that resolves and runs matching hooks
//! - `HookExecutionRecord`: audit-friendly result of a hook run

mod engine;
mod executor;
mod template;
mod types;

pub use engine::{HookEngine, HookExecutionRecord};
pub use executor::{HookExecutor, ProcessHookExecutor};
pub use template::{TemplateVars, render_template};
pub use types::{HookCommandTemplate, HookContext, HookDefinition, HookEvent, HooksSettings};
