use super::*;

impl AgentPipeline {
    fn append_tool_discipline(&self, prompt: &mut String) {
        prompt.push_str(
            "Tool usage discipline:\n- For `task.create`, provide `name` (and preferably `description`); do not send `task_id` because the runtime assigns it.\n- For `task.update_status`, always include both `task_id` and `status`; do not omit `status` and expect the runtime to infer it.\n- Use `read_file` to read one exact file. After reading an existing file, prefer `edit_file` for targeted changes. Use `write_file` only when you provide the full replacement `content`. Reserve the generic `file` tool for list/tree/search-style inspection.\n- `code.batch_edit` requires `edits`, and `edits` must be an array even for a single change. Each entry needs `path`, `old_str`, and `new_str`.\n- For install/build/test/scaffold shell commands, include non-interactive flags when needed and set a generous `timeout_secs` (for example 300). When the command is expected to run long but should keep going while showing shell activity, set `allow_long_running=true` and optionally `stall_timeout_secs`. Do not wrap commands with shell `timeout`; use the tool's own timeout fields instead.\n- Do not manually synthesize a project scaffold with shell heredocs, bulk `mkdir`/`touch` scripts, or ad-hoc file creation when an official scaffold or init tool is still the right tool. If a scaffold tool is non-interactive-sensitive, inspect `--help` and then use one documented non-interactive scaffold/init command.\nCanonical JSON tool call shapes:\n- `task.create`: {\"operation\":\"create\",\"name\":\"Apply requested project changes\",\"description\":\"Inspect the relevant files, implement the request, and run the appropriate verification\"}\n- `task.update_status`: {\"operation\":\"update_status\",\"task_id\":\"abc123\",\"status\":\"inprogress\"}\n- `read_file`: {\"path\":\"README.md\"}\n- `write_file`: {\"path\":\"README.md\",\"content\":\"# Project notes\\n\"}\n- `edit_file`: {\"path\":\"app/main.py\",\"old\":\"print(\\\"Hello\\\")\",\"new\":\"print(\\\"Hello, world!\\\")\"}\n- `code.batch_edit`: {\"operation\":\"batch_edit\",\"edits\":[{\"path\":\"src/lib.rs\",\"old_str\":\"fn greet() {}\",\"new_str\":\"fn greet() { println!(\\\"hello\\\"); }\"}]}\n\n",
        );
    }

