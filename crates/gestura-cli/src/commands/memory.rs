//! Memory console command and interactive browser.

use crate::{MemoryAction, MemoryCommand};
use dialoguer::{Confirm, Editor, Input, Select, theme::ColorfulTheme};
use gestura_core::agent_sessions::{
    AgentSession, AgentSessionStore, FileAgentSessionStore, SessionMemoryPromotionCandidate,
    SessionMemoryPromotionSource,
};
use gestura_core::memory_bank::{MemoryGovernanceState, MemoryKind, MemoryScope, MemoryType};
use gestura_core::memory_console::{
    MemoryConsoleQuery, PromoteMemoryCandidateRequest, UpdateMemoryEntryRequest,
    clear_memory_console, delete_memory_entry_by_id, get_memory_console_overview,
    get_memory_entry_detail, get_memory_promotion_candidates, get_task_memory_console_detail,
    get_working_memory_snapshot, list_memory_console_sessions, load_memory_console_session,
    promote_memory_candidate, refresh_memory_console_governance, search_memory_console,
    set_memory_entry_archived, update_memory_entry_detail,
};
use gestura_core::tasks::TaskManager;
use serde::Serialize;
use std::path::PathBuf;

type Result<T> = crate::commands::Result<T>;

struct MemoryCommandContext {
    workspace_dir: PathBuf,
    session: Option<AgentSession>,
    task_manager: TaskManager,
    store: FileAgentSessionStore,
}

pub async fn run(command: MemoryCommand) -> Result<()> {
    let mut context = build_context(command.session.clone(), command.workspace.clone())?;
    match command.action {
        Some(action) => run_action(action, &mut context).await,
        None => browse_memory_console(&mut context).await,
    }
}

pub async fn browse_session_memory(session: &AgentSession) -> Result<()> {
    let workspace_dir = session
        .workspace_dir()
        .cloned()
        .ok_or_else(|| "Current session has no workspace directory".to_string())?;
    let store = FileAgentSessionStore::default();
    let task_manager = TaskManager::new(&workspace_dir);
    let mut context = MemoryCommandContext {
        workspace_dir,
        session: Some(session.clone()),
        task_manager,
        store,
    };
    browse_memory_console(&mut context).await
}

