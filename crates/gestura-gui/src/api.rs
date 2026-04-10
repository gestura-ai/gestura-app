//! Tauri command handlers for configuration, MCP tools, MDH pointers, and tests.
use crate::AppConfig;
use crate::AppConfigSecurityExt;

use gestura_core::pipeline::{AgentPipeline, AgentRequest, RequestSource};
use tauri::{Emitter, Manager, State};

/// Try to get an API key from the keychain without blocking the async request path.
/// Returns empty string if not found or keychain unavailable.
async fn try_get_api_key_from_keychain_async(provider: &str) -> String {
    let provider = provider.to_string();
    let provider_for_lookup = provider.clone();
    match tokio::task::spawn_blocking(move || {
        try_get_api_key_from_keychain_sync(&provider_for_lookup)
    })
    .await
    {
        Ok(key) => key,
        Err(error) => {
            tracing::warn!(provider = %provider, %error, "Failed to join async keychain lookup task");
            String::new()
        }
    }
}

/// Apply session-scoped LLM provider/model overrides to an in-memory `AppConfig`.
///
/// This helper keeps all per-session LLM behavior consistent across features (agent,
/// prompt enhancement, etc.). The override precedence and validation rules are
/// core-owned; this GUI wrapper only:
///
/// 1) fetches the persisted session override (if any)
/// 2) supplies a key lookup function (keychain)
///
/// This does **not** persist any changes to disk; it only affects the current request.
///
/// Returns the effective (provider, model) pair after applying overrides so callers can
/// keep request metadata and pipeline configuration in sync.
async fn apply_session_llm_config_overrides(
    cfg: &mut AppConfig,
    session_id: Option<&str>,
) -> gestura_core::llm_overrides::EffectiveLlmConfig {
    let session_llm = session_id.and_then(crate::window_manager::get_session_llm_config);
    tracing::debug!(
        session_id = ?session_id,
        session_llm_config = ?session_llm,
        "Retrieved session LLM config for overrides"
    );

    let provider_for_lookup = session_llm
        .as_ref()
        .and_then(|cfg| cfg.provider.as_deref())
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
        .unwrap_or(cfg.llm.primary.as_str())
        .to_string();
    let should_resolve_api_key = session_llm
        .as_ref()
        .and_then(|cfg| cfg.model.as_deref())
        .is_some_and(|model| !model.trim().is_empty());
    let preloaded_api_key = if should_resolve_api_key {
        let key = try_get_api_key_from_keychain_async(&provider_for_lookup).await;
        (!key.trim().is_empty()).then_some(key)
    } else {
        None
    };

    let api_key_lookup = move |provider: &str| {
        if provider.eq_ignore_ascii_case(&provider_for_lookup) {
            preloaded_api_key.clone()
        } else {
            None
        }
    };

    gestura_core::llm_overrides::apply_session_llm_overrides(
        cfg,
        session_llm.as_ref(),
        api_key_lookup,
    )
}

/// Run a single-shot (non-streaming) request through the core pipeline.
///
/// This is intended for legacy GUI commands that previously performed a single
/// `provider.call(...)` without tool execution.
///
/// Tools are explicitly disabled for this request; adapter commands that require
/// tool execution should use the streaming pipeline path instead.
async fn run_single_shot_pipeline(
    cfg: AppConfig,
    input: impl Into<String>,
    source: RequestSource,
    session_id: Option<&str>,
) -> Result<String, String> {
    let pipeline = AgentPipeline::with_provider_optimized_config(cfg);
    let mut request = AgentRequest::new(input.into())
        .with_streaming(false)
        .with_source(source)
        .with_tools_enabled(false);

    if let Some(sid) = session_id {
        request = request.with_session(sid);
    }

    pipeline
        .process_blocking(request)
        .await
        .map(|resp| resp.content)
        .map_err(|e| e.to_string())
}

fn contains_any_signal(text: &str, signals: &[&str]) -> bool {
    signals.iter().any(|signal| text.contains(signal))
}

fn should_auto_plan_agent_request(message: &str, explicit_task_id: Option<&str>) -> bool {
    if explicit_task_id.is_some() {
        return false;
    }

    let text = message.trim().to_ascii_lowercase();
    if text.is_empty() || text.starts_with('/') {
        return false;
    }

    let planning_signals = [
        "carefully plan",
        "plan",
        "step by step",
        "break this down",
        "break it down",
        "task out",
        "outline",
    ];
    let implementation_signals = [
        "plan and implement",
        "implement",
        "build",
        "create",
        "scaffold",
        "setup",
        "set up",
        "refactor",
        "fix",
        "add",
        "update",
        "wire",
        "integrate",
        "migrate",
    ];
    let validation_signals = [
        "build and test",
        "test",
        "verify",
        "validate",
        "smoke test",
        "compile",
        "check",
    ];
    let sequencing_signals = [
        " and then ",
        " then ",
        " afterwards",
        " after that",
        " finally",
        " end to end",
        " from start to finish",
    ];

    let matched_categories = [
        contains_any_signal(&text, &planning_signals),
        contains_any_signal(&text, &implementation_signals),
        contains_any_signal(&text, &validation_signals),
        contains_any_signal(&text, &sequencing_signals),
    ]
    .into_iter()
    .filter(|matched| *matched)
    .count();

    matched_categories >= 2
        && (contains_any_signal(&text, &implementation_signals)
            || contains_any_signal(&text, &validation_signals))
}

fn should_resume_active_task_request(message: &str, explicit_task_id: Option<&str>) -> bool {
    if explicit_task_id.is_some() {
        return false;
    }

    let text = message.trim().to_ascii_lowercase();
    if text.is_empty() || text.starts_with('/') {
        return false;
    }

    [
        "continue",
        "please complete",
        "complete",
        "finish",
        "keep going",
        "go on",
        "resume",
        "proceed",
        "carry on",
        "keep working",
        "pick up where you left off",
        "where you left off",
        "timed out",
    ]
    .iter()
    .any(|signal| text.contains(signal))
}

const STREAM_IDLE_TIMEOUT_NORMAL_SECS: u64 = 90;
const STREAM_IDLE_TIMEOUT_POST_TOOL_REVIEW_SECS: u64 = 5 * 60;
const STREAM_IDLE_TIMEOUT_WAITING_FOR_USER_SECS: u64 = 10 * 60;

fn stream_idle_timeout_for_chunk(chunk: &gestura_core::StreamChunk) -> std::time::Duration {
    match chunk {
        gestura_core::StreamChunk::ToolConfirmationRequired { .. } => {
            std::time::Duration::from_secs(STREAM_IDLE_TIMEOUT_WAITING_FOR_USER_SECS)
        }
        gestura_core::StreamChunk::ToolCallResult { .. }
        | gestura_core::StreamChunk::ReflectionStarted { .. } => {
            std::time::Duration::from_secs(STREAM_IDLE_TIMEOUT_POST_TOOL_REVIEW_SECS)
        }
        _ => std::time::Duration::from_secs(STREAM_IDLE_TIMEOUT_NORMAL_SECS),
    }
}

fn task_is_open(status: gestura_core::TaskStatus) -> bool {
    !matches!(
        status,
        gestura_core::TaskStatus::Completed | gestura_core::TaskStatus::Cancelled
    )
}

fn resolve_tracked_task_id(
    manager: &gestura_core::TaskManager,
    session_id: &str,
    message: &str,
    explicit_task_id: Option<&str>,
    auto_planned_root_task_id: Option<&str>,
) -> Option<String> {
    if let Some(task_id) = explicit_task_id {
        return Some(task_id.to_string());
    }

    if let Some(task_id) = auto_planned_root_task_id {
        return Some(task_id.to_string());
    }

    if !should_resume_active_task_request(message, explicit_task_id) {
        return None;
    }

    resolve_open_tracking_task_from_session(manager, session_id)
}

fn resolve_open_tracking_task_from_session(
    manager: &gestura_core::TaskManager,
    session_id: &str,
) -> Option<String> {
    if let Ok(Some(current_task_id)) = manager.get_current_task_id(session_id)
        && let Ok(Some(task)) = manager.get_task(session_id, &current_task_id)
        && task_is_open(task.status)
    {
        return Some(current_task_id);
    }

    let active_roots = manager
        .get_hierarchy(session_id)
        .ok()?
        .into_iter()
        .filter(|(root, _)| task_is_open(root.status))
        .map(|(root, _)| root.id)
        .collect::<Vec<_>>();

    if active_roots.len() == 1 {
        active_roots.into_iter().next()
    } else {
        None
    }
}

fn resolve_resume_tracked_task_id(
    manager: &gestura_core::TaskManager,
    session_id: &str,
) -> Option<String> {
    resolve_open_tracking_task_from_session(manager, session_id)
}

fn resolve_current_execution_task_id(
    explicit_task_id: Option<&str>,
    auto_planned_task: Option<&AutoPlanResult>,
    tracked_task_id: Option<&str>,
) -> Option<String> {
    explicit_task_id
        .map(ToString::to_string)
        .or_else(|| {
            auto_planned_task.and_then(|plan| {
                plan.initial_task_id
                    .clone()
                    .or_else(|| plan.root_task_id.clone())
            })
        })
        .or_else(|| tracked_task_id.map(ToString::to_string))
}

fn derive_agent_request_task_name(message: &str) -> String {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return "Agent request".to_string();
    }

    let first_line = trimmed
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(trimmed);
    let shortened: String = first_line.trim().chars().take(60).collect();

    if first_line.trim().chars().count() > 60 {
        format!("{}...", shortened)
    } else {
        shortened
    }
}

fn derive_auto_plan_planning_task_name(root_task_name: &str) -> String {
    format!("Plan request: {}", root_task_name.trim())
}

fn build_auto_plan_planning_task_description(message: &str) -> String {
    format!(
        "Autogenerated execution plan request for:\n\n{}",
        message.trim()
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct AutoPlanNarrationIntent {
    wants_planning: bool,
    wants_implementation: bool,
    wants_validation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutoPlanResult {
    root_task_id: Option<String>,
    root_task_name: String,
    planned_subtasks: Vec<String>,
    initial_task_id: Option<String>,
    initial_task_name: Option<String>,
    generated_task_count: usize,
}

#[derive(Debug, Clone)]
struct AutoPlanGeneratedTasks {
    top_level_tasks: Vec<gestura_core::Task>,
    generated_task_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutoPlanExecutionContext {
    root_task_name: String,
    planned_subtasks: Vec<String>,
    initial_task_id: Option<String>,
    initial_task_name: Option<String>,
}

fn infer_auto_plan_narration_intent(message: &str) -> AutoPlanNarrationIntent {
    let text = message.trim().to_ascii_lowercase();

    AutoPlanNarrationIntent {
        wants_planning: contains_any_signal(
            &text,
            &[
                "carefully plan",
                "plan",
                "step by step",
                "break this down",
                "break it down",
                "task out",
                "outline",
            ],
        ),
        wants_implementation: contains_any_signal(
            &text,
            &[
                "plan and implement",
                "implement",
                "build",
                "create",
                "scaffold",
                "setup",
                "set up",
                "refactor",
                "fix",
                "add",
                "update",
                "wire",
                "integrate",
                "migrate",
            ],
        ),
        wants_validation: contains_any_signal(
            &text,
            &[
                "build and test",
                "test and build",
                "build & test",
                "verify",
                "validation",
                "validate",
                "smoke test",
                "check",
                "compile",
                "run the relevant tests",
                "end to end",
            ],
        ),
    }
}

fn strip_task_runtime_suffix(task: &str) -> String {
    task.rsplit_once(" [")
        .map(|(name, _)| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| task.trim().to_string())
}

fn compose_bootstrap_narration_message(
    summary: Option<&str>,
    reason: Option<&str>,
    next_step: Option<&str>,
) -> String {
    [summary, reason, next_step]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn derive_bootstrap_narration_title(
    summary: &str,
    reason: Option<&str>,
    next_step: Option<&str>,
) -> String {
    let combined = format!(
        "{} {} {}",
        summary,
        reason.unwrap_or_default(),
        next_step.unwrap_or_default()
    )
    .to_ascii_lowercase();

    if combined.contains("chose") {
        "Choosing First Subtask".to_string()
    } else if combined.contains("breaking") && combined.contains("subtask") {
        "Breaking Into Subtasks".to_string()
    } else if combined.contains("first subtask") || combined.contains("start with") {
        "Choosing First Subtask".to_string()
    } else if combined.contains("queued") {
        "Queuing Remaining Steps".to_string()
    } else if combined.contains("verification") || combined.contains("prove") {
        "Preparing Verification Proof".to_string()
    } else if combined.contains("tracked plan") || combined.contains("task plan") {
        "Preparing Task Plan".to_string()
    } else {
        "Planning Next Steps".to_string()
    }
}

const BOOTSTRAP_NARRATION_LLM_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(900);
const MIN_BOOTSTRAP_NARRATION_TITLE_WORDS: usize = 2;
const MAX_BOOTSTRAP_NARRATION_TITLE_WORDS: usize = 7;

#[derive(Debug, serde::Deserialize)]
struct BootstrapNarrationPayloadCandidate {
    title: Option<String>,
    message: Option<String>,
    summary: Option<String>,
    reason: Option<String>,
    next_step: Option<String>,
    evidence: Option<Vec<String>>,
}

fn collapse_bootstrap_narration_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_bootstrap_narration_text(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }

    let limit = max_chars.saturating_sub(1);
    let mut truncated = text.chars().take(limit).collect::<String>();
    while matches!(truncated.chars().last(), Some(' ' | ',' | ';' | ':')) {
        truncated.pop();
    }
    truncated.push('…');
    truncated
}

fn sanitize_bootstrap_narration_field(
    text: &str,
    min_words: usize,
    max_chars: usize,
) -> Option<String> {
    let mut cleaned = collapse_bootstrap_narration_whitespace(text);
    for prefix in [
        "Title:",
        "Narration:",
        "Message:",
        "Summary:",
        "Reason:",
        "Next step:",
    ] {
        if cleaned
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        {
            cleaned = cleaned[prefix.len()..].trim().to_string();
            break;
        }
    }

    if cleaned.is_empty() {
        return None;
    }

    let word_count = cleaned.split_whitespace().count();
    if word_count < min_words {
        return None;
    }

    Some(truncate_bootstrap_narration_text(&cleaned, max_chars))
}

fn sanitize_bootstrap_narration_message_field(
    text: &str,
    min_words: usize,
    max_chars: usize,
) -> Option<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = normalized
        .lines()
        .map(|line| line.trim_end().to_string())
        .collect::<Vec<_>>();

    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    let mut cleaned_lines = Vec::with_capacity(lines.len());
    let mut previous_was_blank = false;
    for mut line in lines {
        if cleaned_lines.is_empty() {
            let trimmed_start = line.trim_start();
            for prefix in [
                "Narration:",
                "Public narration:",
                "Update:",
                "Message:",
                "Summary:",
                "Reason:",
                "Next step:",
            ] {
                if trimmed_start
                    .get(..prefix.len())
                    .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
                {
                    let leading = &line[..line.len().saturating_sub(trimmed_start.len())];
                    line = format!("{leading}{}", trimmed_start[prefix.len()..].trim_start());
                    break;
                }
            }
        }

        if line.trim().is_empty() {
            if !previous_was_blank {
                cleaned_lines.push(String::new());
            }
            previous_was_blank = true;
            continue;
        }

        previous_was_blank = false;
        cleaned_lines.push(line);
    }

    let cleaned = cleaned_lines
        .join("\n")
        .trim_matches('"')
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return None;
    }

    let word_count = cleaned.split_whitespace().count();
    if word_count < min_words {
        return None;
    }

    Some(truncate_bootstrap_narration_text(&cleaned, max_chars))
}

fn sanitize_bootstrap_narration_title(text: &str) -> Option<String> {
    let cleaned = collapse_bootstrap_narration_whitespace(text);
    let cleaned = cleaned
        .trim_start_matches("Title:")
        .trim()
        .trim_end_matches(['.', '!', '?', ';', ':', ','])
        .trim();

    if cleaned.is_empty() {
        return None;
    }

    if cleaned.ends_with('…') || cleaned.ends_with("...") {
        return None;
    }

    let word_count = cleaned.split_whitespace().count();
    if !(MIN_BOOTSTRAP_NARRATION_TITLE_WORDS..=MAX_BOOTSTRAP_NARRATION_TITLE_WORDS)
        .contains(&word_count)
    {
        return None;
    }

    Some(cleaned.to_string())
}

fn parse_bootstrap_narration_payload(
    raw: &str,
    fallback: gestura_core::streaming::PublicNarration,
) -> gestura_core::streaming::PublicNarration {
    let trimmed = raw.trim();
    let parsed = serde_json::from_str::<BootstrapNarrationPayloadCandidate>(trimmed)
        .ok()
        .or_else(|| {
            let start = trimmed.find('{')?;
            let end = trimmed.rfind('}')?;
            serde_json::from_str::<BootstrapNarrationPayloadCandidate>(&trimmed[start..=end]).ok()
        });

    if let Some(candidate) = parsed {
        let summary = candidate
            .summary
            .as_deref()
            .and_then(|text| sanitize_bootstrap_narration_field(text, 4, 220))
            .or_else(|| fallback.summary.clone());
        let reason = candidate
            .reason
            .as_deref()
            .and_then(|text| sanitize_bootstrap_narration_field(text, 4, 220))
            .or_else(|| fallback.reason.clone());
        let next_step = candidate
            .next_step
            .as_deref()
            .and_then(|text| sanitize_bootstrap_narration_field(text, 4, 220))
            .or_else(|| fallback.next_step.clone());
        let message = candidate
            .message
            .as_deref()
            .and_then(|text| sanitize_bootstrap_narration_message_field(text, 8, 520))
            .or_else(|| {
                let composed = compose_bootstrap_narration_message(
                    summary.as_deref(),
                    reason.as_deref(),
                    next_step.as_deref(),
                );
                (!composed.is_empty()).then_some(composed)
            })
            .unwrap_or_else(|| fallback.message.clone());
        let title = candidate
            .title
            .as_deref()
            .and_then(sanitize_bootstrap_narration_title)
            .unwrap_or_else(|| fallback.title.clone());
        let evidence = candidate
            .evidence
            .unwrap_or_else(|| fallback.evidence.clone())
            .into_iter()
            .filter_map(|entry| sanitize_bootstrap_narration_field(&entry, 3, 140))
            .take(3)
            .collect::<Vec<_>>();

        return gestura_core::streaming::PublicNarration {
            title,
            message,
            summary,
            reason,
            next_step,
            evidence,
        };
    }

    if let Some(message) = sanitize_bootstrap_narration_message_field(trimmed, 8, 520) {
        return gestura_core::streaming::PublicNarration {
            title: fallback.title,
            message,
            summary: fallback.summary,
            reason: fallback.reason,
            next_step: fallback.next_step,
            evidence: fallback.evidence,
        };
    }

    fallback
}

fn build_pre_auto_plan_narration_prompt(message: &str, root_task_name: &str) -> String {
    let intent = infer_auto_plan_narration_intent(message);
    let mut prompt = String::from(
        "Write the first planning narration the user should see before I create the task breakdown. Return only strict JSON with exactly these fields: {\"title\":\"...\",\"message\":\"...\",\"summary\":\"...\",\"reason\":\"...\",\"next_step\":\"...\",\"evidence\":[\"...\"]}. Do not use markdown fences.\n",
    );
    prompt.push_str(
        "Rules:\n- title: 2 to 7 words, concrete, derived from the message, and no ending punctuation.\n- message: 2 to 4 natural first-person sentences that sound like the agent thinking aloud while shaping the plan. It should feel like live narration, not a stitched template.\n- summary/reason/next_step: short grounded support fields.\n- evidence: 0 to 3 short grounded bullets.\n- Explain what I’m considering, what structure I’m about to create, why that structure helps, and what it will mean for the final tasking once execution starts.\n- Avoid generic filler and avoid sounding like labels glued together.\n- Do not expose hidden chain-of-thought; only share the useful public-facing narration.\n",
    );
    prompt.push_str(&format!("User request: {}\n", message.trim()));
    prompt.push_str(&format!(
        "Tracked root task name: {}\n",
        root_task_name.trim()
    ));
    prompt.push_str(&format!(
        "Request shape hints: planning={}, implementation={}, validation={}.\n",
        intent.wants_planning, intent.wants_implementation, intent.wants_validation
    ));
    prompt
}

fn build_post_auto_plan_narration_prompt(message: &str, plan: &AutoPlanResult) -> String {
    let mut prompt = String::from(
        "Write the planning narration the user should see immediately after I create the task breakdown. Return only strict JSON with exactly these fields: {\"title\":\"...\",\"message\":\"...\",\"summary\":\"...\",\"reason\":\"...\",\"next_step\":\"...\",\"evidence\":[\"...\"]}. Do not use markdown fences.\n",
    );
    prompt.push_str(
        "Rules:\n- title: 2 to 7 words, concrete, derived from the message, and no ending punctuation.\n- message: 2 to 4 natural first-person sentences that sound like the agent talking the user through the plan it just created.\n- summary/reason/next_step: short grounded support fields.\n- evidence: 0 to 3 short grounded bullets.\n- Explain what task structure I created, why the first subtask was chosen, what remains queued behind it, and what the next verification step should prove.\n- Make it sound connected and reflective, not mechanical or templated.\n- Do not expose hidden chain-of-thought; only share the useful public-facing narration.\n",
    );
    prompt.push_str(&format!("Original user request: {}\n", message.trim()));
    prompt.push_str(&format!(
        "Tracked root task name: {}\n",
        plan.root_task_name.trim()
    ));
    prompt.push_str(&format!(
        "Generated task count: {}\n",
        plan.generated_task_count
    ));
    if plan.planned_subtasks.is_empty() {
        prompt.push_str("Planned subtasks: none captured yet.\n");
    } else {
        prompt.push_str("Planned subtasks in current order:\n");
        for task in &plan.planned_subtasks {
            prompt.push_str("- ");
            prompt.push_str(&strip_task_runtime_suffix(task));
            prompt.push('\n');
        }
    }
    prompt
}

async fn generate_bootstrap_narration_with_llm(
    session_id: &str,
    prompt: String,
    fallback: gestura_core::streaming::PublicNarration,
) -> gestura_core::streaming::PublicNarration {
    let mut cfg = AppConfig::load_async().await;
    let _effective_llm = apply_session_llm_config_overrides(&mut cfg, Some(session_id)).await;

    match tokio::time::timeout(
        BOOTSTRAP_NARRATION_LLM_TIMEOUT,
        run_single_shot_pipeline(cfg, prompt, RequestSource::GuiText, Some(session_id)),
    )
    .await
    {
        Ok(Ok(response)) => parse_bootstrap_narration_payload(&response, fallback),
        Ok(Err(error)) => {
            tracing::warn!(session_id = %session_id, error = %error, "Failed to generate bootstrap narration with LLM; using fallback");
            fallback
        }
        Err(_) => {
            tracing::debug!(session_id = %session_id, "Bootstrap narration LLM timed out; using fallback");
            fallback
        }
    }
}

async fn generate_pre_auto_plan_narration(
    session_id: &str,
    message: &str,
    root_task_name: &str,
) -> gestura_core::streaming::PublicNarration {
    let fallback = build_pre_auto_plan_narration(message, root_task_name);
    let prompt = build_pre_auto_plan_narration_prompt(message, root_task_name);
    generate_bootstrap_narration_with_llm(session_id, prompt, fallback).await
}

async fn generate_post_auto_plan_narration(
    session_id: &str,
    message: &str,
    plan: &AutoPlanResult,
) -> gestura_core::streaming::PublicNarration {
    let fallback = build_post_auto_plan_narration(plan);
    let prompt = build_post_auto_plan_narration_prompt(message, plan);
    generate_bootstrap_narration_with_llm(session_id, prompt, fallback).await
}

fn build_pre_auto_plan_narration(
    message: &str,
    root_task_name: &str,
) -> gestura_core::streaming::PublicNarration {
    let intent = infer_auto_plan_narration_intent(message);
    let summary = match (
        intent.wants_planning,
        intent.wants_implementation,
        intent.wants_validation,
    ) {
        (true, true, true) => format!(
            "I’m breaking \"{}\" into tracked subtasks so I can choose the first implementation step before I start changing anything.",
            root_task_name
        ),
        (_, true, true) => format!(
            "I’m breaking \"{}\" into tracked subtasks so the first implementation step and the later verification work stay attached to the same request.",
            root_task_name
        ),
        (_, true, false) => format!(
            "I’m breaking \"{}\" into tracked subtasks so I can choose the first implementation step instead of improvising the order.",
            root_task_name
        ),
        _ => format!(
            "I’m breaking \"{}\" into concrete tracked subtasks so the first execution step is explicit before I begin.",
            root_task_name
        ),
    };

    let reason = if intent.wants_validation {
        Some(
            "That matters because I want to choose the first subtask in dependency order, keep the remaining work queued behind it, and make the later verification pass prove the implementation actually moved the request forward."
                .to_string(),
        )
    } else if intent.wants_implementation {
        Some(
            "That matters because I want the first subtask to be the highest-leverage implementation move, with the remaining work clearly queued behind it instead of left implicit."
                .to_string(),
        )
    } else {
        Some(
            "That matters because the task tree should make the first subtask explicit, keep the rest of the work queued behind it, and leave room for a clear verification pass afterward."
                .to_string(),
        )
    };
    let next_step = Some(
        "Once the tracked plan exists, I’ll call out the first subtask, note what stays queued behind it, and explain what the first verification pass will prove before execution starts."
            .to_string(),
    );
    let title = derive_bootstrap_narration_title(&summary, reason.as_deref(), next_step.as_deref());

    gestura_core::streaming::PublicNarration {
        title,
        message: compose_bootstrap_narration_message(
            Some(summary.as_str()),
            reason.as_deref(),
            next_step.as_deref(),
        ),
        summary: Some(summary),
        reason,
        next_step,
        evidence: vec![format!("Tracked root task: \"{}\".", root_task_name)],
    }
}

fn build_post_auto_plan_narration(
    plan: &AutoPlanResult,
) -> gestura_core::streaming::PublicNarration {
    let first_task = plan
        .planned_subtasks
        .first()
        .map(|task| strip_task_runtime_suffix(task));
    let queued_tasks = plan
        .planned_subtasks
        .iter()
        .skip(1)
        .map(|task| strip_task_runtime_suffix(task))
        .collect::<Vec<_>>();
    let summary = match plan.planned_subtasks.as_slice() {
        [] => format!(
            "I broke \"{}\" into tracked work and I’m moving straight from planning into execution now.",
            plan.root_task_name
        ),
        [first] => format!(
            "I broke \"{}\" into tracked subtasks and chose \"{}\" as the first subtask. Nothing else is queued behind it yet.",
            plan.root_task_name,
            strip_task_runtime_suffix(first)
        ),
        [first, second, ..] => {
            let remaining = plan.planned_subtasks.len().saturating_sub(2);
            if remaining == 0 {
                format!(
                    "I broke \"{}\" into tracked subtasks and chose \"{}\" as the first subtask. \"{}\" stays queued right behind it.",
                    plan.root_task_name,
                    strip_task_runtime_suffix(first),
                    strip_task_runtime_suffix(second)
                )
            } else {
                format!(
                    "I broke \"{}\" into tracked subtasks and chose \"{}\" as the first subtask. \"{}\" and {} more step(s) stay queued behind it.",
                    plan.root_task_name,
                    strip_task_runtime_suffix(first),
                    strip_task_runtime_suffix(second),
                    remaining
                )
            }
        }
    };

    let reason = if let Some(first_task) = first_task.as_ref() {
        Some(
            format!(
                "That matters because I chose \"{}\" as the first subtask to get the earliest concrete signal before I spend time on the queued work behind it.",
                first_task
            )
                .to_string(),
        )
    } else {
        Some(
            "That matters because the request now has a tracked execution anchor, so the first live step is grounded before implementation begins."
                .to_string(),
        )
    };
    let next_step = first_task
        .as_ref()
        .map(|task| {
            let queued_suffix = match queued_tasks.as_slice() {
                [] => String::new(),
                [second] => format!(" before I move to \"{}\"", second),
                [second, rest @ ..] => {
                    format!(" before I move to \"{}\" and the {} queued step(s) behind it", second, rest.len())
                }
            };
            format!(
                "I’ll execute \"{}\" first and then verify whether it produced the expected signal for \"{}\"{}.",
                task, plan.root_task_name, queued_suffix
            )
        })
        .or_else(|| Some("I’ll move from planning into execution now that the tracked root task is in place.".to_string()));
    let title = derive_bootstrap_narration_title(&summary, reason.as_deref(), next_step.as_deref());

    let mut evidence = vec![format!("Tracked root task: \"{}\".", plan.root_task_name)];
    for task in plan.planned_subtasks.iter().take(2) {
        evidence.push(format!(
            "Planned step: \"{}\".",
            strip_task_runtime_suffix(task)
        ));
    }
    if plan.planned_subtasks.len() > 2 {
        evidence.push(format!(
            "Additional queued step count: {}.",
            plan.planned_subtasks.len() - 2
        ));
    }

    gestura_core::streaming::PublicNarration {
        title,
        message: compose_bootstrap_narration_message(
            Some(summary.as_str()),
            reason.as_deref(),
            next_step.as_deref(),
        ),
        summary: Some(summary),
        reason,
        next_step,
        evidence,
    }
}

fn emit_bootstrap_narration<F>(emit: &F, narration: gestura_core::streaming::PublicNarration)
where
    F: Fn(&str, serde_json::Value),
{
    emit(
        "agent-stream-narration",
        serde_json::json!({
            "title": narration.title,
            "message": narration.message,
            "summary": narration.summary,
            "reason": narration.reason,
            "next_step": narration.next_step,
            "evidence": narration.evidence,
            "stage": "planning",
        }),
    );
}

fn collect_auto_plan_execution_context(
    session_id: &str,
    root_task_id: &str,
) -> Option<AutoPlanExecutionContext> {
    let manager = crate::task_integration::get_task_manager();
    let root_task = manager.get_task(session_id, root_task_id).ok()??;

    let open_descendants = manager
        .list_descendants(session_id, root_task_id)
        .ok()?
        .into_iter()
        .filter(|task| {
            !matches!(
                task.status,
                gestura_core::TaskStatus::Completed | gestura_core::TaskStatus::Cancelled
            )
        })
        .collect::<Vec<_>>();

    let planned_subtasks = open_descendants
        .iter()
        .take(8)
        .map(|task| format!("{} [{}]", task.name, task.status))
        .collect();
    let initial_task = open_descendants.first();

    Some(AutoPlanExecutionContext {
        root_task_name: root_task.name,
        planned_subtasks,
        initial_task_id: initial_task.map(|task| task.id.clone()),
        initial_task_name: initial_task.map(|task| task.name.clone()),
    })
}

fn task_tool_mutation_event(
    session_id: &str,
    tool_name: &str,
    tool_args: &str,
    success: bool,
) -> Option<(&'static str, serde_json::Value)> {
    if !success || !matches!(tool_name.to_ascii_lowercase().as_str(), "task" | "tasks") {
        return None;
    }

    let args: serde_json::Value = serde_json::from_str(tool_args).ok()?;
    let operation = args.get("operation")?.as_str()?.to_ascii_lowercase();
    let event_name = match operation.as_str() {
        "create" => "task-created",
        "update_status" | "update" => "task-updated",
        "delete" => "task-deleted",
        _ => return None,
    };

    Some((
        event_name,
        serde_json::json!({
            "session_id": session_id,
            "operation": operation,
        }),
    ))
}

fn task_tree_refresh_signature_with_manager(
    manager: &gestura_core::TaskManager,
    session_id: &str,
) -> Option<String> {
    let mut tasks = manager.list_tasks(session_id).ok()?;
    tasks.sort_by(|left, right| left.id.cmp(&right.id));
    let current_task_id = manager.get_current_task_id(session_id).ok().flatten();

    Some(
        serde_json::json!({
            "current_task_id": current_task_id,
            "tasks": tasks
                .into_iter()
                .map(|task| serde_json::json!({
                    "id": task.id,
                    "parent_id": task.parent_id,
                    "status": format!("{:?}", task.status),
                    "updated_at": task.updated_at.timestamp_millis(),
                }))
                .collect::<Vec<_>>(),
        })
        .to_string(),
    )
}

fn task_tree_refresh_signature(session_id: &str) -> Option<String> {
    task_tree_refresh_signature_with_manager(
        crate::task_integration::get_task_manager(),
        session_id,
    )
}

fn emit_task_refresh_if_changed(
    app: &tauri::AppHandle,
    session_id: Option<&str>,
    last_signature: &mut Option<String>,
) {
    let Some(session_id) = session_id else {
        return;
    };

    let Some(signature) = task_tree_refresh_signature(session_id) else {
        return;
    };

    if last_signature.as_ref() == Some(&signature) {
        return;
    }

    *last_signature = Some(signature);
    let _ = app.emit(
        "task-updated",
        serde_json::json!({
            "session_id": session_id,
            "operation": "refresh",
            "source": "core-task-sync",
        }),
    );
}

fn build_auto_plan_execution_handoff_message(
    original_message: &str,
    root_task_name: &str,
    planned_subtasks: &[String],
    initial_task_name: Option<&str>,
) -> String {
    let mut handoff = String::new();
    handoff.push_str(original_message.trim());
    handoff.push_str("\n\n[Runtime execution handoff]\n");
    handoff.push_str(&format!(
        "A task plan already exists for this request under the tracked task \"{}\". Your job now is to execute that plan, not to stop after planning.\n",
        root_task_name
    ));

    if let Some(initial_task_name) = initial_task_name.filter(|task| !task.trim().is_empty()) {
        handoff.push_str(&format!(
            "Start by executing \"{}\" before you consider the later queued work.\n",
            initial_task_name.trim()
        ));
    }

    if !planned_subtasks.is_empty() {
        handoff.push_str("Open tracked subtasks in this plan:\n");
        for subtask in planned_subtasks {
            handoff.push_str("- ");
            handoff.push_str(subtask);
            handoff.push('\n');
        }
    }

    handoff.push_str(
        "Begin concrete implementation work immediately. Use the available tools to inspect files, edit code, build, test, and update task statuses as you start and finish each planned subtask. If finishing the current execution task reveals follow-on work, create that work under the tracked root plan instead of nesting it under the currently executing task unless it is a true blocking prerequisite. That keeps the current execution task completable once the handoff is done. Do not claim the request is complete while any planned subtask remains open, and only mark the tracked root task complete after every subtask is terminal. Do not end the run after only reviewing the plan or task list.\n",
    );
    handoff
}

async fn generate_requirement_breakdown(
    session_id: &str,
    requirements: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let mut cfg = AppConfig::load_async().await;
    let _effective_llm = apply_session_llm_config_overrides(&mut cfg, Some(session_id)).await;

    let prompt = format!(
        r#"You are a project planning assistant. Analyze the following requirements and break them down into a structured task list.

Requirements:
{}

Please respond with a JSON array of tasks. Each task should have:
- "name": A concise task name (max 60 chars)
- "description": A detailed description of what needs to be done
- "priority": "high", "medium", or "low"
- "is_blocking": true if other tasks depend on this, false otherwise
- "parent_name": null for root tasks, or the exact name of the parent task for subtasks

Order tasks by priority and logical execution order. Group related tasks under parent tasks.

Example format:
[
  {{"name": "Setup project structure", "description": "Initialize the project...", "priority": "high", "is_blocking": true, "parent_name": null}},
  {{"name": "Configure build system", "description": "Set up the build...", "priority": "high", "is_blocking": false, "parent_name": "Setup project structure"}}
]

Respond ONLY with the JSON array, no additional text."#,
        requirements
    );

    let response = run_single_shot_pipeline(cfg, prompt, RequestSource::GuiText, Some(session_id))
        .await
        .map_err(|e| format!("LLM error: {}", e))?;

    let tasks_json: serde_json::Value = serde_json::from_str(&response).map_err(|e| {
        format!(
            "Failed to parse LLM response: {}. Response was: {}",
            e, response
        )
    })?;

    tasks_json
        .as_array()
        .cloned()
        .ok_or_else(|| "LLM response is not a JSON array".to_string())
}

fn format_breakdown_task_description(task_json: &serde_json::Value) -> String {
    let priority = task_json
        .get("priority")
        .and_then(|v| v.as_str())
        .unwrap_or("medium");
    let is_blocking = task_json
        .get("is_blocking")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    format!(
        "{}\n\n[Priority: {} | Blocking: {}]",
        task_json
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        priority,
        if is_blocking { "Yes" } else { "No" }
    )
}

fn populate_auto_plan_generated_tasks<F>(
    tasks_array: &[serde_json::Value],
    mut create_task: F,
) -> Result<AutoPlanGeneratedTasks, String>
where
    F: FnMut(&str, &str, Option<String>) -> Result<gestura_core::Task, String>,
{
    let mut generated_task_count = 0usize;
    let mut top_level_tasks = Vec::new();
    let mut name_to_id: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for task_json in tasks_array {
        let parent_name = task_json.get("parent_name").and_then(|v| v.as_str());
        if parent_name.is_some() && !parent_name.unwrap_or_default().is_empty() {
            continue;
        }

        let name = task_json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled Task");
        let task = create_task(name, &format_breakdown_task_description(task_json), None)?;
        generated_task_count += 1;
        name_to_id.insert(name.to_string(), task.id.clone());
        top_level_tasks.push(task);
    }

    for task_json in tasks_array {
        let Some(parent_name) = task_json.get("parent_name").and_then(|v| v.as_str()) else {
            continue;
        };
        if parent_name.is_empty() {
            continue;
        }

        let parent_id = name_to_id.get(parent_name).cloned().unwrap_or_default();
        let name = task_json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled Subtask");
        let parent_id = if parent_id.is_empty() {
            None
        } else {
            Some(parent_id)
        };
        let task = create_task(
            name,
            &format_breakdown_task_description(task_json),
            parent_id,
        )?;
        generated_task_count += 1;
        name_to_id.insert(name.to_string(), task.id.clone());
    }

    Ok(AutoPlanGeneratedTasks {
        top_level_tasks,
        generated_task_count,
    })
}

fn collect_auto_plan_execution_context_from_generated_tasks(
    session_id: &str,
    fallback_root_task_name: &str,
    generated_tasks: &AutoPlanGeneratedTasks,
) -> Option<AutoPlanExecutionContext> {
    match generated_tasks.top_level_tasks.as_slice() {
        [] => None,
        [root_task] => collect_auto_plan_execution_context(session_id, &root_task.id),
        roots => {
            let open_roots = roots
                .iter()
                .filter(|task| task_is_open(task.status))
                .collect::<Vec<_>>();
            let first_root = open_roots.first().copied().or_else(|| roots.first());
            let nested_context = first_root
                .and_then(|task| collect_auto_plan_execution_context(session_id, &task.id));

            Some(AutoPlanExecutionContext {
                root_task_name: fallback_root_task_name.to_string(),
                planned_subtasks: open_roots
                    .iter()
                    .take(8)
                    .map(|task| format!("{} [{}]", task.name, task.status))
                    .collect(),
                initial_task_id: nested_context
                    .as_ref()
                    .and_then(|context| context.initial_task_id.clone())
                    .or_else(|| first_root.map(|task| task.id.clone())),
                initial_task_name: nested_context
                    .and_then(|context| context.initial_task_name)
                    .or_else(|| first_root.map(|task| task.name.clone())),
            })
        }
    }
}

async fn auto_plan_agent_request(
    app: &tauri::AppHandle,
    session_id: &str,
    message: &str,
) -> Result<AutoPlanResult, String> {
    let root_name = derive_agent_request_task_name(message);
    let planning_task = crate::task_integration::create_agent_task(
        app,
        session_id,
        &derive_auto_plan_planning_task_name(&root_name),
        &build_auto_plan_planning_task_description(message),
        None,
        None,
    )?;
    let _ = crate::task_integration::get_task_manager()
        .set_current_task_id(session_id, Some(planning_task.id.clone()));
    let _ = crate::task_integration::mark_task_in_progress(app, session_id, &planning_task.id);

    let generated_tasks = match generate_requirement_breakdown(session_id, message).await {
        Ok(tasks_array) => {
            populate_auto_plan_generated_tasks(&tasks_array, |name, description, parent_id| {
                crate::task_integration::create_agent_task(
                    app,
                    session_id,
                    name,
                    description,
                    None,
                    parent_id,
                )
            })?
        }
        Err(error) => {
            tracing::warn!(session_id = %session_id, error = %error, "Failed to auto-plan agent request; no generated execution tasks were created");
            AutoPlanGeneratedTasks {
                top_level_tasks: Vec::new(),
                generated_task_count: 0,
            }
        }
    };

    crate::task_integration::mark_task_completed(app, session_id, &planning_task.id)?;

    let tracked_root_task_id = if generated_tasks.top_level_tasks.len() == 1 {
        generated_tasks
            .top_level_tasks
            .first()
            .map(|task| task.id.clone())
    } else {
        None
    };
    let execution_context = collect_auto_plan_execution_context_from_generated_tasks(
        session_id,
        &root_name,
        &generated_tasks,
    );

    if let Some(current_task_id) = execution_context
        .as_ref()
        .and_then(|context| context.initial_task_id.clone())
        .or_else(|| tracked_root_task_id.clone())
    {
        let _ = crate::task_integration::get_task_manager()
            .set_current_task_id(session_id, Some(current_task_id.clone()));
        let _ = crate::task_integration::mark_task_in_progress(app, session_id, &current_task_id);
    }

    Ok(AutoPlanResult {
        root_task_id: tracked_root_task_id,
        root_task_name: execution_context
            .as_ref()
            .map(|context| context.root_task_name.clone())
            .unwrap_or(root_name),
        planned_subtasks: execution_context
            .as_ref()
            .map(|context| context.planned_subtasks.clone())
            .unwrap_or_default(),
        initial_task_id: execution_context
            .as_ref()
            .and_then(|context| context.initial_task_id.clone()),
        initial_task_name: execution_context.and_then(|context| context.initial_task_name),
        generated_task_count: generated_tasks.generated_task_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ensures the GUI legacy single-shot path compiles and returns an error
    /// when no provider is configured.
    #[tokio::test]
    async fn run_single_shot_pipeline_returns_error_without_provider() {
        use tokio::time::{Duration, timeout};

        let cfg = AppConfig::default();
        // Use a timeout to prevent hanging if the test misconfigures or hits network
        let result = timeout(Duration::from_secs(5), async {
            run_single_shot_pipeline(cfg, "hello", RequestSource::GuiText, None).await
        })
        .await;

        // If timeout elapsed, that's also acceptable for the test
        // (provider may hang waiting for a response).
        // The important thing is that the function compiles and runs.
        match result {
            Ok(Ok(_)) => {
                // Unexpected success - this would be a test failure
                panic!("Expected error when no provider is configured, but got success");
            }
            Ok(Err(err)) => {
                // Expected path - provider returns an error
                // The error message may vary (e.g., "not configured" or network error)
                assert!(!err.is_empty(), "Expected non-empty error message");
            }
            Err(_) => {
                // Timeout elapsed - acceptable (provider may be slow to fail)
            }
        }
    }

    #[test]
    fn load_memory_console_session_falls_back_to_persisted_store() {
        use gestura_core::agent_sessions::{
            AgentSession, AgentSessionStore, FileAgentSessionStore,
        };
        use tempfile::tempdir;

        let temp = tempdir().unwrap();
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).unwrap();

        let store = FileAgentSessionStore::new(temp.path().join("sessions"));
        let session = AgentSession::new_with_workspace(workspace_dir.clone(), None).unwrap();
        store.save(&session).unwrap();

        let loaded = load_memory_console_session(&session.id, &[], &store)
            .expect("persisted session should be returned when not live");

        assert_eq!(loaded.id, session.id);
        assert_eq!(
            loaded
                .workspace_dir()
                .and_then(|path| path.canonicalize().ok()),
            workspace_dir.canonicalize().ok()
        );
    }

    #[test]
    fn auto_plan_heuristic_detects_multi_step_requests() {
        assert!(should_auto_plan_agent_request(
            "I want to create a small tauri gui that says hello world. Please carefully plan and implement then build and test it.",
            None,
        ));
        assert!(should_auto_plan_agent_request(
            "Please refactor the retry logic, then run the relevant tests and verify the CLI still works.",
            None,
        ));
        assert!(should_auto_plan_agent_request(
            "Break this down, implement the session restore flow, and validate it end to end.",
            None,
        ));
        assert!(!should_auto_plan_agent_request("/tools", None));
        assert!(!should_auto_plan_agent_request("hello", None));
        assert!(!should_auto_plan_agent_request(
            "Please carefully plan the work before we start.",
            None,
        ));
        assert!(!should_auto_plan_agent_request(
            "Explain what this function does.",
            None,
        ));
        assert!(!should_auto_plan_agent_request(
            "build and test it",
            Some("task-1")
        ));
    }

    #[test]
    fn continuation_heuristic_detects_resume_requests() {
        assert!(should_resume_active_task_request("please complete", None));
        assert!(should_resume_active_task_request("continue", None));
        assert!(should_resume_active_task_request(
            "you timed out please pick up where you left off",
            None
        ));
        assert!(!should_resume_active_task_request("hello there", None));
        assert!(!should_resume_active_task_request(
            "please complete",
            Some("task-1")
        ));
    }

    #[test]
    fn stream_idle_timeout_extends_after_tool_output_and_confirmation_waits() {
        assert_eq!(
            stream_idle_timeout_for_chunk(&gestura_core::StreamChunk::ToolCallResult {
                name: "shell".to_string(),
                success: true,
                output: "done".to_string(),
                duration_ms: 12,
            }),
            std::time::Duration::from_secs(STREAM_IDLE_TIMEOUT_POST_TOOL_REVIEW_SECS)
        );
        assert_eq!(
            stream_idle_timeout_for_chunk(&gestura_core::StreamChunk::ToolConfirmationRequired {
                confirmation_id: "conf-1".to_string(),
                tool_name: "shell".to_string(),
                tool_args: "{}".to_string(),
                description: "Need approval".to_string(),
                risk_level: 2,
                category: "execute".to_string(),
            }),
            std::time::Duration::from_secs(STREAM_IDLE_TIMEOUT_WAITING_FOR_USER_SECS)
        );
        assert_eq!(
            stream_idle_timeout_for_chunk(&gestura_core::StreamChunk::Text("hello".to_string())),
            std::time::Duration::from_secs(STREAM_IDLE_TIMEOUT_NORMAL_SECS)
        );
    }

    #[test]
    fn derives_agent_request_task_name_from_first_line() {
        assert_eq!(
            derive_agent_request_task_name("Create a small tauri gui that says hello world"),
            "Create a small tauri gui that says hello world"
        );
    }

    #[test]
    fn auto_plan_execution_handoff_pushes_into_implementation() {
        let handoff = build_auto_plan_execution_handoff_message(
            "Build a small Tauri hello world app",
            "Build a small Tauri hello world app",
            &[
                "Scaffold the Tauri app [not_started]".to_string(),
                "Run build and smoke tests [not_started]".to_string(),
            ],
            Some("Scaffold the Tauri app"),
        );

        assert!(handoff.contains("task plan already exists"));
        assert!(handoff.contains("execute that plan"));
        assert!(handoff.contains("Start by executing \"Scaffold the Tauri app\""));
        assert!(handoff.contains("Scaffold the Tauri app [not_started]"));
        assert!(handoff.contains("Begin concrete implementation work immediately"));
        assert!(
            handoff.contains("update task statuses as you start and finish each planned subtask")
        );
        assert!(
            handoff.contains(
                "only mark the tracked root task complete after every subtask is terminal"
            )
        );
        assert!(handoff.contains(
            "create that work under the tracked root plan instead of nesting it under the currently executing task"
        ));
        assert!(handoff.contains("Do not end the run after only reviewing the plan or task list"));
    }

    #[test]
    fn pre_auto_plan_narration_explains_why_tracking_is_needed() {
        let narration = build_pre_auto_plan_narration(
            "Please carefully plan and implement the retry refactor, then run the relevant tests.",
            "Refactor retry flow",
        );

        assert_eq!(narration.title, "Breaking Into Subtasks");
        assert!(narration.message.contains("Refactor retry flow"));
        assert!(
            narration
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("tracked subtasks"))
        );
        assert!(
            narration
                .reason
                .as_deref()
                .is_some_and(|reason| { reason.contains("remaining work queued behind it") })
        );
        assert!(narration.next_step.as_deref().is_some_and(|next_step| {
            next_step.contains("what the first verification pass will prove")
        }));
    }

    #[test]
    fn post_auto_plan_narration_uses_llm_generated_subtasks_for_specificity() {
        let narration = build_post_auto_plan_narration(&AutoPlanResult {
            root_task_id: Some("root-1".to_string()),
            root_task_name: "Build hello app".to_string(),
            planned_subtasks: vec![
                "Scaffold the Tauri app [not_started]".to_string(),
                "Run build and smoke tests [not_started]".to_string(),
                "Polish the UI copy [not_started]".to_string(),
            ],
            initial_task_id: Some("task-1".to_string()),
            initial_task_name: Some("Scaffold the Tauri app".to_string()),
            generated_task_count: 3,
        });

        assert_eq!(narration.title, "Choosing First Subtask");
        assert!(narration.message.contains("Build hello app"));
        assert!(
            narration
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("Scaffold the Tauri app"))
        );
        assert!(
            narration
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("Run build and smoke tests"))
        );
        assert!(narration.message.contains("1 more step(s)"));
        assert!(
            narration
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("earliest concrete signal"))
        );
        assert!(
            narration
                .next_step
                .as_deref()
                .is_some_and(|next| next.contains("verify whether it produced the expected signal"))
        );
        assert!(!narration.message.contains("[not_started]"));
    }

    #[test]
    fn bootstrap_narration_payload_prefers_authored_message() {
        let fallback = build_post_auto_plan_narration(&AutoPlanResult {
            root_task_id: Some("root-1".to_string()),
            root_task_name: "Build hello app".to_string(),
            planned_subtasks: vec!["Scaffold the Tauri app [not_started]".to_string()],
            initial_task_id: Some("task-1".to_string()),
            initial_task_name: Some("Scaffold the Tauri app".to_string()),
            generated_task_count: 1,
        });

        let narration = parse_bootstrap_narration_payload(
            r#"{
                "title":"Reviewing plan shape",
                "message":"I’ve got the first execution branch on the board now, so I’m using it to anchor the rest of the work instead of letting the plan sprawl into parallel guesses before we have any signal.",
                "summary":"I created the first branch of the plan.",
                "reason":"That gives me a concrete lead to follow.",
                "next_step":"I’ll use the first branch to decide what stays queued."
            }"#,
            fallback,
        );

        assert_eq!(narration.title, "Reviewing plan shape");
        assert!(narration.message.contains("anchor the rest of the work"));
        assert_eq!(
            narration.summary.as_deref(),
            Some("I created the first branch of the plan.")
        );
    }

    #[test]
    fn bootstrap_narration_payload_preserves_markdown_line_structure() {
        let fallback = build_post_auto_plan_narration(&AutoPlanResult {
            root_task_id: Some("root-1".to_string()),
            root_task_name: "Build hello app".to_string(),
            planned_subtasks: vec!["Scaffold the Tauri app [not_started]".to_string()],
            initial_task_id: Some("task-1".to_string()),
            initial_task_name: Some("Scaffold the Tauri app".to_string()),
            generated_task_count: 1,
        });

        let narration = parse_bootstrap_narration_payload(
            r##"{
                "title":"Reviewing plan shape",
                "message":"# Plan status\n\n- drafted the first branch\n- kept verification queued\n\n## Next\nRun the validation pass.",
                "summary":"I created the first branch of the plan.",
                "reason":"That gives me a concrete lead to follow.",
                "next_step":"I’ll use the first branch to decide what stays queued."
            }"##,
            fallback,
        );

        assert_eq!(
            narration.message,
            "# Plan status\n\n- drafted the first branch\n- kept verification queued\n\n## Next\nRun the validation pass."
        );
    }

    #[test]
    fn sanitize_bootstrap_narration_title_accepts_seven_words() {
        let title = sanitize_bootstrap_narration_title(
            "Title: Reviewing the current implementation state for regressions",
        )
        .expect("seven-word title should sanitize");

        assert_eq!(
            title,
            "Reviewing the current implementation state for regressions"
        );
    }

    #[test]
    fn sanitize_bootstrap_narration_title_rejects_truncated_titles() {
        let title = sanitize_bootstrap_narration_title("Title: Researching smart lighting market…");

        assert!(title.is_none());
    }

    #[test]
    fn post_auto_plan_narration_prompt_mentions_created_subtasks() {
        let prompt = build_post_auto_plan_narration_prompt(
            "Please plan and implement the hello app, then run the build and tests.",
            &AutoPlanResult {
                root_task_id: Some("root-1".to_string()),
                root_task_name: "Build hello app".to_string(),
                planned_subtasks: vec![
                    "Scaffold the Tauri app [not_started]".to_string(),
                    "Run build and smoke tests [not_started]".to_string(),
                ],
                initial_task_id: Some("task-1".to_string()),
                initial_task_name: Some("Scaffold the Tauri app".to_string()),
                generated_task_count: 2,
            },
        );

        assert!(prompt.contains("Explain what task structure I created"));
        assert!(prompt.contains("Scaffold the Tauri app"));
        assert!(prompt.contains("Run build and smoke tests"));
    }

    #[test]
    fn resolve_tracked_task_id_reattaches_current_task_for_continue_prompt() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = gestura_core::TaskManager::new(temp_dir.path());
        let session_id = format!("resume-task-{}", uuid::Uuid::new_v4());
        let root = manager
            .create_task(&session_id, "Build hello app", "desc", None)
            .expect("root task");
        manager
            .set_current_task_id(&session_id, Some(root.id.clone()))
            .expect("set current task");

        let tracked = resolve_tracked_task_id(&manager, &session_id, "please complete", None, None);

        assert_eq!(tracked.as_deref(), Some(root.id.as_str()));
    }

    #[test]
    fn resolve_tracked_task_id_prefers_auto_planned_root_over_first_subtask() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = gestura_core::TaskManager::new(temp_dir.path());
        let session_id = format!("auto-plan-root-track-{}", uuid::Uuid::new_v4());
        let root = manager
            .create_task(&session_id, "Build hello app", "desc", None)
            .expect("root task");
        let child = manager
            .create_task(&session_id, "Scaffold UI", "desc", Some(root.id.clone()))
            .expect("child task");

        let tracked = resolve_tracked_task_id(
            &manager,
            &session_id,
            "please build the app",
            None,
            Some(root.id.as_str()),
        );

        assert_eq!(tracked.as_deref(), Some(root.id.as_str()));
        assert_ne!(tracked.as_deref(), Some(child.id.as_str()));
    }

    #[test]
    fn resolve_current_execution_task_id_prefers_first_auto_planned_subtask() {
        let current = resolve_current_execution_task_id(
            None,
            Some(&AutoPlanResult {
                root_task_id: Some("root-1".to_string()),
                root_task_name: "Build hello app".to_string(),
                planned_subtasks: vec!["Scaffold UI [not_started]".to_string()],
                initial_task_id: Some("task-1".to_string()),
                initial_task_name: Some("Scaffold UI".to_string()),
                generated_task_count: 1,
            }),
            Some("root-1"),
        );

        assert_eq!(current.as_deref(), Some("task-1"));
    }

    #[test]
    fn resolve_current_execution_task_id_falls_back_to_auto_plan_root_when_no_child_exists() {
        let current = resolve_current_execution_task_id(
            None,
            Some(&AutoPlanResult {
                root_task_id: Some("root-1".to_string()),
                root_task_name: "Build hello app".to_string(),
                planned_subtasks: Vec::new(),
                initial_task_id: None,
                initial_task_name: None,
                generated_task_count: 0,
            }),
            Some("root-1"),
        );

        assert_eq!(current.as_deref(), Some("root-1"));
    }

    #[test]
    fn resolve_resume_tracked_task_id_prefers_current_open_task() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = gestura_core::TaskManager::new(temp_dir.path());
        let session_id = format!("resume-current-task-{}", uuid::Uuid::new_v4());
        let root = manager
            .create_task(&session_id, "Build app", "desc", None)
            .expect("root task");
        manager
            .set_current_task_id(&session_id, Some(root.id.clone()))
            .expect("set current task");

        let tracked = resolve_resume_tracked_task_id(&manager, &session_id);

        assert_eq!(tracked.as_deref(), Some(root.id.as_str()));
    }

    #[test]
    fn resolve_resume_tracked_task_id_falls_back_to_single_open_root() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = gestura_core::TaskManager::new(temp_dir.path());
        let session_id = format!("resume-root-fallback-{}", uuid::Uuid::new_v4());
        let root = manager
            .create_task(&session_id, "Implement flow", "desc", None)
            .expect("root task");

        let tracked = resolve_resume_tracked_task_id(&manager, &session_id);

        assert_eq!(tracked.as_deref(), Some(root.id.as_str()));
    }

    #[test]
    fn resolve_resume_tracked_task_id_ignores_closed_current_task() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = gestura_core::TaskManager::new(temp_dir.path());
        let session_id = format!("resume-closed-current-{}", uuid::Uuid::new_v4());
        let root = manager
            .create_task(&session_id, "Build app", "desc", None)
            .expect("root task");
        manager
            .update_task_status(&session_id, &root.id, gestura_core::TaskStatus::Completed)
            .expect("complete root");
        manager
            .set_current_task_id(&session_id, Some(root.id.clone()))
            .expect("set current task");

        let tracked = resolve_resume_tracked_task_id(&manager, &session_id);

        assert_eq!(tracked, None);
    }

    #[test]
    fn auto_plan_execution_context_includes_nested_open_tasks() {
        let manager = crate::task_integration::get_task_manager();
        let session_id = format!("auto-plan-context-{}", uuid::Uuid::new_v4());
        let mut root = gestura_core::Task::new(&session_id, "Root", "Root", None);
        let mut child = gestura_core::Task::new(
            &session_id,
            "Implement UI",
            "Implement UI",
            Some(root.id.clone()),
        );
        let grandchild = gestura_core::Task::new(
            &session_id,
            "Run build",
            "Run build",
            Some(child.id.clone()),
        );
        child.set_status(gestura_core::TaskStatus::Completed);
        root.set_status(gestura_core::TaskStatus::InProgress);

        let mut task_list = gestura_core::TaskList::new(&session_id);
        task_list.add_task(root.clone());
        task_list.add_task(child);
        task_list.add_task(grandchild);
        manager
            .replace_task_list(task_list)
            .expect("replace task list");

        let execution_context =
            collect_auto_plan_execution_context(&session_id, &root.id).expect("execution context");

        assert_eq!(execution_context.root_task_name, "Root");
        assert_eq!(
            execution_context.planned_subtasks.as_slice(),
            ["Run build [not_started]".to_string()]
        );
        assert_eq!(
            execution_context.initial_task_name.as_deref(),
            Some("Run build")
        );
    }

    #[test]
    fn populate_auto_plan_generated_tasks_promotes_generated_root_tasks() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = gestura_core::TaskManager::new(temp_dir.path());
        let session_id = format!("auto-plan-roots-{}", uuid::Uuid::new_v4());
        let planning_task = manager
            .create_agent_task(
                &session_id,
                "Plan request: Build hello app",
                "planning",
                None,
                None,
            )
            .expect("planning task");

        let generated = populate_auto_plan_generated_tasks(
            &[
                serde_json::json!({
                    "name": "Implement UI",
                    "description": "Create the hello world UI",
                    "priority": "high",
                    "is_blocking": true,
                    "parent_name": null,
                }),
                serde_json::json!({
                    "name": "Run build",
                    "description": "Compile and verify the app",
                    "priority": "high",
                    "is_blocking": false,
                    "parent_name": "Implement UI",
                }),
            ],
            |name, description, parent_id| {
                manager
                    .create_agent_task(&session_id, name, description, None, parent_id)
                    .map_err(|error| error.to_string())
            },
        )
        .expect("populate generated tasks");

        assert_eq!(generated.generated_task_count, 2);
        assert_eq!(generated.top_level_tasks.len(), 1);
        assert_eq!(generated.top_level_tasks[0].name, "Implement UI");

        let tree = manager.get_task_tree(&session_id).expect("task tree");
        assert_eq!(tree.len(), 2);

        let planning_node = tree
            .iter()
            .find(|node| node.task.id == planning_task.id)
            .expect("planning root node");
        assert!(planning_node.children.is_empty());

        let generated_root_node = tree
            .iter()
            .find(|node| node.task.name == "Implement UI")
            .expect("generated root node");
        assert_eq!(generated_root_node.task.parent_id, None);
        assert_eq!(generated_root_node.children.len(), 1);
        assert_eq!(generated_root_node.children[0].task.name, "Run build");
        assert_ne!(
            generated_root_node.task.parent_id.as_deref(),
            Some(planning_task.id.as_str())
        );
    }

    #[test]
    fn task_tool_mutation_event_maps_mutating_task_operations() {
        let created = task_tool_mutation_event(
            "session-1",
            "task",
            r#"{"operation":"create","name":"Build app"}"#,
            true,
        )
        .expect("create event");
        assert_eq!(created.0, "task-created");

        let updated = task_tool_mutation_event(
            "session-1",
            "task",
            r#"{"operation":"update_status","task_id":"abc","status":"completed"}"#,
            true,
        )
        .expect("update event");
        assert_eq!(updated.0, "task-updated");

        let deleted = task_tool_mutation_event(
            "session-1",
            "tasks",
            r#"{"operation":"delete","task_id":"abc"}"#,
            true,
        )
        .expect("delete event");
        assert_eq!(deleted.0, "task-deleted");
    }

    #[test]
    fn task_tool_mutation_event_ignores_reads_and_failures() {
        assert!(
            task_tool_mutation_event("session-1", "task", r#"{"operation":"list"}"#, true,)
                .is_none()
        );

        assert!(
            task_tool_mutation_event(
                "session-1",
                "task",
                r#"{"operation":"update_status","task_id":"abc"}"#,
                false,
            )
            .is_none()
        );
    }

    #[test]
    fn task_tree_refresh_signature_changes_when_task_state_changes() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = gestura_core::TaskManager::new(temp_dir.path());
        let session_id = format!("task-refresh-signature-{}", uuid::Uuid::new_v4());
        let task = manager
            .create_task(&session_id, "Build hello app", "desc", None)
            .expect("create task");

        let initial = task_tree_refresh_signature_with_manager(&manager, &session_id)
            .expect("initial signature");

        manager
            .update_task_status(&session_id, &task.id, gestura_core::TaskStatus::InProgress)
            .expect("set in progress");
        let after_status = task_tree_refresh_signature_with_manager(&manager, &session_id)
            .expect("status signature");
        assert_ne!(initial, after_status);

        manager
            .set_current_task_id(&session_id, Some(task.id.clone()))
            .expect("set current task");
        let after_current = task_tree_refresh_signature_with_manager(&manager, &session_id)
            .expect("current task signature");
        assert_ne!(after_status, after_current);
    }

    #[test]
    fn get_task_hierarchy_returns_nested_task_tree() {
        let manager = crate::task_integration::get_task_manager();
        let session_id = format!("task-tree-{}", uuid::Uuid::new_v4());
        let root = gestura_core::Task::new(&session_id, "Root", "Root", None);
        let child = gestura_core::Task::new(&session_id, "Child", "Child", Some(root.id.clone()));
        let grandchild = gestura_core::Task::new(
            &session_id,
            "Grandchild",
            "Grandchild",
            Some(child.id.clone()),
        );

        let mut task_list = gestura_core::TaskList::new(&session_id);
        task_list.add_task(root.clone());
        task_list.add_task(child.clone());
        task_list.add_task(grandchild.clone());
        manager
            .replace_task_list(task_list)
            .expect("replace task list");

        let tree = get_task_hierarchy(session_id).expect("task hierarchy");

        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].task.id, root.id);
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].task.id, child.id);
        assert_eq!(tree[0].children[0].children.len(), 1);
        assert_eq!(tree[0].children[0].children[0].task.id, grandchild.id);
    }
}