    /// Build an optimized prompt from request and context
    pub(super) fn build_prompt(
        &self,
        request: &AgentRequest,
        context: &crate::context::ResolvedContext,
    ) -> String {
        let mut prompt = String::new();

        // Always include a system prompt. Callers may override via `request.system_prompt`.
        let sys = request.system_prompt.clone().unwrap_or_else(|| {
            gestura_core_pipeline::persona::default_system_prompt(&request.metadata)
        });
        prompt.push_str(&format!("System: {}\n\n", sys));

        // Inject repository-local guardrails (AGENTS.md, .gestura/guardrails) when available.
        self.append_project_guardrails(&mut prompt, request);
        self.append_tool_discipline(&mut prompt);

        // Tool definitions are now passed via the structured `tools` API parameter
        // (ProviderToolSchemas) rather than duplicated in the prompt text. This avoids
        // wasting tokens on a less-detailed text listing when the model already receives
        // full JSON schemas out-of-band.

        // Add file context if any
        if !context.files.is_empty() {
            prompt.push_str("File context:\n");
            for file in &context.files {
                let truncation_note = if file.truncated { " (truncated)" } else { "" };
                prompt.push_str(&format!(
                    "--- {} ({} lines){} ---\n{}\n---\n\n",
                    file.path, file.total_lines, truncation_note, file.content
                ));
            }
        }

        if !context.memory_sections.is_empty() {
            const MAX_MEMORY_SECTIONS: usize = 4;
            const MAX_MEMORY_CHARS: usize = 2_400;

            prompt.push_str("Relevant memory:\n");
            let mut used_chars = 0usize;
            for section in context.memory_sections.iter().take(MAX_MEMORY_SECTIONS) {
                if used_chars >= MAX_MEMORY_CHARS {
                    break;
                }

                let remaining = MAX_MEMORY_CHARS.saturating_sub(used_chars);
                let rendered = if section.chars().count() > remaining {
                    format!(
                        "{}…",
                        section
                            .chars()
                            .take(remaining)
                            .collect::<String>()
                            .trim_end()
                    )
                } else {
                    section.clone()
                };

                prompt.push_str(&rendered);
                used_chars += rendered.chars().count();
                if !rendered.ends_with('\n') {
                    prompt.push('\n');
                }
                prompt.push('\n');
            }
        }

        // Add knowledge context (memory bank + enabled knowledge items)
        if !context.knowledge.is_empty() {
            for knowledge_section in &context.knowledge {
                prompt.push_str(knowledge_section);
                prompt.push('\n');
            }
        }

        self.append_tracked_task_context(&mut prompt, &request.metadata);

        // Add history summary if available (for older context)
        if let Some(ref summary) = context.history_summary {
            prompt.push_str(&format!("Conversation summary: {}\n\n", summary));
        }

        // Add recent conversation history (last N messages based on config)
        // This is critical for follow-ups like "ok, proceed" where the action
        // is described in the previous assistant message.
        let history_limit = self.pipeline_config.max_history_messages;
        if !request.history.is_empty() {
            let history_start = request.history.len().saturating_sub(history_limit);
            let included_count = request.history.len() - history_start;

            if self.pipeline_config.log_token_usage {
                tracing::debug!(
                    total_history = request.history.len(),
                    included = included_count,
                    limit = history_limit,
                    "Context management: including recent history messages"
                );
            }

            prompt.push_str("Recent conversation:\n");
            for msg in request.history.iter().skip(history_start) {
                match msg.role.as_str() {
                    "user" => prompt.push_str(&format!("User: {}\n", msg.content)),
                    "assistant" => prompt.push_str(&format!("Assistant: {}\n", msg.content)),
                    "tool" => {
                        // Truncate tool results to prevent token explosion.
                        let truncated_content = self.truncate_tool_result(&msg.content);
                        // G8: Use a structured "Tool[<id>]:" prefix so the LLM can
                        // correlate results back to specific tool calls.  When a
                        // tool_call_id is available we embed it; otherwise we fall
                        // back to the generic "Tool" label.
                        let label = msg
                            .tool_call_id
                            .as_deref()
                            .map(|id| format!("Tool[{id}]"))
                            .unwrap_or_else(|| "Tool".to_string());
                        prompt.push_str(&format!("{label}: {truncated_content}\n"));
                    }
                    _ => prompt.push_str(&format!("{}: {}\n", msg.role, msg.content)),
                }
            }
            prompt.push('\n');
        }

        // Add the current request
        prompt.push_str(&format!("User: {}\n", request.input));

        prompt
    }

    /// Append project guardrails to the prompt when enabled and a workspace root is available.
    ///
    /// Guardrails are discovered from the request's `workspace_dir` (no filesystem scanning)
    /// and are bounded by `PipelineSettings.project_guardrails.max_chars`.
    fn append_project_guardrails(&self, prompt: &mut String, request: &AgentRequest) {
        let Some(workspace_dir) = request.metadata.workspace_dir.as_deref() else {
            return;
        };

        let settings = &self.config.pipeline.project_guardrails;
        let Some(guardrails) = crate::guardrails::load_project_guardrails(workspace_dir, settings)
        else {
            return;
        };

        let truncation_note = if guardrails.truncated {
            format!(" (truncated to {} chars)", settings.max_chars)
        } else {
            String::new()
        };

        prompt.push_str("Project guardrails:\n");
        prompt.push_str(&format!(
            "Source: {}{}\n",
            guardrails.source.relative_path(),
            truncation_note
        ));
        prompt.push_str(&guardrails.content);
        if !guardrails.content.ends_with('\n') {
            prompt.push('\n');
        }
        prompt.push('\n');
    }

    /// Append the tracked task tree so the model can reference exact task IDs.
    fn append_tracked_task_context(&self, prompt: &mut String, metadata: &RequestMetadata) {
        let Some(section) = Self::build_tracked_task_context_with_manager(
            crate::get_global_task_manager(),
            metadata,
        ) else {
            return;
        };

        prompt.push_str(&section);
        if !section.ends_with('\n') {
            prompt.push('\n');
        }
        prompt.push('\n');
    }