async fn run_action(action: MemoryAction, context: &mut MemoryCommandContext) -> Result<()> {
    match action {
        MemoryAction::Sessions { limit, json } => {
            let sessions = list_memory_console_sessions(&context.store, limit)?;
            print_output(&sessions, json);
        }
        MemoryAction::Overview { json } => {
            let overview =
                get_memory_console_overview(&context.workspace_dir, context.session.as_ref())
                    .await?;
            print_output(&overview, json);
        }
        MemoryAction::Search {
            query,
            include_archived,
            limit,
            kind,
            memory_type,
            scope,
            tag,
            task,
            directive,
            agent,
            category,
            json,
        } => {
            let results = search_memory_console(
                &context.workspace_dir,
                context.session.as_ref(),
                build_query(
                    query,
                    include_archived,
                    limit,
                    kind,
                    memory_type,
                    scope,
                    tag,
                    task,
                    directive,
                    agent,
                    category,
                )?,
            )
            .await?;
            print_output(&results, json);
        }
        MemoryAction::Working { json } => {
            let session = require_session(context)?;
            let snapshot = get_working_memory_snapshot(&session);
            print_output(&snapshot, json);
        }
        MemoryAction::Promotions { limit, json } => {
            let session = require_session(context)?;
            let promotions = get_memory_promotion_candidates(&session, limit);
            print_output(&promotions, json);
        }
        MemoryAction::Task { task_id, json } => {
            let session = require_session(context)?;
            let detail =
                get_task_memory_console_detail(&context.task_manager, &session.id, &task_id)?;
            print_output(&detail, json);
        }
        MemoryAction::Show { entry_id, json } => {
            let detail = get_memory_entry_detail(&context.workspace_dir, &entry_id).await?;
            print_output(&detail, json);
        }
        MemoryAction::Promote {
            summary,
            detail,
            category,
            kind,
            memory_type,
            scope,
            task,
            directive,
            agent,
            tag,
            confidence,
            reason,
            json,
        } => {
            let session = require_session(context)?;
            let detail = promote_memory_candidate(
                &context.workspace_dir,
                &session,
                PromoteMemoryCandidateRequest {
                    summary,
                    detail,
                    category,
                    memory_kind: parse_kind(&kind)?,
                    memory_type: parse_type(&memory_type)?,
                    scope: parse_scope(&scope)?,
                    task_id: task,
                    directive_id: directive,
                    agent_id: agent,
                    tags: tag,
                    confidence,
                    promotion_reason: reason,
                },
                Some(&context.task_manager),
            )
            .await?;
            print_output(&detail, json);
        }
        MemoryAction::Edit {
            entry_id,
            summary,
            content,
            category,
            clear_category,
            kind,
            memory_type,
            scope,
            task,
            clear_task,
            directive,
            clear_directive,
            agent,
            clear_agent,
            tags,
            confidence,
            governance_state,
            governance_note,
            clear_governance_note,
            json,
        } => {
            let detail = update_memory_entry_detail(
                &context.workspace_dir,
                &entry_id,
                UpdateMemoryEntryRequest {
                    summary,
                    content,
                    category: category
                        .map(Some)
                        .or_else(|| clear_category.then_some(None)),
                    memory_kind: kind.map(|value| parse_kind(&value)).transpose()?,
                    memory_type: memory_type.map(|value| parse_type(&value)).transpose()?,
                    scope: scope.map(|value| parse_scope(&value)).transpose()?,
                    task_id: task.map(Some).or_else(|| clear_task.then_some(None)),
                    directive_id: directive
                        .map(Some)
                        .or_else(|| clear_directive.then_some(None)),
                    agent_id: agent.map(Some).or_else(|| clear_agent.then_some(None)),
                    tags,
                    confidence,
                    governance_state: governance_state
                        .map(|value| parse_governance_state(&value))
                        .transpose()?,
                    governance_note: governance_note
                        .map(Some)
                        .or_else(|| clear_governance_note.then_some(None)),
                },
            )
            .await?;
            print_output(&detail, json);
        }
        MemoryAction::RefreshGovernance { json } => {
            let report = refresh_memory_console_governance(&context.workspace_dir).await?;
            print_output(&report, json);
        }
        MemoryAction::Archive {
            entry_id,
            restore,
            json,
        } => {
            let detail =
                set_memory_entry_archived(&context.workspace_dir, &entry_id, !restore).await?;
            print_output(&detail, json);
        }
        MemoryAction::Delete { entry_id, yes } => {
            if yes
                || Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt(format!("Delete memory entry `{entry_id}`?"))
                    .interact()?
            {
                delete_memory_entry_by_id(&context.workspace_dir, &entry_id).await?;
                println!("Deleted `{entry_id}`.");
            }
        }
        MemoryAction::Clear { yes } => {
            if yes
                || Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Clear all durable memory for this workspace?")
                    .interact()?
            {
                let deleted = clear_memory_console(&context.workspace_dir).await?;
                println!("Cleared {deleted} memory entries.");
            }
        }
    }
    Ok(())
}

async fn browse_memory_console(context: &mut MemoryCommandContext) -> Result<()> {
    let theme = ColorfulTheme::default();
    loop {
        let selection = Select::with_theme(&theme)
            .with_prompt("Memory console")
            .items(&[
                "Overview",
                "Search",
                "Working Memory",
                "Durable Memory",
                "Promotions",
                "Task Memory",
                "Maintenance",
                "Exit",
            ])
            .default(0)
            .interact()?;

        match selection {
            0 => show_overview(context).await?,
            1 => search_browser(context).await?,
            2 => show_working_memory(context)?,
            3 => durable_memory_browser(context).await?,
            4 => promotions_browser(context).await?,
            5 => task_memory_browser(context)?,
            6 => maintenance_browser(context).await?,
            _ => break,
        }
    }
    Ok(())
}

