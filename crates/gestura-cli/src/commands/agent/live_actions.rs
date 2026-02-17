//! Live-action executors for canonical slash command handlers.
//!
//! `slash.rs` intentionally does **not** execute async work; it returns a "live action" enum.
//! These helpers execute those live actions given a Tokio runtime.

use std::path::{Path, PathBuf};

use tokio::runtime::Runtime;

use gestura_core::memory_bank::MemoryBankEntry;

use super::slash;

/// Output of executing a `/memory` live action.
#[derive(Debug)]
pub(super) enum MemoryExecOutput {
    Listed(Vec<MemoryBankEntry>),
    Searched {
        query: String,
        results: Vec<MemoryBankEntry>,
    },
    Saved(PathBuf),
    Cleared(usize),
    Deleted,
}

pub(super) fn execute_memory_live_action(
    rt: &Runtime,
    workspace_dir: &Path,
    act: slash::MemoryLiveAction,
) -> Result<MemoryExecOutput, String> {
    match act {
        slash::MemoryLiveAction::List => rt
            .block_on(gestura_core::memory_bank::list_memory_bank(workspace_dir))
            .map(MemoryExecOutput::Listed)
            .map_err(|e| e.to_string()),

        slash::MemoryLiveAction::Search { query, limit } => rt
            .block_on(gestura_core::memory_bank::search_memory_bank(
                workspace_dir,
                &query,
                limit,
            ))
            .map(|results| MemoryExecOutput::Searched { query, results })
            .map_err(|e| e.to_string()),

        slash::MemoryLiveAction::Save { entry } => rt
            .block_on(gestura_core::memory_bank::save_to_memory_bank(
                workspace_dir,
                &entry,
            ))
            .map(MemoryExecOutput::Saved)
            .map_err(|e| e.to_string()),

        slash::MemoryLiveAction::ClearAll => rt
            .block_on(gestura_core::memory_bank::clear_memory_bank(workspace_dir))
            .map(MemoryExecOutput::Cleared)
            .map_err(|e| e.to_string()),

        slash::MemoryLiveAction::Delete { file_path } => rt
            .block_on(gestura_core::memory_bank::delete_memory_bank_entry(
                workspace_dir,
                &file_path,
            ))
            .map(|_| MemoryExecOutput::Deleted)
            .map_err(|e| e.to_string()),
    }
}