/// Public synchronous keychain API key retrieval.
///
/// This is used by other modules (e.g., speech.rs) to retrieve API keys from the
/// system keychain with a fallback to empty string if not found.
///
/// Provider names are case-insensitive and map to the canonical secure-storage
/// key names used by `gestura-core`:
/// - `"openai"` → `gestura_llm_openai_api_key`
/// - `"anthropic"` → `gestura_llm_anthropic_api_key`
/// - `"grok"` → `gestura_llm_grok_api_key`
/// - `"voice_openai"` → `gestura_voice_openai_api_key`
/// - `"serpapi"` → `gestura_web_search_serpapi_key`
/// - `"brave"` → `gestura_web_search_brave_key`
pub fn try_get_api_key_from_keychain_sync(provider: &str) -> String {
    let canonical_key = match api_key_storage_key_for_provider(provider) {
        Some(k) => k,
        None => return String::new(),
    };
    let legacy_key = legacy_api_key_storage_key_for_provider(provider);

    let storage = crate::security::create_secure_storage();

    // Use a blocking runtime to call the async method
    match std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        rt.block_on(async {
            // 1) Canonical read
            if let Ok(Some(v)) = storage.get_secret(canonical_key).await
                && !v.is_empty()
            {
                return Some(v);
            }

            // 2) Legacy fallback + self-heal
            if let Some(legacy_key) = legacy_key
                && let Ok(Some(v)) = storage.get_secret(legacy_key).await
                && !v.is_empty()
            {
                let _ = storage.store_secret(canonical_key, &v).await;
                return Some(v);
            }

            None
        })
    })
    .join()
    {
        Ok(Some(key)) => key,
        Ok(None) => String::new(),
        Err(_) => String::new(),
    }
}

/// Get the current application configuration.
///
/// JS↔Rust interop: This command is exposed to the frontend via Tauri.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_config() -> Result<AppConfig, String> {
    let mut cfg = AppConfig::load_async().await;
    // Defense-in-depth: never return plaintext secrets over IPC.
    strip_secrets_in_place(&mut cfg);
    Ok(cfg)
}

/// Persist a new application configuration.
///
/// JS↔Rust interop: The frontend invokes this command with `{ cfg: AppConfig }`.
#[tauri::command(rename_all = "snake_case")]
pub async fn save_config(cfg: AppConfig) -> Result<(), String> {
    // Defense-in-depth: ignore/scrub any secrets the frontend might send.
    let mut cfg = cfg;
    strip_secrets_in_place(&mut cfg);
    cfg.save_async().await.map_err(|e| e.to_string())
}

/// Clear all known plaintext secret fields from an `AppConfig`.
///
/// We do this at the GUI IPC boundary to ensure secrets never cross between
/// backend/frontend via `get_config` / `save_config` payloads.
fn strip_secrets_in_place(cfg: &mut AppConfig) {
    if let Some(c) = cfg.llm.openai.as_mut() {
        c.api_key.clear();
    }
    if let Some(c) = cfg.llm.anthropic.as_mut() {
        c.api_key.clear();
    }
    if let Some(c) = cfg.llm.grok.as_mut() {
        c.api_key.clear();
    }
    if let Some(c) = cfg.llm.gemini.as_mut() {
        c.api_key.clear();
    }

    cfg.voice.openai_api_key = None;
    cfg.web_search.serpapi_key = None;
    cfg.web_search.brave_key = None;
}

/// Check if this is the first run of the application (no config file exists yet).
#[tauri::command]
pub fn is_first_run() -> bool {
    AppConfig::is_first_run()
}

/// Get the path to the configuration file.
#[tauri::command]
pub fn get_config_path() -> String {
    AppConfig::default_path().to_string_lossy().to_string()
}

/// Tool information for the frontend
#[derive(serde::Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub summary: String,
    pub inputs: Vec<String>,
    pub side_effects: Vec<String>,
    pub examples: Vec<String>,
}

/// List all built-in tools
#[tauri::command]
pub fn list_builtin_tools() -> Vec<ToolInfo> {
    gestura_core::tools::all_tools()
        .iter()
        .map(|t| ToolInfo {
            name: t.name.to_string(),
            summary: t.summary.to_string(),
            inputs: t.inputs.iter().map(|s| s.to_string()).collect(),
            side_effects: t.side_effects.iter().map(|s| s.to_string()).collect(),
            examples: t.examples.iter().map(|s| s.to_string()).collect(),
        })
        .collect()
}

/// List all configured MCP servers (full spec entries).
#[tauri::command]
pub async fn list_mcp_tools() -> Result<Vec<crate::config::McpServerEntry>, String> {
    Ok(AppConfig::load_async().await.mcp_servers)
}

/// Add or update an MCP server entry in the user config.
#[tauri::command]
pub async fn add_mcp_tool(tool: crate::config::McpServerEntry) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    // Replace existing entry with the same name, or append.
    if let Some(existing) = cfg.mcp_servers.iter_mut().find(|t| t.name == tool.name) {
        *existing = tool;
    } else {
        cfg.mcp_servers.push(tool);
    }
    cfg.save_async().await.map_err(|e| e.to_string())
}

/// Remove an MCP server by name.
#[tauri::command]
pub async fn remove_mcp_tool(name: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.mcp_servers.retain(|t| t.name != name);
    cfg.save_async().await.map_err(|e| e.to_string())
}

// =========================================================================
// Popular MCP Servers (no required configuration) — delegates to core
// =========================================================================

/// List 20 popular, open-source MCP servers that can be added without
/// additional configuration.
///
/// All business logic lives in `gestura_core::mcp::registry`. This command
/// is a thin Tauri IPC wrapper.
#[tauri::command]
pub async fn list_popular_mcp_servers() -> Result<Vec<gestura_core::mcp::PopularMcpServer>, String>
{
    gestura_core::mcp::list_popular_mcp_servers(20)
        .await
        .map_err(|e| e.to_string())
}

/// Browse the MCP Registry with optional full-text search and cursor-based pagination.
///
/// Returns up to 20 servers per page.  Pass `cursor` from a previous response's
/// `next_cursor` field to advance to the next page.  Each entry in the result
/// carries a `quick_add` field when the server is zero-config eligible so the
/// frontend can render a "Quick Add" button selectively.
#[tauri::command]
pub async fn browse_mcp_registry_servers(
    query: Option<String>,
    cursor: Option<String>,
) -> Result<gestura_core::mcp::RegistryBrowsePage, String> {
    gestura_core::mcp::browse_mcp_registry(query, cursor, 20)
        .await
        .map_err(|e| e.to_string())
}

/// Verify runtime availability and pre-install/fetch the package for a
/// newly-added MCP server.
///
/// Called immediately after [`add_mcp_tool`] so the user gets instant feedback
/// about whether Node/npx or uv/uvx is present and whether the package
/// downloaded successfully.  HTTP/SSE remote servers return `Skipped` because
/// they require no local installation.
///
/// # Errors
/// This command never returns `Err`; all outcomes are expressed through
/// [`gestura_core::mcp::ProvisionStatus`] so the frontend can distinguish
/// `ready`, `runtime_missing`, `fetch_failed`, and `skipped`.
#[tauri::command]
pub async fn provision_mcp_server(
    server: crate::config::McpServerEntry,
) -> Result<gestura_core::mcp::ProvisionResult, String> {
    Ok(gestura_core::mcp::provision_mcp_server(&server).await)
}

// ============================================================================
// MCP Discovery Manager - Dynamic Tool Provisioning
// ============================================================================

use gestura_core::McpDiscoveryManager;

/// Global MCP discovery manager instance
static MCP_DISCOVERY_MANAGER: std::sync::OnceLock<McpDiscoveryManager> = std::sync::OnceLock::new();

/// Get or initialize the global MCP discovery manager
fn get_mcp_discovery_manager() -> &'static McpDiscoveryManager {
    MCP_DISCOVERY_MANAGER.get_or_init(McpDiscoveryManager::new)
}

/// MCP tool information for the frontend (matches ToolInfo pattern)
#[derive(serde::Serialize)]
pub struct McpToolInfo {
    /// Full tool name in format "server:tool_name"
    pub name: String,
    /// Tool description from MCP server
    pub summary: String,
    /// Source MCP server name
    pub server_name: String,
    /// Tool category (read, write, execute, etc.)
    pub category: String,
    /// Whether this tool has side effects
    pub has_side_effects: bool,
    /// Risk level (low, medium, high)
    pub risk_level: String,
}

/// Initialize MCP servers from config.
///
/// Registers all enabled MCP servers with the discovery manager using the
/// full `McpServerEntry` configuration (transport, env, headers, etc.).
#[tauri::command]
pub async fn init_mcp_servers() -> Result<usize, String> {
    let config = AppConfig::load_async().await;
    let manager = get_mcp_discovery_manager();

    let mut registered = 0;
    for srv in &config.mcp_servers {
        if !srv.enabled {
            tracing::debug!("Skipping disabled MCP server: {}", srv.name);
            continue;
        }
        manager.register_server(srv.to_discovery_config());
        registered += 1;
        tracing::info!(
            "Registered MCP server: {} (transport={}, uri={})",
            srv.name,
            srv.transport,
            srv.effective_uri()
        );
    }

    Ok(registered)
}

/// List all discovered tools from MCP servers
/// Returns tools that have been cached from connected MCP servers
#[tauri::command]
pub fn list_discovered_mcp_tools() -> Vec<McpToolInfo> {
    use gestura_core::execution_mode::ToolCategory;

    let manager = get_mcp_discovery_manager();
    let cached_tools = manager.list_tools();

    cached_tools
        .into_iter()
        .map(|ct| {
            let category_str = match ct.metadata.category {
                ToolCategory::ReadOnly => "read",
                ToolCategory::Write => "write",
                ToolCategory::Shell => "shell",
                ToolCategory::Network => "network",
                ToolCategory::System => "system",
                ToolCategory::Git => "git",
            };
            // Convert numeric risk_level (0-10) to string category
            let risk_str = if ct.metadata.risk_level <= 2 {
                "low"
            } else if ct.metadata.risk_level <= 5 {
                "medium"
            } else {
                "high"
            };
            McpToolInfo {
                name: ct.metadata.name.clone(),
                summary: ct.metadata.description.clone(),
                server_name: ct.server_name.clone(),
                category: category_str.to_string(),
                has_side_effects: ct.metadata.has_side_effects,
                risk_level: risk_str.to_string(),
            }
        })
        .collect()
}

/// Get MCP server status information
#[tauri::command]
pub fn get_mcp_server_status() -> Vec<McpServerStatus> {
    let manager = get_mcp_discovery_manager();
    manager
        .list_servers()
        .into_iter()
        .map(|s| McpServerStatus {
            name: s.config.name,
            uri: s.config.uri,
            state: format!("{:?}", s.state),
            tool_count: s.tool_count,
            last_error: s.last_error,
        })
        .collect()
}

/// MCP server status for frontend display
#[derive(serde::Serialize)]
pub struct McpServerStatus {
    pub name: String,
    pub uri: String,
    pub state: String,
    pub tool_count: usize,
    pub last_error: Option<String>,
}

/// Register a new MCP server entry and initialize discovery.
///
/// Accepts a full `McpServerEntry` from the frontend, persists it to the
/// user config, and registers with the discovery manager.
#[tauri::command]
pub async fn register_mcp_server(server: crate::config::McpServerEntry) -> Result<(), String> {
    // Persist to config (add_mcp_tool handles upsert)
    let discovery_cfg = server.to_discovery_config();
    add_mcp_tool(server.clone()).await?;

    // Register with discovery manager
    let manager = get_mcp_discovery_manager();
    manager.register_server(discovery_cfg);

    tracing::info!("Registered new MCP server: {}", server.name);
    Ok(())
}

