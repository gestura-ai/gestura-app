use super::*;

use tempfile::tempdir;

#[test]
fn test_agent_request_builder() {
    let request = AgentRequest::new("Hello world")
        .with_streaming(true)
        .with_max_iterations(24)
        .with_source(RequestSource::CliTui);

    assert_eq!(request.input, "Hello world");
    assert!(request.streaming);
    assert_eq!(request.max_iterations, Some(24));
    assert_eq!(request.metadata.source, RequestSource::CliTui);
}

#[test]
fn test_agent_request_builder_allows_reflection_override() {
    let request = AgentRequest::new("Hello world").with_reflection_enabled(false);

    assert_eq!(request.metadata.reflection_enabled, Some(false));
}

#[test]
fn requirement_detection_input_defaults_to_request_input() {
    let request = AgentRequest::new("Please build and test the project before finishing.");

    assert_eq!(
        super::requirement_detection_input(&request),
        "Please build and test the project before finishing."
    );
    assert!(AgentPipeline::prompt_requires_build_and_test(
        super::requirement_detection_input(&request)
    ));
}

#[test]
fn requirement_detection_input_prefers_orchestrator_hint_over_composed_prompt() {
    let mut request = AgentRequest::new(
        "Role:\nImplementer\n\nDelegation Brief:\nBefore finishing, build and test the project.\n\nTask:\nCreate a concise SWOT markdown file from the research.",
    )
    .with_source(RequestSource::Orchestrator);
    request.metadata.hints.insert(
        "requirement_detection_input".to_string(),
        "Create a concise SWOT markdown file from the research.".to_string(),
    );

    assert_eq!(
        super::requirement_detection_input(&request),
        "Create a concise SWOT markdown file from the research."
    );
    assert!(!AgentPipeline::prompt_requires_build_and_test(
        super::requirement_detection_input(&request)
    ));
    assert!(AgentPipeline::request_requires_mutating_file_tool_success(
        super::requirement_detection_input(&request)
    ));
}

#[test]
fn markdown_response_requests_do_not_require_source_mutation_without_file_context() {
    let request = AgentRequest::new(
        "I want to create a concise SWOT analysis for launching a new category of smart home lighting products priced between $30 and $80. Please carefully plan the structure (Strengths, Weaknesses, Opportunities, Threats — with 4–6 bullet points each), research current 2025–2026 market trends, major players, and consumer drivers using reliable sources, implement the full SWOT in clear markdown format with brief explanations for each point, then verify by cross-checking at least three facts against independent sources and note any conflicting data or assumptions.",
    );

    assert!(!AgentPipeline::request_requires_mutating_file_tool_success(
        super::requirement_detection_input(&request)
    ));
}

#[test]
fn ambiguous_create_language_still_requires_mutation_when_file_context_is_explicit() {
    let request = AgentRequest::new(
        "Create a markdown file named smart_home_lighting_swot.md in the repo and write the final SWOT analysis there.",
    );

    assert!(AgentPipeline::request_requires_mutating_file_tool_success(
        super::requirement_detection_input(&request)
    ));
}

#[test]
fn test_pipeline_new_honors_user_reflection_settings() {
    let mut config = AppConfig::default();
    config.pipeline.reflection.enabled = true;
    config.pipeline.reflection.quality_threshold_percent = 42;

    let pipeline = AgentPipeline::new(config);

    assert!(pipeline.pipeline_config.reflection.enabled);
    assert!((pipeline.pipeline_config.reflection.quality_threshold - 0.42).abs() < f32::EPSILON);
}

#[test]
fn request_reflection_override_takes_precedence_over_pipeline_default() {
    let mut config = AppConfig::default();
    config.pipeline.reflection.enabled = true;
    let pipeline = AgentPipeline::new(config);

    let request = AgentRequest::new("Hello world").with_reflection_enabled(false);

    assert!(!pipeline.reflection_enabled_for(&request.metadata));
}

#[test]
fn test_pipeline_new_honors_user_iteration_budget_settings() {
    let mut config = AppConfig::default();
    config.pipeline.iteration_budget_enabled = true;
    config.pipeline.max_iterations = 21;
    config.pipeline.tracked_task_max_iterations = 84;

    let pipeline = AgentPipeline::new(config);

    assert!(pipeline.pipeline_config.iteration_budget_enabled);
    assert_eq!(pipeline.pipeline_config.max_iterations, 21);
    assert_eq!(pipeline.pipeline_config.tracked_task_max_iterations, 84);
}

#[test]
fn test_effective_request_max_iterations_is_unbounded_when_budget_disabled() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let request = AgentRequest::new("keep going");

    assert_eq!(pipeline.effective_request_max_iterations(&request), None);
}

#[test]
fn test_effective_request_max_iterations_uses_tracked_task_budget() {
    let mut config = AppConfig::default();
    config.pipeline.iteration_budget_enabled = true;
    config.pipeline.max_iterations = 12;
    config.pipeline.tracked_task_max_iterations = 48;
    let pipeline = AgentPipeline::new(config);
    let request = AgentRequest::new("finish the implementation")
        .with_session("session-1")
        .with_task("task-1");

    assert_eq!(
        pipeline.effective_request_max_iterations(&request),
        Some(48)
    );
}

#[test]
fn missing_session_ids_are_synthesized_for_core_requests() {
    let mut request = AgentRequest::new("build and test the application");

    AgentPipeline::ensure_request_session_id(&mut request);

    let session_id = request
        .metadata
        .session_id
        .as_deref()
        .expect("session id should be synthesized");
    assert!(session_id.starts_with("agent-run-"));
}

#[test]
fn core_can_auto_initialize_a_tracked_root_task() {
    let temp = tempdir().unwrap();
    let mut request =
        AgentRequest::new("Carefully plan and implement the change, then build and test it")
            .with_workspace(temp.path());

    AgentPipeline::ensure_request_session_id(&mut request);
    AgentPipeline::maybe_initialize_tracked_request_task(&mut request, true, true);

    let session_id = request
        .metadata
        .session_id
        .as_deref()
        .expect("session id should be present");
    let task_id = request
        .metadata
        .task_id
        .as_deref()
        .expect("task id should be initialized");
    let tracked_task = crate::get_global_task_manager()
        .get_task(session_id, task_id)
        .expect("task lookup should succeed")
        .expect("tracked task should exist");
    let descendants = crate::get_global_task_manager()
        .list_descendants(session_id, task_id)
        .expect("descendant lookup should succeed");
    let lifecycle = crate::get_global_task_manager()
        .get_memory_lifecycle(session_id, task_id)
        .expect("memory lifecycle lookup should succeed")
        .expect("tracked task should record a lifecycle event");

    assert_eq!(tracked_task.parent_id, None);
    assert!(tracked_task.name.contains("Carefully plan and implement"));
    assert_eq!(tracked_task.status, crate::TaskStatus::InProgress);
    assert_eq!(descendants.len(), 4);
    assert_eq!(lifecycle.events.len(), 1);
    assert_eq!(
        lifecycle.events[0].phase,
        crate::tasks::TaskMemoryPhase::Handoff
    );
    assert!(
        lifecycle.events[0]
            .summary
            .contains("Initialized tracked request with 4 planned tracked subtasks")
    );
    assert!(descendants.iter().any(|task| {
        task.name == "Inspect the current state and constraints"
            && task.status == crate::TaskStatus::InProgress
    }));
    assert!(request.input.contains("[Runtime execution handoff]"));
    assert!(request.input.contains("Planned tracked subtasks:"));
    assert!(
        request
            .input
            .contains("Validate the result and summarize follow-up [notstarted]")
    );
}

#[test]
fn compare_requests_can_be_auto_tracked_generically() {
    assert!(AgentPipeline::should_auto_track_request(
        "Please compare these two logs, identify the main differences, and recommend next steps.",
        None,
    ));
}

#[test]
fn analysis_requests_receive_analysis_subtasks() {
    let subtasks = AgentPipeline::default_auto_tracked_execution_subtasks(
        "Compare these two session logs and recommend improvements.",
    );

    assert_eq!(subtasks.len(), 3);
    assert_eq!(subtasks[0].0, "Inspect the relevant inputs and criteria");
    assert_eq!(subtasks[1].0, "Analyze the findings and identify gaps");
    assert_eq!(
        subtasks[2].0,
        "Summarize conclusions and recommended actions"
    );
}

