//! Pipeline types for unified LLM interaction.
//!
//! The canonical definitions live in `gestura-core-pipeline::types`.
//! This module re-exports them for backward compatibility so that
//! `gestura_core::pipeline::*` continues to work.
//!
//! The [`PipelineConfigExt`] extension trait adds methods that depend on
//! core-only types (e.g. [`crate::config::PipelineSettings`]).

pub use gestura_core_pipeline::types::*;

/// Extension trait for [`PipelineConfig`] methods that depend on core-only types.
///
/// This trait exists because `PipelineConfig` is defined in `gestura-core-pipeline`
/// and cannot have inherent methods that reference `gestura-core` types (like
/// `PipelineSettings`). Callers within `gestura-core` get this trait via
/// `pub use types::*`.
pub trait PipelineConfigExt {
    /// Apply user settings from `AppConfig.pipeline` to this configuration.
    ///
    /// See [`PipelineConfig::for_provider`] for provider-optimized defaults.
    ///
    /// # Examples
    ///
    /// ```
    /// use gestura_core::config::PipelineSettings;
    /// use gestura_core::pipeline::{PipelineConfig, PipelineConfigExt, CompactionStrategy};
    ///
    /// let config = PipelineConfig::for_provider("openai");
    /// assert_eq!(config.max_context_tokens, 128_000);
    ///
    /// let mut user_settings = PipelineSettings::default();
    /// user_settings.max_history_messages = 20;
    /// user_settings.compaction_strategy = CompactionStrategy::MemoryBank;
    /// user_settings.max_context_tokens = 0; // Keep provider default
    ///
    /// let config = config.with_user_settings(&user_settings);
    /// assert_eq!(config.max_history_messages, 20);
    /// assert_eq!(config.compaction_strategy, CompactionStrategy::MemoryBank);
    /// assert_eq!(config.max_context_tokens, 128_000);
    /// ```
    fn with_user_settings(self, settings: &crate::config::PipelineSettings) -> Self;
}

impl PipelineConfigExt for PipelineConfig {
    fn with_user_settings(mut self, settings: &crate::config::PipelineSettings) -> Self {
        self.max_history_messages = settings.max_history_messages;
        self.auto_compact_threshold = settings.auto_compact_threshold();
        self.compaction_strategy = settings.compaction_strategy;
        self.log_token_usage = settings.log_token_usage;

        // Only override max_context_tokens if user has set a non-zero value.
        //
        // We clamp to the base config's limit (typically a provider-optimized default) so a
        // user configuration cannot accidentally exceed the provider/model context window.
        if settings.max_context_tokens > 0 {
            self.max_context_tokens = settings.max_context_tokens.min(self.max_context_tokens);
        }

        // Map YAML reflection settings → pipeline ReflectionConfig
        self.reflection.enabled = settings.reflection.enabled;
        self.reflection.quality_threshold = settings.reflection.quality_threshold();
        self.reflection.max_injected_reflections = settings.reflection.max_injected;
        self.reflection.max_retry_attempts = settings.reflection.max_retry_attempts;
        self.reflection.promotion_confidence = settings.reflection.promotion_confidence();

        self
    }
}