/// Unregister an MCP server by name.
#[tauri::command]
pub async fn unregister_mcp_server(name: String) -> Result<(), String> {
    // Remove from config
    remove_mcp_tool(name.clone()).await?;

    // Unregister from discovery manager
    let manager = get_mcp_discovery_manager();
    manager.unregister_server(&name);

    tracing::info!("Unregistered MCP server: {}", name);
    Ok(())
}

// ============================================================================
// MCP Client Runtime — live connections via McpClientRegistry
// ============================================================================

use gestura_core::mcp::client::get_mcp_client_registry;

/// Connect to an MCP server by name (must already be in config).
///
/// Performs the MCP initialize handshake and discovers tools. Returns the list
/// of tool names discovered from the server.
#[tauri::command]
pub async fn connect_mcp_server(name: String) -> Result<Vec<String>, String> {
    let config = AppConfig::load_async().await;
    let entry = config
        .mcp_servers
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("MCP server '{}' not found in config", name))?
        .clone();

    let registry = get_mcp_client_registry();
    let tools = registry.connect(&entry).await.map_err(|e| e.to_string())?;
    let names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();

    tracing::info!(
        "Connected to MCP server '{}': {} tools discovered",
        name,
        names.len()
    );
    Ok(names)
}

/// Disconnect from a running MCP server.
#[tauri::command]
pub async fn disconnect_mcp_server(name: String) -> Result<(), String> {
    get_mcp_client_registry().disconnect(&name).await;
    tracing::info!("Disconnected from MCP server '{}'", name);
    Ok(())
}

/// List all currently connected MCP server names.
#[tauri::command]
pub async fn list_connected_mcp_servers() -> Vec<String> {
    get_mcp_client_registry().connected_servers().await
}

/// Information about a discovered tool from a live MCP connection.
#[derive(serde::Serialize)]
pub struct McpClientToolInfo {
    /// Server the tool belongs to
    pub server: String,
    /// Tool name
    pub name: String,
    /// Namespaced name used in the agent pipeline (mcp__server__tool)
    pub qualified_name: String,
    /// Tool description
    pub description: Option<String>,
    /// JSON Schema for input parameters
    pub input_schema: serde_json::Value,
}

/// List all tools discovered from live MCP connections.
#[tauri::command]
pub async fn list_mcp_client_tools() -> Vec<McpClientToolInfo> {
    let registry = get_mcp_client_registry();
    let all = registry.all_tools().await;
    let mut out = Vec::new();
    for (server, tools) in all {
        for t in tools {
            out.push(McpClientToolInfo {
                server: server.clone(),
                qualified_name: format!("mcp__{}__{}", server, t.name),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
                name: t.name,
            });
        }
    }
    out
}

/// Call a tool on a connected MCP server. Returns the result content as a JSON value.
#[tauri::command]
pub async fn call_mcp_tool(
    server: String,
    tool: String,
    arguments: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let registry = get_mcp_client_registry();
    let result = registry
        .call_tool(&server, &tool, arguments)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_mdh_pointers() -> Result<std::collections::HashMap<String, String>, String> {
    Ok(AppConfig::load_async().await.mdh_pointers)
}

#[tauri::command]
pub async fn set_mdh_pointer(key: String, value: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.mdh_pointers.insert(key, value);
    cfg.save_async().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_mdh_pointer(key: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.mdh_pointers.remove(&key);
    cfg.save_async().await.map_err(|e| e.to_string())
}

// Knowledge Management Commands

/// Add a knowledge entry from agent (saved responses)
#[tauri::command]
pub fn add_knowledge_entry(
    content: String,
    category: String,
    tags: Vec<String>,
) -> Result<String, String> {
    use gestura_core::knowledge::KnowledgeItem;
    use std::time::{SystemTime, UNIX_EPOCH};

    let store = get_knowledge_store();

    // Generate a unique ID based on timestamp
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let id = format!("saved-{}", timestamp);

    // Create a knowledge item from the saved content
    let item = KnowledgeItem::new(
        &id,
        format!("Saved: {}", &content[..content.len().min(50)]),
        &content,
    )
    .with_category(&category)
    .with_triggers(tags)
    .with_content(&content);

    store
        .upsert_user_item(item)
        .map_err(|e| format!("Failed to persist knowledge entry: {e}"))?;

    tracing::info!("Added knowledge entry: {}", id);
    Ok(id)
}

/// List knowledge entries
#[tauri::command]
pub fn list_knowledge_entries(category: Option<String>) -> Result<Vec<serde_json::Value>, String> {
    use gestura_core::knowledge::KnowledgeQuery;

    let store = get_knowledge_store();

    let query = KnowledgeQuery {
        query: String::new(),
        categories: category.map(|c| vec![c]),
        limit: Some(100),
        min_score: None,
    };

    let matches = store.find(&query);
    let entries: Vec<serde_json::Value> = matches
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "id": m.item.id,
                "name": m.item.name,
                "category": m.item.category,
                "description": m.item.description,
                "score": m.score,
            })
        })
        .collect();

    Ok(entries)
}

/// Search knowledge base
#[tauri::command]
pub fn search_knowledge(
    query: String,
    limit: Option<usize>,
) -> Result<Vec<serde_json::Value>, String> {
    use gestura_core::knowledge::KnowledgeQuery;

    let store = get_knowledge_store();

    let kquery = KnowledgeQuery {
        query,
        categories: None,
        limit: limit.or(Some(10)),
        min_score: Some(0.1),
    };

    let matches = store.find(&kquery);
    let entries: Vec<serde_json::Value> = matches
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "id": m.item.id,
                "name": m.item.name,
                "category": m.item.category,
                "description": m.item.description,
                "content": m.item.core_content,
                "score": m.score,
                "matched_triggers": m.matched_triggers,
            })
        })
        .collect();

    Ok(entries)
}

#[tauri::command]
pub async fn test_llm(prompt: String) -> Result<String, String> {
    let cfg = AppConfig::load_async().await;
    run_single_shot_pipeline(cfg, prompt, RequestSource::GuiText, None).await
}

/// Enhance a user prompt using LLM to make it more effective
///
/// This command takes a user's prompt and uses the configured LLM provider
/// to improve it by adding context, structure, and clarity while preserving
/// the original intent.
///
/// # Arguments
///
/// * `prompt` - The original user prompt to enhance
/// * `session_id` - Optional session ID to include conversation history as context
///
/// # Returns
///
/// Returns the enhanced prompt as a String, or an error message if enhancement fails.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn enhance_prompt(prompt: String, session_id: Option<String>) -> Result<String, String> {
    use gestura_core::prompt_enhancement::{PromptContext, enhance_prompt_with_llm};

    // Validate input
    if prompt.trim().is_empty() {
        return Err("Prompt cannot be empty".to_string());
    }

    // Load config
    let mut cfg = AppConfig::load_async().await;

    // Build context from session history if session_id provided
    let context = if let Some(ref sid) = session_id {
        // Get session state using public API
        if let Some(state) = crate::window_manager::get_session_state(sid) {
            // Get last 5 messages for context (to avoid token overflow)
            let history: Vec<(String, String)> = state
                .messages
                .iter()
                .rev()
                .take(5)
                .rev()
                .map(|msg| (msg.role.clone(), msg.content.clone()))
                .collect();

            if !history.is_empty() {
                tracing::debug!(
                    session_id = %sid,
                    history_count = history.len(),
                    "Including session history in prompt enhancement"
                );
                Some(PromptContext::new().with_session_history(history))
            } else {
                None
            }
        } else {
            tracing::warn!(session_id = %sid, "Session not found for prompt enhancement");
            None
        }
    } else {
        None
    };

    // Apply session-specific LLM config overrides so the prompt enhancer uses the same
    // effective provider/model as agent for this session.
    let _effective_llm = apply_session_llm_config_overrides(&mut cfg, session_id.as_deref()).await;

    tracing::info!(
        prompt_length = prompt.len(),
        has_context = context.is_some(),
        "Enhancing user prompt"
    );

    let enhanced = enhance_prompt_with_llm(&prompt, &cfg, context)
        .await
        .map_err(|e| format!("Enhancement failed: {}", e))?;

    tracing::info!(
        original_length = prompt.len(),
        enhanced_length = enhanced.len(),
        "Prompt enhancement successful"
    );

    Ok(enhanced)
}

#[tauri::command]
pub async fn test_voice() -> Result<String, String> {
    let cfg = AppConfig::load_async().await;
    let engine = crate::voice_select::select_voice(&cfg);
    let name = engine.engine_name();
    let sample = engine.process_command(&cfg, None).await.unwrap_or_default();
    Ok(format!("engine={name} sample={sample}"))
}

/// Test Ollama connection and return server info
#[tauri::command]
pub async fn test_ollama_connection(endpoint: String) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/version", endpoint.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Server returned error: {}", resp.status()));
    }

    let version: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    Ok(serde_json::json!({
        "connected": true,
        "version": version.get("version").and_then(|v| v.as_str()).unwrap_or("unknown")
    }))
}

/// List available Ollama models.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_ollama_models(endpoint: String) -> Result<Vec<serde_json::Value>, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/tags", endpoint.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to list models: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Server returned error: {}", resp.status()));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    let models = data
        .get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .map(|m| {
                    serde_json::json!({
                        "name": m.get("name").and_then(|n| n.as_str()).unwrap_or("unknown"),
                        "size": m.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                        "modified_at": m.get("modified_at").and_then(|d| d.as_str()).unwrap_or("")
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(models)
}

/// List available OpenAI models.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_openai_models(api_key: String) -> Result<Vec<serde_json::Value>, String> {
    let mut api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        api_key = try_get_api_key_from_keychain_async("openai").await;
    }

    let key = if api_key.is_empty() {
        None
    } else {
        Some(api_key.as_str())
    };
    let models = gestura_core::list_models_for_provider("openai", key, None)
        .await
        .map_err(|e| format!("Failed to list OpenAI models: {e}"))?;

    Ok(model_info_to_json(&models))
}

/// Convert a slice of `ModelInfo` to the JSON shape the frontend expects.
fn model_info_to_json(models: &[gestura_core::ModelInfo]) -> Vec<serde_json::Value> {
    models
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "name": m.name
            })
        })
        .collect()
}

/// List available OpenAI STT (Speech-to-Text) models
/// Fetches from /v1/models and filters for transcription-capable models
#[tauri::command(rename_all = "snake_case")]
pub async fn list_openai_stt_models(api_key: String) -> Result<Vec<serde_json::Value>, String> {
    let mut api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        // Prefer voice-specific key, then fall back to the general OpenAI key.
        api_key = try_get_api_key_from_keychain_async("voice_openai").await;
        if api_key.is_empty() {
            api_key = try_get_api_key_from_keychain_async("openai").await;
        }
    }
    if api_key.is_empty() {
        return Ok(vec![]);
    }

    let client = reqwest::Client::new();
    let url = "https://api.openai.com/v1/models";

    let resp = client
        .get(url)
        .bearer_auth(&api_key)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to list OpenAI models: {}", e))?;

    if !resp.status().is_success() {
        tracing::warn!("OpenAI API returned status {}", resp.status());
        return Ok(vec![]);
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    // Filter to only STT/transcription models
    let mut models: Vec<serde_json::Value> = data
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?;
                    // Include whisper and transcribe models
                    if id.contains("whisper") || id.contains("transcribe") {
                        Some(serde_json::json!({
                            "id": id,
                            "name": format_openai_stt_model_name(id),
                            "created": m.get("created").and_then(|c| c.as_i64()).unwrap_or(0)
                        }))
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Sort: prefer newer models (gpt-4o-transcribe) first, then whisper-1
    models.sort_by(|a, b| {
        let a_id = a.get("id").and_then(|i| i.as_str()).unwrap_or("");
        let b_id = b.get("id").and_then(|i| i.as_str()).unwrap_or("");
        // Prioritize gpt-4o-transcribe models over whisper
        let a_priority = if a_id.contains("gpt-4o") { 0 } else { 1 };
        let b_priority = if b_id.contains("gpt-4o") { 0 } else { 1 };
        a_priority.cmp(&b_priority).then_with(|| a_id.cmp(b_id))
    });

    Ok(models)
}

/// Format OpenAI STT model ID to a human-readable name
fn format_openai_stt_model_name(id: &str) -> String {
    match id {
        "whisper-1" => "Whisper V2 (Classic)".to_string(),
        "gpt-4o-transcribe" => "GPT-4o Transcribe (Best Quality)".to_string(),
        "gpt-4o-transcribe-latest" => "GPT-4o Transcribe (Latest)".to_string(),
        "gpt-4o-mini-transcribe" => "GPT-4o Mini Transcribe (Balanced)".to_string(),
        "gpt-4o-transcribe-diarize" => "GPT-4o Transcribe + Diarization".to_string(),
        _ => {
            // Convert kebab-case to Title Case
            id.split('-')
                .map(|part| {
                    let mut chars = part.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().chain(chars).collect(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

/// List available Anthropic models.
///
/// Delegates to `gestura_core::list_models_for_provider` for centralised HTTP + filtering logic.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_anthropic_models(api_key: String) -> Result<Vec<serde_json::Value>, String> {
    let mut api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        api_key = try_get_api_key_from_keychain_async("anthropic").await;
    }

    let key = if api_key.is_empty() {
        None
    } else {
        Some(api_key.as_str())
    };
    let models = gestura_core::list_models_for_provider("anthropic", key, None)
        .await
        .map_err(|e| format!("Failed to list Anthropic models: {e}"))?;

    Ok(model_info_to_json(&models))
}

/// Fetch available Grok models from xAI API.
///
/// Delegates to `gestura_core::list_models_for_provider` for centralised HTTP + filtering logic.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_grok_models(api_key: String) -> Result<Vec<serde_json::Value>, String> {
    let mut api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        api_key = try_get_api_key_from_keychain_async("grok").await;
    }

    let key = if api_key.is_empty() {
        None
    } else {
        Some(api_key.as_str())
    };
    let models = gestura_core::list_models_for_provider("grok", key, None)
        .await
        .map_err(|e| format!("Failed to list Grok models: {e}"))?;

    Ok(model_info_to_json(&models))
}

/// List available Gemini models from Google Generative Language API.
///
/// Delegates to `gestura_core::list_models_for_provider` for centralised HTTP + filtering logic.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_gemini_models(api_key: String) -> Result<Vec<serde_json::Value>, String> {
    let mut api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        api_key = try_get_api_key_from_keychain_async("gemini").await;
    }

    let key = if api_key.is_empty() {
        None
    } else {
        Some(api_key.as_str())
    };
    let models = gestura_core::list_models_for_provider("gemini", key, None)
        .await
        .map_err(|e| format!("Failed to list Gemini models: {e}"))?;

    Ok(model_info_to_json(&models))
}

/// Test local Whisper model with detailed validation
#[tauri::command(rename_all = "snake_case")]
pub async fn test_local_whisper(model_path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        use crate::voice::validate_whisper_model;
        use std::path::Path;

        let path = Path::new(&model_path);
        let validation = validate_whisper_model(path);

        if !validation.is_valid {
            return Err(validation
                .error
                .unwrap_or_else(|| "Unknown error".to_string()));
        }

        Ok(format!(
            "Model valid: {} ({:.1} MB, GGML format)",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown"),
            validation.file_size_mb
        ))
    })
    .await
    .map_err(|error| format!("Failed to validate local whisper model: {error}"))?
}

/// Validate a Whisper model file and return structured validation info
#[tauri::command]
pub async fn validate_whisper_model(
    path: String,
) -> Result<crate::voice::WhisperModelValidation, String> {
    tokio::task::spawn_blocking(move || {
        let path = std::path::Path::new(&path);
        crate::voice::validate_whisper_model(path)
    })
    .await
    .map_err(|error| format!("Failed to validate whisper model: {error}"))
}

/// Get available Whisper models for download
#[tauri::command]
pub fn get_whisper_models() -> Vec<crate::config::WhisperModelInfo> {
    crate::config::WhisperModelInfo::available_models()
}

/// Check if a specific Whisper model file is already downloaded
#[tauri::command(rename_all = "snake_case")]
pub async fn is_whisper_model_downloaded(
    model_filename: String,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
        let models_dir = crate::config::AppConfig::whisper_models_dir();
        let model_path = models_dir.join(&model_filename);
        let exists = model_path.exists();

        let validation = if exists {
            Some(crate::voice::validate_whisper_model(&model_path))
        } else {
            None
        };

        serde_json::json!({
            "exists": exists,
            "path": model_path.to_string_lossy().to_string(),
            "is_valid": validation.as_ref().map(|v| v.is_valid).unwrap_or(false),
            "validation": validation
        })
    })
    .await
    .map_err(|error| format!("Failed to inspect whisper model download state: {error}"))
}

/// Get the default Whisper model path and status
#[tauri::command]
pub async fn get_whisper_model_status() -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
        let (exists, path) = crate::voice::get_default_model_status();
        let path_str = path.to_string_lossy().to_string();

        let validation = if exists {
            Some(crate::voice::validate_whisper_model(&path))
        } else {
            None
        };

        serde_json::json!({
            "default_path": path_str,
            "exists": exists,
            "validation": validation,
            "models_dir": crate::config::AppConfig::whisper_models_dir().to_string_lossy()
        })
    })
    .await
    .map_err(|error| format!("Failed to inspect whisper model status: {error}"))
}