async fn show_overview(context: &MemoryCommandContext) -> Result<()> {
    let overview =
        get_memory_console_overview(&context.workspace_dir, context.session.as_ref()).await?;
    println!("\nWorkspace: {}", overview.workspace_dir);
    if let Some(session) = overview.session {
        println!("Session: {} ({})", session.title, session.session_id);
    }
    println!("Durable entries: {}", overview.durable_total);
    println!("Working resources: {}", overview.working_resource_count);
    println!("Working decisions: {}", overview.working_decision_count);
    println!("Open blockers: {}", overview.open_blocker_count);
    println!(
        "Promotion candidates: {}",
        overview.promotion_candidate_count
    );
    println!(
        "Governance review / issues: {} / {}",
        overview.governance_review_count, overview.governance_issue_count
    );
    if let Some(summary) = overview.working_summary {
        println!("Working summary: {summary}");
    }
    pause()?;
    Ok(())
}

async fn search_browser(context: &mut MemoryCommandContext) -> Result<()> {
    let theme = ColorfulTheme::default();
    let query_text: String = Input::with_theme(&theme)
        .with_prompt("Search memory")
        .allow_empty(true)
        .interact_text()?;
    let include_archived = Confirm::with_theme(&theme)
        .with_prompt("Include archived durable entries?")
        .default(false)
        .interact()?;
    let results = search_memory_console(
        &context.workspace_dir,
        context.session.as_ref(),
        MemoryConsoleQuery {
            text: (!query_text.trim().is_empty()).then_some(query_text.trim().to_string()),
            include_archived,
            ..MemoryConsoleQuery::default()
        },
    )
    .await?;

    let mut items: Vec<(String, Option<String>, bool)> = Vec::new();
    for working in &results.working_memory {
        items.push((
            format!("[working:{}] {}", working.section, working.summary),
            None,
            false,
        ));
    }
    for durable in &results.durable_memory {
        items.push((
            format!("[durable:{}] {}", durable.scope, durable.summary),
            Some(durable.entry_id.clone()),
            true,
        ));
    }

    if items.is_empty() {
        println!("No matches found.");
        pause()?;
        return Ok(());
    }

    let labels: Vec<String> = items.iter().map(|(label, _, _)| label.clone()).collect();
    let selected = Select::with_theme(&theme)
        .with_prompt("Search results")
        .items(&labels)
        .default(0)
        .interact()?;

    let (label, entry_id, is_durable) = &items[selected];
    println!("\n{label}");
    if *is_durable {
        if let Some(entry_id) = entry_id {
            durable_entry_actions(context, entry_id).await?;
        }
    } else if let Some(item) = results.working_memory.get(selected) {
        if let Some(detail) = &item.detail {
            println!("{detail}");
        }
        pause()?;
    }

    Ok(())
}

fn show_working_memory(context: &MemoryCommandContext) -> Result<()> {
    let session = require_session_ref(context)?;
    let snapshot = get_working_memory_snapshot(session);
    println!(
        "\nWorking memory summary: {}",
        snapshot.summary.unwrap_or_else(|| "(none)".to_string())
    );
    println!("Resources: {}", snapshot.resources.len());
    println!("Decisions: {}", snapshot.decisions.len());
    println!("Blockers: {}", snapshot.blockers.len());
    println!("Next actions: {}", snapshot.next_actions.len());
    println!("Open questions: {}", snapshot.open_questions.len());
    pause()?;
    Ok(())
}

