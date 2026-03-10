//! Shared memory console service for CLI and GUI inspection workflows.
//!
//! This module provides the unified memory-console surface used across CLI,
//! GUI, and agent-facing memory inspection flows. It composes two underlying
//! systems:
//!
//! - session working memory from `agent_sessions`
//! - durable memory-bank entries from `gestura_core::memory_bank`
//!
//! ## Design role
//!
//! The memory console is intentionally a facade-level service instead of a
//! standalone domain crate because it coordinates multiple domains at once:
//!
//! - session storage
//! - durable memory-bank retrieval and mutation
//! - task lifecycle/memory event integration
//! - shared DTOs consumed by both CLI and GUI presentation layers
//!
//! ## High-signal workflows
//!
//! - overview facets and counts for memory health
//! - cross-memory search over working and durable memory
//! - inspection of promotions, archival state, and provenance
//! - task-aware memory lifecycle views for handoffs and blockers
//!
//! This keeps memory-related UX parity in one core-owned place instead of
//! duplicating query logic in multiple frontends.

use crate::agent_sessions::{
    AgentSession, AgentSessionStore, FileAgentSessionStore, SessionBlockerStatus, SessionFilter,
    SessionMemoryEntryKind, SessionMemoryPromotionCandidate, SessionMemoryResourceKind,
    SessionWorkingMemory,
};
use crate::error::AppError;
use crate::memory_bank::{
    MemoryBankEntry, MemoryBankError, MemoryBankQuery, MemoryKind, MemoryScope, MemoryType,
    clear_memory_bank, delete_memory_bank_entry, list_memory_bank, load_from_memory_bank,
    save_to_memory_bank, search_memory_bank_with_query, update_memory_bank_entry,
};
use crate::tasks::{TaskError, TaskManager, TaskMemoryEvent, TaskMemoryLifecycle, TaskMemoryPhase};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Result type for memory-console workflows.
pub type MemoryConsoleResult<T> = Result<T, MemoryConsoleError>;

/// Errors produced while inspecting or mutating memory-console state.
#[derive(Debug, thiserror::Error)]
pub enum MemoryConsoleError {
    /// Wrapped memory-bank error.
    #[error(transparent)]
    MemoryBank(#[from] MemoryBankError),
    /// Wrapped task-manager error.
    #[error(transparent)]
    Task(#[from] TaskError),
    /// Wrapped session-store error.
    #[error(transparent)]
    Session(#[from] AppError),
    /// A workspace path is required for memory-bank operations.
    #[error("No workspace directory configured for the selected session")]
    MissingWorkspace,
    /// The requested durable memory entry did not contain an on-disk path.
    #[error("Memory entry is missing its file path")]
    MissingFilePath,
    /// The requested session could not be found.
    #[error("Session not found: {0}")]
    SessionNotFound(String),
    /// The requested task memory lifecycle is missing.
    #[error("Task memory lifecycle not found for task: {0}")]
    TaskLifecycleNotFound(String),
    /// Invalid user input for a console action.
    #[error("Invalid memory console input: {0}")]
    InvalidInput(String),
}

/// Lightweight session summary used by the memory console.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConsoleSessionSummary {
    /// Session id.
    pub session_id: String,
    /// Human-friendly title.
    pub title: String,
    /// Last-activity timestamp.
    pub last_active: DateTime<Utc>,
    /// Persisted message count.
    pub message_count: usize,
    /// Optional workspace path.
    pub workspace_dir: Option<String>,
}

/// A simple count bucket for overview facets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConsoleCount {
    /// Facet key.
    pub key: String,
    /// Number of items in the facet.
    pub count: usize,
}

/// Search filters shared by CLI, GUI, and slash/TUI memory consoles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConsoleQuery {
    /// Optional free-text query.
    pub text: Option<String>,
    /// Maximum number of durable and working-memory matches to return.
    pub limit: usize,
    /// Whether to search current-session working memory.
    pub include_working_memory: bool,
    /// Whether to search durable memory-bank entries.
    pub include_durable_memory: bool,
    /// Whether archived entries should be included.
    pub include_archived: bool,
    /// Optional memory-kind restrictions.
    pub kinds: Vec<MemoryKind>,
    /// Optional memory-type restrictions.
    pub memory_types: Vec<MemoryType>,
    /// Optional memory-scope restrictions.
    pub scopes: Vec<MemoryScope>,
    /// Optional session filter.
    pub session_id: Option<String>,
    /// Optional task filter.
    pub task_id: Option<String>,
    /// Optional directive filter.
    pub directive_id: Option<String>,
    /// Optional agent filter.
    pub agent_id: Option<String>,
    /// Optional category filter.
    pub category: Option<String>,
    /// Optional tags filter.
    pub tags: Vec<String>,
    /// Optional minimum confidence.
    pub min_confidence: Option<f32>,
}

