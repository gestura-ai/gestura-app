use super::*;

impl AgentPipeline {
    /// Build a user-facing status message for an auto-compaction event.
    ///
    /// This message is intended to be emitted as `StreamChunk::Status` **before** the
    /// resulting compaction chunk (e.g., `StreamChunk::ContextCompacted`) so adapters can
    /// immediately surface what's happening.
    pub(super) fn build_auto_compaction_status_message(&self, prompt_preview: &str) -> String {
        let estimated_tokens = Self::estimate_tokens(prompt_preview);
        let max_input = self
            .pipeline_config
            .max_context_tokens
            .saturating_sub(self.pipeline_config.max_output_tokens);

        let pct = (estimated_tokens.saturating_mul(100) / max_input.max(1)).min(999);
        let threshold_pct = (self.pipeline_config.auto_compact_threshold * 100.0).round() as u32;

        let strategy = match self.pipeline_config.compaction_strategy {
            CompactionStrategy::Summarize => "summarization",
            CompactionStrategy::Truncate => "truncation",
            CompactionStrategy::Clear => "clearing history",
            CompactionStrategy::Prompt => "prompting for choice",
            CompactionStrategy::MemoryBank => "memory bank save",
        };

        format!(
            "Context near token limit (~{pct}% of {max_input} input tokens; threshold {threshold_pct}%). Auto-compacting using {strategy}…"
        )
    }

    /// Estimate token count for a string
    /// Uses a simple heuristic: ~4 characters per token for English text
    /// This is a reasonable approximation for most LLM tokenizers
    pub(super) fn estimate_tokens(text: &str) -> usize {
        // More accurate estimation:
        // - Count words (roughly 1.3 tokens per word)
        // - Count special characters (often 1 token each)
        // - Average: ~4 chars per token
        let char_count = text.chars().count();
        let word_count = text.split_whitespace().count();

        // Weighted average: words contribute more to token count
        let word_based = (word_count as f64 * 1.3) as usize;
        let char_based = char_count / 4;

        // Use the higher estimate for safety
        word_based.max(char_based).max(1)
    }

    /// Check if prompt exceeds token limit and needs truncation
    pub(super) fn check_token_limit(&self, prompt: &str) -> TokenLimitStatus {
        let estimated_tokens = Self::estimate_tokens(prompt);
        let max_input = self
            .pipeline_config
            .max_context_tokens
            .saturating_sub(self.pipeline_config.max_output_tokens);

        if estimated_tokens > max_input {
            TokenLimitStatus::Exceeded {
                estimated: estimated_tokens,
                limit: max_input,
                overage: estimated_tokens - max_input,
            }
        } else if estimated_tokens > (max_input * 90 / 100) {
            TokenLimitStatus::Warning {
                estimated: estimated_tokens,
                limit: max_input,
                percentage: ((estimated_tokens * 100) / max_input.max(1)) as u8,
            }
        } else {
            TokenLimitStatus::Ok {
                estimated: estimated_tokens,
                limit: max_input,
            }
        }
    }