async fn durable_memory_browser(context: &mut MemoryCommandContext) -> Result<()> {
    let results = search_memory_console(
        &context.workspace_dir,
        context.session.as_ref(),
        MemoryConsoleQuery {
            include_working_memory: false,
            ..MemoryConsoleQuery::default()
        },
    )
    .await?;
    let entries = results.durable_memory;
    if entries.is_empty() {
        println!("No durable memory entries found.");
        pause()?;
        return Ok(());
    }

    let labels: Vec<String> = entries
        .iter()
        .map(|entry| {
            format!(
                "{} [{} / {}]",
                entry.summary, entry.scope, entry.memory_type
            )
        })
        .collect();
    let selected = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Durable memory")
        .items(&labels)
        .default(0)
        .interact()?;
    durable_entry_actions(context, &entries[selected].entry_id).await
}

async fn promotions_browser(context: &mut MemoryCommandContext) -> Result<()> {
    let session = require_session_ref(context)?;
    let candidates = get_memory_promotion_candidates(session, 12);
    if candidates.is_empty() {
        println!("No promotion candidates are currently available.");
        pause()?;
        return Ok(());
    }

    let labels: Vec<String> = candidates
        .iter()
        .map(|candidate| {
            format!(
                "{} [{} / {}]",
                candidate.summary,
                promotion_source_label(candidate.source),
                promotion_candidate_memory_type(candidate)
            )
        })
        .collect();
    let selected = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Promotion candidates")
        .items(&labels)
        .default(0)
        .interact()?;
    let candidate = &candidates[selected];

    println!("\n{}", candidate.summary);
    if let Some(detail) = &candidate.detail {
        println!("{detail}");
    }
    if !Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Promote this candidate?")
        .default(true)
        .interact()?
    {
        return Ok(());
    }

    let tags = pick_tags(&candidate.tags)?;
    let detail = promote_memory_candidate(
        &context.workspace_dir,
        session,
        PromoteMemoryCandidateRequest {
            summary: candidate.summary.clone(),
            detail: candidate.detail.clone(),
            category: None,
            memory_kind: MemoryKind::LongTerm,
            memory_type: promotion_candidate_memory_type(candidate),
            scope: promotion_candidate_scope(candidate),
            task_id: None,
            directive_id: None,
            agent_id: None,
            tags,
            confidence: promotion_candidate_confidence(candidate),
            promotion_reason: Some("Promoted from interactive /memory console".to_string()),
        },
        Some(&context.task_manager),
    )
    .await?;
    println!("Promoted {}", detail.summary.summary);
    pause()?;
    Ok(())
}

fn task_memory_browser(context: &MemoryCommandContext) -> Result<()> {
    let session = require_session_ref(context)?;
    let task_id: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Task id")
        .interact_text()?;
    let detail = get_task_memory_console_detail(&context.task_manager, &session.id, &task_id)?;
    println!("\nTask memory for {}", detail.task_id);
    println!("Events: {}", detail.lifecycle.events.len());
    println!(
        "Latest durable memory: {}",
        detail
            .lifecycle
            .last_memory_file_path
            .as_deref()
            .unwrap_or("(none)")
    );
    pause()?;
    Ok(())
}

async fn maintenance_browser(context: &mut MemoryCommandContext) -> Result<()> {
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Maintenance")
        .items(&[
            "List recent sessions",
            "Refresh governance suggestions",
            "Clear durable memory",
            "Back",
        ])
        .default(0)
        .interact()?;
    match selection {
        0 => {
            let sessions = list_memory_console_sessions(&context.store, 10)?;
            for session in sessions {
                println!("{} — {}", session.session_id, session.title);
            }
            pause()?;
        }
        1 => {
            let report = refresh_memory_console_governance(&context.workspace_dir).await?;
            println!(
                "Refreshed governance: {} scanned, {} updated, {} duplicate, {} conflict, {} superseded suggestions.",
                report.entries_scanned,
                report.updated_entries,
                report.duplicate_suggestions,
                report.conflict_suggestions,
                report.superseded_suggestions,
            );
            pause()?;
        }
        2 => {
            if Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Clear all durable memory entries for this workspace?")
                .default(false)
                .interact()?
            {
                let deleted = clear_memory_console(&context.workspace_dir).await?;
                println!("Cleared {deleted} entries.");
                pause()?;
            }
        }
        _ => {}
    }
    Ok(())
}