impl Default for MemoryConsoleQuery {
    fn default() -> Self {
        Self {
            text: None,
            limit: 12,
            include_working_memory: true,
            include_durable_memory: true,
            include_archived: false,
            kinds: Vec::new(),
            memory_types: Vec::new(),
            scopes: Vec::new(),
            session_id: None,
            task_id: None,
            directive_id: None,
            agent_id: None,
            category: None,
            tags: Vec::new(),
            min_confidence: None,
        }
    }
}

/// Search hit from session working memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingMemoryMatch {
    /// Stable id scoped to the session snapshot.
    pub id: String,
    /// Working-memory section label.
    pub section: String,
    /// Summary text.
    pub summary: String,
    /// Optional detail text.
    pub detail: Option<String>,
    /// Optional tags associated with the record.
    pub tags: Vec<String>,
    /// Optional status value for blockers.
    pub status: Option<String>,
    /// Ranking score.
    pub score: f32,
}

/// Durable memory summary displayed in lists and search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConsoleEntrySummary {
    /// Stable id used by CLI and GUI actions.
    pub entry_id: String,
    /// Entry timestamp.
    pub timestamp: DateTime<Utc>,
    /// Retention kind.
    pub memory_kind: MemoryKind,
    /// Memory type.
    pub memory_type: MemoryType,
    /// Scope.
    pub scope: MemoryScope,
    /// Owning session id.
    pub session_id: String,
    /// Optional category.
    pub category: Option<String>,
    /// Optional task id.
    pub task_id: Option<String>,
    /// Optional directive id.
    pub directive_id: Option<String>,
    /// Optional agent id.
    pub agent_id: Option<String>,
    /// Tags.
    pub tags: Vec<String>,
    /// Confidence.
    pub confidence: f32,
    /// Summary.
    pub summary: String,
    /// Relative file path.
    pub file_path: Option<String>,
    /// Whether the entry is archived.
    pub archived: bool,
    /// Optional ranking score.
    pub score: Option<f32>,
    /// Optional matched fields.
    pub matched_fields: Vec<String>,
}

/// Full durable memory detail including content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConsoleEntryDetail {
    /// Summary fields used in lists.
    #[serde(flatten)]
    pub summary: MemoryConsoleEntrySummary,
    /// Full markdown content.
    pub content: String,
    /// Promotion provenance session id.
    pub promoted_from_session_id: Option<String>,
    /// Promotion rationale.
    pub promotion_reason: Option<String>,
}

/// Overview payload for the memory console home screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConsoleOverview {
    /// Current workspace path.
    pub workspace_dir: String,
    /// Selected session summary, if any.
    pub session: Option<MemoryConsoleSessionSummary>,
    /// Total durable entries.
    pub durable_total: usize,
    /// Count of open blockers in working memory.
    pub open_blocker_count: usize,
    /// Count of promotion candidates.
    pub promotion_candidate_count: usize,
    /// Count of working-memory resources.
    pub working_resource_count: usize,
    /// Count of working-memory decisions.
    pub working_decision_count: usize,
    /// Working-memory rolling summary.
    pub working_summary: Option<String>,
    /// Recent durable entries.
    pub recent_entries: Vec<MemoryConsoleEntrySummary>,
    /// Facet counts by kind.
    pub counts_by_kind: Vec<MemoryConsoleCount>,
    /// Facet counts by type.
    pub counts_by_type: Vec<MemoryConsoleCount>,
    /// Facet counts by scope.
    pub counts_by_scope: Vec<MemoryConsoleCount>,
}