    /// Check if auto-compaction should be triggered based on estimated token usage
    /// Returns Some(StreamChunk) if compaction was performed, None otherwise
    pub(super) async fn check_and_apply_auto_compaction<M>(
        &self,
        history: &[M],
        prompt_preview: &str,
        metadata: &RequestMetadata,
    ) -> Option<StreamChunk>
    where
        M: AsRef<str>,
    {
        // Skip if auto-compaction is disabled (threshold <= 0.0 or >= 1.0)
        if self.pipeline_config.auto_compact_threshold <= 0.0
            || self.pipeline_config.auto_compact_threshold >= 1.0
        {
            return None;
        }

        let estimated_tokens = Self::estimate_tokens(prompt_preview);
        let max_input = self
            .pipeline_config
            .max_context_tokens
            .saturating_sub(self.pipeline_config.max_output_tokens);

        let threshold_tokens =
            (max_input as f64 * self.pipeline_config.auto_compact_threshold) as usize;

        if estimated_tokens > threshold_tokens {
            let messages_before = history.len();

            // Apply compaction strategy
            use crate::pipeline::types::CompactionStrategy;
            match self.pipeline_config.compaction_strategy {
                CompactionStrategy::Summarize => {
                    // Trigger summarization via context manager
                    let _summary = self.context_manager.summarize_history(history);

                    // Calculate tokens saved (rough estimate)
                    // Assume summarization reduces history by ~70%
                    let messages_after = (messages_before as f64 * 0.3) as usize;
                    let tokens_saved = (estimated_tokens as f64 * 0.4) as usize; // Conservative estimate

                    tracing::info!(
                        messages_before = messages_before,
                        messages_after = messages_after,
                        tokens_saved = tokens_saved,
                        estimated_tokens = estimated_tokens,
                        threshold_tokens = threshold_tokens,
                        threshold_pct = (self.pipeline_config.auto_compact_threshold * 100.0) as u8,
                        strategy = "Summarize",
                        "Auto-compaction triggered: context exceeded {}% threshold",
                        (self.pipeline_config.auto_compact_threshold * 100.0) as u8
                    );

                    Some(StreamChunk::ContextCompacted {
                        messages_before,
                        messages_after,
                        tokens_saved,
                        summary: format!(
                            "Context auto-compacted (Summarize): {} messages → {} messages (saved ~{} tokens)",
                            messages_before, messages_after, tokens_saved
                        ),
                    })
                }
                CompactionStrategy::MemoryBank => {
                    // Save context to memory bank file
                    if let Some(workspace_dir) = &metadata.workspace_dir {
                        let session_id = metadata
                            .session_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());

                        // Build summary from history
                        let summary = self.context_manager.summarize_history(history);

                        // Build full content from history
                        let content = history
                            .iter()
                            .map(|m| m.as_ref())
                            .collect::<Vec<_>>()
                            .join("\n\n");

                        let entry = crate::memory_bank::MemoryBankEntry::new(
                            session_id.clone(),
                            summary.clone(),
                            content,
                        );

                        match crate::memory_bank::save_to_memory_bank(workspace_dir, &entry).await {
                            Ok(file_path) => {
                                tracing::info!(
                                    messages_saved = messages_before,
                                    file_path = %file_path.display(),
                                    session_id = %session_id,
                                    estimated_tokens = estimated_tokens,
                                    threshold_tokens = threshold_tokens,
                                    threshold_pct = (self.pipeline_config.auto_compact_threshold * 100.0) as u8,
                                    strategy = "MemoryBank",
                                    "Auto-compaction triggered: saved context to memory bank"
                                );

                                Some(StreamChunk::MemoryBankSaved {
                                    file_path: file_path.display().to_string(),
                                    session_id,
                                    summary,
                                    messages_saved: messages_before,
                                })
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    "Failed to save context to memory bank, falling back to summarization"
                                );

                                // Fallback to summarization
                                let _summary = self.context_manager.summarize_history(history);
                                let messages_after = (messages_before as f64 * 0.3) as usize;
                                let tokens_saved = (estimated_tokens as f64 * 0.4) as usize;

                                Some(StreamChunk::ContextCompacted {
                                    messages_before,
                                    messages_after,
                                    tokens_saved,
                                    summary: format!(
                                        "Context auto-compacted (fallback): {} messages → {} messages",
                                        messages_before, messages_after
                                    ),
                                })
                            }
                        }
                    } else {
                        tracing::warn!(
                            "MemoryBank strategy requires workspace_dir, falling back to summarization"
                        );

                        // Fallback to summarization
                        let _summary = self.context_manager.summarize_history(history);
                        let messages_after = (messages_before as f64 * 0.3) as usize;
                        let tokens_saved = (estimated_tokens as f64 * 0.4) as usize;

                        Some(StreamChunk::ContextCompacted {
                            messages_before,
                            messages_after,
                            tokens_saved,
                            summary: format!(
                                "Context auto-compacted (fallback): {} messages → {} messages",
                                messages_before, messages_after
                            ),
                        })
                    }
                }
                CompactionStrategy::Truncate => {
                    // Simply truncate oldest messages
                    // This is handled by the caller (they should drop oldest messages)
                    tracing::info!(
                        messages_before = messages_before,
                        estimated_tokens = estimated_tokens,
                        threshold_tokens = threshold_tokens,
                        strategy = "Truncate",
                        "Auto-compaction triggered: truncate strategy (caller should drop oldest messages)"
                    );

                    Some(StreamChunk::ContextCompacted {
                        messages_before,
                        messages_after: 0, // Caller will handle truncation
                        tokens_saved: 0,   // Unknown until caller truncates
                        summary: "Context will be truncated (oldest messages dropped)".to_string(),
                    })
                }
                CompactionStrategy::Clear => {
                    // Clear all history
                    tracing::info!(
                        messages_before = messages_before,
                        estimated_tokens = estimated_tokens,
                        threshold_tokens = threshold_tokens,
                        strategy = "Clear",
                        "Auto-compaction triggered: clear strategy (all history will be dropped)"
                    );

                    Some(StreamChunk::ContextCompacted {
                        messages_before,
                        messages_after: 0,
                        tokens_saved: estimated_tokens,
                        summary: "Context cleared (all history dropped)".to_string(),
                    })
                }
                CompactionStrategy::Prompt => {
                    // Prompt user for action
                    // For now, just log and fallback to summarization
                    tracing::info!(
                        messages_before = messages_before,
                        estimated_tokens = estimated_tokens,
                        threshold_tokens = threshold_tokens,
                        strategy = "Prompt",
                        "Auto-compaction triggered: prompt strategy (not yet implemented, falling back to summarization)"
                    );

                    let _summary = self.context_manager.summarize_history(history);
                    let messages_after = (messages_before as f64 * 0.3) as usize;
                    let tokens_saved = (estimated_tokens as f64 * 0.4) as usize;

                    Some(StreamChunk::ContextCompacted {
                        messages_before,
                        messages_after,
                        tokens_saved,
                        summary: format!(
                            "Context auto-compacted (Prompt not yet implemented): {} messages → {} messages",
                            messages_before, messages_after
                        ),
                    })
                }
            }
        } else {
            if self.pipeline_config.log_token_usage {
                let utilization_pct = (estimated_tokens * 100 / max_input) as u8;
                tracing::debug!(
                    estimated_tokens = estimated_tokens,
                    threshold_tokens = threshold_tokens,
                    utilization_pct = utilization_pct,
                    "Auto-compaction check: below threshold ({}%)",
                    utilization_pct
                );
            }
            None
        }
    }

    /// Calculate estimated cost for a request based on provider and token count
    /// Returns cost in USD
    fn calculate_cost(&self, tokens: usize) -> f64 {
        use crate::streaming::pricing;

        let provider = &self.config.llm.primary;
        let model = match provider.as_str() {
            "openai" => self.config.llm.openai.as_ref().map(|c| c.model.as_str()),
            "anthropic" => self.config.llm.anthropic.as_ref().map(|c| c.model.as_str()),
            "gemini" => self.config.llm.gemini.as_ref().map(|c| c.model.as_str()),
            "grok" => self.config.llm.grok.as_ref().map(|c| c.model.as_str()),
            "ollama" => Some("ollama"),
            _ => None,
        };

        // Determine pricing per 1M tokens based on provider and model
        let price_per_million = match (provider.as_str(), model) {
            ("openai", Some(m)) if m.contains("gpt-4") => pricing::OPENAI_GPT4_TURBO_INPUT,
            ("openai", Some(m)) if m.contains("gpt-3.5") => pricing::OPENAI_GPT35_TURBO_INPUT,
            ("openai", _) => pricing::OPENAI_GPT4_TURBO_INPUT, // Default to GPT-4 pricing
            ("anthropic", Some(m)) if m.contains("opus") => pricing::ANTHROPIC_CLAUDE_3_OPUS_INPUT,
            ("anthropic", Some(m)) if m.contains("haiku") => {
                pricing::ANTHROPIC_CLAUDE_3_HAIKU_INPUT
            }
            ("anthropic", _) => pricing::ANTHROPIC_CLAUDE_35_SONNET_INPUT, // Default to 3.5 Sonnet
            ("gemini", Some(m)) if m.contains("1.5-pro") => pricing::GEMINI_15_PRO_INPUT,
            ("gemini", Some(m)) if m.contains("1.5-flash") => pricing::GEMINI_15_FLASH_INPUT,
            ("gemini", Some(m)) if m.contains("flash-lite") => pricing::GEMINI_20_FLASH_LITE_INPUT,
            ("gemini", _) => pricing::GEMINI_20_FLASH_INPUT, // Default to 2.0 Flash pricing
            ("grok", _) => pricing::XAI_GROK_INPUT,
            ("ollama", _) => pricing::OLLAMA_INPUT, // Free/local
            _ => pricing::DEFAULT_INPUT,
        };

        // Calculate cost: (tokens / 1,000,000) * price_per_million
        (tokens as f64 / 1_000_000.0) * price_per_million
    }

    /// Create a token usage update chunk for user feedback
    /// Returns a StreamChunk with current token utilization status
    pub(super) fn create_token_usage_update(&self, prompt: &str) -> StreamChunk {
        use crate::streaming::TokenUsageStatus;

        let estimated_tokens = Self::estimate_tokens(prompt);
        let max_input = self
            .pipeline_config
            .max_context_tokens
            .saturating_sub(self.pipeline_config.max_output_tokens);

        let percentage = ((estimated_tokens * 100) / max_input.max(1)) as u8;

        let status = if percentage < 70 {
            TokenUsageStatus::Green
        } else if percentage < 90 {
            TokenUsageStatus::Yellow
        } else {
            TokenUsageStatus::Red
        };

        let estimated_cost = self.calculate_cost(estimated_tokens);

        StreamChunk::TokenUsageUpdate {
            estimated: estimated_tokens,
            limit: max_input,
            percentage,
            status,
            estimated_cost,
        }
    }

    /// Validate that the prompt is within token limits before sending to LLM
    ///
    /// This is a hard validation that rejects requests exceeding limits,
    /// preventing API errors. Similar to Aider's check_tokens() approach.
    ///
    /// Returns an error if the prompt exceeds the maximum allowed tokens,
    /// with guidance on how to reduce context.
    pub(super) fn validate_token_limit(&self, prompt: &str) -> Result<(), AppError> {
        let status = self.check_token_limit(prompt);

        match status {
            TokenLimitStatus::Exceeded {
                estimated,
                limit,
                overage,
            } => {
                tracing::error!(
                    estimated_tokens = estimated,
                    limit = limit,
                    overage = overage,
                    "Request exceeds token limit - rejecting to prevent API error"
                );

                Err(AppError::Llm(format!(
                    "Context too large: estimated {} tokens exceeds limit of {} tokens (overage: {} tokens). \
                    Try reducing conversation history, disabling unused tools, or using /summarize to compact context.",
                    estimated, limit, overage
                )))
            }
            TokenLimitStatus::Warning {
                estimated,
                limit,
                percentage,
            } => {
                tracing::warn!(
                    estimated_tokens = estimated,
                    limit = limit,
                    utilization_pct = percentage,
                    "Token usage approaching limit ({}%)",
                    percentage
                );
                Ok(())
            }
            TokenLimitStatus::Ok { estimated, limit } => {
                if self.pipeline_config.log_token_usage {
                    tracing::debug!(
                        estimated_tokens = estimated,
                        limit = limit,
                        utilization_pct = (estimated * 100 / limit.max(1)),
                        "Token validation passed"
                    );
                }
                Ok(())
            }
        }
    }

    /// Truncate prompt to fit within token limit
    /// Strategy: Remove oldest history messages first, then truncate file content
    pub(super) fn truncate_prompt_if_needed(
        &self,
        request: &AgentRequest,
        context: &mut crate::context::ResolvedContext,
    ) -> (String, bool) {
        let mut prompt = self.build_prompt(request, context);
        let mut truncated = false;

        // Log initial token estimate if enabled
        let initial_tokens = Self::estimate_tokens(&prompt);
        if self.pipeline_config.log_token_usage {
            let max_input = self
                .pipeline_config
                .max_context_tokens
                .saturating_sub(self.pipeline_config.max_output_tokens);
            tracing::info!(
                estimated_tokens = initial_tokens,
                max_input_tokens = max_input,
                max_context_tokens = self.pipeline_config.max_context_tokens,
                history_messages = request.history.len(),
                file_contexts = context.files.len(),
                "Token usage before optimization"
            );
        }

        // Check if we need to truncate
        if let TokenLimitStatus::Exceeded { overage, .. } = self.check_token_limit(&prompt) {
            truncated = true;
            tracing::warn!(overage = overage, "Prompt exceeds token limit, truncating");

            // Strategy 1: Truncate file contents
            let chars_to_remove = overage * 4; // Approximate chars per token
            let mut removed = 0;

            for file in context.files.iter_mut() {
                if removed >= chars_to_remove {
                    break;
                }
                let file_len = file.content.len();
                if file_len > 500 {
                    // Keep first 200 and last 200 chars
                    let truncated_content = format!(
                        "{}...[truncated {} chars]...{}",
                        &file.content[..200],
                        file_len - 400,
                        &file.content[file_len - 200..]
                    );
                    removed += file_len - truncated_content.len();
                    file.content = truncated_content;
                    file.truncated = true;
                }
            }

            // Rebuild prompt with truncated context
            prompt = self.build_prompt(request, context);

            // Log token usage after optimization
            let final_tokens = Self::estimate_tokens(&prompt);
            if self.pipeline_config.log_token_usage {
                tracing::info!(
                    tokens_before = initial_tokens,
                    tokens_after = final_tokens,
                    tokens_saved = initial_tokens.saturating_sub(final_tokens),
                    "Token usage after optimization"
                );
            }

            // If still over, log a warning (we've done what we can)
            if let TokenLimitStatus::Exceeded {
                estimated, limit, ..
            } = self.check_token_limit(&prompt)
            {
                tracing::error!(
                    estimated = estimated,
                    limit = limit,
                    "Prompt still exceeds limit after truncation"
                );
            }
        }

        (prompt, truncated)
    }
}
