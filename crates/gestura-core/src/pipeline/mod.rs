//! Agent Pipeline - Unified LLM interaction pipeline
//!
//! This module provides a single entry point for all LLM interactions,
//! regardless of input source (text, voice, delegated tasks). It integrates:
//!
//! - Context analysis and reduction
//! - Tool filtering based on request
//! - Agentic loop for tool execution
//! - Optional ERL-inspired reflection, retry, and memory consolidation
//! - Streaming and non-streaming responses
//! - Token estimation and truncation
//! - Fallback to secondary providers
//! - Workspace sandboxing for tool execution

mod agent_loop;
mod compaction;
mod prompt;
mod reflection;
mod request_telemetry;
mod tool_dispatch;
mod tool_router;
pub mod types;
pub(crate) use reflection::sync_task_reflection_outcomes;

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;
use tokio::sync::mpsc;
pub use tool_router::{RoutingResult, ToolRouter, build_tool_router};
use tracing::Instrument as _;

use crate::agent_sessions::{AgentSessionStore, FileAgentSessionStore};
use crate::checkpoints::{CheckpointManager, CheckpointRetentionPolicy, FileCheckpointStore};
use crate::config::AppConfig;
use crate::context::{ContextManager, RequestAnalyzer};
use crate::error::AppError;
use crate::hooks::{HookContext, HookEngine, HookEvent};
use crate::knowledge::{KnowledgeSettingsManager, KnowledgeStore};
use crate::llm_provider::{AgentContext, select_provider};
use crate::session_workspace::SessionWorkspace;
use crate::streaming::{
    CancellationToken, StreamChunk, start_streaming, start_streaming_with_fallback,
};
use crate::tasks::TaskManager;
use crate::tool_confirmation::TOOL_CONFIRMATIONS;
use crate::tools::PermissionManager;
use crate::tools::registry::{ToolDefinition, all_tools};

use request_telemetry::{AgentRequestTelemetry, RequestOutcome, RequestRunMode};
use tool_dispatch::{FinalizePendingToolCallCtx, PendingToolCall};
pub use types::*;

/// Select the correct tool schema slice for a provider name.
///
/// Each provider family has its own tool definition format:
/// - Anthropic: `{name, description, input_schema}`
/// - Gemini: `{name, description, parameters}` (for `functionDeclarations`)
/// - All others (OpenAI, Grok, Ollama): `{type:"function", function:{…}}`
fn tools_slice_for_provider(
    provider_name: &str,
    schemas: &crate::tools::schemas::ProviderToolSchemas,
) -> Vec<serde_json::Value> {
    match provider_name {
        "anthropic" => schemas.anthropic.clone(),
        "gemini" => schemas.gemini.clone(),
        _ => schemas.openai.clone(),
    }
}

/// The main agent pipeline for processing requests
pub struct AgentPipeline {
    /// Application configuration
    config: AppConfig,
    /// Context manager for smart context reduction
    context_manager: ContextManager,
    /// Request analyzer for category detection
    analyzer: RequestAnalyzer,
    /// Pipeline-specific configuration
    pipeline_config: PipelineConfig,
    /// Persistent permission manager used for tool confirmation decisions.
    ///
    /// This enables "Allow always" semantics for tool confirmations.
    permission_manager: PermissionManager,
    /// Knowledge store for specialized expertise
    knowledge_store: Option<&'static KnowledgeStore>,
    /// Knowledge settings manager for session-scoped activation
    knowledge_settings: Option<&'static KnowledgeSettingsManager>,
    /// Optional pre-flight LLM tool router (None when strategy is Keyword).
    tool_router: Option<Box<dyn tool_router::ToolRouter>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoTrackedRequestShape {
    CreateOrModify,
    InvestigateOrFix,
    AnalyzeOrCompare,
    ResearchOrFetch,
    PlanOrDraft,
    GeneralExecution,
}

impl AgentPipeline {
    fn message_contains_any(text: &str, signals: &[&str]) -> bool {
        signals.iter().any(|signal| text.contains(signal))
    }

    fn infer_auto_tracked_request_shape(message: &str) -> AutoTrackedRequestShape {
        let text = message.trim().to_ascii_lowercase();

        if Self::message_contains_any(
            &text,
            &[
                "fix",
                "debug",
                "troubleshoot",
                "resolve",
                "repair",
                "investigate why",
                "diagnose",
            ],
        ) {
            return AutoTrackedRequestShape::InvestigateOrFix;
        }

        if Self::message_contains_any(
            &text,
            &[
                "compare", "review", "audit", "assess", "analyze", "analyse", "evaluate", "inspect",
            ],
        ) {
            return AutoTrackedRequestShape::AnalyzeOrCompare;
        }

        if Self::message_contains_any(
            &text,
            &[
                "research", "find", "look up", "lookup", "search", "fetch", "gather", "collect",
                "explore",
            ],
        ) {
            return AutoTrackedRequestShape::ResearchOrFetch;
        }

        if Self::message_contains_any(
            &text,
            &[
                "create",
                "implement",
                "build",
                "set up",
                "setup",
                "scaffold",
                "write",
                "draft",
                "compose",
                "update",
                "change",
                "modify",
                "refactor",
                "migrate",
            ],
        ) {
            return AutoTrackedRequestShape::CreateOrModify;
        }

        if Self::message_contains_any(
            &text,
            &[
                "plan",
                "design",
                "outline",
                "document",
                "summarize",
                "summarise",
                "propose",
            ],
        ) {
            return AutoTrackedRequestShape::PlanOrDraft;
        }

        AutoTrackedRequestShape::GeneralExecution
    }

    fn ensure_request_session_id(request: &mut AgentRequest) {
        let missing_session_id = request
            .metadata
            .session_id
            .as_deref()
            .is_none_or(|session_id| session_id.trim().is_empty());

        if !missing_session_id {
            return;
        }

        let generated = format!("agent-run-{}", uuid::Uuid::new_v4());
        tracing::warn!(
            generated_session_id = %generated,
            source = ?request.metadata.source,
            "Agent request missing session_id; synthesizing unique session id for isolated workspace/task tracking"
        );
        request.metadata.session_id = Some(generated);
    }

    fn should_auto_track_request(message: &str, explicit_task_id: Option<&str>) -> bool {
        if explicit_task_id.is_some() {
            return false;
        }

        let text = message.trim().to_ascii_lowercase();
        if text.is_empty() || text.starts_with('/') {
            return false;
        }

        Self::message_contains_any(
            &text,
            &[
                "plan and implement",
                "carefully plan",
                "build and test",
                "implement then",
                "step by step",
                "break this down",
                "end to end",
                "scaffold",
                "refactor",
                "fix",
                "debug",
                "build",
                "test",
                "compare",
                "review",
                "audit",
                "analyze",
                "analyse",
                "assess",
                "evaluate",
                "inspect",
                "research",
                "search",
                "find",
                "fetch",
                "gather",
                "collect",
                "explore",
                "plan",
                "design",
                "outline",
                "document",
                "write",
                "draft",
                "summarize",
                "summarise",
                "propose",
                "investigate",
                "troubleshoot",
            ],
        ) && text.split_whitespace().count() >= 6
    }

    fn should_offer_task_tool_for_request(analysis: &crate::context::RequestAnalysis) -> bool {
        analysis.needs_tools && Self::should_auto_track_request(&analysis.request, None)
    }

    fn append_task_tool_for_auto_tracked_request(
        analysis: &crate::context::RequestAnalysis,
        candidate_names: &HashSet<&str>,
        tools: &mut Vec<&'static ToolDefinition>,
    ) {
        if !candidate_names.contains("task")
            || !Self::should_offer_task_tool_for_request(analysis)
            || tools.iter().any(|tool| tool.name == "task")
        {
            return;
        }

        if let Some(task_tool) = crate::tools::registry::find_tool("task") {
            tools.push(task_tool);
        }
    }

    fn derive_agent_request_task_name(message: &str) -> String {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            return "Agent request".to_string();
        }

        let sentence = trimmed
            .split(['\n', '.', '!', '?'])
            .next()
            .unwrap_or(trimmed)
            .trim();
        let candidate =
            sentence.trim_matches(|ch: char| ch.is_ascii_punctuation() || ch.is_whitespace());
        if candidate.is_empty() {
            "Agent request".to_string()
        } else {
            candidate.chars().take(96).collect()
        }
    }

