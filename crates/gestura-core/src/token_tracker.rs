//! Token usage tracking and cost estimation for Gestura
//!
//! Provides session-level and global token usage tracking with cost estimation,
//! budget limits, and usage statistics for GUI and CLI applications.

use crate::llm_provider::TokenUsage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Maximum number of usage records to keep in history
const MAX_USAGE_HISTORY: usize = 1000;

/// A single token usage record with timestamp
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// When this usage occurred
    pub timestamp: DateTime<Utc>,
    /// Token usage details
    pub usage: TokenUsage,
    /// Session ID (if applicable)
    pub session_id: Option<String>,
}

/// Aggregated token usage statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageStats {
    /// Total input tokens used
    pub total_input_tokens: u64,
    /// Total output tokens used
    pub total_output_tokens: u64,
    /// Total tokens (input + output)
    pub total_tokens: u64,
    /// Estimated total cost in USD
    pub estimated_cost_usd: f64,
    /// Number of API calls made
    pub call_count: u64,
    /// Average tokens per call
    pub avg_tokens_per_call: f64,
}

impl UsageStats {
    /// Add a token usage record to the stats
    pub fn add(&mut self, usage: &TokenUsage) {
        self.total_input_tokens += usage.input_tokens as u64;
        self.total_output_tokens += usage.output_tokens as u64;
        self.total_tokens += usage.total_tokens as u64;
        self.estimated_cost_usd += usage.estimated_cost_usd.unwrap_or(0.0);
        self.call_count += 1;
        if self.call_count > 0 {
            self.avg_tokens_per_call = self.total_tokens as f64 / self.call_count as f64;
        }
    }

    /// Format stats as a human-readable string
    pub fn format_summary(&self) -> String {
        format!(
            "{}↓ {}↑ | ${:.4} | {} calls",
            format_token_count(self.total_input_tokens),
            format_token_count(self.total_output_tokens),
            self.estimated_cost_usd,
            self.call_count
        )
    }

    /// Format for status bar (compact)
    pub fn format_compact(&self) -> String {
        format!(
            "{}tok ${:.2}",
            format_token_count(self.total_tokens),
            self.estimated_cost_usd
        )
    }
}

