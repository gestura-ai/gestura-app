//! Shared slash-command helpers for agent (basic + TUI).

use std::path::{Path, PathBuf};

use super::tui::{ManagedCommandAction, ManagedCommandEntry};
use crate::commands::tools::permissions::permission_manager;
use chrono::{Local, Utc};

use gestura_core::{
    AppConfig, AppConfigSecurityExt,
    agent_sessions::{AgentSession, SessionFilter, SessionInfo, SessionPermissionLevel},
    agents::AgentManager,
    config::{McpScope, McpServerEntry, McpTransportType, infer_transport_from_endpoint},
    config_env::{is_secret_key, redact_secret},
    context::{ContextCategory, ContextManager, ContextManagerStats, RequestAnalysis},
    find_tool,
    hooks::{HookCommandTemplate, HookDefinition, HookEvent},
    knowledge::{KnowledgeItem, KnowledgeMatch},
    memory_bank::MemoryBankEntry,
    orchestrator::{
        AgentExecutionMode, AgentOrchestrator, AgentRole, ApprovalActor, ApprovalActorKind,
        ApprovalScope, ApprovalState, ChildSupervisorRunRequest, CollaborationActionStatus,
        CollaborationEscalationLevel, CollaborationRequestKind, DelegatedCheckpointAction,
        DelegatedCheckpointStage, DelegatedReplaySafety, DelegatedResumeDisposition,
        LocalExecutionPhase, LocalExecutionWaitingReason, SharedCognitionKind, SupervisorRun,
        SupervisorRunStatus, SupervisorTaskRecord, SupervisorTaskState, TeamActionRequestDraft,
        TeamEscalationDraft, TeamMessageDraft, TeamMessageKind, TeamThread,
    },
    tasks::{Task, TaskManager, TaskStatus},
    tools::permissions::PermissionScope,
};

fn short_session_id(session: &AgentSession) -> String {
    session.id[..session.id.len().min(8)].to_string()
}

fn has_openai_configured(config: &AppConfig) -> bool {
    std::env::var("OPENAI_API_KEY").is_ok()
        || config
            .llm
            .openai
            .as_ref()
            .is_some_and(|o| !o.api_key.is_empty())
}

fn has_anthropic_configured(config: &AppConfig) -> bool {
    std::env::var("ANTHROPIC_API_KEY").is_ok()
        || config
            .llm
            .anthropic
            .as_ref()
            .is_some_and(|a| !a.api_key.is_empty())
}

fn has_grok_configured(config: &AppConfig) -> bool {
    std::env::var("XAI_API_KEY").is_ok()
        || config
            .llm
            .grok
            .as_ref()
            .is_some_and(|g| !g.api_key.is_empty())
}

fn has_gemini_configured(config: &AppConfig) -> bool {
    std::env::var("GEMINI_API_KEY").is_ok()
        || config
            .llm
            .gemini
            .as_ref()
            .is_some_and(|g| !g.api_key.is_empty())
}

fn has_ollama_configured(config: &AppConfig) -> bool {
    config.llm.ollama.is_some()
}

fn managed_entry(
    title: impl Into<String>,
    summary: impl Into<String>,
    command: impl Into<String>,
    detail: Vec<String>,
    action: ManagedCommandAction,
) -> ManagedCommandEntry {
    ManagedCommandEntry {
        title: title.into(),
        summary: summary.into(),
        command: command.into(),
        detail,
        action,
    }
}

fn current_model_label(config: &AppConfig, session: &AgentSession) -> String {
    if let Some(model) = session.model.as_deref() {
        return model.to_string();
    }

    match config.llm.primary.as_str() {
        "openai" => config
            .llm
            .openai
            .as_ref()
            .map(|cfg| cfg.model.clone())
            .unwrap_or_else(|| "(default)".to_string()),
        "anthropic" => config
            .llm
            .anthropic
            .as_ref()
            .map(|cfg| cfg.model.clone())
            .unwrap_or_else(|| "(default)".to_string()),
        "gemini" => config
            .llm
            .gemini
            .as_ref()
            .map(|cfg| cfg.model.clone())
            .unwrap_or_else(|| "(default)".to_string()),
        "grok" => config
            .llm
            .grok
            .as_ref()
            .map(|cfg| cfg.model.clone())
            .unwrap_or_else(|| "(default)".to_string()),
        "ollama" => config
            .llm
            .ollama
            .as_ref()
            .map(|cfg| cfg.model.clone())
            .unwrap_or_else(|| "(default)".to_string()),
        _ => "(default)".to_string(),
    }
}

fn provider_status_rows(config: &AppConfig) -> Vec<String> {
    let mut rows = vec![format!(
        "  {} OpenAI{}",
        if has_openai_configured(config) {
            "✓"
        } else {
            "○"
        },
        config
            .llm
            .openai
            .as_ref()
            .map(|cfg| format!(" — {}", cfg.model))
            .unwrap_or_default()
    )];
    rows.push(format!(
        "  {} Anthropic{}",
        if has_anthropic_configured(config) {
            "✓"
        } else {
            "○"
        },
        config
            .llm
            .anthropic
            .as_ref()
            .map(|cfg| format!(" — {}", cfg.model))
            .unwrap_or_default()
    ));
    rows.push(format!(
        "  {} Gemini{}",
        if has_gemini_configured(config) {
            "✓"
        } else {
            "○"
        },
        config
            .llm
            .gemini
            .as_ref()
            .map(|cfg| format!(" — {}", cfg.model))
            .unwrap_or_default()
    ));
    rows.push(format!(
        "  {} Grok{}",
        if has_grok_configured(config) {
            "✓"
        } else {
            "○"
        },
        config
            .llm
            .grok
            .as_ref()
            .map(|cfg| format!(" — {}", cfg.model))
            .unwrap_or_default()
    ));
    rows.push(format!(
        "  {} Ollama{}",
        if has_ollama_configured(config) {
            "✓"
        } else {
            "○"
        },
        config
            .llm
            .ollama
            .as_ref()
            .map(|cfg| format!(" — {} @ {}", cfg.model, cfg.base_url))
            .unwrap_or_default()
    ));
    rows
}

pub(crate) fn agent_browser_entries(
    config: &AppConfig,
    session: &AgentSession,
) -> Vec<ManagedCommandEntry> {
    let current_model = current_model_label(config, session);
    let provider_rows = provider_status_rows(config);
    let configured_providers = [
        has_openai_configured(config),
        has_anthropic_configured(config),
        has_gemini_configured(config),
        has_grok_configured(config),
        has_ollama_configured(config),
    ]
    .into_iter()
    .filter(|configured| *configured)
    .count();

    vec![
        managed_entry(
            "Agent Overview",
            format!("{} · {}", config.llm.primary, current_model),
            "/agent status",
            vec![
                format!("Version: {}", gestura_core::VERSION),
                format!("Primary provider: {}", config.llm.primary),
                format!("Active model: {}", current_model),
                format!("Session: {}", short_session_id(session)),
                format!("Messages: {}", session.message_count()),
                format!(
                    "Fallback provider: {}",
                    config.llm.fallback.as_deref().unwrap_or("(none)")
                ),
            ],
            ManagedCommandAction::Execute("/agent status".to_string()),
        ),
        managed_entry(
            "Provider Readiness",
            format!("{configured_providers}/5 providers configured"),
            "/agent status",
            {
                let mut detail = vec![
                    "Configured providers and current defaults:".to_string(),
                    String::new(),
                ];
                detail.extend(provider_rows.clone());
                detail
            },
            ManagedCommandAction::Execute("/agent status".to_string()),
        ),
        managed_entry(
            "Session Model Override",
            session.model.as_deref().unwrap_or("using config default"),
            "/model ",
            vec![
                format!("Current effective model: {}", current_model),
                format!(
                    "Session override: {}",
                    session.model.as_deref().unwrap_or("(none)")
                ),
                "Use /model to choose a different model for this session.".to_string(),
            ],
            ManagedCommandAction::Prefill("/model ".to_string()),
        ),
        managed_entry(
            "LLM Config Drilldown",
            format!(
                "primary={} fallback={}",
                config.llm.primary,
                config.llm.fallback.as_deref().unwrap_or("none")
            ),
            "/config get llm.primary",
            vec![
                "Quick drill-down commands:".to_string(),
                "  /config get llm.primary".to_string(),
                "  /config get llm.fallback".to_string(),
                format!("  /config get llm.{}.model", config.llm.primary),
            ],
            ManagedCommandAction::Prefill("/config get llm.primary".to_string()),
        ),
    ]
}

pub(crate) fn run_agent_subcommand(
    args: &[&str],
    config: &AppConfig,
    session: &AgentSession,
) -> std::result::Result<Vec<String>, String> {
    let subcommand = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    match subcommand.as_str() {
        "" | "status" => {
            let mut lines = vec![
                "━━━ Agent Status ━━━".to_string(),
                String::new(),
                format!("Version: {}", gestura_core::VERSION),
                format!("Primary LLM: {}", config.llm.primary),
                format!("Model: {}", current_model_label(config, session)),
                format!("Session: {}", short_session_id(session)),
                format!("Messages: {}", session.message_count()),
                format!(
                    "Fallback: {}",
                    config.llm.fallback.as_deref().unwrap_or("(none)")
                ),
                String::new(),
                "Provider Status:".to_string(),
            ];
            lines.extend(provider_status_rows(config));
            Ok(lines)
        }
        "config" => {
            let mut lines = vec!["━━━ Agent Configuration ━━━".to_string(), String::new()];
            lines.push(format!("Primary: {}", config.llm.primary));
            lines.push(format!(
                "Fallback: {}",
                config.llm.fallback.as_deref().unwrap_or("(none)")
            ));
            if let Some(ref openai) = config.llm.openai {
                lines.push(format!("OpenAI model: {}", openai.model));
            }
            if let Some(ref anthropic) = config.llm.anthropic {
                lines.push(format!("Anthropic model: {}", anthropic.model));
            }
            if let Some(ref gemini) = config.llm.gemini {
                lines.push(format!("Gemini model: {}", gemini.model));
            }
            if let Some(ref grok) = config.llm.grok {
                lines.push(format!("Grok model: {}", grok.model));
            }
            if let Some(ref ollama) = config.llm.ollama {
                lines.push(format!("Ollama model: {}", ollama.model));
                lines.push(format!("Ollama base URL: {}", ollama.base_url));
            }
            Ok(lines)
        }
        other => Err(format!(
            "Unknown /agent subcommand: {}. Try: status, config",
            other
        )),
    }
}

fn knowledge_store_with_session(_session_id: &str) -> gestura_core::knowledge::KnowledgeStore {
    let store = gestura_core::knowledge::KnowledgeStore::new(
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
    );
    gestura_core::knowledge::register_builtin_knowledge(&store);
    store
}

pub(crate) fn load_session_knowledge_items(session_id: &str) -> Vec<KnowledgeItem> {
    let mut items = knowledge_store_with_session(session_id).list();
    let settings_mgr = gestura_core::knowledge::KnowledgeSettingsManager::new(
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
    );
    if let Ok(enabled_ids) = settings_mgr.get_enabled_knowledge(session_id) {
        for item in &mut items {
            item.enabled = enabled_ids.contains(&item.id);
        }
    }
    items.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.name.cmp(&right.name))
    });
    items
}

pub(crate) fn knowledge_show_usage() -> &'static str {
    "Usage: /knowledge show <id>"
}

pub(crate) fn knowledge_not_found_message(id: &str) -> String {
    format!("Knowledge item '{id}' not found.")
}

pub(crate) fn knowledge_detail_lines(item: &KnowledgeItem, show_full_content: bool) -> Vec<String> {
    let mut lines = vec![
        format!("━━━ {} [{}] ━━━", item.name, item.id),
        String::new(),
        format!("Category: {}", item.category),
        format!("Enabled: {}", if item.enabled { "yes" } else { "no" }),
        format!("Priority: {}", item.priority),
        format!(
            "Origin: {}",
            item.metadata
                .get("origin")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string())
        ),
    ];

    if let Some(repo) = item.metadata.get("source_repo") {
        lines.push(format!("Source repo: {repo}"));
    }
    if let Some(path) = item.metadata.get("source_path") {
        lines.push(format!("Source path: {path}"));
    }
    if let Some(url) = item.metadata.get("source_url") {
        lines.push(format!("Source URL: {url}"));
    }

    lines.push(String::new());
    lines.push(format!("Description: {}", item.description));

    if !item.triggers.is_empty() {
        lines.push(format!("Triggers: {}", item.triggers.join(", ")));
    }

    if !item.references.is_empty() {
        lines.push("References:".to_string());
        for reference in &item.references {
            lines.push(format!("  • {} — {}", reference.topic, reference.path));
        }
    }

    if !item.core_content.trim().is_empty() {
        lines.push(String::new());
        lines.push("Content:".to_string());
        let content_lines: Vec<&str> = item.core_content.lines().collect();
        let visible = if show_full_content {
            content_lines.len()
        } else {
            content_lines.len().min(12)
        };
        for line in content_lines.into_iter().take(visible) {
            lines.push(line.to_string());
        }
        if !show_full_content && item.core_content.lines().count() > visible {
            lines.push("… (truncated; use /knowledge show <id> for full content)".to_string());
        }
    }

    lines
}

pub(crate) fn run_knowledge_subcommand(
    args: &[&str],
    session: &AgentSession,
) -> std::result::Result<Vec<String>, String> {
    let subcommand = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let store = knowledge_store_with_session(&session.id);

    match subcommand.as_str() {
        "" | "list" => {
            let items = load_session_knowledge_items(&session.id);
            if items.is_empty() {
                Ok(vec![knowledge_empty_message().to_string()])
            } else {
                Ok(knowledge_list_lines(&items))
            }
        }
        "search" => {
            let query_text = args.get(1..).unwrap_or_default().join(" ");
            if query_text.is_empty() {
                Err(knowledge_search_usage().to_string())
            } else {
                let query = gestura_core::knowledge::KnowledgeQuery {
                    query: query_text.clone(),
                    limit: Some(10),
                    min_score: Some(0.15),
                    ..Default::default()
                };
                let matches = store.find(&query);
                if matches.is_empty() {
                    Ok(vec![knowledge_no_results_message(&query_text)])
                } else {
                    Ok(knowledge_search_lines(&query_text, &matches))
                }
            }
        }
        "categories" => {
            let cats = store.categories();
            if cats.is_empty() {
                Ok(vec![knowledge_no_categories_message().to_string()])
            } else {
                let category_counts: Vec<(String, usize)> = cats
                    .iter()
                    .map(|cat| (cat.clone(), store.list_by_category(cat).len()))
                    .collect();
                Ok(knowledge_categories_lines(&category_counts))
            }
        }
        "status" => Ok(knowledge_status_lines(
            store.count(),
            store.categories().len(),
            store.base_dir(),
        )),
        "show" => {
            let Some(id) = args.get(1) else {
                return Err(knowledge_show_usage().to_string());
            };
            let items = load_session_knowledge_items(&session.id);
            let Some(item) = items.into_iter().find(|item| item.id == *id) else {
                return Err(knowledge_not_found_message(id));
            };
            Ok(knowledge_detail_lines(&item, true))
        }
        other => Err(format!(
            "Unknown /knowledge subcommand: {}. Try: list, search, categories, status, show",
            other
        )),
    }
}

pub(crate) fn run_device_subcommand(args: &[&str]) -> std::result::Result<Vec<String>, String> {
    let subcommand = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    match subcommand.as_str() {
        "" | "list" | "scan" => {
            let devices = gestura_core::list_audio_input_devices();
            let mic_available = gestura_core::is_microphone_available();

            let mut lines = vec!["━━━ Audio Devices ━━━".to_string(), String::new()];
            lines.push(format!(
                "Microphone available: {}",
                if mic_available { "✓ yes" } else { "✗ no" }
            ));
            lines.push(String::new());

            if devices.is_empty() {
                lines.push("No audio input devices found.".to_string());
            } else {
                lines.push(format!("{} device(s) detected:", devices.len()));
                for dev in &devices {
                    let marker = if dev.is_default { " (default)" } else { "" };
                    lines.push(format!("  • {}{}", dev.name, marker));
                }
            }

            Ok(lines)
        }
        other => Err(format!(
            "Unknown /device subcommand: {}. Try: list, scan",
            other
        )),
    }
}

pub(crate) fn device_browser_entries(config: &AppConfig) -> Vec<ManagedCommandEntry> {
    let devices = gestura_core::list_audio_input_devices();
    let mic_available = gestura_core::is_microphone_available();
    let configured_device = config.voice.audio_device.as_deref();
    let default_device = devices.iter().find(|device| device.is_default);

    let mut entries = vec![managed_entry(
        "Microphone Readiness",
        if mic_available {
            format!("{} input device(s) available", devices.len())
        } else {
            "No microphone input detected".to_string()
        },
        "/device scan",
        vec![
            format!(
                "Microphone available: {}",
                if mic_available { "yes" } else { "no" }
            ),
            format!("Detected input devices: {}", devices.len()),
            format!(
                "Configured voice.audio_device: {}",
                configured_device.unwrap_or("(system default)")
            ),
        ],
        ManagedCommandAction::Execute("/device scan".to_string()),
    )];

    entries.push(managed_entry(
        "Default Input Device",
        default_device
            .map(|device| device.name.clone())
            .unwrap_or_else(|| "(none)".to_string()),
        "/device list",
        vec![
            format!(
                "Default device: {}",
                default_device
                    .map(|device| device.name.as_str())
                    .unwrap_or("(none)")
            ),
            format!(
                "Configured voice.audio_device: {}",
                configured_device.unwrap_or("(system default)")
            ),
            "Use /config get voice.audio_device to inspect or /config update voice.audio_device <name> to change it.".to_string(),
        ],
        ManagedCommandAction::Execute("/device list".to_string()),
    ));

    for device in devices {
        let summary = if device.is_default {
            "default input".to_string()
        } else {
            "available input".to_string()
        };
        let is_configured = configured_device.is_some_and(|name| name == device.name);
        entries.push(managed_entry(
            format!("Device: {}", device.name),
            summary,
            "/device list",
            vec![
                format!("Name: {}", device.name),
                format!(
                    "Default device: {}",
                    if device.is_default { "yes" } else { "no" }
                ),
                format!(
                    "Selected in config: {}",
                    if is_configured { "yes" } else { "no" }
                ),
                "Use /config update voice.audio_device <name> to pin this input device."
                    .to_string(),
            ],
            ManagedCommandAction::Prefill(format!(
                "/config update voice.audio_device \"{}\"",
                device.name
            )),
        ));
    }

    entries
}

pub(crate) fn health_diagnostic_lines(config: &AppConfig) -> Vec<String> {
    let config_path = AppConfig::default_path();
    let config_ok = config_path.exists();
    let devices = gestura_core::list_audio_input_devices();
    let mic_available = gestura_core::is_microphone_available();
    let mcp_count = config.mcp_servers.len();
    let mcp_enabled = config
        .mcp_servers
        .iter()
        .filter(|server| server.enabled)
        .count();

    vec![
        "━━━ System Health ━━━".to_string(),
        String::new(),
        format!("✓ Gestura v{}", gestura_core::VERSION),
        format!(
            "{} Config: {}",
            if config_ok { "✓" } else { "○" },
            config_path.display()
        ),
        String::new(),
        "LLM Providers:".to_string(),
        format!(
            "  {} OpenAI",
            if has_openai_configured(config) {
                "✓"
            } else {
                "○"
            }
        ),
        format!(
            "  {} Anthropic",
            if has_anthropic_configured(config) {
                "✓"
            } else {
                "○"
            }
        ),
        format!(
            "  {} Grok",
            if has_grok_configured(config) {
                "✓"
            } else {
                "○"
            }
        ),
        format!(
            "  {} Ollama",
            if has_ollama_configured(config) {
                "✓"
            } else {
                "○"
            }
        ),
        String::new(),
        "Audio:".to_string(),
        format!("  {} Microphone", if mic_available { "✓" } else { "○" }),
        format!("  {} device(s) detected", devices.len()),
        String::new(),
        "MCP:".to_string(),
        format!(
            "  {} server(s) configured ({} enabled)",
            mcp_count, mcp_enabled
        ),
    ]
}

pub(crate) fn privacy_policy_lines() -> Vec<String> {
    vec![
        "━━━ Data Retention Policy ━━━".to_string(),
        String::new(),
        "Gestura respects user privacy and GDPR compliance:".to_string(),
        String::new(),
        "• Voice recordings: Temporary only, deleted after transcription".to_string(),
        "• Agent sessions: Stored locally in workspace".to_string(),
        "• API keys: Stored in local config file only".to_string(),
        "• Memory bank: Stored locally in .gestura/memory/".to_string(),
        "• No data is sent to third parties except configured LLM providers".to_string(),
        String::new(),
        "Use 'gestura privacy export' for a full GDPR data export.".to_string(),
        "Use 'gestura privacy delete' to exercise right to erasure.".to_string(),
    ]
}

pub(crate) fn privacy_report_lines(pretty_report: String) -> Vec<String> {
    vec![
        "━━━ Privacy Report ━━━".to_string(),
        String::new(),
        pretty_report,
    ]
}

pub(crate) fn a2a_status_lines() -> Vec<String> {
    vec![
        "━━━ A2A Protocol Status ━━━".to_string(),
        String::new(),
        "Protocol: Agent2Agent (A2A)".to_string(),
        "Version: 0.3.0".to_string(),
        "Governance: Linux Foundation".to_string(),
        "License: Apache 2.0".to_string(),
        String::new(),
        "Features:".to_string(),
        "  ✓ Agent discovery via Agent Cards".to_string(),
        "  ✓ Task-based communication".to_string(),
        "  ✓ JSON-RPC 2.0 protocol".to_string(),
        "  ✓ Bearer token authentication".to_string(),
        "  ✓ Profile propagation".to_string(),
        "  ✓ SSE streaming support".to_string(),
        String::new(),
        "Endpoints:".to_string(),
        "  • agent/discover".to_string(),
        "  • task/create".to_string(),
        "  • task/status".to_string(),
        "  • task/cancel".to_string(),
        "  • profile/register".to_string(),
        "  • profile/validate".to_string(),
    ]
}

pub(crate) fn a2a_profiles_lines() -> Vec<String> {
    vec![
        "━━━ A2A Profiles ━━━".to_string(),
        String::new(),
        "Local profile persistence is not wired up yet in the interactive surfaces.".to_string(),
        "Use /a2a register <agent_id> <name> [cap1,cap2] to generate the registration summary."
            .to_string(),
    ]
}

pub(crate) fn a2a_agents_lines() -> Vec<String> {
    vec![
        "━━━ A2A Agents ━━━".to_string(),
        String::new(),
        "No remote agents cached yet.".to_string(),
        "Use /a2a discover <url> to inspect one.".to_string(),
    ]
}

pub(crate) fn context_category_icon(cat: ContextCategory) -> &'static str {
    match cat {
        ContextCategory::FileSystem => "📁",
        ContextCategory::Shell => "🖥️",
        ContextCategory::Git => "🔀",
        ContextCategory::Code => "💻",
        ContextCategory::Web => "🌐",
        ContextCategory::Voice => "🎤",
        ContextCategory::Config => "⚙️",
        ContextCategory::Session => "📜",
        ContextCategory::Tools => "🔧",
        ContextCategory::Agent => "🤖",
        ContextCategory::Mcp => "🔌",
        ContextCategory::A2a => "🔗",
        ContextCategory::Task => "✅",
        ContextCategory::Screen => "🎥",
        ContextCategory::General => "💬",
    }
}

fn context_categories() -> Vec<(ContextCategory, &'static str)> {
    vec![
        (
            ContextCategory::FileSystem,
            "File system operations (read, write, edit)",
        ),
        (ContextCategory::Shell, "Shell command execution"),
        (ContextCategory::Git, "Git version control operations"),
        (ContextCategory::Code, "Code analysis (symbols, references)"),
        (ContextCategory::Web, "Web fetching and search"),
        (ContextCategory::Voice, "Voice and audio processing"),
        (ContextCategory::Config, "Configuration management"),
        (ContextCategory::Session, "Session and history"),
        (ContextCategory::Tools, "Tool introspection"),
        (ContextCategory::Agent, "Agent orchestration"),
        (ContextCategory::Mcp, "MCP protocol operations"),
        (ContextCategory::A2a, "A2A protocol operations"),
        (ContextCategory::Task, "Task management for current session"),
        (
            ContextCategory::Screen,
            "Screen capture and recording (screenshot, screen_record)",
        ),
        (ContextCategory::General, "General conversation (no tools)"),
    ]
}

