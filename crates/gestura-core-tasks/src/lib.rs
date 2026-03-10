//! Task management and reusable workflow primitives for Gestura.
//!
//! `gestura-core-tasks` owns the persistent task-list model and reusable
//! workflow definitions used by agent sessions, orchestration layers, and user
//! interfaces.
//!
//! ## Responsibilities
//!
//! - session-scoped task CRUD and persistence via `TaskManager`
//! - hierarchical task lists, task state transitions, and metadata tracking
//! - task-memory lifecycle events used to mirror memory promotions and blockers
//! - reusable markdown workflow definitions discovered by `WorkflowManager`
//!
//! ## Architecture role
//!
//! This crate is the source of truth for task and workflow domain behavior.
//! Higher-level orchestration—such as deciding when a supervisor creates or
//! blocks tasks—remains in `gestura-core`, but the underlying task graph and
//! workflow loading logic live here.
//!
//! ## Storage model
//!
//! Task state is persisted under the workspace `.gestura/` area so it can be
//! resumed across sessions. Workflow definitions are loaded from workspace-local
//! or user-level workflow directories, allowing reusable templates without
//! hard-coding them into the pipeline.
//!
//! ## Stable import paths
//!
//! Most code should import through the facade:
//!
//! - `gestura_core::tasks::*`
//! - `gestura_core::workflows::*`

pub mod tasks;
pub mod workflows;

pub use tasks::get_global_task_manager;
