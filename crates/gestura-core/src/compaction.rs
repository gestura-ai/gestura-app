//! Context compaction for managing conversation history within token limits
//!
//! This module provides automatic history trimming and summarization when
//! context approaches the token limit. Based on Block Goose architecture patterns.

use crate::pipeline::types::Message;
use serde::{Deserialize, Serialize};

/// Configuration for context compaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Maximum tokens before triggering compaction
    pub max_context_tokens: usize,
    /// Target tokens after compaction (should be < max)
    pub target_context_tokens: usize,
    /// Minimum messages to always keep (most recent)
    pub min_recent_messages: usize,
    /// Whether to preserve tool call/result pairs
    pub preserve_tool_calls: bool,
    /// Whether to preserve messages marked as important
    pub preserve_important: bool,
    /// Strategy for compaction
    pub strategy: CompactionStrategy,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: 100_000,
            target_context_tokens: 80_000,
            min_recent_messages: 4,
            preserve_tool_calls: true,
            preserve_important: true,
            strategy: CompactionStrategy::SlidingWindow,
        }
    }
}

/// Strategy for compacting context
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionStrategy {
    /// Keep most recent messages, drop oldest
    SlidingWindow,
    /// Summarize older messages into a single summary
    Summarize,
    /// Keep important messages, drop less important ones
    ImportanceBased,
}

/// Result of a compaction operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    /// Number of messages before compaction
    pub messages_before: usize,
    /// Number of messages after compaction
    pub messages_after: usize,
    /// Estimated tokens before compaction
    pub tokens_before: usize,
    /// Estimated tokens after compaction
    pub tokens_after: usize,
    /// Summary of dropped content (if any)
    pub summary: Option<String>,
    /// Whether compaction was performed
    pub compacted: bool,
}

/// Event emitted during compaction for user notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionEvent {
    /// Type of compaction event
    pub event_type: CompactionEventType,
    /// Human-readable message
    pub message: String,
    /// Number of messages affected
    pub messages_affected: usize,
    /// Tokens saved
    pub tokens_saved: usize,
}

/// Types of compaction events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionEventType {
    /// Context is approaching limit, compaction will occur soon
    Warning,
    /// Compaction is starting
    Started,
    /// Compaction completed successfully
    Completed,
    /// Compaction failed
    Failed,
}

/// Manages context compaction for conversation history
#[derive(Debug, Clone)]
pub struct ContextCompactor {
    config: CompactionConfig,
}

impl Default for ContextCompactor {
    fn default() -> Self {
        Self::new(CompactionConfig::default())
    }
}

impl ContextCompactor {
    /// Create a new context compactor with the given configuration
    pub fn new(config: CompactionConfig) -> Self {
        Self { config }
    }

    /// Create with default config for a specific max token limit
    pub fn with_max_tokens(max_tokens: usize) -> Self {
        Self::new(CompactionConfig {
            max_context_tokens: max_tokens,
            target_context_tokens: (max_tokens as f64 * 0.8) as usize,
            ..Default::default()
        })
    }

    /// Check if compaction is needed based on current token count
    pub fn needs_compaction(&self, current_tokens: usize) -> bool {
        current_tokens >= self.config.max_context_tokens
    }

    /// Check if we're approaching the limit (warning threshold at 90%)
    pub fn approaching_limit(&self, current_tokens: usize) -> bool {
        let warning_threshold = (self.config.max_context_tokens as f64 * 0.9) as usize;
        current_tokens >= warning_threshold && current_tokens < self.config.max_context_tokens
    }

    /// Estimate token count for a message
    fn estimate_message_tokens(msg: &Message) -> usize {
        let content_tokens = estimate_tokens(&msg.content);
        let thinking_tokens = msg
            .thinking
            .as_ref()
            .map(|t| estimate_tokens(t))
            .unwrap_or(0);
        let role_tokens = 4; // Approximate overhead for role
        content_tokens + thinking_tokens + role_tokens
    }

    /// Estimate total tokens for a list of messages
    pub fn estimate_total_tokens(messages: &[Message]) -> usize {
        messages.iter().map(Self::estimate_message_tokens).sum()
    }

    /// Compact messages to fit within token limit
    pub fn compact(&self, messages: &[Message]) -> CompactionResult {
        let tokens_before = Self::estimate_total_tokens(messages);
        let messages_before = messages.len();

        // Check if compaction is needed
        if tokens_before < self.config.max_context_tokens {
            return CompactionResult {
                messages_before,
                messages_after: messages_before,
                tokens_before,
                tokens_after: tokens_before,
                summary: None,
                compacted: false,
            };
        }

        // Apply compaction strategy
        let (compacted_messages, summary) = match self.config.strategy {
            CompactionStrategy::SlidingWindow => self.compact_sliding_window(messages),
            CompactionStrategy::Summarize => self.compact_with_summary(messages),
            CompactionStrategy::ImportanceBased => self.compact_importance_based(messages),
        };

        let tokens_after = Self::estimate_total_tokens(&compacted_messages);

        CompactionResult {
            messages_before,
            messages_after: compacted_messages.len(),
            tokens_before,
            tokens_after,
            summary,
            compacted: true,
        }
    }

