//! Durable memory-bank storage and retrieval for Gestura.
//!
//! `gestura-core-memory-bank` owns the long-term/shared memory layer used by the
//! runtime. It persists durable memory records as human-readable markdown with
//! typed metadata, making them searchable across sessions while staying easy to
//! inspect and recover manually.
//!
//! ## Memory model
//!
//! The memory bank is the durable counterpart to session-scoped working memory:
//!
//! - short-term working memory lives with active sessions
//! - durable memory-bank entries live on disk and can be reused later
//!
//! Entries carry structured metadata so retrieval can be selective rather than a
//! raw full-text search. High-signal dimensions include:
//!
//! - `MemoryKind`: retention/operational intent
//! - `MemoryType`: procedural, semantic, episodic, resource, or other typed use
//! - `MemoryScope`: task, session, directive, project, or global scope
//! - provenance, tags, confidence, archival state, and promotion metadata
//!
//! ## Main entry points
//!
//! - `MemoryBankEntry`: durable memory record persisted to markdown
//! - `MemoryBankQuery`: query builder for targeted retrieval
//! - `save_to_memory_bank`, `load_from_memory_bank`, `search_memory_bank_with_query`
//! - maintenance helpers such as listing, updating, deleting, and clearing
//!
//! ## Architecture role
//!
//! This crate owns durable memory persistence and retrieval. Higher-level UI and
//! operator workflows—such as the shared memory console used by CLI and GUI—live
//! in `gestura-core`, which composes this crate with session working memory.
//!
//! ## Stable import paths
//!
//! Most code should import through `gestura_core::memory_bank::*`.

mod memory_bank;

pub use memory_bank::*;