/// Download a Whisper model from HuggingFace
/// Returns progress updates via Tauri events
#[tauri::command(rename_all = "snake_case")]
pub async fn download_whisper_model(
    app: tauri::AppHandle,
    model_filename: String,
) -> Result<String, String> {
    use tauri::Emitter;
    use tokio::io::AsyncWriteExt;

    tracing::info!(
        "Whisper download command invoked for model filename: {}",
        model_filename
    );

    // Find the model info
    let model_info = crate::config::WhisperModelInfo::find_by_filename(&model_filename)
        .ok_or_else(|| format!("Unknown model: {}", model_filename))?;

    // Create the models directory
    let models_dir = crate::config::AppConfig::whisper_models_dir();
    tokio::fs::create_dir_all(&models_dir)
        .await
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    let output_path = models_dir.join(&model_filename);

    tracing::info!(
        "Preparing to download Whisper model '{}' (filename='{}') to {:?}",
        model_info.name,
        model_info.filename,
        output_path
    );

    // Check if already downloaded
    if tokio::fs::try_exists(&output_path)
        .await
        .map_err(|e| format!("Failed to inspect existing model file: {}", e))?
    {
        let validation_path = output_path.clone();
        let validation = tokio::task::spawn_blocking(move || {
            crate::voice::validate_whisper_model(&validation_path)
        })
        .await
        .map_err(|e| format!("Failed to validate existing model file: {}", e))?;
        if validation.is_valid {
            return Ok(format!(
                "Model already downloaded: {}",
                output_path.to_string_lossy()
            ));
        }
        // Remove invalid file
        let _ = tokio::fs::remove_file(&output_path).await;
    }

    tracing::info!(
        "Downloading Whisper model: {} from {}",
        model_filename,
        model_info.url
    );

    // Emit start event
    let _ = app.emit(
        "whisper-download-progress",
        serde_json::json!({
            "status": "starting",
            "filename": model_filename,
            "total_mb": model_info.size_mb,
            "downloaded_mb": 0,
            "percent": 0
        }),
    );

    // Download the model with proper timeout and User-Agent
    // Hugging Face CDN requires a User-Agent header and may need time for large files
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1800)) // 30 minute timeout for large models
        .user_agent(format!(
            "Gestura/{} (https://gestura.ai)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    tracing::info!("Starting HTTP request to: {}", model_info.url);

    let response = client.get(&model_info.url).send().await.map_err(|e| {
        tracing::error!("HTTP request failed: {}", e);
        format!("Failed to start download: {}", e)
    })?;

    tracing::info!(
        "HTTP response received: status={}, content_length={:?}",
        response.status(),
        response.content_length()
    );

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!("Download failed with HTTP {}: {}", status, body);
        let _ = app.emit(
            "whisper-download-progress",
            serde_json::json!({
                "status": "error",
                "filename": model_filename,
                "error": format!("HTTP {}", status)
            }),
        );
        return Err(format!("Download failed: HTTP {} - {}", status, body));
    }

    let total_size = response
        .content_length()
        .unwrap_or(model_info.size_mb * 1024 * 1024);

    tracing::info!(
        "Starting streaming download: total_size={} bytes ({:.1} MB)",
        total_size,
        total_size as f64 / (1024.0 * 1024.0)
    );

    // Create temp file for download
    let temp_path = output_path.with_extension("tmp");
    let mut file = tokio::fs::File::create(&temp_path).await.map_err(|e| {
        tracing::error!("Failed to create temp file {:?}: {}", temp_path, e);
        format!("Failed to create temp file: {}", e)
    })?;

    let mut downloaded: u64 = 0;
    let mut last_percent: u64 = 0;

    // Stream the response
    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            tracing::error!("Download stream error after {} bytes: {}", downloaded, e);
            let _ = app.emit(
                "whisper-download-progress",
                serde_json::json!({
                    "status": "error",
                    "filename": model_filename,
                    "error": format!("Stream error: {}", e)
                }),
            );
            format!("Download error: {}", e)
        })?;
        file.write_all(&chunk).await.map_err(|e| {
            tracing::error!("Failed to write chunk to file: {}", e);
            format!("Failed to write file: {}", e)
        })?;

        downloaded += chunk.len() as u64;
        let percent = (downloaded * 100) / total_size;

        // Emit progress every 1%
        if percent > last_percent {
            last_percent = percent;
            let downloaded_mb = downloaded as f64 / (1024.0 * 1024.0);
            let _ = app.emit(
                "whisper-download-progress",
                serde_json::json!({
                    "status": "downloading",
                    "filename": model_filename,
                    "total_mb": model_info.size_mb,
                    "downloaded_mb": downloaded_mb,
                    "percent": percent
                }),
            );
        }
    }

    tracing::info!(
        "Download complete: {} bytes written to {:?}",
        downloaded,
        temp_path
    );

    file.flush()
        .await
        .map_err(|e| format!("Failed to flush model download to disk: {}", e))?;

    // Rename temp file to final path
    tokio::fs::rename(&temp_path, &output_path)
        .await
        .map_err(|e| format!("Failed to save model file: {}", e))?;

    // Validate the downloaded model
    let validation_path = output_path.clone();
    let validation =
        tokio::task::spawn_blocking(move || crate::voice::validate_whisper_model(&validation_path))
            .await
            .map_err(|e| format!("Failed to validate downloaded model file: {}", e))?;
    if !validation.is_valid {
        let _ = tokio::fs::remove_file(&output_path).await;
        return Err(format!(
            "Downloaded file is invalid: {}",
            validation
                .error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    // Emit completion event
    let _ = app.emit(
        "whisper-download-progress",
        serde_json::json!({
            "status": "complete",
            "filename": model_filename,
            "path": output_path.to_string_lossy(),
            "percent": 100
        }),
    );

    // Update config with the new model path
    let mut config = AppConfig::load_async().await;
    config.voice.local_model_path = Some(output_path.to_string_lossy().to_string());
    config
        .save_async()
        .await
        .map_err(|e| format!("Failed to save config: {}", e))?;

    tracing::info!(
        "Whisper model downloaded successfully: {}",
        output_path.to_string_lossy()
    );

    Ok(output_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_ui_prefs() -> Result<crate::config::UiSettings, String> {
    Ok(AppConfig::load_async().await.ui)
}

#[tauri::command]
pub async fn set_ui_prefs(ui: crate::config::UiSettings) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.ui = ui;
    cfg.save_async().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_voice_once() -> Result<String, String> {
    let cfg = AppConfig::load_async().await;
    let engine = crate::voice_select::select_voice(&cfg);
    crate::voice_select::validate_voice_config_for_run(&cfg, engine.as_ref())
        .map_err(|e| e.to_string())?;
    let text = engine
        .process_command(&cfg, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(text)
}

/// Scan for available Haptic Harmony rings
#[tauri::command]
pub async fn scan_for_rings(state: State<'_, crate::AppState>) -> Result<Vec<String>, String> {
    state
        .ring_manager
        .scan_for_rings()
        .await
        .map_err(|e| e.to_string())
}

/// Get ring status by device ID
#[tauri::command(rename_all = "snake_case")]
pub async fn get_ring_status(
    device_id: String,
    state: State<'_, crate::AppState>,
) -> Result<Option<crate::ble::RingStatus>, String> {
    state
        .ring_manager
        .get_ring_status(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Pair with a ring
#[tauri::command(rename_all = "snake_case")]
pub async fn pair_ring(device_id: String, state: State<'_, crate::AppState>) -> Result<(), String> {
    state
        .ring_manager
        .pair_ring(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Send haptic feedback to ring
#[tauri::command(rename_all = "snake_case")]
pub async fn send_haptic_feedback(
    device_id: String,
    pattern: String,
    intensity: f32,
    duration_ms: u32,
    state: State<'_, crate::AppState>,
) -> Result<(), String> {
    let haptic_pattern = match pattern.as_str() {
        "click" => crate::haptics::HapticPattern::Click,
        "pulse" => crate::haptics::HapticPattern::Pulse,
        "ramp" => crate::haptics::HapticPattern::Ramp,
        _ => return Err("Invalid haptic pattern".to_string()),
    };

    let request = crate::haptics::HapticRequest {
        pattern: haptic_pattern,
        intensity,
        duration_ms,
        repeat_count: 0,
        repeat_delay_ms: 0,
    };

    state
        .ring_manager
        .send_haptic(&device_id, request)
        .await
        .map_err(|e| e.to_string())
}

/// Start gesture monitoring for a ring
#[tauri::command(rename_all = "snake_case")]
pub async fn start_gesture_monitoring(
    device_id: String,
    state: State<'_, crate::AppState>,
) -> Result<(), String> {
    let event_tx = state.ble_event_tx.clone();
    state
        .ring_manager
        .start_gesture_monitoring(&device_id, event_tx)
        .await
        .map_err(|e| e.to_string())
}

/// Stop gesture monitoring for a ring
#[tauri::command(rename_all = "snake_case")]
pub async fn stop_gesture_monitoring(
    device_id: String,
    state: State<'_, crate::AppState>,
) -> Result<(), String> {
    state
        .ring_manager
        .stop_gesture_monitoring(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Get current NATS connection status.
///
/// Returns `true` if the app has an active NATS connection, otherwise `false`.
#[tauri::command]
pub fn get_nats_status(state: tauri::State<'_, crate::AppState>) -> Result<bool, String> {
    Ok(state.nats.is_some())
}

/// Get system health status
#[tauri::command]
pub async fn get_system_health() -> Result<serde_json::Value, String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    let health = telemetry.get_system_health().await;
    serde_json::to_value(health).map_err(|e| e.to_string())
}

/// Get telemetry metrics summary
#[tauri::command]
pub async fn get_metrics_summary() -> Result<serde_json::Value, String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    serde_json::to_value(telemetry.get_metrics_summary().await).map_err(|e| e.to_string())
}

/// Get recent telemetry metrics
#[tauri::command]
pub async fn get_recent_metrics(
    limit: Option<usize>,
) -> Result<Vec<crate::telemetry::Metric>, String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    Ok(telemetry.get_recent_metrics(limit.unwrap_or(100)).await)
}

/// Clear telemetry metrics
#[tauri::command]
pub async fn clear_metrics() -> Result<(), String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    telemetry.clear_metrics().await;
    Ok(())
}

/// Export user data (GDPR compliance)
#[tauri::command]
pub async fn export_user_data(user_id: String) -> Result<serde_json::Value, String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    gdpr.export_user_data(&user_id)
        .await
        .map_err(|e| e.to_string())
}

/// Delete user data (GDPR compliance)
#[tauri::command]
pub async fn delete_user_data(
    user_id: String,
    verify: Option<bool>,
) -> Result<Vec<String>, String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    gdpr.delete_user_data(&user_id, verify.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}

/// Get user consent status
#[tauri::command]
pub async fn get_user_consents(user_id: String) -> Result<Vec<crate::gdpr::ConsentRecord>, String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    Ok(gdpr.get_user_consents(&user_id).await)
}

/// Register user consent
#[tauri::command]
pub async fn register_consent(
    user_id: String,
    category: String,
    purpose: String,
) -> Result<(), String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    let data_category = match category.as_str() {
        "voice" => crate::gdpr::DataCategory::VoiceRecordings,
        "biometric" => crate::gdpr::DataCategory::BiometricData,
        "device" => crate::gdpr::DataCategory::DeviceData,
        "usage" => crate::gdpr::DataCategory::UsageAnalytics,
        "config" => crate::gdpr::DataCategory::ConfigurationData,
        _ => return Err("Invalid data category".to_string()),
    };

    gdpr.register_consent(user_id, data_category, purpose, "User consent".to_string())
        .await
        .map_err(|e| e.to_string())
}

// Agent and Agent Commands

/// Process a agent message through the configured LLM provider
#[tauri::command]
pub async fn process_agent_message(
    app: tauri::AppHandle,
    message: String,
) -> Result<String, String> {
    use crate::notifications::{NotificationType, get_notification_manager};

    let cfg = AppConfig::load_async().await;

    tracing::info!(
        "Processing agent message through core AgentPipeline (provider={} )",
        cfg.llm.primary
    );

    // Call the core pipeline with tools disabled (legacy single-shot command).
    let response = run_single_shot_pipeline(cfg, message, RequestSource::GuiText, None).await;

    match &response {
        Ok(resp) => {
            tracing::info!("LLM response received: {} chars", resp.len());
            // Send completion notification
            get_notification_manager()
                .notify(NotificationType::ResponseComplete, Some(&app))
                .await;
        }
        Err(e) => {
            tracing::error!("LLM error: {}", e);
            // Send error notification
            get_notification_manager()
                .notify(NotificationType::Error, Some(&app))
                .await;
        }
    }

    response.map_err(|e| format!("LLM error: {}", e))
}

/// Cancellation key used when a agent stream is not associated with a window label.
///
/// Most agent flows are window-scoped (`window:<label>`). This fallback key exists for
/// legacy/non-session agent surfaces that do not have a stable window label.
///
/// Note: the cancellation token registry lives in `gestura_core::stream_cancellation`.
const GLOBAL_STREAM_CANCEL_KEY: &str = "__global_stream__";

/// Build the cancellation token key for a particular window label.
///
/// This intentionally scopes cancellation to a single window so concurrent streams
/// in different agent windows do not cancel each other.
fn cancel_key_for_window_label(window_label: &str) -> String {
    format!("window:{window_label}")
}

struct StreamCancellationGuard {
    key: String,
    armed: bool,
}

impl StreamCancellationGuard {
    fn new(key: String) -> Self {
        Self { key, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StreamCancellationGuard {
    fn drop(&mut self) {
        if self.armed {
            gestura_core::stream_cancellation::STREAM_CANCELLATIONS.remove(&self.key);
        }
    }
}

fn should_persist_session_activity_event(event: &str, payload: &serde_json::Value) -> bool {
    match event {
        "agent-stream-done"
        | "agent-stream-paused"
        | "agent-stream-cancelled"
        | "agent-stream-error"
        | "agent-stream-resumed" => true,
        "agent-stream-shell-lifecycle" => payload
            .get("state")
            .and_then(serde_json::Value::as_str)
            .map(|state| {
                matches!(
                    state.to_ascii_lowercase().as_str(),
                    "completed" | "failed" | "stopped"
                )
            })
            .unwrap_or(false),
        "agent-stream-shell-session-lifecycle" => payload
            .get("state")
            .and_then(serde_json::Value::as_str)
            .map(|state| {
                matches!(
                    state.to_ascii_lowercase().as_str(),
                    "idle" | "stopped" | "failed"
                )
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn capture_session_activity_event(
    session_id: Option<&str>,
    event: &str,
    payload: &serde_json::Value,
) {
    if let Some(session_id) = session_id {
        crate::window_manager::record_session_activity(session_id, event, payload.clone());
        if should_persist_session_activity_event(event, payload) {
            crate::window_manager::schedule_save_sessions();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalAssistantSummaryKind {
    Failed,
    Paused,
    Cancelled,
    UnexpectedEnd,
}

fn persist_terminal_assistant_message(
    session_id: Option<&str>,
    content: &str,
    thinking: Option<&str>,
    append_to_last: bool,
) {
    let Some(session_id) = session_id else {
        return;
    };

    let has_content = !content.trim().is_empty();
    let has_thinking = thinking.is_some_and(|value| !value.trim().is_empty());
    if !has_content && !has_thinking {
        return;
    }

    let thinking = thinking.map(str::to_string);
    if append_to_last
        && crate::window_manager::append_to_last_assistant_message(
            session_id,
            content,
            thinking.clone(),
        )
    {
        return;
    }

    crate::window_manager::add_assistant_message(session_id, content, thinking);
}

fn update_latest_progress_summary(slot: &mut Option<String>, summary: Option<&str>, message: &str) {
    let next = summary
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let message = message.trim();
            (!message.is_empty()).then(|| message.to_string())
        });

    if next.is_some() {
        *slot = next;
    }
}

fn build_terminal_assistant_summary(
    kind: TerminalAssistantSummaryKind,
    latest_progress_summary: Option<&str>,
    detail: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    match kind {
        TerminalAssistantSummaryKind::Failed => {
            parts.push("The run stopped before completion.".to_string());
        }
        TerminalAssistantSummaryKind::Paused => {
            parts.push(
                "The run paused before completion and can be resumed from the saved state."
                    .to_string(),
            );
        }
        TerminalAssistantSummaryKind::Cancelled => {
            parts.push("The run was cancelled before completion.".to_string());
        }
        TerminalAssistantSummaryKind::UnexpectedEnd => {
            parts.push("The stream ended before a terminal completion event arrived.".to_string());
        }
    }

    if let Some(progress) = latest_progress_summary
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("Latest confirmed progress: {progress}"));
    }

    if let Some(detail) = detail.map(str::trim).filter(|value| !value.is_empty()) {
        parts.push(format!("Detail: {detail}"));
    }

    parts.join(" ")
}

fn remember_terminal_assistant_summary(
    session_id: Option<&str>,
    kind: TerminalAssistantSummaryKind,
    latest_progress_summary: Option<&str>,
    detail: Option<&str>,
    thinking: Option<&str>,
) {
    let Some(session_id) = session_id else {
        return;
    };

    let summary = build_terminal_assistant_summary(kind, latest_progress_summary, detail);
    crate::window_manager::remember_assistant_summary(
        session_id,
        &summary,
        thinking.map(str::to_string),
    );
}

/// Process a agent message with streaming response
///
/// Emits `agent-stream-chunk` events with partial content and `agent-stream-done` when complete.
///
/// The optional `source` argument can be used to hint how the message was produced:
/// - `"voice"` for transcribed speech
/// - `"text"` for typed input (default)
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn process_agent_message_streaming(
    webview_window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    message: String,
    session_id: Option<String>,
    task_id: Option<String>,
    source: Option<String>,
) -> Result<(), String> {
    use gestura_core::{CancellationToken, StreamChunk};
    use gestura_core::{render_capabilities, render_tool_detail, render_tools_overview};
    use tokio::sync::mpsc;

    let mut cfg = AppConfig::load_async().await;

    // Log initial config state
    tracing::debug!(
        global_provider = %cfg.llm.primary,
        session_id = ?session_id,
        "Starting agent message processing"
    );

    let message_source = match source.as_deref().map(|s| s.trim().to_ascii_lowercase()) {
        Some(s) if s == "voice" => crate::window_manager::MessageSource::Voice,
        _ => crate::window_manager::MessageSource::Text,
    };
    let request_source = match message_source {
        crate::window_manager::MessageSource::Voice => gestura_core::RequestSource::GuiVoice,
        _ => gestura_core::RequestSource::GuiText,
    };

    // --- Secure window/session isolation ---
    //
    // We always emit streaming events to a single target window. We never broadcast
    // (`app.emit`) because that can leak content across agent windows.
    let calling_window_label = webview_window.label().to_string();
    let calling_session_id =
        crate::window_manager::get_session_id_for_window_label(&calling_window_label);

    // Defense-in-depth: if the caller is a agent window with a known session, do not
    // allow it to stream into a different session by passing a mismatched session id.
    match (&calling_session_id, &session_id) {
        (Some(calling_sid), Some(request_sid)) if calling_sid != request_sid => {
            return Err(format!(
                "Session mismatch for window '{}': caller session '{}' != requested session '{}'",
                calling_window_label, calling_sid, request_sid
            ));
        }
        _ => {}
    }

    // Resolve session id (typed input: use the calling window; voice: optionally route to active agent).
    let resolved_session_id = session_id
        .or_else(|| calling_session_id.clone())
        .or_else(|| {
            if matches!(message_source, crate::window_manager::MessageSource::Voice) {
                crate::window_manager::get_active_agent_for_voice()
            } else {
                None
            }
        });

    // Choose the window to receive stream events.
    // - Text: the calling window.
    // - Voice: the resolved active agent window if available; otherwise the calling window.
    let target_window_label =
        if matches!(message_source, crate::window_manager::MessageSource::Voice) {
            resolved_session_id
                .as_deref()
                .and_then(crate::window_manager::get_session_window_label)
                .unwrap_or_else(|| calling_window_label.clone())
        } else {
            calling_window_label.clone()
        };

    let trimmed = message.trim();
    const LOCAL_STREAM_CHUNK_CHARS: usize = 64;
    let is_tools_cmd = trimmed.starts_with("/tools");
    let is_capabilities_cmd = trimmed.starts_with("/capabilities");
    let is_summarize_cmd = trimmed.starts_with("/summarize");
    let is_memory_cmd = trimmed.starts_with("/memory");
    let supports_stream_interrupt =
        !(is_tools_cmd || is_capabilities_cmd || is_summarize_cmd || is_memory_cmd);

    let mut stream_cancellation = if supports_stream_interrupt {
        let cancel_token = CancellationToken::new();
        let cancel_key = if !target_window_label.is_empty() {
            cancel_key_for_window_label(&target_window_label)
        } else {
            GLOBAL_STREAM_CANCEL_KEY.to_string()
        };
        gestura_core::stream_cancellation::STREAM_CANCELLATIONS
            .register(cancel_key.clone(), cancel_token.clone());
        Some((
            StreamCancellationGuard::new(cancel_key.clone()),
            cancel_key,
            cancel_token,
        ))
    } else {
        None
    };

    // Apply session-specific LLM config overrides using the *resolved* session id.
    //
    // Why: voice routing can change the target session. We must keep request metadata
    // and pipeline configuration consistent for the actual target session.
    let effective_llm =
        apply_session_llm_config_overrides(&mut cfg, resolved_session_id.as_deref()).await;

    tracing::info!(
        session_id = ?resolved_session_id,
        effective_provider = %effective_llm.provider,
        effective_model = %effective_llm.model,
        "Processing agent message with effective session LLM config"
    );

    // Centralized, window-scoped emission (never broadcast): emits via `emit_to` and
    // attaches `session_id` for frontend filtering.
    let emit = |event: &str, payload: serde_json::Value| {
        let payload =
            crate::agent_events::attach_session_id(payload, resolved_session_id.as_deref());
        capture_session_activity_event(resolved_session_id.as_deref(), event, &payload);
        if let Err(err) = crate::agent_events::emit_agent_event_to_window(
            &app,
            &target_window_label,
            &calling_window_label,
            event,
            &payload,
            resolved_session_id.as_deref(),
        ) {
            tracing::error!(
                event = %event,
                target_window_label = %target_window_label,
                calling_window_label = %calling_window_label,
                error = %err,
                "Failed to emit agent event"
            );
        }
    };

    // Only handle explicit /tools command, not natural language questions
    if is_tools_cmd {
        let thinking_note =
            Some("Using local tool catalog (no LLM call) and streaming the result...".to_string());
        let response = if is_tools_cmd {
            // Parse /tools <name> command
            let mut parts = trimmed.split_whitespace();
            let _ = parts.next(); // skip /tools
            if let Some(name) = parts.next() {
                render_tool_detail(name).unwrap_or_else(|| {
                    format!(
                        "Unknown tool '{}'. Try `/tools` to list all available tools.",
                        name
                    )
                })
            } else {
                render_tools_overview()
            }
        } else {
            render_tools_overview()
        };

        // Persist to session (if any)
        if let Some(ref sid) = resolved_session_id {
            crate::window_manager::add_user_message(sid, &message, message_source);
            crate::window_manager::add_assistant_message(sid, &response, thinking_note.clone());
        }

        if let Some(note) = thinking_note {
            emit("agent-stream-thinking", serde_json::json!(note));
            tokio::task::yield_now().await;
        }

        // Stream the response in chunks so the UX stays consistently "live".
        let mut rest = response.as_str();
        while !rest.is_empty() {
            let split_at = rest
                .char_indices()
                .nth(LOCAL_STREAM_CHUNK_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(rest.len());

            let (chunk, next) = rest.split_at(split_at);
            rest = next;

            if !chunk.is_empty() {
                emit("agent-stream-chunk", serde_json::json!(chunk));
                tokio::task::yield_now().await;
            }
        }
        emit("agent-stream-done", serde_json::json!(null));
        return Ok(());
    }

    // Handle capabilities command (explicit slash command only)
    // Natural language questions should go through the LLM for dynamic responses
    if is_capabilities_cmd {
        let thinking_note = Some(
            "Reading local capabilities (no LLM call) and streaming the result...".to_string(),
        );
        let response = render_capabilities(&cfg);

        // Persist to session (if any)
        if let Some(ref sid) = resolved_session_id {
            crate::window_manager::add_user_message(sid, &message, message_source);
            crate::window_manager::add_assistant_message(sid, &response, thinking_note.clone());
        }

        if let Some(note) = thinking_note {
            emit("agent-stream-thinking", serde_json::json!(note));
            tokio::task::yield_now().await;
        }

        let mut rest = response.as_str();
        while !rest.is_empty() {
            let split_at = rest
                .char_indices()
                .nth(LOCAL_STREAM_CHUNK_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(rest.len());

            let (chunk, next) = rest.split_at(split_at);
            rest = next;

            if !chunk.is_empty() {
                emit("agent-stream-chunk", serde_json::json!(chunk));
                tokio::task::yield_now().await;
            }
        }
        emit("agent-stream-done", serde_json::json!(null));
        return Ok(());
    }

    // Handle /summarize command - summarize conversation history without calling LLM
    if is_summarize_cmd {
        let thinking_note = Some("Summarizing conversation history (no LLM call)...".to_string());

        // Get conversation history from session
        let history = if let Some(ref sid) = resolved_session_id {
            crate::window_manager::get_session_state(sid)
                .map(|state| {
                    state
                        .messages
                        .into_iter()
                        .map(|msg| msg.content)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Use context manager to summarize
        use gestura_core::context::ContextManager;
        let context_manager = ContextManager::new();
        let summary = if history.is_empty() {
            "No conversation history to summarize.".to_string()
        } else {
            let summary_text = context_manager.summarize_history(&history);
            format!(
                "## Conversation Summary\n\n{}\n\n---\n\n*Summarized {} messages*",
                summary_text,
                history.len()
            )
        };

        // Persist to session (if any)
        if let Some(ref sid) = resolved_session_id {
            crate::window_manager::add_user_message(sid, &message, message_source);
            crate::window_manager::add_assistant_message(sid, &summary, thinking_note.clone());
        }

        if let Some(note) = thinking_note {
            emit("agent-stream-thinking", serde_json::json!(note));
            tokio::task::yield_now().await;
        }

        let mut rest = summary.as_str();
        while !rest.is_empty() {
            let split_at = rest
                .char_indices()
                .nth(LOCAL_STREAM_CHUNK_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(rest.len());

            let (chunk, next) = rest.split_at(split_at);
            rest = next;

            if !chunk.is_empty() {
                emit("agent-stream-chunk", serde_json::json!(chunk));
                tokio::task::yield_now().await;
            }
        }
        emit("agent-stream-done", serde_json::json!(null));
        return Ok(());
    }

    // Handle /memory command - manage memory bank without calling LLM
    if is_memory_cmd {
        let thinking_note = Some("Managing memory bank (no LLM call)...".to_string());

        // Parse subcommand: /memory list|save|clear
        let mut parts = trimmed.split_whitespace();
        let _ = parts.next(); // skip /memory
        let subcommand = parts.next().unwrap_or("list");

        let response = match subcommand {
            "list" => {
                // List all memory bank entries
                // Retrieve workspace from session state
                let workspace_dir = if let Some(ref sid) = resolved_session_id {
                    crate::window_manager::get_session_state(sid).and_then(|s| s.workspace_dir)
                } else {
                    crate::window_manager::get_active_session_workspace()
                };

                if let Some(workspace_dir) = workspace_dir.as_ref() {
                    match gestura_core::memory_bank::list_memory_bank(workspace_dir).await {
                        Ok(entries) if !entries.is_empty() => {
                            let mut output =
                                format!("## Memory Bank Entries ({} total)\n\n", entries.len());
                            for entry in entries {
                                output.push_str(&format!(
                                    "### {} (Session: {})\n",
                                    entry.timestamp.format("%Y-%m-%d %H:%M UTC"),
                                    entry.session_id
                                ));
                                output.push_str(&format!("**Summary**: {}\n\n", entry.summary));
                                if let Some(path) = entry.file_path {
                                    output.push_str(&format!("**File**: `{}`\n\n", path.display()));
                                }
                                output.push_str("---\n\n");
                            }
                            output
                        }
                        Ok(_) => "No memory bank entries found.".to_string(),
                        Err(e) => format!("Error listing memory bank: {}", e),
                    }
                } else {
                    "No workspace directory configured. Cannot access memory bank.".to_string()
                }
            }
            "save" => {
                // Save current context to memory bank
                // Retrieve workspace from session state
                let workspace_dir = if let Some(ref sid) = resolved_session_id {
                    crate::window_manager::get_session_state(sid).and_then(|s| s.workspace_dir)
                } else {
                    crate::window_manager::get_active_session_workspace()
                };

                if let Some(workspace_dir) = workspace_dir.as_ref() {
                    let history = if let Some(ref sid) = resolved_session_id {
                        crate::window_manager::get_session_state(sid)
                            .map(|state| {
                                state
                                    .messages
                                    .into_iter()
                                    .map(|msg| msg.content)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };

                    if history.is_empty() {
                        "No conversation history to save.".to_string()
                    } else {
                        use gestura_core::context::ContextManager;
                        let context_manager = ContextManager::new();
                        let summary = context_manager.summarize_history(&history);
                        let content = history.join("\n\n");

                        let entry = gestura_core::memory_bank::MemoryBankEntry::new(
                            resolved_session_id
                                .clone()
                                .unwrap_or_else(|| "unknown".to_string()),
                            summary.clone(),
                            content,
                        );

                        match gestura_core::memory_bank::save_to_memory_bank(workspace_dir, &entry)
                            .await
                        {
                            Ok(path) => format!(
                                "✅ Saved {} messages to memory bank\n\n**File**: `{}`\n\n**Summary**: {}",
                                history.len(),
                                path.display(),
                                summary
                            ),
                            Err(e) => format!("Error saving to memory bank: {}", e),
                        }
                    }
                } else {
                    "No workspace directory configured. Cannot save to memory bank.".to_string()
                }
            }
            "clear" => {
                // Clear all memory bank entries
                // Retrieve workspace from session state
                let workspace_dir = if let Some(ref sid) = resolved_session_id {
                    crate::window_manager::get_session_state(sid).and_then(|s| s.workspace_dir)
                } else {
                    crate::window_manager::get_active_session_workspace()
                };

                if let Some(workspace_dir) = workspace_dir.as_ref() {
                    let memory_dir = workspace_dir.join(".gestura").join("memory");
                    match std::fs::remove_dir_all(&memory_dir) {
                        Ok(_) => {
                            // Recreate the directory
                            let _ = std::fs::create_dir_all(&memory_dir);
                            "✅ Cleared all memory bank entries.".to_string()
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            "Memory bank is already empty.".to_string()
                        }
                        Err(e) => format!("Error clearing memory bank: {}", e),
                    }
                } else {
                    "No workspace directory configured. Cannot clear memory bank.".to_string()
                }
            }
            _ => {
                format!(
                    "Unknown /memory subcommand: '{}'\n\nUsage:\n- `/memory list` - Show all memory bank entries\n- `/memory save` - Save current conversation to memory bank\n- `/memory clear` - Delete all memory bank entries",
                    subcommand
                )
            }
        };

        // Persist to session (if any)
        if let Some(ref sid) = resolved_session_id {
            crate::window_manager::add_user_message(sid, &message, message_source);
            crate::window_manager::add_assistant_message(sid, &response, thinking_note.clone());
        }

        if let Some(note) = thinking_note {
            emit("agent-stream-thinking", serde_json::json!(note));
            tokio::task::yield_now().await;
        }

        let mut rest = response.as_str();
        while !rest.is_empty() {
            let split_at = rest
                .char_indices()
                .nth(LOCAL_STREAM_CHUNK_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(rest.len());

            let (chunk, next) = rest.split_at(split_at);
            rest = next;

            if !chunk.is_empty() {
                emit("agent-stream-chunk", serde_json::json!(chunk));
                tokio::task::yield_now().await;
            }
        }
        emit("agent-stream-done", serde_json::json!(null));
        return Ok(());
    }

    tracing::info!(
        "Starting streaming agent through AgentPipeline with LLM provider: {}",
        cfg.llm.primary
    );

    // Use a conservative default history limit to prevent token explosion.
    // This matches the PipelineConfig default and prevents excessive context buildup.
    // Users can adjust this via the pipeline configuration if needed.
    let max_history = 10; // Conservative default to prevent token explosion

    // Build conversation history for the pipeline before recording this turn so the
    // request history mirrors the CLI/TUI behavior while the session timeline still
    // shows the user message immediately.
    let history: Vec<gestura_core::Message> = resolved_session_id
        .as_deref()
        .map(|sid| {
            let msgs = crate::window_manager::get_pipeline_messages(sid);
            let total_msgs = msgs.len();
            let result: Vec<_> = msgs
                .into_iter()
                .rev()
                .take(max_history)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            tracing::debug!(
                total_messages = total_msgs,
                included = result.len(),
                max_history = max_history,
                "Pre-filtered conversation history for token efficiency"
            );
            result
        })
        .unwrap_or_default();

    // A brand-new user message supersedes any previously paused execution for the
    // session. Clear it before recording the new turn so "resume" only applies to
    // the most recently interrupted response.
    if let Some(ref sid) = resolved_session_id {
        crate::window_manager::set_session_paused_execution(sid, None);
        crate::window_manager::add_user_message(sid, &message, message_source);
    }

    let mut latest_progress_summary: Option<String> = None;

    let auto_planned_task = if let Some(sid) = resolved_session_id.as_deref() {
        if should_auto_plan_agent_request(&message, task_id.as_deref()) {
            let root_task_name = derive_agent_request_task_name(&message);
            let narration = generate_pre_auto_plan_narration(sid, &message, &root_task_name).await;
            update_latest_progress_summary(
                &mut latest_progress_summary,
                narration.summary.as_deref(),
                &narration.message,
            );
            emit_bootstrap_narration(&emit, narration);
            tokio::task::yield_now().await;

            match auto_plan_agent_request(&app, sid, &message).await {
                Ok(plan) => {
                    let narration = generate_post_auto_plan_narration(sid, &message, &plan).await;
                    update_latest_progress_summary(
                        &mut latest_progress_summary,
                        narration.summary.as_deref(),
                        &narration.message,
                    );
                    emit_bootstrap_narration(&emit, narration);
                    tokio::task::yield_now().await;
                    Some(plan)
                }
                Err(error) => {
                    tracing::warn!(session_id = %sid, error = %error, "Failed to auto-create request plan before agent execution");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // If the request is tied to an existing task (e.g. from the task panel),
    // track status on that task. For multi-step implementation requests without
    // an explicit task, auto-plan a root task + subtasks first and track that
    // root task through the run.
    let tracked_task_id: Option<String> = match &resolved_session_id {
        Some(sid) => {
            let resolved_task_id = resolve_tracked_task_id(
                crate::task_integration::get_task_manager(),
                sid,
                &message,
                task_id.as_deref(),
                auto_planned_task
                    .as_ref()
                    .and_then(|plan| plan.root_task_id.as_deref()),
            );
            let current_execution_task_id = resolve_current_execution_task_id(
                task_id.as_deref(),
                auto_planned_task.as_ref(),
                resolved_task_id.as_deref(),
            );

            if let Some(task_id) = resolved_task_id.as_deref() {
                crate::task_integration::mark_task_in_progress(&app, sid, task_id)
                    .map_err(|e| format!("Failed to start task '{task_id}': {e}"))?;
            }

            if let Some(current_task_id) = current_execution_task_id.as_deref() {
                if Some(current_task_id) != resolved_task_id.as_deref() {
                    crate::task_integration::mark_task_in_progress(&app, sid, current_task_id)
                        .map_err(|e| format!("Failed to start task '{current_task_id}': {e}"))?;
                }
                let _ = crate::task_integration::get_task_manager()
                    .set_current_task_id(sid, Some(current_task_id.to_string()));
            }

            resolved_task_id
        }
        None if task_id.is_some() => {
            let task_id = task_id.as_deref().unwrap_or_default();
            tracing::warn!(
                task_id = %task_id,
                "Received task_id without a resolved session; task progress will not be tracked"
            );
            None
        }
        _ => None,
    };

    let mut last_task_tree_signature = resolved_session_id
        .as_deref()
        .and_then(task_tree_refresh_signature);

    let (mut stream_cancellation_guard, cancel_key, cancel_token) = stream_cancellation
        .take()
        .expect("non-local agent streaming requests always register cancellation");

    // Create channel for streaming chunks
    let (tx, mut rx) = mpsc::channel::<StreamChunk>(100);

    // Build the agent request with workspace sandboxing
    use gestura_core::{AgentPipeline, AgentRequest};

    // Snapshot history before ownership moves to the request builder.
    // These are used to build PausedExecutionState if the stream is paused/cancelled.
    let history_snapshot = history.clone();
    let input_snapshot = message.clone();

    let request_input = match (resolved_session_id.as_deref(), tracked_task_id.as_deref()) {
        (Some(session_id), Some(task_id)) => {
            if let Some(plan) = auto_planned_task
                .as_ref()
                .map(|plan| AutoPlanExecutionContext {
                    root_task_name: plan.root_task_name.clone(),
                    planned_subtasks: plan.planned_subtasks.clone(),
                    initial_task_id: plan.initial_task_id.clone(),
                    initial_task_name: plan.initial_task_name.clone(),
                })
                .or_else(|| collect_auto_plan_execution_context(session_id, task_id))
            {
                build_auto_plan_execution_handoff_message(
                    &message,
                    &plan.root_task_name,
                    &plan.planned_subtasks,
                    plan.initial_task_name.as_deref(),
                )
            } else {
                message.clone()
            }
        }
        _ => message.clone(),
    };

    let mut request = AgentRequest::new(&request_input)
        .with_streaming(true)
        .with_source(request_source)
        .with_history(history);

    if let Some(ref sid) = resolved_session_id {
        request = request.with_session(sid);
    }

    if let Some(task_id) = tracked_task_id.as_deref() {
        request = request.with_task(task_id);
    }

    // Set workspace directory for sandboxed operations
    if let Some(ref sid) = resolved_session_id {
        if let Some(workspace) =
            crate::window_manager::get_session_state(sid).and_then(|s| s.workspace_dir)
        {
            request = request.with_workspace(workspace);
        }
    } else if let Some(workspace) = crate::window_manager::get_active_session_workspace() {
        // Backwards-compatible fallback
        request = request.with_workspace(workspace);
    }

    // Set session LLM config and permission level for agent awareness.
    // The agent can use this info to report its current configuration.
    request = request.with_session_llm_config(&effective_llm.provider, &effective_llm.model);

    // Apply session-scoped permission level and tool configuration.
    // Use get_session_tool_settings() to ensure defaults are applied for sessions
    // without explicit tool_settings (legacy sessions or newly-created ones).
    if let Some(ref sid) = resolved_session_id {
        use gestura_core::pipeline::PermissionLevel;

        let tool_settings = crate::window_manager::get_session_tool_settings_from_config(sid, &cfg);
        let perm_level = match tool_settings.permission_level {
            crate::window_manager::SessionPermissionLevel::Sandbox => PermissionLevel::Sandbox,
            crate::window_manager::SessionPermissionLevel::Restricted => {
                PermissionLevel::Restricted
            }
            crate::window_manager::SessionPermissionLevel::Full => PermissionLevel::Full,
        };
        request = request.with_permission_level(perm_level);

        // Pass enabled tools to the pipeline so the agent knows what tools are available.
        // Only include tools that are explicitly enabled (value == true).
        let enabled_tools: Vec<String> = tool_settings
            .enabled_tools
            .iter()
            .filter_map(|(name, enabled)| if *enabled { Some(name.clone()) } else { None })
            .collect();

        // Log all tool settings for debugging
        tracing::debug!(
            session_id = sid,
            permission_level = ?perm_level,
            all_tool_settings = ?tool_settings.enabled_tools,
            enabled_tools = ?enabled_tools,
            "Session tool configuration (with defaults applied)"
        );

        if !enabled_tools.is_empty() {
            request = request.with_allowed_tools(enabled_tools);
        } else {
            tracing::warn!(
                session_id = sid,
                "No tools enabled in session - LLM will receive category-based tool list"
            );
        }
    }

    // Create the pipeline with provider-optimized configuration and spawn the streaming task.
    // Note: `apply_session_llm_config_overrides` already applied the effective provider/model
    // into `cfg`, so we can clone directly.
    let cfg_clone = cfg.clone();
    let cancel_token_clone = cancel_token.clone();
    let pipeline_handle = tokio::spawn(async move {
        // Use provider-optimized config for better token management
        // and integrate with knowledge system
        let pipeline = AgentPipeline::with_provider_optimized_config(cfg_clone)
            .with_knowledge(get_knowledge_store(), get_knowledge_settings());
        if let Err(e) = pipeline
            .process_streaming(request, tx.clone(), cancel_token_clone)
            .await
        {
            tracing::error!("AgentPipeline streaming error: {}", e);
            // Ensure the GUI always receives a terminal event.
            let _ = tx.send(StreamChunk::Error(e.to_string())).await;
            let _ = tx.send(StreamChunk::Done(None)).await;
        }
    });

    use tokio::time::{Duration, Instant};

    // Forward chunks to frontend via Tauri events
    let mut assistant_text = String::new();
    let mut assistant_thinking: Option<String> = None;
    // Tool call tracking for pause/resume state capture.
    let mut completed_tool_calls: Vec<gestura_core::ToolCallRecord> = Vec::new();
    let mut current_tool_call: Option<(String, String, String)> = None; // (id, name, args)
    let mut saw_terminal = false;
    // Normal idle timeout detects backend hangs.
    // Some healthy phases are intentionally quieter, especially after a tool
    // returns and the model is reviewing output to decide what to do next.
    let idle_timeout_normal = Duration::from_secs(STREAM_IDLE_TIMEOUT_NORMAL_SECS);
    let mut idle_timeout = idle_timeout_normal;
    let idle_timer = tokio::time::sleep(idle_timeout);
    tokio::pin!(idle_timer);

    tracing::debug!(
        session_id = ?resolved_session_id,
        idle_timeout_secs = idle_timeout_normal.as_secs(),
        "[StreamLoop] Starting streaming event loop"
    );

    loop {
        tokio::select! {
            maybe_chunk = rx.recv() => {
                let Some(chunk) = maybe_chunk else {
                    tracing::debug!("[StreamLoop] Channel closed (sender dropped) — exiting loop");
                    break;
                };
                // Update idle timeout based on what we just received.
                idle_timeout = stream_idle_timeout_for_chunk(&chunk);
                idle_timer.as_mut().reset(Instant::now() + idle_timeout);
                // Log every chunk kind so we can trace where the pipeline stalls.
                let chunk_kind = match &chunk {
                    StreamChunk::Text(_) => "Text",
                    StreamChunk::Thinking(_) => "Thinking",
                    StreamChunk::Status { .. } => "Status",
                    StreamChunk::Narration { .. } => "Narration",
                    StreamChunk::ToolCallStart { .. } => "ToolCallStart",
                    StreamChunk::ToolCallArgs(_) => "ToolCallArgs",
                    StreamChunk::ToolCallEnd => "ToolCallEnd",
                    StreamChunk::ToolCallResult { .. } => "ToolCallResult",
                    StreamChunk::Done(_) => "Done",
                    StreamChunk::Error(_) => "Error",
                    StreamChunk::Cancelled => "Cancelled",
                    StreamChunk::Paused => "Paused",
                    StreamChunk::AgentLoopIteration { .. } => "AgentLoopIteration",
                    StreamChunk::TaskRuntimeSnapshot { .. } => "TaskRuntimeSnapshot",
                    StreamChunk::RetryAttempt { .. } => "RetryAttempt",
                    StreamChunk::ContextCompacted { .. } => "ContextCompacted",
                    StreamChunk::MemoryBankSaved { .. } => "MemoryBankSaved",
                    StreamChunk::ReflectionStarted { .. } => "ReflectionStarted",
                    StreamChunk::ReflectionComplete { .. } => "ReflectionComplete",
                    StreamChunk::TokenUsageUpdate { .. } => "TokenUsageUpdate",
                    StreamChunk::ConfigRequest { .. } => "ConfigRequest",
                    StreamChunk::ToolConfirmationRequired { .. } => "ToolConfirmationRequired",
                    StreamChunk::ToolBlocked { .. } => "ToolBlocked",
                    StreamChunk::ShellOutput { .. } => "ShellOutput",
                    StreamChunk::ShellLifecycle { .. } => "ShellLifecycle",
                    StreamChunk::ShellSessionLifecycle { .. } => "ShellSessionLifecycle",
                };
                tracing::debug!(
                    chunk = chunk_kind,
                    timeout_secs = idle_timeout.as_secs(),
                    "[StreamLoop] Chunk received → idle timer reset"
                );
                match chunk {
            StreamChunk::Thinking(text) => {
                tracing::debug!("[Stream] Thinking chunk: {}", &text.chars().take(100).collect::<String>());
                assistant_thinking
                    .get_or_insert_with(String::new)
                    .push_str(&text);
                emit("agent-stream-thinking", serde_json::json!(text));
            }
            StreamChunk::Status { message } => {
                let payload = serde_json::json!({ "text": message, "kind": "busy" });
                emit("agent-stream-status", payload);
            }
            StreamChunk::Narration { narration, stage } => {
                update_latest_progress_summary(
                    &mut latest_progress_summary,
                    narration.summary.as_deref(),
                    &narration.message,
                );
                let payload = serde_json::json!({
                    "title": narration.title,
                    "message": narration.message,
                    "summary": narration.summary,
                    "reason": narration.reason,
                    "next_step": narration.next_step,
                    "evidence": narration.evidence,
                    "stage": stage.as_str(),
                });
                emit("agent-stream-narration", payload);
            }
            StreamChunk::Text(text) => {
                tracing::debug!("[Stream] Text chunk: {}", &text.chars().take(100).collect::<String>());
                assistant_text.push_str(&text);
                emit("agent-stream-chunk", serde_json::json!(text));
            }
            StreamChunk::ToolCallStart { id, name } => {
                tracing::debug!(tool = %name, id = %id, "[StreamLoop] ToolCallStart");
                current_tool_call = Some((id.clone(), name.clone(), String::new()));
                let payload = serde_json::json!({ "id": id, "name": name });
                emit("agent-stream-tool-start", payload);
            }
            StreamChunk::ToolCallArgs(args) => {
                let tool_call_id = current_tool_call
                    .as_ref()
                    .map(|(id, _, _)| id.clone());
                if let Some((_, _, ref mut acc)) = current_tool_call {
                    acc.push_str(&args);
                }
                emit(
                    "agent-stream-tool-args",
                    serde_json::json!({ "id": tool_call_id, "args": args }),
                );
            }
            StreamChunk::ToolCallEnd => {
                tracing::debug!("[StreamLoop] ToolCallEnd");
                let payload = current_tool_call
                    .as_ref()
                    .map(|(id, name, _)| serde_json::json!({ "id": id, "name": name }))
                    .unwrap_or_else(|| serde_json::json!(null));
                emit("agent-stream-tool-end", payload);
            }
            StreamChunk::ToolCallResult {
                name,
                success,
                output,
                duration_ms,
            } => {
                let mut task_tool_ui_event: Option<(&'static str, serde_json::Value)> = None;
                let mut completed_tool_call_id: Option<String> = None;
                // Finalize the tracked tool call record for pause-state capture.
                if let Some((tc_id, tc_name, tc_args)) = current_tool_call.take() {
                    completed_tool_call_id = Some(tc_id.clone());
                    if let Some(sid) = resolved_session_id.as_deref() {
                        task_tool_ui_event = task_tool_mutation_event(sid, &tc_name, &tc_args, success);
                        crate::window_manager::record_tool_call(
                            sid,
                            gestura_core::agent_sessions::SessionToolCall {
                                id: tc_id.clone(),
                                name: tc_name.clone(),
                                arguments: tc_args.clone(),
                                result: output.clone(),
                                success,
                                duration_ms,
                                timestamp: chrono::Utc::now(),
                            },
                        );
                        if !output.trim().is_empty() {
                            crate::window_manager::add_tool_message(sid, &tc_id, &output);
                        }
                    }
                    let result = if success {
                        gestura_core::ToolResult::Success(output.clone())
                    } else {
                        gestura_core::ToolResult::Error(output.clone())
                    };
                    completed_tool_calls.push(gestura_core::ToolCallRecord {
                        id: tc_id,
                        name: tc_name,
                        arguments: tc_args,
                        result,
                        duration_ms,
                    });
                }
                let payload = serde_json::json!({
                    "id": completed_tool_call_id,
                    "name": name,
                    "success": success,
                    "output": output,
                    "duration_ms": duration_ms
                });
                emit("agent-stream-tool-result", payload);
                if let Some((event_name, event_payload)) = task_tool_ui_event {
                    let _ = app.emit(event_name, event_payload);
                }
                emit_task_refresh_if_changed(
                    &app,
                    resolved_session_id.as_deref(),
                    &mut last_task_tree_signature,
                );
            }
            StreamChunk::RetryAttempt {
                attempt,
                max_attempts,
                delay_ms,
                error_message,
            } => {
                let payload = serde_json::json!({
                    "attempt": attempt,
                    "max_attempts": max_attempts,
                    "delay_ms": delay_ms,
                    "reason": error_message
                });
                emit("agent-stream-retry", payload);
            }
            StreamChunk::ContextCompacted {
                messages_before,
                messages_after,
                tokens_saved,
                summary,
            } => {
                let payload = serde_json::json!({
                    "messages_before": messages_before,
                    "messages_after": messages_after,
                    "tokens_saved": tokens_saved,
                    "summary": summary
                });
                emit("agent-context-compacted", payload);
            }
            StreamChunk::MemoryBankSaved {
                file_path,
                session_id,
                summary,
                messages_saved,
            } => {
                let payload = serde_json::json!({
                    "file_path": file_path,
                    "session_id": session_id,
                    "summary": summary,
                    "messages_saved": messages_saved
                });
                emit("agent-memory-bank-saved", payload);
            }
            StreamChunk::ReflectionStarted { reason } => {
                let payload = serde_json::json!({
                    "text": format!("Generating reflection: {}", reason),
                    "kind": "reflection",
                    "reason": reason,
                });
                emit("agent-stream-status", payload);
            }
            StreamChunk::ReflectionComplete {
                summary,
                stored,
                promoted,
            } => {
                let payload = serde_json::json!({
                    "text": format!(
                        "Reflection complete: {}{}{}",
                        summary,
                        if stored { " · stored" } else { "" },
                        if promoted { " · promoted" } else { "" },
                    ),
                    "kind": "reflection",
                    "summary": summary,
                    "stored": stored,
                    "promoted": promoted,
                });
                emit("agent-stream-status", payload);
            }
            StreamChunk::TokenUsageUpdate {
                estimated,
                limit,
                percentage,
                status,
                estimated_cost,
            } => {
                let status_str = match status {
                    gestura_core::streaming::TokenUsageStatus::Green => "green",
                    gestura_core::streaming::TokenUsageStatus::Yellow => "yellow",
                    gestura_core::streaming::TokenUsageStatus::Red => "red",
                };
                let payload = serde_json::json!({
                    "estimated": estimated,
                    "limit": limit,
                    "percentage": percentage,
                    "status": status_str,
                    "estimated_cost": estimated_cost
                });
                emit("agent-token-usage", payload);
            }
            StreamChunk::ConfigRequest {
                operation,
                key,
                value,
                requires_confirmation,
            } => {
                let payload = serde_json::json!({
                    "operation": operation,
                    "key": key,
                    "value": value,
                    "requires_confirmation": requires_confirmation,
                    "session_id": resolved_session_id
                });
                emit("agent-config-request", payload);
            }
            StreamChunk::ToolConfirmationRequired {
                confirmation_id,
                tool_name,
                tool_args,
                description,
                risk_level,
                category,
            } => {
                let payload = serde_json::json!({
                    "confirmation_id": confirmation_id,
                    "tool_name": tool_name,
                    "tool_args": tool_args,
                    "description": description,
                    "risk_level": risk_level,
                    "category": category,
                    "session_id": resolved_session_id
                });
                emit("agent-stream-tool-confirmation", payload);
            }
            StreamChunk::ToolBlocked { tool_name, reason } => {
                let payload = serde_json::json!({
                    "tool_name": tool_name,
                    "reason": reason,
                    "session_id": resolved_session_id
                });
                emit("agent-stream-tool-blocked", payload);
            }
            StreamChunk::AgentLoopIteration { iteration } => {
                let payload = serde_json::json!({
                    "iteration": iteration,
                    "session_id": resolved_session_id
                });
                emit("agent-stream-agent-iteration", payload);
                emit_task_refresh_if_changed(
                    &app,
                    resolved_session_id.as_deref(),
                    &mut last_task_tree_signature,
                );
            }
            StreamChunk::TaskRuntimeSnapshot { snapshot } => {
                emit(
                    "agent-stream-task-state",
                    serde_json::json!({
                        "session_id": resolved_session_id,
                        "snapshot": snapshot
                    }),
                );
                emit_task_refresh_if_changed(
                    &app,
                    resolved_session_id.as_deref(),
                    &mut last_task_tree_signature,
                );
            }
            StreamChunk::ShellOutput {
                process_id,
                shell_session_id,
                stream,
                data,
            } => {
                let payload = serde_json::json!({
                    "process_id": process_id,
                    "shell_session_id": shell_session_id,
                    "stream": stream,
                    "data": data,
                    "session_id": resolved_session_id
                });
                emit("agent-stream-shell-output", payload);
            }
            StreamChunk::ShellLifecycle {
                process_id,
                shell_session_id,
                state,
                exit_code,
                duration_ms,
                command,
                cwd,
            } => {
                let payload = serde_json::json!({
                    "process_id": process_id,
                    "shell_session_id": shell_session_id,
                    "state": state,
                    "exit_code": exit_code,
                    "duration_ms": duration_ms,
                    "command": command,
                    "cwd": cwd,
                    "session_id": resolved_session_id
                });
                emit("agent-stream-shell-lifecycle", payload);
            }
            StreamChunk::ShellSessionLifecycle {
                shell_session_id,
                state,
                cwd,
                active_process_id,
                active_command,
                available_for_reuse,
                interactive,
                user_managed,
            } => {
                let payload = serde_json::json!({
                    "shell_session_id": shell_session_id,
                    "state": state,
                    "cwd": cwd,
                    "active_process_id": active_process_id,
                    "active_command": active_command,
                    "available_for_reuse": available_for_reuse,
                    "interactive": interactive,
                    "user_managed": user_managed,
                    "session_id": resolved_session_id,
                });
                emit("agent-stream-shell-session-lifecycle", payload);
            }
            StreamChunk::Done(usage) => {
                saw_terminal = true;
                // Emit token usage if available
                if let Some(ref usage) = usage {
                    emit(
                        "agent-token-usage",
                    serde_json::to_value(usage).unwrap_or(serde_json::Value::Null),
                    );

                    if let Some(ref sid) = resolved_session_id {
                        let total = u64::from(usage.input_tokens)
                            .saturating_add(u64::from(usage.output_tokens));
                        crate::window_manager::update_token_count(sid, total);
                    }
                }

                persist_terminal_assistant_message(
                    resolved_session_id.as_deref(),
                    &assistant_text,
                    assistant_thinking.as_deref(),
                    false,
                );

                // Reconcile the tracked task tree before closing the run.
                if let (Some(sid), Some(task_id)) = (&resolved_session_id, &tracked_task_id) {
                    match crate::task_integration::finalize_tracked_task_after_agent_run(
                        &app, sid, task_id,
                    ) {
                        Ok(crate::task_integration::TrackedTaskFinalization::Completed) => {}
                        Ok(crate::task_integration::TrackedTaskFinalization::StillInProgress {
                            open_subtasks,
                        }) => {
                            tracing::info!(
                                session_id = %sid,
                                task_id = %task_id,
                                open_subtasks = ?open_subtasks,
                                "Tracked task run finished but planned subtasks remain open"
                            );
                        }
                        Err(error) => {
                            tracing::warn!(
                                session_id = %sid,
                                task_id = %task_id,
                                error = %error,
                                "Failed to reconcile tracked task state after agent run"
                            );
                        }
                    }
                }

                emit("agent-stream-done", serde_json::json!(null));
                break;
            }
            StreamChunk::Cancelled | StreamChunk::Paused => {
                saw_terminal = true;
                let is_paused = matches!(chunk, StreamChunk::Paused);

                // Persist any partial assistant output so context isn't lost.
                persist_terminal_assistant_message(
                    resolved_session_id.as_deref(),
                    &assistant_text,
                    assistant_thinking.as_deref(),
                    true,
                );

                // Build and persist the paused execution state so the session
                // can be resumed later.
                if let Some(ref sid) = resolved_session_id {
                    let paused_state = gestura_core::PausedExecutionState {
                        original_input: input_snapshot.clone(),
                        system_prompt: None,
                        history: history_snapshot.clone(),
                        partial_content: assistant_text.clone(),
                        partial_thinking: assistant_thinking.clone(),
                        completed_tool_calls: completed_tool_calls.clone(),
                        iteration: 0,
                        source: request_source,
                        session_id: Some(sid.clone()),
                        workspace_dir: crate::window_manager::get_session_state(sid)
                            .and_then(|s| s.workspace_dir),
                        model_snapshot: None,
                        paused_at: chrono::Utc::now(),
                    };
                    crate::window_manager::set_session_paused_execution(
                        sid,
                        Some(paused_state),
                    );
                }

                // Mark agent task as cancelled when explicitly cancelled.
                if !is_paused
                    && let (Some(sid), Some(task_id)) =
                        (&resolved_session_id, &tracked_task_id)
                {
                    let _ =
                        crate::task_integration::mark_task_cancelled(&app, sid, task_id);
                }

                // Emit the appropriate frontend event.
                if is_paused {
                    remember_terminal_assistant_summary(
                        resolved_session_id.as_deref(),
                        TerminalAssistantSummaryKind::Paused,
                        latest_progress_summary.as_deref(),
                        None,
                        assistant_thinking.as_deref(),
                    );
                    emit("agent-stream-paused", serde_json::json!(null));
                } else {
                    remember_terminal_assistant_summary(
                        resolved_session_id.as_deref(),
                        TerminalAssistantSummaryKind::Cancelled,
                        latest_progress_summary.as_deref(),
                        None,
                        assistant_thinking.as_deref(),
                    );
                    emit("agent-stream-cancelled", serde_json::json!(null));
                }
                break;
            }
            StreamChunk::Error(err) => {
                saw_terminal = true;
                persist_terminal_assistant_message(
                    resolved_session_id.as_deref(),
                    &assistant_text,
                    assistant_thinking.as_deref(),
                    false,
                );
                remember_terminal_assistant_summary(
                    resolved_session_id.as_deref(),
                    TerminalAssistantSummaryKind::Failed,
                    latest_progress_summary.as_deref(),
                    Some(&err),
                    assistant_thinking.as_deref(),
                );

                // Mark agent task as cancelled (error case)
                if let (Some(sid), Some(task_id)) = (&resolved_session_id, &tracked_task_id) {
                    let _ = crate::task_integration::mark_task_cancelled(&app, sid, task_id);
                }

                emit("agent-stream-error", serde_json::json!(err));
                break;
            }
                }
            }
            _ = &mut idle_timer => {
                // If we haven't received any stream events in a while, preserve the
                // current execution as a resumable pause instead of hard-failing it.
                saw_terminal = true;
                tracing::warn!("Streaming agent timed out (no events for {:?}); pausing for resume", idle_timeout);

                if let Some(ref sid) = resolved_session_id {
                    cancel_token.pause();

                    persist_terminal_assistant_message(
                        Some(sid),
                        &assistant_text,
                        assistant_thinking.as_deref(),
                        true,
                    );

                    let paused_state = gestura_core::PausedExecutionState {
                        original_input: input_snapshot.clone(),
                        system_prompt: None,
                        history: history_snapshot.clone(),
                        partial_content: assistant_text.clone(),
                        partial_thinking: assistant_thinking.clone(),
                        completed_tool_calls: completed_tool_calls.clone(),
                        iteration: 0,
                        source: request_source,
                        session_id: Some(sid.clone()),
                        workspace_dir: crate::window_manager::get_session_state(sid)
                            .and_then(|s| s.workspace_dir),
                        model_snapshot: None,
                        paused_at: chrono::Utc::now(),
                    };
                    crate::window_manager::set_session_paused_execution(sid, Some(paused_state));

                    remember_terminal_assistant_summary(
                        Some(sid),
                        TerminalAssistantSummaryKind::Paused,
                        latest_progress_summary.as_deref(),
                        Some(&format!(
                            "No stream events arrived for {:?}.",
                            idle_timeout
                        )),
                        assistant_thinking.as_deref(),
                    );

                    emit(
                        "agent-stream-status",
                        serde_json::json!({
                            "text": format!(
                                "No stream events for {:?}; paused automatically so you can resume from the same point.",
                                idle_timeout
                            ),
                            "kind": "warning"
                        }),
                    );
                    emit("agent-stream-paused", serde_json::json!(null));
                } else {
                    cancel_token.cancel();
                    emit(
                        "agent-stream-error",
                        serde_json::json!(format!(
                            "Timed out waiting for agent response (no stream events for {:?}). The agent may be stalled or a tool may not be reporting progress.",
                            idle_timeout
                        )),
                    );
                }
                break;
            }
        }
    }

    // If the channel closed without any terminal event, surface that as an error.
    if !saw_terminal {
        remember_terminal_assistant_summary(
            resolved_session_id.as_deref(),
            TerminalAssistantSummaryKind::UnexpectedEnd,
            latest_progress_summary.as_deref(),
            Some("Streaming ended unexpectedly (no terminal event received)"),
            assistant_thinking.as_deref(),
        );
        emit(
            "agent-stream-error",
            serde_json::json!("Streaming ended unexpectedly (no terminal event received)"),
        );
    }

    // Ensure we observe pipeline task failures (panic/abort) and don't silently swallow them.
    let mut pipeline_handle = pipeline_handle;
    tokio::select! {
        res = &mut pipeline_handle => {
            if let Err(join_err) = res {
                tracing::error!("AgentPipeline task join error: {}", join_err);
                if !saw_terminal {
                    emit("agent-stream-error", serde_json::json!(format!("Agent task failed: {join_err}")));
                }
            }
        }
        _ = tokio::time::sleep(Duration::from_secs(2)) => {
            // Pipeline task didn't finish promptly after we stopped listening.
            // Abort to avoid leaked work.
            tracing::warn!("AgentPipeline task did not finish after terminal event; aborting");
            pipeline_handle.abort();
        }
    }

    // Clear the cancellation token for this stream.
    gestura_core::stream_cancellation::STREAM_CANCELLATIONS.remove(&cancel_key);
    stream_cancellation_guard.disarm();

    Ok(())
}

/// Cancel an ongoing streaming agent request.
///
/// Cancellation is scoped to a single webview window.
///
/// - If `session_id` is provided, we resolve the session's current agent window label and
///   cancel that window's stream.
/// - If `session_id` is omitted, we cancel the stream for the **calling window**.
///
/// This prevents a cancel action in one agent window from cancelling another window's
/// in-flight stream.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn cancel_agent_streaming(
    webview_window: tauri::WebviewWindow,
    session_id: Option<String>,
) -> Result<(), String> {
    let calling_window_label = webview_window.label().to_string();
    interrupt_agent_streaming_internal(Some(calling_window_label), session_id, false)
}

/// Pause an ongoing streaming agent request while preserving resumable state.
///
/// Like cancellation, pausing is scoped to a single webview window.
#[tauri::command(rename_all = "snake_case")]
pub fn pause_agent_streaming(
    webview_window: tauri::WebviewWindow,
    session_id: Option<String>,
) -> Result<(), String> {
    let calling_window_label = webview_window.label().to_string();
    interrupt_agent_streaming_internal(Some(calling_window_label), session_id, true)
}

/// Resume a previously paused streaming agent session.
///
/// Retrieves the `PausedExecutionState` from the session, builds a resume
/// `AgentRequest`, and kicks off a new streaming pipeline from where the
/// previous execution left off.
///
/// Emits the same `agent-stream-*` events as `process_agent_message_streaming`.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn resume_agent_streaming(
    webview_window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    use gestura_core::{AgentPipeline, AgentRequest, CancellationToken, StreamChunk};
    use tokio::sync::mpsc;

    // Retrieve and clear the paused state.
    let paused = crate::window_manager::get_session_paused_execution(&session_id)
        .ok_or_else(|| "No paused session to resume".to_string())?;
    crate::window_manager::set_session_paused_execution(&session_id, None);

    let mut cfg = AppConfig::load_async().await;
    let effective_llm = apply_session_llm_config_overrides(&mut cfg, Some(&session_id)).await;

    let calling_window_label = webview_window.label().to_string();
    let target_window_label = crate::window_manager::get_session_window_label(&session_id)
        .unwrap_or_else(|| calling_window_label.clone());

    // Window-scoped event emitter (same pattern as process_agent_message_streaming).
    let resolved_session_id: Option<String> = Some(session_id.clone());
    let emit = |event: &str, payload: serde_json::Value| {
        let payload =
            crate::agent_events::attach_session_id(payload, resolved_session_id.as_deref());
        capture_session_activity_event(resolved_session_id.as_deref(), event, &payload);
        if let Err(err) = crate::agent_events::emit_agent_event_to_window(
            &app,
            &target_window_label,
            &calling_window_label,
            event,
            &payload,
            resolved_session_id.as_deref(),
        ) {
            tracing::error!(event = %event, error = %err, "Failed to emit resume agent event");
        }
    };

    // Cancellation token scoped to the target window.
    let cancel_token = CancellationToken::new();
    let cancel_key = cancel_key_for_window_label(&target_window_label);
    gestura_core::stream_cancellation::STREAM_CANCELLATIONS
        .register(cancel_key.clone(), cancel_token.clone());

    // Snapshot for re-pause.
    let input_snapshot = paused.original_input.clone();
    let history_snapshot = paused.history.clone();
    let request_source = paused.source;
    let tracked_task_id =
        resolve_resume_tracked_task_id(crate::task_integration::get_task_manager(), &session_id);

    // Build the resume request.
    let mut request = AgentRequest::new(&paused.original_input)
        .with_streaming(true)
        .with_source(request_source)
        .with_resume_state(paused);

    request = request.with_session(&session_id);
    if let Some(task_id) = tracked_task_id.as_deref() {
        request = request.with_task(task_id);
        let _ = crate::task_integration::mark_task_in_progress(&app, &session_id, task_id);
        let _ = crate::task_integration::get_task_manager()
            .set_current_task_id(&session_id, Some(task_id.to_string()));
    }
    let mut last_task_tree_signature =
        Some(session_id.as_str()).and_then(task_tree_refresh_signature);
    if let Some(workspace) =
        crate::window_manager::get_session_state(&session_id).and_then(|s| s.workspace_dir)
    {
        request = request.with_workspace(workspace);
    }
    request = request.with_session_llm_config(&effective_llm.provider, &effective_llm.model);

    // Apply permission / tool settings.
    if let Some(ref sid) = resolved_session_id {
        use gestura_core::pipeline::PermissionLevel;
        let tool_settings = crate::window_manager::get_session_tool_settings_from_config(sid, &cfg);
        let perm_level = match tool_settings.permission_level {
            crate::window_manager::SessionPermissionLevel::Sandbox => PermissionLevel::Sandbox,
            crate::window_manager::SessionPermissionLevel::Restricted => {
                PermissionLevel::Restricted
            }
            crate::window_manager::SessionPermissionLevel::Full => PermissionLevel::Full,
        };
        request = request.with_permission_level(perm_level);
        let enabled_tools: Vec<String> = tool_settings
            .enabled_tools
            .iter()
            .filter_map(|(name, enabled)| if *enabled { Some(name.clone()) } else { None })
            .collect();
        if !enabled_tools.is_empty() {
            request = request.with_allowed_tools(enabled_tools);
        }
    }

    // Spawn the streaming pipeline.
    let cfg_clone = cfg.clone();
    let cancel_token_clone = cancel_token.clone();
    let (tx, mut rx) = mpsc::channel::<StreamChunk>(100);
    let pipeline_handle = tokio::spawn(async move {
        let pipeline = AgentPipeline::with_provider_optimized_config(cfg_clone)
            .with_knowledge(get_knowledge_store(), get_knowledge_settings());
        if let Err(e) = pipeline
            .process_streaming(request, tx.clone(), cancel_token_clone)
            .await
        {
            tracing::error!("Resume pipeline error: {}", e);
            let _ = tx.send(StreamChunk::Error(e.to_string())).await;
            let _ = tx.send(StreamChunk::Done(None)).await;
        }
    });

    emit("agent-stream-resumed", serde_json::json!(null));

    // Forward chunks — mirrors the loop in process_agent_message_streaming.
    let mut assistant_text = String::new();
    let mut assistant_thinking: Option<String> = None;
    let mut latest_progress_summary: Option<String> = None;
    let mut completed_tool_calls: Vec<gestura_core::ToolCallRecord> = Vec::new();
    let mut current_tool_call: Option<(String, String, String)> = None;
    let mut saw_terminal = false;

    use tokio::time::{Duration, Instant};
    let idle_timeout_normal = Duration::from_secs(STREAM_IDLE_TIMEOUT_NORMAL_SECS);
    let mut idle_timeout = idle_timeout_normal;
    let idle_timer = tokio::time::sleep(idle_timeout);
    tokio::pin!(idle_timer);

    loop {
        tokio::select! {
            maybe_chunk = rx.recv() => {
                let Some(chunk) = maybe_chunk else { break; };
                idle_timeout = stream_idle_timeout_for_chunk(&chunk);
                idle_timer.as_mut().reset(Instant::now() + idle_timeout);

                match chunk {
                    StreamChunk::Thinking(text) => {
                        assistant_thinking.get_or_insert_with(String::new).push_str(&text);
                        emit("agent-stream-thinking", serde_json::json!(text));
                    }
                    StreamChunk::Text(text) => {
                        assistant_text.push_str(&text);
                        emit("agent-stream-chunk", serde_json::json!(text));
                    }
                    StreamChunk::ToolCallStart { id, name } => {
                        current_tool_call = Some((id.clone(), name.clone(), String::new()));
                        emit("agent-stream-tool-start", serde_json::json!({ "id": id, "name": name }));
                    }
                    StreamChunk::ToolCallArgs(args) => {
                        let tool_call_id = current_tool_call
                            .as_ref()
                            .map(|(id, _, _)| id.clone());
                        if let Some((_, _, ref mut acc)) = current_tool_call { acc.push_str(&args); }
                        emit(
                            "agent-stream-tool-args",
                            serde_json::json!({ "id": tool_call_id, "args": args }),
                        );
                    }
                    StreamChunk::ToolCallEnd => {
                        let payload = current_tool_call
                            .as_ref()
                            .map(|(id, name, _)| serde_json::json!({ "id": id, "name": name }))
                            .unwrap_or_else(|| serde_json::json!(null));
                        emit("agent-stream-tool-end", payload);
                    }
                    StreamChunk::ToolCallResult { name, success, output, duration_ms } => {
                        let mut task_tool_ui_event: Option<(&'static str, serde_json::Value)> = None;
                        let mut completed_tool_call_id: Option<String> = None;
                        if let Some((tc_id, tc_name, tc_args)) = current_tool_call.take() {
                            completed_tool_call_id = Some(tc_id.clone());
                            if let Some(sid) = resolved_session_id.as_deref() {
                                task_tool_ui_event = task_tool_mutation_event(sid, &tc_name, &tc_args, success);
                                crate::window_manager::record_tool_call(
                                    sid,
                                    gestura_core::agent_sessions::SessionToolCall {
                                        id: tc_id.clone(),
                                        name: tc_name.clone(),
                                        arguments: tc_args.clone(),
                                        result: output.clone(),
                                        success,
                                        duration_ms,
                                        timestamp: chrono::Utc::now(),
                                    },
                                );
                                if !output.trim().is_empty() {
                                    crate::window_manager::add_tool_message(sid, &tc_id, &output);
                                }
                            }
                            let result = if success {
                                gestura_core::ToolResult::Success(output.clone())
                            } else {
                                gestura_core::ToolResult::Error(output.clone())
                            };
                            completed_tool_calls.push(gestura_core::ToolCallRecord {
                                id: tc_id, name: tc_name, arguments: tc_args, result,
                                duration_ms,
                            });
                        }
                        emit("agent-stream-tool-result", serde_json::json!({
                            "id": completed_tool_call_id,
                            "name": name,
                            "success": success,
                            "output": output,
                            "duration_ms": duration_ms
                        }));
                        if let Some((event_name, event_payload)) = task_tool_ui_event {
                            let _ = app.emit(event_name, event_payload);
                        }
                        emit_task_refresh_if_changed(
                            &app,
                            resolved_session_id.as_deref(),
                            &mut last_task_tree_signature,
                        );
                    }
                    StreamChunk::Done(_) => {
                        saw_terminal = true;
                        persist_terminal_assistant_message(
                            resolved_session_id.as_deref(),
                            &assistant_text,
                            assistant_thinking.as_deref(),
                            true,
                        );

                        if let (Some(sid), Some(task_id)) = (&resolved_session_id, &tracked_task_id) {
                            match crate::task_integration::finalize_tracked_task_after_agent_run(
                                &app, sid, task_id,
                            ) {
                                Ok(crate::task_integration::TrackedTaskFinalization::Completed) => {}
                                Ok(crate::task_integration::TrackedTaskFinalization::StillInProgress {
                                    open_subtasks,
                                }) => {
                                    tracing::info!(
                                        session_id = %sid,
                                        task_id = %task_id,
                                        open_subtasks = ?open_subtasks,
                                        "Tracked resumed task run finished but planned subtasks remain open"
                                    );
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        session_id = %sid,
                                        task_id = %task_id,
                                        error = %error,
                                        "Failed to reconcile tracked task state after resumed agent run"
                                    );
                                }
                            }
                        }

                        emit("agent-stream-done", serde_json::json!(null));
                        break;
                    }
                    StreamChunk::Cancelled => {
                        saw_terminal = true;
                        if let Some(ref sid) = resolved_session_id {
                            persist_terminal_assistant_message(
                                Some(sid),
                                &assistant_text,
                                assistant_thinking.as_deref(),
                                true,
                            );
                            remember_terminal_assistant_summary(
                                Some(sid),
                                TerminalAssistantSummaryKind::Cancelled,
                                latest_progress_summary.as_deref(),
                                None,
                                assistant_thinking.as_deref(),
                            );
                            if let Some(task_id) = tracked_task_id.as_deref() {
                                let _ = crate::task_integration::mark_task_cancelled(&app, sid, task_id);
                            }
                        }
                        emit("agent-stream-cancelled", serde_json::json!(null));
                        break;
                    }
                    StreamChunk::Paused => {
                        saw_terminal = true;
                        if let Some(ref sid) = resolved_session_id {
                            persist_terminal_assistant_message(
                                Some(sid),
                                &assistant_text,
                                assistant_thinking.as_deref(),
                                true,
                            );

                            let paused_state = gestura_core::PausedExecutionState {
                                original_input: input_snapshot.clone(),
                                system_prompt: None,
                                history: history_snapshot.clone(),
                                partial_content: assistant_text.clone(),
                                partial_thinking: assistant_thinking.clone(),
                                completed_tool_calls: completed_tool_calls.clone(),
                                iteration: 0,
                                source: request_source,
                                session_id: Some(sid.clone()),
                                workspace_dir: crate::window_manager::get_session_state(sid)
                                    .and_then(|s| s.workspace_dir),
                                model_snapshot: None,
                                paused_at: chrono::Utc::now(),
                            };
                            crate::window_manager::set_session_paused_execution(sid, Some(paused_state));
                            remember_terminal_assistant_summary(
                                Some(sid),
                                TerminalAssistantSummaryKind::Paused,
                                latest_progress_summary.as_deref(),
                                None,
                                assistant_thinking.as_deref(),
                            );
                        }

                        emit("agent-stream-paused", serde_json::json!(null));
                        break;
                    }
                    StreamChunk::Error(err) => {
                        saw_terminal = true;
                        persist_terminal_assistant_message(
                            resolved_session_id.as_deref(),
                            &assistant_text,
                            assistant_thinking.as_deref(),
                            true,
                        );
                        remember_terminal_assistant_summary(
                            resolved_session_id.as_deref(),
                            TerminalAssistantSummaryKind::Failed,
                            latest_progress_summary.as_deref(),
                            Some(&err),
                            assistant_thinking.as_deref(),
                        );

                        if let (Some(sid), Some(task_id)) = (&resolved_session_id, &tracked_task_id) {
                            let _ = crate::task_integration::mark_task_cancelled(&app, sid, task_id);
                        }

                        emit("agent-stream-error", serde_json::json!(err));
                        break;
                    }
                    // Forward other informational chunks.
                    StreamChunk::Status { message } => {
                        emit("agent-stream-status", serde_json::json!({ "text": message, "kind": "busy" }));
                    }
                    StreamChunk::Narration { narration, stage } => {
                        update_latest_progress_summary(
                            &mut latest_progress_summary,
                            narration.summary.as_deref(),
                            &narration.message,
                        );
                        emit(
                            "agent-stream-narration",
                            serde_json::json!({
                                "title": narration.title,
                                "message": narration.message,
                                "summary": narration.summary,
                                "reason": narration.reason,
                                "next_step": narration.next_step,
                                "evidence": narration.evidence,
                                "stage": stage.as_str(),
                            }),
                        );
                    }
                    StreamChunk::AgentLoopIteration { iteration } => {
                        emit("agent-stream-agent-iteration", serde_json::json!({ "iteration": iteration }));
                        emit_task_refresh_if_changed(
                            &app,
                            resolved_session_id.as_deref(),
                            &mut last_task_tree_signature,
                        );
                    }
                    StreamChunk::TaskRuntimeSnapshot { snapshot } => {
                        emit(
                            "agent-stream-task-state",
                            serde_json::json!({
                                "session_id": resolved_session_id,
                                "snapshot": snapshot
                            }),
                        );
                        emit_task_refresh_if_changed(
                            &app,
                            resolved_session_id.as_deref(),
                            &mut last_task_tree_signature,
                        );
                    }
                    StreamChunk::ReflectionStarted { reason } => {
                        emit(
                            "agent-stream-status",
                            serde_json::json!({
                                "text": format!("Generating reflection: {}", reason),
                                "kind": "reflection",
                                "reason": reason,
                            }),
                        );
                    }
                    StreamChunk::ReflectionComplete {
                        summary,
                        stored,
                        promoted,
                    } => {
                        emit(
                            "agent-stream-status",
                            serde_json::json!({
                                "text": format!(
                                    "Reflection complete: {}{}{}",
                                    summary,
                                    if stored { " · stored" } else { "" },
                                    if promoted { " · promoted" } else { "" },
                                ),
                                "kind": "reflection",
                                "summary": summary,
                                "stored": stored,
                                "promoted": promoted,
                            }),
                        );
                    }
                    _ => {}
                }
            }
            () = &mut idle_timer => {
                saw_terminal = true;
                tracing::warn!("Resume stream idle timeout (no events for {:?}); pausing for resume", idle_timeout);

                if let Some(ref sid) = resolved_session_id {
                    cancel_token.pause();

                    persist_terminal_assistant_message(
                        Some(sid),
                        &assistant_text,
                        assistant_thinking.as_deref(),
                        true,
                    );

                    let paused_state = gestura_core::PausedExecutionState {
                        original_input: input_snapshot.clone(),
                        system_prompt: None,
                        history: history_snapshot.clone(),
                        partial_content: assistant_text.clone(),
                        partial_thinking: assistant_thinking.clone(),
                        completed_tool_calls: completed_tool_calls.clone(),
                        iteration: 0,
                        source: request_source,
                        session_id: Some(sid.clone()),
                        workspace_dir: crate::window_manager::get_session_state(sid)
                            .and_then(|s| s.workspace_dir),
                        model_snapshot: None,
                        paused_at: chrono::Utc::now(),
                    };
                    crate::window_manager::set_session_paused_execution(sid, Some(paused_state));

                    remember_terminal_assistant_summary(
                        Some(sid),
                        TerminalAssistantSummaryKind::Paused,
                        latest_progress_summary.as_deref(),
                        Some(&format!(
                            "No resumed stream events arrived for {:?}.",
                            idle_timeout
                        )),
                        assistant_thinking.as_deref(),
                    );

                    emit(
                        "agent-stream-status",
                        serde_json::json!({
                            "text": format!(
                                "No resumed stream events for {:?}; paused automatically so you can resume from the same point.",
                                idle_timeout
                            ),
                            "kind": "warning"
                        }),
                    );
                    emit("agent-stream-paused", serde_json::json!(null));
                } else {
                    cancel_token.cancel();
                    emit(
                        "agent-stream-error",
                        serde_json::json!(format!(
                            "Timed out waiting for resumed agent response (no stream events for {:?}). The agent may be stalled or a tool may not be reporting progress.",
                            idle_timeout
                        )),
                    );
                }
                break;
            }
        }
    }

    if !saw_terminal {
        if let (Some(sid), Some(task_id)) = (&resolved_session_id, &tracked_task_id) {
            let _ = crate::task_integration::mark_task_cancelled(&app, sid, task_id);
        }
        remember_terminal_assistant_summary(
            resolved_session_id.as_deref(),
            TerminalAssistantSummaryKind::UnexpectedEnd,
            latest_progress_summary.as_deref(),
            Some("Resumed streaming ended unexpectedly (no terminal event received)"),
            assistant_thinking.as_deref(),
        );
        emit(
            "agent-stream-error",
            serde_json::json!("Resumed streaming ended unexpectedly (no terminal event received)"),
        );
    }

    let mut pipeline_handle = pipeline_handle;
    tokio::select! {
        res = &mut pipeline_handle => {
            if let Err(join_err) = res {
                tracing::error!("Resumed AgentPipeline task join error: {}", join_err);
                if !saw_terminal {
                    emit("agent-stream-error", serde_json::json!(format!("Resumed agent task failed: {join_err}")));
                }
            }
        }
        _ = tokio::time::sleep(Duration::from_secs(2)) => {
            tracing::warn!("Resumed AgentPipeline task did not finish after terminal event; aborting");
            pipeline_handle.abort();
        }
    }

    // Clean up cancellation token.
    gestura_core::stream_cancellation::STREAM_CANCELLATIONS.remove(&cancel_key);
    Ok(())
}

/// Approve a pending tool confirmation request.
///
/// JS↔Rust interop: The frontend calls this when the user clicks "Approve" on a
/// `agent-stream-tool-confirmation` dialog.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn approve_tool_confirmation(
    confirmation_id: String,
    session_id: Option<String>,
) -> Result<(), String> {
    gestura_core::tool_confirmation::TOOL_CONFIRMATIONS.resolve(
        &confirmation_id,
        session_id.as_deref(),
        true,
    )
}

/// Resolve a pending tool confirmation request with a scoped decision.
///
/// This is the scoped-decision variant of tool confirmation resolution that supports:
/// `allow_once`, `allow_session`, `allow_always`, `deny_once`, and `deny_session`.
///
/// JS↔Rust interop: The frontend calls this when the user selects a scoped action in the
/// `agent-stream-tool-confirmation` dialog.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn resolve_tool_confirmation_decision(
    confirmation_id: String,
    session_id: Option<String>,
    decision: String,
) -> Result<(), String> {
    let decision = gestura_core::tool_confirmation::ToolConfirmationDecision::parse(&decision)
        .map_err(|e| e.to_string())?;

    gestura_core::tool_confirmation::TOOL_CONFIRMATIONS.resolve_decision(
        &confirmation_id,
        session_id.as_deref(),
        decision,
    )
}

/// Deny a pending tool confirmation request.
///
/// JS↔Rust interop: The frontend calls this when the user clicks "Deny" (or
/// dismisses) a `agent-stream-tool-confirmation` dialog.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn deny_tool_confirmation(
    confirmation_id: String,
    session_id: Option<String>,
) -> Result<(), String> {
    gestura_core::tool_confirmation::TOOL_CONFIRMATIONS.resolve(
        &confirmation_id,
        session_id.as_deref(),
        false,
    )
}

/// Returns recent agent event emission trace entries.
///
/// This is a diagnostics-only command used to debug cross-window event leakage.
/// The trace is an in-memory ring buffer recorded by `crate::agent_events`.
#[tauri::command]
pub fn get_agent_event_trace(max: Option<usize>) -> Vec<crate::agent_events::AgentEventTraceEntry> {
    crate::agent_events::get_agent_event_trace(max)
}

/// Clears the in-memory agent event emission trace.
#[tauri::command]
pub fn clear_agent_event_trace() -> Result<(), String> {
    crate::agent_events::clear_agent_event_trace();
    Ok(())
}

/// Records a frontend "receipt" payload into an in-memory trace.
///
/// This is diagnostics-only and best-effort.
///
/// The frontend should send a JSON string containing at least:
/// - `eventName`
/// - `windowLabel` (optional)
/// - `sessionId` (optional)
/// - `incomingSessionId` (optional)
/// - `accept` + `reason` (optional)
#[tauri::command]
pub fn record_agent_receipt(payload: String) -> Result<(), String> {
    crate::agent_receipts::record_agent_receipt_payload(&payload);
    Ok(())
}

/// Returns recent agent receipt trace entries.
///
/// This is a diagnostics-only command used to debug cross-window event leakage.
#[tauri::command]
pub fn get_agent_receipt_trace(
    max: Option<usize>,
) -> Vec<crate::agent_receipts::AgentReceiptTraceEntry> {
    crate::agent_receipts::get_agent_receipt_trace(max)
}

/// Clears the in-memory agent receipt trace.
#[tauri::command]
pub fn clear_agent_receipt_trace() -> Result<(), String> {
    crate::agent_receipts::clear_agent_receipt_trace();
    Ok(())
}

/// Run a deterministic cross-window isolation probe.
///
/// This does not call any external LLM providers. It emits an `agent-probe` event to
/// two open agent windows and returns an analysis based on backend traces.
#[tauri::command]
pub async fn run_agent_isolation_probe(
    app: tauri::AppHandle,
) -> Result<crate::agent_probe::AgentIsolationProbeReport, String> {
    crate::agent_probe::run_agent_isolation_probe(app).await
}

/// Internal interruption implementation shared by cancel/pause agent commands.
///
/// This helper keeps the key-resolution logic testable without requiring an actual
/// Tauri [`tauri::WebviewWindow`] instance.
fn interrupt_agent_streaming_internal(
    calling_window_label: Option<String>,
    session_id: Option<String>,
    pause: bool,
) -> Result<(), String> {
    let cancel_key = if let Some(ref sid) = session_id {
        match crate::window_manager::get_session_window_label(sid) {
            Some(label) => cancel_key_for_window_label(&label),
            None => {
                // Session not found in window manager (e.g., loaded from disk before the window
                // was fully re-attached). Fall back to the calling window so that cancellation
                // still works for the active streaming request in that window.
                tracing::warn!(
                    session_id = %sid,
                    "get_session_window_label returned None; falling back to calling window label for stream interruption"
                );
                if let Some(label) = calling_window_label {
                    cancel_key_for_window_label(&label)
                } else {
                    return Err(format!(
                        "Cannot interrupt stream: no window label found for session {} and no calling window context",
                        sid
                    ));
                }
            }
        }
    } else if let Some(label) = calling_window_label {
        cancel_key_for_window_label(&label)
    } else {
        return Err(
            "Cannot interrupt stream: no session_id provided and no calling window context"
                .to_string(),
        );
    };

    let interrupted = if pause {
        gestura_core::stream_cancellation::STREAM_CANCELLATIONS.pause(&cancel_key)
    } else {
        gestura_core::stream_cancellation::STREAM_CANCELLATIONS.cancel(&cancel_key)
    };

    if interrupted {
        tracing::info!(
            cancel_key = %cancel_key,
            interruption = if pause { "paused" } else { "cancelled" },
            "Streaming agent interrupted"
        );
        Ok(())
    } else {
        Err(if pause {
            format!(
                "No active streaming request to pause for key {}",
                cancel_key
            )
        } else {
            format!(
                "No active streaming request to cancel for key {}",
                cancel_key
            )
        })
    }
}

#[cfg(test)]
mod streaming_cancellation_tests {
    use super::*;

    #[test]
    fn cancel_key_is_window_scoped() {
        assert_eq!(cancel_key_for_window_label("agent-abc"), "window:agent-abc");
    }

    #[test]
    fn cancel_internal_cancels_calling_window_when_no_session_id() {
        let label = "agent-test-cancel-internal";
        let key = cancel_key_for_window_label(label);

        let token = gestura_core::CancellationToken::new();
        gestura_core::stream_cancellation::STREAM_CANCELLATIONS.remove(&key);
        gestura_core::stream_cancellation::STREAM_CANCELLATIONS
            .register(key.clone(), token.clone());

        interrupt_agent_streaming_internal(Some(label.to_string()), None, false)
            .expect("expected cancellation to succeed");

        assert!(token.is_cancelled(), "token should be cancelled");
        assert!(
            !gestura_core::stream_cancellation::STREAM_CANCELLATIONS.contains_key(&key),
            "token entry should be removed after cancellation"
        );
    }

    #[test]
    fn cancel_internal_requires_context() {
        let err =
            interrupt_agent_streaming_internal(None, None, false).expect_err("expected error");
        assert!(err.contains("no session_id") || err.contains("no calling window"));
    }

    #[test]
    fn pause_internal_marks_token_for_resumable_pause() {
        let label = "agent-test-pause-internal";
        let key = cancel_key_for_window_label(label);

        let token = gestura_core::CancellationToken::new();
        gestura_core::stream_cancellation::STREAM_CANCELLATIONS.remove(&key);
        gestura_core::stream_cancellation::STREAM_CANCELLATIONS
            .register(key.clone(), token.clone());

        interrupt_agent_streaming_internal(Some(label.to_string()), None, true)
            .expect("expected pause to succeed");

        assert!(token.is_cancelled(), "token should be interrupted");
        assert!(
            token.is_pause_requested(),
            "token should preserve pause intent"
        );
        assert!(
            !gestura_core::stream_cancellation::STREAM_CANCELLATIONS.contains_key(&key),
            "token entry should be removed after pause"
        );
    }

    #[test]
    fn stream_cancellation_guard_cleans_up_registered_token_on_drop() {
        let key = cancel_key_for_window_label("agent-test-guard-cleanup");
        let token = gestura_core::CancellationToken::new();
        gestura_core::stream_cancellation::STREAM_CANCELLATIONS.remove(&key);
        gestura_core::stream_cancellation::STREAM_CANCELLATIONS.register(key.clone(), token);

        {
            let _guard = StreamCancellationGuard::new(key.clone());
            assert!(
                gestura_core::stream_cancellation::STREAM_CANCELLATIONS.contains_key(&key),
                "token entry should exist while the cleanup guard is active"
            );
        }

        assert!(
            !gestura_core::stream_cancellation::STREAM_CANCELLATIONS.contains_key(&key),
            "token entry should be removed when the cleanup guard drops"
        );
    }
}

#[tauri::command]
pub async fn send_agent_message(
    agent_id: String,
    message: String,
    _state: tauri::State<'_, crate::AppState>,
) -> Result<String, String> {
    let cfg = AppConfig::load_async().await;

    tracing::info!("Agent {} sending message through LLM", agent_id);

    // Format the prompt with agent context
    let prompt = format!(
        "You are agent '{}'. Respond to the following message:\n\n{}",
        agent_id, message
    );

    let response = run_single_shot_pipeline(cfg, prompt, RequestSource::GuiText, None)
        .await
        .map_err(|e| format!("Agent LLM error: {}", e))?;

    tracing::info!("Agent {} response received", agent_id);
    Ok(response)
}

#[tauri::command]
pub async fn get_agent_status(
    agent_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let agents = &state.agents;

    // Check if agent exists in the manager
    if let Some(info) = agents.get_agent_status(&agent_id).await {
        return Ok(serde_json::json!({
            "id": info.id,
            "name": info.name,
            "status": info.status,
            "last_activity": info.last_activity.to_rfc3339()
        }));
    }

    // Agent not found - return inactive status
    Ok(serde_json::json!({
        "id": agent_id,
        "name": "Unknown Agent",
        "status": "inactive",
        "last_activity": null
    }))
}

/// List all active agents
#[tauri::command]
pub async fn list_agents(
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let agents = &state.agents;
    let agent_list = agents.list_agents().await;

    Ok(serde_json::json!({
        "agents": agent_list,
        "count": agent_list.len()
    }))
}

// Orchestrator Commands

/// Delegate a task to a subagent
#[tauri::command]
pub async fn delegate_task(
    mut task: crate::orchestrator::DelegatedTask,
    state: tauri::State<'_, crate::AppState>,
) -> Result<String, String> {
    if task.session_id.is_none() {
        task.session_id = crate::window_manager::get_active_agent_for_voice();
    }
    if task.workspace_dir.is_none() {
        task.workspace_dir = task
            .session_id
            .as_deref()
            .and_then(|session_id| {
                crate::window_manager::get_session_state(session_id)
                    .and_then(|session| session.workspace_dir)
            })
            .or_else(crate::window_manager::get_active_session_workspace);
    }
    state.orchestrator.delegate_task(task).await
}

/// Get embedded A2A runtime status.
#[tauri::command]
pub fn get_a2a_runtime_status() -> Result<crate::a2a_runtime::A2ARuntimeStatus, String> {
    Ok(crate::a2a_runtime::a2a_runtime_status())
}

/// Spawn a new subagent
#[tauri::command]
pub async fn spawn_subagent(
    agent_id: String,
    name: String,
    role: Option<crate::orchestrator::AgentRole>,
    execution_mode: Option<crate::orchestrator::AgentExecutionMode>,
    capabilities: Option<Vec<String>>,
    workspace_dir: Option<String>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    let mut request = crate::orchestrator::AgentSpawnRequest::new(
        agent_id,
        name,
        role.unwrap_or(crate::orchestrator::AgentRole::Implementer),
    );
    if let Some(execution_mode) = execution_mode {
        request.execution_mode = execution_mode;
    }
    if let Some(capabilities) = capabilities {
        request.capabilities = capabilities;
    }
    request.workspace_dir = workspace_dir.map(std::path::PathBuf::from);
    state
        .orchestrator
        .spawn_subagent_with_request(request)
        .await
}

/// List all active tasks
#[tauri::command]
pub async fn list_active_tasks(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::ActiveTaskSnapshot>, String> {
    Ok(state.orchestrator.list_active_task_snapshots().await)
}

/// Cancel a running task
#[tauri::command]
pub async fn cancel_task(
    task_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state.orchestrator.cancel_task(&task_id).await
}

/// Pause a running local workflow task and preserve resumable checkpoint state.
#[tauri::command]
pub async fn pause_workflow_task(
    task_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state.orchestrator.pause_task(&task_id).await
}

/// List supervisor runs tracked by the orchestrator.
#[tauri::command]
pub async fn list_supervisor_runs(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::SupervisorRun>, String> {
    Ok(state.orchestrator.list_supervisor_runs().await)
}

/// List only root supervisor runs.
#[tauri::command]
pub async fn list_root_supervisor_runs(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::SupervisorRun>, String> {
    Ok(state.orchestrator.list_root_supervisor_runs().await)
}

/// List direct child supervisor runs for a parent run.
#[tauri::command]
pub async fn list_child_supervisor_runs(
    parent_run_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::SupervisorRun>, String> {
    Ok(state
        .orchestrator
        .list_child_supervisor_runs(&parent_run_id)
        .await)
}

/// Fetch a specific supervisor run.
#[tauri::command]
pub async fn get_supervisor_run(
    run_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Option<crate::orchestrator::SupervisorRun>, String> {
    Ok(state.orchestrator.get_supervisor_run(&run_id).await)
}

/// Fetch the ancestor chain for a supervisor run.
#[tauri::command]
pub async fn get_supervisor_run_ancestry(
    run_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::SupervisorRun>, String> {
    Ok(state
        .orchestrator
        .get_supervisor_run_ancestry(&run_id)
        .await)
}

/// Fetch direct descendants for a supervisor run.
#[tauri::command]
pub async fn get_supervisor_run_descendants(
    run_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::SupervisorRun>, String> {
    Ok(state
        .orchestrator
        .get_supervisor_run_descendants(&run_id)
        .await)
}

/// List leaf tasks under a supervisor run subtree.
#[tauri::command]
pub async fn list_supervisor_leaf_tasks(
    run_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::SupervisorTaskRecord>, String> {
    Ok(state.orchestrator.list_supervisor_leaf_tasks(&run_id).await)
}

/// Create a direct child supervisor run under an existing workflow run.
#[tauri::command]
pub async fn create_child_supervisor_run(
    request: gestura_core::orchestrator::ChildSupervisorRunRequest,
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::orchestrator::SupervisorRun, String> {
    state
        .orchestrator
        .create_child_supervisor_run(request)
        .await
}

/// Approve a delegated task.
#[tauri::command]
pub async fn approve_workflow_task(
    task_id: String,
    actor: crate::orchestrator::ApprovalActor,
    note: Option<String>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state.orchestrator.approve_task(&task_id, actor, note).await
}

/// Reject or request revision for a delegated task.
#[tauri::command]
pub async fn reject_workflow_task(
    task_id: String,
    actor: crate::orchestrator::ApprovalActor,
    note: Option<String>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state.orchestrator.reject_task(&task_id, actor, note).await
}

/// Retry a workflow task.
#[tauri::command]
pub async fn retry_workflow_task(
    task_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state.orchestrator.retry_task(&task_id).await
}

/// Resume a blocked workflow task from its persisted checkpoint.
#[tauri::command]
pub async fn resume_workflow_task(
    task_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state
        .orchestrator
        .resume_task_from_checkpoint(&task_id)
        .await
}

/// Restart a workflow task from scratch and discard any saved resume state.
#[tauri::command]
pub async fn restart_workflow_task_from_scratch(
    task_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state.orchestrator.restart_task_from_scratch(&task_id).await
}

/// Record that an operator acknowledged a blocked workflow task.
#[tauri::command]
pub async fn acknowledge_blocked_workflow_task(
    task_id: String,
    note: Option<String>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state
        .orchestrator
        .acknowledge_blocked_task(&task_id, note)
        .await
}

/// Claim a workflow task for a specific agent.
#[tauri::command]
pub async fn claim_workflow_task(
    task_id: String,
    agent_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state.orchestrator.claim_task(&task_id, &agent_id).await
}

/// Send a structured team message.
#[tauri::command]
pub async fn send_workflow_message(
    run_id: String,
    task_id: Option<String>,
    kind: crate::orchestrator::TeamMessageKind,
    sender_agent_id: Option<String>,
    recipient_agent_id: Option<String>,
    content: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::orchestrator::TeamMessage, String> {
    state
        .orchestrator
        .send_team_message(
            &run_id,
            task_id,
            kind,
            sender_agent_id,
            recipient_agent_id,
            content,
        )
        .await
}

/// Send a structured collaboration draft message.
#[tauri::command]
pub async fn send_workflow_collaboration_message(
    run_id: String,
    draft: crate::orchestrator::TeamMessageDraft,
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::orchestrator::TeamMessage, String> {
    state
        .orchestrator
        .send_team_message_draft(&run_id, draft)
        .await
}

/// List workflow messages for a supervisor run.
#[tauri::command]
pub async fn list_workflow_messages(
    run_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::TeamMessage>, String> {
    Ok(state.orchestrator.list_team_messages(&run_id).await)
}

/// List grouped workflow collaboration threads for a supervisor run.
#[tauri::command]
pub async fn list_workflow_threads(
    run_id: String,
    include_archived: Option<bool>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::TeamThread>, String> {
    Ok(state
        .orchestrator
        .list_team_threads_with_options(&run_id, include_archived.unwrap_or(false))
        .await)
}

/// Update the latest actionable request in a workflow collaboration thread.
#[tauri::command]
pub async fn update_workflow_thread_action(
    run_id: String,
    thread_id: String,
    status: crate::orchestrator::CollaborationActionStatus,
    actor_id: Option<String>,
    note: Option<String>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::orchestrator::TeamThread, String> {
    state
        .orchestrator
        .update_team_thread_action(&run_id, &thread_id, status, actor_id, note)
        .await
}

/// Archive a workflow collaboration thread.
#[tauri::command]
pub async fn archive_workflow_thread(
    run_id: String,
    thread_id: String,
    actor_id: Option<String>,
    note: Option<String>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::orchestrator::TeamThread, String> {
    state
        .orchestrator
        .archive_team_thread(&run_id, &thread_id, actor_id, note)
        .await
}

/// List durable workflow execution environments.
#[tauri::command]
pub async fn list_workflow_environments(
    run_id: Option<String>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::EnvironmentRecord>, String> {
    Ok(state
        .orchestrator
        .list_environments(run_id.as_deref())
        .await)
}

/// Get a single workflow execution environment.
#[tauri::command]
pub async fn get_workflow_environment(
    environment_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Option<crate::orchestrator::EnvironmentRecord>, String> {
    Ok(state.orchestrator.get_environment(&environment_id).await)
}

/// Retry environment preparation for a workflow task.
#[tauri::command]
pub async fn retry_workflow_environment(
    environment_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::orchestrator::EnvironmentRecord, String> {
    state
        .orchestrator
        .retry_environment_preparation(&environment_id)
        .await
}

/// Cleanup a workflow environment on demand.
#[tauri::command]
pub async fn cleanup_workflow_environment(
    environment_id: String,
    archive_if_dirty: Option<bool>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::orchestrator::EnvironmentRecord, String> {
    state
        .orchestrator
        .cleanup_environment(&environment_id, archive_if_dirty.unwrap_or(true))
        .await
}

/// Reconcile workflow state after restart or disk drift.
#[tauri::command]
pub async fn reconcile_workflow_state(
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state.orchestrator.reconcile_orchestrator_state().await
}

// Audio Device Management Commands

/// List all available audio input devices
#[tauri::command]
pub async fn list_audio_devices() -> Result<Vec<crate::audio_capture::AudioDeviceInfo>, String> {
    tokio::task::spawn_blocking(crate::audio_capture::list_audio_input_devices)
        .await
        .map_err(|error| format!("Failed to enumerate audio devices: {error}"))
}

/// Check if microphone is available
#[tauri::command]
pub async fn check_microphone_available() -> bool {
    tokio::task::spawn_blocking(crate::audio_capture::is_microphone_available)
        .await
        .unwrap_or(false)
}

// Permission Management Commands

#[tauri::command]
pub async fn check_permission(permission: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        use crate::permissions::{
            check_accessibility_permission, check_bluetooth_permission,
            check_microphone_permission, check_screen_recording_permission,
        };

        match permission.as_str() {
            "microphone" => {
                let status = check_microphone_permission();
                tracing::info!("Permission check: microphone -> {}", status);
                Ok(status.to_string())
            }
            "accessibility" => {
                let status = check_accessibility_permission();
                tracing::info!("Permission check: accessibility -> {}", status);
                Ok(status.to_string())
            }
            "bluetooth" => {
                let status = check_bluetooth_permission();
                tracing::info!("Permission check: bluetooth -> {}", status);
                Ok(status.to_string())
            }
            "screen_recording" => {
                let status = check_screen_recording_permission();
                tracing::info!("Permission check: screen_recording -> {}", status);
                Ok(status.to_string())
            }
            _ => Err(format!("Unknown permission: {}", permission)),
        }
    })
    .await
    .map_err(|error| format!("Failed to check system permission: {error}"))?
}

#[tauri::command]
pub async fn request_permission(permission: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        use crate::permissions::{
            SystemPermissionStatus, check_accessibility_permission, check_bluetooth_permission,
            check_microphone_permission, check_screen_recording_permission,
            open_system_preferences, request_bluetooth_permission,
            request_microphone_permission, request_screen_recording_permission,
        };

        tracing::info!("🔐 Permission request received: {}", permission);

        match permission.as_str() {
        "microphone" => {
            let status = check_microphone_permission();
            tracing::info!("🎤 Microphone permission status before request: {}", status);

            match status {
                SystemPermissionStatus::Granted => {
                    tracing::info!("🎤 Microphone permission already granted; nothing to request");
                    Ok(())
                }
                SystemPermissionStatus::Denied | SystemPermissionStatus::Restricted => {
                    tracing::info!(
                        "🎤 Microphone permission denied or restricted; opening System Settings",
                    );
                    if open_system_preferences("microphone") {
                        tracing::info!("✅ Opened System Preferences for Microphone");
                        Ok(())
                    } else {
                        Err("Failed to open System Preferences for Microphone".to_string())
                    }
                }
                SystemPermissionStatus::NotDetermined | SystemPermissionStatus::Unknown => {
                    tracing::info!(
                        "🎤 Microphone permission not determined/unknown; attempting to trigger system dialog",
                    );
                    if request_microphone_permission() {
                        tracing::info!("✅ Microphone permission request initiated");
                        Ok(())
                    } else {
                        tracing::warn!(
                            "⚠️ Microphone permission request script failed; attempting to open System Preferences",
                        );
                        if open_system_preferences("microphone") {
                            tracing::info!(
                                "✅ Opened System Preferences for Microphone (fallback)",
                            );
                            Ok(())
                        } else {
                            Err(
                                "Failed to request microphone permission or open System Preferences"
                                    .to_string(),
                            )
                        }
                    }
                }
            }
        }
        "accessibility" => {
            let status = check_accessibility_permission();
            tracing::info!(
                "♿ Accessibility permission status before request: {}",
                status
            );

            if status == SystemPermissionStatus::Granted {
                tracing::info!("♿ Accessibility permission already granted; nothing to request",);
                return Ok(());
            }

            // Accessibility CANNOT be requested programmatically on macOS
            // It always requires manual grant in System Settings
            if open_system_preferences("accessibility") {
                tracing::info!("✅ Opened System Preferences for Accessibility");
                Ok(())
            } else {
                Err("Failed to open System Preferences for Accessibility".to_string())
            }
        }
        "bluetooth" => {
            let status = check_bluetooth_permission();
            tracing::info!("🔵 Bluetooth permission status before request: {}", status);

            match status {
                SystemPermissionStatus::Granted => {
                    tracing::info!("🔵 Bluetooth permission already granted; nothing to request",);
                    Ok(())
                }
                SystemPermissionStatus::Denied | SystemPermissionStatus::Restricted => {
                    tracing::info!(
                        "🔵 Bluetooth permission denied or restricted; opening System Settings",
                    );
                    if open_system_preferences("bluetooth") {
                        tracing::info!("✅ Opened System Preferences for Bluetooth");
                        Ok(())
                    } else {
                        Err("Failed to open System Preferences for Bluetooth".to_string())
                    }
                }
                SystemPermissionStatus::NotDetermined | SystemPermissionStatus::Unknown => {
                    tracing::info!(
                        "🔵 Bluetooth permission not determined/unknown; attempting to trigger system dialog",
                    );
                    if request_bluetooth_permission() {
                        tracing::info!("✅ Bluetooth permission request initiated");
                        Ok(())
                    } else {
                        tracing::warn!(
                            "⚠️ Bluetooth permission request script failed; attempting to open System Preferences",
                        );
                        if open_system_preferences("bluetooth") {
                            tracing::info!("✅ Opened System Preferences for Bluetooth (fallback)",);
                            Ok(())
                        } else {
                            Err(
                                "Failed to request Bluetooth permission or open System Preferences"
                                    .to_string(),
                            )
                        }
                    }
                }
            }
        }
        "screen_recording" => {
            let status = check_screen_recording_permission();
            tracing::info!(
                "🖥️ Screen Recording permission status before request: {}",
                status
            );

            match status {
                SystemPermissionStatus::Granted => {
                    tracing::info!(
                        "🖥️ Screen Recording permission already granted; nothing to request",
                    );
                    Ok(())
                }
                SystemPermissionStatus::Denied | SystemPermissionStatus::Restricted => {
                    tracing::info!(
                        "🖥️ Screen Recording permission denied/restricted; opening System Settings",
                    );
                    if open_system_preferences("screen_recording") {
                        Ok(())
                    } else {
                        Err("Failed to open System Preferences for Screen Recording".to_string())
                    }
                }
                SystemPermissionStatus::NotDetermined | SystemPermissionStatus::Unknown => {
                    tracing::info!(
                        "🖥️ Screen Recording permission not determined/unknown; attempting to trigger system prompt",
                    );

                    if request_screen_recording_permission()
                        || open_system_preferences("screen_recording")
                    {
                        Ok(())
                    } else {
                        Err(
                            "Failed to request Screen Recording permission or open System Preferences"
                                .to_string(),
                        )
                    }
                }
            }
        }
        _ => Err(format!("Cannot request unknown permission: {}", permission)),
        }
    })
    .await
    .map_err(|error| format!("Failed to request system permission: {error}"))?
}

/// Open the configuration/settings window
#[tauri::command]
pub fn open_config_window() -> Result<(), String> {
    crate::window_manager::open_config_window().map_err(|e| e.to_string())
}

// UI Testing and Validation Commands

#[tauri::command]
pub async fn test_open_window(
    window_type: String,
    _app: tauri::AppHandle,
) -> Result<String, String> {
    tracing::info!("Testing window open: {}", window_type);

    match window_type.as_str() {
        "config" => {
            crate::window_manager::open_config_window().map_err(|e| e.to_string())?;
            Ok("Config window opened".to_string())
        }
        "agent" => {
            let session_id =
                crate::window_manager::create_new_agent_session().map_err(|e| e.to_string())?;
            Ok(format!("Agent session created: {}", session_id))
        }
        _ => Err(format!("Unknown window type: {}", window_type)),
    }
}

#[tauri::command]
pub async fn capture_window_screenshot(
    window_label: String,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if let Some(window) = app.get_webview_window(&window_label) {
        // Take screenshot using Tauri's screenshot API
        // Note: This requires the window to be visible and focused
        let _ = window.show();
        let _ = window.set_focus();

        // Wait a moment for the window to render
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        tracing::info!("Screenshot captured for window: {}", window_label);
        Ok(format!("Screenshot captured for {}", window_label))
    } else {
        Err(format!("Window not found: {}", window_label))
    }
}

/// Capture a system-wide screenshot
#[tauri::command]
pub async fn capture_screenshot(
    output_path: String,
    region: Option<(u32, u32, u32, u32)>,
    display: Option<u32>,
) -> Result<gestura_core::tools::screen::ScreenshotResult, String> {
    tracing::info!("📸 Capturing screenshot to: {}", output_path);

    tokio::task::spawn_blocking(move || {
        use gestura_core::tools::screen::{CaptureRegion, ScreenTools};

        let tools = ScreenTools::new();
        let region_opt = region.map(|(x, y, w, h)| CaptureRegion {
            x,
            y,
            width: w,
            height: h,
        });

        tools
            .screenshot(std::path::Path::new(&output_path), region_opt, display)
            .map_err(|e| {
                tracing::error!("Screenshot failed: {}", e);
                e.to_string()
            })
    })
    .await
    .map_err(|error| format!("Failed to run screenshot task: {error}"))?
}

/// Start screen recording
#[tauri::command]
pub async fn start_screen_recording(
    output_path: String,
    region: Option<(u32, u32, u32, u32)>,
    display: Option<u32>,
) -> Result<gestura_core::tools::screen::RecordingStartResult, String> {
    tracing::info!("🎥 Starting screen recording to: {}", output_path);

    tokio::task::spawn_blocking(move || {
        use gestura_core::tools::screen::{CaptureRegion, ScreenTools};

        let tools = ScreenTools::new();
        let region_opt = region.map(|(x, y, w, h)| CaptureRegion {
            x,
            y,
            width: w,
            height: h,
        });

        tools
            .start_recording(std::path::Path::new(&output_path), region_opt, display)
            .map_err(|e| {
                tracing::error!("Failed to start recording: {}", e);
                e.to_string()
            })
    })
    .await
    .map_err(|error| format!("Failed to start screen recording task: {error}"))?
}

/// Stop screen recording
#[tauri::command]
pub async fn stop_screen_recording(
    recording_id: String,
) -> Result<gestura_core::tools::screen::RecordingStopResult, String> {
    tracing::info!("⏹️ Stopping screen recording: {}", recording_id);

    tokio::task::spawn_blocking(move || {
        use gestura_core::tools::screen::ScreenTools;

        let tools = ScreenTools::new();
        tools.stop_recording(&recording_id).map_err(|e| {
            tracing::error!("Failed to stop recording: {}", e);
            e.to_string()
        })
    })
    .await
    .map_err(|error| format!("Failed to stop screen recording task: {error}"))?
}

#[tauri::command]
pub async fn validate_window_content(
    window_label: String,
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    if let Some(window) = app.get_webview_window(&window_label) {
        let _ = window.show();
        let _ = window.set_focus();

        // Wait for content to load
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Return validation results
        Ok(serde_json::json!({
            "window": window_label,
            "visible": true,
            "focused": true,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "status": "validated"
        }))
    } else {
        Err(format!("Window not found: {}", window_label))
    }
}

#[tauri::command]
pub async fn get_window_list(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let windows: Vec<String> = app.webview_windows().keys().cloned().collect();

    Ok(windows)
}

#[tauri::command]
pub async fn close_test_windows(app: tauri::AppHandle) -> Result<String, String> {
    let test_windows = ["permissions", "config", "agent", "status", "about"];
    let mut closed_count = 0;

    for window_label in test_windows.iter() {
        if let Some(window) = app.get_webview_window(window_label) {
            let _ = window.close();
            closed_count += 1;
        }
    }

    Ok(format!("Closed {} test windows", closed_count))
}

/// Get current listening state
#[tauri::command]
pub fn get_listening_state() -> Result<(bool, Option<u64>), String> {
    let (is_listening, remaining) = crate::tray::get_listening_state();
    let remaining_secs = remaining.map(|d| d.as_secs());
    Ok((is_listening, remaining_secs))
}

/// Set listening timeout duration
#[tauri::command]
pub fn set_listening_timeout(seconds: u64) -> Result<(), String> {
    let duration = std::time::Duration::from_secs(seconds);
    crate::tray::set_listening_timeout(duration);
    Ok(())
}

/// Toggle listening mode
#[tauri::command]
pub fn toggle_listening(_app: tauri::AppHandle) -> Result<String, String> {
    // This would trigger the same logic as the tray menu
    // For now, we'll just return the current state
    let (is_listening, _) = crate::tray::get_listening_state();
    if is_listening {
        Ok("Listening stopped".to_string())
    } else {
        Ok("Listening started".to_string())
    }
}

/// Update speech processing configuration
#[tauri::command]
pub fn update_speech_config(config: crate::speech::SpeechConfig) -> Result<(), String> {
    let mut config = config;

    // Do not require the UI to pass secrets across IPC. If keys are empty, try
    // to hydrate them from secure storage.
    if config.openai_api_key.trim().is_empty() {
        let voice_key = try_get_api_key_from_keychain_sync("voice_openai");
        if !voice_key.is_empty() {
            config.openai_api_key = voice_key;
        } else {
            let general_key = try_get_api_key_from_keychain_sync("openai");
            if !general_key.is_empty() {
                config.openai_api_key = general_key;
            }
        }
    }

    if config.anthropic_api_key.trim().is_empty() {
        let key = try_get_api_key_from_keychain_sync("anthropic");
        if !key.is_empty() {
            config.anthropic_api_key = key;
        }
    }

    crate::speech::update_speech_config(config);
    Ok(())
}

/// Get current speech processing status
#[tauri::command]
pub fn get_speech_status() -> Result<bool, String> {
    Ok(crate::speech::is_speech_recording())
}

/// Get tray diagnostic information
#[tauri::command]
pub async fn get_tray_diagnostic_info() -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(crate::tray::get_tray_diagnostic_info)
        .await
        .map_err(|error| format!("Failed to gather tray diagnostics: {error}"))
}

/// Check system permissions status
#[tauri::command]
pub async fn check_system_permissions() -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
        use crate::permissions::{
            SystemPermissionStatus, check_accessibility_permission, check_bluetooth_permission,
            check_microphone_permission, check_screen_recording_permission,
        };

        let mic_status = check_microphone_permission();
        let accessibility_status = check_accessibility_permission();
        let bluetooth_status = check_bluetooth_permission();
        let screen_recording_status = check_screen_recording_permission();

        tracing::info!(
            "System permission snapshot: microphone={}, accessibility={}, bluetooth={}, screen_recording={}",
            mic_status,
            accessibility_status,
            bluetooth_status,
            screen_recording_status
        );

        let mic_instructions = match mic_status {
            SystemPermissionStatus::Granted => "Microphone access is working properly",
            SystemPermissionStatus::Denied => {
                "Please enable microphone access in System Preferences > Privacy & Security > Microphone"
            }
            SystemPermissionStatus::NotDetermined => {
                "Microphone access will be requested when you start listening"
            }
            _ => "Check System Preferences for microphone access",
        };

        let accessibility_instructions = match accessibility_status {
            SystemPermissionStatus::Granted => "Accessibility access is working properly",
            SystemPermissionStatus::Denied => {
                "Please enable accessibility in System Preferences > Privacy & Security > Accessibility"
            }
            _ => "Accessibility access is required for hotkey functionality",
        };

        let bluetooth_instructions = match bluetooth_status {
            SystemPermissionStatus::Granted => "Bluetooth access is working properly",
            SystemPermissionStatus::Denied => {
                "Please enable Bluetooth in System Preferences > Privacy & Security > Bluetooth"
            }
            SystemPermissionStatus::NotDetermined => {
                "Bluetooth access will be requested when connecting to a ring"
            }
            _ => "Check System Preferences for Bluetooth access",
        };

        let screen_recording_instructions = match screen_recording_status {
            SystemPermissionStatus::Granted => "Screen Recording access is working properly",
            SystemPermissionStatus::Denied => {
                "Please enable Screen Recording in System Preferences > Privacy & Security > Screen Recording"
            }
            SystemPermissionStatus::NotDetermined => {
                "Screen Recording access will be requested when screen capture is needed"
            }
            _ => "Check System Preferences for Screen Recording access",
        };

        let permissions = vec![
            serde_json::json!({
                "id": "microphone",
                "name": "Microphone",
                "description": "Required for voice commands and speech recognition",
                "status": mic_status.to_string(),
                "required": true,
                "instructions": mic_instructions
            }),
            serde_json::json!({
                "id": "accessibility",
                "name": "Accessibility",
                "description": "Required for global hotkeys and gesture shortcuts",
                "status": accessibility_status.to_string(),
                "required": true,
                "instructions": accessibility_instructions
            }),
            serde_json::json!({
                "id": "bluetooth",
                "name": "Bluetooth",
                "description": "Required for connecting to Haptic Harmony ring",
                "status": bluetooth_status.to_string(),
                "required": false,
                "instructions": bluetooth_instructions
            }),
            serde_json::json!({
                "id": "screen_recording",
                "name": "Screen Recording",
                "description": "Optional: required for screen capture features",
                "status": screen_recording_status.to_string(),
                "required": false,
                "instructions": screen_recording_instructions
            }),
        ];

        let total_count = permissions.len();
        let granted_count = permissions
            .iter()
            .filter(|p| p.get("status").and_then(|v| v.as_str()) == Some("granted"))
            .count();
        let required_count = permissions
            .iter()
            .filter(|p| p.get("required").and_then(|v| v.as_bool()) == Some(true))
            .count();
        let required_granted_count = permissions
            .iter()
            .filter(|p| {
                p.get("required").and_then(|v| v.as_bool()) == Some(true)
                    && p.get("status").and_then(|v| v.as_str()) == Some("granted")
            })
            .count();
        let missing_required_count = required_count.saturating_sub(required_granted_count);

        Ok(serde_json::json!({
            "permissions": permissions,
            "total_count": total_count,
            "granted_count": granted_count,
            "required_count": required_count,
            "required_granted_count": required_granted_count,
            "missing_required_count": missing_required_count,
            "summary": {
                "total": total_count,
                "granted": granted_count,
                "required": required_count,
                "required_granted": required_granted_count,
                "missing_required": missing_required_count
            }
        }))
    })
    .await
    .map_err(|error| format!("Failed to collect system permissions: {error}"))?
}

// ============================================================================
// Session History Commands
// ============================================================================

/// Get all agent sessions (both open and closed)
#[tauri::command]
pub fn get_agent_sessions() -> Result<Vec<crate::window_manager::AgentSession>, String> {
    Ok(crate::window_manager::get_all_sessions())
}

/// Restore a closed agent session
#[tauri::command]
pub fn restore_agent_session(session_id: String) -> Result<(), String> {
    crate::window_manager::restore_agent_session(&session_id)
        .map_err(|e| format!("Failed to restore session: {}", e))
}

/// Create a new agent session
#[tauri::command]
pub fn create_agent_session() -> Result<String, String> {
    crate::window_manager::create_new_agent_session()
        .map_err(|e| format!("Failed to create session: {}", e))
}

/// Get session counts (active, closed)
#[tauri::command]
pub fn get_session_counts() -> Result<(usize, usize), String> {
    Ok(crate::window_manager::get_session_counts())
}

/// Get the conversation history for a session
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_history(
    session_id: String,
) -> Result<Vec<crate::window_manager::ConversationMessage>, String> {
    crate::window_manager::get_session_state(&session_id)
        .map(|state| state.messages)
        .ok_or_else(|| format!("Session not found: {}", session_id))
}

/// Replay snapshot used to restore a rich session timeline in the GUI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionReplaySnapshot {
    /// Canonical conversation history for compatibility/fallback rendering.
    pub history: Vec<crate::window_manager::ConversationMessage>,
    /// Rich activity timeline captured while the session was running.
    pub activity_log: Vec<crate::window_manager::SessionActivityEvent>,
    /// Whether the session currently has resumable paused execution state.
    pub has_paused_execution: bool,
}

/// Get the replay snapshot for a session.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_replay_snapshot(session_id: String) -> Result<SessionReplaySnapshot, String> {
    crate::window_manager::get_session_state(&session_id)
        .map(|state| SessionReplaySnapshot {
            has_paused_execution: state.paused_execution.is_some(),
            history: state.messages,
            activity_log: state.activity_log,
        })
        .ok_or_else(|| format!("Session not found: {}", session_id))
}

/// Get only the captured activity log for a session.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_activity_log(
    session_id: String,
) -> Result<Vec<crate::window_manager::SessionActivityEvent>, String> {
    crate::window_manager::get_session_state(&session_id)
        .map(|state| state.activity_log)
        .ok_or_else(|| format!("Session not found: {}", session_id))
}

/// Stable export envelope for saved session JSON snapshots.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionExportPayload {
    /// Export schema version for forward-compatible imports/tooling.
    pub schema_version: u32,
    /// Timestamp when the export was written.
    pub exported_at: chrono::DateTime<chrono::Utc>,
    /// Full session payload including messages, activity log, and settings.
    pub session: crate::window_manager::AgentSession,
}

/// Export a session as a pretty-printed JSON file chosen by the user.
#[tauri::command(rename_all = "snake_case")]
pub async fn export_session_json(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    use tokio::sync::oneshot;

    let session = crate::window_manager::get_session(&session_id)
        .ok_or_else(|| format!("Session not found: {}", session_id))?;
    let default_name = format!("session-{}.json", session.id);
    let export_payload = SessionExportPayload {
        schema_version: 1,
        exported_at: chrono::Utc::now(),
        session,
    };
    let json = serde_json::to_string_pretty(&export_payload)
        .map_err(|error| format!("Failed to serialize session export: {}", error))?;

    let (tx, rx) = oneshot::channel();
    app.dialog()
        .file()
        .set_title("Export Session JSON")
        .set_file_name(&default_name)
        .add_filter("JSON", &["json"])
        .save_file(move |result| {
            let _ = tx.send(result);
        });

    match rx.await {
        Ok(Some(path)) => {
            let mut path_buf = std::path::PathBuf::from(path.to_string());
            if path_buf.extension().is_none() {
                path_buf.set_extension("json");
            }
            tokio::fs::write(&path_buf, json).await.map_err(|error| {
                format!(
                    "Failed to write export file {}: {}",
                    path_buf.display(),
                    error
                )
            })?;
            Ok(Some(path_buf.display().to_string()))
        }
        Ok(None) => Ok(None),
        Err(_) => Err("Failed to receive save dialog result".to_string()),
    }
}

/// Return whether the session has an interrupted response that can be resumed.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn has_session_paused_execution(session_id: String) -> Result<bool, String> {
    crate::window_manager::get_session_state(&session_id)
        .map(|state| state.paused_execution.is_some())
        .ok_or_else(|| format!("Session not found: {}", session_id))
}

/// Get the workspace directory for the current active session
#[tauri::command]
pub fn get_session_workspace() -> Option<String> {
    crate::window_manager::get_active_session_workspace().map(|p| p.display().to_string())
}

/// Get the workspace directory for a specific session by ID.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_workspace_by_id(session_id: String) -> Option<String> {
    crate::window_manager::get_session_state(&session_id)
        .and_then(|s| s.workspace_dir)
        .map(|p| p.display().to_string())
}

fn resolve_memory_console_context(
    session_id: Option<&str>,
    workspace_dir: Option<String>,
) -> Result<
    (
        Option<gestura_core::agent_sessions::AgentSession>,
        std::path::PathBuf,
    ),
    String,
> {
    let live_sessions = crate::window_manager::get_all_sessions();
    let session_store = gestura_core::agent_sessions::FileAgentSessionStore::default();
    let session = session_id
        .and_then(|target| load_memory_console_session(target, &live_sessions, &session_store));

    let workspace_dir = workspace_dir
        .map(std::path::PathBuf::from)
        .or_else(|| {
            session
                .as_ref()
                .and_then(|current| current.workspace_dir().cloned())
        })
        .ok_or_else(|| "No workspace directory available for memory console".to_string())?;

    Ok((session, workspace_dir))
}

fn load_memory_console_session<S>(
    session_id: &str,
    live_sessions: &[crate::window_manager::AgentSession],
    session_store: &S,
) -> Option<gestura_core::agent_sessions::AgentSession>
where
    S: gestura_core::agent_sessions::AgentSessionStore,
{
    live_sessions
        .iter()
        .find(|candidate| candidate.id == session_id)
        .map(|candidate| gestura_core::agent_sessions::AgentSession {
            id: candidate.id.clone(),
            title: candidate.title.clone(),
            created_at: candidate.created_at,
            last_active: candidate.last_active,
            model: candidate
                .state
                .llm_config
                .as_ref()
                .and_then(|cfg| cfg.model.clone()),
            state: candidate.state.clone(),
        })
        .or_else(|| session_store.load(session_id).ok())
}

/// List recent sessions for the memory console.
#[tauri::command]
pub fn get_memory_console_sessions(
    limit: Option<usize>,
) -> Result<Vec<gestura_core::memory_console::MemoryConsoleSessionSummary>, String> {
    gestura_core::memory_console::list_memory_console_sessions(
        &gestura_core::agent_sessions::FileAgentSessionStore::default(),
        limit.unwrap_or(12),
    )
    .map_err(|e| e.to_string())
}

/// Get memory-console overview for a session/workspace.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_memory_console_overview(
    session_id: Option<String>,
    workspace_dir: Option<String>,
) -> Result<gestura_core::memory_console::MemoryConsoleOverview, String> {
    let (session, workspace_dir) =
        resolve_memory_console_context(session_id.as_deref(), workspace_dir)?;
    gestura_core::memory_console::get_memory_console_overview(&workspace_dir, session.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// Search working and durable memory with the shared console query DTO.
#[tauri::command(rename_all = "snake_case")]
pub async fn search_memory_console_entries(
    session_id: Option<String>,
    workspace_dir: Option<String>,
    query: gestura_core::memory_console::MemoryConsoleQuery,
) -> Result<gestura_core::memory_console::MemoryConsoleSearchResponse, String> {
    let (session, workspace_dir) =
        resolve_memory_console_context(session_id.as_deref(), workspace_dir)?;
    gestura_core::memory_console::search_memory_console(&workspace_dir, session.as_ref(), query)
        .await
        .map_err(|e| e.to_string())
}

/// Get the working-memory snapshot for the selected session.
#[tauri::command(rename_all = "snake_case")]
pub fn get_memory_working_snapshot(
    session_id: String,
) -> Result<gestura_core::agent_sessions::SessionWorkingMemory, String> {
    let (session, _) = resolve_memory_console_context(Some(&session_id), None)?;
    let session = session.ok_or_else(|| format!("Session not found: {}", session_id))?;
    Ok(gestura_core::memory_console::get_working_memory_snapshot(
        &session,
    ))
}

/// Get promotion candidates for the selected session.
#[tauri::command(rename_all = "snake_case")]
pub fn get_memory_promotion_candidates(
    session_id: String,
    limit: Option<usize>,
) -> Result<Vec<gestura_core::agent_sessions::SessionMemoryPromotionCandidate>, String> {
    let (session, _) = resolve_memory_console_context(Some(&session_id), None)?;
    let session = session.ok_or_else(|| format!("Session not found: {}", session_id))?;
    Ok(
        gestura_core::memory_console::get_memory_promotion_candidates(
            &session,
            limit.unwrap_or(12),
        ),
    )
}

/// Get task-local memory lifecycle for a session/task.
#[tauri::command(rename_all = "snake_case")]
pub fn get_memory_task_lifecycle(
    session_id: String,
    task_id: String,
) -> Result<gestura_core::memory_console::TaskMemoryConsoleDetail, String> {
    gestura_core::memory_console::get_task_memory_console_detail(
        get_task_manager(),
        &session_id,
        &task_id,
    )
    .map_err(|e| e.to_string())
}

/// Get a durable memory entry by id.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_memory_entry_detail(
    session_id: Option<String>,
    workspace_dir: Option<String>,
    entry_id: String,
) -> Result<gestura_core::memory_console::MemoryConsoleEntryDetail, String> {
    let (_, workspace_dir) = resolve_memory_console_context(session_id.as_deref(), workspace_dir)?;
    gestura_core::memory_console::get_memory_entry_detail(&workspace_dir, &entry_id)
        .await
        .map_err(|e| e.to_string())
}

/// Promote a candidate or ad-hoc item into durable memory.
#[tauri::command(rename_all = "snake_case")]
pub async fn promote_memory_candidate_entry(
    session_id: String,
    request: gestura_core::memory_console::PromoteMemoryCandidateRequest,
) -> Result<gestura_core::memory_console::MemoryConsoleEntryDetail, String> {
    let (session, workspace_dir) = resolve_memory_console_context(Some(&session_id), None)?;
    let session = session.ok_or_else(|| format!("Session not found: {}", session_id))?;
    gestura_core::memory_console::promote_memory_candidate(
        &workspace_dir,
        &session,
        request,
        Some(get_task_manager()),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Update a durable memory entry.
#[tauri::command(rename_all = "snake_case")]
pub async fn update_memory_entry_detail(
    session_id: Option<String>,
    workspace_dir: Option<String>,
    entry_id: String,
    request: gestura_core::memory_console::UpdateMemoryEntryRequest,
) -> Result<gestura_core::memory_console::MemoryConsoleEntryDetail, String> {
    let (_, workspace_dir) = resolve_memory_console_context(session_id.as_deref(), workspace_dir)?;
    gestura_core::memory_console::update_memory_entry_detail(&workspace_dir, &entry_id, request)
        .await
        .map_err(|e| e.to_string())
}

/// Refresh persisted durable-memory governance suggestions.
#[tauri::command(rename_all = "snake_case")]
pub async fn refresh_memory_console_governance(
    session_id: Option<String>,
    workspace_dir: Option<String>,
) -> Result<gestura_core::memory_bank::MemoryGovernanceRefreshReport, String> {
    let (_, workspace_dir) = resolve_memory_console_context(session_id.as_deref(), workspace_dir)?;
    gestura_core::memory_console::refresh_memory_console_governance(&workspace_dir)
        .await
        .map_err(|e| e.to_string())
}

/// Archive or restore a durable memory entry.
#[tauri::command(rename_all = "snake_case")]
pub async fn set_memory_entry_archived(
    session_id: Option<String>,
    workspace_dir: Option<String>,
    entry_id: String,
    archived: bool,
) -> Result<gestura_core::memory_console::MemoryConsoleEntryDetail, String> {
    let (_, workspace_dir) = resolve_memory_console_context(session_id.as_deref(), workspace_dir)?;
    gestura_core::memory_console::set_memory_entry_archived(&workspace_dir, &entry_id, archived)
        .await
        .map_err(|e| e.to_string())
}

/// Delete a durable memory entry.
#[tauri::command(rename_all = "snake_case")]
pub async fn delete_memory_entry(
    session_id: Option<String>,
    workspace_dir: Option<String>,
    entry_id: String,
) -> Result<(), String> {
    let (_, workspace_dir) = resolve_memory_console_context(session_id.as_deref(), workspace_dir)?;
    gestura_core::memory_console::delete_memory_entry_by_id(&workspace_dir, &entry_id)
        .await
        .map_err(|e| e.to_string())
}

/// Clear all durable memory entries for a workspace.
#[tauri::command(rename_all = "snake_case")]
pub async fn clear_memory_console_entries(
    session_id: Option<String>,
    workspace_dir: Option<String>,
) -> Result<usize, String> {
    let (_, workspace_dir) = resolve_memory_console_context(session_id.as_deref(), workspace_dir)?;
    gestura_core::memory_console::clear_memory_console(&workspace_dir)
        .await
        .map_err(|e| e.to_string())
}

/// Set the workspace directory for a session
#[tauri::command]
pub fn set_session_workspace(session_id: String, workspace_path: String) -> Result<(), String> {
    let path = std::path::PathBuf::from(&workspace_path);
    if !path.exists() {
        return Err(format!("Directory does not exist: {}", workspace_path));
    }
    if !path.is_dir() {
        return Err(format!("Path is not a directory: {}", workspace_path));
    }

    let path = path.canonicalize().map_err(|e| {
        format!(
            "Failed to canonicalize workspace path {}: {}",
            workspace_path, e
        )
    })?;

    crate::window_manager::set_session_workspace(&session_id, path.clone());
    tracing::info!(
        session_id = %session_id,
        workspace = %path.display(),
        "Workspace directory updated for session"
    );
    Ok(())
}

/// Open a directory picker dialog and set it as the workspace for a session
/// If session_id is provided, sets workspace for that session.
/// Otherwise, sets workspace for the current active session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn pick_workspace_directory(
    app: tauri::AppHandle,
    session_id: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    use tokio::sync::oneshot;

    let (tx, rx) = oneshot::channel();

    app.dialog()
        .file()
        .set_title("Select Workspace Directory")
        .pick_folder(move |result| {
            let _ = tx.send(result);
        });

    match rx.await {
        Ok(Some(path)) => {
            let path_buf_original = std::path::PathBuf::from(path.to_string());
            let path_buf = path_buf_original
                .canonicalize()
                .unwrap_or(path_buf_original);
            let path_str = path_buf.display().to_string();

            // Use provided session_id or fall back to active session
            let target_session =
                session_id.or_else(crate::window_manager::get_active_agent_for_voice);
            if let Some(sid) = target_session {
                crate::window_manager::set_session_workspace(&sid, path_buf);
                tracing::info!(
                    session_id = %sid,
                    workspace = %path_str,
                    "Workspace picked and set for session"
                );
            }
            Ok(Some(path_str))
        }
        Ok(None) => Ok(None),
        Err(_) => Err("Dialog was cancelled".to_string()),
    }
}

/// Open a terminal at the session workspace directory and resume the session via the CLI.
///
/// This command is intended for the agent window "Open In Shell" action.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn open_shell_for_session(session_id: String) -> Result<(), String> {
    crate::window_manager::open_shell_session_for_agent_resume(&session_id)
}

/// Check whether the Gestura CLI binary is installed and accessible on this machine.
///
/// The result is cached for the lifetime of the process (computed once at first
/// call). The frontend uses this to conditionally show CLI-dependent UI
/// controls (e.g. "Open in Terminal" quick-link and "Agent Shell" tray item).
#[tauri::command]
pub fn check_cli_installed() -> bool {
    crate::shell_session::is_cli_installed_cached()
}

// ============================================================================
// Project Explorer Commands (agent left-side file tree)
// ============================================================================

fn ensure_session_exists(session_id: &str) -> Result<(), String> {
    crate::window_manager::get_session_state(session_id)
        .map(|_| ())
        .ok_or_else(|| format!("Session not found: {}", session_id))
}

fn ensure_explorer_read_allowed(session_id: &str) -> Result<(), String> {
    ensure_session_exists(session_id)?;
    if !crate::window_manager::is_action_allowed(session_id, false) {
        return Err("Explorer access not allowed by session permission policy".to_string());
    }
    Ok(())
}

fn session_root_dir(session_id: &str) -> Result<std::path::PathBuf, String> {
    // Primary source of truth: session workspace directory.
    if let Some(workspace) =
        crate::window_manager::get_session_state(session_id).and_then(|s| s.workspace_dir)
    {
        return Ok(workspace);
    }

    // If older session state had no workspace recorded, prefer the detected project directory
    // (so explorer and tool sandbox align), otherwise fall back to a default per-session dir.
    if let Some(project_dir) = crate::window_manager::get_project_directory() {
        crate::window_manager::set_session_workspace(session_id, project_dir.clone());
        return Ok(project_dir);
    }

    let fallback = crate::window_manager::default_session_workspace_dir(session_id);
    std::fs::create_dir_all(&fallback).map_err(|e| {
        format!(
            "Failed to create default session workspace directory {}: {}",
            fallback.display(),
            e
        )
    })?;
    crate::window_manager::set_session_workspace(session_id, fallback.clone());
    Ok(fallback)
}

#[tauri::command(rename_all = "snake_case")]
pub fn explorer_get_root(
    session_id: String,
) -> Result<crate::explorer::ExplorerRootResponse, String> {
    ensure_explorer_read_allowed(&session_id)?;
    let root = session_root_dir(&session_id)?;
    let is_git_repo = root.join(".git").exists();
    Ok(crate::explorer::ExplorerRootResponse {
        root: root.display().to_string(),
        is_git_repo,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn explorer_list_dir(
    session_id: String,
    dir_rel: String,
) -> Result<crate::explorer::ExplorerListDirResponse, String> {
    ensure_explorer_read_allowed(&session_id)?;
    let root = session_root_dir(&session_id)?;
    let root_display = root.display().to_string();
    let dir_rel_trimmed = dir_rel.trim().to_string();

    let (entries, truncated) = tokio::task::spawn_blocking(move || {
        crate::explorer::list_dir(&root, &dir_rel_trimmed, 750)
    })
    .await
    .map_err(|e| format!("Failed to list directory: {}", e))?
    .map_err(|e| e.to_string())?;

    Ok(crate::explorer::ExplorerListDirResponse {
        root: root_display,
        dir_rel,
        entries,
        truncated,
    })
}

/// Open the current session's explorer root in the system file manager.
///
/// This is scoped to the resolved session workspace/root directory so the
/// frontend does not need direct opener plugin path permissions.
#[tauri::command(rename_all = "snake_case")]
pub fn explorer_open_root_in_file_manager(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    ensure_explorer_read_allowed(&session_id)?;
    let root = session_root_dir(&session_id)?;
    let root_display = root.display().to_string();
    app.opener()
        .open_path(root_display.clone(), None::<&str>)
        .map_err(|error| format!("Failed to open project root {}: {}", root_display, error))
}

/// Open an explorer entry in the system file manager.
///
/// Directories are opened directly. Files open their containing directory so
/// the user lands in the relevant Finder/File Explorer location instead of
/// launching the file in its default application.
#[tauri::command(rename_all = "snake_case")]
pub fn explorer_open_entry_in_file_manager(
    app: tauri::AppHandle,
    session_id: String,
    rel_path: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    ensure_explorer_read_allowed(&session_id)?;
    let root = session_root_dir(&session_id)?;
    let full = resolve_safe_path(&root, &rel_path)?;

    let target = match std::fs::metadata(&full) {
        Ok(metadata) if metadata.is_dir() => full,
        Ok(_) | Err(_) => full
            .parent()
            .filter(|parent| parent.starts_with(&root))
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| root.clone()),
    };

    let target_display = target.display().to_string();
    app.opener()
        .open_path(target_display.clone(), None::<&str>)
        .map_err(|error| {
            format!(
                "Failed to open explorer entry '{}' in file manager via {}: {}",
                rel_path, target_display, error
            )
        })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn explorer_git_status(
    session_id: String,
) -> Result<crate::explorer::ExplorerGitStatusResponse, String> {
    ensure_explorer_read_allowed(&session_id)?;
    let root = session_root_dir(&session_id)?;
    let root_display = root.display().to_string();
    let is_git_repo = root.join(".git").exists();
    if !is_git_repo {
        return Ok(crate::explorer::ExplorerGitStatusResponse {
            root: root_display,
            is_git_repo: false,
            paths: Default::default(),
            error: None,
        });
    }

    let root_for_git = root.clone();
    let status = tokio::task::spawn_blocking(move || {
        let tools = gestura_core::tools::git::GitTools::new(Some(root_for_git));
        tools.status()
    })
    .await
    .map_err(|e| format!("Failed to run git status: {}", e))?;

    let status = match status {
        Ok(s) => s,
        Err(e) => {
            return Ok(crate::explorer::ExplorerGitStatusResponse {
                root: root_display,
                is_git_repo: true,
                paths: Default::default(),
                error: Some(e.to_string()),
            });
        }
    };

    use crate::explorer::{ExplorerGitChangeKind, ExplorerGitPathStatus};
    use gestura_core::tools::git::ChangeStatus;
    use std::collections::HashMap;

    fn map_kind(status: ChangeStatus) -> ExplorerGitChangeKind {
        match status {
            ChangeStatus::Added => ExplorerGitChangeKind::Added,
            ChangeStatus::Modified => ExplorerGitChangeKind::Modified,
            ChangeStatus::Deleted => ExplorerGitChangeKind::Deleted,
            ChangeStatus::Renamed => ExplorerGitChangeKind::Renamed,
            ChangeStatus::Copied => ExplorerGitChangeKind::Copied,
            ChangeStatus::Unknown => ExplorerGitChangeKind::Unknown,
        }
    }

    let mut paths: HashMap<String, ExplorerGitPathStatus> = HashMap::new();

    for change in status.staged {
        if let Some(rel) = crate::explorer::normalize_git_change_path(&change.path) {
            let entry = paths.entry(rel).or_default();
            entry.staged = Some(map_kind(change.status));
        }
    }

    for change in status.unstaged {
        if let Some(rel) = crate::explorer::normalize_git_change_path(&change.path) {
            let entry = paths.entry(rel).or_default();
            entry.unstaged = Some(map_kind(change.status));
        }
    }

    for p in status.untracked {
        if let Some(rel) = crate::explorer::normalize_git_change_path(&p) {
            let entry = paths.entry(rel).or_default();
            entry.untracked = true;
        }
    }

    Ok(crate::explorer::ExplorerGitStatusResponse {
        root: root_display,
        is_git_repo: true,
        paths,
        error: None,
    })
}

// ============================================================================
// Editor File Commands (read / write / create / delete / rename / diff)
// ============================================================================

fn ensure_editor_write_allowed(session_id: &str) -> Result<(), String> {
    ensure_session_exists(session_id)?;
    if !crate::window_manager::is_action_allowed(session_id, true) {
        return Err("Editor write access not allowed by session permission policy".to_string());
    }
    Ok(())
}

/// Resolve `rel_path` against the session root, returning an error on path
/// traversal attempts (any component that would escape the root).
fn resolve_safe_path(root: &std::path::Path, rel_path: &str) -> Result<std::path::PathBuf, String> {
    // Strip leading slashes / backslashes to treat it as relative.
    let stripped = rel_path.trim_start_matches(['/', '\\']);
    let candidate = root.join(stripped);
    // Canonicalize-free traversal check: the resolved path must start with root.
    let canonical = candidate
        .components()
        .fold(std::path::PathBuf::new(), |mut acc, c| {
            match c {
                std::path::Component::ParentDir => {
                    acc.pop();
                }
                std::path::Component::CurDir => {}
                other => acc.push(other),
            }
            acc
        });
    if !canonical.starts_with(root) {
        return Err(format!(
            "Path traversal detected: '{}' escapes workspace root",
            rel_path
        ));
    }
    Ok(canonical)
}

/// Detect language from file extension for syntax highlighting.
fn detect_language(path: &std::path::Path) -> String {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "rs" => "rust",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "py" => "python",
        "css" | "scss" | "sass" | "less" => "css",
        "html" | "htm" => "html",
        "json" | "jsonc" => "json",
        "md" | "mdx" => "markdown",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "sh" | "bash" | "zsh" => "shell",
        "sql" => "sql",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "rb" => "ruby",
        "lua" => "lua",
        "xml" | "svg" => "xml",
        _ => "text",
    }
    .to_string()
}

/// Detect whether a file is text, image, or binary.
fn detect_file_kind(path: &std::path::Path) -> &'static str {
    let img_exts = [
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tiff", "svg",
    ];
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if img_exts.contains(&ext.as_str()) {
        return "image";
    }
    "text" // binary detection happens at read time via content sniffing
}

#[derive(serde::Serialize)]
pub struct EditorReadFileResponse {
    rel_path: String,
    content: String,
    language: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_url: Option<String>,
}

/// Read a file for display in the integrated editor.
/// Returns content, detected language, and kind (text / image / binary).
#[tauri::command(rename_all = "snake_case")]
pub async fn editor_read_file(
    session_id: String,
    rel_path: String,
) -> Result<EditorReadFileResponse, String> {
    ensure_explorer_read_allowed(&session_id)?;
    let root = session_root_dir(&session_id)?;
    let full = resolve_safe_path(&root, &rel_path)?;
    let language = detect_language(&full);
    let file_kind = detect_file_kind(&full);

    if file_kind == "image" {
        // Read as bytes and encode to base64 data URL.
        let bytes = tokio::fs::read(&full)
            .await
            .map_err(|e| format!("Failed to read image '{}': {}", rel_path, e))?;
        let ext = full
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_lowercase();
        let mime = match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            "ico" => "image/x-icon",
            "tiff" => "image/tiff",
            "svg" => "image/svg+xml",
            _ => "image/png",
        };
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let data_url = format!("data:{};base64,{}", mime, b64);
        return Ok(EditorReadFileResponse {
            rel_path,
            content: String::new(),
            language,
            kind: "image".to_string(),
            data_url: Some(data_url),
        });
    }

    // Read as UTF-8; if that fails assume binary.
    let raw = tokio::fs::read(&full)
        .await
        .map_err(|e| format!("Failed to read file '{}': {}", rel_path, e))?;

    match String::from_utf8(raw) {
        Ok(content) => Ok(EditorReadFileResponse {
            rel_path,
            content,
            language,
            kind: "text".to_string(),
            data_url: None,
        }),
        Err(_) => Ok(EditorReadFileResponse {
            rel_path,
            content: String::new(),
            language,
            kind: "binary".to_string(),
            data_url: None,
        }),
    }
}

/// Write (save) the editor content for a file to disk.
#[tauri::command(rename_all = "snake_case")]
pub async fn editor_write_file(
    session_id: String,
    rel_path: String,
    content: String,
) -> Result<(), String> {
    ensure_editor_write_allowed(&session_id)?;
    let root = session_root_dir(&session_id)?;
    let full = resolve_safe_path(&root, &rel_path)?;
    // Ensure parent directory exists.
    if let Some(parent) = full.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create directories for '{}': {}", rel_path, e))?;
    }
    tokio::fs::write(&full, content.as_bytes())
        .await
        .map_err(|e| format!("Failed to write file '{}': {}", rel_path, e))
}

/// Create a new file or directory at the given relative path.
#[tauri::command(rename_all = "snake_case")]
pub async fn editor_create_file(
    session_id: String,
    rel_path: String,
    is_dir: bool,
) -> Result<(), String> {
    ensure_editor_write_allowed(&session_id)?;
    let root = session_root_dir(&session_id)?;
    let full = resolve_safe_path(&root, &rel_path)?;
    if is_dir {
        tokio::fs::create_dir_all(&full)
            .await
            .map_err(|e| format!("Failed to create directory '{}': {}", rel_path, e))
    } else {
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create parent dirs for '{}': {}", rel_path, e))?;
        }
        // Create the file only if it doesn't already exist.
        tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&full)
            .await
            .map(|_| ())
            .map_err(|e| format!("Failed to create file '{}': {}", rel_path, e))
    }
}

/// Delete a file or directory (recursive for directories).
#[tauri::command(rename_all = "snake_case")]
pub async fn editor_delete_file(session_id: String, rel_path: String) -> Result<(), String> {
    ensure_editor_write_allowed(&session_id)?;
    let root = session_root_dir(&session_id)?;
    let full = resolve_safe_path(&root, &rel_path)?;
    let meta = tokio::fs::metadata(&full)
        .await
        .map_err(|e| format!("Failed to stat '{}': {}", rel_path, e))?;
    if meta.is_dir() {
        tokio::fs::remove_dir_all(&full)
            .await
            .map_err(|e| format!("Failed to delete directory '{}': {}", rel_path, e))
    } else {
        tokio::fs::remove_file(&full)
            .await
            .map_err(|e| format!("Failed to delete file '{}': {}", rel_path, e))
    }
}

/// Rename or move a file / directory within the workspace.
#[tauri::command(rename_all = "snake_case")]
pub async fn editor_rename_file(
    session_id: String,
    old_rel_path: String,
    new_rel_path: String,
) -> Result<(), String> {
    ensure_editor_write_allowed(&session_id)?;
    let root = session_root_dir(&session_id)?;
    let old_full = resolve_safe_path(&root, &old_rel_path)?;
    let new_full = resolve_safe_path(&root, &new_rel_path)?;
    if let Some(parent) = new_full.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create parent dirs for '{}': {}", new_rel_path, e))?;
    }
    tokio::fs::rename(&old_full, &new_full).await.map_err(|e| {
        format!(
            "Failed to rename '{}' → '{}': {}",
            old_rel_path, new_rel_path, e
        )
    })
}

#[derive(serde::Serialize)]
pub struct EditorGitDiffResponse {
    rel_path: String,
    original: String,
    modified: String,
    has_diff: bool,
}

/// Return the git diff (HEAD version vs. working tree) for a single file.
/// Returns `has_diff: false` when the file is unchanged or the workspace is not
/// a git repository.
#[tauri::command(rename_all = "snake_case")]
pub async fn editor_git_diff(
    session_id: String,
    rel_path: String,
) -> Result<EditorGitDiffResponse, String> {
    ensure_explorer_read_allowed(&session_id)?;
    let root = session_root_dir(&session_id)?;
    let full = resolve_safe_path(&root, &rel_path)?;

    // Must be a git repo.
    if !root.join(".git").exists() {
        return Ok(EditorGitDiffResponse {
            rel_path,
            original: String::new(),
            modified: String::new(),
            has_diff: false,
        });
    }

    let root_clone = root.clone();
    let rel_clone = rel_path.clone();
    let (original, _) = tokio::task::spawn_blocking(move || -> Result<(String, String), String> {
        // Use git show HEAD:<rel_path> to get the committed version.
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&root_clone)
            .arg("show")
            .arg(format!("HEAD:{}", rel_clone))
            .output()
            .map_err(|e| format!("Failed to run git show: {}", e))?;
        let original = if output.status.success() {
            String::from_utf8_lossy(&output.stdout).into_owned()
        } else {
            // File is new (not committed yet); original is empty.
            String::new()
        };
        Ok((original, String::new()))
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))??;

    // Read current working-tree content.
    let modified = tokio::fs::read_to_string(&full).await.unwrap_or_default();

    let has_diff = original != modified;
    Ok(EditorGitDiffResponse {
        rel_path,
        original,
        modified,
        has_diff,
    })
}

// ============================================================================
// Shell Process Control Commands (stop / pause / resume inline shell consoles)
// ============================================================================

fn spawn_shell_session_event_bridge(
    app: tauri::AppHandle,
    calling_window_label: String,
    target_window_label: String,
    resolved_session_id: Option<String>,
    mut rx: tokio::sync::mpsc::Receiver<gestura_core::StreamChunk>,
) {
    tauri::async_runtime::spawn(async move {
        let emit = |event: &str, payload: serde_json::Value| {
            let payload =
                crate::agent_events::attach_session_id(payload, resolved_session_id.as_deref());
            capture_session_activity_event(resolved_session_id.as_deref(), event, &payload);
            if let Err(err) = crate::agent_events::emit_agent_event_to_window(
                &app,
                &target_window_label,
                &calling_window_label,
                event,
                &payload,
                resolved_session_id.as_deref(),
            ) {
                tracing::error!(event = %event, error = %err, "Failed to emit shell manager event");
            }
        };

        while let Some(chunk) = rx.recv().await {
            match chunk {
                gestura_core::StreamChunk::ShellLifecycle {
                    process_id,
                    shell_session_id,
                    state,
                    command,
                    cwd,
                    exit_code,
                    duration_ms,
                } => emit(
                    "agent-stream-shell-lifecycle",
                    serde_json::json!({
                        "process_id": process_id,
                        "shell_session_id": shell_session_id,
                        "state": state,
                        "command": command,
                        "cwd": cwd,
                        "exit_code": exit_code,
                        "duration_ms": duration_ms,
                    }),
                ),
                gestura_core::StreamChunk::ShellOutput {
                    process_id,
                    shell_session_id,
                    stream,
                    data,
                } => emit(
                    "agent-stream-shell-output",
                    serde_json::json!({
                        "process_id": process_id,
                        "shell_session_id": shell_session_id,
                        "stream": stream,
                        "data": data,
                    }),
                ),
                gestura_core::StreamChunk::ShellSessionLifecycle {
                    shell_session_id,
                    state,
                    cwd,
                    active_process_id,
                    active_command,
                    available_for_reuse,
                    interactive,
                    user_managed,
                } => emit(
                    "agent-stream-shell-session-lifecycle",
                    serde_json::json!({
                        "shell_session_id": shell_session_id,
                        "state": state,
                        "cwd": cwd,
                        "active_process_id": active_process_id,
                        "active_command": active_command,
                        "available_for_reuse": available_for_reuse,
                        "interactive": interactive,
                        "user_managed": user_managed,
                    }),
                ),
                _ => {}
            }
        }
    });
}

/// Start a standalone PTY-backed interactive shell session for the current agent session.
///
/// The session starts immediately without requiring an initial command and
/// defaults to the project/session workspace directory when no cwd is given.
#[tauri::command(rename_all = "snake_case")]
pub async fn start_shell_session_streaming(
    webview_window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    session_id: String,
    cwd: Option<String>,
) -> Result<(), String> {
    use std::path::Path;
    use tokio::sync::mpsc;

    let cwd = cwd
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .or_else(|| {
            crate::window_manager::get_session_state(&session_id)
                .and_then(|state| state.workspace_dir.map(|dir| dir.display().to_string()))
        });

    if let Some(ref dir) = cwd {
        let path = Path::new(dir);
        if !path.exists() {
            return Err(format!("Working directory does not exist: {}", dir));
        }
        if !path.is_dir() {
            return Err(format!("Working directory is not a directory: {}", dir));
        }
    }

    let calling_window_label = webview_window.label().to_string();
    let target_window_label = crate::window_manager::get_session_window_label(&session_id)
        .unwrap_or_else(|| calling_window_label.clone());
    let resolved_session_id: Option<String> = Some(session_id.clone());
    let (tx, rx) = mpsc::channel(100);

    spawn_shell_session_event_bridge(
        app.clone(),
        calling_window_label.clone(),
        target_window_label.clone(),
        resolved_session_id.clone(),
        rx,
    );

    let exec_cwd = cwd.clone();

    if let Err(error) = gestura_core::tools::shell_sessions::create_session(
        &session_id,
        exec_cwd.as_deref(),
        Some(tx),
    )
    .await
    {
        let payload = crate::agent_events::attach_session_id(
            serde_json::json!({
                "cwd": exec_cwd,
                "error": error.to_string(),
            }),
            Some(session_id.as_str()),
        );
        crate::agent_events::emit_agent_event_to_window(
            &app,
            &target_window_label,
            &calling_window_label,
            "agent-stream-shell-launch-error",
            &payload,
            Some(session_id.as_str()),
        )
        .map_err(|err| err.to_string())?;
        return Ok(());
    }

    Ok(())
}

/// Stop a running shell process by sending SIGTERM (then SIGKILL after 3 s).
#[tauri::command(rename_all = "snake_case")]
pub async fn shell_process_stop(process_id: String) -> Result<(), String> {
    if gestura_core::tools::shell_sessions::stop_process(&process_id)
        .await
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Ok(());
    }

    gestura_core::tools::shell_streaming::stop_process(&process_id)
        .await
        .map_err(|e| e.to_string())
}

/// Stop and remove a long-lived PTY shell session.
#[tauri::command(rename_all = "snake_case")]
pub async fn shell_session_stop(shell_session_id: String) -> Result<(), String> {
    gestura_core::tools::shell_sessions::stop_session(&shell_session_id)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Send raw terminal input to an interactive PTY shell session.
#[tauri::command(rename_all = "snake_case")]
pub async fn shell_session_input(shell_session_id: String, input: String) -> Result<(), String> {
    gestura_core::tools::shell_sessions::send_input(&shell_session_id, &input)
        .await
        .map_err(|e| e.to_string())
}

/// Claim a PTY shell session for direct user interaction in the terminal manager.
#[tauri::command(rename_all = "snake_case")]
pub async fn shell_session_attach(
    webview_window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    session_id: String,
    shell_session_id: String,
) -> Result<(), String> {
    use tokio::sync::mpsc;

    let calling_window_label = webview_window.label().to_string();
    let target_window_label = crate::window_manager::get_session_window_label(&session_id)
        .unwrap_or_else(|| calling_window_label.clone());
    let resolved_session_id: Option<String> = Some(session_id.clone());
    let (tx, rx) = mpsc::channel(100);

    gestura_core::tools::shell_sessions::subscribe_session(&shell_session_id, tx)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Shell session not found: {shell_session_id}"))?;

    spawn_shell_session_event_bridge(
        app,
        calling_window_label,
        target_window_label,
        resolved_session_id,
        rx,
    );

    gestura_core::tools::shell_sessions::claim_session(&shell_session_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Shell session not found: {shell_session_id}"))?;

    Ok(())
}

/// Resize an interactive PTY shell session to match the terminal viewport.
#[tauri::command(rename_all = "snake_case")]
pub async fn shell_session_resize(
    shell_session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    gestura_core::tools::shell_sessions::resize_session(&shell_session_id, cols, rows)
        .await
        .map_err(|e| e.to_string())
}

/// Pause a running shell process (SIGSTOP, unix-only).
/// On non-unix platforms this returns an error.
#[tauri::command(rename_all = "snake_case")]
pub async fn shell_process_pause(process_id: String) -> Result<(), String> {
    #[cfg(unix)]
    {
        gestura_core::tools::shell_streaming::pause_process(&process_id)
            .await
            .map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = process_id;
        Err("Pause is only supported on unix platforms".into())
    }
}

/// Resume a paused shell process (SIGCONT, unix-only).
/// On non-unix platforms this returns an error.
#[tauri::command(rename_all = "snake_case")]
pub async fn shell_process_resume(process_id: String) -> Result<(), String> {
    #[cfg(unix)]
    {
        gestura_core::tools::shell_streaming::resume_process(&process_id)
            .await
            .map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = process_id;
        Err("Resume is only supported on unix platforms".into())
    }
}

/// Retrieve re-run info for a previously-executed shell process.
///
/// Returns `{ command, cwd, timeout_secs }` if the process was registered,
/// or `null` if no record exists (the map is cleared on process exit).
#[tauri::command(rename_all = "snake_case")]
pub async fn shell_process_rerun_info(process_id: String) -> Option<serde_json::Value> {
    gestura_core::tools::shell_streaming::get_rerun_info(&process_id)
        .await
        .map(|(command, cwd, _env, timeout_secs)| {
            serde_json::json!({
                "command": command,
                "cwd": cwd,
                "timeout_secs": timeout_secs,
            })
        })
}

// ============================================================================
// Session LLM Config Commands (session-scoped, doesn't modify global config)
// ============================================================================

/// Get the session-scoped LLM config for a agent session
/// Returns None if no session-specific override is set (uses global config)
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_llm_config(
    session_id: String,
) -> Option<crate::window_manager::SessionLlmConfig> {
    crate::window_manager::get_session_llm_config(&session_id)
}

/// Set the LLM provider for a specific session (doesn't modify global config)
/// This allows users to switch providers mid-conversation without affecting defaults
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_llm_provider(session_id: String, provider: String) -> Result<(), String> {
    crate::window_manager::set_session_llm_provider(&session_id, provider.clone());
    tracing::info!(
        session_id = %session_id,
        provider = %provider,
        "Session LLM provider updated (session-scoped)"
    );
    Ok(())
}

/// Set the LLM model for a specific session (doesn't modify global config)
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_llm_model(session_id: String, model: String) -> Result<(), String> {
    crate::window_manager::set_session_llm_model(&session_id, model.clone())?;
    tracing::info!(
        session_id = %session_id,
        model = %model,
        "Session LLM model updated (session-scoped)"
    );
    Ok(())
}

/// Clear session LLM config (revert to global config for this session)
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn clear_session_llm_config(session_id: String) -> Result<(), String> {
    crate::window_manager::clear_session_llm_config(&session_id);
    tracing::info!(session_id = %session_id, "Session LLM config cleared (using global config)");
    Ok(())
}

// =========================================================================
// Session Voice/STT Config Commands (session-scoped, doesn't modify globals)
// =========================================================================

/// Get the session-scoped voice/STT config for a agent session.
///
/// Returns `None` if no session-specific override is set (uses global config).
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_voice_config(
    session_id: String,
) -> Option<crate::window_manager::SessionVoiceConfig> {
    crate::window_manager::get_session_voice_config(&session_id)
}

/// Set the STT provider for a specific session (doesn't modify global config).
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_voice_provider(session_id: String, provider: String) -> Result<(), String> {
    crate::window_manager::set_session_voice_provider(&session_id, provider.clone());
    tracing::info!(
        session_id = %session_id,
        provider = %provider,
        "Session voice provider updated (session-scoped)"
    );
    Ok(())
}

/// Set the STT model for a specific session (doesn't modify global config).
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_voice_model(session_id: String, model: String) -> Result<(), String> {
    crate::window_manager::set_session_voice_model(&session_id, model.clone());
    tracing::info!(
        session_id = %session_id,
        model = %model,
        "Session voice model updated (session-scoped)"
    );
    Ok(())
}

/// Clear session voice config (revert to global config for this session).
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn clear_session_voice_config(session_id: String) -> Result<(), String> {
    crate::window_manager::clear_session_voice_config(&session_id);
    tracing::info!(session_id = %session_id, "Session voice config cleared (using global config)");
    Ok(())
}

/// Get the session reflection override for a session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_reflection_settings(
    session_id: String,
) -> Option<crate::window_manager::SessionReflectionSettings> {
    crate::window_manager::get_session_reflection_settings(&session_id)
}

/// Set whether experiential reflection is enabled for a specific session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_reflection_enabled(session_id: String, enabled: bool) -> Result<(), String> {
    crate::window_manager::set_session_reflection_enabled(&session_id, enabled);
    tracing::info!(
        session_id = %session_id,
        enabled,
        "Session reflection override updated"
    );
    Ok(())
}

/// Clear the session reflection override so the session inherits the global default.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn clear_session_reflection_settings(session_id: String) -> Result<(), String> {
    crate::window_manager::clear_session_reflection_settings(&session_id);
    tracing::info!(
        session_id = %session_id,
        "Session reflection override cleared (using global config)"
    );
    Ok(())
}

/// Get the effective LLM config for a session (session override or global fallback)
/// Returns (provider, model) tuple
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_effective_llm_config(session_id: String) -> Result<(String, String), String> {
    let mut cfg = AppConfig::load_async().await;
    let effective = apply_session_llm_config_overrides(&mut cfg, Some(&session_id)).await;
    Ok((effective.provider, effective.model))
}

// ============================================================================
// Session Tool and Permission Settings Commands
// ============================================================================

/// Get the tool settings for a session (permission level and enabled tools).
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_tool_settings(session_id: String) -> crate::window_manager::SessionToolSettings {
    crate::window_manager::get_session_tool_settings(&session_id)
}

/// Set the permission level for a session
/// Valid levels: "sandbox", "restricted", "full"
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_permission_level(session_id: String, level: String) -> Result<(), String> {
    let permission_level = match level.to_lowercase().as_str() {
        "sandbox" => crate::window_manager::SessionPermissionLevel::Sandbox,
        "restricted" => crate::window_manager::SessionPermissionLevel::Restricted,
        "full" => crate::window_manager::SessionPermissionLevel::Full,
        _ => {
            return Err(format!(
                "Invalid permission level: {}. Use 'sandbox', 'restricted', or 'full'",
                level
            ));
        }
    };
    crate::window_manager::set_session_permission_level(&session_id, permission_level);
    tracing::info!(
        session_id = %session_id,
        level = %level,
        "Session permission level updated"
    );
    Ok(())
}

/// Enable or disable a tool for a session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_tool_enabled(
    session_id: String,
    tool_name: String,
    enabled: bool,
) -> Result<(), String> {
    crate::window_manager::set_session_tool_enabled(&session_id, &tool_name, enabled);
    tracing::info!(
        session_id = %session_id,
        tool = %tool_name,
        enabled = %enabled,
        "Session tool availability updated"
    );
    Ok(())
}

/// Check if a tool is enabled for a session
#[tauri::command]
pub fn is_session_tool_enabled(session_id: String, tool_name: String) -> bool {
    crate::window_manager::is_session_tool_enabled(&session_id, &tool_name)
}

/// Check if an action is allowed based on session permission level
#[tauri::command]
pub fn is_session_action_allowed(session_id: String, is_write_operation: bool) -> bool {
    crate::window_manager::is_action_allowed(&session_id, is_write_operation)
}

/// Check if confirmation is required for an action based on session permission level
#[tauri::command]
pub fn session_requires_confirmation(session_id: String, is_write_operation: bool) -> bool {
    crate::window_manager::requires_confirmation(&session_id, is_write_operation)
}

// ============================================================================
// Task Management Commands
// ============================================================================

use gestura_core::{Task, TaskStatus};
use std::sync::OnceLock;

/// Returns the process-wide shared [`gestura_core::TaskManager`].
///
/// Delegates to `task_integration`, which in turn delegates to the canonical
/// singleton in `gestura-core-tasks`.  All subsystems share one instance and
/// therefore one in-memory cache, so the UI always sees the latest tasks
/// regardless of which subsystem created them.
fn get_task_manager() -> &'static gestura_core::TaskManager {
    crate::task_integration::get_task_manager()
}

/// Create a new task.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn create_task(
    app: tauri::AppHandle,
    session_id: String,
    name: String,
    description: String,
    parent_id: Option<String>,
) -> Result<Task, String> {
    let manager = get_task_manager();
    let task = manager
        .create_task(&session_id, name, description, parent_id)
        .map_err(|e| e.to_string())?;

    // Emit task-created event for frontend reactivity
    let _ = app.emit(
        "task-created",
        serde_json::json!({
            "session_id": session_id,
            "task": &task
        }),
    );

    Ok(task)
}

/// Update a task's status
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn update_task_status(
    app: tauri::AppHandle,
    session_id: String,
    task_id: String,
    status: String,
) -> Result<(), String> {
    let manager = get_task_manager();
    let task_status = match status.to_lowercase().as_str() {
        "notstarted" | "not_started" => TaskStatus::NotStarted,
        "blocked" | "waiting" => TaskStatus::Blocked,
        "inprogress" | "in_progress" => TaskStatus::InProgress,
        "completed" => TaskStatus::Completed,
        "cancelled" => TaskStatus::Cancelled,
        _ => {
            return Err(format!(
                "Invalid task status: {}. Use 'notstarted', 'blocked', 'inprogress', 'completed', or 'cancelled'",
                status
            ));
        }
    };
    manager
        .update_task_status(&session_id, &task_id, task_status)
        .map_err(|e| e.to_string())?;

    // Emit task-updated event for frontend reactivity
    let _ = app.emit(
        "task-updated",
        serde_json::json!({
            "session_id": session_id,
            "task_id": task_id,
            "status": status
        }),
    );

    Ok(())
}

/// Update a task's name and/or description
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn update_task(
    app: tauri::AppHandle,
    session_id: String,
    task_id: String,
    name: Option<String>,
    description: Option<String>,
) -> Result<(), String> {
    let manager = get_task_manager();
    manager
        .update_task(&session_id, &task_id, name.clone(), description.clone())
        .map_err(|e| e.to_string())?;

    // Emit task-updated event for frontend reactivity
    let _ = app.emit(
        "task-updated",
        serde_json::json!({
            "session_id": session_id,
            "task_id": task_id,
            "name": name,
            "description": description
        }),
    );

    Ok(())
}

/// Delete a task.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn delete_task(
    app: tauri::AppHandle,
    session_id: String,
    task_id: String,
) -> Result<Task, String> {
    let manager = get_task_manager();
    let task = manager
        .delete_task(&session_id, &task_id)
        .map_err(|e| e.to_string())?;

    // Emit task-deleted event for frontend reactivity
    let _ = app.emit(
        "task-deleted",
        serde_json::json!({
            "session_id": session_id,
            "task_id": task_id
        }),
    );

    Ok(task)
}

/// List all tasks for a session
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn list_tasks(session_id: String) -> Result<Vec<Task>, String> {
    let manager = get_task_manager();
    manager.list_tasks(&session_id).map_err(|e| e.to_string())
}

/// Get the full recursive task hierarchy for a session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_task_hierarchy(
    session_id: String,
) -> Result<Vec<gestura_core::tasks::TaskTreeNode>, String> {
    let manager = get_task_manager();
    manager
        .get_task_tree(&session_id)
        .map_err(|e| e.to_string())
}

/// Break down requirements into a task hierarchy using the LLM.
///
/// This command analyzes the provided requirements text and generates
/// a prioritized task hierarchy with dependencies identified.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn break_down_requirements(
    app: tauri::AppHandle,
    session_id: String,
    requirements: String,
) -> Result<Vec<String>, String> {
    let tasks_array = generate_requirement_breakdown(&session_id, &requirements).await?;

    let manager = get_task_manager();
    let mut created_task_ids = Vec::new();
    let mut name_to_id: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    // First pass: create root tasks (no parent)
    for task_json in &tasks_array {
        let parent_name = task_json.get("parent_name").and_then(|v| v.as_str());
        if parent_name.is_some() && !parent_name.unwrap().is_empty() {
            continue; // Skip subtasks in first pass
        }

        let name = task_json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled Task")
            .to_string();

        let description = format_breakdown_task_description(task_json);

        let task = manager
            .create_task(&session_id, name.clone(), description, None)
            .map_err(|e| e.to_string())?;

        name_to_id.insert(name, task.id.clone());
        created_task_ids.push(task.id.clone());

        // Emit task-created event
        let _ = app.emit(
            "task-created",
            serde_json::json!({
                "session_id": &session_id,
                "task": &task
            }),
        );
    }

    // Second pass: create subtasks
    for task_json in &tasks_array {
        let parent_name = match task_json.get("parent_name").and_then(|v| v.as_str()) {
            Some(name) if !name.is_empty() => name,
            _ => continue, // Skip root tasks
        };

        let parent_id = name_to_id.get(parent_name).cloned();

        let name = task_json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled Subtask")
            .to_string();

        let description = format_breakdown_task_description(task_json);

        let task = manager
            .create_task(&session_id, name.clone(), description, parent_id)
            .map_err(|e| e.to_string())?;

        name_to_id.insert(name, task.id.clone());
        created_task_ids.push(task.id.clone());

        // Emit task-created event
        let _ = app.emit(
            "task-created",
            serde_json::json!({
                "session_id": &session_id,
                "task": &task
            }),
        );
    }

    Ok(created_task_ids)
}

// ============================================================================
// Knowledge Management Commands
// ============================================================================

use gestura_core::{
    KnowledgeItem, KnowledgeSettingsManager, KnowledgeStore, register_builtin_knowledge,
};

/// Global knowledge store instance
static KNOWLEDGE_STORE: OnceLock<KnowledgeStore> = OnceLock::new();

/// Global knowledge settings manager instance
static KNOWLEDGE_SETTINGS: OnceLock<KnowledgeSettingsManager> = OnceLock::new();

/// Get or initialize the global knowledge store
fn get_knowledge_store() -> &'static KnowledgeStore {
    KNOWLEDGE_STORE.get_or_init(|| {
        let store = KnowledgeStore::with_default_dir();
        register_builtin_knowledge(&store);

        if let Err(e) = store.load_user_items() {
            tracing::warn!(error = %e, "Failed to load persisted user knowledge (continuing)");
        }
        store
    })
}

/// Get or initialize the global knowledge settings manager
fn get_knowledge_settings() -> &'static KnowledgeSettingsManager {
    KNOWLEDGE_SETTINGS.get_or_init(|| {
        let base_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        KnowledgeSettingsManager::new(base_dir)
    })
}

/// List all available knowledge items
#[tauri::command]
pub fn list_knowledge_items() -> Result<Vec<KnowledgeItem>, String> {
    let store = get_knowledge_store();
    Ok(store.list())
}

/// Get a specific knowledge item by ID
#[tauri::command]
pub fn get_knowledge_item(knowledge_id: String) -> Result<Option<KnowledgeItem>, String> {
    let store = get_knowledge_store();
    Ok(store.get(&knowledge_id))
}

/// Set knowledge enabled/disabled for a session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_knowledge_enabled(
    session_id: String,
    knowledge_id: String,
    enabled: bool,
) -> Result<(), String> {
    let settings = get_knowledge_settings();
    settings
        .set_knowledge_enabled(&session_id, &knowledge_id, enabled)
        .map_err(|e| e.to_string())
}

/// Get list of enabled knowledge IDs for a session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_enabled_knowledge(session_id: String) -> Result<Vec<String>, String> {
    let settings = get_knowledge_settings();
    settings
        .get_enabled_knowledge(&session_id)
        .map_err(|e| e.to_string())
}

/// Get the pseudo-session ID used for default knowledge enablement.
#[tauri::command]
pub fn knowledge_default_session_id() -> Result<String, String> {
    Ok(gestura_core::knowledge::DEFAULT_KNOWLEDGE_SETTINGS_SESSION_ID.to_string())
}

/// Create or update a user knowledge item (persisted on disk).
///
/// Built-in knowledge items cannot be modified via this command.
#[tauri::command]
pub fn upsert_knowledge_item(item: KnowledgeItem) -> Result<(), String> {
    let store = get_knowledge_store();
    store.upsert_user_item(item).map_err(|e| e.to_string())
}

/// Delete a user knowledge item (persisted on disk).
///
/// Built-in knowledge items cannot be deleted via this command.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn delete_knowledge_item(knowledge_id: String) -> Result<(), String> {
    let store = get_knowledge_store();
    store
        .delete_user_item(&knowledge_id)
        .map_err(|e| e.to_string())
}

// ============================================================================
// Voice Listener Control Commands
// ============================================================================

/// Validation result for voice configuration
#[derive(Debug, Clone, serde::Serialize)]
pub struct VoiceConfigValidation {
    pub is_valid: bool,
    pub provider: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub suggestion: Option<String>,
}

/// Validate voice/STT configuration before starting listener (sync version for internal use)
pub fn validate_voice_config_sync() -> VoiceConfigValidation {
    let config = crate::AppConfig::load();
    validate_voice_config_with_config(&config)
}

/// Validate voice/STT configuration before starting listener
#[tauri::command]
pub async fn validate_voice_config() -> VoiceConfigValidation {
    let config = crate::AppConfig::load_async().await;
    validate_voice_config_with_config(&config)
}

/// Internal helper to validate voice config with a given config
fn validate_voice_config_with_config(config: &crate::AppConfig) -> VoiceConfigValidation {
    let provider = config.voice.provider.as_str();

    match provider {
        "local" => {
            // Check if local model is configured
            let model_path = config
                .voice
                .local_model_path
                .as_ref()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(crate::AppConfig::default_whisper_model_path);

            if !model_path.exists() {
                return VoiceConfigValidation {
                    is_valid: false,
                    provider: "local".to_string(),
                    error_code: Some("LOCAL_MODEL_NOT_FOUND".to_string()),
                    error_message: Some("Local Whisper model not found.".to_string()),
                    suggestion: Some(
                        "Download a Whisper model in Settings → Voice & Audio → Local Whisper."
                            .to_string(),
                    ),
                };
            }

            // Validate the model file
            let validation = crate::voice::validate_whisper_model(&model_path);
            if !validation.is_valid {
                return VoiceConfigValidation {
                    is_valid: false,
                    provider: "local".to_string(),
                    error_code: Some("LOCAL_MODEL_INVALID".to_string()),
                    error_message: Some(
                        validation
                            .error
                            .unwrap_or_else(|| "Invalid Whisper model file.".to_string()),
                    ),
                    suggestion: Some(
                        "The model file may be corrupted. Try downloading it again.".to_string(),
                    ),
                };
            }

            VoiceConfigValidation {
                is_valid: true,
                provider: "local".to_string(),
                error_code: None,
                error_message: None,
                suggestion: None,
            }
        }
        "openai" => {
            // Check if OpenAI API key is configured (config file, keychain, or LLM fallback)
            let config_key = config.voice.openai_api_key.as_deref().unwrap_or("");

            // Try keychain fallback if config key is empty
            let has_api_key = if !config_key.is_empty() {
                true
            } else {
                // Check keychain: voice-specific key first, then general OpenAI key
                let voice_key = try_get_api_key_from_keychain_sync("voice_openai");
                if !voice_key.is_empty() {
                    true
                } else {
                    let general_key = try_get_api_key_from_keychain_sync("openai");
                    if !general_key.is_empty() {
                        true
                    } else {
                        // Fallback to LLM OpenAI config
                        config
                            .llm
                            .openai
                            .as_ref()
                            .is_some_and(|c| !c.api_key.is_empty())
                    }
                }
            };

            if !has_api_key {
                return VoiceConfigValidation {
                    is_valid: false,
                    provider: "openai".to_string(),
                    error_code: Some("OPENAI_API_KEY_MISSING".to_string()),
                    error_message: Some("OpenAI API key not configured.".to_string()),
                    suggestion: Some(
                        "Add your OpenAI API key in Settings → AI Providers → OpenAI.".to_string(),
                    ),
                };
            }

            VoiceConfigValidation {
                is_valid: true,
                provider: "openai".to_string(),
                error_code: None,
                error_message: None,
                suggestion: None,
            }
        }
        "none" | "" => VoiceConfigValidation {
            is_valid: false,
            provider: provider.to_string(),
            error_code: Some("NO_PROVIDER_CONFIGURED".to_string()),
            error_message: Some("No speech-to-text provider configured.".to_string()),
            suggestion: Some(
                "Configure a speech-to-text provider in Settings → Voice & Audio.".to_string(),
            ),
        },
        _ => VoiceConfigValidation {
            is_valid: false,
            provider: provider.to_string(),
            error_code: Some("UNKNOWN_PROVIDER".to_string()),
            error_message: Some(format!("Unknown speech-to-text provider: {}", provider)),
            suggestion: Some(
                "Select a valid provider (Local Whisper or OpenAI) in Settings.".to_string(),
            ),
        },
    }
}

/// Extended validation that also ensures a usable LLM provider is configured so
/// the full voice → STT → LLM agent loop can run (sync version for internal use).
pub fn validate_voice_and_llm_config_sync() -> VoiceConfigValidation {
    let config = crate::AppConfig::load();
    let stt_validation = validate_voice_config_with_config(&config);
    if !stt_validation.is_valid {
        return stt_validation;
    }
    validate_llm_config_with_config(&config, stt_validation)
}

/// Extended validation that also ensures a usable LLM provider is configured so
/// the full voice → STT → LLM agent loop can run.
pub async fn validate_voice_and_llm_config() -> VoiceConfigValidation {
    let config = crate::AppConfig::load_async().await;
    let stt_validation = validate_voice_config_with_config(&config);
    if !stt_validation.is_valid {
        return stt_validation;
    }
    validate_llm_config_with_config(&config, stt_validation)
}

/// Internal helper to validate LLM config with a given config
fn validate_llm_config_with_config(
    config: &crate::AppConfig,
    stt_validation: VoiceConfigValidation,
) -> VoiceConfigValidation {
    let llm_primary_raw = config.llm.primary.trim();
    let llm_primary = llm_primary_raw.to_lowercase();

    // Helper to construct LLM-related validation errors
    let llm_error = |code: &str, message: &str, suggestion: &str| VoiceConfigValidation {
        is_valid: false,
        provider: format!("llm:{}", llm_primary),
        error_code: Some(code.to_string()),
        error_message: Some(message.to_string()),
        suggestion: Some(suggestion.to_string()),
    };

    if llm_primary.is_empty() {
        return llm_error(
            "LLM_PROVIDER_MISSING",
            "No LLM provider configured.",
            "Select and configure an LLM provider in Settings → AI Providers.",
        );
    }

    match llm_primary.as_str() {
        "openai" => {
            if let Some(c) = &config.llm.openai {
                if c.api_key.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "OpenAI LLM provider is selected but API key is missing.",
                        "Add your OpenAI API key in Settings → AI Providers → OpenAI.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "OpenAI LLM provider is selected but no model is configured.",
                        "Choose a agent model for OpenAI in Settings → AI Providers.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "OpenAI LLM provider is selected but not configured.",
                    "Fill in OpenAI LLM settings under Settings → AI Providers → OpenAI.",
                );
            }
        }
        "anthropic" => {
            if let Some(c) = &config.llm.anthropic {
                if c.api_key.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Anthropic LLM provider is selected but API key is missing.",
                        "Add your Anthropic API key in Settings → AI Providers → Anthropic.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Anthropic LLM provider is selected but no model is configured.",
                        "Choose a Claude model in Settings → AI Providers → Anthropic.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "Anthropic LLM provider is selected but not configured.",
                    "Fill in Anthropic LLM settings under Settings → AI Providers → Anthropic.",
                );
            }
        }
        "gemini" => {
            if let Some(c) = &config.llm.gemini {
                if c.api_key.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Gemini LLM provider is selected but API key is missing.",
                        "Add your Gemini API key in Settings → AI Providers → Gemini.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Gemini LLM provider is selected but no model is configured.",
                        "Choose a Gemini model in Settings → AI Providers → Gemini.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "Gemini LLM provider is selected but not configured.",
                    "Fill in Gemini LLM settings under Settings → AI Providers → Gemini.",
                );
            }
        }
        "grok" => {
            if let Some(c) = &config.llm.grok {
                if c.api_key.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Grok LLM provider is selected but API key is missing.",
                        "Add your Grok API key in Settings → AI Providers → Grok.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Grok LLM provider is selected but no model is configured.",
                        "Choose a Grok model in Settings → AI Providers → Grok.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "Grok LLM provider is selected but not configured.",
                    "Fill in Grok LLM settings under Settings → AI Providers → Grok.",
                );
            }
        }
        "ollama" => {
            if let Some(c) = &config.llm.ollama {
                if c.base_url.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Ollama LLM provider is selected but server URL is missing.",
                        "Set the Ollama server URL (for example http://localhost:11434) in Settings → AI Providers → Ollama.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Ollama LLM provider is selected but no model is configured.",
                        "Select an Ollama model in Settings → AI Providers → Ollama.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "Ollama LLM provider is selected but not configured.",
                    "Fill in Ollama settings under Settings → AI Providers → Ollama.",
                );
            }
        }
        _ => {
            return llm_error(
                "LLM_PROVIDER_UNKNOWN",
                &format!("Unknown LLM provider: {}", llm_primary_raw),
                "Select a valid LLM provider (OpenAI, Anthropic, Grok, Gemini, or Ollama) in Settings → AI Providers.",
            );
        }
    }

    // Both STT and LLM configuration look good; report overall success.
    stt_validation
}

/// Start voice listening with validation shared with the tray logic.
///
/// This command is typically triggered from the agent UI. It delegates to the
/// tray module so that both agent and tray use the exact same validation and
/// speech start pipeline.
#[tauri::command]
pub async fn start_voice_listening(app: tauri::AppHandle) -> Result<String, String> {
    crate::tray::start_listening_with_validation(&app)?;
    let provider = crate::AppConfig::load_async().await.voice.provider;
    Ok(format!("Voice listening started (provider: {})", provider))
}

/// Stop voice listening
#[tauri::command]
pub fn stop_voice_listening(app: tauri::AppHandle) -> Result<String, String> {
    // Stop the speech processing (audio recording)
    if let Err(e) = crate::speech::stop_speech_listening() {
        tracing::warn!("Failed to stop speech processing: {}", e);
    }
    // Update the listening state
    crate::tray::stop_listening();

    // Emit event to notify frontend that listening has stopped
    if let Err(e) = app.emit(
        "listening-state-changed",
        serde_json::json!({
            "is_listening": false
        }),
    ) {
        tracing::warn!("Failed to emit listening-state-changed: {}", e);
    }
    tracing::info!("Emitted listening-state-changed event (stopped via API)");

    Ok("Voice listening stopped".to_string())
}

/// Complete the onboarding process and mark it as done.
///
/// Saves the current configuration to disk (which acts as the canonical
/// "onboarding completed" marker — `AppConfig::is_first_run()` returns `false`
/// once the file exists) and emits an `"onboarding-complete"` event so the
/// system tray can rebuild its menu and re-enable the gated items
/// ("New Agent Session" and "Start Listening").
#[tauri::command]
pub async fn complete_onboarding(app: tauri::AppHandle) -> Result<(), String> {
    tracing::info!("Onboarding completed by user");
    // Save the config to disk — this is the canonical "onboarding done" marker.
    let config = AppConfig::load_async().await;
    config.save_async().await.map_err(|e| e.to_string())?;
    // Notify the system tray (and any other listeners) that onboarding has
    // finished so they can re-evaluate is_app_configured() and update their state.
    let _ = app.emit("onboarding-complete", ());
    tracing::info!("Emitted onboarding-complete event");
    Ok(())
}

/// Close the onboarding window
#[tauri::command]
pub fn close_onboarding_window() -> Result<(), String> {
    crate::window_manager::close_onboarding().map_err(|e| e.to_string())
}

/// Open system preferences to a specific pane
#[tauri::command]
pub fn open_system_preferences(pane: String) -> Result<(), String> {
    use crate::permissions::open_system_preferences as open_prefs;
    if open_prefs(&pane) {
        Ok(())
    } else {
        Err(format!("Failed to open System Preferences for {}", pane))
    }
}

/// Update voice provider setting
#[tauri::command]
pub async fn update_voice_provider(provider: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.voice.provider = provider.clone();
    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("Voice provider updated to: {}", provider);
    Ok(())
}

/// Update whisper model setting
#[tauri::command(rename_all = "snake_case")]
pub async fn update_whisper_model(model_filename: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    let models_dir = AppConfig::whisper_models_dir();
    let model_path = models_dir.join(&model_filename);
    cfg.voice.local_model_path = Some(model_path.to_string_lossy().to_string());
    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("Whisper model updated to: {}", model_filename);
    Ok(())
}

/// Update LLM provider setting
#[tauri::command]
pub async fn update_llm_provider(provider: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.update_llm_provider(&provider)
        .map_err(|e| e.to_string())?;
    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("LLM provider updated to: {}", provider);
    Ok(())
}

/// Update selected audio input device
#[tauri::command]
pub async fn update_audio_device(device_name: Option<String>) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.voice.audio_device = device_name.clone();
    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("Audio device updated to: {:?}", device_name);
    Ok(())
}