pub(crate) fn context_status_lines(stats: &ContextManagerStats) -> Vec<String> {
    vec![
        "━━━ Context Manager Status ━━━".to_string(),
        String::new(),
        "Cache Statistics".to_string(),
        format!(
            "  Context Cache: {} / {} entries",
            stats.context_cache.size, stats.context_cache.max_size
        ),
        format!(
            "  File Cache:    {} / {} entries",
            stats.file_cache.size, stats.file_cache.max_size
        ),
        format!(
            "  History Cache: {} / {} entries",
            stats.history_cache.size, stats.history_cache.max_size
        ),
        String::new(),
        "Features".to_string(),
        "  ✓ Request analysis without LLM".to_string(),
        "  ✓ Category-based tool filtering".to_string(),
        "  ✓ Smart context caching with TTL".to_string(),
        "  ✓ Entity extraction (paths, URLs)".to_string(),
        "  ✓ Follow-up detection".to_string(),
    ]
}

pub(crate) fn context_analysis_lines(request: &str, analysis: &RequestAnalysis) -> Vec<String> {
    let mut lines = vec![
        "━━━ Request Analysis ━━━".to_string(),
        String::new(),
        format!("Request: {}", request),
        String::new(),
        "Detected Categories".to_string(),
    ];

    if analysis.categories.is_empty() {
        lines.push("  (none)".to_string());
    } else {
        for cat in &analysis.categories {
            lines.push(format!("  {} {:?}", context_category_icon(*cat), cat));
        }
    }

    lines.push(String::new());
    lines.push("Suggested Tools".to_string());
    if analysis.suggested_tools.is_empty() {
        lines.push("  (none — general conversation)".to_string());
    } else {
        for tool in &analysis.suggested_tools {
            lines.push(format!("  • {}", tool));
        }
    }

    if !analysis.entities.is_empty() {
        lines.push(String::new());
        lines.push("Extracted Entities".to_string());
        for entity in &analysis.entities {
            lines.push(format!("  → [{:?}] {}", entity.entity_type, entity.value));
        }
    }

    lines.push(String::new());
    lines.push("Analysis Flags".to_string());
    lines.push(format!(
        "  Needs Tools: {}",
        if analysis.needs_tools { "✓" } else { "✗" }
    ));
    lines.push(format!(
        "  Is Follow-up: {}",
        if analysis.is_followup { "✓" } else { "✗" }
    ));
    lines.push(format!(
        "  Confidence: {}%",
        (analysis.confidence * 100.0) as u32
    ));

    lines
}

pub(crate) fn context_categories_lines() -> Vec<String> {
    let mut lines = vec!["━━━ Context Categories ━━━".to_string(), String::new()];
    for (category, description) in context_categories() {
        lines.push(format!(
            "{} {:?} — {}",
            context_category_icon(category),
            category,
            description
        ));
    }
    lines
}

pub(crate) fn context_clear_message() -> &'static str {
    "Context caches cleared"
}

const FEATURED_CONFIG_KEYS: &[&str] = &[
    "llm.primary",
    "voice.provider",
    "voice.local_model_path",
    "voice.audio_device",
    "ui.theme_mode",
    "hotkey_listen",
    "nats_url",
    "pipeline.max_history_messages",
    "pipeline.auto_compact_threshold_percent",
    "pipeline.compaction_strategy",
    "pipeline.max_context_tokens",
    "pipeline.log_token_usage",
];

pub(crate) fn config_lookup_value(config: &AppConfig, key: &str) -> Option<String> {
    match key {
        "llm.openai.api_key" => config
            .llm
            .openai
            .as_ref()
            .map(|c| redact_secret(&c.api_key)),
        "llm.anthropic.api_key" => config
            .llm
            .anthropic
            .as_ref()
            .map(|c| redact_secret(&c.api_key)),
        "llm.grok.api_key" => config.llm.grok.as_ref().map(|c| redact_secret(&c.api_key)),
        _ => config.get(key),
    }
}

pub(crate) fn config_list_lines(config: &AppConfig) -> Vec<String> {
    let mut lines = vec!["━━━ Configuration ━━━".to_string(), String::new()];
    for key in FEATURED_CONFIG_KEYS {
        let value = config_lookup_value(config, key).unwrap_or_else(|| "(unset)".to_string());
        lines.push(format!("  {key:<42} {value}"));
    }
    lines.push(String::new());
    lines.push(config_path_line());
    lines.push(String::new());
    lines.push("Use /config get <key> for a specific value".to_string());
    lines.push("Use /config keys to list all available keys".to_string());
    lines
}

pub(crate) fn config_get_line(config: &AppConfig, key: &str) -> Option<String> {
    config_lookup_value(config, key).map(|value| format!("{key} = {value}"))
}

pub(crate) fn config_keys_lines() -> Vec<String> {
    let mut lines = vec!["━━━ Available Config Keys ━━━".to_string(), String::new()];
    lines.extend(AppConfig::list_keys().into_iter().map(str::to_string));
    lines.push(String::new());
    lines.push("Use /config get <key> to view a value".to_string());
    lines
}

pub(crate) fn config_updated_message(key: &str, value: &str) -> String {
    let display_value = if is_secret_key(key) {
        redact_secret(value)
    } else {
        value.to_string()
    };
    format!("Updated config: {key} = {display_value}")
}

pub(crate) fn config_path_line() -> String {
    format!("Config file: {}", AppConfig::default_path().display())
}

pub(crate) fn config_reset_message() -> &'static str {
    "Configuration reset to defaults"
}

pub(crate) fn parse_session_list_filter(value: Option<&str>) -> (SessionFilter, String) {
    match value.map(str::to_ascii_lowercase).as_deref() {
        Some("today") => (SessionFilter::Today, " (today)".to_string()),
        Some("week") | Some("thisweek") => (SessionFilter::ThisWeek, " (this week)".to_string()),
        Some("month") | Some("thismonth") => {
            (SessionFilter::ThisMonth, " (this month)".to_string())
        }
        _ => (SessionFilter::All, String::new()),
    }
}

pub(crate) fn session_empty_message(filter_label: &str) -> String {
    format!("No saved sessions found{filter_label}")
}

fn session_relative_time(last_active: chrono::DateTime<Utc>) -> String {
    let elapsed = Utc::now().signed_duration_since(last_active);
    let secs = elapsed.num_seconds();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{} min ago", secs / 60)
    } else if secs < 86_400 {
        format!("{} hours ago", secs / 3600)
    } else {
        format!("{} days ago", secs / 86_400)
    }
}

pub(crate) fn session_list_lines(
    sessions: &[SessionInfo],
    current_id: &str,
    filter_label: &str,
    max_items: usize,
    include_hints: bool,
) -> Vec<String> {
    let mut lines = vec![
        format!("━━━ Saved Sessions{filter_label} ━━━"),
        String::new(),
    ];

    for (i, session) in sessions.iter().take(max_items).enumerate() {
        let is_current = session.id == current_id;
        let marker = if is_current { "▶ " } else { "  " };
        let model_info = session.model.as_deref().unwrap_or("default");
        lines.push(format!(
            "{marker}{}. {} ({} msgs, {model_info})",
            i + 1,
            &session.id[..session.id.len().min(8)],
            session.message_count,
        ));
        if !session.title.trim().is_empty() {
            lines.push(format!("   Title: {}", session.title));
        }
        lines.push(format!(
            "   Created: {} | Updated: {} ({})",
            session
                .created_at
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M"),
            session
                .last_active
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M"),
            session_relative_time(session.last_active)
        ));
        lines.push(String::new());
    }

    if sessions.len() > max_items {
        lines.push(format!(
            "… and {} more session(s)",
            sessions.len() - max_items
        ));
        lines.push(String::new());
    }

    lines.push(format!("Total: {} session(s)", sessions.len()));
    if include_hints {
        lines.push(String::new());
        lines.push("Filters: /session list today|week|month".to_string());
        lines.push(
            "Commands: /session load <id> | /session delete <id> | /session export [id]"
                .to_string(),
        );
    }
    lines
}

pub(crate) fn session_info_lines(session: &AgentSession) -> Vec<String> {
    let user_count = session
        .state
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .count();
    let assistant_count = session
        .state
        .messages
        .iter()
        .filter(|m| m.role == "assistant")
        .count();
    let system_count = session
        .state
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .count();

    let mut lines = vec![
        "━━━ Current Session ━━━".to_string(),
        String::new(),
        format!("ID: {}", session.id),
        format!("Title: {}", session.title),
        format!(
            "Created: {}",
            session
                .created_at
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
        ),
        format!(
            "Updated: {}",
            session
                .last_active
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
        ),
        format!("Model: {}", session.model.as_deref().unwrap_or("default")),
        format!(
            "Messages: {} (you: {}, assistant: {}, system: {})",
            session.message_count(),
            user_count,
            assistant_count,
            system_count
        ),
    ];

    if let Some(workspace) = &session.state.workspace_dir {
        lines.push(format!("Workspace: {}", workspace.display()));
    }

    lines
}

pub(crate) fn knowledge_empty_message() -> &'static str {
    "No knowledge items registered."
}

pub(crate) fn knowledge_search_usage() -> &'static str {
    "Usage: /knowledge search <query>"
}

pub(crate) fn knowledge_no_results_message(query: &str) -> String {
    format!("No knowledge items match '{query}'.")
}

pub(crate) fn knowledge_no_categories_message() -> &'static str {
    "No knowledge categories found."
}

pub(crate) fn knowledge_list_lines(items: &[KnowledgeItem]) -> Vec<String> {
    let mut lines = vec![
        format!("━━━ Knowledge Base ({} items) ━━━", items.len()),
        String::new(),
    ];
    for item in items {
        lines.push(format!(
            "  • [{}] {} — {}",
            item.category, item.name, item.description
        ));
    }
    lines
}

pub(crate) fn knowledge_search_lines(query: &str, matches: &[KnowledgeMatch]) -> Vec<String> {
    let mut lines = vec![
        format!("━━━ Knowledge Search: '{query}' ━━━"),
        String::new(),
    ];
    for matched in matches {
        lines.push(format!(
            "  • {} (score: {:.2}) — {}",
            matched.item.name, matched.score, matched.item.description
        ));
    }
    lines.push(String::new());
    lines.push(format!("{} result(s)", matches.len()));
    lines
}

pub(crate) fn knowledge_categories_lines(category_counts: &[(String, usize)]) -> Vec<String> {
    let mut lines = vec!["━━━ Knowledge Categories ━━━".to_string(), String::new()];
    for (category, count) in category_counts {
        lines.push(format!("  • {} ({} items)", category, count));
    }
    lines
}

pub(crate) fn knowledge_status_lines(
    total_items: usize,
    category_count: usize,
    base_dir: &Path,
) -> Vec<String> {
    vec![
        "━━━ Knowledge Base Status ━━━".to_string(),
        String::new(),
        format!("Total items: {}", total_items),
        format!("Categories: {}", category_count),
        format!("Base directory: {}", base_dir.display()),
    ]
}

// ===================== /hooks =====================

pub(crate) enum HooksOutcome {
    Changed(Vec<String>),
    Unchanged(Vec<String>),
}

impl HooksOutcome {
    pub(crate) fn changed(&self) -> bool {
        matches!(self, HooksOutcome::Changed(_))
    }

    pub(crate) fn into_lines(self) -> Vec<String> {
        match self {
            HooksOutcome::Changed(lines) | HooksOutcome::Unchanged(lines) => lines,
        }
    }
}

pub(crate) fn apply_hooks_subcommand(
    args: &[&str],
    config: &mut AppConfig,
) -> std::result::Result<HooksOutcome, String> {
    let mut lines: Vec<String> = Vec::new();

    let sub = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if sub.is_empty() || sub == "help" || sub == "--help" || sub == "-h" {
        return Ok(HooksOutcome::Unchanged(hooks_usage_lines()));
    }

    match sub.as_str() {
        "show" | "status" => {
            let hooks = &config.hooks;
            lines.push("━━━ Hooks Configuration ━━━".to_string());
            lines.push(String::new());
            lines.push(format!(
                "Enabled: {}",
                if hooks.enabled { "yes" } else { "no" }
            ));
            lines.push(format!("Timeout: {} ms", hooks.timeout_ms));
            lines.push(format!("Max output: {} bytes", hooks.max_output_bytes));
            lines.push(String::new());
            if hooks.allowed_programs.is_empty() {
                lines.push("Allowed programs: (none)".to_string());
            } else {
                lines.push(format!(
                    "Allowed programs: {}",
                    hooks.allowed_programs.join(", ")
                ));
            }
            lines.push(String::new());
            if hooks.hooks.is_empty() {
                lines.push("Hooks: (none)".to_string());
            } else {
                lines.push(format!("Hooks: {} configured", hooks.hooks.len()));
                for h in &hooks.hooks {
                    lines.push(format!(
                        "- {} ({:?}) -> {} {}",
                        h.name,
                        h.event,
                        h.command.program,
                        h.command.args.join(" ")
                    ));
                }
            }
            Ok(HooksOutcome::Unchanged(lines))
        }
        "list" | "ls" => {
            let hooks = &config.hooks;
            lines.push("━━━ Hooks ━━━".to_string());
            lines.push(String::new());
            if hooks.hooks.is_empty() {
                lines.push("No hooks configured.".to_string());
            } else {
                for h in &hooks.hooks {
                    lines.push(format!(
                        "- {} ({:?}) -> {} {}",
                        h.name,
                        h.event,
                        h.command.program,
                        h.command.args.join(" ")
                    ));
                }
            }
            Ok(HooksOutcome::Unchanged(lines))
        }
        "enable" => {
            if config.hooks.enabled {
                lines.push("Hooks already enabled.".to_string());
                Ok(HooksOutcome::Unchanged(lines))
            } else {
                config.hooks.enabled = true;
                lines.push("Enabled hooks.".to_string());
                Ok(HooksOutcome::Changed(lines))
            }
        }
        "disable" => {
            if !config.hooks.enabled {
                lines.push("Hooks already disabled.".to_string());
                Ok(HooksOutcome::Unchanged(lines))
            } else {
                config.hooks.enabled = false;
                lines.push("Disabled hooks.".to_string());
                Ok(HooksOutcome::Changed(lines))
            }
        }
        "allow" => {
            let action = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            match action.as_str() {
                "list" | "ls" | "" => {
                    if config.hooks.allowed_programs.is_empty() {
                        lines.push("Allowed programs: (none)".to_string());
                    } else {
                        lines.push(format!(
                            "Allowed programs ({}): {}",
                            config.hooks.allowed_programs.len(),
                            config.hooks.allowed_programs.join(", ")
                        ));
                    }
                    Ok(HooksOutcome::Unchanged(lines))
                }
                "add" => {
                    let Some(program) = args.get(2).copied() else {
                        return Err("Usage: /hooks allow add <program>".to_string());
                    };
                    if config.hooks.allowed_programs.iter().any(|p| p == program) {
                        lines.push(format!("Program already allow-listed: {program}"));
                        Ok(HooksOutcome::Unchanged(lines))
                    } else {
                        config.hooks.allowed_programs.push(program.to_string());
                        lines.push(format!("Allow-listed program: {program}"));
                        Ok(HooksOutcome::Changed(lines))
                    }
                }
                "remove" | "rm" | "del" | "delete" => {
                    let Some(program) = args.get(2).copied() else {
                        return Err("Usage: /hooks allow remove <program>".to_string());
                    };
                    let before = config.hooks.allowed_programs.len();
                    config.hooks.allowed_programs.retain(|p| p != program);
                    if config.hooks.allowed_programs.len() == before {
                        lines.push(format!("Program not in allow-list: {program}"));
                        Ok(HooksOutcome::Unchanged(lines))
                    } else {
                        lines.push(format!("Removed from allow-list: {program}"));
                        Ok(HooksOutcome::Changed(lines))
                    }
                }
                _ => Err(format!(
                    "Unknown allow subcommand '{action}'. Try: /hooks allow list|add|remove"
                )),
            }
        }
        "set" => {
            let key = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            let value = args.get(2).copied().unwrap_or("");
            if key.is_empty() {
                return Err(
                    "Usage: /hooks set timeout_ms <n> | /hooks set max_output_bytes <n>"
                        .to_string(),
                );
            }
            match key.as_str() {
                "timeout" | "timeout_ms" => {
                    let n: u64 = value
                        .parse()
                        .map_err(|_| "timeout_ms must be an integer".to_string())?;
                    config.hooks.timeout_ms = n;
                    lines.push(format!("Set hooks timeout_ms to {n}"));
                    Ok(HooksOutcome::Changed(lines))
                }
                "max_output" | "max_output_bytes" => {
                    let n: usize = value
                        .parse()
                        .map_err(|_| "max_output_bytes must be an integer".to_string())?;
                    config.hooks.max_output_bytes = n;
                    lines.push(format!("Set hooks max_output_bytes to {n}"));
                    Ok(HooksOutcome::Changed(lines))
                }
                _ => Err(format!(
                    "Unknown key '{key}'. Valid: timeout_ms, max_output_bytes"
                )),
            }
        }
        "create" | "update" => {
            let is_update = sub == "update";
            let Some(name) = args.get(1).copied() else {
                return Err(format!(
                    "Usage: /hooks {} <name> <event> <program> [args...]",
                    if is_update { "update" } else { "create" }
                ));
            };
            let Some(event_str) = args.get(2).copied() else {
                return Err(
                    "Missing <event>. Try: pre_pipeline|post_pipeline|pre_tool|post_tool"
                        .to_string(),
                );
            };
            let Some(program) = args.get(3).copied() else {
                return Err(
                    "Missing <program>. Usage: /hooks create <name> <event> <program> [args...]"
                        .to_string(),
                );
            };
            let event: HookEvent = event_str.parse().map_err(|_: String| {
                format!("Unknown hook event '{event_str}'. Try pre_pipeline|post_pipeline|pre_tool|post_tool")
            })?;
            let cmd = HookCommandTemplate {
                program: program.to_string(),
                args: args
                    .get(4..)
                    .unwrap_or_default()
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect(),
            };

            let idx = config.hooks.hooks.iter().position(|h| h.name == name);
            match (is_update, idx) {
                (false, Some(_)) => Err(format!(
                    "Hook '{name}' already exists. Use /hooks update {name} … or /hooks delete {name}"
                )),
                (true, None) => Err(format!(
                    "Hook '{name}' not found. Use /hooks create {name} …"
                )),
                (true, Some(i)) => {
                    config.hooks.hooks[i].event = event;
                    config.hooks.hooks[i].command = cmd;
                    lines.push(format!("Updated hook: {name}"));
                    lines.push(format!(
                        "Note: program must be allow-listed: /hooks allow add {program}"
                    ));
                    Ok(HooksOutcome::Changed(lines))
                }
                (false, None) => {
                    config.hooks.hooks.push(HookDefinition {
                        name: name.to_string(),
                        event,
                        command: cmd,
                    });
                    lines.push(format!("Created hook: {name}"));
                    lines.push(format!(
                        "Note: program must be allow-listed: /hooks allow add {program}"
                    ));
                    Ok(HooksOutcome::Changed(lines))
                }
            }
        }
        "delete" | "del" | "rm" | "remove" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /hooks delete <name>".to_string());
            };
            let before = config.hooks.hooks.len();
            config.hooks.hooks.retain(|h| h.name != name);
            if config.hooks.hooks.len() == before {
                lines.push(format!("Hook not found: {name}"));
                Ok(HooksOutcome::Unchanged(lines))
            } else {
                lines.push(format!("Deleted hook: {name}"));
                Ok(HooksOutcome::Changed(lines))
            }
        }
        _ => Err(format!(
            "Unknown /hooks subcommand '{sub}'. Try: /hooks help"
        )),
    }
}

fn hooks_usage_lines() -> Vec<String> {
    vec![
        "Hooks commands:".to_string(),
        "  /hooks                     (managed shell in TUI/basic mode)".to_string(),
        "  /hooks show                (print config)".to_string(),
        "  /hooks enable|disable".to_string(),
        "  /hooks allow list".to_string(),
        "  /hooks allow add <program>".to_string(),
        "  /hooks allow remove <program>".to_string(),
        "  /hooks list".to_string(),
        "  /hooks create <name> <event> <program> [args...]".to_string(),
        "  /hooks update <name> <event> <program> [args...]".to_string(),
        "  /hooks delete <name>".to_string(),
        "  /hooks set timeout_ms <n>".to_string(),
        "  /hooks set max_output_bytes <n>".to_string(),
        "Events: pre_pipeline | post_pipeline | pre_tool | post_tool".to_string(),
    ]
}

// ===================== /permissions =====================

pub(crate) struct PermissionsOutcome {
    pub(crate) lines: Vec<String>,
    pub(crate) changed_permissions: bool,
    pub(crate) session_changed: bool,
}

pub(crate) fn run_permissions_subcommand(
    args: &[&str],
    session: &mut AgentSession,
) -> std::result::Result<PermissionsOutcome, String> {
    let sub = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if sub.is_empty() || sub == "help" || sub == "--help" || sub == "-h" {
        return Ok(PermissionsOutcome {
            lines: permissions_usage_lines(),
            changed_permissions: false,
            session_changed: false,
        });
    }

    match sub.as_str() {
        "list" | "ls" => {
            let perms = permission_manager()
                .list()
                .map_err(|e| format!("Failed to list permissions: {e}"))?;
            let mut lines = vec!["━━━ Granted Permissions ━━━".to_string(), String::new()];
            if perms.is_empty() {
                lines.push("No tool permissions have been granted.".to_string());
            } else {
                for perm in &perms {
                    let scope_str = match &perm.scope {
                        gestura_core::PermissionScope::Global => "global".to_string(),
                        gestura_core::PermissionScope::Path(p) => format!("path:{p}"),
                        gestura_core::PermissionScope::Command(c) => format!("cmd:{c}"),
                    };
                    let expires = perm
                        .expires_at
                        .map(|e| e.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "never".to_string());
                    lines.push(format!(
                        "  {}:{} [{}] expires: {}",
                        perm.tool, perm.action, scope_str, expires
                    ));
                }
            }
            lines.push(String::new());
            lines.push("Try: /permissions grant <tool.action> [scope]".to_string());
            lines.push("Try: /permissions revoke <tool.action>".to_string());
            Ok(PermissionsOutcome {
                lines,
                changed_permissions: false,
                session_changed: false,
            })
        }
        "grant" => {
            let (tool, action, scope) = parse_permission_grant_args(args)?;
            permission_manager()
                .grant(&tool, &action, scope, None)
                .map_err(|e| format!("Failed to grant permission: {e}"))?;
            Ok(PermissionsOutcome {
                lines: vec![format!("Granted permission: {tool}.{action}")],
                changed_permissions: true,
                session_changed: false,
            })
        }
        "revoke" => {
            let (tool, action) = parse_permission_tool_action(args.get(1..).unwrap_or_default())?;
            let count = permission_manager()
                .revoke(&tool, &action)
                .map_err(|e| format!("Failed to revoke permission: {e}"))?;
            let msg = if count > 0 {
                format!("Revoked permission: {tool}.{action} ({count} removed)")
            } else {
                format!("No matching permission found: {tool}.{action}")
            };
            Ok(PermissionsOutcome {
                lines: vec![msg],
                changed_permissions: count > 0,
                session_changed: false,
            })
        }
        "reset" => {
            let count = permission_manager()
                .reset()
                .map_err(|e| format!("Failed to reset permissions: {e}"))?;
            Ok(PermissionsOutcome {
                lines: vec![format!("Reset permissions ({count} removed)")],
                changed_permissions: count > 0,
                session_changed: false,
            })
        }
        "check" => {
            let (tool, action, target) = parse_permission_check_args(args)?;
            let check = permission_manager()
                .check(&tool, &action, target.as_deref())
                .map_err(|e| format!("Failed to check permission: {e}"))?;

            let mut lines = Vec::new();
            let target_str = target.as_deref().unwrap_or("-");
            if check.allowed {
                lines.push(format!("ALLOWED: {tool}.{action} [{target_str}]"));
            } else {
                lines.push(format!("DENIED: {tool}.{action} [{target_str}]"));
                lines.push(format!("Reason: {}", check.reason));
            }
            Ok(PermissionsOutcome {
                lines,
                changed_permissions: false,
                session_changed: false,
            })
        }
        "audit" => {
            // /permissions audit [clear]
            let action = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            if action == "clear" {
                let removed = permission_manager()
                    .clear_audit_log()
                    .map_err(|e| format!("Failed to clear audit log: {e}"))?;
                return Ok(PermissionsOutcome {
                    lines: vec![format!("Cleared permission audit log ({removed} entries)")],
                    changed_permissions: false,
                    session_changed: false,
                });
            }

            let log = permission_manager()
                .audit_log()
                .map_err(|e| format!("Failed to load audit log: {e}"))?;
            if log.is_empty() {
                return Ok(PermissionsOutcome {
                    lines: vec!["Permission audit log is empty.".to_string()],
                    changed_permissions: false,
                    session_changed: false,
                });
            }

            let mut lines = vec!["━━━ Permission Audit Log ━━━".to_string(), String::new()];
            for entry in log.iter().rev().take(20) {
                let status = if entry.allowed { "✓" } else { "✗" };
                let res = entry.resource.as_deref().unwrap_or("-");
                lines.push(format!(
                    "  {status} {}:{} [{res}] - {}",
                    entry.tool, entry.action, entry.reason
                ));
            }
            if log.len() > 20 {
                lines.push(format!("  ... and {} more entries", log.len() - 20));
            }
            Ok(PermissionsOutcome {
                lines,
                changed_permissions: false,
                session_changed: false,
            })
        }
        "level" => {
            let action = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            match action.as_str() {
                "" | "show" => {
                    let current = session
                        .state
                        .tool_settings
                        .as_ref()
                        .map(|s| s.permission_level)
                        .unwrap_or_default();
                    Ok(PermissionsOutcome {
                        lines: vec![format!("Session permission level: {current}")],
                        changed_permissions: false,
                        session_changed: false,
                    })
                }
                "set" => {
                    let Some(level_str) = args.get(2).copied() else {
                        return Err(
                            "Usage: /permissions level set <sandbox|restricted|full>".to_string(),
                        );
                    };
                    let level: SessionPermissionLevel = level_str.parse()
                        .map_err(|_: String| format!("Unknown permission level '{level_str}'"))?;
                    let settings = session.state.tool_settings.get_or_insert_with(Default::default);
                    let changed = settings.permission_level != level;
                    settings.permission_level = level;
                    Ok(PermissionsOutcome {
                        lines: vec![format!("Set session permission level -> {level}")],
                        changed_permissions: false,
                        session_changed: changed,
                    })
                }
                _ => Err(
                    "Usage: /permissions level [show] | /permissions level set <sandbox|restricted|full>"
                        .to_string(),
                ),
            }
        }
        _ => Err(format!(
            "Unknown /permissions subcommand '{sub}'. Try: /permissions help"
        )),
    }
}

