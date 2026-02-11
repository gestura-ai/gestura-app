use super::*;

impl AgentPipeline {
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

        // Add knowledge context (memory bank + enabled knowledge items)
        if !context.knowledge.is_empty() {
            for knowledge_section in &context.knowledge {
                prompt.push_str(knowledge_section);
                prompt.push('\n');
            }
        }

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
                        // Truncate tool results to prevent token explosion
                        let truncated_content = self.truncate_tool_result(&msg.content);
                        prompt.push_str(&format!("Tool result: {}\n", truncated_content));
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

    /// Search memory bank for relevant entries and load them into context
    /// Returns additional context string to prepend to the resolved context
    pub(super) async fn search_and_load_memory_bank(
        &self,
        workspace_dir: &std::path::Path,
        query: &str,
        max_entries: usize,
    ) -> Option<String> {
        // Search for relevant memory bank entries
        match crate::memory_bank::search_memory_bank(workspace_dir, query, max_entries).await {
            Ok(entries) if !entries.is_empty() => {
                tracing::info!(
                    entries_found = entries.len(),
                    max_entries = max_entries,
                    "Found relevant memory bank entries"
                );

                // Build context from entries
                let mut context = String::from("## Relevant Context from Memory Bank\n\n");

                for entry in entries {
                    context.push_str(&format!(
                        "### Memory Entry ({})\n",
                        entry.timestamp.format("%Y-%m-%d %H:%M UTC")
                    ));
                    context.push_str(&format!("**Summary**: {}\n\n", entry.summary));

                    // Include a preview of the content (first 500 chars)
                    let preview = if entry.content.len() > 500 {
                        format!("{}...\n\n", &entry.content[..500])
                    } else {
                        format!("{}\n\n", entry.content)
                    };
                    context.push_str(&preview);
                    context.push_str("---\n\n");
                }

                Some(context)
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

        for knowledge_id in enabled_ids {
            if let Some(item) = store.get(&knowledge_id) {
                context.push_str(&format!(
                    "### {}\n\n",
                    knowledge_id.replace('-', " ").to_uppercase()
                ));

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

        Some(context)
    }
}