    fn build_tracked_task_context_with_manager(
        manager: &crate::TaskManager,
        metadata: &RequestMetadata,
    ) -> Option<String> {
        let session_id = metadata.session_id.as_deref()?;
        let task_id = metadata.task_id.as_deref()?;
        let tracked_task = manager.get_task(session_id, task_id).ok().flatten()?;

        let mut section = String::from("Tracked task context:\n");
        section.push_str(
            "Use these exact task IDs when calling the task tool to update progress. For `update_status`, ALWAYS send both the exact `task_id` and an explicit `status` (`notstarted`, `inprogress`, `completed`, or `cancelled`). The runtime already manages the tracked root task's overall lifecycle during this run, so do not call `task.update_status` on the root task just to keep it `InProgress` or preserve the current state. Reserve manual task updates for genuine status changes, especially on concrete subtasks; if no status changed, continue the real work instead.\n",
        );

        let mut remaining = 12usize;
        if let Some(node) = manager
            .get_task_tree(session_id)
            .ok()
            .and_then(|nodes| Self::find_task_tree_node(&nodes, task_id).cloned())
        {
            Self::append_task_tree_node_prompt(&mut section, &node, 0, &mut remaining);
        } else {
            section.push_str(&format!(
                "- {} (ID: {}, Status: {:?})\n",
                tracked_task.name, tracked_task.id, tracked_task.status
            ));
            remaining = remaining.saturating_sub(1);
        }

        if remaining == 0 {
            section.push_str("- … additional subtasks omitted for brevity\n");
        }

        Some(section)
    }