fn permissions_usage_lines() -> Vec<String> {
    vec![
        "Permissions commands:".to_string(),
        "  /permissions                    (managed shell in TUI/basic mode)".to_string(),
        "  /permissions list".to_string(),
        "  /permissions grant <tool.action> [scope]".to_string(),
        "  /permissions grant <tool> <action> [scope]".to_string(),
        "  /permissions revoke <tool.action>".to_string(),
        "  /permissions revoke <tool> <action>".to_string(),
        "  /permissions reset".to_string(),
        "  /permissions check <read|write|shell|fetch|tool.action> [target]".to_string(),
        "  /permissions check <tool> <action> [target]".to_string(),
        "  /permissions audit [clear]".to_string(),
        "  /permissions level [show]".to_string(),
        "  /permissions level set <sandbox|restricted|full>".to_string(),
        "Scope: omit for global; start with '/' for path scope; otherwise command substring scope"
            .to_string(),
    ]
}

fn parse_permission_grant_args(args: &[&str]) -> Result<(String, String, PermissionScope), String> {
    // /permissions grant <tool.action> [scope]
    // /permissions grant <tool> <action> [scope]
    let rest = args.get(1..).unwrap_or_default();
    let (tool, action, scope_str) = match rest {
        [perm] => {
            let (tool, action) = parse_permission_tool_action(&[*perm])?;
            (tool, action, None)
        }
        [perm, scope] if perm.contains('.') => {
            let (tool, action) = parse_permission_tool_action(&[*perm])?;
            (tool, action, Some(*scope))
        }
        [tool, action] => ((*tool).to_string(), (*action).to_string(), None),
        [tool, action, scope, ..] => ((*tool).to_string(), (*action).to_string(), Some(*scope)),
        _ => {
            return Err(
                "Usage: /permissions grant <tool.action> [scope] OR /permissions grant <tool> <action> [scope]"
                    .to_string(),
            );
        }
    };

    let scope = scope_str
        .map(|s| s.parse::<PermissionScope>().unwrap())
        .unwrap_or(PermissionScope::Global);
    Ok((tool, action, scope))
}

fn parse_permission_tool_action(args: &[&str]) -> Result<(String, String), String> {
    // Accept either: ["tool.action"] or ["tool", "action"]
    match args {
        [one] => {
            let parts: Vec<&str> = one.splitn(2, '.').collect();
            if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
                return Err("Expected 'tool.action' (e.g. 'file.read')".to_string());
            }
            Ok((parts[0].to_string(), parts[1].to_string()))
        }
        [tool, action] => Ok(((*tool).to_string(), (*action).to_string())),
        _ => Err("Expected 'tool.action' or '<tool> <action>'".to_string()),
    }
}

fn parse_permission_check_args(args: &[&str]) -> Result<(String, String, Option<String>), String> {
    // /permissions check <friendly|tool.action> [target]
    // /permissions check <tool> <action> [target]
    let rest = args.get(1..).unwrap_or_default();
    match rest {
        [] => Err(
            "Usage: /permissions check <read|write|shell|fetch|tool.action> [target]".to_string(),
        ),
        [action] => {
            let (tool, action) = map_check_action(action);
            Ok((tool, action, None))
        }
        [action, target] if action.contains('.') => {
            let (tool, action) = parse_permission_tool_action(&[*action])?;
            Ok((tool, action, Some((*target).to_string())))
        }
        [tool, action] if find_tool(tool).is_some() => {
            Ok(((*tool).to_string(), (*action).to_string(), None))
        }
        [tool, action, target, ..] if find_tool(tool).is_some() => Ok((
            (*tool).to_string(),
            (*action).to_string(),
            Some((*target).to_string()),
        )),
        [friendly, target] => {
            let (tool, action) = map_check_action(friendly);
            Ok((tool, action, Some((*target).to_string())))
        }
        _ => Err(
            "Usage: /permissions check <read|write|shell|fetch|tool.action> [target]".to_string(),
        ),
    }
}

fn map_check_action(action: &str) -> (String, String) {
    match action {
        "read" => ("file".to_string(), "read".to_string()),
        "write" => ("file".to_string(), "write".to_string()),
        "delete" => ("file".to_string(), "delete".to_string()),
        "run" | "exec" | "shell" => ("shell".to_string(), "run".to_string()),
        "sudo" => ("shell".to_string(), "sudo".to_string()),
        "git-read" => ("git".to_string(), "read".to_string()),
        "git-write" | "commit" | "push" => ("git".to_string(), "write".to_string()),
        "fetch" | "get" => ("web".to_string(), "fetch".to_string()),
        "post" => ("web".to_string(), "post".to_string()),
        "lint" => ("code".to_string(), "lint".to_string()),
        "test" => ("code".to_string(), "test".to_string()),
        other => {
            let parts: Vec<&str> = other.splitn(2, '.').collect();
            if parts.len() == 2 {
                (parts[0].to_string(), parts[1].to_string())
            } else {
                ("unknown".to_string(), other.to_string())
            }
        }
    }
}

// ===================== /task(s) =====================

#[derive(Debug)]
pub(crate) struct TasksOutcome {
    pub(crate) lines: Vec<String>,
    pub(crate) changed: bool,
    pub(crate) live_action: Option<TasksLiveAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TasksLiveAction {
    ListHierarchy {
        session_id: String,
        workspace_dir: PathBuf,
    },
    ListApprovals {
        session_id: String,
        workspace_dir: PathBuf,
    },
    ListThreads {
        session_id: String,
        workspace_dir: PathBuf,
        include_archived: bool,
    },
    DecideApproval {
        session_id: String,
        workspace_dir: PathBuf,
        task_spec: String,
        actor_kind: ApprovalActorKind,
        decision: TaskApprovalCliDecision,
        note: Option<String>,
    },
    PauseWorkflowTask {
        session_id: String,
        workspace_dir: PathBuf,
        task_spec: String,
    },
    CancelWorkflowTask {
        session_id: String,
        workspace_dir: PathBuf,
        task_spec: String,
    },
    ResumeWorkflowTask {
        session_id: String,
        workspace_dir: PathBuf,
        task_spec: String,
    },
    RestartWorkflowTask {
        session_id: String,
        workspace_dir: PathBuf,
        task_spec: String,
    },
    AcknowledgeBlockedTask {
        session_id: String,
        workspace_dir: PathBuf,
        task_spec: String,
        note: Option<String>,
    },
    CreateCollaboration {
        session_id: String,
        workspace_dir: PathBuf,
        target_spec: String,
        kind: TeamMessageKind,
        note: String,
    },
    UpdateThread {
        session_id: String,
        workspace_dir: PathBuf,
        thread_id: String,
        status: Option<CollaborationActionStatus>,
        archive: bool,
        escalate: bool,
        note: Option<String>,
    },
    CreateChildSupervisorRun {
        session_id: String,
        workspace_dir: PathBuf,
        parent_run_spec: String,
        lead_agent_id: String,
        objective: String,
        name: Option<String>,
        approval_required: bool,
        reviewer_required: bool,
        test_required: bool,
        execution_mode: AgentExecutionMode,
        memory_tags: Vec<String>,
        constraint_notes: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskApprovalCliDecision {
    Approve,
    Reject,
}

pub(crate) fn run_tasks_subcommand(
    args: &[&str],
    manager: &TaskManager,
    session_id: &str,
    workspace_dir: Option<&Path>,
) -> std::result::Result<TasksOutcome, String> {
    let sub = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if sub.is_empty() || sub == "help" || sub == "--help" || sub == "-h" {
        return Ok(TasksOutcome {
            lines: tasks_usage_lines(),
            changed: false,
            live_action: None,
        });
    }

    match sub.as_str() {
        "list" | "ls" => {
            let hierarchy = manager
                .get_hierarchy(session_id)
                .map_err(|e| format!("Failed to load tasks: {e}"))?;
            let lines = format_task_hierarchy(&hierarchy);
            Ok(TasksOutcome {
                lines,
                changed: false,
                live_action: None,
            })
        }
        "create" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /task create <name> [description...]".to_string());
            };
            let desc = args.get(2..).unwrap_or_default().join(" ");
            let task = manager
                .create_task(session_id, name, desc, None)
                .map_err(|e| format!("Failed to create task: {e}"))?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Created task {}: {}",
                    short_id(&task.id),
                    task.name
                )],
                changed: true,
                live_action: None,
            })
        }
        "create-sub" | "sub" => {
            let Some(parent_spec) = args.get(1).copied() else {
                return Err(
                    "Usage: /task create-sub <parent_id> <name> [description...]".to_string(),
                );
            };
            let Some(name) = args.get(2).copied() else {
                return Err(
                    "Usage: /task create-sub <parent_id> <name> [description...]".to_string(),
                );
            };
            let desc = args.get(3..).unwrap_or_default().join(" ");
            let parent_id = resolve_task_id_spec(manager, session_id, parent_spec)?;
            let task = manager
                .create_task(session_id, name, desc, Some(parent_id.clone()))
                .map_err(|e| format!("Failed to create subtask: {e}"))?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Created subtask {} under {}",
                    short_id(&task.id),
                    short_id(&parent_id)
                )],
                changed: true,
                live_action: None,
            })
        }
        "show" => {
            let Some(spec) = args.get(1).copied() else {
                return Err("Usage: /task show <id>".to_string());
            };
            let task_id = resolve_task_id_spec(manager, session_id, spec)?;
            let tasks = manager
                .list_tasks(session_id)
                .map_err(|e| format!("Failed to list tasks: {e}"))?;
            let task = tasks
                .iter()
                .find(|t| t.id == task_id)
                .ok_or_else(|| "Task not found".to_string())?;
            Ok(TasksOutcome {
                lines: format_task_details(task),
                changed: false,
                live_action: None,
            })
        }
        "status" => {
            let Some(spec) = args.get(1).copied() else {
                return Err(
                    "Usage: /task status <id> <not_started|in_progress|completed|cancelled>"
                        .to_string(),
                );
            };
            let Some(status_str) = args.get(2).copied() else {
                return Err(
                    "Usage: /task status <id> <not_started|in_progress|completed|cancelled>"
                        .to_string(),
                );
            };
            let task_id = resolve_task_id_spec(manager, session_id, spec)?;
            let status: TaskStatus = status_str
                .parse()
                .map_err(|_: String| format!("Unknown status '{status_str}'"))?;
            manager
                .update_task_status(session_id, &task_id, status)
                .map_err(|e| format!("Failed to update status: {e}"))?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Set task {} status -> {:?}",
                    short_id(&task_id),
                    status
                )],
                changed: true,
                live_action: None,
            })
        }
        "update" => {
            let Some(spec) = args.get(1).copied() else {
                return Err("Usage: /task update <id> name|desc <value...>".to_string());
            };
            let Some(field) = args.get(2).copied() else {
                return Err("Usage: /task update <id> name|desc <value...>".to_string());
            };
            let value = args.get(3..).unwrap_or_default().join(" ");
            if value.trim().is_empty() {
                return Err("Update value cannot be empty".to_string());
            }
            let task_id = resolve_task_id_spec(manager, session_id, spec)?;
            match field.to_ascii_lowercase().as_str() {
                "name" => manager
                    .update_task(session_id, &task_id, Some(value), None)
                    .map_err(|e| format!("Failed to update task: {e}"))?,
                "desc" | "description" => manager
                    .update_task(session_id, &task_id, None, Some(value))
                    .map_err(|e| format!("Failed to update task: {e}"))?,
                _ => return Err("Field must be 'name' or 'desc'".to_string()),
            }
            Ok(TasksOutcome {
                lines: vec![format!("Updated task {}", short_id(&task_id))],
                changed: true,
                live_action: None,
            })
        }
        "delete" | "del" | "rm" | "remove" => {
            // Destructive: require explicit confirmation.
            // Accept either order:
            //   /task delete --confirmed <id>
            //   /task delete <id> --confirmed
            let mut confirmed = false;
            let mut spec: Option<&str> = None;
            for a in args.iter().skip(1).copied() {
                if a == "--confirmed" {
                    confirmed = true;
                } else if spec.is_none() {
                    spec = Some(a);
                } else {
                    return Err("Usage: /task delete --confirmed <id>".to_string());
                }
            }

            if !confirmed {
                return Err(
                    "Refusing to delete without confirmation. Use: /task delete --confirmed <id>"
                        .to_string(),
                );
            }
            let Some(spec) = spec else {
                return Err("Usage: /task delete --confirmed <id>".to_string());
            };

            let task_id = resolve_task_id_spec(manager, session_id, spec)?;
            let deleted = manager
                .delete_task(session_id, &task_id)
                .map_err(|e| format!("Failed to delete task: {e}"))?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Deleted task {}: {}",
                    short_id(&deleted.id),
                    deleted.name
                )],
                changed: true,
                live_action: None,
            })
        }
        "current" => {
            let action = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            match action.as_str() {
                "" | "show" => {
                    let cur = manager
                        .get_current_task_id(session_id)
                        .map_err(|e| format!("Failed to read current task: {e}"))?;
                    Ok(TasksOutcome {
                        lines: vec![match cur {
                            Some(id) => format!("Current task: {}", short_id(&id)),
                            None => "Current task: (none)".to_string(),
                        }],
                        changed: false,
                        live_action: None,
                    })
                }
                "set" => {
                    let Some(spec) = args.get(2).copied() else {
                        return Err("Usage: /task current set <id>".to_string());
                    };
                    let task_id = resolve_task_id_spec(manager, session_id, spec)?;
                    manager
                        .set_current_task_id(session_id, Some(task_id.clone()))
                        .map_err(|e| format!("Failed to set current task: {e}"))?;
                    Ok(TasksOutcome {
                        lines: vec![format!("Set current task -> {}", short_id(&task_id))],
                        changed: true,
                        live_action: None,
                    })
                }
                "clear" | "unset" => {
                    manager
                        .set_current_task_id(session_id, None)
                        .map_err(|e| format!("Failed to clear current task: {e}"))?;
                    Ok(TasksOutcome {
                        lines: vec!["Cleared current task".to_string()],
                        changed: true,
                        live_action: None,
                    })
                }
                _ => Err(
                    "Usage: /task current [show] | /task current set <id> | /task current clear"
                        .to_string(),
                ),
            }
        }
        "dep" | "deps" | "dependency" => {
            let action = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            if action.as_str() != "add" {
                return Err("Usage: /task dep add <task_id> <blocked_by_id>".to_string());
            }
            let Some(task_spec) = args.get(2).copied() else {
                return Err("Usage: /task dep add <task_id> <blocked_by_id>".to_string());
            };
            let Some(blocked_by_spec) = args.get(3).copied() else {
                return Err("Usage: /task dep add <task_id> <blocked_by_id>".to_string());
            };
            let task_id = resolve_task_id_spec(manager, session_id, task_spec)?;
            let blocked_by_id = resolve_task_id_spec(manager, session_id, blocked_by_spec)?;
            manager
                .add_task_dependency(session_id, &task_id, &blocked_by_id)
                .map_err(|e| format!("Failed to add dependency: {e}"))?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Added dependency: {} blocked by {}",
                    short_id(&task_id),
                    short_id(&blocked_by_id)
                )],
                changed: true,
                live_action: None,
            })
        }
        "tree" | "workflow-tree" | "runs" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot inspect workflow hierarchy."
                        .to_string()
                })?
                .to_path_buf();
            Ok(TasksOutcome {
                lines: vec!["Listing workflow hierarchy…".to_string()],
                changed: false,
                live_action: Some(TasksLiveAction::ListHierarchy {
                    session_id: session_id.to_string(),
                    workspace_dir,
                }),
            })
        }
        "child-run" | "child" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot create child supervisor runs."
                        .to_string()
                })?
                .to_path_buf();
            let child_request = parse_child_supervisor_run_command(args)?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Creating child supervisor run under {} with lead {}…",
                    child_request.parent_run_spec, child_request.lead_agent_id
                )],
                changed: true,
                live_action: Some(TasksLiveAction::CreateChildSupervisorRun {
                    session_id: session_id.to_string(),
                    workspace_dir,
                    parent_run_spec: child_request.parent_run_spec,
                    lead_agent_id: child_request.lead_agent_id,
                    objective: child_request.objective,
                    name: child_request.name,
                    approval_required: child_request.approval_required,
                    reviewer_required: child_request.reviewer_required,
                    test_required: child_request.test_required,
                    execution_mode: child_request.execution_mode,
                    memory_tags: child_request.memory_tags,
                    constraint_notes: child_request.constraint_notes,
                }),
            })
        }
        "threads" | "collaboration" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot inspect workflow collaboration threads."
                        .to_string()
                })?
                .to_path_buf();
            let include_archived = args
                .iter()
                .skip(1)
                .any(|arg| matches!((*arg).to_ascii_lowercase().as_str(), "--all" | "--archived"));
            Ok(TasksOutcome {
                lines: vec!["Listing workflow collaboration threads…".to_string()],
                changed: false,
                live_action: Some(TasksLiveAction::ListThreads {
                    session_id: session_id.to_string(),
                    workspace_dir,
                    include_archived,
                }),
            })
        }
        "message" | "collab" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot create workflow collaboration messages."
                        .to_string()
                })?
                .to_path_buf();
            let Some(target_spec) = args.get(1).copied() else {
                return Err(
                    "Usage: /task message <run_id|task_id> <status_update|clarification|blocker|handoff|review_request|approval_request|test_validation_request> <note...>"
                        .to_string(),
                );
            };
            let Some(kind_raw) = args.get(2).copied() else {
                return Err(
                    "Usage: /task message <run_id|task_id> <status_update|clarification|blocker|handoff|review_request|approval_request|test_validation_request> <note...>"
                        .to_string(),
                );
            };
            let kind = parse_team_message_kind(kind_raw)?;
            let note = args.get(3..).unwrap_or_default().join(" ");
            if note.trim().is_empty() {
                return Err("Collaboration message note cannot be empty.".to_string());
            }
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Queueing workflow collaboration message ({kind_raw})…"
                )],
                changed: true,
                live_action: Some(TasksLiveAction::CreateCollaboration {
                    session_id: session_id.to_string(),
                    workspace_dir,
                    target_spec: target_spec.to_string(),
                    kind,
                    note,
                }),
            })
        }
        "thread" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot update workflow collaboration threads."
                        .to_string()
                })?
                .to_path_buf();
            let Some(action) = args.get(1).copied() else {
                return Err(
                    "Usage: /task thread <ack|resolve|revise|archive|escalate> <thread_id> [note...]"
                        .to_string(),
                );
            };
            let Some(thread_id) = args.get(2).copied() else {
                return Err(
                    "Usage: /task thread <ack|resolve|revise|archive|escalate> <thread_id> [note...]"
                        .to_string(),
                );
            };
            let note = args
                .get(3..)
                .map(|parts| parts.join(" "))
                .filter(|note| !note.trim().is_empty());
            let (status, archive, escalate) = match action.to_ascii_lowercase().as_str() {
                "ack" | "acknowledge" => (Some(CollaborationActionStatus::Acknowledged), false, false),
                "resolve" | "resolved" => (Some(CollaborationActionStatus::Resolved), false, false),
                "revise" | "needs-revision" | "revision" => {
                    (Some(CollaborationActionStatus::NeedsRevision), false, false)
                }
                "archive" => (None, true, false),
                "escalate" => (None, false, true),
                _ => {
                    return Err(
                        "Usage: /task thread <ack|resolve|revise|archive|escalate> <thread_id> [note...]"
                            .to_string(),
                    )
                }
            };
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Queueing workflow thread action '{}' for {}…",
                    action, thread_id
                )],
                changed: true,
                live_action: Some(TasksLiveAction::UpdateThread {
                    session_id: session_id.to_string(),
                    workspace_dir,
                    thread_id: thread_id.to_string(),
                    status,
                    archive,
                    escalate,
                    note,
                }),
            })
        }
        "approvals" | "approval" | "pending-approvals" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot inspect workflow approvals."
                        .to_string()
                })?
                .to_path_buf();
            Ok(TasksOutcome {
                lines: vec!["Listing workflow approvals…".to_string()],
                changed: false,
                live_action: Some(TasksLiveAction::ListApprovals {
                    session_id: session_id.to_string(),
                    workspace_dir,
                }),
            })
        }
        "approve" | "reject" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot update workflow approvals."
                        .to_string()
                })?
                .to_path_buf();
            let decision = if sub == "approve" {
                TaskApprovalCliDecision::Approve
            } else {
                TaskApprovalCliDecision::Reject
            };
            let usage = format!(
                "Usage: /task {sub} <workflow_task_id> [--actor <supervisor|reviewer|tester|user>] [note...]"
            );
            let (task_spec, actor_kind, note) = parse_task_approval_command(args, &usage)?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Submitting {:?} decision for workflow task {} as {}…",
                    decision,
                    task_spec,
                    format_approval_actor_kind(actor_kind)
                )],
                changed: true,
                live_action: Some(TasksLiveAction::DecideApproval {
                    session_id: session_id.to_string(),
                    workspace_dir,
                    task_spec,
                    actor_kind,
                    decision,
                    note,
                }),
            })
        }
        "pause" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot pause workflow tasks.".to_string()
                })?
                .to_path_buf();
            let Some(task_spec) = args.get(1).copied() else {
                return Err("Usage: /task pause <workflow_task_id>".to_string());
            };
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Pausing workflow task {} at the next safe boundary…",
                    task_spec
                )],
                changed: true,
                live_action: Some(TasksLiveAction::PauseWorkflowTask {
                    session_id: session_id.to_string(),
                    workspace_dir,
                    task_spec: task_spec.to_string(),
                }),
            })
        }
        "cancel" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot cancel workflow tasks.".to_string()
                })?
                .to_path_buf();
            let Some(task_spec) = args.get(1).copied() else {
                return Err("Usage: /task cancel <workflow_task_id>".to_string());
            };
            Ok(TasksOutcome {
                lines: vec![format!("Cancelling workflow task {}…", task_spec)],
                changed: true,
                live_action: Some(TasksLiveAction::CancelWorkflowTask {
                    session_id: session_id.to_string(),
                    workspace_dir,
                    task_spec: task_spec.to_string(),
                }),
            })
        }
        "resume" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot resume workflow tasks.".to_string()
                })?
                .to_path_buf();
            let Some(task_spec) = args.get(1).copied() else {
                return Err("Usage: /task resume <workflow_task_id>".to_string());
            };
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Resuming workflow task {} from checkpoint…",
                    task_spec
                )],
                changed: true,
                live_action: Some(TasksLiveAction::ResumeWorkflowTask {
                    session_id: session_id.to_string(),
                    workspace_dir,
                    task_spec: task_spec.to_string(),
                }),
            })
        }
        "restart" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot restart workflow tasks.".to_string()
                })?
                .to_path_buf();
            let Some(task_spec) = args.get(1).copied() else {
                return Err("Usage: /task restart <workflow_task_id>".to_string());
            };
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Restarting workflow task {} from scratch…",
                    task_spec
                )],
                changed: true,
                live_action: Some(TasksLiveAction::RestartWorkflowTask {
                    session_id: session_id.to_string(),
                    workspace_dir,
                    task_spec: task_spec.to_string(),
                }),
            })
        }
        "ack-blocked" | "acknowledge-blocked" => {
            let workspace_dir = workspace_dir
                .ok_or_else(|| {
                    "No workspace directory configured. Cannot acknowledge blocked workflow tasks."
                        .to_string()
                })?
                .to_path_buf();
            let Some(task_spec) = args.get(1).copied() else {
                return Err("Usage: /task ack-blocked <workflow_task_id> [note...]".to_string());
            };
            let note = args
                .get(2..)
                .map(|parts| parts.join(" "))
                .filter(|note| !note.trim().is_empty());
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Acknowledging blocked workflow task {}…",
                    task_spec
                )],
                changed: true,
                live_action: Some(TasksLiveAction::AcknowledgeBlockedTask {
                    session_id: session_id.to_string(),
                    workspace_dir,
                    task_spec: task_spec.to_string(),
                    note,
                }),
            })
        }
        _ => Err(format!("Unknown /task subcommand '{sub}'. Try: /task help")),
    }
}