async fn durable_entry_actions(context: &mut MemoryCommandContext, entry_id: &str) -> Result<()> {
    loop {
        let detail = get_memory_entry_detail(&context.workspace_dir, entry_id).await?;
        println!("\n{}", detail.summary.summary);
        println!(
            "Type: {} / Scope: {}",
            detail.summary.memory_type, detail.summary.scope
        );
        println!(
            "Governance: {}{}",
            format_governance_state(detail.summary.governance_state),
            if detail.summary.governance_issue_count == 0 {
                String::new()
            } else {
                format!(" ({} suggestions)", detail.summary.governance_issue_count)
            }
        );
        if let Some(note) = &detail.governance_note {
            println!("Governance note: {note}");
        }
        println!("Tags: {}", detail.summary.tags.join(", "));
        if !detail.governance_suggestions.is_empty() {
            println!("Suggestions:");
            for suggestion in &detail.governance_suggestions {
                println!(
                    "  - {} {} ({:.0}%): {}",
                    suggestion.relationship,
                    suggestion.entry_id,
                    suggestion.confidence * 100.0,
                    suggestion.rationale,
                );
            }
        }
        println!("\n{}", detail.content);

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Entry actions")
            .items(&[
                "Edit",
                "Pin",
                "Mark needs review",
                "Mark superseded",
                "Mark active",
                if detail.summary.archived {
                    "Restore"
                } else {
                    "Archive"
                },
                "Refresh governance",
                "Delete",
                "Back",
            ])
            .default(0)
            .interact()?;
        match selection {
            0 => {
                let updated = edit_entry_interactively(&detail)?;
                update_memory_entry_detail(&context.workspace_dir, entry_id, updated).await?;
            }
            1 => {
                update_memory_entry_detail(
                    &context.workspace_dir,
                    entry_id,
                    UpdateMemoryEntryRequest {
                        governance_state: Some(MemoryGovernanceState::Pinned),
                        ..UpdateMemoryEntryRequest::default()
                    },
                )
                .await?;
            }
            2 => {
                update_memory_entry_detail(
                    &context.workspace_dir,
                    entry_id,
                    UpdateMemoryEntryRequest {
                        governance_state: Some(MemoryGovernanceState::NeedsReview),
                        ..UpdateMemoryEntryRequest::default()
                    },
                )
                .await?;
            }
            3 => {
                update_memory_entry_detail(
                    &context.workspace_dir,
                    entry_id,
                    UpdateMemoryEntryRequest {
                        governance_state: Some(MemoryGovernanceState::Superseded),
                        ..UpdateMemoryEntryRequest::default()
                    },
                )
                .await?;
            }
            4 => {
                update_memory_entry_detail(
                    &context.workspace_dir,
                    entry_id,
                    UpdateMemoryEntryRequest {
                        governance_state: Some(MemoryGovernanceState::Active),
                        ..UpdateMemoryEntryRequest::default()
                    },
                )
                .await?;
            }
            5 => {
                set_memory_entry_archived(
                    &context.workspace_dir,
                    entry_id,
                    !detail.summary.archived,
                )
                .await?;
            }
            6 => {
                let report = refresh_memory_console_governance(&context.workspace_dir).await?;
                println!(
                    "Refreshed governance: {} updated entries.",
                    report.updated_entries
                );
                pause()?;
            }
            7 => {
                if Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Delete this memory entry?")
                    .default(false)
                    .interact()?
                {
                    delete_memory_entry_by_id(&context.workspace_dir, entry_id).await?;
                    break;
                }
            }
            _ => break,
        }
    }
    Ok(())
}

