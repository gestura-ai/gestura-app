//! Predictive text and commands for Gestura.app
//! Provides intelligent text prediction and command completion

#[allow(unused_imports)]
use crate::AppError;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Text prediction result
#[derive(Debug, Clone, serde::Serialize)]
pub struct PredictionResult {
    pub suggestions: Vec<TextSuggestion>,
    pub confidence: f32,
    pub processing_time_ms: f32,
}

/// Text suggestion
#[derive(Debug, Clone, serde::Serialize)]
pub struct TextSuggestion {
    pub text: String,
    pub confidence: f32,
    pub suggestion_type: SuggestionType,
    pub context: Option<String>,
}

/// Types of suggestions
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum SuggestionType {
    WordCompletion,
    PhraseCompletion,
    CommandCompletion,
    ContextualSuggestion,
    AutoCorrection,
}

/// Language model for predictions
#[derive(Debug, Clone, Default)]
pub struct LanguageModel {
    /// N-gram frequencies (n-gram -> frequency)
    ngrams: HashMap<String, u32>,
    /// Word frequencies
    word_frequencies: HashMap<String, u32>,
    /// Command patterns
    command_patterns: HashMap<String, Vec<String>>,
    /// Context-aware suggestions
    context_suggestions: HashMap<String, Vec<String>>,
}

/// Predictive text engine
#[derive(Clone)]
pub struct PredictiveTextEngine {
    model: Arc<RwLock<LanguageModel>>,
    user_history: Arc<RwLock<VecDeque<String>>>,
    command_history: Arc<RwLock<VecDeque<String>>>,
    max_history_size: usize,
    min_confidence_threshold: f32,
}

impl PredictiveTextEngine {
    /// Create a new predictive text engine
    pub fn new(max_history_size: usize, min_confidence_threshold: f32) -> Self {
        let engine = Self {
            model: Arc::new(RwLock::new(LanguageModel::default())),
            user_history: Arc::new(RwLock::new(VecDeque::new())),
            command_history: Arc::new(RwLock::new(VecDeque::new())),
            max_history_size,
            min_confidence_threshold,
        };

        // Initialize with common patterns in background
        let engine_clone = engine.clone();
        tokio::spawn(async move {
            if let Err(e) = engine_clone.initialize_default_patterns().await {
                tracing::error!("Failed to initialize default patterns: {}", e);
            }
        });

        engine
    }

    /// Get text predictions for input
    pub async fn predict(
        &self,
        input: &str,
        context: Option<&str>,
    ) -> Result<PredictionResult, AppError> {
        let start_time = std::time::Instant::now();

        if input.is_empty() {
            return Ok(PredictionResult {
                suggestions: Vec::new(),
                confidence: 0.0,
                processing_time_ms: 0.0,
            });
        }

        let model = self.model.read().await;
        let mut suggestions = Vec::new();

        // Word completion
        if let Some(word_suggestions) = self.get_word_completions(&model, input).await {
            suggestions.extend(word_suggestions);
        }

        // Phrase completion
        if let Some(phrase_suggestions) = self.get_phrase_completions(&model, input).await {
            suggestions.extend(phrase_suggestions);
        }

        // Command completion
        if (input.starts_with('/') || input.starts_with('!'))
            && let Some(command_suggestions) = self.get_command_completions(&model, input).await
        {
            suggestions.extend(command_suggestions);
        }

        // Contextual suggestions
        if let Some(ctx) = context
            && let Some(context_suggestions) =
                self.get_contextual_suggestions(&model, input, ctx).await
        {
            suggestions.extend(context_suggestions);
        }

        // Auto-correction
        if let Some(corrections) = self.get_auto_corrections(&model, input).await {
            suggestions.extend(corrections);
        }

        // Sort by confidence and filter
        suggestions.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        suggestions.retain(|s| s.confidence >= self.min_confidence_threshold);
        suggestions.truncate(10); // Limit to top 10 suggestions

        let overall_confidence = if !suggestions.is_empty() {
            suggestions.iter().map(|s| s.confidence).sum::<f32>() / suggestions.len() as f32
        } else {
            0.0
        };

        let processing_time = start_time.elapsed().as_millis() as f32;

        Ok(PredictionResult {
            suggestions,
            confidence: overall_confidence,
            processing_time_ms: processing_time,
        })
    }