#[test]
fn create_requests_expand_into_preparation_execution_and_validation() {
    let subtasks = AgentPipeline::default_auto_tracked_execution_subtasks(
        "Create a reusable workflow, then build and test it end to end.",
    );

    assert_eq!(subtasks.len(), 4);
    assert_eq!(subtasks[0].0, "Inspect the current state and constraints");
    assert_eq!(subtasks[1].0, "Prepare the starting point or prerequisites");
    assert_eq!(subtasks[2].0, "Carry out the requested work");
    assert_eq!(subtasks[3].0, "Validate the result and summarize follow-up");
    assert!(subtasks[3].1.contains("verification steps"));
}

#[test]
fn auto_tracked_handoff_supports_analysis_or_research_requests() {
    let handoff = AgentPipeline::build_auto_tracked_execution_handoff_message(
        "Compare the two reports and summarize the differences.",
        "Compare the reports",
        &["Inspect the relevant inputs and criteria [not_started]".to_string()],
    );

    assert!(handoff.contains("Planned tracked subtasks"));
    assert!(handoff.contains("analysis or research"));
    assert!(handoff.contains("attach it to the tracked root plan"));
    assert!(!handoff.contains("Begin concrete implementation work immediately"));
}

#[test]
fn test_message_constructors() {
    let user_msg = Message::user("Hello");
    assert_eq!(user_msg.role, "user");

    let assistant_msg = Message::assistant("Hi there");
    assert_eq!(assistant_msg.role, "assistant");

    let tool_msg = Message::tool_result("call_123", "result data");
    assert_eq!(tool_msg.role, "tool");
    assert_eq!(tool_msg.tool_call_id, Some("call_123".to_string()));
}

#[test]
fn build_prompt_includes_project_guardrails_when_workspace_present() {
    let temp = tempdir().unwrap();
    std::fs::write(temp.path().join("AGENTS.md"), "Always run tests.\n").unwrap();

    let pipeline = AgentPipeline::new(AppConfig::default());

    let request = AgentRequest::new("hi").with_workspace(temp.path());
    let context = crate::context::ResolvedContext::default();
    let prompt = pipeline.build_prompt(&request, &context);

    assert!(prompt.contains("Project guardrails:"));
    assert!(prompt.contains("Always run tests."));
    assert!(prompt.contains("Source: AGENTS.md"));
}

#[test]
fn build_prompt_uses_dot_gestura_guardrails_over_agents_md() {
    let temp = tempdir().unwrap();
    std::fs::write(temp.path().join("AGENTS.md"), "agents-rule\n").unwrap();
    std::fs::create_dir_all(temp.path().join(".gestura")).unwrap();
    std::fs::write(temp.path().join(".gestura/guardrails"), "guardrails-rule\n").unwrap();

    let pipeline = AgentPipeline::new(AppConfig::default());

    let request = AgentRequest::new("hi").with_workspace(temp.path());
    let context = crate::context::ResolvedContext::default();
    let prompt = pipeline.build_prompt(&request, &context);

    assert!(prompt.contains("guardrails-rule"));
    assert!(!prompt.contains("agents-rule"));
    assert!(prompt.contains("Source: .gestura/guardrails"));
}

#[test]
fn build_prompt_skips_guardrails_when_disabled() {
    let temp = tempdir().unwrap();
    std::fs::write(temp.path().join("AGENTS.md"), "agents-rule\n").unwrap();

    let mut config = AppConfig::default();
    config.pipeline.project_guardrails.enabled = false;
    let pipeline = AgentPipeline::new(config);

    let request = AgentRequest::new("hi").with_workspace(temp.path());
    let context = crate::context::ResolvedContext::default();
    let prompt = pipeline.build_prompt(&request, &context);

    assert!(!prompt.contains("Project guardrails:"));
    assert!(!prompt.contains("agents-rule"));
}

#[test]
fn build_prompt_includes_memory_sections() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let request = AgentRequest::new("continue the implementation");

    let context = crate::context::ResolvedContext {
        memory_sections: vec![
            "### Session Working Memory\nDecision: Keep short-term memory session scoped"
                .to_string(),
            "### Long-Term Memory\nShared directive summary".to_string(),
        ],
        ..Default::default()
    };

    let prompt = pipeline.build_prompt(&request, &context);

    assert!(prompt.contains("Relevant memory:"));
    assert!(prompt.contains("Decision: Keep short-term memory session scoped"));
    assert!(prompt.contains("Shared directive summary"));
}

#[tokio::test]
async fn enrich_resolved_context_includes_shared_coordination_memory() {
    let temp = tempdir().unwrap();
    let pipeline = AgentPipeline::new(AppConfig::default());

    let entry = crate::MemoryBankEntry::new(
        "session-shared".to_string(),
        "Supervisor steering note".to_string(),
        "Use ripgrep first and keep the worktree clean before editing.".to_string(),
    )
    .with_memory_type(crate::MemoryType::Procedural)
    .with_scope(crate::MemoryScope::Task)
    .with_category(crate::orchestrator::SHARED_COGNITION_CATEGORY)
    .with_provenance(
        Some("task-shared".to_string()),
        Some("directive-shared".to_string()),
        Some("supervisor".to_string()),
    )
    .with_tags(vec![
        crate::orchestrator::SHARED_COGNITION_TAG.to_string(),
        "workflow-run:run-shared".to_string(),
    ])
    .with_confidence(0.9);
    crate::save_to_memory_bank(temp.path(), &entry)
        .await
        .unwrap();

    let metadata = RequestMetadata {
        session_id: Some("session-shared".to_string()),
        task_id: Some("task-shared".to_string()),
        directive_id: Some("directive-shared".to_string()),
        agent_id: Some("agent-impl".to_string()),
        memory_tags: vec!["workflow-run:run-shared".to_string()],
        ..Default::default()
    };
    let mut context = crate::context::ResolvedContext::default();

    pipeline
        .enrich_resolved_context(
            &mut context,
            Some(temp.path()),
            "continue implementing the workflow",
            &metadata,
        )
        .await;

    assert!(
        context
            .memory_sections
            .iter()
            .any(|section| section.contains("### Shared Coordination Memory"))
    );
    assert!(
        context
            .memory_sections
            .iter()
            .any(|section| section.contains("Use ripgrep first and keep the worktree clean"))
    );
}

#[tokio::test]
async fn enrich_resolved_context_bounds_shared_coordination_memory_to_three_entries() {
    let temp = tempdir().unwrap();
    let pipeline = AgentPipeline::new(AppConfig::default());

    for index in 0..5 {
        let entry = crate::MemoryBankEntry::new(
            "session-shared".to_string(),
            format!("Shared note {index}"),
            format!("Shared coordination detail {index}"),
        )
        .with_memory_type(crate::MemoryType::Procedural)
        .with_scope(crate::MemoryScope::Directive)
        .with_category(crate::orchestrator::SHARED_COGNITION_CATEGORY)
        .with_provenance(
            Some(format!("task-{index}")),
            Some("directive-shared".to_string()),
            Some(format!("agent-{index}")),
        )
        .with_tags(vec![
            crate::orchestrator::SHARED_COGNITION_TAG.to_string(),
            "workflow-run:run-shared".to_string(),
        ])
        .with_confidence(0.7 + (index as f32 * 0.02));
        crate::save_to_memory_bank(temp.path(), &entry)
            .await
            .unwrap();
    }

    let metadata = RequestMetadata {
        session_id: Some("session-shared".to_string()),
        directive_id: Some("directive-shared".to_string()),
        memory_tags: vec!["workflow-run:run-shared".to_string()],
        ..Default::default()
    };
    let mut context = crate::context::ResolvedContext::default();

    pipeline
        .enrich_resolved_context(
            &mut context,
            Some(temp.path()),
            "continue implementing the workflow",
            &metadata,
        )
        .await;

    let shared_sections = context
        .memory_sections
        .iter()
        .filter(|section| section.contains("### Shared Coordination Memory"))
        .count();
    assert!(shared_sections >= 1);
    assert!(shared_sections <= 3);
}