/// Combined search response for working + durable memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConsoleSearchResponse {
    /// The normalized query that was executed.
    pub query: MemoryConsoleQuery,
    /// Working-memory matches.
    pub working_memory: Vec<WorkingMemoryMatch>,
    /// Durable memory matches.
    pub durable_memory: Vec<MemoryConsoleEntrySummary>,
}

/// Request for promoting a working-memory candidate into durable memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteMemoryCandidateRequest {
    /// Candidate summary.
    pub summary: String,
    /// Optional detail/body.
    pub detail: Option<String>,
    /// Optional category.
    pub category: Option<String>,
    /// Durable memory kind.
    pub memory_kind: MemoryKind,
    /// Durable memory type.
    pub memory_type: MemoryType,
    /// Durable memory scope.
    pub scope: MemoryScope,
    /// Optional task id to associate.
    pub task_id: Option<String>,
    /// Optional directive id to associate.
    pub directive_id: Option<String>,
    /// Optional agent id to associate.
    pub agent_id: Option<String>,
    /// Tags to attach.
    pub tags: Vec<String>,
    /// Confidence to assign.
    pub confidence: f32,
    /// Promotion reason shown in metadata.
    pub promotion_reason: Option<String>,
}

/// Request for updating an existing durable memory entry.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateMemoryEntryRequest {
    /// Replacement summary.
    pub summary: Option<String>,
    /// Replacement markdown body.
    pub content: Option<String>,
    /// Replacement category.
    pub category: Option<Option<String>>,
    /// Replacement kind.
    pub memory_kind: Option<MemoryKind>,
    /// Replacement type.
    pub memory_type: Option<MemoryType>,
    /// Replacement scope.
    pub scope: Option<MemoryScope>,
    /// Replacement task id.
    pub task_id: Option<Option<String>>,
    /// Replacement directive id.
    pub directive_id: Option<Option<String>>,
    /// Replacement agent id.
    pub agent_id: Option<Option<String>>,
    /// Replacement tags.
    pub tags: Option<Vec<String>>,
    /// Replacement confidence.
    pub confidence: Option<f32>,
}

/// Task-memory detail view shared by CLI and GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMemoryConsoleDetail {
    /// The looked-up task id.
    pub task_id: String,
    /// Lifecycle payload recorded in metadata.
    pub lifecycle: TaskMemoryLifecycle,
}

/// List recent sessions for global memory-console browsing.
pub fn list_memory_console_sessions(
    store: &FileAgentSessionStore,
    limit: usize,
) -> MemoryConsoleResult<Vec<MemoryConsoleSessionSummary>> {
    let infos = store.list(SessionFilter::All)?;
    let mut sessions = Vec::new();
    for info in infos.into_iter().take(limit.max(1)) {
        let workspace_dir = store.load(&info.id).ok().and_then(|session| {
            session
                .workspace_dir()
                .map(|path| path.display().to_string())
        });
        sessions.push(MemoryConsoleSessionSummary {
            session_id: info.id,
            title: info.title,
            last_active: info.last_active,
            message_count: info.message_count,
            workspace_dir,
        });
    }
    Ok(sessions)
}

/// Resolve a session id or prefix into a persisted session.
pub fn load_memory_console_session(
    store: &FileAgentSessionStore,
    session_ref: &str,
) -> MemoryConsoleResult<AgentSession> {
    if session_ref == "last" {
        return store
            .load_last()?
            .ok_or_else(|| MemoryConsoleError::SessionNotFound("last".to_string()));
    }

    if let Ok(session) = store.load(session_ref) {
        return Ok(session);
    }

    let Some(resolved) = store.find_by_prefix(session_ref)? else {
        return Err(MemoryConsoleError::SessionNotFound(session_ref.to_string()));
    };
    Ok(store.load(&resolved)?)
}