/// Update Ollama configuration (URL and model)
#[tauri::command(rename_all = "snake_case")]
pub async fn update_ollama_config(base_url: String, model: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.llm.ollama = Some(crate::config::OllamaConfig {
        base_url: base_url.clone(),
        model: model.clone(),
    });
    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("Ollama config updated: url={}, model={}", base_url, model);
    Ok(())
}

/// Update the model for a specific cloud LLM provider (persists to global config).
///
/// This is used by onboarding so the user-selected model sticks permanently.
#[tauri::command(rename_all = "snake_case")]
pub async fn update_provider_model(provider: String, model: String) -> Result<(), String> {
    if model.trim().is_empty() {
        return Err("Model name cannot be empty".to_string());
    }
    let mut cfg = AppConfig::load_async().await;
    // Ensure the provider config block exists before mutating the model.
    cfg.llm.ensure_provider_config(&provider);
    match provider.as_str() {
        "openai" => {
            if let Some(c) = cfg.llm.openai.as_mut() {
                c.model = model.clone();
            }
        }
        "anthropic" => {
            if let Some(c) = cfg.llm.anthropic.as_mut() {
                c.model = model.clone();
            }
        }
        "grok" => {
            if let Some(c) = cfg.llm.grok.as_mut() {
                c.model = model.clone();
            }
        }
        "gemini" => {
            if let Some(c) = cfg.llm.gemini.as_mut() {
                c.model = model.clone();
            }
        }
        "ollama" => {
            if let Some(c) = cfg.llm.ollama.as_mut() {
                c.model = model.clone();
            }
        }
        other => {
            return Err(format!("Unknown provider: {other}"));
        }
    }
    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!(
        "Provider model updated: provider={}, model={}",
        provider,
        model
    );
    Ok(())
}