fn tasks_usage_lines() -> Vec<String> {
    vec![
        "Task commands:".to_string(),
        "  /tasks                    (managed shell in TUI/basic mode)".to_string(),
        "  /task                     (alias for /tasks when no args)".to_string(),
        "  /task list".to_string(),
        "  /task create <name> [description...]".to_string(),
        "  /task create-sub <parent_id> <name> [description...]".to_string(),
        "  /task show <id>".to_string(),
        "  /task update <id> name <new name...>".to_string(),
        "  /task update <id> desc <new description...>".to_string(),
        "  /task status <id> <not_started|in_progress|completed|cancelled>".to_string(),
        "  /task delete --confirmed <id>".to_string(),
        "  /task current [show]".to_string(),
        "  /task current set <id>".to_string(),
        "  /task current clear".to_string(),
        "  /task dep add <task_id> <blocked_by_id>".to_string(),
        "  /task tree".to_string(),
        "  /task child-run <parent_run_id> <lead_agent_id> --objective <text...> [--name <display>] [--mode <shared_workspace|isolated_workspace|git_worktree|remote>] [--approval] [--review] [--test] [--tags <comma,separated>] [--constraint <note>]...".to_string(),
        "  /task threads [--archived]".to_string(),
        "  /task message <run_id|task_id> <kind> <note...>".to_string(),
        "  /task thread <ack|resolve|revise|archive|escalate> <thread_id> [note...]".to_string(),
        "  /task approvals".to_string(),
        "  /task approve <workflow_task_id> [--actor <supervisor|reviewer|tester|user>] [note...]".to_string(),
        "  /task reject <workflow_task_id> [--actor <supervisor|reviewer|tester|user>] [note...]".to_string(),
        "  /task pause <workflow_task_id>".to_string(),
        "  /task cancel <workflow_task_id>".to_string(),
        "  /task resume <workflow_task_id>".to_string(),
        "  /task restart <workflow_task_id>".to_string(),
        "  /task ack-blocked <workflow_task_id> [note...]".to_string(),
        "IDs can be full UUIDs or unique prefixes. Use '.' to refer to current task.".to_string(),
        "Approval commands use delegated workflow task IDs/prefixes and default to actor=supervisor.".to_string(),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedChildSupervisorRunCommand {
    parent_run_spec: String,
    lead_agent_id: String,
    objective: String,
    name: Option<String>,
    approval_required: bool,
    reviewer_required: bool,
    test_required: bool,
    execution_mode: AgentExecutionMode,
    memory_tags: Vec<String>,
    constraint_notes: Vec<String>,
}

fn parse_child_supervisor_run_command(
    args: &[&str],
) -> std::result::Result<ParsedChildSupervisorRunCommand, String> {
    let usage = "Usage: /task child-run <parent_run_id> <lead_agent_id> --objective <text...> [--name <display>] [--mode <shared_workspace|isolated_workspace|git_worktree|remote>] [--approval] [--review] [--test] [--tags <comma,separated>] [--constraint <note>]...";
    let Some(parent_run_spec) = args.get(1).copied() else {
        return Err(usage.to_string());
    };
    let Some(lead_agent_id) = args.get(2).copied() else {
        return Err(usage.to_string());
    };

    let mut index = 3;
    let mut name = None;
    let mut approval_required = false;
    let mut reviewer_required = false;
    let mut test_required = false;
    let mut execution_mode = AgentExecutionMode::SharedWorkspace;
    let mut memory_tags = Vec::new();
    let mut constraint_notes = Vec::new();
    let mut objective: Option<String> = None;

    while index < args.len() {
        match args[index] {
            "--objective" => {
                index += 1;
                let start = index;
                while index < args.len() && !args[index].starts_with("--") {
                    index += 1;
                }
                let value = args[start..index].join(" ");
                if value.trim().is_empty() {
                    return Err("Child supervisor objective cannot be empty.".to_string());
                }
                objective = Some(value);
            }
            "--name" => {
                index += 1;
                let start = index;
                while index < args.len() && !args[index].starts_with("--") {
                    index += 1;
                }
                let value = args[start..index].join(" ");
                if value.trim().is_empty() {
                    return Err("Child supervisor name cannot be empty.".to_string());
                }
                name = Some(value);
            }
            "--mode" => {
                let Some(raw_mode) = args.get(index + 1).copied() else {
                    return Err(usage.to_string());
                };
                execution_mode = parse_agent_execution_mode(raw_mode)?;
                index += 2;
            }
            "--approval" => {
                approval_required = true;
                index += 1;
            }
            "--review" => {
                reviewer_required = true;
                index += 1;
            }
            "--test" => {
                test_required = true;
                index += 1;
            }
            "--tags" => {
                let Some(raw_tags) = args.get(index + 1).copied() else {
                    return Err(usage.to_string());
                };
                memory_tags = raw_tags
                    .split(',')
                    .map(|entry| entry.trim())
                    .filter(|entry| !entry.is_empty())
                    .map(str::to_string)
                    .collect();
                index += 2;
            }
            "--constraint" => {
                index += 1;
                let start = index;
                while index < args.len() && !args[index].starts_with("--") {
                    index += 1;
                }
                let value = args[start..index].join(" ");
                if value.trim().is_empty() {
                    return Err("Constraint note cannot be empty.".to_string());
                }
                constraint_notes.push(value);
            }
            _ => return Err(usage.to_string()),
        }
    }

    let Some(objective) = objective else {
        return Err(format!("{usage} Missing required --objective flag."));
    };

    Ok(ParsedChildSupervisorRunCommand {
        parent_run_spec: parent_run_spec.to_string(),
        lead_agent_id: lead_agent_id.to_string(),
        objective,
        name,
        approval_required,
        reviewer_required,
        test_required,
        execution_mode,
        memory_tags,
        constraint_notes,
    })
}

fn parse_agent_execution_mode(raw: &str) -> std::result::Result<AgentExecutionMode, String> {
    match raw.to_ascii_lowercase().as_str() {
        "shared_workspace" | "shared" => Ok(AgentExecutionMode::SharedWorkspace),
        "isolated_workspace" | "isolated" => Ok(AgentExecutionMode::IsolatedWorkspace),
        "git_worktree" | "worktree" => Ok(AgentExecutionMode::GitWorktree),
        "remote" => Ok(AgentExecutionMode::Remote),
        _ => Err(format!(
            "Unknown execution mode '{raw}'. Use shared_workspace, isolated_workspace, git_worktree, or remote."
        )),
    }
}

fn parse_team_message_kind(kind: &str) -> std::result::Result<TeamMessageKind, String> {
    match kind.to_ascii_lowercase().as_str() {
        "status" | "status_update" => Ok(TeamMessageKind::StatusUpdate),
        "clarification" | "clarify" => Ok(TeamMessageKind::Clarification),
        "blocker" => Ok(TeamMessageKind::Blocker),
        "handoff" => Ok(TeamMessageKind::Handoff),
        "review" | "review_request" => Ok(TeamMessageKind::ReviewRequest),
        "approval" | "approval_request" => Ok(TeamMessageKind::ApprovalRequest),
        "test" | "test_validation" | "test_validation_request" => {
            Ok(TeamMessageKind::TestValidationRequest)
        }
        other => Err(format!(
            "Unknown collaboration kind '{other}'. Use status_update, clarification, blocker, handoff, review_request, approval_request, or test_validation_request."
        )),
    }
}

fn parse_task_approval_command(
    args: &[&str],
    usage: &str,
) -> std::result::Result<(String, ApprovalActorKind, Option<String>), String> {
    let Some(task_spec) = args.get(1).copied() else {
        return Err(usage.to_string());
    };

    let mut actor_kind = ApprovalActorKind::Supervisor;
    let mut note_parts: Vec<&str> = Vec::new();
    let mut i = 2;
    while i < args.len() {
        match args[i] {
            "--actor" | "-a" => {
                let Some(value) = args.get(i + 1).copied() else {
                    return Err(usage.to_string());
                };
                actor_kind = parse_approval_actor_kind(value)?;
                i += 2;
            }
            other => {
                note_parts.push(other);
                i += 1;
            }
        }
    }

    let note = if note_parts.is_empty() {
        None
    } else {
        Some(note_parts.join(" "))
    };

    Ok((task_spec.to_string(), actor_kind, note))
}

fn parse_approval_actor_kind(value: &str) -> std::result::Result<ApprovalActorKind, String> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "user" => Ok(ApprovalActorKind::User),
        "supervisor" => Ok(ApprovalActorKind::Supervisor),
        "reviewer" => Ok(ApprovalActorKind::Reviewer),
        "tester" => Ok(ApprovalActorKind::Tester),
        "system" => Ok(ApprovalActorKind::System),
        other => Err(format!(
            "Unknown approval actor '{other}'. Expected one of: supervisor, reviewer, tester, user"
        )),
    }
}

fn format_approval_actor_kind(kind: ApprovalActorKind) -> &'static str {
    match kind {
        ApprovalActorKind::User => "user",
        ApprovalActorKind::Supervisor => "supervisor",
        ApprovalActorKind::Reviewer => "reviewer",
        ApprovalActorKind::Tester => "tester",
        ApprovalActorKind::System => "system",
    }
}

fn resolve_task_id_spec(
    manager: &TaskManager,
    session_id: &str,
    spec: &str,
) -> std::result::Result<String, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("Task id cannot be empty".to_string());
    }
    let current = manager
        .get_current_task_id(session_id)
        .map_err(|e| format!("Failed to read current task: {e}"))?;
    let tasks = manager
        .list_tasks(session_id)
        .map_err(|e| format!("Failed to list tasks: {e}"))?;
    resolve_task_id_from_list(spec, &tasks, current.as_deref())
}

fn resolve_task_id_from_list(
    spec: &str,
    tasks: &[Task],
    current_id: Option<&str>,
) -> std::result::Result<String, String> {
    if spec == "." || spec.eq_ignore_ascii_case("current") {
        return current_id
            .map(|s| s.to_string())
            .ok_or_else(|| "No current task set".to_string());
    }

    // Exact match wins.
    if tasks.iter().any(|t| t.id == spec) {
        return Ok(spec.to_string());
    }

    let matches: Vec<&Task> = tasks.iter().filter(|t| t.id.starts_with(spec)).collect();
    match matches.len() {
        0 => Err(format!("No task id matches prefix '{spec}'")),
        1 => Ok(matches[0].id.clone()),
        _ => {
            let mut ids: Vec<String> = matches.iter().take(8).map(|t| short_id(&t.id)).collect();
            if matches.len() > 8 {
                ids.push("…".to_string());
            }
            Err(format!(
                "Ambiguous task prefix '{spec}' (matches: {})",
                ids.join(", ")
            ))
        }
    }
}

pub(crate) fn execute_tasks_live_action(
    rt: &tokio::runtime::Runtime,
    action: TasksLiveAction,
) -> std::result::Result<Vec<String>, String> {
    let orchestrator = build_cli_orchestrator(action.workspace_dir());

    match action {
        TasksLiveAction::ListHierarchy {
            session_id,
            workspace_dir,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            Ok(format_supervisor_run_tree_lines(&runs))
        }),
        TasksLiveAction::ListApprovals {
            session_id,
            workspace_dir,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            Ok(format_pending_approval_lines(&runs))
        }),
        TasksLiveAction::ListThreads {
            session_id,
            workspace_dir,
            include_archived,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            Ok(format_collaboration_thread_lines(&orchestrator, &runs, include_archived).await)
        }),
        TasksLiveAction::DecideApproval {
            session_id,
            workspace_dir,
            task_spec,
            actor_kind,
            decision,
            note,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            let (run_id, task_id) = resolve_pending_workflow_task_spec(&task_spec, &runs)?;
            let mut actor = ApprovalActor::new(
                actor_kind,
                format!(
                    "cli:{}:{}",
                    session_id,
                    format_approval_actor_kind(actor_kind)
                ),
            );
            actor.display_name = Some(format!("CLI ({})", format_approval_actor_kind(actor_kind)));

            match decision {
                TaskApprovalCliDecision::Approve => {
                    orchestrator
                        .approve_task(&task_id, actor, note.clone())
                        .await?
                }
                TaskApprovalCliDecision::Reject => {
                    orchestrator
                        .reject_task(&task_id, actor, note.clone())
                        .await?
                }
            }

            let run = orchestrator
                .get_supervisor_run(&run_id)
                .await
                .ok_or_else(|| format!("Workflow run '{}' no longer exists", run_id))?;
            let record = run
                .tasks
                .iter()
                .find(|record| record.task.id == task_id)
                .ok_or_else(|| format!("Workflow task '{}' no longer exists", task_id))?;
            Ok(format_task_approval_decision_lines(
                record,
                decision,
                note.as_deref(),
            ))
        }),
        TasksLiveAction::PauseWorkflowTask {
            session_id,
            workspace_dir,
            task_spec,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            let (_run_id, task_id) = resolve_workflow_task_spec(&task_spec, &runs)?;
            orchestrator.pause_task(&task_id).await?;
            Ok(vec![format!(
                "Requested pause for workflow task {}. It will stop at the next safe checkpoint and remain resumable.",
                task_id
            )])
        }),
        TasksLiveAction::CancelWorkflowTask {
            session_id,
            workspace_dir,
            task_spec,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            let (_run_id, task_id) = resolve_workflow_task_spec(&task_spec, &runs)?;
            orchestrator.cancel_task(&task_id).await?;
            Ok(vec![format!(
                "Requested cancellation for workflow task {}.",
                task_id
            )])
        }),
        TasksLiveAction::ResumeWorkflowTask {
            session_id,
            workspace_dir,
            task_spec,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            let (run_id, task_id) = resolve_workflow_task_spec(&task_spec, &runs)?;
            orchestrator.resume_task_from_checkpoint(&task_id).await?;
            let run = orchestrator
                .get_supervisor_run(&run_id)
                .await
                .ok_or_else(|| format!("Workflow run '{}' no longer exists", run_id))?;
            let record = run
                .tasks
                .iter()
                .find(|record| record.task.id == task_id)
                .ok_or_else(|| format!("Workflow task '{}' no longer exists", task_id))?;
            Ok(format_workflow_checkpoint_action_lines(
                record,
                "Resumed workflow task from checkpoint",
            ))
        }),
        TasksLiveAction::RestartWorkflowTask {
            session_id,
            workspace_dir,
            task_spec,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            let (run_id, task_id) = resolve_workflow_task_spec(&task_spec, &runs)?;
            orchestrator.restart_task_from_scratch(&task_id).await?;
            let run = orchestrator
                .get_supervisor_run(&run_id)
                .await
                .ok_or_else(|| format!("Workflow run '{}' no longer exists", run_id))?;
            let record = run
                .tasks
                .iter()
                .find(|record| record.task.id == task_id)
                .ok_or_else(|| format!("Workflow task '{}' no longer exists", task_id))?;
            Ok(format_workflow_checkpoint_action_lines(
                record,
                "Restarted workflow task from scratch",
            ))
        }),
        TasksLiveAction::AcknowledgeBlockedTask {
            session_id,
            workspace_dir,
            task_spec,
            note,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            let (run_id, task_id) = resolve_workflow_task_spec(&task_spec, &runs)?;
            orchestrator
                .acknowledge_blocked_task(&task_id, note.clone())
                .await?;
            let run = orchestrator
                .get_supervisor_run(&run_id)
                .await
                .ok_or_else(|| format!("Workflow run '{}' no longer exists", run_id))?;
            let record = run
                .tasks
                .iter()
                .find(|record| record.task.id == task_id)
                .ok_or_else(|| format!("Workflow task '{}' no longer exists", task_id))?;
            Ok(format_workflow_checkpoint_action_lines(
                record,
                "Acknowledged blocked workflow task",
            ))
        }),
        TasksLiveAction::CreateCollaboration {
            session_id,
            workspace_dir,
            target_spec,
            kind,
            note,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            let (run_id, task_id) = resolve_workflow_run_or_task_spec(&target_spec, &runs)?;
            let message = orchestrator
                .send_team_message_draft(
                    &run_id,
                    build_cli_collaboration_draft(&session_id, task_id, kind, note),
                )
                .await?;
            Ok(vec![format!(
                "Recorded {} collaboration message in thread {}",
                format_team_message_kind(message.kind),
                message.effective_thread_id()
            )])
        }),
        TasksLiveAction::UpdateThread {
            session_id,
            workspace_dir,
            thread_id,
            status,
            archive,
            escalate,
            note,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            let run_id = resolve_workflow_thread_run_id(&thread_id, &runs)?;
            let lines = if archive {
                let thread = orchestrator
                    .archive_team_thread(
                        &run_id,
                        &thread_id,
                        Some(format!("cli:{session_id}")),
                        note.clone(),
                    )
                    .await?;
                vec![format!(
                    "Archived collaboration thread {} ({:?})",
                    thread.id, thread.status
                )]
            } else if escalate {
                let thread = orchestrator
                    .list_team_threads_with_options(&run_id, true)
                    .await
                    .into_iter()
                    .find(|thread| thread.id == thread_id)
                    .ok_or_else(|| format!("Workflow thread '{}' no longer exists", thread_id))?;
                let latest_message = thread.messages.last().cloned();
                let message = orchestrator
                    .send_team_message_draft(
                        &run_id,
                        TeamMessageDraft {
                            task_id: thread.task_id.clone(),
                            kind: TeamMessageKind::Blocker,
                            sender_agent_id: Some(format!("cli:{session_id}")),
                            recipient_agent_id: None,
                            content: note.clone().unwrap_or_else(|| {
                                format!(
                                    "Escalated collaboration thread {} from the CLI.",
                                    thread_id
                                )
                            }),
                            thread_id: Some(thread.id.clone()),
                            reply_to_message_id: latest_message.map(|message| message.id),
                            action_request: Some(TeamActionRequestDraft {
                                kind: CollaborationRequestKind::BlockerEscalation,
                                requested_for_agent_ids: Vec::new(),
                                requested_for_roles: vec![AgentRole::Supervisor],
                                requested_for_actor_kinds: Vec::new(),
                                approval_scope: None,
                                note: note.clone(),
                            }),
                            escalation: Some(TeamEscalationDraft {
                                level: CollaborationEscalationLevel::Warning,
                                escalated_by_agent_id: Some(format!("cli:{session_id}")),
                                target_role: Some(AgentRole::Supervisor),
                                note: note.clone(),
                            }),
                            unread_by_agent_ids: Vec::new(),
                        },
                    )
                    .await?;
                vec![format!(
                    "Escalated collaboration thread {} with message {}",
                    thread_id,
                    short_id(&message.id)
                )]
            } else {
                let status =
                    status.ok_or_else(|| "Thread action is missing a status".to_string())?;
                let thread = orchestrator
                    .update_team_thread_action(
                        &run_id,
                        &thread_id,
                        status,
                        Some(format!("cli:{session_id}")),
                        note.clone(),
                    )
                    .await?;
                vec![format!(
                    "Updated collaboration thread {} -> {:?}",
                    thread.id, thread.status
                )]
            };
            Ok(lines)
        }),
        TasksLiveAction::CreateChildSupervisorRun {
            session_id,
            workspace_dir,
            parent_run_spec,
            lead_agent_id,
            objective,
            name,
            approval_required,
            reviewer_required,
            test_required,
            execution_mode,
            memory_tags,
            constraint_notes,
        } => rt.block_on(async move {
            let runs = scoped_supervisor_runs(&orchestrator, &session_id, &workspace_dir).await;
            let parent_run = resolve_supervisor_run_spec(&parent_run_spec, &runs)?;
            let child_run = orchestrator
                .create_child_supervisor_run(ChildSupervisorRunRequest {
                    parent_run_id: parent_run.id.clone(),
                    run_id: None,
                    lead_agent_id,
                    objective,
                    name,
                    parent_task_id: None,
                    session_id: parent_run.session_id.clone(),
                    workspace_dir: parent_run.workspace_dir.clone(),
                    approval_required,
                    reviewer_required,
                    test_required,
                    execution_mode,
                    memory_tags,
                    constraint_notes,
                })
                .await?;
            Ok(vec![
                format!(
                    "Created child supervisor run {} under {}",
                    child_run.id, parent_run.id
                ),
                format!(
                    "Lead: {} · Status: {}",
                    child_run.lead_agent_id.as_deref().unwrap_or("unknown"),
                    format_supervisor_run_status(child_run.status)
                ),
            ])
        }),
    }
}

impl TasksLiveAction {
    fn workspace_dir(&self) -> &Path {
        match self {
            Self::ListHierarchy { workspace_dir, .. }
            | Self::ListApprovals { workspace_dir, .. }
            | Self::ListThreads { workspace_dir, .. }
            | Self::DecideApproval { workspace_dir, .. }
            | Self::PauseWorkflowTask { workspace_dir, .. }
            | Self::CancelWorkflowTask { workspace_dir, .. }
            | Self::ResumeWorkflowTask { workspace_dir, .. }
            | Self::RestartWorkflowTask { workspace_dir, .. }
            | Self::AcknowledgeBlockedTask { workspace_dir, .. }
            | Self::CreateCollaboration { workspace_dir, .. }
            | Self::UpdateThread { workspace_dir, .. }
            | Self::CreateChildSupervisorRun { workspace_dir, .. } => workspace_dir.as_path(),
        }
    }
}

fn build_cli_orchestrator(workspace_dir: &Path) -> AgentOrchestrator<AgentManager> {
    AgentOrchestrator::new_with_workspace_root(
        AgentManager::new(AgentManager::default_db_path()),
        AppConfig::load(),
        Some(workspace_dir.to_path_buf()),
    )
}

async fn scoped_supervisor_runs(
    orchestrator: &AgentOrchestrator<AgentManager>,
    session_id: &str,
    workspace_dir: &Path,
) -> Vec<SupervisorRun> {
    orchestrator
        .list_supervisor_runs()
        .await
        .into_iter()
        .filter(|run| supervisor_run_matches_scope(run, session_id, workspace_dir))
        .collect()
}

fn supervisor_run_matches_scope(
    run: &SupervisorRun,
    session_id: &str,
    workspace_dir: &Path,
) -> bool {
    run.session_id.as_deref() == Some(session_id)
        || run.workspace_dir.as_deref() == Some(workspace_dir)
}