    fn find_task_tree_node<'a>(
        nodes: &'a [crate::tasks::TaskTreeNode],
        task_id: &str,
    ) -> Option<&'a crate::tasks::TaskTreeNode> {
        for node in nodes {
            if node.task.id == task_id {
                return Some(node);
            }

            if let Some(found) = Self::find_task_tree_node(&node.children, task_id) {
                return Some(found);
            }
        }

        None
    }

    fn append_task_tree_node_prompt(
        section: &mut String,
        node: &crate::tasks::TaskTreeNode,
        depth: usize,
        remaining: &mut usize,
    ) {
        if *remaining == 0 {
            return;
        }

        let indent = "  ".repeat(depth);
        section.push_str(&format!(
            "{}- {} (ID: {}, Status: {:?})\n",
            indent, node.task.name, node.task.id, node.task.status
        ));
        *remaining = remaining.saturating_sub(1);

        for child in &node.children {
            if *remaining == 0 {
                break;
            }

            Self::append_task_tree_node_prompt(section, child, depth + 1, remaining);
        }
    }

    /// Search memory bank for relevant entries and load them into context
    /// Returns additional context string to prepend to the resolved context
    pub(super) fn load_session_working_memory(
        &self,
        session_id: Option<&str>,
        query: &str,
        max_entries: usize,
    ) -> Option<Vec<String>> {
        let session_id = session_id?;
        let store = FileAgentSessionStore::new_default();

        match store.load(session_id) {
            Ok(session) => {
                let sections = session
                    .state
                    .relevant_working_memory_sections(query, max_entries)
                    .into_iter()
                    .map(|section| format!("### Session Working Memory\n{section}"))
                    .collect::<Vec<_>>();

                if sections.is_empty() {
                    tracing::debug!(session_id, "No short-term working memory found for session");
                    None
                } else {
                    Some(sections)
                }
            }
            Err(error) => {
                tracing::debug!(session_id, error = %error, "Failed to load session working memory");
                None
            }
        }
    }

    pub(super) async fn load_shared_coordination_memory(
        &self,
        workspace_dir: &std::path::Path,
        metadata: &RequestMetadata,
        max_entries: usize,
    ) -> Option<Vec<String>> {
        if metadata.task_id.is_none()
            && metadata.directive_id.is_none()
            && metadata.memory_tags.is_empty()
        {
            return None;
        }

        let mut memory_query = crate::memory_bank::MemoryBankQuery::default()
            .with_limit(max_entries)
            .with_category(crate::orchestrator::SHARED_COGNITION_CATEGORY)
            .with_min_confidence(0.55);
        memory_query.kinds = vec![crate::memory_bank::MemoryKind::LongTerm];
        memory_query.scopes = vec![
            crate::memory_bank::MemoryScope::Task,
            crate::memory_bank::MemoryScope::Directive,
            crate::memory_bank::MemoryScope::Session,
            crate::memory_bank::MemoryScope::Workspace,
        ];

        if let Some(session_id) = metadata.session_id.as_deref() {
            memory_query.session_id = Some(session_id.to_string());
        }
        if let Some(task_id) = metadata.task_id.as_deref() {
            memory_query = memory_query.with_task(task_id.to_string());
        }
        if let Some(directive_id) = metadata.directive_id.as_deref() {
            memory_query = memory_query.with_directive(directive_id.to_string());
        }
        if !metadata.memory_tags.is_empty() {
            memory_query = memory_query.with_tags(metadata.memory_tags.clone());
        }

        match crate::memory_bank::search_memory_bank_with_query(workspace_dir, &memory_query).await
        {
            Ok(results) if !results.is_empty() => Some(
                results
                    .into_iter()
                    .map(|result| {
                        let mut section = String::from("### Shared Coordination Memory\n");
                        section.push_str(&result.entry.to_prompt_section(320));
                        if !result.matched_fields.is_empty() {
                            section.push_str(&format!(
                                "Matched via: {}\n",
                                result.matched_fields.join(", ")
                            ));
                        }
                        section
                    })
                    .collect(),
            ),
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(error = %error, "Failed to load shared coordination memory");
                None
            }
        }
    }

    pub(super) async fn search_and_load_memory_bank(
        &self,
        workspace_dir: &std::path::Path,
        metadata: &RequestMetadata,
        query: &str,
        max_entries: usize,
    ) -> Option<Vec<String>> {
        let mut memory_query = crate::memory_bank::MemoryBankQuery::text(query)
            .with_limit(max_entries)
            .with_min_confidence(0.45);
        memory_query.kinds = vec![crate::memory_bank::MemoryKind::LongTerm];
        memory_query.scopes = vec![
            crate::memory_bank::MemoryScope::Directive,
            crate::memory_bank::MemoryScope::Workspace,
            crate::memory_bank::MemoryScope::Repository,
        ];

        if metadata.session_id.is_some() {
            memory_query
                .scopes
                .push(crate::memory_bank::MemoryScope::Session);
        }
        if metadata.task_id.is_some() {
            memory_query
                .scopes
                .push(crate::memory_bank::MemoryScope::Task);
            if let Some(task_id) = metadata.task_id.as_deref() {
                memory_query = memory_query.with_task(task_id.to_string());
            }
        }
        if let Some(directive_id) = metadata.directive_id.as_deref() {
            memory_query = memory_query.with_directive(directive_id.to_string());
        }
        if let Some(agent_id) = metadata.agent_id.as_deref() {
            memory_query = memory_query.with_agent(agent_id.to_string());
        }
        if !metadata.memory_tags.is_empty() {
            memory_query = memory_query.with_tags(metadata.memory_tags.clone());
        }

        match crate::memory_bank::search_memory_bank_with_query(workspace_dir, &memory_query).await
        {
            Ok(results) if !results.is_empty() => {
                tracing::info!(
                    entries_found = results.len(),
                    max_entries = max_entries,
                    "Found relevant memory bank entries"
                );

                let sections = results
                    .into_iter()
                    .map(|result| {
                        let mut section = String::from("### Long-Term Memory\n");
                        section.push_str(&result.entry.to_prompt_section(400));
                        if !result.matched_fields.is_empty() {
                            section.push_str(&format!(
                                "Matched via: {}\n",
                                result.matched_fields.join(", ")
                            ));
                        }
                        section
                    })
                    .collect::<Vec<_>>();

                Some(sections)
            }
            Ok(_) => {
                tracing::debug!("No relevant memory bank entries found");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to search memory bank");
                None
            }
        }
    }

    /// Load enabled knowledge items for the session and format them as context
    /// Returns additional context string to include in the prompt
    pub(super) fn load_enabled_knowledge(&self, session_id: Option<&str>) -> Option<String> {
        // Check if knowledge system is configured
        let store = self.knowledge_store?;
        let settings = self.knowledge_settings?;
        let session_id = session_id?;

        // Get enabled knowledge IDs for this session
        let enabled_ids = match settings.get_enabled_knowledge(session_id) {
            Ok(ids) if !ids.is_empty() => ids,
            Ok(_) => {
                tracing::debug!("No knowledge items enabled for session");
                return None;
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to load enabled knowledge settings");
                return None;
            }
        };

        tracing::info!(
            session_id = session_id,
            enabled_count = enabled_ids.len(),
            "Loading enabled knowledge items"
        );

        // Build context from enabled knowledge items
        let mut context = String::from("## Specialized Knowledge\n\n");
        context.push_str("The following specialized knowledge is available for this session:\n\n");
        let mut added_any = false;

        for knowledge_id in enabled_ids {
            if let Some(item) = store.get(&knowledge_id) {
                added_any = true;
                context.push_str(&format!("### {}\n\n", item.name));

                // Add category
                context.push_str(&format!("**Category**: {}\n\n", item.category));

                // Add core content
                context.push_str(&item.core_content);
                context.push_str("\n\n---\n\n");

                tracing::debug!(
                    knowledge_id = %knowledge_id,
                    content_len = item.core_content.len(),
                    "Added knowledge item to context"
                );
            } else {
                tracing::warn!(
                    knowledge_id = %knowledge_id,
                    "Enabled knowledge item not found in store"
                );
            }
        }

        added_any.then_some(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TaskStatus;

    #[test]
    fn build_prompt_includes_tool_discipline_guidance() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let request = AgentRequest::new("Apply the requested project changes and verify them");
        let context = crate::context::ResolvedContext::default();

        let prompt = pipeline.build_prompt(&request, &context);

        assert!(prompt.contains("Tool usage discipline:"));
        assert!(prompt.contains("do not send `task_id` because the runtime assigns it"));
        assert!(prompt.contains("always include both `task_id` and `status`"));
        assert!(prompt.contains("prefer `edit_file` for targeted changes"));
        assert!(
            prompt
                .contains("Reserve the generic `file` tool for list/tree/search-style inspection")
        );
        assert!(prompt.contains("`code.batch_edit` requires `edits`"));
        assert!(prompt.contains("set a generous `timeout_secs`"));
        assert!(prompt.contains("Do not manually synthesize a project scaffold"));
        assert!(prompt.contains("Canonical JSON tool call shapes:"));
        assert!(prompt.contains("\"operation\":\"batch_edit\""));
    }

    #[test]
    fn tracked_task_context_includes_exact_ids_for_nested_subtasks() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let manager = crate::TaskManager::new(temp_dir.path());
        let session_id = format!("prompt-task-context-{}", uuid::Uuid::new_v4());

        let root = manager
            .create_task(&session_id, "Build hello app", "desc", None)
            .expect("root task");
        let child = manager
            .create_task(&session_id, "Implement UI", "desc", Some(root.id.clone()))
            .expect("child task");
        let grandchild = manager
            .create_task(&session_id, "Run build", "desc", Some(child.id.clone()))
            .expect("grandchild task");

        manager
            .update_task_status(&session_id, &root.id, TaskStatus::InProgress)
            .expect("root in progress");
        manager
            .update_task_status(&session_id, &child.id, TaskStatus::InProgress)
            .expect("child in progress");

        let metadata = RequestMetadata {
            session_id: Some(session_id),
            task_id: Some(root.id.clone()),
            ..Default::default()
        };

        let section = AgentPipeline::build_tracked_task_context_with_manager(&manager, &metadata)
            .expect("tracked task context");

        assert!(section.contains("Tracked task context:"));
        assert!(section.contains("Use these exact task IDs"));
        assert!(section.contains(root.id.as_str()));
        assert!(section.contains(child.id.as_str()));
        assert!(section.contains(grandchild.id.as_str()));
        assert!(section.contains("Build hello app"));
        assert!(section.contains("Implement UI"));
        assert!(section.contains("Run build"));
        assert!(section.contains("ALWAYS send both the exact `task_id` and an explicit `status`"));
        assert!(
            section.contains("runtime already manages the tracked root task's overall lifecycle")
        );
        assert!(section.contains(
            "do not call `task.update_status` on the root task just to keep it `InProgress`"
        ));
        assert!(!section.contains("Keep the tracked root task in progress until every planned descendant is completed or cancelled"));
    }
}