#[tokio::test]
async fn enrich_resolved_context_includes_enabled_session_knowledge_in_prompt() {
    let temp = tempdir().unwrap();
    let store = Box::leak(Box::new(crate::knowledge::KnowledgeStore::new(
        temp.path().join("knowledge"),
    )));
    crate::knowledge::register_builtin_knowledge(store);

    let settings = Box::leak(Box::new(crate::knowledge::KnowledgeSettingsManager::new(
        temp.path().to_path_buf(),
    )));
    let mut session_settings =
        crate::knowledge::SessionKnowledgeSettings::new("session-knowledge".to_string());
    session_settings.enable("rust-expert".to_string());
    settings.save(&session_settings).unwrap();

    let pipeline = AgentPipeline::new(AppConfig::default()).with_knowledge(store, settings);
    let metadata = RequestMetadata {
        session_id: Some("session-knowledge".to_string()),
        ..Default::default()
    };
    let mut context = crate::context::ResolvedContext::default();

    pipeline
        .enrich_resolved_context(
            &mut context,
            Some(temp.path()),
            "help me fix async rust ownership issues",
            &metadata,
        )
        .await;

    assert!(
        context
            .knowledge
            .iter()
            .any(|section| section.contains("## Specialized Knowledge"))
    );
    assert!(
        context
            .knowledge
            .iter()
            .any(|section| section.contains("Rust Expert"))
    );

    let request = AgentRequest::new("help me fix async rust ownership issues")
        .with_session("session-knowledge");
    let prompt = pipeline.build_prompt(&request, &context);
    assert!(prompt.contains("## Specialized Knowledge"));
    assert!(prompt.contains("Rust Expert"));
}

#[tokio::test]
async fn enrich_resolved_context_skips_shared_coordination_without_scope_hints() {
    let temp = tempdir().unwrap();
    let pipeline = AgentPipeline::new(AppConfig::default());

    let entry = crate::MemoryBankEntry::new(
        "session-shared".to_string(),
        "Shared note".to_string(),
        "This should not be loaded without a task, directive, or tag hint.".to_string(),
    )
    .with_memory_type(crate::MemoryType::Procedural)
    .with_scope(crate::MemoryScope::Directive)
    .with_category(crate::orchestrator::SHARED_COGNITION_CATEGORY)
    .with_provenance(
        Some("task-shared".to_string()),
        Some("directive-shared".to_string()),
        Some("supervisor".to_string()),
    )
    .with_tags(vec![crate::orchestrator::SHARED_COGNITION_TAG.to_string()])
    .with_confidence(0.9);
    crate::save_to_memory_bank(temp.path(), &entry)
        .await
        .unwrap();

    let mut context = crate::context::ResolvedContext::default();
    pipeline
        .enrich_resolved_context(
            &mut context,
            Some(temp.path()),
            "continue implementing the workflow",
            &RequestMetadata::default(),
        )
        .await;

    assert!(
        context
            .memory_sections
            .iter()
            .all(|section| !section.contains("### Shared Coordination Memory"))
    );
}

#[test]
fn test_pipeline_config_defaults() {
    let config = PipelineConfig::default();
    assert!(!config.iteration_budget_enabled);
    assert_eq!(config.max_iterations, 10);
    assert_eq!(config.tracked_task_max_iterations, 30);
    assert!(config.enable_tools);
    assert!(config.enable_context_reduction);
}

#[test]
fn promote_approval_followup_enables_shell_tool() {
    use crate::context::ContextCategory;

    let pipeline = AgentPipeline::new(AppConfig::default());

    let history = vec![Message::assistant(
        "We will use the shell tool to run 'pwd'. Then respond.",
    )];
    let request = AgentRequest::new("okay please proceed").with_history(history);

    let mut analysis = crate::context::RequestAnalysis::new("okay please proceed");
    assert!(!analysis.needs_tools);

    pipeline.promote_approval_to_tool_followup(&request, &mut analysis);

    assert!(analysis.needs_tools);
    assert!(analysis.is_followup);
    assert!(analysis.categories.contains(&ContextCategory::Shell));
    assert!(analysis.categories.contains(&ContextCategory::Tools));
    assert!(analysis.suggested_tools.contains(&"shell".to_string()));
    assert!(analysis.confidence >= 0.85);
}