/// Build a memory-console overview for a workspace and optional session.
pub async fn get_memory_console_overview(
    workspace_dir: &Path,
    session: Option<&AgentSession>,
) -> MemoryConsoleResult<MemoryConsoleOverview> {
    let entries = list_memory_bank(workspace_dir).await?;
    let filtered_entries: Vec<MemoryBankEntry> = entries
        .into_iter()
        .filter(|entry| !is_archived(entry))
        .collect();

    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut scopes: BTreeMap<String, usize> = BTreeMap::new();
    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &filtered_entries {
        *kinds.entry(entry.memory_kind.to_string()).or_default() += 1;
        *scopes.entry(entry.scope.to_string()).or_default() += 1;
        *types.entry(entry.memory_type.to_string()).or_default() += 1;
    }

    let session_summary = session.map(session_summary_from_session);
    let working_memory = session.map(|current| &current.state.working_memory);
    let recent_entries = filtered_entries
        .iter()
        .take(8)
        .map(|entry| entry_summary_from_entry(workspace_dir, entry, None, Vec::new()))
        .collect();

    Ok(MemoryConsoleOverview {
        workspace_dir: workspace_dir.display().to_string(),
        session: session_summary,
        durable_total: filtered_entries.len(),
        open_blocker_count: working_memory
            .map(|wm| {
                wm.blockers
                    .iter()
                    .filter(|b| matches!(b.status, SessionBlockerStatus::Open))
                    .count()
            })
            .unwrap_or(0),
        promotion_candidate_count: session
            .map(|s| s.state.promotion_candidates(12).len())
            .unwrap_or(0),
        working_resource_count: working_memory.map(|wm| wm.resources.len()).unwrap_or(0),
        working_decision_count: working_memory.map(|wm| wm.decisions.len()).unwrap_or(0),
        working_summary: working_memory.and_then(|wm| wm.summary.clone()),
        recent_entries,
        counts_by_kind: counts_to_vec(kinds),
        counts_by_type: counts_to_vec(types),
        counts_by_scope: counts_to_vec(scopes),
    })
}

/// Return the current session working-memory snapshot.
pub fn get_working_memory_snapshot(session: &AgentSession) -> SessionWorkingMemory {
    session.state.working_memory.clone()
}

/// Return promotion candidates for the selected session.
pub fn get_memory_promotion_candidates(
    session: &AgentSession,
    limit: usize,
) -> Vec<SessionMemoryPromotionCandidate> {
    session.state.promotion_candidates(limit.max(1))
}

/// Look up a durable memory entry by CLI/GUI id.
pub async fn get_memory_entry_detail(
    workspace_dir: &Path,
    entry_id: &str,
) -> MemoryConsoleResult<MemoryConsoleEntryDetail> {
    let path = resolve_entry_id(workspace_dir, entry_id);
    let entry = load_from_memory_bank(&path).await?;
    Ok(entry_detail_from_entry(
        workspace_dir,
        &entry,
        None,
        Vec::new(),
    ))
}