    fn default_auto_tracked_execution_subtasks(message: &str) -> Vec<(String, String)> {
        let request = message.trim();
        let mentions_validation = Self::message_contains_any(
            &request.to_ascii_lowercase(),
            &[
                "build",
                "test",
                "verify",
                "validation",
                "validate",
                "check",
                "run",
                "compile",
                "lint",
                "smoke",
            ],
        );

        match Self::infer_auto_tracked_request_shape(request) {
            AutoTrackedRequestShape::CreateOrModify => vec![
                (
                    "Inspect the current state and constraints".to_string(),
                    format!(
                        "Inspect the current environment, relevant context, and constraints before acting on:\n\n{}",
                        request
                    ),
                ),
                (
                    "Prepare the starting point or prerequisites".to_string(),
                    format!(
                        "Set up the starting point, prerequisites, or scaffolding needed to complete:\n\n{}",
                        request
                    ),
                ),
                (
                    "Carry out the requested work".to_string(),
                    format!(
                        "Create, modify, draft, or otherwise perform the primary work needed for:\n\n{}",
                        request
                    ),
                ),
                (
                    "Validate the result and summarize follow-up".to_string(),
                    if mentions_validation {
                        format!(
                            "Run the appropriate verification steps, checks, or commands and summarize the final outcome for:\n\n{}",
                            request
                        )
                    } else {
                        format!(
                            "Review the completed result, verify it in the most appropriate way, and summarize any follow-up for:\n\n{}",
                            request
                        )
                    },
                ),
            ],
            AutoTrackedRequestShape::InvestigateOrFix => vec![
                (
                    "Investigate the current issue or constraints".to_string(),
                    format!(
                        "Gather the evidence needed to understand the problem, failure, or constraints involved in:\n\n{}",
                        request
                    ),
                ),
                (
                    "Apply the fix or adjustment".to_string(),
                    format!(
                        "Make the change, adjustment, or corrective action needed to address:\n\n{}",
                        request
                    ),
                ),
                (
                    "Validate the fix and remaining risk".to_string(),
                    format!(
                        "Verify whether the issue is resolved and summarize any remaining risk or follow-up for:\n\n{}",
                        request
                    ),
                ),
            ],
            AutoTrackedRequestShape::AnalyzeOrCompare => vec![
                (
                    "Inspect the relevant inputs and criteria".to_string(),
                    format!(
                        "Gather the materials, context, and comparison criteria needed to evaluate:\n\n{}",
                        request
                    ),
                ),
                (
                    "Analyze the findings and identify gaps".to_string(),
                    format!(
                        "Analyze the relevant differences, patterns, tradeoffs, or gaps involved in:\n\n{}",
                        request
                    ),
                ),
                (
                    "Summarize conclusions and recommended actions".to_string(),
                    format!(
                        "Deliver a clear summary of the conclusions, recommendations, or next steps for:\n\n{}",
                        request
                    ),
                ),
            ],
            AutoTrackedRequestShape::ResearchOrFetch => vec![
                (
                    "Gather the relevant context and sources".to_string(),
                    format!(
                        "Collect the most relevant information, evidence, or source material needed for:\n\n{}",
                        request
                    ),
                ),
                (
                    "Extract the information that matters".to_string(),
                    format!(
                        "Filter and organize the most useful details, signals, or facts for:\n\n{}",
                        request
                    ),
                ),
                (
                    "Summarize findings and next steps".to_string(),
                    format!(
                        "Present the findings clearly and include any recommended next steps for:\n\n{}",
                        request
                    ),
                ),
            ],
            AutoTrackedRequestShape::PlanOrDraft => vec![
                (
                    "Clarify the goal, audience, and constraints".to_string(),
                    format!(
                        "Identify the intent, audience, constraints, and success criteria behind:\n\n{}",
                        request
                    ),
                ),
                (
                    "Draft the requested output".to_string(),
                    format!(
                        "Produce the requested plan, document, draft, or structured output for:\n\n{}",
                        request
                    ),
                ),
                (
                    "Review and refine the result".to_string(),
                    format!(
                        "Review the draft for completeness, quality, and alignment with the request:\n\n{}",
                        request
                    ),
                ),
            ],
            AutoTrackedRequestShape::GeneralExecution => vec![
                (
                    "Inspect the current state and gather context".to_string(),
                    format!(
                        "Inspect the current state, gather the needed context, and identify the concrete next steps for:\n\n{}",
                        request
                    ),
                ),
                (
                    "Carry out the requested work".to_string(),
                    format!(
                        "Perform the primary work needed to complete:\n\n{}",
                        request
                    ),
                ),
                (
                    "Verify the outcome and summarize next steps".to_string(),
                    if mentions_validation {
                        format!(
                            "Run the appropriate checks or commands, verify the outcome, and summarize what remains for:\n\n{}",
                            request
                        )
                    } else {
                        format!(
                            "Review the outcome, verify that it satisfies the request, and summarize next steps for:\n\n{}",
                            request
                        )
                    },
                ),
            ],
        }
    }

    fn build_auto_tracked_execution_handoff_message(
        original_message: &str,
        root_task_name: &str,
        planned_subtasks: &[String],
    ) -> String {
        let mut handoff = String::new();
        handoff.push_str(original_message.trim());
        handoff.push_str("\n\n[Runtime execution handoff]\n");
        handoff.push_str(&format!(
            "A task plan already exists for this request under the tracked task \"{}\". Execute that plan now instead of creating a fresh plan from scratch.\n",
            root_task_name
        ));
        if !planned_subtasks.is_empty() {
            handoff.push_str("Planned tracked subtasks:\n");
            for subtask in planned_subtasks {
                handoff.push_str("- ");
                handoff.push_str(subtask);
                handoff.push('\n');
            }
        }
        handoff.push_str(
            "Update task statuses as you start and finish each concrete subtask. If you create new work, create a concrete subtask with a specific name. Do not mark the tracked root task complete until every planned subtask is completed or explicitly cancelled for a real reason. Begin concrete work immediately. If the request is primarily analysis or research, gather evidence first and summarize the outcome clearly instead of forcing unnecessary edits or commands."
        );
        handoff
    }