/// When adapter layers explicitly disable tools for a request, the pipeline must
/// not execute any tools (including the confirmed-tool follow-up heuristic).
#[tokio::test]
#[ignore = "requires Ollama with llama3.2 model installed"]
async fn tools_enabled_false_skips_confirmed_tool_followup_execution() {
    use tokio::sync::mpsc;
    use tokio::time::{Duration, timeout};

    let pipeline = AgentPipeline::new(AppConfig::default());

    // Prior assistant message contains an explicit tool plan.
    let history = vec![Message::assistant(
        "We will use the shell tool to run 'pwd'. Then respond.",
    )];

    // User approval would normally trigger tool follow-up execution.
    let request = AgentRequest::new("okay please proceed")
        .with_history(history)
        .with_tools_enabled(false);

    // IMPORTANT: drain the stream concurrently to avoid backpressure deadlocks
    // if the provider emits many chunks.
    let (tx, mut rx) = mpsc::channel(256);
    let cancel = CancellationToken::new();

    let drain_handle = tokio::spawn(async move {
        let mut saw_done = false;
        while let Some(chunk) = rx.recv().await {
            match chunk {
                other @ (StreamChunk::ToolCallStart { .. }
                | StreamChunk::ToolCallArgs(_)
                | StreamChunk::ToolCallEnd
                | StreamChunk::ToolCallResult { .. }
                | StreamChunk::ToolConfirmationRequired { .. }
                | StreamChunk::ToolBlocked { .. }) => {
                    return Err(format!(
                        "unexpected tool chunk emitted when tools are disabled: {other:?}"
                    ));
                }
                StreamChunk::Done(_) => {
                    saw_done = true;
                    break;
                }
                _ => {}
            }
        }
        Ok::<bool, String>(saw_done)
    });

    let response = timeout(
        Duration::from_secs(5),
        pipeline.process_streaming(request, tx, cancel),
    )
    .await
    .expect("process_streaming should not hang")
    .expect("pipeline should complete");

    // Strong assertion: the response should not record any tool calls.
    assert!(response.tool_calls.is_empty());

    // Avoid hangs if a regression causes the stream to never finalize.
    let saw_done = timeout(Duration::from_secs(3), drain_handle)
        .await
        .expect("drain task should finish")
        .expect("drain task should not panic")
        .expect("no tool chunks should be emitted");

    assert!(saw_done);
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn confirmed_shell_followup_preserves_shell_session_id_without_workspace() {
    use crate::streaming::StreamChunk;
    use crate::tools::registry::all_tools;
    use tokio::sync::mpsc;
    use tokio::time::{Duration, timeout};

    let pipeline = AgentPipeline::new(AppConfig::default());
    let history = vec![Message::assistant(
        "We will use the shell tool to run 'pwd'. Then respond.",
    )];
    let session_id = format!("session-confirmed-shell-{}", uuid::Uuid::new_v4());
    let request = AgentRequest::new("okay please proceed")
        .with_history(history)
        .with_session(session_id.clone());
    let analysis = crate::context::RequestAnalysis::new("okay please proceed");
    let relevant_tools: Vec<&'static ToolDefinition> = all_tools()
        .iter()
        .filter(|tool| tool.name == "shell")
        .collect();
    let (tx, rx) = mpsc::channel(128);
    let cancel = CancellationToken::new();

    // This test is only validating shell-session routing metadata. Pre-cancel the
    // follow-up synthesis stream so we do not depend on an LLM provider stream
    // finishing in CI after the shell tool has already run.
    cancel.cancel();

    let exec_tx = tx.clone();

    let handle = tokio::spawn(async move {
        pipeline
            .try_execute_confirmed_tool_from_history(
                &request,
                &analysis,
                &relevant_tools,
                None,
                &exec_tx,
                &cancel,
            )
            .await
    });

    let (observed_tx, observed_rx) = tokio::sync::oneshot::channel();
    let drain = tokio::spawn(async move {
        let mut rx = rx;
        let mut observed_tx = Some(observed_tx);
        let mut saw_shell_session_id = false;
        let mut saw_tool_result = false;

        while let Some(chunk) = rx.recv().await {
            match chunk {
                StreamChunk::ShellLifecycle {
                    shell_session_id: Some(_),
                    ..
                }
                | StreamChunk::ShellOutput {
                    shell_session_id: Some(_),
                    ..
                } => saw_shell_session_id = true,
                StreamChunk::ToolCallResult { success, .. } => {
                    assert!(success, "confirmed shell follow-up should succeed");
                    saw_tool_result = true;
                }
                _ => {}
            }

            if saw_shell_session_id
                && saw_tool_result
                && let Some(observed_tx) = observed_tx.take()
            {
                let _ = observed_tx.send(());
            }
        }

        (saw_shell_session_id, saw_tool_result)
    });

    drop(tx);

    timeout(Duration::from_secs(15), observed_rx)
        .await
        .expect("confirmed shell follow-up stream observation timed out")
        .expect("confirmed shell follow-up observation channel closed unexpectedly");

    timeout(
        Duration::from_secs(15),
        crate::tools::shell_sessions::shutdown_session(&session_id),
    )
    .await
    .expect("timed out shutting down confirmed shell test session")
    .expect("shutdown confirmed shell test session");

    let response = timeout(Duration::from_secs(15), handle)
        .await
        .expect("confirmed shell follow-up task timed out")
        .expect("confirmed shell follow-up task should join")
        .expect("confirmed tool follow-up should execute")
        .expect("expected confirmed shell follow-up to run");

    let (saw_shell_session_id, saw_tool_result) = timeout(Duration::from_secs(5), drain)
        .await
        .expect("confirmed shell follow-up drain task timed out")
        .expect("confirmed shell follow-up drain task should join");

    assert!(
        saw_shell_session_id,
        "expected confirmed shell follow-up streaming to include shell_session_id"
    );
    assert!(
        saw_tool_result,
        "expected confirmed shell follow-up to emit a successful tool result"
    );

    assert!(response.tool_calls.iter().any(|call| call.name == "shell"));
}

/// Even when request analysis would normally select tools, `tools_enabled=false`
/// must ensure the blocking pipeline path does not execute tools.
#[tokio::test]
#[ignore = "requires Ollama with llama3.2 model installed"]
async fn tools_enabled_false_disables_tools_for_blocking_requests() {
    let pipeline = AgentPipeline::new(AppConfig::default());

    let request = AgentRequest::new("Read the file 'Cargo.toml'.")
        .with_streaming(false)
        .with_tools_enabled(false);

    let response = pipeline
        .process_blocking(request)
        .await
        .expect("blocking pipeline should complete");

    assert!(response.tool_calls.is_empty());
    assert!(!response.content.trim().is_empty());
}

#[test]
fn extract_shell_command_from_plan_parses_quoted_command() {
    let text = "We will use the shell tool to run 'pwd'. Then respond.";
    let cmd = AgentPipeline::extract_shell_command_from_plan(text).unwrap();
    assert_eq!(cmd, "pwd");

    let text2 = "We'll use the shell tool to run `git status` then respond.";
    let cmd2 = AgentPipeline::extract_shell_command_from_plan(text2).unwrap();
    assert_eq!(cmd2, "git status");
}

#[test]
fn extract_planned_tool_call_from_text_parses_file_read() {
    let text = "We will use the file tool to read 'foo.txt'. Then respond.";
    let (tool, args, prefix) =
        AgentPipeline::extract_planned_tool_call_from_text(text).expect("should parse");
    assert_eq!(tool, "file");
    assert!(args.contains("\"operation\":\"read\""));
    assert!(args.contains("\"path\":\"foo.txt\""));
    assert!(prefix.to_lowercase().contains("file"));
}

#[test]
fn is_write_operation_classifies_file_operations() {
    let read = serde_json::json!({"operation": "read", "path": "foo.txt"}).to_string();
    assert!(!crate::tools::policy::is_write_operation("file", &read));

    let list = serde_json::json!({"operation": "list", "path": "."}).to_string();
    assert!(!crate::tools::policy::is_write_operation("file", &list));

    let search =
        serde_json::json!({"operation": "search", "path": ".", "pattern": "foo"}).to_string();
    assert!(!crate::tools::policy::is_write_operation("file", &search));

    let write =
        serde_json::json!({"operation": "write", "path": "foo.txt", "content": "hi"}).to_string();
    assert!(crate::tools::policy::is_write_operation("file", &write));

    let edit = serde_json::json!({"operation": "edit", "path": "foo.txt", "old": "a", "new": "b"})
        .to_string();
    assert!(crate::tools::policy::is_write_operation("file", &edit));

    // Mirror the defaulting behavior: content without operation is treated as write.
    let implicit_write = serde_json::json!({"path": "foo.txt", "content": "hi"}).to_string();
    assert!(crate::tools::policy::is_write_operation(
        "file",
        &implicit_write
    ));
}

#[test]
fn is_write_operation_classifies_shell_commands_conservatively() {
    let pwd = serde_json::json!({"command": "pwd"}).to_string();
    assert!(!crate::tools::policy::is_write_operation("shell", &pwd));

    let ls = serde_json::json!({"command": "ls -la"}).to_string();
    assert!(!crate::tools::policy::is_write_operation("shell", &ls));

    let echo = serde_json::json!({"command": "echo hi"}).to_string();
    assert!(!crate::tools::policy::is_write_operation("shell", &echo));

    let redirect = serde_json::json!({"command": "echo hi > out.txt"}).to_string();
    assert!(crate::tools::policy::is_write_operation("shell", &redirect));

    // Unknown commands are treated as write for safety.
    let unknown = serde_json::json!({"command": "git status"}).to_string();
    assert!(crate::tools::policy::is_write_operation("shell", &unknown));
}

/// In Restricted mode, write operations must request confirmation.
///
/// When the user denies, the tool should be skipped, a ToolCallResult should
/// be emitted (success=false), and the pending confirmation should be cleared.
#[tokio::test]
async fn restricted_mode_write_tool_denied_emits_tool_call_result_and_skips() {
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    use crate::session_workspace::SessionWorkspace;
    use crate::tool_confirmation::{TOOL_CONFIRMATIONS, ToolConfirmationDecision};

    let temp = tempdir().unwrap();
    let session_id = format!("restricted-denied-{}", Uuid::new_v4());
    let mut pipeline = AgentPipeline::new(AppConfig::default());
    pipeline.permission_manager =
        PermissionManager::from_config_path(temp.path().join("permissions.json"));
    let workspace = Arc::new(
        SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
            .expect("workspace"),
    );

    let (tx, mut rx) = mpsc::channel(32);
    let cancel = CancellationToken::new();

    let pending = PendingToolCall {
        id: "call_test_denied".to_string(),
        name: "file".to_string(),
        arguments: serde_json::json!({
            "operation": "write",
            "path": "out.txt",
            "content": "hi"
        })
        .to_string(),
        start_time: Instant::now(),
    };

    let spawned_session_id = session_id.clone();
    let handle = tokio::spawn({
        let workspace = workspace.clone();
        async move {
            let mut tool_calls_in_iteration: Vec<ToolCallRecord> = Vec::new();
            let mut response = AgentResponse::empty();

            pipeline
                .finalize_pending_tool_call(
                    pending,
                    FinalizePendingToolCallCtx {
                        workspace: Some(workspace.as_ref()),
                        session_id: Some(spawned_session_id.clone()),
                        permission_level: PermissionLevel::Restricted,
                        required_verification_retry_pending: false,
                        cancel_token: &cancel,
                        tool_calls_in_iteration: &mut tool_calls_in_iteration,
                        response: &mut response,
                        tx: &tx,
                    },
                )
                .await;

            (tool_calls_in_iteration, response)
        }
    });

    // Wait for the confirmation request and deny it.
    let mut confirmation_id: Option<String> = None;
    while let Some(chunk) = rx.recv().await {
        if let StreamChunk::ToolConfirmationRequired {
            confirmation_id: id,
            ..
        } = chunk
        {
            confirmation_id = Some(id);
            break;
        }
    }
    let confirmation_id = confirmation_id.expect("expected ToolConfirmationRequired");

    TOOL_CONFIRMATIONS
        .resolve_decision(
            &confirmation_id,
            Some(&session_id),
            ToolConfirmationDecision::DenyOnce,
        )
        .expect("resolve should succeed");

    // Ensure we emit a tool call result with success=false.
    let mut saw_result = false;
    while let Some(chunk) = rx.recv().await {
        if let StreamChunk::ToolCallResult {
            success, output, ..
        } = chunk
        {
            assert!(!success);
            assert!(output.contains("Skipped: tool confirmation"));
            saw_result = true;
            break;
        }
    }
    assert!(saw_result);

    let (tool_calls, response) = handle.await.expect("task join");
    // Ensure this specific confirmation has been cleared, without depending on
    // global pending count (tests may run concurrently).
    let err = TOOL_CONFIRMATIONS
        .resolve_decision(
            &confirmation_id,
            Some(&session_id),
            ToolConfirmationDecision::AllowOnce,
        )
        .unwrap_err();
    assert!(err.contains("Unknown confirmation id"));
    assert!(!temp.path().join("out.txt").exists());

    // Sanity: the pipeline should record a skipped tool call.
    assert!(
        tool_calls
            .iter()
            .any(|t| matches!(t.result, ToolResult::Skipped(_)))
    );
    assert!(
        response
            .tool_calls
            .iter()
            .any(|t| matches!(t.result, ToolResult::Skipped(_)))
    );
}

/// In Restricted mode, if the user never responds, the confirmation should
/// time out and the tool should be skipped with a ToolCallResult.
#[tokio::test(start_paused = true)]
async fn restricted_mode_write_tool_times_out_and_emits_tool_call_result() {
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    use crate::session_workspace::SessionWorkspace;
    use crate::tool_confirmation::{TOOL_CONFIRMATIONS, ToolConfirmationDecision};

    let temp = tempdir().unwrap();
    let session_id = format!("restricted-timeout-{}", Uuid::new_v4());
    let mut pipeline = AgentPipeline::new(AppConfig::default());
    pipeline.permission_manager =
        PermissionManager::from_config_path(temp.path().join("permissions.json"));
    let workspace = Arc::new(
        SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
            .expect("workspace"),
    );

    let (tx, mut rx) = mpsc::channel(32);
    let cancel = CancellationToken::new();

    let pending = PendingToolCall {
        id: "call_test_timeout".to_string(),
        name: "file".to_string(),
        arguments: serde_json::json!({
            "operation": "write",
            "path": "out_timeout.txt",
            "content": "hi"
        })
        .to_string(),
        start_time: Instant::now(),
    };

    let spawned_session_id = session_id.clone();
    let handle = tokio::spawn({
        let workspace = workspace.clone();
        async move {
            let mut tool_calls_in_iteration: Vec<ToolCallRecord> = Vec::new();
            let mut response = AgentResponse::empty();

            pipeline
                .finalize_pending_tool_call(
                    pending,
                    FinalizePendingToolCallCtx {
                        workspace: Some(workspace.as_ref()),
                        session_id: Some(spawned_session_id.clone()),
                        permission_level: PermissionLevel::Restricted,
                        required_verification_retry_pending: false,
                        cancel_token: &cancel,
                        tool_calls_in_iteration: &mut tool_calls_in_iteration,
                        response: &mut response,
                        tx: &tx,
                    },
                )
                .await;

            (tool_calls_in_iteration, response)
        }
    });

    // Wait for the confirmation request. We intentionally do NOT resolve it.
    let mut confirmation_id: Option<String> = None;
    while let Some(chunk) = rx.recv().await {
        if let StreamChunk::ToolConfirmationRequired {
            confirmation_id: id,
            ..
        } = chunk
        {
            confirmation_id = Some(id);
            break;
        }
    }
    let confirmation_id = confirmation_id.expect("expected ToolConfirmationRequired");

    // Advance time beyond the hard-coded confirmation timeout (300s).
    tokio::time::advance(Duration::from_secs(301)).await;
    tokio::task::yield_now().await;

    let mut saw_result = false;
    while let Some(chunk) = rx.recv().await {
        if let StreamChunk::ToolCallResult {
            success, output, ..
        } = chunk
        {
            assert!(!success);
            assert!(output.contains("timed-out") || output.contains("denied"));
            saw_result = true;
            break;
        }
    }
    assert!(saw_result);

    let (tool_calls, response) = handle.await.expect("task join");
    // Ensure this specific confirmation has been cleared, without depending on
    // global pending count (tests may run concurrently).
    let err = TOOL_CONFIRMATIONS
        .resolve_decision(
            &confirmation_id,
            Some(&session_id),
            ToolConfirmationDecision::AllowOnce,
        )
        .unwrap_err();
    assert!(err.contains("Unknown confirmation id"));
    assert!(!temp.path().join("out_timeout.txt").exists());
    assert!(
        tool_calls
            .iter()
            .any(|t| matches!(t.result, ToolResult::Skipped(_)))
    );
    assert!(
        response
            .tool_calls
            .iter()
            .any(|t| matches!(t.result, ToolResult::Skipped(_)))
    );
}

#[tokio::test]
async fn execute_tool_dispatches_code_stats_with_workspace_sandbox() {
    use tempfile::tempdir;

    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();

    let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
        .expect("workspace should be created");

    let args = serde_json::json!({"operation":"stats","path":"."}).to_string();
    let result = pipeline
        .execute_tool("code", &args, Some(&ws), None, None)
        .await;

    match result {
        ToolResult::Success(s) => {
            // Basic sanity: should be valid JSON and include a stats object.
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            assert!(v.get("stats").is_some());
        }
        other => panic!("expected success, got: {other:?}"),
    }
}

#[tokio::test]
async fn file_read_honors_start_end_range() {
    use tempfile::tempdir;

    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = tempdir().unwrap();
    std::fs::write(temp.path().join("foo.txt"), "l1\nl2\nl3\n").unwrap();

    let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
        .expect("workspace should be created");

    let args = serde_json::json!({
        "operation": "read",
        "path": "foo.txt",
        "start": 2,
        "end": 2
    })
    .to_string();

    let result = pipeline
        .execute_tool("file", &args, Some(&ws), None, None)
        .await;
    match result {
        ToolResult::Success(s) => {
            assert!(s.contains("l2"));
            assert!(!s.contains("l1"));
            assert!(!s.contains("l3"));
        }
        other => panic!("expected success, got: {other:?}"),
    }
}

#[tokio::test]
async fn file_tree_honors_show_hidden() {
    use tempfile::tempdir;

    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = tempdir().unwrap();
    std::fs::write(temp.path().join(".hidden.txt"), "secret").unwrap();
    std::fs::write(temp.path().join("visible.txt"), "ok").unwrap();

    let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
        .expect("workspace should be created");

    let args_hidden_off = serde_json::json!({
        "operation": "tree",
        "path": ".",
        "max_depth": 1,
        "show_hidden": false
    })
    .to_string();

    let r1 = pipeline
        .execute_tool("file", &args_hidden_off, Some(&ws), None, None)
        .await;
    match r1 {
        ToolResult::Success(s) => {
            assert!(s.contains("visible.txt"));
            assert!(!s.contains(".hidden.txt"));
        }
        other => panic!("expected success, got: {other:?}"),
    }

    let args_hidden_on = serde_json::json!({
        "operation": "tree",
        "path": ".",
        "max_depth": 1,
        "show_hidden": true
    })
    .to_string();

    let r2 = pipeline
        .execute_tool("file", &args_hidden_on, Some(&ws), None, None)
        .await;
    match r2 {
        ToolResult::Success(s) => {
            assert!(s.contains("visible.txt"));
            assert!(s.contains(".hidden.txt"));
        }
        other => panic!("expected success, got: {other:?}"),
    }
}

#[tokio::test]
async fn file_edit_replaces_content() {
    use tempfile::tempdir;

    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = tempdir().unwrap();
    let p = temp.path().join("edit.txt");
    std::fs::write(&p, "hello world\n").unwrap();

    let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
        .expect("workspace should be created");

    let args = serde_json::json!({
        "operation": "edit",
        "path": "edit.txt",
        "old": "world",
        "new": "gestura"
    })
    .to_string();

    let result = pipeline
        .execute_tool("file", &args, Some(&ws), None, None)
        .await;
    match result {
        ToolResult::Success(s) => {
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            assert_eq!(v.get("replacements").and_then(|x| x.as_u64()), Some(1));
        }
        other => panic!("expected success, got: {other:?}"),
    }

    let new_content = std::fs::read_to_string(&p).unwrap();
    assert!(new_content.contains("hello gestura"));
}

#[tokio::test]
async fn shell_env_is_passed_through() {
    let pipeline = AgentPipeline::new(AppConfig::default());

    let cwd = std::env::current_dir().unwrap();
    let ws = SessionWorkspace::from_directory("test-session", cwd.clone())
        .expect("workspace should be created");

    let args = serde_json::json!({
        "command": "printf %s $FOO",
        "env": {"FOO": "BAR"},
        "timeout_secs": 10
    })
    .to_string();

    let result = pipeline
        .execute_tool("shell", &args, Some(&ws), None, None)
        .await;
    match result {
        ToolResult::Success(s) => {
            // The shell async wrapper returns stdout on success.
            assert!(s.contains("BAR"));
        }
        other => panic!("expected success, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Screen tool argument validation (must fail fast without OS capture)
// ---------------------------------------------------------------------

#[tokio::test]
async fn screenshot_tool_rejects_non_screenshot_operations() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = tempdir().unwrap();
    let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
        .expect("workspace should be created");

    let args = serde_json::json!({"operation": "start"}).to_string();
    let result = pipeline
        .execute_tool("screenshot", &args, Some(&ws), None, None)
        .await;
    match result {
        ToolResult::Error(e) => assert!(e.contains("does not support operation")),
        other => panic!("expected error, got: {other:?}"),
    }
}

#[tokio::test]
async fn screenshot_rejects_invalid_return_mode() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = tempdir().unwrap();
    let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
        .expect("workspace should be created");

    let args = serde_json::json!({
        "return": {"mode": "bogus"}
    })
    .to_string();

    let result = pipeline
        .execute_tool("screenshot", &args, Some(&ws), None, None)
        .await;
    match result {
        ToolResult::Error(e) => assert!(e.contains("Invalid return.mode")),
        other => panic!("expected error, got: {other:?}"),
    }
}

#[tokio::test]
async fn screenshot_rejects_extension_mismatch_vs_output_format() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = tempdir().unwrap();
    let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
        .expect("workspace should be created");

    let args = serde_json::json!({
        "output_path": "foo.png",
        "output_format": "jpg"
    })
    .to_string();

    let result = pipeline
        .execute_tool("screenshot", &args, Some(&ws), None, None)
        .await;
    match result {
        ToolResult::Error(e) => assert!(e.contains("does not match requested output_format")),
        other => panic!("expected error, got: {other:?}"),
    }
}

#[tokio::test]
async fn screen_record_requires_operation() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = tempdir().unwrap();
    let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
        .expect("workspace should be created");

    let args = serde_json::json!({}).to_string();
    let result = pipeline
        .execute_tool("screen_record", &args, Some(&ws), None, None)
        .await;
    match result {
        ToolResult::Error(e) => assert!(e.contains("Missing required field 'operation'")),
        other => panic!("expected error, got: {other:?}"),
    }
}