    /// Learn from user input
    pub async fn learn_from_input(&self, input: &str, is_command: bool) -> Result<(), AppError> {
        if input.trim().is_empty() {
            return Ok(());
        }

        // Add to appropriate history
        if is_command {
            let mut command_history = self.command_history.write().await;
            command_history.push_back(input.to_string());
            if command_history.len() > self.max_history_size {
                command_history.pop_front();
            }
        } else {
            let mut user_history = self.user_history.write().await;
            user_history.push_back(input.to_string());
            if user_history.len() > self.max_history_size {
                user_history.pop_front();
            }
        }

        // Update language model
        self.update_model(input, is_command).await?;

        Ok(())
    }

    /// Get word completions
    async fn get_word_completions(
        &self,
        model: &LanguageModel,
        input: &str,
    ) -> Option<Vec<TextSuggestion>> {
        let words = input.split_whitespace().collect::<Vec<&str>>();
        let last_word = words.last()?;

        if last_word.len() < 2 {
            return None;
        }

        let mut suggestions = Vec::new();

        for (word, frequency) in &model.word_frequencies {
            if word.starts_with(last_word) && word != *last_word {
                let confidence = (*frequency as f32).log10() / 10.0; // Normalize frequency to confidence
                suggestions.push(TextSuggestion {
                    text: word.clone(),
                    confidence: confidence.clamp(0.1, 1.0),
                    suggestion_type: SuggestionType::WordCompletion,
                    context: None,
                });
            }
        }

        if suggestions.is_empty() {
            None
        } else {
            Some(suggestions)
        }
    }

    /// Get phrase completions
    async fn get_phrase_completions(
        &self,
        model: &LanguageModel,
        input: &str,
    ) -> Option<Vec<TextSuggestion>> {
        let words = input.split_whitespace().collect::<Vec<&str>>();
        if words.len() < 2 {
            return None;
        }

        let mut suggestions = Vec::new();
        let last_two_words = format!("{} {}", words[words.len() - 2], words[words.len() - 1]);

        for (ngram, frequency) in &model.ngrams {
            if ngram.starts_with(&last_two_words) && ngram != &last_two_words {
                let remaining = ngram.strip_prefix(&last_two_words)?.trim();
                if !remaining.is_empty() {
                    let confidence = (*frequency as f32).log10() / 15.0;
                    suggestions.push(TextSuggestion {
                        text: remaining.to_string(),
                        confidence: confidence.clamp(0.1, 1.0),
                        suggestion_type: SuggestionType::PhraseCompletion,
                        context: Some(last_two_words.clone()),
                    });
                }
            }
        }

        if suggestions.is_empty() {
            None
        } else {
            Some(suggestions)
        }
    }

    /// Get command completions
    async fn get_command_completions(
        &self,
        model: &LanguageModel,
        input: &str,
    ) -> Option<Vec<TextSuggestion>> {
        let mut suggestions = Vec::new();

        for (command_prefix, completions) in &model.command_patterns {
            if input.starts_with(command_prefix) {
                for completion in completions {
                    if completion.starts_with(input) && completion != input {
                        suggestions.push(TextSuggestion {
                            text: completion.clone(),
                            confidence: 0.9, // High confidence for exact command matches
                            suggestion_type: SuggestionType::CommandCompletion,
                            context: Some(command_prefix.clone()),
                        });
                    }
                }
            }
        }

        if suggestions.is_empty() {
            None
        } else {
            Some(suggestions)
        }
    }