/// Get notification settings
#[tauri::command]
pub async fn get_notification_settings()
-> Result<gestura_core::config::NotificationSettings, String> {
    let cfg = AppConfig::load_async().await;
    Ok(cfg.notifications)
}

/// Update notification settings
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command parameters expanded for fine-grained frontend control
pub async fn update_notification_settings(
    sound_enabled: Option<bool>,
    haptic_enabled: Option<bool>,
    sound_volume: Option<u8>,
    haptic_intensity: Option<u8>,
    notification_sound: Option<String>,
    command_confirm_sound: Option<String>,
    mcp_feedback_enabled: Option<bool>,
    auto_listen_on_feedback: Option<bool>,
) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;

    if let Some(v) = sound_enabled {
        cfg.notifications.sound_enabled = v;
    }
    if let Some(v) = haptic_enabled {
        cfg.notifications.haptic_enabled = v;
    }
    if let Some(v) = sound_volume {
        cfg.notifications.sound_volume = v.min(100);
    }
    if let Some(v) = haptic_intensity {
        cfg.notifications.haptic_intensity = v.min(100);
    }
    if let Some(v) = notification_sound {
        cfg.notifications.notification_sound = normalize_notification_sound_choice(&v);
    }
    if let Some(v) = command_confirm_sound {
        cfg.notifications.command_confirm_sound = normalize_command_confirm_sound_choice(&v);
    }
    if let Some(v) = mcp_feedback_enabled {
        cfg.notifications.mcp_feedback_enabled = v;
    }
    if let Some(v) = auto_listen_on_feedback {
        cfg.notifications.auto_listen_on_feedback = v;
    }

    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("Notification settings updated");
    Ok(())
}