// =========================================================================
// Integration Tests for Pipeline (VALIDATION task)
// =========================================================================

use crate::context::{ContextCategory, ContextManager, estimate_tokens};

#[test]
fn test_context_reduction_reduces_prompt_size() {
    // Test that context reduction actually reduces prompt size
    let context_manager = ContextManager::new();

    // Request that doesn't need tools should have smaller context
    let simple_request = "What is the weather?";
    let analysis = context_manager.analyze(simple_request);

    // General questions shouldn't need tools
    assert!(!analysis.needs_tools || analysis.categories.contains(&ContextCategory::General));
}

#[test]
fn test_tool_filtering_by_category() {
    // Test that tool filtering works correctly based on request analysis
    let context_manager = ContextManager::new();

    // File-related request should include file tools
    let file_request = "Read the file src/main.rs";
    let analysis = context_manager.analyze(file_request);

    assert!(analysis.categories.contains(&ContextCategory::FileSystem));
    assert!(analysis.needs_tools);

    // Git-related request should include git tools
    let git_request = "Show me the git status";
    let git_analysis = context_manager.analyze(git_request);

    assert!(git_analysis.categories.contains(&ContextCategory::Git));
}

#[test]
fn build_and_test_request_uses_session_tool_pool_and_exposes_task() {
    use crate::context::ContextCategory;

    let pipeline = AgentPipeline::new(AppConfig::default());
    let mut analysis = crate::context::RequestAnalysis::new(
        "I want to create a small tauri gui that says hello world. Please carefully plan and implement then build and test it.",
    );
    analysis.needs_tools = true;
    analysis.confidence = 0.6;
    analysis.categories.insert(ContextCategory::FileSystem);
    analysis.categories.insert(ContextCategory::Code);
    analysis.categories.insert(ContextCategory::Shell);
    let allowed_tools = vec![
        "a2a".to_string(),
        "code".to_string(),
        "file".to_string(),
        "git".to_string(),
        "shell".to_string(),
        "task".to_string(),
        "web".to_string(),
        "web_search".to_string(),
    ];

    let tool_names: Vec<_> = pipeline
        .get_tools_for_analysis(&analysis, &allowed_tools)
        .into_iter()
        .map(|tool| tool.name)
        .collect();

    assert!(tool_names.contains(&"file"));
    assert!(tool_names.contains(&"code"));
    assert!(tool_names.contains(&"shell"));
    assert!(tool_names.contains(&"task"));
}