/// Format token count with K/M suffix
pub fn format_token_count(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

/// Token usage tracker for session and global usage
#[derive(Debug)]
pub struct TokenTracker {
    /// Usage history (circular buffer)
    history: RwLock<VecDeque<UsageRecord>>,
    /// Session-level stats
    session_stats: RwLock<UsageStats>,
    /// Global stats (all sessions)
    global_stats: RwLock<UsageStats>,
    /// Current session ID
    session_id: RwLock<Option<String>>,
    /// Daily budget limit in USD (optional)
    daily_budget_usd: RwLock<Option<f64>>,
    /// Today's usage for budget tracking
    today_stats: RwLock<UsageStats>,
    /// Date of today_stats
    today_date: RwLock<chrono::NaiveDate>,
}

impl Default for TokenTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenTracker {
    /// Create a new token tracker
    pub fn new() -> Self {
        Self {
            history: RwLock::new(VecDeque::with_capacity(MAX_USAGE_HISTORY)),
            session_stats: RwLock::new(UsageStats::default()),
            global_stats: RwLock::new(UsageStats::default()),
            session_id: RwLock::new(None),
            daily_budget_usd: RwLock::new(None),
            today_stats: RwLock::new(UsageStats::default()),
            today_date: RwLock::new(Utc::now().date_naive()),
        }
    }

    /// Set the current session ID
    pub async fn set_session(&self, session_id: impl Into<String>) {
        let mut id = self.session_id.write().await;
        *id = Some(session_id.into());
        // Reset session stats for new session
        let mut stats = self.session_stats.write().await;
        *stats = UsageStats::default();
    }

    /// Set daily budget limit
    pub async fn set_daily_budget(&self, budget_usd: f64) {
        let mut budget = self.daily_budget_usd.write().await;
        *budget = Some(budget_usd);
    }

    /// Record a token usage event
    pub async fn record_usage(&self, usage: TokenUsage) {
        let now = Utc::now();
        let session_id = self.session_id.read().await.clone();

        // Create usage record
        let record = UsageRecord {
            timestamp: now,
            usage: usage.clone(),
            session_id,
        };

        // Add to history
        let mut history = self.history.write().await;
        if history.len() >= MAX_USAGE_HISTORY {
            history.pop_front();
        }
        history.push_back(record);
        drop(history);

        // Update session stats
        let mut session_stats = self.session_stats.write().await;
        session_stats.add(&usage);
        drop(session_stats);

        // Update global stats
        let mut global_stats = self.global_stats.write().await;
        global_stats.add(&usage);
        drop(global_stats);

        // Check if we need to reset today's stats
        let today = now.date_naive();
        let mut today_date = self.today_date.write().await;
        if *today_date != today {
            *today_date = today;
            let mut today_stats = self.today_stats.write().await;
            *today_stats = UsageStats::default();
        }
        drop(today_date);

        // Update today's stats
        let mut today_stats = self.today_stats.write().await;
        today_stats.add(&usage);
    }

    /// Get session statistics
    pub async fn get_session_stats(&self) -> UsageStats {
        self.session_stats.read().await.clone()
    }

    /// Get global statistics
    pub async fn get_global_stats(&self) -> UsageStats {
        self.global_stats.read().await.clone()
    }

    /// Get today's usage statistics
    pub async fn get_today_stats(&self) -> UsageStats {
        self.today_stats.read().await.clone()
    }

    /// Check if daily budget would be exceeded by estimated usage
    pub async fn check_budget(&self, estimated_cost: f64) -> BudgetStatus {
        let budget = self.daily_budget_usd.read().await;
        match *budget {
            None => BudgetStatus::NoBudgetSet,
            Some(limit) => {
                let today = self.today_stats.read().await;
                let current = today.estimated_cost_usd;
                let projected = current + estimated_cost;
                if projected > limit {
                    BudgetStatus::WouldExceed {
                        current,
                        projected,
                        limit,
                    }
                } else if current > limit * 0.8 {
                    BudgetStatus::NearLimit {
                        current,
                        limit,
                        remaining: limit - current,
                    }
                } else {
                    BudgetStatus::Ok {
                        current,
                        limit,
                        remaining: limit - current,
                    }
                }
            }
        }
    }

    /// Get recent usage history
    pub async fn get_recent_history(&self, count: usize) -> Vec<UsageRecord> {
        let history = self.history.read().await;
        history.iter().rev().take(count).cloned().collect()
    }

    /// Reset session stats (for new session)
    pub async fn reset_session(&self) {
        let mut stats = self.session_stats.write().await;
        *stats = UsageStats::default();
    }
}

/// Budget check result
#[derive(Debug, Clone)]
pub enum BudgetStatus {
    /// No budget limit set
    NoBudgetSet,
    /// Usage is within budget
    Ok {
        current: f64,
        limit: f64,
        remaining: f64,
    },
    /// Near budget limit (>80%)
    NearLimit {
        current: f64,
        limit: f64,
        remaining: f64,
    },
    /// Request would exceed budget
    WouldExceed {
        current: f64,
        projected: f64,
        limit: f64,
    },
}

/// Global token tracker instance
static TOKEN_TRACKER: tokio::sync::OnceCell<Arc<TokenTracker>> = tokio::sync::OnceCell::const_new();

/// Get the global token tracker
pub async fn get_token_tracker() -> &'static Arc<TokenTracker> {
    TOKEN_TRACKER
        .get_or_init(|| async { Arc::new(TokenTracker::new()) })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_token_tracker_basic() {
        let tracker = TokenTracker::new();

        let usage = TokenUsage::new(100, 50).with_cost(0.001);
        tracker.record_usage(usage).await;

        let stats = tracker.get_session_stats().await;
        assert_eq!(stats.total_input_tokens, 100);
        assert_eq!(stats.total_output_tokens, 50);
        assert_eq!(stats.total_tokens, 150);
        assert_eq!(stats.call_count, 1);
    }

    #[tokio::test]
    async fn test_budget_tracking() {
        let tracker = TokenTracker::new();
        tracker.set_daily_budget(1.0).await;

        // Use $0.85 out of $1.00 budget (>80%)
        let usage = TokenUsage::new(10000, 5000).with_cost(0.85);
        tracker.record_usage(usage).await;

        // Should be near limit (>80% used)
        let status = tracker.check_budget(0.05).await;
        assert!(matches!(status, BudgetStatus::NearLimit { .. }));
    }

    #[test]
    fn test_format_token_count() {
        assert_eq!(format_token_count(500), "500");
        assert_eq!(format_token_count(1500), "1.5K");
        assert_eq!(format_token_count(1_500_000), "1.5M");
    }
}