fn resolve_pending_workflow_task_spec(
    spec: &str,
    runs: &[SupervisorRun],
) -> std::result::Result<(String, String), String> {
    let pending_records: Vec<(&SupervisorRun, &SupervisorTaskRecord)> = runs
        .iter()
        .flat_map(|run| {
            run.tasks
                .iter()
                .filter(|record| is_pending_approval_record(record))
                .map(move |record| (run, record))
        })
        .collect();

    if let Some((run, record)) = pending_records
        .iter()
        .find(|(_, record)| record.task.id == spec.trim())
        .copied()
    {
        return Ok((run.id.clone(), record.task.id.clone()));
    }

    let matches: Vec<(&SupervisorRun, &SupervisorTaskRecord)> = pending_records
        .into_iter()
        .filter(|(_, record)| record.task.id.starts_with(spec.trim()))
        .collect();

    match matches.len() {
        0 => Err(format!(
            "No pending workflow approval matches task id/prefix '{spec}'"
        )),
        1 => Ok((matches[0].0.id.clone(), matches[0].1.task.id.clone())),
        _ => {
            let mut ids: Vec<String> = matches
                .iter()
                .take(8)
                .map(|(_, record)| record.task.id.clone())
                .collect();
            if matches.len() > 8 {
                ids.push("…".to_string());
            }
            Err(format!(
                "Ambiguous workflow task prefix '{spec}' (matches: {})",
                ids.join(", ")
            ))
        }
    }
}

fn resolve_workflow_run_or_task_spec(
    spec: &str,
    runs: &[SupervisorRun],
) -> std::result::Result<(String, Option<String>), String> {
    let spec = spec.trim();
    if let Some(run) = runs
        .iter()
        .find(|run| run.id == spec || run.id.starts_with(spec))
    {
        return Ok((run.id.clone(), None));
    }

    let matches = runs
        .iter()
        .flat_map(|run| {
            run.tasks.iter().filter_map(move |record| {
                if record.task.id == spec || record.task.id.starts_with(spec) {
                    Some((run.id.clone(), record.task.id.clone()))
                } else {
                    None
                }
            })
        })
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [] => Err(format!("No workflow run or task matched '{spec}'")),
        [(run_id, task_id)] => Ok((run_id.clone(), Some(task_id.clone()))),
        _ => Err(format!(
            "Workflow task id '{spec}' matched multiple runs/tasks"
        )),
    }
}

fn resolve_workflow_task_spec(
    spec: &str,
    runs: &[SupervisorRun],
) -> std::result::Result<(String, String), String> {
    match resolve_workflow_run_or_task_spec(spec, runs)? {
        (run_id, Some(task_id)) => Ok((run_id, task_id)),
        _ => Err(format!(
            "'{spec}' matched a workflow run, but a task id is required"
        )),
    }
}

fn resolve_supervisor_run_spec<'a>(
    spec: &str,
    runs: &'a [SupervisorRun],
) -> std::result::Result<&'a SupervisorRun, String> {
    let spec = spec.trim();
    let matches = runs
        .iter()
        .filter(|run| run.id == spec || run.id.starts_with(spec))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(format!("No workflow run matched '{spec}'")),
        [run] => Ok(*run),
        _ => Err(format!("Workflow run id '{spec}' matched multiple runs")),
    }
}

fn resolve_workflow_thread_run_id(
    thread_id: &str,
    runs: &[SupervisorRun],
) -> std::result::Result<String, String> {
    let matches = runs
        .iter()
        .filter(|run| {
            run.messages
                .iter()
                .any(|message| message.effective_thread_id() == thread_id)
                || run.tasks.iter().any(|record| {
                    record
                        .messages
                        .iter()
                        .any(|message| message.effective_thread_id() == thread_id)
                })
        })
        .map(|run| run.id.clone())
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [] => Err(format!("No workflow thread matched '{thread_id}'")),
        [run_id] => Ok(run_id.clone()),
        _ => Err(format!(
            "Workflow thread '{thread_id}' matched multiple runs"
        )),
    }
}

fn build_cli_collaboration_draft(
    session_id: &str,
    task_id: Option<String>,
    kind: TeamMessageKind,
    note: String,
) -> TeamMessageDraft {
    TeamMessageDraft {
        task_id,
        kind,
        sender_agent_id: Some(format!("cli:{session_id}")),
        recipient_agent_id: None,
        content: note.clone(),
        thread_id: None,
        reply_to_message_id: None,
        action_request: collaboration_request_kind_for_message_kind(kind).map(|request_kind| {
            TeamActionRequestDraft {
                kind: request_kind,
                requested_for_agent_ids: Vec::new(),
                requested_for_roles: default_requested_roles_for_message_kind(kind),
                requested_for_actor_kinds: Vec::new(),
                approval_scope: None,
                note: Some(note.clone()),
            }
        }),
        escalation: if matches!(kind, TeamMessageKind::Blocker) {
            Some(TeamEscalationDraft {
                level: CollaborationEscalationLevel::Warning,
                escalated_by_agent_id: Some(format!("cli:{session_id}")),
                target_role: Some(AgentRole::Supervisor),
                note: Some(note.clone()),
            })
        } else {
            None
        },
        unread_by_agent_ids: Vec::new(),
    }
}

fn collaboration_request_kind_for_message_kind(
    kind: TeamMessageKind,
) -> Option<CollaborationRequestKind> {
    match kind {
        TeamMessageKind::Clarification => Some(CollaborationRequestKind::Clarification),
        TeamMessageKind::Blocker => Some(CollaborationRequestKind::BlockerEscalation),
        TeamMessageKind::Handoff => Some(CollaborationRequestKind::Handoff),
        TeamMessageKind::ReviewRequest => Some(CollaborationRequestKind::ReviewRequest),
        TeamMessageKind::ApprovalRequest => Some(CollaborationRequestKind::ApprovalRequest),
        TeamMessageKind::TestValidationRequest => {
            Some(CollaborationRequestKind::TestValidationRequest)
        }
        TeamMessageKind::StatusUpdate
        | TeamMessageKind::ApprovalDecision
        | TeamMessageKind::ReviewFeedback => None,
    }
}

fn default_requested_roles_for_message_kind(kind: TeamMessageKind) -> Vec<AgentRole> {
    match kind {
        TeamMessageKind::ReviewRequest => vec![AgentRole::Reviewer],
        TeamMessageKind::ApprovalRequest => vec![AgentRole::Supervisor],
        TeamMessageKind::TestValidationRequest => vec![AgentRole::Tester],
        _ => Vec::new(),
    }
}

fn format_team_message_kind(kind: TeamMessageKind) -> &'static str {
    match kind {
        TeamMessageKind::StatusUpdate => "status update",
        TeamMessageKind::ApprovalRequest => "approval request",
        TeamMessageKind::ApprovalDecision => "approval decision",
        TeamMessageKind::ReviewRequest => "review request",
        TeamMessageKind::ReviewFeedback => "review feedback",
        TeamMessageKind::TestValidationRequest => "test validation request",
        TeamMessageKind::Clarification => "clarification",
        TeamMessageKind::Blocker => "blocker",
        TeamMessageKind::Handoff => "handoff",
    }
}

async fn format_collaboration_thread_lines(
    orchestrator: &AgentOrchestrator<AgentManager>,
    runs: &[SupervisorRun],
    include_archived: bool,
) -> Vec<String> {
    let mut lines = vec!["Workflow collaboration threads:".to_string()];
    let mut any = false;

    for run in runs {
        let threads = orchestrator
            .list_team_threads_with_options(&run.id, include_archived)
            .await;
        if threads.is_empty() {
            continue;
        }
        any = true;
        lines.push(format!("- Run {}", short_id(&run.id)));
        for thread in threads {
            lines.extend(format_collaboration_thread_summary(&thread));
        }
    }

    if !any {
        lines.push("- No collaboration threads found in the current workflow scope.".to_string());
    }
    lines
}

fn format_collaboration_thread_summary(thread: &TeamThread) -> Vec<String> {
    let latest_message = thread.messages.last();
    let mut lines = vec![format!(
        "  - {} [{:?}]{}{}",
        short_id(&thread.id),
        thread.status,
        thread
            .task_id
            .as_deref()
            .map(|task_id| format!(" task={}", short_id(task_id)))
            .unwrap_or_default(),
        if thread.archived { " archived" } else { "" },
    )];
    if let Some(message) = latest_message {
        lines.push(format!(
            "    latest: {} — {}",
            format_team_message_kind(message.kind),
            message.content
        ));
    }
    if let Some(request) = thread.latest_action_request.as_ref() {
        lines.push(format!(
            "    request: {:?} ({:?})",
            request.kind, request.status
        ));
    }
    if !thread.artifact_references.is_empty() {
        lines.push(format!(
            "    artifacts: {}",
            thread
                .artifact_references
                .iter()
                .map(|artifact| artifact.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    lines
}

fn format_pending_approval_lines(runs: &[SupervisorRun]) -> Vec<String> {
    let pending_records: Vec<(&SupervisorRun, &SupervisorTaskRecord)> = runs
        .iter()
        .flat_map(|run| {
            run.tasks
                .iter()
                .filter(|record| is_pending_approval_record(record))
                .map(move |record| (run, record))
        })
        .collect();

    let mut lines = vec![
        "━━━ Pending Workflow Approvals ━━━".to_string(),
        String::new(),
    ];

    if pending_records.is_empty() {
        lines.push("No pending workflow approvals for this session/workspace.".to_string());
        return lines;
    }

    for (run, record) in pending_records {
        let scope = record
            .approval
            .scope
            .or_else(|| approval_scope_for_task_state(record.state))
            .map(format_approval_scope)
            .unwrap_or("unknown");
        let requested_by = record
            .approval
            .active_request
            .as_ref()
            .map(|request| {
                request
                    .requested_by
                    .display_name
                    .clone()
                    .unwrap_or_else(|| request.requested_by.id.clone())
            })
            .unwrap_or_else(|| "system".to_string());
        let allowed = allowed_actor_kinds_for_record(record)
            .into_iter()
            .map(format_approval_actor_kind)
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "• {} [{}] run={} state={}",
            record.task.id,
            scope,
            short_id(&run.id),
            format_supervisor_task_state(record.state)
        ));
        lines.push(format!(
            "  Task: {}",
            record
                .task
                .name
                .as_deref()
                .unwrap_or(record.task.prompt.as_str())
        ));
        lines.push(format!("  Requested by: {requested_by}"));
        lines.push(format!("  Allowed actors: {allowed}"));
        if let Some(note) = record.approval.note.as_deref() {
            lines.push(format!("  Note: {note}"));
        }
        lines.push(String::new());
    }

    lines
}

fn format_task_approval_decision_lines(
    record: &SupervisorTaskRecord,
    decision: TaskApprovalCliDecision,
    note: Option<&str>,
) -> Vec<String> {
    let mut lines = vec![format!(
        "{} workflow task {}",
        match decision {
            TaskApprovalCliDecision::Approve => "Approved",
            TaskApprovalCliDecision::Reject => "Requested revision for",
        },
        record.task.id
    )];
    lines.push(format!(
        "Task state: {}",
        format_supervisor_task_state(record.state)
    ));
    lines.push(format!(
        "Approval state: {}",
        format_approval_state(record.approval.state)
    ));
    if let Some(scope) = record
        .approval
        .latest_decision()
        .map(|decision| decision.scope)
        .or(record.approval.scope)
        .or_else(|| approval_scope_for_task_state(record.state))
    {
        lines.push(format!("Gate: {}", format_approval_scope(scope)));
    }
    if let Some(latest) = record.approval.latest_decision() {
        lines.push(format!(
            "Latest decision: {:?} by {} ({})",
            latest.decision,
            latest
                .actor
                .display_name
                .as_deref()
                .unwrap_or(latest.actor.id.as_str()),
            format_approval_actor_kind(latest.actor.kind)
        ));
    }
    if let Some(note) = note.filter(|note| !note.trim().is_empty()) {
        lines.push(format!("Note: {note}"));
    }
    lines
}

fn format_workflow_checkpoint_action_lines(
    record: &SupervisorTaskRecord,
    headline: &str,
) -> Vec<String> {
    let mut lines = vec![format!("{headline}: {}", record.task.id)];
    lines.push(format!(
        "Task state: {}",
        format_supervisor_task_state(record.state)
    ));
    if let Some(checkpoint) = record.checkpoint.as_ref() {
        lines.push(format!(
            "Checkpoint: {} · {} · boundary={} · tool_calls={}",
            format_checkpoint_stage(checkpoint.stage),
            format_resume_disposition(checkpoint.resume_disposition),
            checkpoint.safe_boundary_label,
            checkpoint.completed_tool_call_count
        ));
        lines.push(format!(
            "Replay safety: {} · Resume state: {}",
            format_replay_safety(checkpoint.replay_safety),
            if checkpoint.has_resume_state {
                "available"
            } else {
                "not captured"
            }
        ));
        if !checkpoint.available_actions.is_empty() {
            lines.push(format!(
                "Available actions: {}",
                checkpoint
                    .available_actions
                    .iter()
                    .map(|action| format_checkpoint_action(*action))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(note) = checkpoint.note.as_deref() {
            lines.push(format!("Checkpoint note: {note}"));
        }
    }
    if let Some(line) = local_execution_tree_line(record) {
        lines.push(line.trim_start().to_string());
    }
    if let Some(line) = remote_execution_tree_line(record) {
        lines.push(line.trim_start().to_string());
    }
    if let Some(line) = result_tool_trace_tree_line(record) {
        lines.push(line.trim_start().to_string());
    }
    lines
}

fn is_pending_approval_record(record: &SupervisorTaskRecord) -> bool {
    matches!(
        record.state,
        SupervisorTaskState::PendingApproval
            | SupervisorTaskState::ReviewPending
            | SupervisorTaskState::TestPending
    ) && matches!(record.approval.state, ApprovalState::Pending)
}

fn approval_scope_for_task_state(state: SupervisorTaskState) -> Option<ApprovalScope> {
    match state {
        SupervisorTaskState::PendingApproval => Some(ApprovalScope::PreExecution),
        SupervisorTaskState::ReviewPending => Some(ApprovalScope::Review),
        SupervisorTaskState::TestPending => Some(ApprovalScope::TestValidation),
        _ => None,
    }
}

fn allowed_actor_kinds_for_record(record: &SupervisorTaskRecord) -> Vec<ApprovalActorKind> {
    let Some(scope) = record
        .approval
        .scope
        .or_else(|| approval_scope_for_task_state(record.state))
    else {
        return Vec::new();
    };

    record.approval.allowed_actor_kinds(scope).to_vec()
}

fn format_approval_scope(scope: ApprovalScope) -> &'static str {
    match scope {
        ApprovalScope::PreExecution => "pre_execution",
        ApprovalScope::Review => "review",
        ApprovalScope::TestValidation => "test_validation",
    }
}

fn format_approval_state(state: ApprovalState) -> &'static str {
    match state {
        ApprovalState::NotRequired => "not_required",
        ApprovalState::Pending => "pending",
        ApprovalState::Approved => "approved",
        ApprovalState::Rejected => "rejected",
        ApprovalState::NeedsRevision => "needs_revision",
    }
}

fn format_supervisor_task_state(state: SupervisorTaskState) -> &'static str {
    match state {
        SupervisorTaskState::Queued => "queued",
        SupervisorTaskState::PendingApproval => "pending_approval",
        SupervisorTaskState::Running => "running",
        SupervisorTaskState::ReviewPending => "review_pending",
        SupervisorTaskState::TestPending => "test_pending",
        SupervisorTaskState::Completed => "completed",
        SupervisorTaskState::Cancelled => "cancelled",
        SupervisorTaskState::Failed => "failed",
        SupervisorTaskState::Blocked => "blocked",
    }
}

fn format_checkpoint_stage(stage: DelegatedCheckpointStage) -> &'static str {
    match stage {
        DelegatedCheckpointStage::Queued => "queued",
        DelegatedCheckpointStage::Dispatched => "dispatched",
        DelegatedCheckpointStage::Running => "running",
        DelegatedCheckpointStage::Completed => "completed",
        DelegatedCheckpointStage::Failed => "failed",
        DelegatedCheckpointStage::Cancelled => "cancelled",
        DelegatedCheckpointStage::Blocked => "blocked",
    }
}

fn format_replay_safety(safety: DelegatedReplaySafety) -> &'static str {
    match safety {
        DelegatedReplaySafety::PureReadonly => "pure_readonly",
        DelegatedReplaySafety::IdempotentWrite => "idempotent_write",
        DelegatedReplaySafety::CheckpointResumable => "checkpoint_resumable",
        DelegatedReplaySafety::OperatorGated => "operator_gated",
        DelegatedReplaySafety::NonReplayableSideEffect => "non_replayable_side_effect",
    }
}

fn format_resume_disposition(disposition: DelegatedResumeDisposition) -> &'static str {
    match disposition {
        DelegatedResumeDisposition::ResumeFromCheckpoint => "resume_from_checkpoint",
        DelegatedResumeDisposition::RestartFromBoundary => "restart_from_boundary",
        DelegatedResumeDisposition::OperatorInterventionRequired => {
            "operator_intervention_required"
        }
        DelegatedResumeDisposition::NotApplicable => "not_applicable",
    }
}

fn format_checkpoint_action(action: DelegatedCheckpointAction) -> &'static str {
    match action {
        DelegatedCheckpointAction::ResumeFromCheckpoint => "resume_from_checkpoint",
        DelegatedCheckpointAction::RestartFromScratch => "restart_from_scratch",
        DelegatedCheckpointAction::AcknowledgeBlocked => "acknowledge_blocked",
    }
}

fn format_supervisor_run_status(status: SupervisorRunStatus) -> &'static str {
    match status {
        SupervisorRunStatus::Draft => "draft",
        SupervisorRunStatus::Running => "running",
        SupervisorRunStatus::Waiting => "waiting",
        SupervisorRunStatus::Completed => "completed",
        SupervisorRunStatus::Failed => "failed",
        SupervisorRunStatus::Cancelled => "cancelled",
    }
}

fn format_local_execution_phase(phase: LocalExecutionPhase) -> &'static str {
    match phase {
        LocalExecutionPhase::Queued => "queued",
        LocalExecutionPhase::Running => "running",
        LocalExecutionPhase::Waiting => "waiting",
        LocalExecutionPhase::Blocked => "blocked",
        LocalExecutionPhase::Failed => "failed",
        LocalExecutionPhase::Completed => "completed",
        LocalExecutionPhase::Cancelled => "cancelled",
    }
}

fn format_local_execution_waiting_reason(reason: LocalExecutionWaitingReason) -> &'static str {
    match reason {
        LocalExecutionWaitingReason::ToolConfirmation => "tool_confirmation",
        LocalExecutionWaitingReason::ShellProcess => "shell_process",
        LocalExecutionWaitingReason::Reflection => "reflection",
        LocalExecutionWaitingReason::EnvironmentTransition => "environment_transition",
    }
}

fn local_execution_tree_line(record: &SupervisorTaskRecord) -> Option<String> {
    let progress = record.local_execution.as_ref()?.progress.as_ref()?;
    let mut parts = vec![format!(
        "  • task {} [{}] · local={}",
        short_id(&record.task.id),
        format_supervisor_task_state(record.state),
        format_local_execution_phase(progress.phase)
    )];
    if let Some(reason) = progress.waiting_reason {
        parts.push(format!(
            "waiting={}",
            format_local_execution_waiting_reason(reason)
        ));
    }
    if let Some(stage) = progress.stage.as_deref() {
        parts.push(format!("stage={stage}"));
    }
    if let Some(percent) = progress.percent {
        parts.push(format!("progress={percent}%"));
    }
    if progress.iteration > 0 {
        parts.push(format!("iteration={}", progress.iteration));
    }
    if let Some(tool) = progress.current_tool_name.as_deref() {
        parts.push(format!("current_tool={tool}"));
    }
    if let Some(tool) = progress.last_completed_tool_name.as_deref() {
        parts.push(match progress.last_completed_tool_duration_ms {
            Some(duration_ms) => format!("last_tool={tool}({duration_ms}ms)"),
            None => format!("last_tool={tool}"),
        });
    }
    if progress.completed_tool_call_count > 0 {
        parts.push(format!(
            "completed_calls={}",
            progress.completed_tool_call_count
        ));
    }
    if let Some(message) = progress.message.as_deref() {
        parts.push(format!("message={message}"));
    }
    Some(parts.join(" · "))
}

fn result_tool_trace_tree_line(record: &SupervisorTaskRecord) -> Option<String> {
    let tool_calls = &record.result.as_ref()?.tool_calls;
    if tool_calls.is_empty() {
        return None;
    }

    Some(format!(
        "  • task {} [{}] · tool_trace={}",
        short_id(&record.task.id),
        format_supervisor_task_state(record.state),
        tool_calls
            .iter()
            .take(3)
            .map(|tool_call| format!(
                "{}:{}({}ms)",
                tool_call.tool_name,
                if tool_call.success { "ok" } else { "err" },
                tool_call.duration_ms
            ))
            .collect::<Vec<_>>()
            .join(",")
    ))
}

fn remote_execution_tree_line(record: &SupervisorTaskRecord) -> Option<String> {
    let remote = record.remote_execution.as_ref()?;
    let mut parts = vec![format!(
        "  • task {} [{}] · remote={}",
        short_id(&record.task.id),
        format_supervisor_task_state(record.state),
        remote.status
    )];
    if let Some(reason) = remote.status_reason.as_deref() {
        parts.push(format!("reason={reason}"));
    }
    if let Some(progress) = remote.progress.as_ref() {
        if let Some(percent) = progress.percent {
            parts.push(format!("progress={percent}%"));
        }
        if let Some(stage) = progress.stage.as_deref() {
            parts.push(format!("stage={stage}"));
        }
        if let Some(message) = progress.message.as_deref() {
            parts.push(format!("message={message}"));
        }
    }
    if !remote.artifacts.is_empty() {
        parts.push(format!("artifacts={}", remote.artifacts.len()));
    }
    Some(parts.join(" · "))
}

fn checkpoint_tree_line(record: &SupervisorTaskRecord) -> Option<String> {
    let checkpoint = record.checkpoint.as_ref()?;
    Some(format!(
        "  • task {} [{}] · {} · {} · boundary={}{}",
        short_id(&record.task.id),
        format_supervisor_task_state(record.state),
        format_checkpoint_stage(checkpoint.stage),
        format_resume_disposition(checkpoint.resume_disposition),
        checkpoint.safe_boundary_label,
        if checkpoint.available_actions.is_empty() {
            String::new()
        } else {
            format!(
                " · actions={}",
                checkpoint
                    .available_actions
                    .iter()
                    .map(|action| format_checkpoint_action(*action))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    ))
}

fn format_shared_cognition_kind(kind: SharedCognitionKind) -> &'static str {
    match kind {
        SharedCognitionKind::Discovery => "discovery",
        SharedCognitionKind::Hypothesis => "hypothesis",
        SharedCognitionKind::Steering => "steering",
        SharedCognitionKind::Blocker => "blocker",
        SharedCognitionKind::Decision => "decision",
        SharedCognitionKind::Handoff => "handoff",
    }
}

fn format_shared_cognition_tree_line(run: &SupervisorRun, indent: &str) -> Option<String> {
    if run.shared_cognition.is_empty() {
        return None;
    }

    let latest = run
        .shared_cognition
        .iter()
        .max_by_key(|note| note.created_at)?;
    let hypothesis_count = run
        .shared_cognition
        .iter()
        .filter(|note| matches!(note.kind, SharedCognitionKind::Hypothesis))
        .count();
    let sender = latest.sender_agent_id.as_deref().unwrap_or("unknown");
    let mut summary = format!(
        "{indent}shared cognition: {} notes · latest={} by {} · confidence={}%",
        run.shared_cognition.len(),
        format_shared_cognition_kind(latest.kind),
        sender,
        (latest.confidence * 100.0).round() as u32,
    );
    if hypothesis_count > 0 {
        summary.push_str(&format!(" · open hypotheses={hypothesis_count}"));
    }
    Some(summary)
}

fn format_supervisor_run_tree_lines(runs: &[SupervisorRun]) -> Vec<String> {
    if runs.is_empty() {
        return vec![
            "No workflow runs yet. Create one with /task create or /task child-run.".to_string(),
        ];
    }

    let mut lines = vec!["━━━ Workflow Hierarchy ━━━".to_string(), String::new()];
    let mut roots = runs
        .iter()
        .filter(|run| run.parent_run.is_none())
        .collect::<Vec<_>>();
    roots.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));

    for root in roots {
        lines.push(format!(
            "◆ {} {} [{}] · tasks={} children={} attention={}",
            short_id(&root.id),
            root.name.as_deref().unwrap_or(root.id.as_str()),
            format_supervisor_run_status(
                root.hierarchy_summary
                    .as_ref()
                    .map(|summary| summary.rollup_status)
                    .unwrap_or(root.status)
            ),
            root.task_summary.total,
            root.child_runs.len(),
            root.hierarchy_summary
                .as_ref()
                .map(|summary| summary.action_required_child_count)
                .unwrap_or_default()
        ));
        if let Some(summary) = root.hierarchy_summary.as_ref()
            && summary.descendant_task_count > 0
        {
            lines.push(format!(
                "  descendant tasks: {} · blocked signals: {}",
                summary.descendant_task_count,
                summary.blocked_reasons.join(", ")
            ));
        }
        if let Some(line) = format_shared_cognition_tree_line(root, "  ") {
            lines.push(line);
        }
        for record in root.tasks.iter().filter(|record| {
            record.checkpoint.is_some()
                || record.local_execution.is_some()
                || record.remote_execution.is_some()
                || record
                    .result
                    .as_ref()
                    .is_some_and(|result| !result.tool_calls.is_empty())
        }) {
            if let Some(line) = local_execution_tree_line(record) {
                lines.push(line);
            }
            if let Some(line) = remote_execution_tree_line(record) {
                lines.push(line);
            }
            if let Some(line) = checkpoint_tree_line(record) {
                lines.push(line);
            }
            if let Some(line) = result_tool_trace_tree_line(record) {
                lines.push(line);
            }
        }
        for child in runs.iter().filter(|run| {
            run.parent_run
                .as_ref()
                .is_some_and(|parent| parent.parent_run_id == root.id)
        }) {
            let objective = child
                .parent_run
                .as_ref()
                .map(|parent| parent.objective.as_str())
                .unwrap_or("(no objective)");
            lines.push(format!(
                "  └─ {} {} [{}] · lead={} · tasks={} · objective={}",
                short_id(&child.id),
                child.name.as_deref().unwrap_or(child.id.as_str()),
                format_supervisor_run_status(child.status),
                child.lead_agent_id.as_deref().unwrap_or("unknown"),
                child.task_summary.total,
                objective
            ));
            if let Some(line) = format_shared_cognition_tree_line(child, "    ") {
                lines.push(line);
            }
            for record in child.tasks.iter().filter(|record| {
                record.checkpoint.is_some()
                    || record.local_execution.is_some()
                    || record.remote_execution.is_some()
                    || record
                        .result
                        .as_ref()
                        .is_some_and(|result| !result.tool_calls.is_empty())
            }) {
                if let Some(line) = local_execution_tree_line(record) {
                    lines.push(format!("  {line}"));
                }
                if let Some(line) = remote_execution_tree_line(record) {
                    lines.push(format!("  {line}"));
                }
                if let Some(line) = checkpoint_tree_line(record) {
                    lines.push(format!("  {line}"));
                }
                if let Some(line) = result_tool_trace_tree_line(record) {
                    lines.push(format!("  {line}"));
                }
            }
        }
        lines.push(String::new());
    }

    lines
}