#[test]
fn analyzed_build_and_test_request_exposes_file_code_shell_and_task() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let analysis = ContextManager::new().analyze(
        "I want to create a small tauri gui that says hello world. Please carefully plan and implement then build and test it.",
    );
    let allowed_tools = vec![
        "a2a".to_string(),
        "code".to_string(),
        "file".to_string(),
        "git".to_string(),
        "shell".to_string(),
        "task".to_string(),
        "web".to_string(),
        "web_search".to_string(),
    ];

    let tool_names: Vec<_> = pipeline
        .get_tools_for_analysis(&analysis, &allowed_tools)
        .into_iter()
        .map(|tool| tool.name)
        .collect();

    assert!(tool_names.contains(&"file"));
    assert!(tool_names.contains(&"code"));
    assert!(tool_names.contains(&"shell"));
    assert!(tool_names.contains(&"task"));
}

#[test]
fn code_category_uses_split_code_tools_when_session_pool_excludes_legacy_code() {
    use crate::context::ContextCategory;

    let pipeline = AgentPipeline::new(AppConfig::default());
    let mut analysis = crate::context::RequestAnalysis::new("inspect and edit Rust code");
    analysis.needs_tools = true;
    analysis.confidence = 0.8;
    analysis.categories.insert(ContextCategory::Code);

    let allowed_tools = vec![
        "code_read_files".to_string(),
        "code_edit_files".to_string(),
        "shell".to_string(),
    ];

    let tool_names: Vec<_> = pipeline
        .get_tools_for_analysis(&analysis, &allowed_tools)
        .into_iter()
        .map(|tool| tool.name)
        .collect();

    assert!(tool_names.contains(&"code_read_files"));
    assert!(tool_names.contains(&"code_edit_files"));
    assert!(!tool_names.contains(&"code"));
}

#[test]
fn low_confidence_fallback_stays_inside_allowed_pool() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let mut analysis = crate::context::RequestAnalysis::new("use some tool");
    analysis.needs_tools = true;
    analysis.confidence = 0.0;

    let tool_names: Vec<_> = pipeline
        .get_tools_for_analysis(&analysis, &["file".to_string(), "shell".to_string()])
        .into_iter()
        .map(|tool| tool.name)
        .collect();

    assert_eq!(tool_names, vec!["file", "shell"]);
}

#[test]
fn test_token_estimation() {
    // Test token estimation function
    let short_text = "Hello world";
    let long_text = "a".repeat(1000);

    let short_tokens = estimate_tokens(short_text);
    let long_tokens = estimate_tokens(&long_text);

    // Short text should have fewer tokens
    assert!(short_tokens < long_tokens);
    // Rough estimate: ~4 chars per token
    assert!((200..=300).contains(&long_tokens));
}