    /// Compact messages and return the compacted list
    pub fn compact_messages(&self, messages: Vec<Message>) -> (Vec<Message>, CompactionResult) {
        let tokens_before = Self::estimate_total_tokens(&messages);
        let messages_before = messages.len();

        if tokens_before < self.config.max_context_tokens {
            let result = CompactionResult {
                messages_before,
                messages_after: messages_before,
                tokens_before,
                tokens_after: tokens_before,
                summary: None,
                compacted: false,
            };
            return (messages, result);
        }

        let (compacted, summary) = match self.config.strategy {
            CompactionStrategy::SlidingWindow => self.compact_sliding_window(&messages),
            CompactionStrategy::Summarize => self.compact_with_summary(&messages),
            CompactionStrategy::ImportanceBased => self.compact_importance_based(&messages),
        };

        let tokens_after = Self::estimate_total_tokens(&compacted);
        let result = CompactionResult {
            messages_before,
            messages_after: compacted.len(),
            tokens_before,
            tokens_after,
            summary,
            compacted: true,
        };

        (compacted, result)
    }

    /// Sliding window compaction: keep most recent messages
    fn compact_sliding_window(&self, messages: &[Message]) -> (Vec<Message>, Option<String>) {
        let mut result = Vec::new();
        let mut current_tokens = 0;
        let target = self.config.target_context_tokens;

        // Always include minimum recent messages
        let min_recent = self.config.min_recent_messages.min(messages.len());

        // Add messages from the end until we hit the target
        for msg in messages.iter().rev() {
            let msg_tokens = Self::estimate_message_tokens(msg);
            if current_tokens + msg_tokens > target && result.len() >= min_recent {
                break;
            }
            result.push(msg.clone());
            current_tokens += msg_tokens;
        }

        result.reverse();

        let dropped = messages.len() - result.len();
        let summary = if dropped > 0 {
            Some(format!(
                "[Context compacted: {} earlier messages removed to stay within token limit]",
                dropped
            ))
        } else {
            None
        };

        (result, summary)
    }

    /// Summarize older messages into a single summary message
    fn compact_with_summary(&self, messages: &[Message]) -> (Vec<Message>, Option<String>) {
        let target = self.config.target_context_tokens;
        let min_recent = self.config.min_recent_messages.min(messages.len());

        // Calculate how many recent messages to keep
        let mut recent_tokens = 0;
        let mut keep_from = messages.len();

        for (i, msg) in messages.iter().enumerate().rev() {
            let msg_tokens = Self::estimate_message_tokens(msg);
            if recent_tokens + msg_tokens > target / 2 && messages.len() - i >= min_recent {
                keep_from = i + 1;
                break;
            }
            recent_tokens += msg_tokens;
            keep_from = i;
        }

        // Create summary of older messages
        let older_messages = &messages[..keep_from];
        let summary_text = self.create_summary(older_messages);

        // Build result with summary + recent messages
        let mut result = Vec::new();

        if !summary_text.is_empty() {
            result.push(Message {
                role: "system".to_string(),
                content: format!("[Conversation summary: {}]", summary_text),
                tool_call_id: None,
                thinking: None,
            });
        }

        result.extend(messages[keep_from..].iter().cloned());

        (result, Some(summary_text))
    }

    /// Importance-based compaction: keep important messages, drop less important
    fn compact_importance_based(&self, messages: &[Message]) -> (Vec<Message>, Option<String>) {
        let target = self.config.target_context_tokens;
        let min_recent = self.config.min_recent_messages;

        // Score each message by importance
        let scored: Vec<(usize, i32, &Message)> = messages
            .iter()
            .enumerate()
            .map(|(i, msg)| (i, self.score_importance(msg, i, messages.len()), msg))
            .collect();

        // Sort by importance (descending), keeping original order for ties
        let mut sorted = scored.clone();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

        // Select messages until we hit target
        let mut selected_indices: Vec<usize> = Vec::new();
        let mut current_tokens = 0;

        for (idx, _score, msg) in sorted {
            let msg_tokens = Self::estimate_message_tokens(msg);
            if current_tokens + msg_tokens <= target || selected_indices.len() < min_recent {
                selected_indices.push(idx);
                current_tokens += msg_tokens;
            }
        }

        // Sort indices to maintain original order
        selected_indices.sort();

        // Build result
        let result: Vec<Message> = selected_indices
            .iter()
            .map(|&i| messages[i].clone())
            .collect();

        let dropped = messages.len() - result.len();
        let summary = if dropped > 0 {
            Some(format!(
                "[Context compacted: {} less important messages removed]",
                dropped
            ))
        } else {
            None
        };

        (result, summary)
    }