/// Search across working memory and durable memory using shared filters.
pub async fn search_memory_console(
    workspace_dir: &Path,
    session: Option<&AgentSession>,
    mut query: MemoryConsoleQuery,
) -> MemoryConsoleResult<MemoryConsoleSearchResponse> {
    query.limit = query.limit.max(1);

    let working_memory = if query.include_working_memory {
        session
            .map(|current| search_working_memory(current, &query))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let durable_memory = if query.include_durable_memory {
        let bank_query = MemoryBankQuery {
            text: query.text.clone(),
            limit: query.limit,
            kinds: query.kinds.clone(),
            memory_types: query.memory_types.clone(),
            scopes: query.scopes.clone(),
            session_id: query.session_id.clone(),
            task_id: query.task_id.clone(),
            directive_id: query.directive_id.clone(),
            agent_id: query.agent_id.clone(),
            category: query.category.clone(),
            tags: query.tags.clone(),
            min_confidence: query.min_confidence,
        };

        search_memory_bank_with_query(workspace_dir, &bank_query)
            .await?
            .into_iter()
            .filter(|result| query.include_archived || !is_archived(&result.entry))
            .map(|result| {
                entry_summary_from_entry(
                    workspace_dir,
                    &result.entry,
                    Some(result.score),
                    result.matched_fields,
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(MemoryConsoleSearchResponse {
        query,
        working_memory,
        durable_memory,
    })
}

/// Read task-local memory lifecycle information.
pub fn get_task_memory_console_detail(
    task_manager: &TaskManager,
    session_id: &str,
    task_id: &str,
) -> MemoryConsoleResult<TaskMemoryConsoleDetail> {
    let Some(lifecycle) = task_manager.get_memory_lifecycle(session_id, task_id)? else {
        return Err(MemoryConsoleError::TaskLifecycleNotFound(
            task_id.to_string(),
        ));
    };
    Ok(TaskMemoryConsoleDetail {
        task_id: task_id.to_string(),
        lifecycle,
    })
}

/// Promote a working-memory candidate into durable memory.
pub async fn promote_memory_candidate(
    workspace_dir: &Path,
    session: &AgentSession,
    request: PromoteMemoryCandidateRequest,
    task_manager: Option<&TaskManager>,
) -> MemoryConsoleResult<MemoryConsoleEntryDetail> {
    let mut entry = MemoryBankEntry::new(
        session.id.clone(),
        request.summary.clone(),
        request
            .detail
            .clone()
            .unwrap_or_else(|| request.summary.clone()),
    )
    .with_memory_type(request.memory_type)
    .with_scope(request.scope)
    .with_provenance(
        request.task_id.clone(),
        request.directive_id.clone(),
        request.agent_id.clone(),
    )
    .with_tags(normalize_tags(request.tags))
    .with_confidence(request.confidence);

    entry.memory_kind = request.memory_kind;
    entry.category = request.category.clone();
    let promotion_reason = request
        .promotion_reason
        .clone()
        .unwrap_or_else(|| format!("Promoted from working memory ({})", session.id));
    entry = entry.with_promotion(session.id.clone(), promotion_reason);

    let saved_path = save_to_memory_bank(workspace_dir, &entry).await?;
    let saved = load_from_memory_bank(&saved_path).await?;

    if let (Some(manager), Some(task_id)) = (task_manager, request.task_id.as_deref()) {
        manager.record_memory_event(
            &session.id,
            task_id,
            TaskMemoryEvent::new(
                TaskMemoryPhase::Promoted,
                format!("Promoted durable memory: {}", saved.summary),
                Some(saved.scope.to_string()),
                Some(saved.memory_type.to_string()),
                Some(memory_entry_id(workspace_dir, &saved)),
            ),
        )?;
    }

    Ok(entry_detail_from_entry(
        workspace_dir,
        &saved,
        None,
        Vec::new(),
    ))
}

/// Update a durable memory entry in place.
pub async fn update_memory_entry_detail(
    workspace_dir: &Path,
    entry_id: &str,
    request: UpdateMemoryEntryRequest,
) -> MemoryConsoleResult<MemoryConsoleEntryDetail> {
    let path = resolve_entry_id(workspace_dir, entry_id);
    let mut entry = load_from_memory_bank(&path).await?;

    if let Some(summary) = request.summary {
        entry.summary = summary;
    }
    if let Some(content) = request.content {
        entry.content = content;
    }
    if let Some(category) = request.category {
        entry.category = category;
    }
    if let Some(memory_kind) = request.memory_kind {
        entry.memory_kind = memory_kind;
    }
    if let Some(memory_type) = request.memory_type {
        entry.memory_type = memory_type;
    }
    if let Some(scope) = request.scope {
        entry.scope = scope;
    }
    if let Some(task_id) = request.task_id {
        entry.task_id = task_id;
    }
    if let Some(directive_id) = request.directive_id {
        entry.directive_id = directive_id;
    }
    if let Some(agent_id) = request.agent_id {
        entry.agent_id = agent_id;
    }
    if let Some(tags) = request.tags {
        entry.tags = normalize_tags(tags);
    }
    if let Some(confidence) = request.confidence {
        entry.confidence = confidence.clamp(0.0, 1.0);
    }

    let Some(file_path) = entry.file_path.clone() else {
        return Err(MemoryConsoleError::MissingFilePath);
    };
    update_memory_bank_entry(workspace_dir, &file_path, &entry).await?;

    let reloaded = load_from_memory_bank(&file_path).await?;
    Ok(entry_detail_from_entry(
        workspace_dir,
        &reloaded,
        None,
        Vec::new(),
    ))
}

/// Archive or unarchive a durable memory entry.
pub async fn set_memory_entry_archived(
    workspace_dir: &Path,
    entry_id: &str,
    archived: bool,
) -> MemoryConsoleResult<MemoryConsoleEntryDetail> {
    let path = resolve_entry_id(workspace_dir, entry_id);
    let mut entry = load_from_memory_bank(&path).await?;
    let mut tags = entry.tags.clone();
    if archived {
        if !tags.iter().any(|tag| tag.eq_ignore_ascii_case("archived")) {
            tags.push("archived".to_string());
        }
    } else {
        tags.retain(|tag| !tag.eq_ignore_ascii_case("archived"));
    }
    entry.tags = normalize_tags(tags);

    let Some(file_path) = entry.file_path.clone() else {
        return Err(MemoryConsoleError::MissingFilePath);
    };
    update_memory_bank_entry(workspace_dir, &file_path, &entry).await?;

    let reloaded = load_from_memory_bank(&file_path).await?;
    Ok(entry_detail_from_entry(
        workspace_dir,
        &reloaded,
        None,
        Vec::new(),
    ))
}

/// Delete a durable memory entry by id.
pub async fn delete_memory_entry_by_id(
    workspace_dir: &Path,
    entry_id: &str,
) -> MemoryConsoleResult<()> {
    delete_memory_bank_entry(workspace_dir, &resolve_entry_id(workspace_dir, entry_id)).await?;
    Ok(())
}

/// Clear all durable memory entries in the workspace.
pub async fn clear_memory_console(workspace_dir: &Path) -> MemoryConsoleResult<usize> {
    Ok(clear_memory_bank(workspace_dir).await?)
}

fn search_working_memory(
    session: &AgentSession,
    query: &MemoryConsoleQuery,
) -> Vec<WorkingMemoryMatch> {
    let mut matches = Vec::new();
    let working = &session.state.working_memory;
    let query_text = query
        .text
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let has_query = !query_text.is_empty();

    if let Some(summary) = &working.summary {
        push_working_match(
            &mut matches,
            has_query,
            &query_text,
            WorkingMemoryMatch {
                id: "summary".to_string(),
                section: "summary".to_string(),
                summary: summary.clone(),
                detail: None,
                tags: vec!["summary".to_string()],
                status: None,
                score: 2.5,
            },
        );
    }

    for resource in &working.resources {
        push_working_match(
            &mut matches,
            has_query,
            &query_text,
            WorkingMemoryMatch {
                id: resource.id.clone(),
                section: "resource".to_string(),
                summary: format!("{}: {}", resource_kind_label(resource.kind), resource.label),
                detail: Some(resource.value.clone()),
                tags: vec![
                    resource_kind_label(resource.kind).to_string(),
                    resource.source.clone(),
                ],
                status: None,
                score: 1.4,
            },
        );
    }

    for decision in &working.decisions {
        push_working_match(
            &mut matches,
            has_query,
            &query_text,
            WorkingMemoryMatch {
                id: decision.id.clone(),
                section: "decision".to_string(),
                summary: decision.summary.clone(),
                detail: decision.rationale.clone(),
                tags: decision.tags.clone(),
                status: None,
                score: 3.2,
            },
        );
    }

    for blocker in &working.blockers {
        push_working_match(
            &mut matches,
            has_query,
            &query_text,
            WorkingMemoryMatch {
                id: blocker.id.clone(),
                section: "blocker".to_string(),
                summary: blocker.summary.clone(),
                detail: blocker.detail.clone(),
                tags: vec!["blocker".to_string()],
                status: Some(blocker_status_label(blocker.status).to_string()),
                score: if matches!(blocker.status, SessionBlockerStatus::Open) {
                    3.8
                } else {
                    1.0
                },
            },
        );
    }

    for (index, action) in working.next_actions.iter().enumerate() {
        push_working_match(
            &mut matches,
            has_query,
            &query_text,
            WorkingMemoryMatch {
                id: format!("next-action-{index}"),
                section: "next_action".to_string(),
                summary: action.clone(),
                detail: None,
                tags: vec!["next_action".to_string()],
                status: None,
                score: 2.1,
            },
        );
    }

    for (index, question) in working.open_questions.iter().enumerate() {
        push_working_match(
            &mut matches,
            has_query,
            &query_text,
            WorkingMemoryMatch {
                id: format!("open-question-{index}"),
                section: "open_question".to_string(),
                summary: question.clone(),
                detail: None,
                tags: vec!["open_question".to_string()],
                status: None,
                score: 1.8,
            },
        );
    }

    for entry in &working.timeline {
        push_working_match(
            &mut matches,
            has_query,
            &query_text,
            WorkingMemoryMatch {
                id: entry.id.clone(),
                section: "timeline".to_string(),
                summary: entry.summary.clone(),
                detail: entry.detail.clone(),
                tags: vec![timeline_kind_label(entry.kind).to_string()],
                status: None,
                score: 1.3,
            },
        );
    }

    matches.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.summary.cmp(&b.summary))
    });
    matches.truncate(query.limit);
    matches
}

fn push_working_match(
    matches: &mut Vec<WorkingMemoryMatch>,
    has_query: bool,
    query: &str,
    mut candidate: WorkingMemoryMatch,
) {
    if !has_query {
        matches.push(candidate);
        return;
    }

    let haystack = format!(
        "{} {} {}",
        candidate.section,
        candidate.summary,
        candidate.detail.clone().unwrap_or_default()
    )
    .to_ascii_lowercase();

    if haystack.contains(query)
        || candidate
            .tags
            .iter()
            .any(|tag| tag.to_ascii_lowercase().contains(query))
    {
        candidate.score += 1.5;
        matches.push(candidate);
    }
}

fn entry_summary_from_entry(
    workspace_dir: &Path,
    entry: &MemoryBankEntry,
    score: Option<f32>,
    matched_fields: Vec<String>,
) -> MemoryConsoleEntrySummary {
    MemoryConsoleEntrySummary {
        entry_id: memory_entry_id(workspace_dir, entry),
        timestamp: entry.timestamp,
        memory_kind: entry.memory_kind,
        memory_type: entry.memory_type,
        scope: entry.scope,
        session_id: entry.session_id.clone(),
        category: entry.category.clone(),
        task_id: entry.task_id.clone(),
        directive_id: entry.directive_id.clone(),
        agent_id: entry.agent_id.clone(),
        tags: entry.tags.clone(),
        confidence: entry.confidence,
        summary: entry.summary.clone(),
        file_path: entry
            .file_path
            .as_ref()
            .map(|path| path_to_id(workspace_dir, path)),
        archived: is_archived(entry),
        score,
        matched_fields,
    }
}

fn entry_detail_from_entry(
    workspace_dir: &Path,
    entry: &MemoryBankEntry,
    score: Option<f32>,
    matched_fields: Vec<String>,
) -> MemoryConsoleEntryDetail {
    MemoryConsoleEntryDetail {
        summary: entry_summary_from_entry(workspace_dir, entry, score, matched_fields),
        content: entry.content.clone(),
        promoted_from_session_id: entry.promoted_from_session_id.clone(),
        promotion_reason: entry.promotion_reason.clone(),
    }
}

fn session_summary_from_session(session: &AgentSession) -> MemoryConsoleSessionSummary {
    MemoryConsoleSessionSummary {
        session_id: session.id.clone(),
        title: session.title.clone(),
        last_active: session.last_active,
        message_count: session.message_count(),
        workspace_dir: session
            .workspace_dir()
            .map(|path| path.display().to_string()),
    }
}

fn counts_to_vec(counts: BTreeMap<String, usize>) -> Vec<MemoryConsoleCount> {
    counts
        .into_iter()
        .map(|(key, count)| MemoryConsoleCount { key, count })
        .collect()
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !normalized
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
        {
            normalized.push(trimmed.to_string());
        }
    }
    normalized
}

fn is_archived(entry: &MemoryBankEntry) -> bool {
    entry
        .tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case("archived"))
}