fn edit_entry_interactively(
    detail: &gestura_core::memory_console::MemoryConsoleEntryDetail,
) -> Result<UpdateMemoryEntryRequest> {
    let theme = ColorfulTheme::default();
    let summary: String = Input::with_theme(&theme)
        .with_prompt("Summary")
        .with_initial_text(&detail.summary.summary)
        .interact_text()?;
    let content = Editor::new()
        .edit(&detail.content)?
        .unwrap_or_else(|| detail.content.clone());
    let governance_note: String = Input::with_theme(&theme)
        .with_prompt("Governance note")
        .with_initial_text(detail.governance_note.clone().unwrap_or_default())
        .allow_empty(true)
        .interact_text()?;
    let tags = pick_tags(&detail.summary.tags)?;
    Ok(UpdateMemoryEntryRequest {
        summary: Some(summary),
        content: Some(content),
        tags: Some(tags),
        governance_state: Some(detail.summary.governance_state),
        governance_note: Some((!governance_note.trim().is_empty()).then_some(governance_note)),
        ..UpdateMemoryEntryRequest::default()
    })
}

fn pick_tags(existing: &[String]) -> Result<Vec<String>> {
    let theme = ColorfulTheme::default();
    let mut base = existing.to_vec();
    let extra: String = Input::with_theme(&theme)
        .with_prompt("Comma-separated tags")
        .with_initial_text(existing.join(","))
        .allow_empty(true)
        .interact_text()?;
    if !extra.trim().is_empty() {
        base = extra
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToString::to_string)
            .collect();
    }
    Ok(base)
}

fn build_context(
    session_ref: Option<String>,
    workspace: Option<PathBuf>,
) -> Result<MemoryCommandContext> {
    let store = FileAgentSessionStore::default();
    let session = match session_ref.as_deref() {
        Some(reference) => Some(load_memory_console_session(&store, reference)?),
        None => store.load_last()?,
    };

    let workspace_dir = workspace
        .or_else(|| {
            session
                .as_ref()
                .and_then(|current| current.workspace_dir().cloned())
        })
        .ok_or_else(|| "No workspace available. Pass --workspace or --session.".to_string())?;

    let task_manager = TaskManager::new(&workspace_dir);
    Ok(MemoryCommandContext {
        workspace_dir,
        session,
        task_manager,
        store,
    })
}

fn require_session(context: &MemoryCommandContext) -> Result<AgentSession> {
    context
        .session
        .clone()
        .ok_or_else(|| "This memory operation requires a session. Pass --session <id>.".into())
}

fn require_session_ref(context: &MemoryCommandContext) -> Result<&AgentSession> {
    context
        .session
        .as_ref()
        .ok_or_else(|| "This memory operation requires a session. Pass --session <id>.".into())
}

#[allow(clippy::too_many_arguments)]
fn build_query(
    text: Option<String>,
    include_archived: bool,
    limit: usize,
    kinds: Vec<String>,
    memory_types: Vec<String>,
    scopes: Vec<String>,
    tags: Vec<String>,
    task_id: Option<String>,
    directive_id: Option<String>,
    agent_id: Option<String>,
    category: Option<String>,
) -> Result<MemoryConsoleQuery> {
    Ok(MemoryConsoleQuery {
        text,
        include_archived,
        limit,
        kinds: kinds
            .iter()
            .map(|value| parse_kind(value))
            .collect::<Result<Vec<_>>>()?,
        memory_types: memory_types
            .iter()
            .map(|value| parse_type(value))
            .collect::<Result<Vec<_>>>()?,
        scopes: scopes
            .iter()
            .map(|value| parse_scope(value))
            .collect::<Result<Vec<_>>>()?,
        task_id,
        directive_id,
        agent_id,
        category,
        tags,
        ..MemoryConsoleQuery::default()
    })
}

fn parse_kind(value: &str) -> Result<MemoryKind> {
    match value.to_ascii_lowercase().as_str() {
        "short-term" | "short_term" | "short" => Ok(MemoryKind::ShortTerm),
        "long-term" | "long_term" | "long" => Ok(MemoryKind::LongTerm),
        other => Err(format!("Unknown memory kind: {other}").into()),
    }
}