    fn maybe_initialize_tracked_request_task(
        request: &mut AgentRequest,
        analysis_needs_tools: bool,
        task_tool_available: bool,
    ) {
        if request.metadata.task_id.is_some()
            || !analysis_needs_tools
            || !task_tool_available
            || !Self::should_auto_track_request(&request.input, None)
        {
            return;
        }

        Self::ensure_request_session_id(request);
        let Some(session_id) = request.metadata.session_id.as_deref() else {
            return;
        };

        let manager = crate::get_global_task_manager();
        let task_name = Self::derive_agent_request_task_name(&request.input);
        let execution_subtasks = Self::default_auto_tracked_execution_subtasks(&request.input);
        let description = format!(
            "Autogenerated tracked execution task for request:\n\n{}",
            request.input.trim()
        );

        match manager.create_task(session_id, &task_name, &description, None) {
            Ok(task) => {
                let mut planned_subtasks = Vec::new();
                request.metadata.task_id = Some(task.id.clone());

                if let Err(error) =
                    manager.update_task_status(session_id, &task.id, crate::TaskStatus::InProgress)
                {
                    tracing::warn!(
                        session_id = %session_id,
                        task_id = %task.id,
                        error = %error,
                        "Failed to mark tracked root task in progress during initialization"
                    );
                }

                let mut first_subtask_id: Option<String> = None;
                for (index, (name, subtask_description)) in execution_subtasks.iter().enumerate() {
                    match manager.create_task(
                        session_id,
                        name,
                        subtask_description,
                        Some(task.id.clone()),
                    ) {
                        Ok(subtask) => {
                            if index == 0 {
                                if let Err(error) = manager.update_task_status(
                                    session_id,
                                    &subtask.id,
                                    crate::TaskStatus::InProgress,
                                ) {
                                    tracing::warn!(
                                        session_id = %session_id,
                                        task_id = %subtask.id,
                                        error = %error,
                                        "Failed to mark initial execution subtask in progress"
                                    );
                                } else {
                                    first_subtask_id = Some(subtask.id.clone());
                                    planned_subtasks.push(format!("{} [inprogress]", subtask.name));
                                    continue;
                                }
                            }
                            planned_subtasks.push(format!("{} [notstarted]", subtask.name));
                        }
                        Err(error) => {
                            tracing::warn!(
                                session_id = %session_id,
                                root_task_id = %task.id,
                                subtask_name = %name,
                                error = %error,
                                "Failed to initialize default execution subtask for tracked request"
                            );
                        }
                    }
                }

                let current_task_id = first_subtask_id.unwrap_or_else(|| task.id.clone());
                let _ = manager.set_current_task_id(session_id, Some(current_task_id));
                if let Err(error) = manager.record_memory_event(
                    session_id,
                    &task.id,
                    crate::tasks::TaskMemoryEvent::new(
                        crate::tasks::TaskMemoryPhase::Handoff,
                        format!(
                            "Initialized tracked request with {} planned tracked subtasks",
                            planned_subtasks.len()
                        ),
                        Some("session".to_string()),
                        Some("handoff".to_string()),
                        None,
                    ),
                ) {
                    tracing::warn!(
                        session_id = %session_id,
                        task_id = %task.id,
                        error = %error,
                        "Failed to record tracked request initialization memory event"
                    );
                }

                request.input = Self::build_auto_tracked_execution_handoff_message(
                    &request.input,
                    &task.name,
                    &planned_subtasks,
                );
                tracing::info!(
                    session_id = %session_id,
                    task_id = %task.id,
                    task_name = %task.name,
                    planned_subtasks = planned_subtasks.len(),
                    "Initialized tracked root task and default tracked subtasks for agent request"
                );
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = %error,
                    "Failed to initialize tracked root task for agent request"
                );
            }
        }
    }

    /// Build a [`ContextManager`] pre-wired with the built-in tool registry.
    fn build_context_manager() -> ContextManager {
        ContextManager::new().with_tool_provider(Box::new(|| {
            crate::tools::registry::all_tools()
                .iter()
                .map(|t| (t.name.to_string(), t.description.to_string()))
                .collect()
        }))
    }

    /// Create a new pipeline using the default runtime configuration merged with
    /// persisted user pipeline settings from [`AppConfig`].
    pub fn new(config: AppConfig) -> Self {
        let pipeline_config = PipelineConfig::default().with_user_settings(&config.pipeline);
        let arc_config = std::sync::Arc::new(config.clone());
        let tool_router = build_tool_router(&pipeline_config.tool_routing_strategy, arc_config);
        Self {
            config,
            context_manager: Self::build_context_manager(),
            analyzer: RequestAnalyzer::new(),
            pipeline_config,
            permission_manager: PermissionManager::new(),
            knowledge_store: None,
            knowledge_settings: None,
            tool_router,
        }
    }

    /// Create a pipeline with custom configuration
    pub fn with_config(config: AppConfig, pipeline_config: PipelineConfig) -> Self {
        let arc_config = std::sync::Arc::new(config.clone());
        let tool_router = build_tool_router(&pipeline_config.tool_routing_strategy, arc_config);
        Self {
            config,
            context_manager: Self::build_context_manager(),
            analyzer: RequestAnalyzer::new(),
            pipeline_config,
            permission_manager: PermissionManager::new(),
            knowledge_store: None,
            knowledge_settings: None,
            tool_router,
        }
    }

    /// Set the knowledge store and settings manager for this pipeline
    pub fn with_knowledge(
        mut self,
        store: &'static KnowledgeStore,
        settings: &'static KnowledgeSettingsManager,
    ) -> Self {
        self.knowledge_store = Some(store);
        self.knowledge_settings = Some(settings);
        self
    }

    /// Create a HookEngine from the current configuration.
    ///
    /// Returns `None` if hooks are disabled or empty.
    fn create_hook_engine(&self) -> Option<HookEngine> {
        if !self.config.hooks.enabled || self.config.hooks.hooks.is_empty() {
            return None;
        }
        Some(HookEngine::new(self.config.hooks.clone()))
    }

    /// Run a hook event, logging any failures but not propagating them.
    ///
    /// This is used for best-effort hooks (PostPipeline, PostTool) where failures
    /// should not affect the main flow.
    async fn run_hook_best_effort(&self, engine: &HookEngine, event: HookEvent, ctx: &HookContext) {
        match engine.run(event, ctx).await {
            Ok(records) => {
                for record in &records {
                    tracing::debug!(
                        hook = %record.name,
                        event = ?record.event,
                        exit_code = record.output.exit_code,
                        "Hook executed (best-effort)"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    event = ?event,
                    error = %e,
                    "Hook execution failed (best-effort, continuing)"
                );
            }
        }
    }

    /// Ensure all enabled MCP servers from the application config are connected
    /// in the global client registry. Already-connected servers are skipped.
    async fn ensure_mcp_servers_connected(&self) {
        let registry = crate::mcp::get_mcp_client_registry();
        let connected = registry.connected_servers().await;

        for entry in &self.config.mcp_servers {
            if !entry.enabled || connected.contains(&entry.name) {
                continue;
            }
            match registry.connect(entry).await {
                Ok(tools) => {
                    tracing::info!(
                        server = %entry.name,
                        tool_count = tools.len(),
                        "MCP server connected"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        server = %entry.name,
                        error = %e,
                        "Failed to connect MCP server (skipping)"
                    );
                }
            }
        }
    }

    /// Streaming-friendly MCP preflight.
    ///
    /// This emits periodic `StreamChunk::Status` updates while connecting so the
    /// GUI never hits its "no events" idle timeout during slow/hung MCP servers.
    async fn ensure_mcp_servers_connected_streaming(
        &self,
        tx: &mpsc::Sender<StreamChunk>,
        cancel_token: &CancellationToken,
    ) {
        use tokio::time::{Duration, MissedTickBehavior};

        let registry = crate::mcp::get_mcp_client_registry();
        let connected = registry.connected_servers().await;

        for entry in &self.config.mcp_servers {
            if cancel_token.is_cancelled() {
                return;
            }
            if !entry.enabled || connected.contains(&entry.name) {
                continue;
            }

            let base_msg = format!("Connecting to MCP server '{}'…", entry.name);
            let _ = tx
                .send(StreamChunk::Status {
                    message: base_msg.clone(),
                })
                .await;

            // Keepalive status while we await connect(). This prevents the GUI's
            // 90s "no events" timeout even when a server has a long timeout.
            let mut tick = tokio::time::interval(Duration::from_secs(20));
            tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

            let connect_fut = registry.connect(entry);
            tokio::pin!(connect_fut);

            let result = loop {
                if cancel_token.is_cancelled() {
                    return;
                }

                tokio::select! {
                    res = &mut connect_fut => break res,
                    _ = tick.tick() => {
                        let _ = tx.send(StreamChunk::Status { message: base_msg.clone() }).await;
                    }
                }
            };

            match result {
                Ok(tools) => {
                    tracing::info!(
                        server = %entry.name,
                        tool_count = tools.len(),
                        "MCP server connected"
                    );
                    let _ = tx
                        .send(StreamChunk::Status {
                            message: format!(
                                "MCP server '{}' connected ({})",
                                entry.name,
                                tools.len()
                            ),
                        })
                        .await;
                }
                Err(e) => {
                    tracing::warn!(
                        server = %entry.name,
                        error = %e,
                        "Failed to connect MCP server (skipping)"
                    );
                    let _ = tx
                        .send(StreamChunk::Status {
                            message: format!("MCP server '{}' unavailable (skipping)", entry.name),
                        })
                        .await;
                }
            }
        }
    }

    /// Create a checkpoint before a write tool execution.
    ///
    /// This is a best-effort operation: failures are logged but do not block tool execution.
    fn try_create_checkpoint_before_tool(&self, session_id: &str, tool_name: &str) {
        let label = format!("before:{}", tool_name);

        // Use default stores - these are lightweight to construct
        let session_store = FileAgentSessionStore::new_default();
        let checkpoint_store = FileCheckpointStore::new_default();
        let manager =
            CheckpointManager::new(checkpoint_store, CheckpointRetentionPolicy::default());

        // TaskManager needs a base directory
        let task_manager = TaskManager::new(AppConfig::data_dir());

        match manager.create_session_checkpoint(
            session_id,
            &session_store,
            &task_manager,
            &self.config,
            Some(label),
        ) {
            Ok(meta) => {
                tracing::info!(
                    checkpoint_id = %meta.id,
                    session_id = session_id,
                    tool = tool_name,
                    "Created auto-checkpoint before write tool"
                );
            }
            Err(e) => {
                tracing::warn!(
                    session_id = session_id,
                    tool = tool_name,
                    error = %e,
                    "Failed to create auto-checkpoint (continuing with tool execution)"
                );
            }
        }
    }

    /// Create a pipeline with configuration optimized for the current LLM provider
    ///
    /// This automatically sets the context token limit based on the provider's capabilities
    /// and applies user settings from AppConfig.pipeline.
    pub fn with_provider_optimized_config(config: AppConfig) -> Self {
        let provider = config.llm.primary.as_str();
        let pipeline_config =
            PipelineConfig::for_provider(provider).with_user_settings(&config.pipeline);

        tracing::info!(
            provider = provider,
            max_context_tokens = pipeline_config.max_context_tokens,
            max_history_messages = pipeline_config.max_history_messages,
            auto_compact_threshold = pipeline_config.auto_compact_threshold,
            compaction_strategy = ?pipeline_config.compaction_strategy,
            "Created pipeline with provider-optimized configuration and user settings"
        );

        let arc_config = std::sync::Arc::new(config.clone());
        let tool_router = build_tool_router(&pipeline_config.tool_routing_strategy, arc_config);
        Self {
            config,
            context_manager: Self::build_context_manager(),
            analyzer: RequestAnalyzer::new(),
            pipeline_config,
            permission_manager: PermissionManager::new(),
            knowledge_store: None,
            knowledge_settings: None,
            tool_router,
        }
    }

    fn effective_request_max_iterations(&self, request: &AgentRequest) -> Option<usize> {
        if let Some(override_limit) = request.max_iterations {
            return Some(override_limit);
        }

        if !self.pipeline_config.iteration_budget_enabled {
            return None;
        }

        if request.metadata.task_id.is_some() {
            Some(self.pipeline_config.tracked_task_max_iterations.max(1))
        } else {
            Some(self.pipeline_config.max_iterations.max(1))
        }
    }

    /// Process a request with streaming response
    ///
    /// This is the main entry point for streaming LLM interactions.
    /// It handles context reduction, tool filtering, and the agentic loop.
    pub async fn process_streaming(
        &self,
        request: AgentRequest,
        tx: mpsc::Sender<StreamChunk>,
        cancel_token: CancellationToken,
    ) -> Result<AgentResponse, AppError> {
        // 0. If resuming from a paused state, reconstruct the request with the
        //    full conversational context so the model continues from where it
        //    left off.
        let request = if let Some(paused) = request.resume_from.clone() {
            tracing::info!(
                iteration = paused.iteration,
                partial_len = paused.partial_content.len(),
                tool_calls = paused.completed_tool_calls.len(),
                "Resuming from paused execution state"
            );

            let _ = tx
                .send(StreamChunk::Status {
                    message: "Resuming paused session…".to_string(),
                })
                .await;

            // Build resumed history:
            // 1. Start with the history that existed before the paused request.
            let mut resumed_history = paused.history.clone();
            // 2. Re-add the original user message.
            resumed_history.push(Message::user(&paused.original_input));
            // 3. Append partial assistant content (if any) so the model sees it.
            if !paused.partial_content.is_empty() {
                resumed_history.push(Message::assistant(&paused.partial_content));
            }
            // 4. Append tool call results so the model has full tool context.
            for tc in &paused.completed_tool_calls {
                let output = match &tc.result {
                    ToolResult::Success(s) => s.clone(),
                    ToolResult::Error(e) => format!("Error: {e}"),
                    ToolResult::Skipped(msg) => format!("Skipped: {msg}"),
                };
                resumed_history.push(Message::tool_result(&tc.id, &output));
            }

            let resume_input = if paused.has_content() {
                "Please continue from where you left off. \
                 Your previous response was interrupted."
                    .to_string()
            } else {
                paused.original_input.clone()
            };

            let mut resumed = AgentRequest::new(resume_input)
                .with_streaming(request.streaming)
                .with_history(resumed_history);
            resumed.max_iterations = request.max_iterations;
            resumed.metadata = request.metadata.clone();
            if let Some(sp) = paused.system_prompt {
                resumed = resumed.with_system_prompt(sp);
            } else if let Some(sp) = request.system_prompt.clone() {
                resumed = resumed.with_system_prompt(sp);
            }
            resumed
        } else {
            request
        };

        // G1+G5: Auto-detect workspace_dir from the process working directory when
        // the caller did not supply one.  This ensures guardrails (AGENTS.md) and
        // the memory bank are always available in a standard project checkout.
        let mut request = request;
        if request.metadata.workspace_dir.is_none()
            && let Ok(cwd) = std::env::current_dir()
        {
            tracing::debug!(
                cwd = %cwd.display(),
                "workspace_dir not set; defaulting to CWD"
            );
            request.metadata.workspace_dir = Some(cwd);
        }
        Self::ensure_request_session_id(&mut request);
        Self::ensure_request_session_id(&mut request);

        let telemetry = AgentRequestTelemetry::start(
            &request,
            RequestRunMode::Streaming,
            self.config.pipeline.agent_telemetry.enabled,
        )
        .await;
        let result = telemetry.in_request_scope(async {

        // 1. Analyze the request
        let mut analysis = tracing::info_span!("agent.pipeline.analyze_request").in_scope(|| {
            let mut analysis = self.analyzer.analyze(&request.input);

            // Heuristic: if the user is replying with an approval ("ok", "please proceed")
            // and the previous assistant turn proposed using a tool, promote this turn into
            // a tool-enabled follow-up so the agent can actually execute the intended tool.
            self.promote_approval_to_tool_followup(&request, &mut analysis);
            analysis
        });
        // Emit analysis telemetry after local request-shaping heuristics have run
        // so the trace reflects the final analyzer state for this request.
        telemetry.record_analysis(&analysis).await;
        tracing::debug!(
            "Request analysis: categories={:?}, needs_tools={}, confidence={}",
            analysis.categories,
            analysis.needs_tools,
            analysis.confidence
        );

        // 1b. Pre-flight LLM tool routing (only when strategy != Keyword).
        // The router merges its selection into analysis.suggested_tools, which
        // get_tools_for_analysis() checks before the category map.
        if let Some(router) = &self.tool_router
            && analysis.needs_tools
        {
            let all: Vec<&'static ToolDefinition> = all_tools().iter().collect();
            let routing = router
                .route(&request.input, &all, analysis.confidence)
                .instrument(tracing::info_span!("agent.pipeline.route_tools"))
                .await;
            if routing.has_selection() {
                tracing::debug!(
                    tools = ?routing.suggested_tools,
                    "Pre-flight LLM router selected tools (streaming)"
                );
                analysis.suggested_tools = routing.suggested_tools;
            }
        }

        // 2. Filter tools based on categories (and allowed_tools if specified)
        let tools_enabled_for_request = request.metadata.tools_enabled.unwrap_or(true);

        let relevant_tools = if self.pipeline_config.enable_tools
            && tools_enabled_for_request
            && analysis.needs_tools
        {
            self.get_tools_for_analysis(&analysis, &request.metadata.allowed_tools)
        } else {
            Vec::new()
        };

        // Decide whether MCP tool *schemas* should be included for this request.
        //
        // We only want to touch MCP (connect, enumerate tools) when it is actually
        // relevant/allowed. Otherwise, slow/hung MCP servers can starve the streaming
        // UI of events and trip the GUI's idle timeout.
        let include_mcp_tool_schemas = self.pipeline_config.enable_tools
            && tools_enabled_for_request
            && analysis.needs_tools
            && (
                // The built-in MCP tool is part of the filtered tool set.
                relevant_tools.iter().any(|t| t.name == "mcp")
                    // Or the request explicitly whitelists MCP tools by name.
                    || request
                        .metadata
                        .allowed_tools
                        .iter()
                        .any(|t| t == "mcp" || t.starts_with("mcp__"))
            );
        Self::maybe_initialize_tracked_request_task(
            &mut request,
            analysis.needs_tools,
            relevant_tools.iter().any(|tool| tool.name == "task"),
        );
        // Record the final filtered tool set that will shape prompt/tool schema construction.
        telemetry
            .record_tool_selection(
                relevant_tools.len(),
                include_mcp_tool_schemas,
                analysis.needs_tools && relevant_tools.is_empty(),
            )
            .await;
        tracing::debug!(
            "Relevant tools: {:?}",
            relevant_tools.iter().map(|t| t.name).collect::<Vec<_>>()
        );

        // Workspace sandboxing (used by tool execution)
        let workspace = request.metadata.workspace_dir.as_ref().and_then(|p| {
            SessionWorkspace::from_directory(
                request.metadata.session_id.as_deref().unwrap_or("unknown"),
                p.clone(),
            )
            .ok()
        });

        // Fast-path: if the user is explicitly approving a previously proposed tool call
        // (e.g. "okay please proceed"), execute the intended tool directly from history.
        //
        // This prevents a common UX failure mode where the model describes tool usage,
        // the user approves, but the provider doesn't emit a structured tool call so the
        // app appears to "hang" or never produces an answer.
        if self.pipeline_config.enable_tools
            && tools_enabled_for_request
            && analysis.is_followup
            && Self::looks_like_approval(&request.input)
            && let Some(resp) = self
                .try_execute_confirmed_tool_from_history(
                    &request,
                    &analysis,
                    &relevant_tools,
                    workspace.as_ref(),
                    &tx,
                    &cancel_token,
                )
                .await?
        {
            return Ok(resp);
        }

        // 2b. If MCP tools are relevant for this request, pre-connect MCP servers with
        // streaming keepalive status so we never silently block before the first LLM chunk.
        if include_mcp_tool_schemas {
            self.ensure_mcp_servers_connected_streaming(&tx, &cancel_token)
                .instrument(tracing::info_span!("agent.pipeline.connect_mcp"))
                .await;
        }

        // 3. Resolve context
        let mut resolved_context = tracing::info_span!(
            "agent.pipeline.resolve_context",
            phase = "initial"
        )
        .in_scope(|| {
            self.context_manager.resolve_context(
                &request.input,
                &analysis,
                &request.history,
                request.metadata.workspace_dir.as_deref(),
            )
        });

        // 3.1+3.2. Enrich context with memory bank and enabled knowledge items.
        self.enrich_resolved_context(
            &mut resolved_context,
            request.metadata.workspace_dir.as_deref(),
            &request.input,
            &request.metadata,
        )
        .instrument(tracing::info_span!(
            "agent.pipeline.enrich_context",
            phase = "initial"
        ))
        .await;
        // Capture the enriched context before compaction/prompt construction.
        telemetry
            .record_context_resolved("initial", &resolved_context)
            .await;

        // 3.5. Check for auto-compaction before building prompt
        // Build a preview prompt to estimate tokens
        let preview_prompt = tracing::info_span!("agent.pipeline.build_preview_prompt")
            .in_scope(|| self.build_prompt(&request, &resolved_context));
        if let Some(compaction_chunk) = self
            .check_and_apply_auto_compaction(&request.history, &preview_prompt, &request.metadata)
            .instrument(tracing::info_span!("agent.pipeline.auto_compaction"))
            .await
        {
            telemetry.record_compaction(&compaction_chunk).await;
            // Emit user-visible status **before** the compaction result chunk.
            let message = self.build_auto_compaction_status_message(&preview_prompt);
            let _ = tx.send(StreamChunk::Status { message }).await;

            // Emit compaction notification to user
            let _ = tx.send(compaction_chunk).await;

            // Re-resolve context after compaction and re-enrich (G4: enrichment
            // must run again so memory bank + knowledge survive compaction).
            resolved_context = tracing::info_span!(
                "agent.pipeline.resolve_context",
                phase = "post_compaction"
            )
            .in_scope(|| {
                self.context_manager.resolve_context(
                    &request.input,
                    &analysis,
                    &request.history,
                    request.metadata.workspace_dir.as_deref(),
                )
            });
            self.enrich_resolved_context(
                &mut resolved_context,
                request.metadata.workspace_dir.as_deref(),
                &request.input,
                &request.metadata,
            )
            .instrument(tracing::info_span!(
                "agent.pipeline.enrich_context",
                phase = "post_compaction"
            ))
            .await;
            telemetry
                .record_context_resolved("post_compaction", &resolved_context)
                .await;
        }

        // 4. Build the optimized prompt with token limit checking
        let (prompt, truncated) = tracing::info_span!("agent.pipeline.prepare_prompt")
            .in_scope(|| self.truncate_prompt_if_needed(&request, &mut resolved_context));
        telemetry
            .record_prompt_prepared(&prompt, truncated, &resolved_context)
            .await;

        if truncated {
            tracing::info!("Prompt was truncated to fit token limit");
        }

        // 4.5. Hard validation: reject if still over limit after truncation
        // This prevents API errors and provides clear feedback to the user
        self.validate_token_limit(&prompt)?;

        // 4.6. Emit token usage update for user visibility
        let token_usage_chunk = self.create_token_usage_update(&prompt);
        let _ = tx.send(token_usage_chunk).await;

        // 4.7. Run PrePipeline hooks (if enabled)
        let hook_engine = self.create_hook_engine();
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: request.metadata.workspace_dir.clone(),
                session_id: request.metadata.session_id.clone(),
                pipeline_prompt: Some(prompt.clone()),
                ..Default::default()
            };
            if let Err(e) = engine.run(HookEvent::PrePipeline, &hook_ctx).await {
                tracing::warn!(error = %e, "PrePipeline hook failed (continuing)");
            }
        }

        // 5. Execute the agentic loop with workspace sandboxing.
        // 6. If experiential reflection is enabled, evaluate the completed turn,
        //    optionally generate a structured reflection, attempt one bounded
        //    corrective retry (text revision or safe re-execution), and
        //    consolidate the result into memory before running PostPipeline hooks.
        let reflection_tx = tx.clone();
        let reflection_cancel_token = cancel_token.clone();
        let reflection_retry_prompt = prompt.clone();
        let reflection_retry_tools = relevant_tools.clone();
        let reflection_retry_context = resolved_context.clone();
        let effective_max_iterations = self.effective_request_max_iterations(&request);
        let reflection_quality_budget = effective_max_iterations.unwrap_or(0);
        let relevant_tool_count = relevant_tools.len();
        let requires_build_and_test = Self::prompt_requires_build_and_test(&request.input);
        let requires_mutating_file_tool_success =
            Self::request_requires_mutating_file_tool_success(&request.input);
        if requires_build_and_test {
            tracing::warn!(
                request_input_preview = %request.input.chars().take(160).collect::<String>(),
                source = ?request.metadata.source,
                session_id = ?request.metadata.session_id,
                task_id = ?request.metadata.task_id,
                "Agent request seeded requires_build_and_test=true from request.input"
            );
        }
        let mut response = self
            .execute_agentic_loop_streaming(
                prompt,
                requires_build_and_test,
                requires_mutating_file_tool_success,
                relevant_tools,
                include_mcp_tool_schemas,
                resolved_context,
                tx,
                cancel_token,
                workspace.as_ref(),
                request.metadata.session_id.clone(),
                request.metadata.task_id.clone(),
                effective_max_iterations,
                request.metadata.permission_level,
                &telemetry,
            )
            .instrument(tracing::info_span!(
                "agent.pipeline.execute_agent_loop",
                mode = "streaming",
                tools = relevant_tool_count,
                max_iterations = ?effective_max_iterations
            ))
            .await?;

        response.truncated = truncated;

        if reflection_cancel_token.is_cancelled() {
            telemetry.mark_outcome(RequestOutcome::Cancelled);
            return Ok(response);
        }

        if let Some(mut generated_reflection) = self
            .maybe_generate_reflection(
                &request.input,
                &response,
                &request.metadata,
                Some(&reflection_tx),
                &reflection_cancel_token,
                reflection_quality_budget,
            )
            .instrument(tracing::info_span!("agent.pipeline.reflection", mode = "streaming"))
            .await
        {
            telemetry
                .record_reflection_generated(
                    generated_reflection.initial_quality_score,
                    generated_reflection.reflection.promotion_confidence(),
                )
                .await;
            let retry = if self.should_attempt_reflection_reexecution(
                &response,
                &generated_reflection.reflection,
                !reflection_retry_tools.is_empty(),
            ) {
                let _ = reflection_tx
                    .send(StreamChunk::Status {
                        message: "Low-confidence answer detected; running one safe reflection-guided retry with sandboxed tool access...".to_string(),
                    })
                    .await;
                self.maybe_run_reflection_reexecution(
                    &response,
                    &generated_reflection.reflection,
                    generated_reflection.initial_quality_score,
                    reflection::ReflectionReexecutionContext {
                        base_prompt: &reflection_retry_prompt,
                        tools: reflection_retry_tools.clone(),
                        context: reflection_retry_context.clone(),
                        workspace: workspace.as_ref(),
                    },
                )
                .await
            } else {
                self.maybe_run_reflection_retry(
                    &request.input,
                    &response,
                    &generated_reflection.reflection,
                    generated_reflection.initial_quality_score,
                    Some(&reflection_tx),
                )
                .await
            };

            if let Some(retry) = retry.as_ref() {
                telemetry
                    .record_reflection_retry(
                        retry.improved,
                        retry.improvement_score,
                        retry.iterations,
                    )
                    .await;
                if let Some(usage) = retry.usage.as_ref() {
                    merge_token_usage(&mut response.usage, usage);
                }
                generated_reflection.reflection.improvement_score = Some(retry.improvement_score);
                response.tool_calls.extend(retry.tool_calls.clone());
                response.iterations += retry.iterations;

                if retry.improved {
                    let status_message = match retry.mode {
                        reflection::ReflectionRetryMode::TextRevision => format!(
                            "Reflection-guided revision improved quality from {:.0}% to {:.0}%",
                            generated_reflection.initial_quality_score * 100.0,
                            retry.retry_quality_score * 100.0,
                        ),
                        reflection::ReflectionRetryMode::CorrectiveReexecution => format!(
                            "Reflection-guided safe retry improved quality from {:.0}% to {:.0}% using {} tool calls across {} iterations",
                            generated_reflection.initial_quality_score * 100.0,
                            retry.retry_quality_score * 100.0,
                            retry.tool_calls.len(),
                            retry.iterations,
                        ),
                    };
                    let _ = reflection_tx
                        .send(StreamChunk::Status {
                            message: status_message,
                        })
                        .await;

                    let revised_block = format!(
                        "{}{}",
                        reflection::REFLECTION_RETRY_SEPARATOR,
                        retry.revised_content
                    );
                    response.content.push_str(&revised_block);
                    self.emit_reflection_retry_text(&reflection_tx, &revised_block)
                        .await;
                } else {
                    let status_message = match retry.mode {
                        reflection::ReflectionRetryMode::TextRevision => format!(
                            "Reflection-guided revision did not materially improve quality ({:.0}% → {:.0}%)",
                            generated_reflection.initial_quality_score * 100.0,
                            retry.retry_quality_score * 100.0,
                        ),
                        reflection::ReflectionRetryMode::CorrectiveReexecution => format!(
                            "Reflection-guided safe retry did not materially improve quality ({:.0}% → {:.0}%, {} tool calls)",
                            generated_reflection.initial_quality_score * 100.0,
                            retry.retry_quality_score * 100.0,
                            retry.tool_calls.len(),
                        ),
                    };
                    let _ = reflection_tx
                        .send(StreamChunk::Status {
                            message: status_message,
                        })
                        .await;
                }
            }

            self.finalize_reflection(
                &generated_reflection.reflection,
                &request.metadata,
                workspace.as_ref(),
                Some(&reflection_tx),
                retry.as_ref(),
            )
            .await;
        }

        // 7. Run PostPipeline hooks (best-effort) after the reflection phase.
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: request.metadata.workspace_dir.clone(),
                session_id: request.metadata.session_id.clone(),
                ..Default::default()
            };
            self.run_hook_best_effort(engine, HookEvent::PostPipeline, &hook_ctx)
                .await;
        }

        if !reflection_cancel_token.is_cancelled() {
            let _ = reflection_tx
                .send(StreamChunk::Done(response.usage.clone()))
                .await;
        }

        Ok(response)
        })
        .await;

        match &result {
            Ok(response) => telemetry.finish(Some(response), None).await,
            Err(error) => {
                telemetry.mark_outcome(RequestOutcome::Failed);
                telemetry.finish(None, Some(error)).await;
            }
        }

        result
    }

    fn looks_like_approval(input: &str) -> bool {
        let s = input.trim().to_lowercase();
        matches!(
            s.as_str(),
            "ok" | "okay"
                | "ok."
                | "okay."
                | "yes"
                | "y"
                | "sure"
                | "please proceed"
                | "proceed"
                | "go ahead"
                | "do it"
                | "run it"
                | "continue"
        ) || s.contains("please proceed")
            || s.contains("go ahead")
            || s.contains("please do")
            || s.contains("yes, proceed")
    }

    /// Attempt to execute a previously proposed tool call directly from the assistant's
    /// last message, when the current user turn is an approval/follow-up.
    ///
    /// This is a defensive fallback for a common failure mode where the model:
    /// 1) proposes a tool call,
    /// 2) asks for confirmation,
    /// 3) after user approval, fails to emit a structured tool call.
    ///
    /// We infer the intended tool from the previous assistant message and execute it.
    async fn try_execute_confirmed_tool_from_history(
        &self,
        request: &AgentRequest,
        _analysis: &crate::context::RequestAnalysis,
        relevant_tools: &[&'static ToolDefinition],
        workspace: Option<&SessionWorkspace>,
        tx: &mpsc::Sender<StreamChunk>,
        cancel_token: &CancellationToken,
    ) -> Result<Option<AgentResponse>, AppError> {
        let has_tool = |name: &str| relevant_tools.iter().any(|t| t.name == name);

        let Some(prev_assistant) = request.history.iter().rev().find(|m| m.role == "assistant")
        else {
            return Ok(None);
        };

        let Some((tool_name, args, _answer_prefix)) =
            Self::extract_planned_tool_call_from_text(&prev_assistant.content)
        else {
            return Ok(None);
        };

        // Only run if the tool is actually available on this turn.
        if !has_tool(&tool_name) {
            return Ok(None);
        }

        // Execute immediately (still subject to the normal safety checks inside execute_tool).
        let tool_call_id = format!("confirmed_{tool_name}");

        let _ = tx
            .send(StreamChunk::Thinking(
                "Executing approved command...\n".to_string(),
            ))
            .await;
        let _ = tx
            .send(StreamChunk::ToolCallStart {
                id: tool_call_id.clone(),
                name: tool_name.clone(),
            })
            .await;
        let _ = tx.send(StreamChunk::ToolCallArgs(args.clone())).await;

        let start_time = Instant::now();
        let result = self
            .execute_tool(&tool_name, &args, workspace, Some(tx))
            .await;
        let duration_ms = start_time.elapsed().as_millis() as u64;

        let _ = tx.send(StreamChunk::ToolCallEnd).await;

        // Emit structured tool result for frontend display
        let (success, output) = match &result {
            ToolResult::Success(out) => (true, out.trim_end().to_string()),
            ToolResult::Error(e) => (false, e.clone()),
            ToolResult::Skipped(msg) => (false, format!("Skipped: {}", msg)),
        };
        let _ = tx
            .send(StreamChunk::ToolCallResult {
                name: tool_name.clone(),
                success,
                output: output.clone(),
                duration_ms,
            })
            .await;

        let record = ToolCallRecord {
            id: tool_call_id,
            name: tool_name,
            arguments: args,
            result,
            duration_ms,
        };

        // Build a continuation prompt so the LLM can synthesize the tool output
        // into a helpful response for the user, instead of leaving raw tool output
        // as the final answer.
        let base_prompt = self.build_prompt(request, &crate::context::ResolvedContext::default());
        let continuation_prompt = self.build_tool_continuation_prompt(
            &base_prompt,
            "Executing the approved tool call.",
            std::slice::from_ref(&record),
        );

        // Stream one more LLM call for synthesis (no tool schemas — text only).
        let (inner_tx, mut inner_rx) = mpsc::channel::<StreamChunk>(100);
        let streaming_cfg = crate::streaming::streaming_config_from(&self.config);
        let enable_fallback = self.pipeline_config.enable_fallback;
        let inner_cancel = cancel_token.clone();

        let stream_handle = tokio::spawn(
            async move {
                if enable_fallback {
                    let _ = start_streaming_with_fallback(
                        &streaming_cfg,
                        &continuation_prompt,
                        None,
                        inner_tx,
                        inner_cancel,
                    )
                    .await;
                } else {
                    let _ = start_streaming(
                        &streaming_cfg,
                        &continuation_prompt,
                        None,
                        inner_tx,
                        inner_cancel,
                    )
                    .await;
                }
            }
            .instrument(tracing::Span::current()),
        );

        let mut synthesis_text = String::new();
        let mut synthesis_usage = None;

        while let Some(chunk) = inner_rx.recv().await {
            match &chunk {
                StreamChunk::Text(text) => {
                    synthesis_text.push_str(text);
                    let _ = tx.send(chunk).await;
                }
                StreamChunk::Thinking(_) => {
                    let _ = tx.send(chunk).await;
                }
                StreamChunk::Done(usage) => {
                    synthesis_usage = usage.clone();
                    break;
                }
                StreamChunk::Error(_) | StreamChunk::Cancelled | StreamChunk::Paused => {
                    let _ = tx.send(chunk).await;
                    break;
                }
                _ => {
                    // Forward status or other informational chunks
                    let _ = tx.send(chunk).await;
                }
            }
        }

        let _ = stream_handle.await;
        let _ = tx.send(StreamChunk::Done(synthesis_usage.clone())).await;

        Ok(Some(AgentResponse {
            content: synthesis_text,
            thinking: None,
            tool_calls: vec![record],
            usage: synthesis_usage,
            context_used: crate::context::ResolvedContext::default(),
            truncated: false,
            iterations: 1,
        }))
    }

    fn extract_shell_command_from_plan(text: &str) -> Option<String> {
        // Try common patterns first: run '...'/"..."/`...`
        if let Some(cmd) = Self::extract_quoted_after_keyword(text, "run") {
            return Some(cmd);
        }
        if let Some(cmd) = Self::extract_quoted_after_keyword(text, "execute") {
            return Some(cmd);
        }

        // Fallback: try to grab the first token after "run".
        let lower = text.to_lowercase();
        let idx = lower.find("run ")?;
        let after = text[idx + 4..].trim_start();
        let token: String = after
            .chars()
            .take_while(|c| !c.is_whitespace() && !matches!(*c, '.' | ',' | ';' | ')' | '('))
            .collect();
        if token.is_empty() { None } else { Some(token) }
    }

    fn extract_quoted_after_keyword(text: &str, keyword: &str) -> Option<String> {
        let lower = text.to_lowercase();
        let key = format!("{} ", keyword.to_lowercase());
        let start = lower.find(&key)?;
        let mut rest = text[start + key.len()..].trim_start();
        let quote = rest.chars().next()?;
        if quote != '\'' && quote != '"' && quote != '`' {
            return None;
        }
        rest = &rest[quote.len_utf8()..];
        let end = rest.find(quote)?;
        let cmd = rest[..end].trim().to_string();
        if cmd.is_empty() { None } else { Some(cmd) }
    }

    fn extract_first_quoted(text: &str) -> Option<String> {
        for quote in ['\'', '"', '`'] {
            if let Some(start) = text.find(quote) {
                let rest = &text[start + quote.len_utf8()..];
                if let Some(end) = rest.find(quote) {
                    let s = rest[..end].trim();
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
        }
        None
    }

    fn extract_first_url(text: &str) -> Option<String> {
        let lower = text.to_lowercase();
        let idx = lower.find("https://").or_else(|| lower.find("http://"))?;
        let after = &text[idx..];
        let url: String = after
            .chars()
            .take_while(|c| {
                !c.is_whitespace() && !matches!(*c, ')' | '(' | ']' | '[' | '"' | '\'' | '`' | ',')
            })
            .collect();
        if url.is_empty() { None } else { Some(url) }
    }

    /// Infer a planned tool call from a prior assistant message.
    ///
    /// Returns: (tool_name, args_json, answer_prefix)
    fn extract_planned_tool_call_from_text(text: &str) -> Option<(String, String, String)> {
        let lower = text.to_lowercase();

        // Shell
        if lower.contains("shell tool")
            || lower.contains("tool: shell")
            || lower.contains("use the shell")
            || lower.contains("run 'pwd'")
            || lower.contains("`pwd`")
        {
            let command = Self::extract_shell_command_from_plan(text)?;
            let args = serde_json::json!({"command": command}).to_string();
            return Some((
                "shell".to_string(),
                args,
                "Workspace directory root: ".to_string(),
            ));
        }

        // File read
        if lower.contains("file tool")
            || lower.contains("read file")
            || lower.contains("read the file")
        {
            let path = Self::extract_quoted_after_keyword(text, "read")
                .or_else(|| Self::extract_first_quoted(text))?;
            let args = serde_json::json!({"operation": "read", "path": path}).to_string();
            return Some(("file".to_string(), args, "File contents: \n".to_string()));
        }

        // Git status
        if lower.contains("git tool") || lower.contains("git status") {
            let args = serde_json::json!({"operation": "status"}).to_string();
            return Some(("git".to_string(), args, "Git status:\n".to_string()));
        }

        // Web
        if lower.contains("web tool")
            || lower.contains("search the web")
            || lower.contains("web_search")
        {
            if lower.contains("fetch") || lower.contains("download") {
                let url = Self::extract_first_url(text)?;
                let args = serde_json::json!({"operation": "fetch", "url": url}).to_string();
                return Some(("web".to_string(), args, "Web fetch result:\n".to_string()));
            }

            let query = Self::extract_quoted_after_keyword(text, "search")
                .or_else(|| Self::extract_first_quoted(text))?;
            let args = serde_json::json!({"operation": "search", "query": query}).to_string();
            return Some(("web".to_string(), args, "Web search results:\n".to_string()));
        }

        // Code stats
        if lower.contains("code tool") || lower.contains("code stats") {
            let path = Self::extract_first_quoted(text).unwrap_or_else(|| ".".to_string());
            let args = serde_json::json!({"operation": "stats", "path": path}).to_string();
            return Some(("code".to_string(), args, "Code stats:\n".to_string()));
        }

        None
    }

    /// Process a request without streaming (blocking)
    pub async fn process_blocking(&self, request: AgentRequest) -> Result<AgentResponse, AppError> {
        // G1+G5: Auto-detect workspace_dir from the process working directory when
        // the caller did not supply one.  This ensures guardrails (AGENTS.md) and
        // the memory bank are always available in a standard project checkout.
        let mut request = request;
        if request.metadata.workspace_dir.is_none()
            && let Ok(cwd) = std::env::current_dir()
        {
            tracing::debug!(
                cwd = %cwd.display(),
                "workspace_dir not set; defaulting to CWD (blocking path)"
            );
            request.metadata.workspace_dir = Some(cwd);
        }

        let telemetry = AgentRequestTelemetry::start(
            &request,
            RequestRunMode::Blocking,
            self.config.pipeline.agent_telemetry.enabled,
        )
        .await;
        let result = telemetry.in_request_scope(async {

        // 1. Analyze the request
        let mut analysis = tracing::info_span!("agent.pipeline.analyze_request").in_scope(|| {
            self.analyzer.analyze(&request.input)
        });
        telemetry.record_analysis(&analysis).await;

        // 1b. Pre-flight LLM tool routing (only when strategy != Keyword).
        if let Some(router) = &self.tool_router
            && analysis.needs_tools
        {
            let all: Vec<&'static ToolDefinition> = all_tools().iter().collect();
            let routing = router
                .route(&request.input, &all, analysis.confidence)
                .instrument(tracing::info_span!("agent.pipeline.route_tools"))
                .await;
            if routing.has_selection() {
                tracing::debug!(
                    tools = ?routing.suggested_tools,
                    "Pre-flight LLM router selected tools (blocking)"
                );
                analysis.suggested_tools = routing.suggested_tools;
            }
        }

        // 2. Filter tools (and allowed_tools if specified)
        let tools_enabled_for_request = request.metadata.tools_enabled.unwrap_or(true);

        let relevant_tools = if self.pipeline_config.enable_tools
            && tools_enabled_for_request
            && analysis.needs_tools
        {
            self.get_tools_for_analysis(&analysis, &request.metadata.allowed_tools)
        } else {
            Vec::new()
        };

        let include_mcp_tool_schemas = self.pipeline_config.enable_tools
            && tools_enabled_for_request
            && analysis.needs_tools
            && (relevant_tools.iter().any(|t| t.name == "mcp")
                || request
                    .metadata
                    .allowed_tools
                    .iter()
                    .any(|t| t == "mcp" || t.starts_with("mcp__")));
        Self::maybe_initialize_tracked_request_task(
            &mut request,
            analysis.needs_tools,
            relevant_tools.iter().any(|tool| tool.name == "task"),
        );
        telemetry
            .record_tool_selection(
                relevant_tools.len(),
                include_mcp_tool_schemas,
                analysis.needs_tools && relevant_tools.is_empty(),
            )
            .await;

        // Blocking mode: connect MCP servers only when MCP is relevant/allowed.
        if include_mcp_tool_schemas {
            self.ensure_mcp_servers_connected()
                .instrument(tracing::info_span!("agent.pipeline.connect_mcp"))
                .await;
        }

        // 3. Resolve context
        let mut resolved_context = tracing::info_span!(
            "agent.pipeline.resolve_context",
            phase = "initial"
        )
        .in_scope(|| {
            self.context_manager.resolve_context(
                &request.input,
                &analysis,
                &request.history,
                request.metadata.workspace_dir.as_deref(),
            )
        });

        // 3.1+3.2. Enrich context with memory bank and enabled knowledge items.
        self.enrich_resolved_context(
            &mut resolved_context,
            request.metadata.workspace_dir.as_deref(),
            &request.input,
            &request.metadata,
        )
        .instrument(tracing::info_span!(
            "agent.pipeline.enrich_context",
            phase = "initial"
        ))
        .await;
        telemetry
            .record_context_resolved("initial", &resolved_context)
            .await;

        // 3.5. Check for auto-compaction before building prompt
        // Build a preview prompt to estimate tokens
        let preview_prompt = tracing::info_span!("agent.pipeline.build_preview_prompt")
            .in_scope(|| self.build_prompt(&request, &resolved_context));
        if let Some(compaction_chunk) = self
            .check_and_apply_auto_compaction(&request.history, &preview_prompt, &request.metadata)
            .instrument(tracing::info_span!("agent.pipeline.auto_compaction"))
            .await
        {
            telemetry.record_compaction(&compaction_chunk).await;
            // Log compaction in blocking mode (no stream to emit to)
            match compaction_chunk {
                StreamChunk::ContextCompacted {
                    messages_before,
                    messages_after,
                    tokens_saved,
                    summary,
                } => {
                    tracing::info!(
                        messages_before = messages_before,
                        messages_after = messages_after,
                        tokens_saved = tokens_saved,
                        "Context auto-compacted in blocking mode: {}",
                        summary
                    );
                }
                StreamChunk::MemoryBankSaved {
                    file_path,
                    session_id,
                    summary,
                    messages_saved,
                } => {
                    tracing::info!(
                        file_path = %file_path,
                        session_id = %session_id,
                        messages_saved = messages_saved,
                        "Memory bank saved in blocking mode: {}",
                        summary
                    );
                }
                _ => {}
            }

            // Re-resolve context after compaction and re-enrich (G4: enrichment
            // must run again so memory bank + knowledge survive compaction).
            resolved_context = tracing::info_span!(
                "agent.pipeline.resolve_context",
                phase = "post_compaction"
            )
            .in_scope(|| {
                self.context_manager.resolve_context(
                    &request.input,
                    &analysis,
                    &request.history,
                    request.metadata.workspace_dir.as_deref(),
                )
            });
            self.enrich_resolved_context(
                &mut resolved_context,
                request.metadata.workspace_dir.as_deref(),
                &request.input,
                &request.metadata,
            )
            .instrument(tracing::info_span!(
                "agent.pipeline.enrich_context",
                phase = "post_compaction"
            ))
            .await;
            telemetry
                .record_context_resolved("post_compaction", &resolved_context)
                .await;
        }

        // 4. Build prompt with token limit checking
        let (prompt, truncated) = tracing::info_span!("agent.pipeline.prepare_prompt")
            .in_scope(|| self.truncate_prompt_if_needed(&request, &mut resolved_context));
        telemetry
            .record_prompt_prepared(&prompt, truncated, &resolved_context)
            .await;

        if truncated {
            tracing::info!("Prompt was truncated to fit token limit");
        }

        // 4.5. Hard validation: reject if still over limit after truncation
        // This prevents API errors and provides clear feedback to the user
        self.validate_token_limit(&prompt)?;

        // 4.6. Log token usage in blocking mode
        if let StreamChunk::TokenUsageUpdate {
            estimated,
            limit,
            percentage,
            status,
            estimated_cost,
        } = self.create_token_usage_update(&prompt)
        {
            let status_str = match status {
                crate::streaming::TokenUsageStatus::Green => "🟢 Green",
                crate::streaming::TokenUsageStatus::Yellow => "🟡 Yellow",
                crate::streaming::TokenUsageStatus::Red => "🔴 Red",
            };
            tracing::info!(
                estimated_tokens = estimated,
                limit = limit,
                percentage = percentage,
                status = status_str,
                estimated_cost_usd = format!("${:.4}", estimated_cost),
                "Token usage in blocking mode: {} tokens / {} tokens ({}%) - Est. cost: ${:.4}",
                estimated,
                limit,
                percentage,
                estimated_cost
            );
        }

        // 4.7. Run PrePipeline hooks (if enabled)
        let hook_engine = self.create_hook_engine();
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: request.metadata.workspace_dir.clone(),
                session_id: request.metadata.session_id.clone(),
                pipeline_prompt: Some(prompt.clone()),
                ..Default::default()
            };
            if let Err(e) = engine.run(HookEvent::PrePipeline, &hook_ctx).await {
                tracing::warn!(error = %e, "PrePipeline hook failed in blocking mode (continuing)");
            }
        }

        // 5. Execute blocking agentic loop with workspace sandboxing
        let workspace = request.metadata.workspace_dir.as_ref().and_then(|p| {
            SessionWorkspace::from_directory(
                request.metadata.session_id.as_deref().unwrap_or("unknown"),
                p.clone(),
            )
            .ok()
        });
        let reflection_retry_prompt = prompt.clone();
        let reflection_retry_tools = relevant_tools.clone();
        let reflection_retry_context = resolved_context.clone();
        let effective_max_iterations = self.effective_request_max_iterations(&request);
        let reflection_quality_budget = effective_max_iterations.unwrap_or(0);
        let relevant_tool_count = relevant_tools.len();
        let requires_build_and_test = Self::prompt_requires_build_and_test(&request.input);
        let requires_mutating_file_tool_success =
            Self::request_requires_mutating_file_tool_success(&request.input);
        if requires_build_and_test {
            tracing::warn!(
                request_input_preview = %request.input.chars().take(160).collect::<String>(),
                source = ?request.metadata.source,
                session_id = ?request.metadata.session_id,
                task_id = ?request.metadata.task_id,
                "Agent request seeded requires_build_and_test=true from request.input"
            );
        }

        let mut response = self
            .execute_agentic_loop_blocking(
                prompt,
                requires_build_and_test,
                requires_mutating_file_tool_success,
                relevant_tools,
                include_mcp_tool_schemas,
                resolved_context,
                workspace.as_ref(),
                request.metadata.session_id.clone(),
                request.metadata.task_id.clone(),
                effective_max_iterations,
                &telemetry,
            )
            .instrument(tracing::info_span!(
                "agent.pipeline.execute_agent_loop",
                mode = "blocking",
                tools = relevant_tool_count,
                max_iterations = ?effective_max_iterations
            ))
            .await?;

        response.truncated = truncated;

        if let Some(mut generated_reflection) = self
            .maybe_generate_reflection(
                &request.input,
                &response,
                &request.metadata,
                None,
                &crate::streaming::CancellationToken::new(),
                reflection_quality_budget,
            )
            .instrument(tracing::info_span!("agent.pipeline.reflection", mode = "blocking"))
            .await
        {
            telemetry
                .record_reflection_generated(
                    generated_reflection.initial_quality_score,
                    generated_reflection.reflection.promotion_confidence(),
                )
                .await;
            let retry = if self.should_attempt_reflection_reexecution(
                &response,
                &generated_reflection.reflection,
                !reflection_retry_tools.is_empty(),
            ) {
                self.maybe_run_reflection_reexecution(
                    &response,
                    &generated_reflection.reflection,
                    generated_reflection.initial_quality_score,
                    reflection::ReflectionReexecutionContext {
                        base_prompt: &reflection_retry_prompt,
                        tools: reflection_retry_tools,
                        context: reflection_retry_context,
                        workspace: workspace.as_ref(),
                    },
                )
                .await
            } else {
                self.maybe_run_reflection_retry(
                    &request.input,
                    &response,
                    &generated_reflection.reflection,
                    generated_reflection.initial_quality_score,
                    None,
                )
                .await
            };

            if let Some(retry) = retry.as_ref() {
                telemetry
                    .record_reflection_retry(
                        retry.improved,
                        retry.improvement_score,
                        retry.iterations,
                    )
                    .await;
                if let Some(usage) = retry.usage.as_ref() {
                    merge_token_usage(&mut response.usage, usage);
                }
                generated_reflection.reflection.improvement_score = Some(retry.improvement_score);
                response.tool_calls.extend(retry.tool_calls.clone());
                response.iterations += retry.iterations;

                if retry.improved {
                    response.content = retry.revised_content.clone();
                    response.thinking = None;
                }
            }

            self.finalize_reflection(
                &generated_reflection.reflection,
                &request.metadata,
                workspace.as_ref(),
                None,
                retry.as_ref(),
            )
            .await;
        }

        // 5.1. Run PostPipeline hooks (best-effort)
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: request.metadata.workspace_dir.clone(),
                session_id: request.metadata.session_id.clone(),
                ..Default::default()
            };
            self.run_hook_best_effort(engine, HookEvent::PostPipeline, &hook_ctx)
                .await;
        }

        Ok(response)
        })
        .await;

        match &result {
            Ok(response) => telemetry.finish(Some(response), None).await,
            Err(error) => {
                telemetry.mark_outcome(RequestOutcome::Failed);
                telemetry.finish(None, Some(error)).await;
            }
        }

        result
    }

    /// Get tools relevant to the analyzed request
    /// If allowed_tools is non-empty, treat them as the candidate pool and still
    /// narrow to the tools that are relevant for the analyzed request.
    fn get_tools_for_analysis(
        &self,
        analysis: &crate::context::RequestAnalysis,
        allowed_tools: &[String],
    ) -> Vec<&'static ToolDefinition> {
        use crate::context::ContextCategory;

        let candidate_tools: Vec<_> = if allowed_tools.is_empty() {
            all_tools().iter().collect()
        } else {
            allowed_tools
                .iter()
                .filter_map(|tool_name| crate::tools::registry::find_tool(tool_name))
                .collect()
        };
        let candidate_names: HashSet<_> = candidate_tools.iter().map(|tool| tool.name).collect();
        let mut tools = Vec::new();

        if !allowed_tools.is_empty() && self.pipeline_config.log_token_usage {
            tracing::debug!(
                allowed_tools = ?allowed_tools,
                candidate_tools = ?candidate_tools.iter().map(|t| t.name).collect::<Vec<_>>(),
                "Using session-specific tool configuration as candidate pool"
            );
        }

        // If the pre-flight LLM router already chose tools, use that selection
        // directly (skipping the category map entirely for this request), but keep
        // the selection inside the candidate pool.
        if !analysis.suggested_tools.is_empty() {
            let mut resolved: Vec<_> = analysis
                .suggested_tools
                .iter()
                .filter(|name| candidate_names.contains(name.as_str()))
                .filter_map(|name| crate::tools::registry::find_tool(name))
                .collect();

            Self::append_task_tool_for_auto_tracked_request(
                analysis,
                &candidate_names,
                &mut resolved,
            );

            if !resolved.is_empty() {
                resolved.sort_by_key(|tool| tool.name);
                resolved.dedup_by_key(|tool| tool.name);
                if self.pipeline_config.log_token_usage {
                    tracing::debug!(
                        suggested = ?analysis.suggested_tools,
                        resolved_tools = ?resolved.iter().map(|t| t.name).collect::<Vec<_>>(),
                        "Using LLM router tool selection"
                    );
                }
                return resolved;
            }
        }

        let mut push_if_allowed = |tool_name: &str| {
            if candidate_names.contains(tool_name)
                && let Some(tool) = crate::tools::registry::find_tool(tool_name)
            {
                tools.push(tool);
            }
        };

        // Otherwise, filter the candidate pool by category.
        for category in &analysis.categories {
            match category {
                ContextCategory::FileSystem => push_if_allowed("file"),
                ContextCategory::Shell => push_if_allowed("shell"),
                ContextCategory::Git => push_if_allowed("git"),
                ContextCategory::Code => {
                    for tool_name in crate::tools::registry::code_tool_names() {
                        push_if_allowed(tool_name);
                    }
                }
                ContextCategory::Web => {
                    push_if_allowed("web");
                    // Also include web_search for search-related queries
                    push_if_allowed("web_search");
                }
                ContextCategory::Screen => {
                    push_if_allowed("screenshot");
                    push_if_allowed("screen_record");
                }
                ContextCategory::Agent => push_if_allowed("a2a"),
                ContextCategory::Mcp => push_if_allowed("mcp"),
                ContextCategory::A2a => push_if_allowed("a2a"),
                ContextCategory::Task => push_if_allowed("task"),
                ContextCategory::Tools => push_if_allowed("permissions"),
                ContextCategory::Voice
                | ContextCategory::Config
                | ContextCategory::Session
                | ContextCategory::General => {}
            }
        }

        // If no specific tools found, or confidence is too low to trust the category match,
        // fall back to the entire candidate pool so the LLM can make the correct selection
        // without seeing disabled or irrelevant session tools.
        // confidence < 0.2 means only a single weak keyword fired — not reliable enough to
        // narrow the tool set, and risks silently excluding the right tool.
        if analysis.needs_tools && (tools.is_empty() || analysis.confidence < 0.2) {
            tools = candidate_tools;

            if self.pipeline_config.log_token_usage {
                tracing::debug!(
                    confidence = analysis.confidence,
                    candidate_tools = ?tools.iter().map(|t| t.name).collect::<Vec<_>>(),
                    "Using candidate-pool fallback (no category match or confidence too low)"
                );
            }
        }

        Self::append_task_tool_for_auto_tracked_request(analysis, &candidate_names, &mut tools);

        if self.pipeline_config.log_token_usage {
            tracing::debug!(
                categories = ?analysis.categories,
                needs_tools = analysis.needs_tools,
                resolved_tools = ?tools.iter().map(|t| t.name).collect::<Vec<_>>(),
                "Category-based tool filtering"
            );
        }

        // Deduplicate
        tools.sort_by_key(|t| t.name);
        tools.dedup_by_key(|t| t.name);

        tools
    }

    /// If the user responds with an approval ("ok", "yes", "please proceed") and the
    /// previous assistant message proposed using a tool, ensure this turn is treated as
    /// a tool-capable follow-up.
    ///
    /// This prevents a common failure mode where the model asked for confirmation and,
    /// after the user confirms, the follow-up message contains no tool keywords, so the
    /// analyzer disables tools and the agent can't complete the action.
    fn promote_approval_to_tool_followup(
        &self,
        request: &AgentRequest,
        analysis: &mut crate::context::RequestAnalysis,
    ) {
        use crate::context::ContextCategory;

        let input = request.input.trim().to_lowercase();
        let looks_like_approval = matches!(
            input.as_str(),
            "ok" | "okay"
                | "ok."
                | "okay."
                | "yes"
                | "y"
                | "sure"
                | "please proceed"
                | "proceed"
                | "go ahead"
                | "do it"
                | "run it"
                | "continue"
        ) || input.contains("please proceed")
            || input.contains("go ahead")
            || input.contains("please do")
            || input.contains("yes, proceed");

        if !looks_like_approval {
            return;
        }

        // If tools are already enabled, nothing to do.
        if analysis.needs_tools {
            return;
        }

        // Find the most recent assistant message.
        let Some(prev_assistant) = request.history.iter().rev().find(|m| m.role == "assistant")
        else {
            return;
        };

        let prev = prev_assistant.content.to_lowercase();

        // Try to infer which tool the assistant intended to use.
        let tool_name = if prev.contains("shell tool")
            || (prev.contains("run") && prev.contains("pwd"))
            || prev.contains("`pwd`")
        {
            Some("shell")
        } else if prev.contains("git tool") || prev.contains("git status") {
            Some("git")
        } else if prev.contains("file tool") || prev.contains("read file") {
            Some("file")
        } else if prev.contains("web tool") || prev.contains("search the web") {
            Some("web")
        } else if prev.contains("code tool") || prev.contains("code stats") {
            Some("code")
        } else {
            None
        };

        let Some(tool_name) = tool_name else {
            return;
        };

        analysis.needs_tools = true;
        analysis.is_followup = true;
        analysis.confidence = analysis.confidence.max(0.85);
        analysis.suggested_tools.push(tool_name.to_string());

        // Ensure the tool category is present so tool filtering will include it.
        match tool_name {
            "shell" => {
                analysis.categories.insert(ContextCategory::Shell);
                analysis.categories.insert(ContextCategory::Tools);
            }
            "git" => {
                analysis.categories.insert(ContextCategory::Git);
                analysis.categories.insert(ContextCategory::Tools);
            }
            "file" => {
                analysis.categories.insert(ContextCategory::FileSystem);
                analysis.categories.insert(ContextCategory::Tools);
            }
            "web" => {
                analysis.categories.insert(ContextCategory::Web);
                analysis.categories.insert(ContextCategory::Tools);
            }
            "code" => {
                analysis.categories.insert(ContextCategory::Code);
                analysis.categories.insert(ContextCategory::Tools);
            }
            _ => {
                analysis.categories.insert(ContextCategory::Tools);
            }
        }
    }

    /// Enrich a resolved context with memory bank entries and enabled knowledge items.
    ///
    /// G4: Extracted into a shared helper so it is called both on the initial resolve
    /// *and* after any auto-compaction re-resolve, ensuring memory bank and knowledge
    /// survive context compaction on both the streaming and blocking code paths.
    async fn enrich_resolved_context(
        &self,
        resolved_context: &mut crate::context::ResolvedContext,
        workspace_dir: Option<&std::path::Path>,
        query: &str,
        metadata: &RequestMetadata,
    ) {
        // 3.1 Short-term working memory — session-local and loaded first.
        if let Some(short_term_sections) =
            self.load_session_working_memory(metadata.session_id.as_deref(), query, 4)
            && !short_term_sections.is_empty()
        {
            tracing::debug!(
                memory_sections = short_term_sections.len(),
                "Added session working memory to request"
            );
            resolved_context.memory_sections.extend(short_term_sections);
        }

        if let Some(workspace_dir) = workspace_dir
            && let Some(shared_coordination) = self
                .load_shared_coordination_memory(workspace_dir, metadata, 3)
                .await
            && !shared_coordination.is_empty()
        {
            tracing::debug!(
                shared_coordination_len = shared_coordination.len(),
                "Added shared coordination memory to request"
            );
            resolved_context.memory_sections.extend(shared_coordination);
        }

        // 3.2 Long-term memory bank — only available when workspace_dir is known.
        //     When experiential reflection is enabled, we also inject a small
        //     number of relevant past reflections after the regular memory-bank
        //     lookup so the model can learn from prior failures without turning
        //     every prompt into a reflection dump.
        if let Some(workspace_dir) = workspace_dir
            && let Some(memory_context) = self
                .search_and_load_memory_bank(workspace_dir, metadata, query, 3)
                .await
        {
            tracing::debug!(
                memory_context_len = memory_context.len(),
                "Added memory bank context to request"
            );
            resolved_context.memory_sections.extend(memory_context);

            if self.pipeline_config.reflection.enabled
                && let Some(reflection_sections) = self
                    .load_relevant_reflections(workspace_dir, metadata, query)
                    .await
                && !reflection_sections.is_empty()
            {
                tracing::debug!(
                    reflection_context_len = reflection_sections.len(),
                    "Added past reflections to request"
                );
                resolved_context.memory_sections.extend(reflection_sections);
            }
        }

        // 3.3 Knowledge items — only available when the pipeline was wired with
        //     `with_knowledge()` *and* the session has items enabled.
        if let Some(knowledge_context) = self.load_enabled_knowledge(metadata.session_id.as_deref())
        {
            tracing::debug!(
                knowledge_context_len = knowledge_context.len(),
                "Added enabled knowledge to request"
            );
            resolved_context.knowledge.push(knowledge_context);
        }
    }
}

fn merge_token_usage(
    usage: &mut Option<crate::llm_provider::TokenUsage>,
    additional: &crate::llm_provider::TokenUsage,
) {
    if let Some(existing) = usage.as_mut() {
        existing.input_tokens += additional.input_tokens;
        existing.output_tokens += additional.output_tokens;
        existing.total_tokens += additional.total_tokens;
        existing.estimated_cost_usd =
            match (existing.estimated_cost_usd, additional.estimated_cost_usd) {
                (Some(lhs), Some(rhs)) => Some(lhs + rhs),
                (Some(lhs), None) => Some(lhs),
                (None, Some(rhs)) => Some(rhs),
                (None, None) => None,
            };
        if existing.model.is_none() {
            existing.model = additional.model.clone();
        }
        if existing.provider.is_none() {
            existing.provider = additional.provider.clone();
        }
    } else {
        *usage = Some(additional.clone());
    }
}

#[cfg(test)]
mod tests;