    /// Score message importance (higher = more important)
    fn score_importance(&self, msg: &Message, index: usize, total: usize) -> i32 {
        let mut score = 0i32;

        // Recent messages are more important
        let recency = (index as f64 / total as f64 * 50.0) as i32;
        score += recency;

        // Tool calls and results are important if configured
        if self.config.preserve_tool_calls {
            if msg.role == "tool" || msg.tool_call_id.is_some() {
                score += 30;
            }
            // Assistant messages with tool calls
            if msg.role == "assistant" && msg.content.contains("tool_call") {
                score += 20;
            }
        }

        // User messages are generally important
        if msg.role == "user" {
            score += 15;
        }

        // System messages are very important
        if msg.role == "system" {
            score += 40;
        }

        // Longer messages might contain more context
        let length_bonus = (msg.content.len() / 100).min(10) as i32;
        score += length_bonus;

        score
    }

    /// Create a simple summary of messages
    fn create_summary(&self, messages: &[Message]) -> String {
        if messages.is_empty() {
            return String::new();
        }

        let user_count = messages.iter().filter(|m| m.role == "user").count();
        let assistant_count = messages.iter().filter(|m| m.role == "assistant").count();
        let tool_count = messages.iter().filter(|m| m.role == "tool").count();

        let mut summary_parts = Vec::new();

        if user_count > 0 {
            summary_parts.push(format!("{} user messages", user_count));
        }
        if assistant_count > 0 {
            summary_parts.push(format!("{} assistant responses", assistant_count));
        }
        if tool_count > 0 {
            summary_parts.push(format!("{} tool interactions", tool_count));
        }

        // Extract key topics from user messages
        let topics: Vec<&str> = messages
            .iter()
            .filter(|m| m.role == "user")
            .take(3)
            .filter_map(|m| m.content.split_whitespace().take(5).last())
            .collect();

        if !topics.is_empty() {
            summary_parts.push(format!("Topics: {}", topics.join(", ")));
        }

        summary_parts.join("; ")
    }

    /// Get the current configuration
    pub fn config(&self) -> &CompactionConfig {
        &self.config
    }
}

/// Estimate token count for a string (same as in context/manager.rs)
pub fn estimate_tokens(s: &str) -> usize {
    // Rough estimate: ~4 chars per token on average
    (s.len() / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_message(role: &str, content: &str) -> Message {
        Message {
            role: role.to_string(),
            content: content.to_string(),
            tool_call_id: None,
            thinking: None,
        }
    }

    #[test]
    fn test_no_compaction_needed() {
        let compactor = ContextCompactor::with_max_tokens(10000);
        let messages = vec![
            make_message("user", "Hello"),
            make_message("assistant", "Hi there!"),
        ];

        let result = compactor.compact(&messages);
        assert!(!result.compacted);
        assert_eq!(result.messages_before, result.messages_after);
    }

    #[test]
    fn test_sliding_window_compaction() {
        let config = CompactionConfig {
            max_context_tokens: 100,
            target_context_tokens: 50,
            min_recent_messages: 2,
            strategy: CompactionStrategy::SlidingWindow,
            ..Default::default()
        };
        let compactor = ContextCompactor::new(config);

        // Create messages that exceed the limit
        let messages: Vec<Message> = (0..10)
            .map(|i| make_message("user", &format!("Message number {} with some content", i)))
            .collect();

        let (compacted, result) = compactor.compact_messages(messages);
        assert!(result.compacted);
        assert!(compacted.len() < 10);
        assert!(compacted.len() >= 2); // min_recent_messages
    }

    #[test]
    fn test_approaching_limit() {
        let compactor = ContextCompactor::with_max_tokens(1000);

        // 90% of limit should trigger warning
        assert!(compactor.approaching_limit(900));
        assert!(compactor.approaching_limit(950));

        // Below 90% should not trigger
        assert!(!compactor.approaching_limit(800));

        // At or above limit should not trigger (needs_compaction instead)
        assert!(!compactor.approaching_limit(1000));
    }

    #[test]
    fn test_needs_compaction() {
        let compactor = ContextCompactor::with_max_tokens(1000);

        assert!(!compactor.needs_compaction(500));
        assert!(!compactor.needs_compaction(999));
        assert!(compactor.needs_compaction(1000));
        assert!(compactor.needs_compaction(1500));
    }

    #[test]
    fn test_estimate_tokens() {
        // 5 chars / 4 = 1.25 -> 1, max(1) = 1
        assert_eq!(estimate_tokens("hello"), 1);
        // 16 chars / 4 = 4
        assert_eq!(estimate_tokens("hello world test"), 4);
        // Empty string -> 0/4 = 0, max(1) = 1
        assert_eq!(estimate_tokens(""), 1);
    }
}