fn parse_type(value: &str) -> Result<MemoryType> {
    match value.to_ascii_lowercase().as_str() {
        "procedural" => Ok(MemoryType::Procedural),
        "semantic" => Ok(MemoryType::Semantic),
        "episodic" => Ok(MemoryType::Episodic),
        "resource" => Ok(MemoryType::Resource),
        "decision" => Ok(MemoryType::Decision),
        "blocker" => Ok(MemoryType::Blocker),
        "handoff" => Ok(MemoryType::Handoff),
        "reflection" => Ok(MemoryType::Reflection),
        other => Err(format!("Unknown memory type: {other}").into()),
    }
}

fn parse_governance_state(value: &str) -> Result<MemoryGovernanceState> {
    match value.to_ascii_lowercase().as_str() {
        "active" => Ok(MemoryGovernanceState::Active),
        "pinned" | "pin" => Ok(MemoryGovernanceState::Pinned),
        "needs_review" | "needs-review" | "needs review" | "review" => {
            Ok(MemoryGovernanceState::NeedsReview)
        }
        "superseded" | "supersede" => Ok(MemoryGovernanceState::Superseded),
        "archived" | "archive" => Ok(MemoryGovernanceState::Archived),
        other => Err(format!("Unknown memory governance state: {other}").into()),
    }
}

fn format_governance_state(value: MemoryGovernanceState) -> &'static str {
    match value {
        MemoryGovernanceState::Active => "active",
        MemoryGovernanceState::Pinned => "pinned",
        MemoryGovernanceState::NeedsReview => "needs review",
        MemoryGovernanceState::Superseded => "superseded",
        MemoryGovernanceState::Archived => "archived",
    }
}

fn parse_scope(value: &str) -> Result<MemoryScope> {
    match value.to_ascii_lowercase().as_str() {
        "session" => Ok(MemoryScope::Session),
        "task" => Ok(MemoryScope::Task),
        "directive" => Ok(MemoryScope::Directive),
        "workspace" => Ok(MemoryScope::Workspace),
        "repository" | "repo" => Ok(MemoryScope::Repository),
        other => Err(format!("Unknown memory scope: {other}").into()),
    }
}

fn promotion_source_label(source: SessionMemoryPromotionSource) -> &'static str {
    match source {
        SessionMemoryPromotionSource::Resource => "resource",
        SessionMemoryPromotionSource::Finding => "finding",
        SessionMemoryPromotionSource::Decision => "decision",
        SessionMemoryPromotionSource::Blocker => "blocker",
        SessionMemoryPromotionSource::Timeline => "timeline",
        SessionMemoryPromotionSource::NextAction => "next_action",
    }
}

fn promotion_candidate_memory_type(candidate: &SessionMemoryPromotionCandidate) -> MemoryType {
    match candidate.source {
        SessionMemoryPromotionSource::Resource => MemoryType::Resource,
        SessionMemoryPromotionSource::Finding => MemoryType::Semantic,
        SessionMemoryPromotionSource::Decision => MemoryType::Decision,
        SessionMemoryPromotionSource::Blocker => MemoryType::Blocker,
        SessionMemoryPromotionSource::Timeline => MemoryType::Episodic,
        SessionMemoryPromotionSource::NextAction => MemoryType::Procedural,
    }
}

fn promotion_candidate_scope(_candidate: &SessionMemoryPromotionCandidate) -> MemoryScope {
    MemoryScope::Workspace
}

fn promotion_candidate_confidence(candidate: &SessionMemoryPromotionCandidate) -> f32 {
    (candidate.score / 5.0).clamp(0.55, 0.95)
}

fn print_output<T: Serialize>(value: &T, _json: bool) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

fn pause() -> Result<()> {
    Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("Press enter to continue")
        .allow_empty(true)
        .interact_text()?;
    Ok(())
}