    /// Get contextual suggestions
    async fn get_contextual_suggestions(
        &self,
        model: &LanguageModel,
        input: &str,
        context: &str,
    ) -> Option<Vec<TextSuggestion>> {
        let mut suggestions = Vec::new();

        if let Some(context_suggestions) = model.context_suggestions.get(context) {
            for suggestion in context_suggestions {
                if suggestion.contains(input) || input.contains(suggestion) {
                    suggestions.push(TextSuggestion {
                        text: suggestion.clone(),
                        confidence: 0.7,
                        suggestion_type: SuggestionType::ContextualSuggestion,
                        context: Some(context.to_string()),
                    });
                }
            }
        }

        if suggestions.is_empty() {
            None
        } else {
            Some(suggestions)
        }
    }

    /// Get auto-corrections
    async fn get_auto_corrections(
        &self,
        model: &LanguageModel,
        input: &str,
    ) -> Option<Vec<TextSuggestion>> {
        let words = input.split_whitespace().collect::<Vec<&str>>();
        let last_word = words.last()?;

        if last_word.len() < 3 {
            return None;
        }

        let mut suggestions = Vec::new();

        // Simple edit distance-based corrections
        for (word, frequency) in &model.word_frequencies {
            if word.len() >= last_word.len().saturating_sub(2) && word.len() <= last_word.len() + 2
            {
                let edit_distance = self.calculate_edit_distance(last_word, word);
                if edit_distance <= 2 && edit_distance > 0 {
                    let confidence = 1.0 - (edit_distance as f32 / last_word.len() as f32);
                    suggestions.push(TextSuggestion {
                        text: word.clone(),
                        confidence: confidence * (*frequency as f32).log10() / 10.0,
                        suggestion_type: SuggestionType::AutoCorrection,
                        context: Some(format!("correction for '{}'", last_word)),
                    });
                }
            }
        }

        if suggestions.is_empty() {
            None
        } else {
            Some(suggestions)
        }
    }

    /// Calculate edit distance between two strings
    fn calculate_edit_distance(&self, s1: &str, s2: &str) -> usize {
        let len1 = s1.len();
        let len2 = s2.len();
        let mut matrix = vec![vec![0; len2 + 1]; len1 + 1];

        for (i, row) in matrix.iter_mut().enumerate().take(len1 + 1) {
            row[0] = i;
        }
        for (j, val) in matrix[0].iter_mut().enumerate().take(len2 + 1) {
            *val = j;
        }

        for (i, c1) in s1.chars().enumerate() {
            for (j, c2) in s2.chars().enumerate() {
                let cost = if c1 == c2 { 0 } else { 1 };
                matrix[i + 1][j + 1] = std::cmp::min(
                    std::cmp::min(
                        matrix[i][j + 1] + 1, // deletion
                        matrix[i + 1][j] + 1, // insertion
                    ),
                    matrix[i][j] + cost, // substitution
                );
            }
        }

        matrix[len1][len2]
    }

    /// Update language model with new input
    async fn update_model(&self, input: &str, is_command: bool) -> Result<(), AppError> {
        let mut model = self.model.write().await;

        if is_command {
            // Update command patterns
            let command_prefix = if input.starts_with('/') {
                "/"
            } else if input.starts_with('!') {
                "!"
            } else {
                "cmd"
            };

            model
                .command_patterns
                .entry(command_prefix.to_string())
                .or_insert_with(Vec::new)
                .push(input.to_string());
        } else {
            // Update word frequencies
            for word in input.split_whitespace() {
                let word = word.to_lowercase();
                *model.word_frequencies.entry(word).or_insert(0) += 1;
            }

            // Update n-grams
            let words: Vec<&str> = input.split_whitespace().collect();
            for window in words.windows(3) {
                let ngram = window.join(" ");
                *model.ngrams.entry(ngram).or_insert(0) += 1;
            }
        }

        Ok(())
    }