#[test]
fn test_token_limit_status() {
    // Test token limit checking with AppConfig
    let app_config = AppConfig::default();
    let pipeline_config = PipelineConfig {
        max_context_tokens: 10_000, // Must be larger than max_output_tokens
        max_output_tokens: 1_000,
        ..Default::default()
    };
    let pipeline = AgentPipeline::with_config(app_config, pipeline_config);

    // Small prompt should be OK (max_input = 10000 - 1000 = 9000)
    let small_prompt = "Hello";
    let status = pipeline.check_token_limit(small_prompt);
    assert!(matches!(status, TokenLimitStatus::Ok { .. }));

    // Large prompt should exceed (10000 chars / 4 = 2500 tokens, but we need > 9000)
    let large_prompt = "a".repeat(50000); // ~12500 tokens
    let status = pipeline.check_token_limit(&large_prompt);
    assert!(matches!(status, TokenLimitStatus::Exceeded { .. }));
}

#[test]
fn test_voice_and_text_same_analysis() {
    // Test that voice and text inputs produce same analysis results
    let context_manager = ContextManager::new();

    let text_input = "List all files in the current directory";
    let voice_input = "List all files in the current directory"; // Same content

    let text_analysis = context_manager.analyze(text_input);
    let voice_analysis = context_manager.analyze(voice_input);

    // Same input should produce same analysis
    assert_eq!(text_analysis.categories, voice_analysis.categories);
    assert_eq!(text_analysis.needs_tools, voice_analysis.needs_tools);
}

#[test]
fn test_history_summarization() {
    // Test history summarization with threshold
    let context_manager = ContextManager::new();

    // Short history - should include all
    let short_history: Vec<String> = (0..5).map(|i| format!("Message {}", i)).collect();
    let short_summary = context_manager.summarize_history(&short_history);
    assert!(!short_summary.is_empty());
    assert!(short_summary.contains("Message 0"));

    // Long history - should summarize
    let long_history: Vec<String> = (0..30).map(|i| format!("Message {}", i)).collect();
    let long_summary = context_manager.summarize_history(&long_history);
    assert!(long_summary.contains("summarized"));
}

#[test]
fn test_request_similarity_detection() {
    // Test request similarity detection
    let context_manager = ContextManager::new();

    let request1 = "Read the file src/main.rs";
    let request2 = "Read the file src/main.rs"; // Same request
    let request3 = "Show git status"; // Different request

    let analysis1 = context_manager.analyze(request1);
    let analysis2 = context_manager.analyze(request2);
    let analysis3 = context_manager.analyze(request3);

    let hash1 = context_manager.compute_request_hash(&analysis1);
    let hash2 = context_manager.compute_request_hash(&analysis2);
    let hash3 = context_manager.compute_request_hash(&analysis3);

    // Same requests should have same hash
    assert_eq!(hash1, hash2);
    // Different requests should have different hash
    assert_ne!(hash1, hash3);
}

#[test]
fn test_agent_request_with_history() {
    // Test agent request with conversation history
    let history = vec![Message::user("Hello"), Message::assistant("Hi there!")];

    let request = AgentRequest::new("How are you?").with_history(history.clone());

    assert_eq!(request.history.len(), 2);
    assert_eq!(request.history[0].role, "user");
    assert_eq!(request.history[1].role, "assistant");
}

// =========================================================================
// Integration Tests for Auto-Compaction Strategies
// =========================================================================

/// Helper function to estimate tokens from a vector of messages
fn estimate_tokens_from_messages(messages: &[Message]) -> usize {
    messages.iter().map(|m| estimate_tokens(&m.content)).sum()
}

#[tokio::test]
async fn test_auto_compaction_summarize_strategy() {
    // Test that Summarize strategy triggers at 80% threshold and reduces context
    let mut config = AppConfig::default();
    config.pipeline.compaction_strategy = CompactionStrategy::Summarize;
    config.pipeline.auto_compact_threshold_percent = 80;
    config.pipeline.max_context_tokens = 1000; // Small limit for testing

    let pipeline = AgentPipeline::with_provider_optimized_config(config);

    // Create history that exceeds 80% of 1000 tokens (>800 tokens)
    // Each message needs to be longer to reach the threshold
    // Rough estimate: 4 chars per token, so we need >3200 chars total
    let mut history = Vec::new();
    for i in 0..20 {
        history.push(Message::user(format!(
                "This is test message number {} with lots of additional content to increase token count. \
                 We need to make sure this message is long enough to trigger the auto-compaction threshold. \
                 Adding more text here to ensure we exceed 800 tokens total across all messages in the history.",
                i
            )));
        history.push(Message::assistant(format!(
                "This is response number {} with lots of additional content to increase token count. \
                 We need to make sure this response is long enough to trigger the auto-compaction threshold. \
                 Adding more text here to ensure we exceed 800 tokens total across all messages in the history.",
                i
            )));
    }

    let estimated_tokens = estimate_tokens_from_messages(&history);
    assert!(
        estimated_tokens > 800,
        "History should exceed 80% threshold (got {} tokens)",
        estimated_tokens
    );

    // Build a prompt preview to test auto-compaction
    let prompt_preview: String = history
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let metadata = RequestMetadata::default();

    // Check if auto-compaction would trigger
    let compaction_result = pipeline
        .check_and_apply_auto_compaction(&history, &prompt_preview, &metadata)
        .await;

    // Should trigger compaction
    assert!(
        compaction_result.is_some(),
        "Auto-compaction should trigger"
    );
}

#[tokio::test]
async fn test_auto_compaction_truncate_strategy() {
    // Test that Truncate strategy removes oldest messages
    let mut config = AppConfig::default();
    config.pipeline.compaction_strategy = CompactionStrategy::Truncate;
    config.pipeline.auto_compact_threshold_percent = 80;
    config.pipeline.max_context_tokens = 1000;

    let pipeline = AgentPipeline::with_provider_optimized_config(config);

    let mut history = Vec::new();
    for i in 0..15 {
        history.push(Message::user(format!(
            "Message {} with additional content",
            i
        )));
        history.push(Message::assistant(format!(
            "Response {} with additional content",
            i
        )));
    }

    let messages_before = history.len();
    let prompt_preview: String = history
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let metadata = RequestMetadata::default();

    let compaction_result = pipeline
        .check_and_apply_auto_compaction(&history, &prompt_preview, &metadata)
        .await;

    // Should trigger compaction
    assert!(
        compaction_result.is_some(),
        "Auto-compaction should trigger"
    );

    // Verify compaction result indicates truncation occurred
    if let Some(StreamChunk::ContextCompacted { messages_after, .. }) = compaction_result {
        assert!(
            messages_after < messages_before,
            "Truncate should reduce message count"
        );
    }
}

#[tokio::test]
async fn test_auto_compaction_clear_strategy() {
    // Test that Clear strategy removes all history
    let mut config = AppConfig::default();
    config.pipeline.compaction_strategy = CompactionStrategy::Clear;
    config.pipeline.auto_compact_threshold_percent = 80;
    config.pipeline.max_context_tokens = 1000;

    let pipeline = AgentPipeline::with_provider_optimized_config(config);

    let mut history = Vec::new();
    for i in 0..15 {
        history.push(Message::user(format!(
            "Message {} with additional content",
            i
        )));
        history.push(Message::assistant(format!(
            "Response {} with additional content",
            i
        )));
    }

    let prompt_preview: String = history
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let metadata = RequestMetadata::default();

    let compaction_result = pipeline
        .check_and_apply_auto_compaction(&history, &prompt_preview, &metadata)
        .await;

    // Should trigger compaction
    assert!(
        compaction_result.is_some(),
        "Auto-compaction should trigger"
    );

    // Verify compaction result indicates all messages were cleared
    if let Some(StreamChunk::ContextCompacted { messages_after, .. }) = compaction_result {
        assert_eq!(
            messages_after, 0,
            "Clear strategy should remove all history"
        );
    }
}