fn format_task_hierarchy(hierarchy: &[(Task, Vec<Task>)]) -> Vec<String> {
    if hierarchy.is_empty() {
        return vec![
            "No tasks yet. Create one with: /task create <name> [description...]".to_string(),
        ];
    }
    let mut lines = vec!["━━━ Tasks ━━━".to_string(), String::new()];
    for (root, subs) in hierarchy {
        lines.push(format!(
            "{} {}  {}",
            status_icon(root.status),
            short_id(&root.id),
            root.name
        ));
        for t in subs {
            lines.push(format!(
                "  {} {}  {}",
                status_icon(t.status),
                short_id(&t.id),
                t.name
            ));
        }
    }
    lines
}

fn format_task_details(task: &Task) -> Vec<String> {
    let mut lines = vec!["━━━ Task ━━━".to_string(), String::new()];
    lines.push(format!("ID: {}", task.id));
    lines.push(format!("Name: {}", task.name));
    lines.push(format!("Status: {:?}", task.status));
    if let Some(background) = &task.background_job {
        lines.push(format!("Background: {:?}", background.status));
        if let Some(message) = &background.message {
            lines.push(format!("Background message: {message}"));
        }
    }
    if let Some(parent) = &task.parent_id {
        lines.push(format!("Parent: {}", short_id(parent)));
    }
    if !task.blocked_by.is_empty() {
        lines.push(format!(
            "Blocked by: {}",
            task.blocked_by
                .iter()
                .map(|id| short_id(id))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if let Some(metadata) = &task.metadata
        && let Some(delegation) = metadata.get("delegation")
    {
        if let Some(run_id) = delegation.get("run_id").and_then(|value| value.as_str()) {
            lines.push(format!("Run: {run_id}"));
        }
        if let Some(agent_id) = delegation.get("agent_id").and_then(|value| value.as_str()) {
            lines.push(format!("Agent: {agent_id}"));
        }
        if let Some(approval) = delegation.get("approval") {
            if let Some(scope) = approval.get("scope").and_then(|value| value.as_str()) {
                lines.push(format!("Approval gate: {scope}"));
            }
            if let Some(requested_by) = approval
                .get("active_request")
                .and_then(|value| value.get("requested_by"))
                .and_then(|value| value.get("id"))
                .and_then(|value| value.as_str())
            {
                lines.push(format!("Approval requested by: {requested_by}"));
            }
            if let Some(decision) = approval.get("latest_decision") {
                let decision_kind = decision
                    .get("decision")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                let actor = decision
                    .get("actor")
                    .and_then(|value| value.get("id"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                lines.push(format!(
                    "Latest approval decision: {decision_kind} by {actor}"
                ));
            }
        }
        if let Some(environment) = delegation.get("environment") {
            let environment_id = environment
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let mode = environment
                .get("mode")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let state = environment
                .get("state")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let health = environment
                .get("health")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            lines.push(format!("Environment: {environment_id} ({mode})"));
            lines.push(format!("Environment state: {state} / {health}"));
            if let Some(action) = environment
                .get("recovery_action")
                .and_then(|value| value.as_str())
            {
                lines.push(format!("Recovery action: {action}"));
            }
            if let Some(path) = environment
                .get("worktree_path")
                .and_then(|value| value.as_str())
                .or_else(|| environment.get("root_dir").and_then(|value| value.as_str()))
            {
                lines.push(format!("Environment path: {path}"));
            }
            if let Some(message) = environment
                .get("failure")
                .and_then(|value| value.get("message"))
                .and_then(|value| value.as_str())
            {
                lines.push(format!("Environment failure: {message}"));
            }
        }
    }

    lines.push(String::new());
    lines.push("Description:".to_string());
    lines.push(task.description.clone());
    lines
}

fn status_icon(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::NotStarted => "[ ]",
        TaskStatus::Blocked => "[!]",
        TaskStatus::InProgress => "[/]",
        TaskStatus::Completed => "[x]",
        TaskStatus::Cancelled => "[-]",
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

// ===================== /memory =====================

#[derive(Debug)]
pub(crate) struct MemoryOutcome {
    pub(crate) lines: Vec<String>,
    pub(crate) changed: bool,
    pub(crate) live_action: Option<MemoryLiveAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MemoryLiveAction {
    List,
    Search { query: String, limit: usize },
    Save { entry: Box<MemoryBankEntry> },
    ClearAll,
    Delete { file_path: PathBuf },
}

pub(crate) fn run_memory_subcommand(
    args: &[&str],
    session: &AgentSession,
) -> std::result::Result<MemoryOutcome, String> {
    let sub = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if sub.is_empty() || sub == "help" || sub == "--help" || sub == "-h" {
        return Ok(MemoryOutcome {
            lines: memory_usage_lines(),
            changed: false,
            live_action: None,
        });
    }

    let workspace_dir = session.workspace_dir().ok_or_else(|| {
        "No workspace directory configured. Cannot access memory bank.".to_string()
    })?;

    match sub.as_str() {
        "list" | "ls" => Ok(MemoryOutcome {
            lines: vec!["Listing memory bank entries…".to_string()],
            changed: false,
            live_action: Some(MemoryLiveAction::List),
        }),
        "search" => {
            // Flags: --limit <n>
            let mut limit: usize = 10;
            let mut query_parts: Vec<&str> = Vec::new();

            let mut i = 1;
            while i < args.len() {
                match args[i] {
                    "--limit" | "-l" => {
                        let Some(v) = args.get(i + 1).copied() else {
                            return Err("Usage: /memory search <query> [--limit <n>]".to_string());
                        };
                        limit = v
                            .parse::<usize>()
                            .map_err(|_| format!("Invalid --limit value: '{v}'"))?;
                        i += 2;
                    }
                    other => {
                        query_parts.push(other);
                        i += 1;
                    }
                }
            }

            let query = query_parts.join(" ").trim().to_string();
            if query.is_empty() {
                return Err("Usage: /memory search <query> [--limit <n>]".to_string());
            }

            Ok(MemoryOutcome {
                lines: vec![format!("Searching memory bank for '{query}'…")],
                changed: false,
                live_action: Some(MemoryLiveAction::Search { query, limit }),
            })
        }
        "save" => {
            // Flags:
            // - --summary <text>
            // - --category/-c <name>
            // - --last <n>
            let mut summary_override: Option<String> = None;
            let mut category: Option<String> = None;
            let mut last_n: Option<usize> = None;

            let mut i = 1;
            while i < args.len() {
                match args[i] {
                    "--summary" => {
                        let Some(v) = args.get(i + 1).copied() else {
                            return Err("Usage: /memory save [--summary <text>] [--category <name>] [--last <n>]".to_string());
                        };
                        summary_override = Some(v.to_string());
                        i += 2;
                    }
                    "--category" | "-c" => {
                        let Some(v) = args.get(i + 1).copied() else {
                            return Err("Usage: /memory save [--summary <text>] [--category <name>] [--last <n>]".to_string());
                        };
                        category = Some(v.to_string());
                        i += 2;
                    }
                    "--last" => {
                        let Some(v) = args.get(i + 1).copied() else {
                            return Err("Usage: /memory save [--summary <text>] [--category <name>] [--last <n>]".to_string());
                        };
                        last_n = Some(
                            v.parse::<usize>()
                                .map_err(|_| format!("Invalid --last value: '{v}'"))?,
                        );
                        i += 2;
                    }
                    other => {
                        return Err(format!(
                            "Unknown flag for /memory save: '{other}'. Try: --summary, --category, --last"
                        ));
                    }
                }
            }

            let mut history: Vec<String> = session
                .state
                .messages
                .iter()
                .map(|m| m.content.clone())
                .collect();
            if let Some(n) = last_n {
                if n == 0 {
                    history.clear();
                } else if history.len() > n {
                    history = history.split_off(history.len().saturating_sub(n));
                }
            }

            if history.is_empty() {
                return Err("No conversation history to save.".to_string());
            }

            let summary = summary_override
                .unwrap_or_else(|| ContextManager::new().summarize_history(&history));
            let content = history.join("\n\n");

            let mut entry = gestura_core::memory_bank::MemoryBankEntry::new(
                session.id.clone(),
                summary,
                content,
            );
            if let Some(cat) = category {
                entry = entry.with_category(cat);
            }

            Ok(MemoryOutcome {
                lines: vec!["Saving conversation to memory bank…".to_string()],
                changed: true,
                live_action: Some(MemoryLiveAction::Save {
                    entry: Box::new(entry),
                }),
            })
        }
        "clear" => {
            let confirmed = args.contains(&"--confirmed");
            if !confirmed {
                return Err(
                    "Refusing to clear without confirmation. Use: /memory clear --confirmed"
                        .to_string(),
                );
            }
            Ok(MemoryOutcome {
                lines: vec!["Clearing memory bank…".to_string()],
                changed: true,
                live_action: Some(MemoryLiveAction::ClearAll),
            })
        }
        "delete" => {
            let mut confirmed = false;
            let mut path_arg: Option<&str> = None;
            for a in args.iter().skip(1).copied() {
                if a == "--confirmed" {
                    confirmed = true;
                } else {
                    path_arg = Some(a);
                }
            }

            if !confirmed {
                return Err(
                    "Refusing to delete without confirmation. Use: /memory delete --confirmed <path>"
                        .to_string(),
                );
            }

            let Some(path_str) = path_arg else {
                return Err("Usage: /memory delete --confirmed <path>".to_string());
            };

            let input_path = std::path::Path::new(path_str);
            let resolved = if input_path.is_absolute() {
                input_path.to_path_buf()
            } else {
                workspace_dir.join(input_path)
            };

            Ok(MemoryOutcome {
                lines: vec![format!("Deleting memory entry: {path_str}")],
                changed: true,
                live_action: Some(MemoryLiveAction::Delete {
                    file_path: resolved,
                }),
            })
        }
        other => Err(format!(
            "Unknown /memory subcommand: '{other}'. Try: list, search, save, clear, delete"
        )),
    }
}

fn memory_usage_lines() -> Vec<String> {
    vec![
        "━━━ /memory ━━━".to_string(),
        String::new(),
        "Managed memory shell: /memory".to_string(),
        String::new(),
        "Quick commands:".to_string(),
        "  /memory list".to_string(),
        "  /memory search <query> [--limit <n>]".to_string(),
        "  /memory save [--summary <text>] [--category <name>] [--last <n>]".to_string(),
        "  /memory delete --confirmed <path>".to_string(),
        "  /memory clear --confirmed".to_string(),
        String::new(),
        "Open /memory with no subcommand to browse Overview, Search, Working, Durable, Promotions, Task Memory, and Maintenance.".to_string(),
        "Destructive actions require --confirmed (or use the interactive UI).".to_string(),
    ]
}

// ===================== /mcp =====================

#[derive(Debug)]
pub(crate) struct McpOutcome {
    pub(crate) lines: Vec<String>,
    pub(crate) changed: bool,
    /// Live actions that must be executed by a caller that has a Tokio runtime
    /// and access to the MCP registry.
    pub(crate) live_action: Option<McpLiveAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum McpLiveAction {
    Status,
    Tools { server: Option<String> },
    Connect { name: String },
    Disconnect { name: String },
}

pub(crate) fn run_mcp_subcommand(
    args: &[&str],
    config: &mut AppConfig,
) -> std::result::Result<McpOutcome, String> {
    let sub = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if sub.is_empty() || sub == "help" || sub == "--help" || sub == "-h" {
        return Ok(McpOutcome {
            lines: mcp_usage_lines(),
            changed: false,
            live_action: None,
        });
    }

    match sub.as_str() {
        "list" | "ls" => Ok(McpOutcome {
            lines: mcp_list_lines(config),
            changed: false,
            live_action: None,
        }),
        "get" | "show" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp get <name>".to_string());
            };
            let srv = config
                .find_mcp_server(name)
                .ok_or_else(|| format!("MCP server '{name}' not found"))?;
            Ok(McpOutcome {
                lines: mcp_server_details_lines(srv),
                changed: false,
                live_action: None,
            })
        }
        "enable" | "disable" => {
            let Some(name) = args.get(1).copied() else {
                return Err(format!("Usage: /mcp {sub} <name>"));
            };
            let enabled = sub == "enable";
            let srv = config
                .find_mcp_server_mut(name)
                .ok_or_else(|| format!("MCP server '{name}' not found"))?;
            let changed = srv.enabled != enabled;
            srv.enabled = enabled;
            Ok(McpOutcome {
                lines: vec![format!(
                    "{} MCP server: {name}",
                    if enabled { "Enabled" } else { "Disabled" }
                )],
                changed,
                live_action: None,
            })
        }
        "remove" | "rm" | "delete" | "del" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp remove <name>".to_string());
            };
            let before = config.mcp_servers.len();
            config.mcp_servers.retain(|s| s.name != name);
            let changed = config.mcp_servers.len() != before;
            if changed {
                Ok(McpOutcome {
                    lines: vec![format!("Removed MCP server: {name}")],
                    changed: true,
                    live_action: None,
                })
            } else {
                Ok(McpOutcome {
                    lines: vec![format!("MCP server not found: {name}")],
                    changed: false,
                    live_action: None,
                })
            }
        }
        "add" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp add <name> [endpoint] [flags...]".to_string());
            };
            if config.mcp_servers.iter().any(|s| s.name == name) {
                return Err(format!(
                    "MCP server '{name}' already exists. Use: /mcp edit {name} ..."
                ));
            }

            let (pos_endpoint, rest) = split_positional_endpoint(args.get(2..).unwrap_or_default());
            let parsed = McpParsedArgs::parse(rest)?;
            let transport = parsed
                .transport
                .or_else(|| infer_transport_from_endpoint(pos_endpoint.as_deref()))
                .unwrap_or(McpTransportType::Stdio);

            let mut entry = McpServerEntry {
                name: name.to_string(),
                transport,
                enabled: parsed.enabled.unwrap_or(true),
                scope: parsed.scope.unwrap_or(McpScope::User),
                timeout_secs: parsed.timeout_secs.unwrap_or(30),
                auto_reconnect: parsed.auto_reconnect.unwrap_or(true),
                ..McpServerEntry::default()
            };

            apply_transport_specific_add_fields(&mut entry, transport, &parsed, pos_endpoint)?;
            validate_mcp_entry(&entry)?;

            config.mcp_servers.push(entry);
            Ok(McpOutcome {
                lines: vec![format!("Added MCP server: {name}")],
                changed: true,
                live_action: None,
            })
        }
        "edit" | "update" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp edit <name> [endpoint] [flags...]".to_string());
            };

            let (pos_endpoint, rest) = split_positional_endpoint(args.get(2..).unwrap_or_default());
            let parsed = McpParsedArgs::parse(rest)?;

            let srv = config
                .find_mcp_server_mut(name)
                .ok_or_else(|| format!("MCP server '{name}' not found"))?;
            let before = srv.clone();
            apply_mcp_edit_patch(srv, &parsed, pos_endpoint)?;
            validate_mcp_entry(srv)?;
            let changed = *srv != before;

            Ok(McpOutcome {
                lines: vec![format!("Updated MCP server: {name}")],
                changed,
                live_action: None,
            })
        }
        // Live / runtime-backed operations. We parse + validate inputs here,
        // but a caller must actually execute them.
        "status" => Ok(McpOutcome {
            lines: vec!["Fetching MCP status...".to_string()],
            changed: false,
            live_action: Some(McpLiveAction::Status),
        }),
        "tools" => {
            let server = args.get(1).copied().map(|s| s.to_string());
            Ok(McpOutcome {
                lines: vec!["Fetching MCP tools...".to_string()],
                changed: false,
                live_action: Some(McpLiveAction::Tools { server }),
            })
        }
        "connect" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp connect <name>".to_string());
            };
            // Ensure server exists (and surfaces a good error message) even though
            // the actual connect is performed by the caller.
            let _ = config
                .find_mcp_server(name)
                .ok_or_else(|| format!("MCP server '{name}' not found"))?;
            Ok(McpOutcome {
                lines: vec![format!("Connecting to MCP server: {name}...")],
                changed: false,
                live_action: Some(McpLiveAction::Connect {
                    name: name.to_string(),
                }),
            })
        }
        "disconnect" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp disconnect <name>".to_string());
            };
            Ok(McpOutcome {
                lines: vec![format!("Disconnecting from MCP server: {name}...")],
                changed: false,
                live_action: Some(McpLiveAction::Disconnect {
                    name: name.to_string(),
                }),
            })
        }
        _ => Err(format!("Unknown /mcp subcommand '{sub}'. Try: /mcp help")),
    }
}

fn mcp_usage_lines() -> Vec<String> {
    vec![
        "MCP commands:".to_string(),
        "  /mcp                     (managed shell in TUI/basic mode)".to_string(),
        "  /mcp list".to_string(),
        "  /mcp get <name>".to_string(),
        "  /mcp add <name> [endpoint] [flags...]".to_string(),
        "  /mcp edit <name> [endpoint] [flags...]".to_string(),
        "  /mcp remove <name>".to_string(),
        "  /mcp enable <name>".to_string(),
        "  /mcp disable <name>".to_string(),
        "  /mcp status              (requires runtime)".to_string(),
        "  /mcp tools [server]      (requires runtime)".to_string(),
        "  /mcp connect <name>      (requires runtime)".to_string(),
        "  /mcp disconnect <name>   (requires runtime)".to_string(),
        "".to_string(),
        "Flags (add/edit):".to_string(),
        "  --transport|-t stdio|http|sse".to_string(),
        "  --scope|-s user|project|local".to_string(),
        "  --timeout <secs>".to_string(),
        "  --auto-reconnect | --no-auto-reconnect".to_string(),
        "  --enabled | --disabled".to_string(),
        "".to_string(),
        "Stdio flags:".to_string(),
        "  --command <cmd>   (or positional endpoint)".to_string(),
        "  --arg <value>     (repeatable)".to_string(),
        "  --env KEY=VALUE   (repeatable)".to_string(),
        "".to_string(),
        "HTTP/SSE flags:".to_string(),
        "  --url <url>       (or positional endpoint)".to_string(),
        "  --header K:V      (repeatable)".to_string(),
        "".to_string(),
        "Edit-only helpers:".to_string(),
        "  --clear-args | --clear-env | --clear-headers".to_string(),
    ]
}

fn mcp_list_lines(config: &AppConfig) -> Vec<String> {
    if config.mcp_servers.is_empty() {
        return vec![
            "No MCP servers configured.".to_string(),
            "Add one with: /mcp (managed shell)".to_string(),
        ];
    }
    let mut lines = vec!["━━━ MCP Servers ━━━".to_string(), String::new()];
    for srv in &config.mcp_servers {
        let status = if srv.enabled { "✓" } else { "○" };
        lines.push(format!(
            "{status} {:<20} [{:<5}] {:<7} {}",
            srv.name,
            format!("{}", srv.transport),
            format!("{}", srv.scope),
            mcp_endpoint_hint(srv)
        ));
    }
    lines
}

fn mcp_endpoint_hint(srv: &McpServerEntry) -> String {
    match srv.transport {
        McpTransportType::Stdio => {
            let cmd = srv.command.as_deref().unwrap_or("(no command)");
            let args = if srv.args.is_empty() {
                "".to_string()
            } else {
                format!(" {}", srv.args.join(" "))
            };
            format!("{}{}", cmd, args)
        }
        McpTransportType::Http | McpTransportType::Sse => {
            srv.url.as_deref().unwrap_or("(no url)").to_string()
        }
    }
}

fn mcp_server_details_lines(srv: &McpServerEntry) -> Vec<String> {
    let mut lines = vec!["━━━ MCP Server ━━━".to_string(), String::new()];
    lines.push(format!("Name:           {}", srv.name));
    lines.push(format!("Transport:      {}", srv.transport));
    lines.push(format!("Scope:          {}", srv.scope));
    lines.push(format!(
        "Enabled:        {}",
        if srv.enabled { "yes" } else { "no" }
    ));
    lines.push(format!("Timeout:        {}s", srv.timeout_secs));
    lines.push(format!(
        "Auto-reconnect: {}",
        if srv.auto_reconnect { "yes" } else { "no" }
    ));
    lines.push(String::new());
    match srv.transport {
        McpTransportType::Stdio => {
            lines.push(format!(
                "Command:        {}",
                srv.command.as_deref().unwrap_or("(none)")
            ));
            if !srv.args.is_empty() {
                lines.push(format!("Args:           {}", srv.args.join(" ")));
            }
            if !srv.env.is_empty() {
                lines.push(format!("Env:            {} vars", srv.env.len()));
            }
        }
        McpTransportType::Http | McpTransportType::Sse => {
            lines.push(format!(
                "URL:            {}",
                srv.url.as_deref().unwrap_or("(none)")
            ));
            if !srv.headers.is_empty() {
                lines.push(format!("Headers:        {}", srv.headers.len()));
            }
        }
    }
    lines
}