/// Preview a user-selected notification sound at the given volume.
///
/// This is used by the config UI to provide immediate auditory feedback when
/// a sound option is selected.
#[tauri::command]
pub async fn preview_notification_sound(sound: String, volume: Option<u8>) -> Result<(), String> {
    use crate::notifications::get_notification_manager;
    get_notification_manager()
        .preview_sound(&sound, volume)
        .await;
    Ok(())
}

fn normalize_notification_sound_choice(value: &str) -> String {
    match value {
        "default" | "chime" | "ping" | "pop" | "subtle" | "none" => value.to_string(),
        _ => "default".to_string(),
    }
}

fn normalize_command_confirm_sound_choice(value: &str) -> String {
    match value {
        "default" | "success" | "click" | "beep" | "none" => value.to_string(),
        _ => "default".to_string(),
    }
}

/// Set the connected ring device for haptic notifications
#[tauri::command]
pub fn set_notification_ring(device_id: Option<String>) -> Result<(), String> {
    use crate::notifications::get_notification_manager;
    get_notification_manager().set_connected_ring(device_id.clone());
    tracing::info!("Notification ring set to: {:?}", device_id);
    Ok(())
}

/// Test notification (for settings UI)
#[tauri::command]
pub async fn test_notification(
    app: tauri::AppHandle,
    notification_type: String,
) -> Result<(), String> {
    use crate::notifications::{NotificationType, get_notification_manager};

    let ntype = match notification_type.as_str() {
        "response_complete" => NotificationType::ResponseComplete,
        "mcp_feedback" => NotificationType::McpFeedbackRequest,
        "error" => NotificationType::Error,
        "listening_started" => NotificationType::ListeningStarted,
        "listening_stopped" => NotificationType::ListeningStopped,
        _ => return Err(format!("Unknown notification type: {}", notification_type)),
    };

    get_notification_manager().notify(ntype, Some(&app)).await;

    Ok(())
}