fn memory_entry_id(workspace_dir: &Path, entry: &MemoryBankEntry) -> String {
    entry
        .file_path
        .as_ref()
        .map(|path| path_to_id(workspace_dir, path))
        .unwrap_or_else(|| {
            format!(
                ".gestura/memory/{}_{}.md",
                entry.timestamp.format("%Y%m%d%H%M%S"),
                entry.session_id
            )
        })
}

fn path_to_id(workspace_dir: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn resolve_entry_id(workspace_dir: &Path, entry_id: &str) -> PathBuf {
    let path = Path::new(entry_id);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_dir.join(path)
    }
}

fn resource_kind_label(kind: SessionMemoryResourceKind) -> &'static str {
    match kind {
        SessionMemoryResourceKind::Message => "message",
        SessionMemoryResourceKind::ToolCall => "tool_call",
        SessionMemoryResourceKind::File => "file",
        SessionMemoryResourceKind::Command => "command",
        SessionMemoryResourceKind::Web => "web",
        SessionMemoryResourceKind::Task => "task",
        SessionMemoryResourceKind::Knowledge => "knowledge",
        SessionMemoryResourceKind::Other => "other",
    }
}

fn blocker_status_label(status: SessionBlockerStatus) -> &'static str {
    match status {
        SessionBlockerStatus::Open => "open",
        SessionBlockerStatus::Resolved => "resolved",
    }
}