fn split_positional_endpoint<'a>(rest: &'a [&'a str]) -> (Option<String>, &'a [&'a str]) {
    let Some(first) = rest.first().copied() else {
        return (None, rest);
    };
    if first.starts_with('-') {
        (None, rest)
    } else {
        (Some(first.to_string()), &rest[1..])
    }
}

#[derive(Default, Debug, Clone)]
struct McpParsedArgs {
    transport: Option<McpTransportType>,
    scope: Option<McpScope>,
    timeout_secs: Option<u64>,
    auto_reconnect: Option<bool>,
    enabled: Option<bool>,

    command: Option<String>,
    url: Option<String>,
    args: Option<Vec<String>>,
    env: std::collections::HashMap<String, String>,
    headers: std::collections::HashMap<String, String>,

    clear_args: bool,
    clear_env: bool,
    clear_headers: bool,
}

impl McpParsedArgs {
    fn parse(rest: &[&str]) -> Result<Self, String> {
        let mut out = Self::default();
        let mut i = 0;
        while i < rest.len() {
            match rest[i] {
                "--transport" | "-t" => {
                    let val = rest.get(i + 1).copied().ok_or_else(|| {
                        "Missing value for --transport. Try: --transport stdio|http|sse".to_string()
                    })?;
                    out.transport = Some(parse_mcp_transport(val)?);
                    i += 2;
                }
                "--scope" | "-s" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --scope".to_string())?;
                    out.scope = Some(parse_mcp_scope(val)?);
                    i += 2;
                }
                "--timeout" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --timeout".to_string())?;
                    out.timeout_secs = Some(val.parse::<u64>().map_err(|_| {
                        "--timeout must be an integer number of seconds".to_string()
                    })?);
                    i += 2;
                }
                "--auto-reconnect" => {
                    out.auto_reconnect = Some(true);
                    i += 1;
                }
                "--no-auto-reconnect" => {
                    out.auto_reconnect = Some(false);
                    i += 1;
                }
                "--enabled" => {
                    out.enabled = Some(true);
                    i += 1;
                }
                "--disabled" => {
                    out.enabled = Some(false);
                    i += 1;
                }
                "--command" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --command".to_string())?;
                    out.command = Some(val.to_string());
                    i += 2;
                }
                "--url" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --url".to_string())?;
                    out.url = Some(val.to_string());
                    i += 2;
                }
                "--arg" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --arg".to_string())?;
                    out.args.get_or_insert_with(Vec::new).push(val.to_string());
                    i += 2;
                }
                "--env" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --env".to_string())?;
                    let (k, v) = parse_key_value(val, '=')
                        .ok_or_else(|| "--env must be KEY=VALUE".to_string())?;
                    out.env.insert(k, v);
                    i += 2;
                }
                "--header" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --header".to_string())?;
                    let (k, v) = parse_key_value(val, ':')
                        .or_else(|| parse_key_value(val, '='))
                        .ok_or_else(|| "--header must be 'Key: Value'".to_string())?;
                    out.headers.insert(k, v);
                    i += 2;
                }
                "--clear-args" => {
                    out.clear_args = true;
                    i += 1;
                }
                "--clear-env" => {
                    out.clear_env = true;
                    i += 1;
                }
                "--clear-headers" => {
                    out.clear_headers = true;
                    i += 1;
                }
                other if other.starts_with('-') => {
                    return Err(format!("Unknown flag '{other}'. Try: /mcp help"));
                }
                other => {
                    return Err(format!(
                        "Unexpected positional argument '{other}'. If you meant an endpoint, it must come immediately after the name."
                    ));
                }
            }
        }
        Ok(out)
    }
}

fn parse_key_value(input: &str, sep: char) -> Option<(String, String)> {
    let (k, v) = input.split_once(sep)?;
    let k = k.trim();
    let v = v.trim();
    if k.is_empty() {
        return None;
    }
    Some((k.to_string(), v.to_string()))
}

fn parse_mcp_transport(s: &str) -> Result<McpTransportType, String> {
    s.parse::<McpTransportType>()
        .map_err(|e| format!("Invalid transport '{s}': {e}"))
}

fn parse_mcp_scope(s: &str) -> Result<McpScope, String> {
    s.parse::<McpScope>()
        .map_err(|e| format!("Invalid scope '{s}': {e}"))
}

fn apply_transport_specific_add_fields(
    entry: &mut McpServerEntry,
    transport: McpTransportType,
    parsed: &McpParsedArgs,
    pos_endpoint: Option<String>,
) -> Result<(), String> {
    match transport {
        McpTransportType::Stdio => {
            entry.command = parsed.command.clone().or(pos_endpoint);
            entry.args = parsed.args.clone().unwrap_or_default();
            entry.env = parsed.env.clone();
        }
        McpTransportType::Http | McpTransportType::Sse => {
            entry.url = parsed.url.clone().or(pos_endpoint);
            entry.headers = parsed.headers.clone();
        }
    }
    Ok(())
}

fn apply_mcp_edit_patch(
    srv: &mut McpServerEntry,
    parsed: &McpParsedArgs,
    pos_endpoint: Option<String>,
) -> Result<(), String> {
    if let Some(scope) = parsed.scope {
        srv.scope = scope;
    }
    if let Some(timeout) = parsed.timeout_secs {
        srv.timeout_secs = timeout;
    }
    if let Some(enabled) = parsed.enabled {
        srv.enabled = enabled;
    }
    if let Some(ar) = parsed.auto_reconnect {
        srv.auto_reconnect = ar;
    }

    if let Some(new_transport) = parsed.transport
        && srv.transport != new_transport
    {
        srv.transport = new_transport;
        // Clear fields that are irrelevant in the new transport.
        match new_transport {
            McpTransportType::Stdio => {
                srv.url = None;
                srv.headers.clear();
            }
            McpTransportType::Http | McpTransportType::Sse => {
                srv.command = None;
                srv.args.clear();
                srv.env.clear();
            }
        }
    }

    // Transport-specific updates
    match srv.transport {
        McpTransportType::Stdio => {
            if let Some(cmd) = &parsed.command {
                srv.command = Some(cmd.clone());
            } else if let Some(ep) = pos_endpoint {
                srv.command = Some(ep);
            }
            if parsed.clear_args {
                srv.args.clear();
            }
            if let Some(args) = &parsed.args {
                srv.args = args.clone();
            }
            if parsed.clear_env {
                srv.env.clear();
            }
            for (k, v) in &parsed.env {
                srv.env.insert(k.clone(), v.clone());
            }
        }
        McpTransportType::Http | McpTransportType::Sse => {
            if let Some(url) = &parsed.url {
                srv.url = Some(url.clone());
            } else if let Some(ep) = pos_endpoint {
                srv.url = Some(ep);
            }
            if parsed.clear_headers {
                srv.headers.clear();
            }
            for (k, v) in &parsed.headers {
                srv.headers.insert(k.clone(), v.clone());
            }
        }
    }

    Ok(())
}