    /// Initialize default patterns
    async fn initialize_default_patterns(&self) -> Result<(), AppError> {
        let mut model = self.model.write().await;

        // Common words
        let common_words = vec![
            ("the", 1000),
            ("and", 800),
            ("for", 600),
            ("are", 500),
            ("but", 400),
            ("not", 350),
            ("you", 300),
            ("all", 250),
            ("can", 200),
            ("had", 150),
            ("her", 120),
            ("was", 100),
            ("one", 90),
            ("our", 80),
            ("out", 70),
        ];

        for (word, freq) in common_words {
            model.word_frequencies.insert(word.to_string(), freq);
        }

        // Common commands
        let voice_commands = vec![
            "/play",
            "/pause",
            "/stop",
            "/next",
            "/previous",
            "/volume",
            "/mute",
            "/open",
            "/close",
            "/save",
            "/delete",
            "/copy",
            "/paste",
            "/undo",
            "/redo",
        ];

        model.command_patterns.insert(
            "/".to_string(),
            voice_commands.iter().map(|s| s.to_string()).collect(),
        );

        // Gesture commands
        let gesture_commands = [
            "!tap",
            "!double_tap",
            "!swipe_left",
            "!swipe_right",
            "!swipe_up",
            "!swipe_down",
            "!pinch",
            "!zoom",
            "!rotate",
            "!hold",
            "!release",
        ];

        model.command_patterns.insert(
            "!".to_string(),
            gesture_commands.iter().map(|s| s.to_string()).collect(),
        );

        // Context suggestions
        model.context_suggestions.insert(
            "voice".to_string(),
            vec![
                "speak louder".to_string(),
                "repeat that".to_string(),
                "what did you say".to_string(),
                "I didn't understand".to_string(),
            ],
        );

        model.context_suggestions.insert(
            "gesture".to_string(),
            vec![
                "try again".to_string(),
                "gesture not recognized".to_string(),
                "please repeat gesture".to_string(),
                "calibrate ring".to_string(),
            ],
        );

        tracing::info!("Initialized predictive text with default patterns");
        Ok(())
    }

    /// Get prediction statistics
    pub async fn get_stats(&self) -> serde_json::Value {
        let model = self.model.read().await;
        let user_history = self.user_history.read().await;
        let command_history = self.command_history.read().await;

        serde_json::json!({
            "word_count": model.word_frequencies.len(),
            "ngram_count": model.ngrams.len(),
            "command_patterns": model.command_patterns.len(),
            "context_suggestions": model.context_suggestions.len(),
            "user_history_size": user_history.len(),
            "command_history_size": command_history.len(),
            "min_confidence_threshold": self.min_confidence_threshold
        })
    }

    /// Clear learning data
    pub async fn clear_learning_data(&self) -> Result<(), AppError> {
        let mut model = self.model.write().await;
        let mut user_history = self.user_history.write().await;
        let mut command_history = self.command_history.write().await;

        model.ngrams.clear();
        model.word_frequencies.clear();
        user_history.clear();
        command_history.clear();

        // Reinitialize with defaults
        drop(model);
        drop(user_history);
        drop(command_history);

        self.initialize_default_patterns().await?;

        tracing::info!("Cleared all learning data and reinitialized defaults");
        Ok(())
    }

    /// Export learned model
    pub async fn export_model(&self) -> Result<serde_json::Value, AppError> {
        let model = self.model.read().await;
        let user_history = self.user_history.read().await;
        let command_history = self.command_history.read().await;

        Ok(serde_json::json!({
            "word_frequencies": model.word_frequencies.clone(),
            "ngrams": model.ngrams.clone(),
            "command_patterns": model.command_patterns.clone(),
            "context_suggestions": model.context_suggestions.clone(),
            "user_history": user_history.clone(),
            "command_history": command_history.clone(),
            "exported_at": chrono::Utc::now()
        }))
    }