fn timeline_kind_label(kind: SessionMemoryEntryKind) -> &'static str {
    match kind {
        SessionMemoryEntryKind::UserGoal => "user_goal",
        SessionMemoryEntryKind::AssistantSummary => "assistant_summary",
        SessionMemoryEntryKind::ToolInsight => "tool_insight",
        SessionMemoryEntryKind::Handoff => "handoff",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_sessions::AgentSession;
    use tempfile::tempdir;

    #[tokio::test]
    async fn search_memory_console_returns_working_and_durable_matches() {
        let temp = tempdir().unwrap();
        let mut session =
            AgentSession::new_with_workspace(temp.path().to_path_buf(), None).unwrap();
        session.state.remember_decision(
            "Adopt memory console",
            Some("Parity first".to_string()),
            vec!["memory".to_string()],
        );

        let entry = MemoryBankEntry::new(
            session.id.clone(),
            "Memory console promoted decision".to_string(),
            "Durable detail".to_string(),
        )
        .with_memory_type(MemoryType::Decision)
        .with_scope(MemoryScope::Directive)
        .with_tags(vec!["memory".to_string(), "console".to_string()]);
        save_to_memory_bank(temp.path(), &entry).await.unwrap();

        let results = search_memory_console(
            temp.path(),
            Some(&session),
            MemoryConsoleQuery {
                text: Some("memory".to_string()),
                ..MemoryConsoleQuery::default()
            },
        )
        .await
        .unwrap();

        assert!(!results.working_memory.is_empty());
        assert!(!results.durable_memory.is_empty());
    }

    #[tokio::test]
    async fn archive_filters_out_entries_by_default() {
        let temp = tempdir().unwrap();
        let session = AgentSession::new_with_workspace(temp.path().to_path_buf(), None).unwrap();
        let entry = MemoryBankEntry::new(
            session.id.clone(),
            "Archived memory".to_string(),
            "Hidden from default search".to_string(),
        )
        .with_tags(vec!["archived".to_string()]);
        let path = save_to_memory_bank(temp.path(), &entry).await.unwrap();

        let default_results =
            search_memory_console(temp.path(), Some(&session), MemoryConsoleQuery::default())
                .await
                .unwrap();
        assert!(default_results.durable_memory.is_empty());

        let detail = set_memory_entry_archived(
            temp.path(),
            &path.strip_prefix(temp.path()).unwrap().to_string_lossy(),
            false,
        )
        .await
        .unwrap();
        assert!(!detail.summary.archived);
    }
}