fn validate_mcp_entry(entry: &McpServerEntry) -> Result<(), String> {
    if entry.name.trim().is_empty() {
        return Err("MCP server name cannot be empty".to_string());
    }
    if entry.timeout_secs == 0 {
        return Err("timeout_secs must be > 0".to_string());
    }

    match entry.transport {
        McpTransportType::Stdio => {
            let cmd = entry.command.as_deref().unwrap_or("").trim();
            if cmd.is_empty() {
                return Err("stdio transport requires a non-empty command".to_string());
            }
            if entry.url.is_some() {
                return Err("stdio transport cannot set url".to_string());
            }
            if !entry.headers.is_empty() {
                return Err("stdio transport cannot set headers".to_string());
            }
        }
        McpTransportType::Http | McpTransportType::Sse => {
            let url = entry.url.as_deref().unwrap_or("").trim();
            if url.is_empty() {
                return Err("http/sse transport requires a non-empty url".to_string());
            }
            if entry.command.is_some() {
                return Err("http/sse transport cannot set command".to_string());
            }
            if !entry.args.is_empty() {
                return Err("http/sse transport cannot set args".to_string());
            }
            if !entry.env.is_empty() {
                return Err("http/sse transport cannot set env".to_string());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gestura_core::agent_sessions::MessageSource;
    use uuid::Uuid;

    fn new_test_session() -> AgentSession {
        let base = std::env::temp_dir()
            .join("gestura-slash-tests")
            .join(Uuid::new_v4().to_string());
        std::fs::create_dir_all(&base).unwrap();

        AgentSession::new_with_workspace(base, None).unwrap()
    }

    #[test]
    fn agent_browser_entries_expose_drill_down_console_items() {
        let config = AppConfig::default();
        let session = new_test_session();

        let entries = agent_browser_entries(&config, &session);

        assert!(entries.iter().any(|entry| entry.title == "Agent Overview"));
        assert!(
            entries
                .iter()
                .any(|entry| entry.title == "Provider Readiness")
        );
        assert!(entries.iter().any(|entry| entry.command == "/model "));
    }

    #[test]
    fn agent_subcommand_status_includes_provider_section() {
        let config = AppConfig::default();
        let session = new_test_session();

        let lines = run_agent_subcommand(&["status"], &config, &session).unwrap();
        let joined = lines.join("\n");

        assert!(joined.contains("Agent Status"));
        assert!(joined.contains("Primary LLM"));
        assert!(joined.contains("Provider Status"));
    }

    #[test]
    fn device_subcommand_accepts_list_and_scan() {
        let list_lines = run_device_subcommand(&["list"]).unwrap();
        let scan_lines = run_device_subcommand(&["scan"]).unwrap();

        assert!(list_lines.join("\n").contains("Audio Devices"));
        assert!(scan_lines.join("\n").contains("Microphone available:"));
    }

    #[test]
    fn device_browser_entries_include_readiness_and_default_device_views() {
        let entries = device_browser_entries(&AppConfig::default());

        assert!(
            entries
                .iter()
                .any(|entry| entry.title == "Microphone Readiness")
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.title == "Default Input Device")
        );
    }

    #[test]
    fn knowledge_show_returns_rich_detail_lines() {
        let session = new_test_session();
        let item = load_session_knowledge_items(&session.id)
            .into_iter()
            .next()
            .expect("knowledge item");

        let lines = run_knowledge_subcommand(&["show", item.id.as_str()], &session).unwrap();
        let joined = lines.join("\n");

        assert!(joined.contains(&format!("[{}]", item.id)));
        assert!(joined.contains("Category:"));
        assert!(joined.contains("Origin:"));
        assert!(joined.contains("Content:"));
    }

    #[test]
    fn health_diagnostic_lines_include_audio_and_mcp_sections() {
        let lines = health_diagnostic_lines(&AppConfig::default());
        let joined = lines.join("\n");

        assert!(joined.contains("System Health"));
        assert!(joined.contains("Audio:"));
        assert!(joined.contains("MCP:"));
    }

    #[test]
    fn privacy_helpers_render_policy_and_report_sections() {
        let policy = privacy_policy_lines().join("\n");
        let report = privacy_report_lines("{\n  \"ok\": true\n}".to_string()).join("\n");

        assert!(policy.contains("Data Retention Policy"));
        assert!(policy.contains("gestura privacy export"));
        assert!(report.contains("Privacy Report"));
        assert!(report.contains("\"ok\": true"));
    }

    #[test]
    fn a2a_helper_lines_cover_status_profiles_and_agents() {
        let status = a2a_status_lines().join("\n");
        let profiles = a2a_profiles_lines().join("\n");
        let agents = a2a_agents_lines().join("\n");

        assert!(status.contains("A2A Protocol Status"));
        assert!(status.contains("profile/register"));
        assert!(profiles.contains("A2A Profiles"));
        assert!(profiles.contains("/a2a register"));
        assert!(agents.contains("A2A Agents"));
        assert!(agents.contains("/a2a discover <url>"));
    }

    #[test]
    fn context_helpers_render_status_analysis_and_categories() {
        let stats = ContextManager::new().cache_stats();
        let status = context_status_lines(&stats).join("\n");

        let analyzer = gestura_core::context::RequestAnalyzer::new();
        let analysis = analyzer.analyze("read src/main.rs and inspect https://example.com");
        let analyzed = context_analysis_lines(
            "read src/main.rs and inspect https://example.com",
            &analysis,
        )
        .join("\n");

        let categories = context_categories_lines().join("\n");

        assert!(status.contains("Context Manager Status"));
        assert!(status.contains("Context Cache:"));
        assert!(analyzed.contains("Request Analysis"));
        assert!(analyzed.contains("Suggested Tools"));
        assert!(analyzed.contains("Analysis Flags"));
        assert!(categories.contains("Context Categories"));
        assert!(categories.contains("FileSystem"));
        assert_eq!(context_clear_message(), "Context caches cleared");
    }

    #[test]
    fn config_helpers_render_list_keys_and_redacted_secret_values() {
        let mut config = AppConfig::default();
        config.llm.primary = "anthropic".to_string();
        config.llm.openai.get_or_insert(Default::default()).api_key = "sk-secret-123".to_string();

        let list = config_list_lines(&config).join("\n");
        let keys = config_keys_lines().join("\n");
        let get_secret = config_get_line(&config, "llm.openai.api_key").expect("secret key line");
        let update_secret = config_updated_message("llm.openai.api_key", "sk-secret-123");

        assert!(list.contains("Configuration"));
        assert!(list.contains("llm.primary"));
        assert!(list.contains("Config file:"));
        assert!(keys.contains("Available Config Keys"));
        assert!(keys.contains("llm.primary"));
        assert!(get_secret.contains("llm.openai.api_key = "));
        assert!(!get_secret.contains("sk-secret-123"));
        assert!(!update_secret.contains("sk-secret-123"));
        assert!(config_path_line().contains("Config file:"));
        assert_eq!(config_reset_message(), "Configuration reset to defaults");
    }

    #[test]
    fn session_helpers_render_filters_list_and_info() {
        let mut current = new_test_session();
        current.title = "Current test session".to_string();
        current.model = Some("anthropic/claude-test".to_string());
        current.add_user_message("hello", MessageSource::Text);
        current.add_assistant_message("hi", None);

        let other = SessionInfo {
            id: "12345678-abcdef00-session".to_string(),
            title: "Other session".to_string(),
            created_at: chrono::Utc::now() - chrono::Duration::hours(3),
            last_active: chrono::Utc::now() - chrono::Duration::hours(2),
            message_count: 4,
            model: Some("openai/gpt-test".to_string()),
        };

        let (filter, filter_label) = parse_session_list_filter(Some("week"));
        assert!(matches!(filter, SessionFilter::ThisWeek));
        assert_eq!(filter_label, " (this week)");

        let list = session_list_lines(
            &[
                SessionInfo {
                    id: current.id.clone(),
                    title: current.title.clone(),
                    created_at: current.created_at,
                    last_active: current.last_active,
                    message_count: current.message_count(),
                    model: current.model.clone(),
                },
                other,
            ],
            &current.id,
            &filter_label,
            10,
            true,
        )
        .join("\n");
        let info = session_info_lines(&current).join("\n");

        assert!(list.contains("Saved Sessions (this week)"));
        assert!(list.contains("Filters: /session list today|week|month"));
        assert!(list.contains("Commands: /session load <id>"));
        assert!(list.contains(&current.id[..8]));
        assert_eq!(
            session_empty_message(" (today)"),
            "No saved sessions found (today)"
        );
        assert!(info.contains("Current Session"));
        assert!(info.contains("Current test session"));
        assert!(info.contains("Messages: 2 (you: 1, assistant: 1, system: 0)"));
    }

    #[test]
    fn knowledge_helpers_render_list_search_categories_and_status() {
        let items = vec![
            gestura_core::KnowledgeItem::new(
                "rust-expert",
                "Rust Expert",
                "Expert Rust programming knowledge",
            )
            .with_category("language"),
            gestura_core::KnowledgeItem::new(
                "tauri-expert",
                "Tauri Expert",
                "Desktop app guidance",
            )
            .with_category("framework"),
        ];
        let matches = vec![gestura_core::KnowledgeMatch {
            item: items[0].clone(),
            score: 0.82,
            matched_triggers: vec!["rust".to_string()],
            suggested_references: vec![],
        }];
        let category_counts = vec![
            ("framework".to_string(), 1usize),
            ("language".to_string(), 1usize),
        ];

        let list = knowledge_list_lines(&items).join("\n");
        let search = knowledge_search_lines("rust ownership", &matches).join("\n");
        let categories = knowledge_categories_lines(&category_counts).join("\n");
        let status = knowledge_status_lines(2, 2, Path::new("/tmp/knowledge")).join("\n");

        assert!(list.contains("Knowledge Base (2 items)"));
        assert!(list.contains("[language] Rust Expert"));
        assert!(search.contains("Knowledge Search: 'rust ownership'"));
        assert!(search.contains("Rust Expert (score: 0.82)"));
        assert!(categories.contains("Knowledge Categories"));
        assert!(categories.contains("framework (1 items)"));
        assert!(status.contains("Knowledge Base Status"));
        assert!(status.contains("Base directory: /tmp/knowledge"));
        assert_eq!(knowledge_empty_message(), "No knowledge items registered.");
        assert_eq!(knowledge_search_usage(), "Usage: /knowledge search <query>");
        assert_eq!(
            knowledge_no_results_message("rust"),
            "No knowledge items match 'rust'."
        );
        assert_eq!(
            knowledge_no_categories_message(),
            "No knowledge categories found."
        );
    }

    #[test]
    fn memory_help_does_not_require_workspace() {
        let mut session = new_test_session();
        session.state.workspace_dir = None;

        let out = run_memory_subcommand(&["help"], &session).unwrap();
        assert!(out.live_action.is_none());
        assert!(out.lines.iter().any(|l| l.contains("/memory")));
    }

    #[test]
    fn memory_list_requires_workspace() {
        let mut session = new_test_session();
        session.state.workspace_dir = None;

        let err = run_memory_subcommand(&["list"], &session).unwrap_err();
        assert!(err.contains("No workspace directory"));
    }

    #[test]
    fn memory_search_parses_query_and_limit() {
        let session = new_test_session();

        let out =
            run_memory_subcommand(&["search", "hello", "world", "--limit", "5"], &session).unwrap();
        assert_eq!(
            out.live_action,
            Some(MemoryLiveAction::Search {
                query: "hello world".to_string(),
                limit: 5
            })
        );

        let err = run_memory_subcommand(&["search", "--limit", "5"], &session).unwrap_err();
        assert!(err.contains("Usage: /memory search"));
    }

    #[test]
    fn memory_save_validates_history_and_last_n() {
        let mut session = new_test_session();

        let err = run_memory_subcommand(&["save"], &session).unwrap_err();
        assert!(err.contains("No conversation history"));

        session.add_user_message("u1", MessageSource::Text);
        session.add_assistant_message("a1", None);

        let out = run_memory_subcommand(
            &[
                "save",
                "--summary",
                "sum",
                "--category",
                "cat",
                "--last",
                "1",
            ],
            &session,
        )
        .unwrap();
        assert!(out.changed);

        let act = out.live_action.unwrap();
        match act {
            MemoryLiveAction::Save { entry } => {
                assert_eq!(entry.session_id, session.id);
                assert_eq!(entry.summary, "sum");
                assert_eq!(entry.category, Some("cat".to_string()));
                assert!(entry.content.contains("a1"));
                assert!(!entry.content.contains("u1"));
            }
            other => panic!("expected Save live action, got {other:?}"),
        }

        let err = run_memory_subcommand(&["save", "--last", "0"], &session).unwrap_err();
        assert!(err.contains("No conversation history"));
    }

    #[test]
    fn memory_clear_and_delete_require_confirmed_and_resolve_paths() {
        let session = new_test_session();

        let err = run_memory_subcommand(&["clear"], &session).unwrap_err();
        assert!(err.contains("--confirmed"));

        let out = run_memory_subcommand(&["clear", "--confirmed"], &session).unwrap();
        assert_eq!(out.live_action, Some(MemoryLiveAction::ClearAll));

        let err = run_memory_subcommand(&["delete", "foo.json"], &session).unwrap_err();
        assert!(err.contains("Refusing to delete"));

        let err = run_memory_subcommand(&["delete", "--confirmed"], &session).unwrap_err();
        assert!(err.contains("Usage: /memory delete"));

        let ws = session.workspace_dir().unwrap().clone();
        let out =
            run_memory_subcommand(&["delete", "--confirmed", "rel/path.json"], &session).unwrap();
        match out.live_action {
            Some(MemoryLiveAction::Delete { file_path }) => {
                assert_eq!(file_path, ws.join("rel/path.json"));
            }
            other => panic!("expected Delete live action, got {other:?}"),
        }

        let abs = std::env::temp_dir().join("gestura-abs-delete.json");
        let abs_str = abs.to_string_lossy().to_string();
        let out =
            run_memory_subcommand(&["delete", "--confirmed", abs_str.as_str()], &session).unwrap();
        match out.live_action {
            Some(MemoryLiveAction::Delete { file_path }) => {
                assert_eq!(file_path, abs);
            }
            other => panic!("expected Delete live action, got {other:?}"),
        }
    }

    #[test]
    fn tasks_delete_requires_confirmed() {
        use gestura_core::tasks::TaskManager;

        let base = std::env::temp_dir()
            .join("gestura-slash-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&base).unwrap();

        let manager = TaskManager::new(&base);
        let session_id = "session-slash-test";
        let task = manager
            .create_task(session_id, "TestTask", "desc", None)
            .unwrap();

        let err = run_tasks_subcommand(
            &["delete", task.id.as_str()],
            &manager,
            session_id,
            Some(&base),
        )
        .unwrap_err();
        assert!(err.contains("--confirmed"));

        let out = run_tasks_subcommand(
            &["delete", "--confirmed", task.id.as_str()],
            &manager,
            session_id,
            Some(&base),
        )
        .unwrap();
        assert!(out.changed);
        assert!(out.lines.join("\n").contains("Deleted task"));
    }

    #[test]
    fn task_approval_commands_require_workspace_and_parse_live_actions() {
        use gestura_core::tasks::TaskManager;

        let base = std::env::temp_dir()
            .join("gestura-slash-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&base).unwrap();

        let manager = TaskManager::new(&base);
        let err = run_tasks_subcommand(&["approvals"], &manager, "session-1", None).unwrap_err();
        assert!(err.contains("workspace directory"));

        let out = run_tasks_subcommand(
            &[
                "approve", "task-123", "--actor", "reviewer", "Looks", "good",
            ],
            &manager,
            "session-1",
            Some(&base),
        )
        .unwrap();
        assert!(out.changed);
        assert_eq!(
            out.live_action,
            Some(TasksLiveAction::DecideApproval {
                session_id: "session-1".to_string(),
                workspace_dir: base.clone(),
                task_spec: "task-123".to_string(),
                actor_kind: ApprovalActorKind::Reviewer,
                decision: TaskApprovalCliDecision::Approve,
                note: Some("Looks good".to_string()),
            })
        );

        let out = run_tasks_subcommand(&["approvals"], &manager, "session-1", Some(&base)).unwrap();
        assert_eq!(
            out.live_action,
            Some(TasksLiveAction::ListApprovals {
                session_id: "session-1".to_string(),
                workspace_dir: base,
            })
        );
    }

    #[test]
    fn pending_workflow_approval_lines_include_scope_and_allowed_actors() {
        use chrono::Utc;
        use gestura_core::agents::{AgentExecutionMode, AgentRole, DelegatedTask};
        use gestura_core::orchestrator::{
            ApprovalActor, CleanupPolicy, EnvironmentHealth, EnvironmentState,
            ExecutionEnvironment, RecoveryStatus, SupervisorRun, SupervisorRunStatus,
            SupervisorTaskRecord, SupervisorTaskState, TaskApprovalRecord,
        };

        let workspace_dir = std::env::temp_dir().join("gestura-cli-approval-lines");
        let task = DelegatedTask {
            id: "task-review-1".to_string(),
            agent_id: "agent-review-1".to_string(),
            prompt: "Review this patch".to_string(),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: Some("session-approval".to_string()),
            directive_id: None,
            tracking_task_id: None,
            run_id: Some("run-approval".to_string()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Reviewer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: true,
            test_required: false,
            workspace_dir: Some(workspace_dir.clone()),
            execution_mode: AgentExecutionMode::SharedWorkspace,
            environment_id: Some("env-approval".to_string()),
            remote_target: None,
            memory_tags: vec![],
            name: Some("Review patch".to_string()),
        };
        let approval = TaskApprovalRecord::pending(
            &task,
            ApprovalScope::Review,
            ApprovalActor::system("orchestrator"),
            Some("Execution finished. Awaiting explicit review approval.".to_string()),
        );
        let run = SupervisorRun {
            id: "run-approval".to_string(),
            name: Some("approval-run".to_string()),
            session_id: Some("session-approval".to_string()),
            workspace_dir: Some(workspace_dir.clone()),
            lead_agent_id: Some("lead-1".to_string()),
            parent_run: None,
            child_runs: vec![],
            hierarchy_depth: 0,
            max_hierarchy_depth: 1,
            inherited_policy: None,
            metadata: None,
            status: SupervisorRunStatus::Waiting,
            task_summary: Default::default(),
            hierarchy_summary: None,
            tasks: vec![SupervisorTaskRecord {
                task,
                state: SupervisorTaskState::ReviewPending,
                approval,
                environment_id: "env-approval".to_string(),
                environment: ExecutionEnvironment {
                    id: "env-approval".to_string(),
                    execution_mode: AgentExecutionMode::SharedWorkspace,
                    root_dir: workspace_dir,
                    write_access: true,
                    branch_name: None,
                    worktree_path: None,
                    remote_url: None,
                    state: EnvironmentState::Ready,
                    health: EnvironmentHealth::Clean,
                    cleanup_policy: CleanupPolicy::KeepAlways,
                    recovery_status: RecoveryStatus::NotRequired,
                    recovery_action: None,
                    failure: None,
                    cleanup_result: None,
                },
                claimed_by: Some("agent-review-1".to_string()),
                attempts: 0,
                blocked_reasons: vec![],
                result: None,
                remote_execution: None,
                local_execution: None,
                messages: vec![],
                checkpoint: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                started_at: None,
                completed_at: None,
            }],
            messages: vec![],
            shared_cognition: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            completed_at: None,
        };

        let lines = format_pending_approval_lines(&[run]);
        let joined = lines.join("\n");
        assert!(joined.contains("task-review-1 [review]"));
        assert!(joined.contains("Requested by: orchestrator"));
        assert!(joined.contains("Allowed actors: reviewer, supervisor"));
    }

    #[test]
    fn workflow_tree_lines_include_checkpoint_resumability_details() {
        use chrono::Utc;
        use gestura_core::agents::{
            AgentExecutionMode, AgentRole, DelegatedTask, OrchestratorToolCall, TaskResult,
            TaskTerminalStateHint,
        };
        use gestura_core::orchestrator::{
            CleanupPolicy, DelegatedCheckpointAction, DelegatedCheckpointStage,
            DelegatedCheckpointSummary, DelegatedReplaySafety, DelegatedResumeDisposition,
            EnvironmentHealth, EnvironmentState, ExecutionEnvironment, LocalExecutionPhase,
            LocalExecutionProgress, LocalExecutionRecord, LocalExecutionWaitingReason,
            RecoveryStatus, SupervisorRun, SupervisorRunStatus, SupervisorRunTaskSummary,
            SupervisorTaskRecord, SupervisorTaskState, TaskApprovalRecord,
        };

        let workspace_dir = std::env::temp_dir().join("gestura-cli-checkpoint-lines");
        let run = SupervisorRun {
            id: "run-checkpoint-tree".to_string(),
            name: Some("checkpoint-run".to_string()),
            session_id: Some("session-tree".to_string()),
            workspace_dir: Some(workspace_dir.clone()),
            lead_agent_id: Some("lead-1".to_string()),
            parent_run: None,
            child_runs: vec![],
            hierarchy_depth: 0,
            max_hierarchy_depth: 1,
            inherited_policy: None,
            metadata: None,
            status: SupervisorRunStatus::Waiting,
            task_summary: SupervisorRunTaskSummary {
                total: 1,
                blocked: 1,
                ..SupervisorRunTaskSummary::default()
            },
            hierarchy_summary: None,
            tasks: vec![SupervisorTaskRecord {
                task: DelegatedTask {
                    id: "task-checkpoint-tree".to_string(),
                    agent_id: "agent-tree".to_string(),
                    prompt: "Resume work".to_string(),
                    context: None,
                    required_tools: vec![],
                    priority: 1,
                    session_id: Some("session-tree".to_string()),
                    directive_id: None,
                    tracking_task_id: None,
                    run_id: Some("run-checkpoint-tree".to_string()),
                    parent_task_id: None,
                    depends_on: vec![],
                    role: Some(AgentRole::Implementer),
                    delegation_brief: None,
                    planning_only: false,
                    approval_required: false,
                    reviewer_required: false,
                    test_required: false,
                    workspace_dir: Some(workspace_dir.clone()),
                    execution_mode: AgentExecutionMode::SharedWorkspace,
                    environment_id: Some("env-tree".to_string()),
                    remote_target: None,
                    memory_tags: vec![],
                    name: Some("Resume work".to_string()),
                },
                state: SupervisorTaskState::Blocked,
                approval: TaskApprovalRecord::default(),
                environment_id: "env-tree".to_string(),
                environment: ExecutionEnvironment {
                    id: "env-tree".to_string(),
                    execution_mode: AgentExecutionMode::SharedWorkspace,
                    root_dir: workspace_dir,
                    write_access: true,
                    branch_name: None,
                    worktree_path: None,
                    remote_url: None,
                    state: EnvironmentState::Ready,
                    health: EnvironmentHealth::Clean,
                    cleanup_policy: CleanupPolicy::KeepAlways,
                    recovery_status: RecoveryStatus::NotRequired,
                    recovery_action: None,
                    failure: None,
                    cleanup_result: None,
                },
                claimed_by: Some("agent-tree".to_string()),
                attempts: 1,
                blocked_reasons: vec!["execution interrupted during restart".to_string()],
                result: Some(TaskResult {
                    task_id: "task-checkpoint-tree".to_string(),
                    agent_id: "agent-tree".to_string(),
                    success: true,
                    run_id: Some("run-checkpoint-tree".to_string()),
                    tracking_task_id: None,
                    output: "done".to_string(),
                    summary: Some("Resume work".to_string()),
                    tool_calls: vec![OrchestratorToolCall {
                        tool_name: "file".to_string(),
                        input: serde_json::json!({ "path": "README.md" }),
                        output: serde_json::json!({ "ok": true }),
                        success: true,
                        duration_ms: 12,
                    }],
                    artifacts: vec![],
                    terminal_state_hint: Some(TaskTerminalStateHint::Blocked),
                    duration_ms: 42,
                }),
                remote_execution: None,
                local_execution: Some(LocalExecutionRecord {
                    status: "running".to_string(),
                    status_reason: None,
                    progress: Some(LocalExecutionProgress {
                        phase: LocalExecutionPhase::Waiting,
                        waiting_reason: Some(LocalExecutionWaitingReason::ShellProcess),
                        stage: Some("shell_running".to_string()),
                        message: Some("Streaming shell output".to_string()),
                        percent: Some(45),
                        iteration: 2,
                        current_tool_name: Some("shell".to_string()),
                        last_completed_tool_name: Some("file".to_string()),
                        last_completed_tool_duration_ms: Some(12),
                        completed_tool_call_count: 1,
                        has_partial_content: true,
                        partial_content_chars: 48,
                        has_partial_thinking: false,
                        partial_thinking_chars: 0,
                        token_usage: None,
                        environment: None,
                        updated_at: Utc::now(),
                    }),
                    last_synced_at: Utc::now(),
                }),
                messages: vec![],
                checkpoint: Some(DelegatedCheckpointSummary {
                    stage: DelegatedCheckpointStage::Blocked,
                    replay_safety: DelegatedReplaySafety::CheckpointResumable,
                    resume_disposition: DelegatedResumeDisposition::ResumeFromCheckpoint,
                    safe_boundary_label: "after tool 'file' result".to_string(),
                    available_actions: vec![
                        DelegatedCheckpointAction::ResumeFromCheckpoint,
                        DelegatedCheckpointAction::RestartFromScratch,
                        DelegatedCheckpointAction::AcknowledgeBlocked,
                    ],
                    note: Some("resume available after restart".to_string()),
                    completed_tool_call_count: 1,
                    has_resume_state: true,
                    result_published: false,
                    updated_at: Utc::now(),
                }),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                started_at: None,
                completed_at: None,
            }],
            messages: vec![],
            shared_cognition: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            completed_at: None,
        };

        let joined = format_supervisor_run_tree_lines(&[run]).join("\n");
        assert!(joined.contains("after tool 'file' result"));
        assert!(joined.contains("resume_from_checkpoint"));
        assert!(joined.contains("local=waiting"));
        assert!(joined.contains("current_tool=shell"));
        assert!(joined.contains("tool_trace=file:ok(12ms)"));
        assert!(
            joined.contains(
                "actions=resume_from_checkpoint,restart_from_scratch,acknowledge_blocked"
            )
        );
    }

    #[test]
    fn workflow_tree_lines_include_remote_progress_details() {
        use chrono::Utc;
        use gestura_core::agents::{
            AgentExecutionMode, AgentRole, DelegatedTask, RemoteAgentTarget,
        };
        use gestura_core::orchestrator::{
            CleanupPolicy, EnvironmentHealth, EnvironmentState, ExecutionEnvironment,
            RecoveryStatus, RemoteExecutionArtifact, RemoteExecutionCompatibility,
            RemoteExecutionProgress, RemoteExecutionRecord, SupervisorRun, SupervisorRunStatus,
            SupervisorRunTaskSummary, SupervisorTaskRecord, SupervisorTaskState,
            TaskApprovalRecord,
        };

        let workspace_dir = std::env::temp_dir().join("gestura-cli-remote-lines");
        let remote_target = RemoteAgentTarget {
            url: "http://localhost:32145/a2a".to_string(),
            name: Some("remote-peer".to_string()),
            auth_token: None,
            capabilities: vec!["shell".to_string()],
        };
        let run = SupervisorRun {
            id: "run-remote-tree".to_string(),
            name: Some("remote-run".to_string()),
            session_id: Some("session-tree".to_string()),
            workspace_dir: Some(workspace_dir.clone()),
            lead_agent_id: Some("lead-1".to_string()),
            parent_run: None,
            child_runs: vec![],
            hierarchy_depth: 0,
            max_hierarchy_depth: 1,
            inherited_policy: None,
            metadata: None,
            status: SupervisorRunStatus::Running,
            task_summary: SupervisorRunTaskSummary {
                total: 1,
                running: 1,
                ..SupervisorRunTaskSummary::default()
            },
            hierarchy_summary: None,
            tasks: vec![SupervisorTaskRecord {
                task: DelegatedTask {
                    id: "task-remote-tree".to_string(),
                    agent_id: "agent-remote".to_string(),
                    prompt: "Inspect remote status".to_string(),
                    context: None,
                    required_tools: vec![],
                    priority: 1,
                    session_id: Some("session-tree".to_string()),
                    directive_id: None,
                    tracking_task_id: None,
                    run_id: Some("run-remote-tree".to_string()),
                    parent_task_id: None,
                    depends_on: vec![],
                    role: Some(AgentRole::Implementer),
                    delegation_brief: None,
                    planning_only: false,
                    approval_required: false,
                    reviewer_required: false,
                    test_required: false,
                    workspace_dir: None,
                    execution_mode: AgentExecutionMode::Remote,
                    environment_id: None,
                    remote_target: Some(remote_target.clone()),
                    memory_tags: vec![],
                    name: Some("Remote parity work".to_string()),
                },
                state: SupervisorTaskState::Running,
                approval: TaskApprovalRecord::default(),
                environment_id: "env-tree".to_string(),
                environment: ExecutionEnvironment {
                    id: "env-tree".to_string(),
                    execution_mode: AgentExecutionMode::Remote,
                    root_dir: workspace_dir,
                    write_access: true,
                    branch_name: None,
                    worktree_path: None,
                    remote_url: None,
                    state: EnvironmentState::Ready,
                    health: EnvironmentHealth::Clean,
                    cleanup_policy: CleanupPolicy::KeepAlways,
                    recovery_status: RecoveryStatus::NotRequired,
                    recovery_action: None,
                    failure: None,
                    cleanup_result: None,
                },
                claimed_by: Some("agent-remote".to_string()),
                attempts: 1,
                blocked_reasons: vec![],
                result: None,
                remote_execution: Some(RemoteExecutionRecord {
                    target: remote_target,
                    remote_task_id: "remote-task-1".to_string(),
                    status: "running".to_string(),
                    status_reason: Some("Awaiting remote shell completion".to_string()),
                    lease: None,
                    progress: Some(RemoteExecutionProgress {
                        stage: Some("shell_running".to_string()),
                        message: Some("Remote shell still streaming".to_string()),
                        percent: Some(60),
                        updated_at: Utc::now(),
                    }),
                    artifacts: vec![RemoteExecutionArtifact {
                        name: "result.txt".to_string(),
                        part_count: 1,
                        metadata: std::collections::HashMap::new(),
                    }],
                    provenance: None,
                    compatibility: RemoteExecutionCompatibility {
                        supported_features: vec!["artifacts".to_string()],
                        warnings: vec![],
                        protocol_version: Some("2025-11-25".to_string()),
                    },
                    last_synced_at: Utc::now(),
                }),
                local_execution: None,
                messages: vec![],
                checkpoint: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                started_at: Some(Utc::now()),
                completed_at: None,
            }],
            messages: vec![],
            shared_cognition: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            completed_at: None,
        };

        let joined = format_supervisor_run_tree_lines(&[run]).join("\n");
        assert!(joined.contains("remote=running"));
        assert!(joined.contains("reason=Awaiting remote shell completion"));
        assert!(joined.contains("progress=60%"));
        assert!(joined.contains("stage=shell_running"));
        assert!(joined.contains("artifacts=1"));
    }

    #[test]
    fn workflow_tree_lines_include_shared_cognition_summary() {
        use chrono::Utc;
        use gestura_core::orchestrator::{
            SharedCognitionKind, SharedCognitionNote, SupervisorRun, SupervisorRunStatus,
            SupervisorRunTaskSummary, TeamMessageKind,
        };

        let now = Utc::now();
        let run = SupervisorRun {
            id: "run-cognition-tree".to_string(),
            name: Some("cognition-run".to_string()),
            session_id: Some("session-tree".to_string()),
            workspace_dir: Some(std::env::temp_dir()),
            lead_agent_id: Some("lead-1".to_string()),
            parent_run: None,
            child_runs: vec![],
            hierarchy_depth: 0,
            max_hierarchy_depth: 1,
            inherited_policy: None,
            metadata: None,
            status: SupervisorRunStatus::Running,
            task_summary: SupervisorRunTaskSummary::default(),
            hierarchy_summary: None,
            tasks: vec![],
            messages: vec![],
            shared_cognition: vec![
                SharedCognitionNote {
                    id: "note-1".to_string(),
                    run_id: "run-cognition-tree".to_string(),
                    task_id: Some("task-1".to_string()),
                    directive_id: Some("directive-1".to_string()),
                    kind: SharedCognitionKind::Hypothesis,
                    message_kind: TeamMessageKind::Clarification,
                    summary: "Need to verify whether the shim already normalizes ownership".to_string(),
                    detail: "Conflicting evidence in the ownership shim path; verify before widening scope.".to_string(),
                    sender_agent_id: Some("agent-alpha".to_string()),
                    recipient_agent_id: Some("supervisor".to_string()),
                    tags: vec!["shared-cognition".to_string(), "ownership".to_string()],
                    confidence: 0.64,
                    source_message_id: "msg-1".to_string(),
                    created_at: now,
                },
                SharedCognitionNote {
                    id: "note-2".to_string(),
                    run_id: "run-cognition-tree".to_string(),
                    task_id: Some("task-1".to_string()),
                    directive_id: Some("directive-1".to_string()),
                    kind: SharedCognitionKind::Steering,
                    message_kind: TeamMessageKind::StatusUpdate,
                    summary: "Keep execution limited to the ownership shim".to_string(),
                    detail: "Supervisor narrowed the active task to the ownership shim only.".to_string(),
                    sender_agent_id: Some("supervisor".to_string()),
                    recipient_agent_id: Some("agent-alpha".to_string()),
                    tags: vec!["shared-cognition".to_string(), "steering".to_string()],
                    confidence: 0.82,
                    source_message_id: "msg-2".to_string(),
                    created_at: now + chrono::Duration::seconds(5),
                },
            ],
            created_at: now,
            updated_at: now,
            completed_at: None,
        };

        let joined = format_supervisor_run_tree_lines(&[run]).join("\n");
        assert!(joined.contains("shared cognition: 2 notes"));
        assert!(joined.contains("latest=steering by supervisor"));
        assert!(joined.contains("confidence=82%"));
        assert!(joined.contains("open hypotheses=1"));
    }

    #[test]
    fn usage_lines_describe_root_commands_as_managed_shells() {
        let hooks = hooks_usage_lines().join("\n");
        let permissions = permissions_usage_lines().join("\n");
        let tasks = tasks_usage_lines().join("\n");
        let memory = memory_usage_lines().join("\n");
        let mcp = mcp_usage_lines().join("\n");

        assert!(hooks.contains("managed shell"));
        assert!(permissions.contains("managed shell"));
        assert!(tasks.contains("managed shell"));
        assert!(memory.contains("Managed memory shell"));
        assert!(mcp.contains("managed shell"));
    }

    #[test]
    fn collaboration_commands_require_workspace_and_parse_live_actions() {
        use gestura_core::tasks::TaskManager;

        let base = std::env::temp_dir()
            .join("gestura-slash-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&base).unwrap();

        let manager = TaskManager::new(&base);
        let err = run_tasks_subcommand(&["threads"], &manager, "session-1", None).unwrap_err();
        assert!(err.contains("workspace directory"));

        let threads = run_tasks_subcommand(
            &["threads", "--archived"],
            &manager,
            "session-1",
            Some(&base),
        )
        .unwrap();
        assert_eq!(
            threads.live_action,
            Some(TasksLiveAction::ListThreads {
                session_id: "session-1".to_string(),
                workspace_dir: base.clone(),
                include_archived: true,
            })
        );

        let tree = run_tasks_subcommand(&["tree"], &manager, "session-1", Some(&base)).unwrap();
        assert_eq!(
            tree.live_action,
            Some(TasksLiveAction::ListHierarchy {
                session_id: "session-1".to_string(),
                workspace_dir: base.clone(),
            })
        );

        let pause =
            run_tasks_subcommand(&["pause", "task-123"], &manager, "session-1", Some(&base))
                .unwrap();
        assert_eq!(
            pause.live_action,
            Some(TasksLiveAction::PauseWorkflowTask {
                session_id: "session-1".to_string(),
                workspace_dir: base.clone(),
                task_spec: "task-123".to_string(),
            })
        );

        let cancel =
            run_tasks_subcommand(&["cancel", "task-123"], &manager, "session-1", Some(&base))
                .unwrap();
        assert_eq!(
            cancel.live_action,
            Some(TasksLiveAction::CancelWorkflowTask {
                session_id: "session-1".to_string(),
                workspace_dir: base.clone(),
                task_spec: "task-123".to_string(),
            })
        );

        let resume =
            run_tasks_subcommand(&["resume", "task-123"], &manager, "session-1", Some(&base))
                .unwrap();
        assert_eq!(
            resume.live_action,
            Some(TasksLiveAction::ResumeWorkflowTask {
                session_id: "session-1".to_string(),
                workspace_dir: base.clone(),
                task_spec: "task-123".to_string(),
            })
        );

        let restart =
            run_tasks_subcommand(&["restart", "task-123"], &manager, "session-1", Some(&base))
                .unwrap();
        assert_eq!(
            restart.live_action,
            Some(TasksLiveAction::RestartWorkflowTask {
                session_id: "session-1".to_string(),
                workspace_dir: base.clone(),
                task_spec: "task-123".to_string(),
            })
        );

        let ack = run_tasks_subcommand(
            &["ack-blocked", "task-123", "waiting", "for", "ops"],
            &manager,
            "session-1",
            Some(&base),
        )
        .unwrap();
        assert_eq!(
            ack.live_action,
            Some(TasksLiveAction::AcknowledgeBlockedTask {
                session_id: "session-1".to_string(),
                workspace_dir: base.clone(),
                task_spec: "task-123".to_string(),
                note: Some("waiting for ops".to_string()),
            })
        );

        let message = run_tasks_subcommand(
            &[
                "message",
                "task-123",
                "blocker",
                "Waiting",
                "on",
                "credentials",
            ],
            &manager,
            "session-1",
            Some(&base),
        )
        .unwrap();
        assert_eq!(
            message.live_action,
            Some(TasksLiveAction::CreateCollaboration {
                session_id: "session-1".to_string(),
                workspace_dir: base.clone(),
                target_spec: "task-123".to_string(),
                kind: TeamMessageKind::Blocker,
                note: "Waiting on credentials".to_string(),
            })
        );

        let child = run_tasks_subcommand(
            &[
                "child-run",
                "run-parent",
                "supervisor-alpha",
                "--objective",
                "Coordinate",
                "frontend",
                "delivery",
                "--name",
                "Frontend",
                "pod",
                "--mode",
                "git_worktree",
                "--approval",
                "--review",
                "--test",
                "--tags",
                "frontend,delivery",
                "--constraint",
                "Escalate",
                "API",
                "changes",
            ],
            &manager,
            "session-1",
            Some(&base),
        )
        .unwrap();
        assert_eq!(
            child.live_action,
            Some(TasksLiveAction::CreateChildSupervisorRun {
                session_id: "session-1".to_string(),
                workspace_dir: base.clone(),
                parent_run_spec: "run-parent".to_string(),
                lead_agent_id: "supervisor-alpha".to_string(),
                objective: "Coordinate frontend delivery".to_string(),
                name: Some("Frontend pod".to_string()),
                approval_required: true,
                reviewer_required: true,
                test_required: true,
                execution_mode: AgentExecutionMode::GitWorktree,
                memory_tags: vec!["frontend".to_string(), "delivery".to_string()],
                constraint_notes: vec!["Escalate API changes".to_string()],
            })
        );

        let thread = run_tasks_subcommand(
            &["thread", "resolve", "thread-123", "Fixed"],
            &manager,
            "session-1",
            Some(&base),
        )
        .unwrap();
        assert_eq!(
            thread.live_action,
            Some(TasksLiveAction::UpdateThread {
                session_id: "session-1".to_string(),
                workspace_dir: base,
                thread_id: "thread-123".to_string(),
                status: Some(CollaborationActionStatus::Resolved),
                archive: false,
                escalate: false,
                note: Some("Fixed".to_string()),
            })
        );
    }

    #[test]
    fn hook_event_parsing_accepts_variants() {
        assert_eq!(
            "pre_pipeline".parse::<HookEvent>().ok(),
            Some(HookEvent::PrePipeline)
        );
        assert_eq!(
            "pre-pipeline".parse::<HookEvent>().ok(),
            Some(HookEvent::PrePipeline)
        );
        assert_eq!(
            "PreTool".parse::<HookEvent>().ok(),
            Some(HookEvent::PreTool)
        );
        assert_eq!(
            "post tool".parse::<HookEvent>().ok(),
            Some(HookEvent::PostTool)
        );
        assert_eq!("nope".parse::<HookEvent>().ok(), None);
    }

    #[test]
    fn resolve_task_id_from_list_supports_prefix_and_current() {
        let mut t1 = Task::new("session-1", "A", "", None);
        t1.id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string();

        let mut t2 = t1.clone();
        t2.id = "bbbbbbbb-1111-2222-3333-444444444444".to_string();

        let mut t3 = t1.clone();
        t3.id = "bbbbbbbb-9999-8888-7777-666666666666".to_string();

        let tasks = vec![t1.clone(), t2.clone(), t3.clone()];
        let resolved = resolve_task_id_from_list("aaaa", &tasks, None).unwrap();
        assert_eq!(resolved, t1.id);

        let resolved = resolve_task_id_from_list(".", &tasks, Some(&t2.id)).unwrap();
        assert_eq!(resolved, t2.id);

        let err = resolve_task_id_from_list("b", &tasks, None).unwrap_err();
        assert!(err.contains("Ambiguous"));
    }

    #[test]
    fn permissions_parsing_accepts_tool_action_and_tool_dot_action() {
        let (tool, action) = parse_permission_tool_action(&["file.read"]).unwrap();
        assert_eq!(tool, "file");
        assert_eq!(action, "read");

        let (tool, action) = parse_permission_tool_action(&["shell", "run"]).unwrap();
        assert_eq!(tool, "shell");
        assert_eq!(action, "run");
    }

    #[test]
    fn permission_level_parsing_accepts_variants() {
        assert_eq!(
            "full-permissions".parse::<SessionPermissionLevel>().ok(),
            Some(SessionPermissionLevel::Full)
        );
        assert_eq!(
            "restricted".parse::<SessionPermissionLevel>().ok(),
            Some(SessionPermissionLevel::Restricted)
        );
        assert_eq!(
            "sandbox".parse::<SessionPermissionLevel>().ok(),
            Some(SessionPermissionLevel::Sandbox)
        );
        assert_eq!("nope".parse::<SessionPermissionLevel>().ok(), None);
    }

    #[test]
    fn mcp_add_validates_transport_specific_requirements() {
        let mut cfg = AppConfig::default();

        // Missing command for stdio.
        let err =
            run_mcp_subcommand(&["add", "srv1", "--transport", "stdio"], &mut cfg).unwrap_err();
        assert!(err.contains("requires"));

        // Valid stdio add.
        let out = run_mcp_subcommand(
            &[
                "add",
                "srv1",
                "--transport",
                "stdio",
                "--command",
                "npx",
                "--arg",
                "-y",
            ],
            &mut cfg,
        )
        .unwrap();
        assert!(out.changed);
        assert_eq!(cfg.mcp_servers.len(), 1);

        // Duplicate name.
        let err = run_mcp_subcommand(
            &["add", "srv1", "--transport", "http", "--url", "https://x"],
            &mut cfg,
        )
        .unwrap_err();
        assert!(err.contains("already exists"));
    }
}