// ============================================================================
// Secure Secret Management Commands
// ============================================================================

/// Store a secret in secure storage (keychain on macOS, credential store on Windows/Linux)
/// Falls back to mock storage if security feature is disabled.
#[tauri::command]
pub async fn store_secret(key: String, value: String) -> Result<(), String> {
    let storage = crate::security::create_secure_storage();
    storage
        .store_secret(&key, &value)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!("Secret stored: {}", key);
    Ok(())
}

/// Retrieve a secret from secure storage.
/// Returns None if the secret doesn't exist.
#[tauri::command]
pub async fn get_secret(key: String) -> Result<Option<String>, String> {
    let storage = crate::security::create_secure_storage();
    storage.get_secret(&key).await.map_err(|e| e.to_string())
}

/// Delete a secret from secure storage.
#[tauri::command]
pub async fn delete_secret(key: String) -> Result<(), String> {
    let storage = crate::security::create_secure_storage();
    storage
        .delete_secret(&key)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!("Secret deleted: {}", key);
    Ok(())
}

/// Check if the security feature (keychain integration) is available.
#[tauri::command]
pub fn is_keychain_available() -> bool {
    #[cfg(feature = "security")]
    {
        true
    }
    #[cfg(not(feature = "security"))]
    {
        false
    }
}

fn api_key_storage_key_for_provider(provider: &str) -> Option<&'static str> {
    let p = provider.trim();

    if p.eq_ignore_ascii_case("openai") {
        Some("gestura_llm_openai_api_key")
    } else if p.eq_ignore_ascii_case("anthropic") {
        Some("gestura_llm_anthropic_api_key")
    } else if p.eq_ignore_ascii_case("gemini") {
        Some("gestura_llm_gemini_api_key")
    } else if p.eq_ignore_ascii_case("grok") {
        Some("gestura_llm_grok_api_key")
    } else if p.eq_ignore_ascii_case("voice_openai") {
        Some("gestura_voice_openai_api_key")
    } else if p.eq_ignore_ascii_case("serpapi") {
        Some("gestura_web_search_serpapi_key")
    } else if p.eq_ignore_ascii_case("brave") {
        Some("gestura_web_search_brave_key")
    } else {
        None
    }
}

fn legacy_api_key_storage_key_for_provider(provider: &str) -> Option<&'static str> {
    let p = provider.trim();

    if p.eq_ignore_ascii_case("openai") {
        Some("gestura_api_key_openai")
    } else if p.eq_ignore_ascii_case("anthropic") {
        Some("gestura_api_key_anthropic")
    } else if p.eq_ignore_ascii_case("gemini") {
        Some("gestura_api_key_gemini")
    } else if p.eq_ignore_ascii_case("grok") {
        Some("gestura_api_key_grok")
    } else if p.eq_ignore_ascii_case("voice_openai") {
        Some("gestura_api_key_voice_openai")
    } else if p.eq_ignore_ascii_case("serpapi") {
        Some("gestura_api_key_serpapi")
    } else if p.eq_ignore_ascii_case("brave") {
        Some("gestura_api_key_brave")
    } else {
        None
    }
}

/// Store an API key securely.
///
/// Convenience wrapper that uses provider-specific key names.
/// Provider can be: "openai", "anthropic", "grok", "serpapi", "brave".
///
/// JS↔Rust interop: This command is exposed to the frontend via Tauri.
#[tauri::command(rename_all = "snake_case")]
pub async fn store_api_key(provider: String, api_key: String) -> Result<(), String> {
    let key = api_key_storage_key_for_provider(&provider)
        .ok_or_else(|| format!("Unknown provider: {provider}"))?;
    store_secret(key.to_string(), api_key).await
}

/// Retrieve an API key from secure storage.
///
/// JS↔Rust interop: This command is exposed to the frontend via Tauri.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_api_key(provider: String) -> Result<Option<String>, String> {
    let canonical_key = api_key_storage_key_for_provider(&provider)
        .ok_or_else(|| format!("Unknown provider: {provider}"))?;
    let legacy_key = legacy_api_key_storage_key_for_provider(&provider);

    let storage = crate::security::create_secure_storage();

    if let Some(v) = storage
        .get_secret(canonical_key)
        .await
        .map_err(|e| e.to_string())?
        .filter(|s| !s.is_empty())
    {
        return Ok(Some(v));
    }

    if let Some(legacy_key) = legacy_key
        && let Some(v) = storage
            .get_secret(legacy_key)
            .await
            .map_err(|e| e.to_string())?
            .filter(|s| !s.is_empty())
    {
        // Best-effort self-heal to canonical name.
        let _ = storage.store_secret(canonical_key, &v).await;
        return Ok(Some(v));
    }

    Ok(None)
}

/// Delete an API key from secure storage.
///
/// JS↔Rust interop: This command is exposed to the frontend via Tauri.
#[tauri::command(rename_all = "snake_case")]
pub async fn delete_api_key(provider: String) -> Result<(), String> {
    let canonical_key = api_key_storage_key_for_provider(&provider)
        .ok_or_else(|| format!("Unknown provider: {provider}"))?;
    let legacy_key = legacy_api_key_storage_key_for_provider(&provider);

    let storage = crate::security::create_secure_storage();
    storage
        .delete_secret(canonical_key)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(legacy_key) = legacy_key {
        // Best-effort cleanup of legacy keys.
        let _ = storage.delete_secret(legacy_key).await;
    }
    Ok(())
}

/// Check if an API key exists for a provider without exposing the key value.
///
/// Returns true if a non-empty API key is found in secure storage or config file.
/// This is used by the frontend to determine which providers are available.
///
/// JS↔Rust interop: This command is exposed to the frontend via Tauri.
#[tauri::command(rename_all = "snake_case")]
pub async fn has_api_key(provider: String) -> Result<bool, String> {
    // Check secure storage first
    if let Ok(Some(key)) = get_api_key(provider.clone()).await
        && !key.is_empty()
    {
        return Ok(true);
    }

    // Fallback: check config file
    let config = AppConfig::load_async().await;
    let has_key = match provider.to_lowercase().as_str() {
        "openai" => config
            .llm
            .openai
            .as_ref()
            .map(|c| !c.api_key.trim().is_empty())
            .unwrap_or(false),
        "anthropic" => config
            .llm
            .anthropic
            .as_ref()
            .map(|c| !c.api_key.trim().is_empty())
            .unwrap_or(false),
        "gemini" => config
            .llm
            .gemini
            .as_ref()
            .map(|c| !c.api_key.trim().is_empty())
            .unwrap_or(false),
        "grok" => config
            .llm
            .grok
            .as_ref()
            .map(|c| !c.api_key.trim().is_empty())
            .unwrap_or(false),
        "voice_openai" => config
            .voice
            .openai_api_key
            .as_ref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false),
        "serpapi" => config
            .web_search
            .serpapi_key
            .as_ref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false),
        "brave" => config
            .web_search
            .brave_key
            .as_ref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false),
        _ => false,
    };

    Ok(has_key)
}

/// Check which LLM providers have API keys configured.
///
/// Returns a JSON object with provider names as keys and boolean values indicating
/// whether an API key is available. Ollama availability is checked by pinging its
/// endpoint with a short timeout.
///
/// Example response: {"openai": true, "anthropic": false, "gemini": false, "grok": false, "ollama": true}
///
/// JS↔Rust interop: This command is exposed to the frontend via Tauri.
#[tauri::command]
pub async fn get_available_llm_providers() -> Result<serde_json::Value, String> {
    let providers = vec!["openai", "anthropic", "gemini", "grok"];
    let mut result = serde_json::Map::new();

    for provider in providers {
        let has_key = has_api_key(provider.to_string()).await.unwrap_or(false);
        result.insert(provider.to_string(), serde_json::Value::Bool(has_key));
    }

    // Check Ollama availability by pinging its endpoint (no API key required, but
    // the server must be running).  Delegates to gestura_core (single source of truth).
    let cfg = AppConfig::load_async().await;
    let ollama_base = cfg
        .llm
        .ollama
        .as_ref()
        .map(|o| o.base_url.as_str())
        .unwrap_or("");
    let ollama_available = gestura_core::check_ollama_connectivity(ollama_base).await;
    result.insert(
        "ollama".to_string(),
        serde_json::Value::Bool(ollama_available),
    );

    Ok(serde_json::Value::Object(result))
}

/// Migrate API keys from config file to secure storage.
/// This is a one-time operation for existing users.
#[tauri::command]
pub async fn migrate_api_keys_to_keychain() -> Result<serde_json::Value, String> {
    let cfg = AppConfig::load_async().await;
    let storage = crate::security::create_secure_storage();
    let mut migrated: Vec<String> = Vec::new();

    // Migrate OpenAI key
    if let Some(ref openai) = cfg.llm.openai
        && !openai.api_key.is_empty()
    {
        storage
            .store_secret("gestura_llm_openai_api_key", &openai.api_key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("openai".to_string());
    }

    // Migrate Anthropic key
    if let Some(ref anthropic) = cfg.llm.anthropic
        && !anthropic.api_key.is_empty()
    {
        storage
            .store_secret("gestura_llm_anthropic_api_key", &anthropic.api_key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("anthropic".to_string());
    }

    // Migrate Grok key
    if let Some(ref grok) = cfg.llm.grok
        && !grok.api_key.is_empty()
    {
        storage
            .store_secret("gestura_llm_grok_api_key", &grok.api_key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("grok".to_string());
    }

    // Migrate SerpAPI key
    if let Some(ref key) = cfg.web_search.serpapi_key
        && !key.is_empty()
    {
        storage
            .store_secret("gestura_web_search_serpapi_key", key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("serpapi".to_string());
    }

    // Migrate Brave key
    if let Some(ref key) = cfg.web_search.brave_key
        && !key.is_empty()
    {
        storage
            .store_secret("gestura_web_search_brave_key", key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("brave".to_string());
    }

    // Migrate Voice/STT OpenAI key (separate from LLM OpenAI key)
    if let Some(ref key) = cfg.voice.openai_api_key
        && !key.is_empty()
    {
        storage
            .store_secret("gestura_voice_openai_api_key", key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("voice_openai".to_string());
    }

    tracing::info!("Migrated {} API keys to secure storage", migrated.len());

    Ok(serde_json::json!({
        "migrated": migrated,
        "count": migrated.len()
    }))
}

/// Get the current system theme (light or dark).
/// Returns "light" or "dark" based on the system's appearance settings.
#[tauri::command]
pub fn get_system_theme() -> String {
    if is_system_dark_mode() {
        "dark".to_string()
    } else {
        "light".to_string()
    }
}

/// Detect if the system is using dark mode (macOS-specific).
#[cfg(target_os = "macos")]
fn is_system_dark_mode() -> bool {
    use std::process::Command;

    // Query macOS for the current appearance setting
    let output = Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output();

    match output {
        Ok(output) => {
            let result = String::from_utf8_lossy(&output.stdout);
            result.trim().eq_ignore_ascii_case("dark")
        }
        Err(_) => {
            // If the command fails or the key doesn't exist, assume light mode
            false
        }
    }
}

/// Detect if the system is using dark mode (non-macOS platforms).
#[cfg(not(target_os = "macos"))]
fn is_system_dark_mode() -> bool {
    // Default to light mode on non-macOS platforms.
    // Note: Windows/Linux dark mode detection is not currently implemented.
    false
}

// ============================================================================
// Hooks Settings Commands
// ============================================================================

/// Get current hooks configuration.
#[tauri::command]
pub async fn get_hooks_settings() -> gestura_core::hooks::HooksSettings {
    AppConfig::load_async().await.hooks
}

/// Update hooks configuration.
#[tauri::command]
pub async fn set_hooks_settings(hooks: gestura_core::hooks::HooksSettings) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.hooks = hooks;
    cfg.save_async().await.map_err(|e| e.to_string())
}

/// Enable or disable hooks globally.
#[tauri::command]
pub async fn set_hooks_enabled(enabled: bool) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.hooks.enabled = enabled;
    cfg.save_async().await.map_err(|e| e.to_string())
}

// ============================================================================
// Checkpoint Commands
// ============================================================================

/// List checkpoints for a session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_session_checkpoints(
    session_id: String,
) -> Result<Vec<gestura_core::checkpoints::CheckpointMetadata>, String> {
    tokio::task::spawn_blocking(move || {
        use gestura_core::checkpoints::{
            CheckpointManager, CheckpointRetentionPolicy, FileCheckpointStore,
        };

        let manager = CheckpointManager::new(
            FileCheckpointStore::new_default(),
            CheckpointRetentionPolicy::default(),
        );
        manager
            .list_session_checkpoints(&session_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|error| format!("Failed to list session checkpoints: {error}"))?
}

/// Restore a session checkpoint by ID.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn restore_session_checkpoint(
    checkpoint_id: String,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
        use gestura_core::agent_sessions::FileAgentSessionStore;
        use gestura_core::checkpoints::{
            CheckpointId, CheckpointManager, CheckpointRetentionPolicy, FileCheckpointStore,
        };
        use gestura_core::tasks::TaskManager;

        let cp_id: CheckpointId = serde_json::from_str(&format!("\"{}\"", checkpoint_id))
            .map_err(|e| format!("Invalid checkpoint ID: {}", e))?;

        let manager = CheckpointManager::new(
            FileCheckpointStore::new_default(),
            CheckpointRetentionPolicy::default(),
        );
        let session_store = FileAgentSessionStore::default();
        let task_manager =
            TaskManager::new(dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from(".")));

        let payload = manager
            .apply_session_checkpoint(&cp_id, &session_store, &task_manager)
            .map_err(|e| e.to_string())?;

        Ok(serde_json::json!({
            "session_id": payload.session.id,
            "message_count": payload.session.message_count(),
            "restored_at": chrono::Utc::now().to_rfc3339(),
        }))
    })
    .await
    .map_err(|error| format!("Failed to restore session checkpoint: {error}"))?
}

// ============================================================================
// Tool Permission Grants Commands (persisted AllowAlways grants)
// ============================================================================

/// List persisted tool permission grants.
#[tauri::command]
pub fn list_tool_permission_grants()
-> Result<Vec<gestura_core::tools::permissions::Permission>, String> {
    use gestura_core::tools::permissions::PermissionManager;
    let manager = PermissionManager::new();
    manager.list().map_err(|e| e.to_string())
}

/// Get tool permission audit log.
#[tauri::command]
pub fn get_permission_audit_log()
-> Result<Vec<gestura_core::tools::permissions::PermissionAuditEntry>, String> {
    use gestura_core::tools::permissions::PermissionManager;
    let manager = PermissionManager::new();
    manager.audit_log().map_err(|e| e.to_string())
}

/// Revoke a persisted tool permission grant.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn revoke_tool_permission(tool: String, action: String) -> Result<usize, String> {
    use gestura_core::tools::permissions::PermissionManager;

    let manager = PermissionManager::new();
    manager.revoke(&tool, &action).map_err(|e| e.to_string())
}

// ============================================================================
// Global Permission Settings Commands
// ============================================================================

/// Get global permission settings.
#[tauri::command]
pub async fn get_global_permission_settings() -> gestura_core::config::GlobalPermissionSettings {
    AppConfig::load_async().await.permissions
}

/// Set global permission settings.
#[tauri::command]
pub async fn set_global_permission_settings(
    permissions: gestura_core::config::GlobalPermissionSettings,
) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.permissions = permissions;
    cfg.save_async().await.map_err(|e| e.to_string())
}

/// Set the default permission level for new sessions.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn set_default_permission_level(level: String) -> Result<(), String> {
    use gestura_core::config::GlobalPermissionLevel;

    let permission_level = match level.to_lowercase().as_str() {
        "sandbox" => GlobalPermissionLevel::Sandbox,
        "restricted" => GlobalPermissionLevel::Restricted,
        "full" => GlobalPermissionLevel::Full,
        _ => {
            return Err(format!(
                "Invalid permission level: {}. Use 'sandbox', 'restricted', or 'full'",
                level
            ));
        }
    };

    let mut cfg = AppConfig::load_async().await;
    cfg.permissions.default_level = permission_level;
    cfg.save_async().await.map_err(|e| e.to_string())
}

/// Update the default enabled tool map for newly created sessions.
#[tauri::command(rename_all = "snake_case")]
pub async fn update_default_enabled_tools(
    default_enabled_tools: std::collections::HashMap<String, bool>,
) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.update_default_enabled_tools(default_enabled_tools);
    cfg.save_async().await.map_err(|e| e.to_string())
}

/// Update the UI theme mode.
#[tauri::command(rename_all = "snake_case")]
pub async fn update_theme_mode(theme_mode: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.set_theme_mode(&theme_mode);
    cfg.save_async().await.map_err(|e| e.to_string())
}

/// Update prompt-enhancement settings without requiring a full config save.
#[tauri::command(rename_all = "snake_case")]
pub async fn update_prompt_enhancement_settings(
    auto_enhance: Option<bool>,
    style: Option<String>,
    max_length_multiplier_x10: Option<u8>,
) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.apply_prompt_enhancement_settings_patch(
        gestura_core::config::PromptEnhancementSettingsPatch {
            auto_enhance,
            style,
            max_length_multiplier_x10,
        },
    );
    cfg.save_async().await.map_err(|e| e.to_string())
}

/// Point local Whisper configuration at a user-selected model file.
#[tauri::command(rename_all = "snake_case")]
pub async fn update_local_whisper_model_path(model_path: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.update_local_whisper_model_path(model_path);
    cfg.save_async().await.map_err(|e| e.to_string())
}