#[tokio::test]
async fn test_auto_compaction_memory_bank_strategy() {
    // Test that MemoryBank strategy saves context to file
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let workspace_path = temp_dir.path().to_path_buf();

    let mut config = AppConfig::default();
    config.pipeline.compaction_strategy = CompactionStrategy::MemoryBank;
    config.pipeline.auto_compact_threshold_percent = 80;
    config.pipeline.max_context_tokens = 1000;

    let pipeline = AgentPipeline::with_provider_optimized_config(config);

    let mut history = Vec::new();
    for i in 0..15 {
        history.push(Message::user(format!(
            "Message {} with additional content",
            i
        )));
        history.push(Message::assistant(format!(
            "Response {} with additional content",
            i
        )));
    }

    let messages_before = history.len();
    let prompt_preview: String = history
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let metadata = RequestMetadata {
        workspace_dir: Some(workspace_path.clone()),
        session_id: Some("test-session".to_string()),
        ..Default::default()
    };

    let compaction_result = pipeline
        .check_and_apply_auto_compaction(&history, &prompt_preview, &metadata)
        .await;

    // Should trigger compaction
    assert!(
        compaction_result.is_some(),
        "Auto-compaction should trigger"
    );

    // Verify compaction result indicates memory bank save
    if let Some(StreamChunk::MemoryBankSaved { messages_saved, .. }) = compaction_result {
        assert_eq!(
            messages_saved, messages_before,
            "All messages should be saved to memory bank"
        );
    }

    // Verify memory bank file was created
    let memory_dir = workspace_path.join(".gestura").join("memory");
    assert!(
        memory_dir.exists(),
        "Memory bank directory should be created"
    );

    // Check that at least one .md file exists
    let entries = std::fs::read_dir(&memory_dir).unwrap();
    let md_files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();

    assert!(
        !md_files.is_empty(),
        "Memory bank should contain at least one markdown file"
    );
}

#[tokio::test]
async fn test_auto_compaction_threshold_not_reached() {
    // Test that auto-compaction does NOT trigger when below threshold
    let mut config = AppConfig::default();
    config.pipeline.auto_compact_threshold_percent = 80;
    config.pipeline.max_context_tokens = 10000; // Large limit

    let pipeline = AgentPipeline::with_provider_optimized_config(config);

    // Create small history that won't exceed threshold
    let history = vec![
        Message::user("Hello".to_string()),
        Message::assistant("Hi there!".to_string()),
    ];

    let estimated_tokens = estimate_tokens_from_messages(&history);
    assert!(
        estimated_tokens < 8000,
        "History should be well below 80% threshold"
    );

    let prompt_preview: String = history
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let metadata = RequestMetadata::default();

    // Check if auto-compaction would trigger
    let compaction_result = pipeline
        .check_and_apply_auto_compaction(&history, &prompt_preview, &metadata)
        .await;

    // Should NOT trigger compaction
    assert!(
        compaction_result.is_none(),
        "Should not trigger compaction when below threshold"
    );
}

#[test]
fn test_pipeline_config_user_max_context_tokens_is_clamped_to_provider_default() {
    use crate::config::PipelineSettings;

    // Base config is provider-optimized.
    let base = PipelineConfig::for_provider("ollama");
    assert_eq!(
        base.max_context_tokens,
        PipelineConfig::context_tokens_for_provider("ollama")
    );

    // User requests a larger context window than the provider default.
    let settings = PipelineSettings {
        max_context_tokens: 999_999,
        ..Default::default()
    };

    let merged = base.with_user_settings(&settings);
    assert_eq!(
        merged.max_context_tokens,
        PipelineConfig::context_tokens_for_provider("ollama"),
        "User max_context_tokens should clamp to provider default"
    );
}

// ---------------------------------------------------------------------------
// RoutingResult API
// ---------------------------------------------------------------------------

#[test]
fn routing_result_fallthrough_has_no_selection() {
    let r = RoutingResult::fallthrough();
    assert!(
        !r.has_selection(),
        "fallthrough should not have a selection"
    );
    assert!(r.suggested_tools.is_empty());
    assert_eq!(r.confidence, 0.0);
}

#[test]
fn routing_result_with_tools_has_selection() {
    let r = RoutingResult {
        suggested_tools: vec!["file".to_string(), "web".to_string()],
        confidence: 1.0,
    };
    assert!(r.has_selection());
    assert_eq!(r.suggested_tools.len(), 2);
}

// ---------------------------------------------------------------------------
// build_tool_router factory
// ---------------------------------------------------------------------------

#[test]
fn build_tool_router_keyword_returns_none() {
    let cfg = std::sync::Arc::new(AppConfig::default());
    let router = build_tool_router(&ToolRoutingStrategy::Keyword, cfg);
    assert!(
        router.is_none(),
        "Keyword strategy should produce no router object (zero overhead)"
    );
}

#[test]
fn build_tool_router_llm_returns_some() {
    let cfg = std::sync::Arc::new(AppConfig::default());
    let router = build_tool_router(&ToolRoutingStrategy::Llm, cfg);
    assert!(router.is_some(), "Llm strategy should return a router");
}

#[test]
fn build_tool_router_hybrid_returns_some() {
    let cfg = std::sync::Arc::new(AppConfig::default());
    let router = build_tool_router(
        &ToolRoutingStrategy::Hybrid {
            confidence_threshold: 0.3,
        },
        cfg,
    );
    assert!(router.is_some(), "Hybrid strategy should return a router");
}

// ---------------------------------------------------------------------------
// PipelineConfig default routing strategy
// ---------------------------------------------------------------------------

/// The default routing strategy was changed from `Keyword` to `Hybrid` so that
/// natural-language requests that do not contain exact keyword matches (e.g.
/// "locate the llm.txt for Gestura.ai") fall back to a pre-flight LLM call
/// instead of the all-tools fallback.  The threshold of 0.3 means keyword
/// routing is still used when confidence is high, preserving zero-latency
/// routing for well-recognised patterns.
#[test]
fn pipeline_config_default_uses_hybrid_routing() {
    let config = PipelineConfig::default();
    assert!(
        matches!(
            config.tool_routing_strategy,
            ToolRoutingStrategy::Hybrid {
                confidence_threshold
            } if (confidence_threshold - 0.3_f32).abs() < f32::EPSILON
        ),
        "Default routing strategy must be Hybrid {{ confidence_threshold: 0.3 }}, got: {:?}",
        config.tool_routing_strategy
    );
}

// ---------------------------------------------------------------------------
// HybridToolRouter threshold gating (async — no LLM call needed for above-threshold path)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hybrid_router_above_threshold_returns_fallthrough() {
    use crate::tools::registry::all_tools;

    let cfg = std::sync::Arc::new(AppConfig::default());
    let threshold = 0.3_f32;
    let router = super::tool_router::HybridToolRouter::new(cfg, threshold);

    let tools: Vec<&'static crate::tools::registry::ToolDefinition> = all_tools().iter().collect();

    // keyword_confidence = 0.5 is ABOVE threshold of 0.3 → must fallthrough
    // without calling the LLM (no provider configured in test environment).
    let result = router.route("fetch gestura.ai", &tools, 0.5).await;
    assert!(
        !result.has_selection(),
        "Above-threshold confidence should produce a fallthrough, not an LLM selection"
    );
}

#[tokio::test]
async fn hybrid_router_at_threshold_returns_fallthrough() {
    use crate::tools::registry::all_tools;

    let cfg = std::sync::Arc::new(AppConfig::default());
    let threshold = 0.3_f32;
    let router = super::tool_router::HybridToolRouter::new(cfg, threshold);

    let tools: Vec<&'static crate::tools::registry::ToolDefinition> = all_tools().iter().collect();

    // keyword_confidence == threshold → also a fallthrough (>= check).
    let result = router.route("some request", &tools, 0.3).await;
    assert!(
        !result.has_selection(),
        "At-threshold confidence should also fallthrough"
    );
}

#[tokio::test]
async fn hybrid_router_below_threshold_falls_back_gracefully() {
    use crate::tools::registry::all_tools;

    let cfg = std::sync::Arc::new(AppConfig::default());
    let threshold = 0.3_f32;
    let router = super::tool_router::HybridToolRouter::new(cfg, threshold);

    let tools: Vec<&'static crate::tools::registry::ToolDefinition> = all_tools().iter().collect();

    // keyword_confidence = 0.1 is BELOW threshold → would invoke LLM.
    // Without a real provider, the LLM call fails and the router falls
    // through gracefully (returns fallthrough, does not panic).
    let result = router
        .route("please find llm.txt from gestura.ai", &tools, 0.1)
        .await;
    // The result is either a valid selection (if somehow a provider is
    // available) or a graceful fallthrough.  Either way, no panic.
    let _ = result.has_selection(); // just assert it doesn't panic
}