    /// Import learned model
    pub async fn import_model(&self, data: serde_json::Value) -> Result<(), AppError> {
        let mut model = self.model.write().await;
        let mut user_history = self.user_history.write().await;
        let mut command_history = self.command_history.write().await;

        if let Some(word_freq) = data.get("word_frequencies")
            && let Ok(freq_map) = serde_json::from_value::<HashMap<String, u32>>(word_freq.clone())
        {
            model.word_frequencies = freq_map;
        }

        if let Some(ngrams) = data.get("ngrams")
            && let Ok(ngram_map) = serde_json::from_value::<HashMap<String, u32>>(ngrams.clone())
        {
            model.ngrams = ngram_map;
        }

        if let Some(commands) = data.get("command_patterns")
            && let Ok(cmd_map) =
                serde_json::from_value::<HashMap<String, Vec<String>>>(commands.clone())
        {
            model.command_patterns = cmd_map;
        }

        if let Some(context) = data.get("context_suggestions")
            && let Ok(ctx_map) =
                serde_json::from_value::<HashMap<String, Vec<String>>>(context.clone())
        {
            model.context_suggestions = ctx_map;
        }

        if let Some(user_hist) = data.get("user_history")
            && let Ok(hist_vec) = serde_json::from_value::<Vec<String>>(user_hist.clone())
        {
            *user_history = hist_vec.into();
        }

        if let Some(cmd_hist) = data.get("command_history")
            && let Ok(hist_vec) = serde_json::from_value::<Vec<String>>(cmd_hist.clone())
        {
            *command_history = hist_vec.into();
        }

        tracing::info!("Imported predictive text model");
        Ok(())
    }
}

/// Global predictive text engine instance
static PREDICTIVE_TEXT_ENGINE: tokio::sync::OnceCell<PredictiveTextEngine> =
    tokio::sync::OnceCell::const_new();

/// Get the global predictive text engine
pub async fn get_predictive_text_engine() -> &'static PredictiveTextEngine {
    PREDICTIVE_TEXT_ENGINE
        .get_or_init(|| async { PredictiveTextEngine::new(1000, 0.3) })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_word_completion() {
        let engine = PredictiveTextEngine::new(100, 0.1);

        // Learn some words
        engine.learn_from_input("hello world", false).await.unwrap();
        engine.learn_from_input("hello there", false).await.unwrap();

        // Test prediction
        let result = engine.predict("hel", None).await.unwrap();
        assert!(!result.suggestions.is_empty());

        let hello_suggestions: Vec<_> = result
            .suggestions
            .iter()
            .filter(|s| s.text.starts_with("hello"))
            .collect();
        assert!(!hello_suggestions.is_empty());
    }

    #[tokio::test]
    async fn test_command_completion() {
        let engine = PredictiveTextEngine::new(100, 0.1);

        // Wait a bit for background initialization to complete
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Test command prediction
        let result = engine.predict("/pl", None).await.unwrap();
        assert!(!result.suggestions.is_empty());

        let play_suggestions: Vec<_> = result
            .suggestions
            .iter()
            .filter(|s| s.text == "/play")
            .collect();
        assert!(!play_suggestions.is_empty());
    }

    #[tokio::test]
    async fn test_learning() {
        let engine = PredictiveTextEngine::new(100, 0.1);

        // Learn from input
        engine
            .learn_from_input("machine learning is awesome", false)
            .await
            .unwrap();

        let stats = engine.get_stats().await;
        assert!(stats["word_count"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_edit_distance() {
        let engine = PredictiveTextEngine::new(100, 0.1);

        assert_eq!(engine.calculate_edit_distance("hello", "hello"), 0);
        assert_eq!(engine.calculate_edit_distance("hello", "helo"), 1);
        assert_eq!(engine.calculate_edit_distance("hello", "world"), 4);
    }
}
