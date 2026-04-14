//! Scenario and variation types for the Gestura evaluation harness.
//!
//! Loaded from the embedded `testdata/scenarios.json` fixture. Each [`EvalScenario`]
//! maps to one of the 8 standardised test categories; each has 3 [`EvalVariation`]s
//! that cover different phrasings of the same activity.

use serde::{Deserialize, Serialize};

/// Embedded scenario definitions — compiled into the binary so the harness is self-contained.
const BUILTIN_SCENARIOS_JSON: &str = include_str!("../testdata/scenarios.json");

/// Root fixture type — the top-level object in `scenarios.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalScenarioSuite {
    pub version: u32,
    pub description: String,
    pub scenarios: Vec<EvalScenario>,
}

impl EvalScenarioSuite {
    /// Load the built-in scenarios compiled into this binary.
    ///
    /// # Panics
    /// Panics if the embedded JSON is malformed (indicates a build-time bug).
    pub fn load_builtin() -> Self {
        serde_json::from_str(BUILTIN_SCENARIOS_JSON)
            .expect("gestura-core-eval: embedded scenarios.json is malformed — this is a build-time bug")
    }

    /// Return only the scenarios whose IDs appear in `ids`. If `ids` is empty, returns all.
    pub fn filter_by_ids<'a>(&'a self, ids: &[String]) -> Vec<&'a EvalScenario> {
        if ids.is_empty() {
            self.scenarios.iter().collect()
        } else {
            self.scenarios
                .iter()
                .filter(|s| ids.contains(&s.id))
                .collect()
        }
    }
}

/// One of the 8 standardised test scenarios.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalScenario {
    /// Stable machine-readable ID (e.g. `"s1_simple_query"`).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Category label (e.g. `"simple_query"`, `"planning"`).
    pub category: String,
    /// Short description of what the scenario tests.
    pub description: String,
    /// Pipeline-level settings that apply to all variations.
    pub rubric: Rubric,
    /// Concrete prompt variations — usually 3 per scenario.
    pub variations: Vec<EvalVariation>,
}

/// Pipeline-level settings for a scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rubric {
    /// Whether to allow tool execution in the pipeline (default: false for eval).
    #[serde(default)]
    pub tools_enabled: bool,
    /// Hard cap on agentic-loop iterations (default: 1 for determinism).
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
}

fn default_max_iterations() -> usize { 1 }

/// A single prompt variation within a scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalVariation {
    /// Stable ID within the scenario (e.g. `"v1"`).
    pub id: String,
    /// The prompt sent to the pipeline for this variation.
    pub prompt: String,
    /// Optional conversation history for multi-turn scenarios.
    #[serde(default)]
    pub history: Vec<HistoryMessage>,
    /// At least one of these strings must appear in the response (case-insensitive).
    #[serde(default)]
    pub expected_keywords: Vec<String>,
    /// Response must contain fewer than this many whitespace-delimited words.
    #[serde(default)]
    pub max_words: Option<usize>,
    /// Response must contain at least this many whitespace-delimited words.
    #[serde(default)]
    pub min_words: Option<usize>,
    /// Regex patterns that must NOT match anywhere in the response.
    #[serde(default)]
    pub forbidden_patterns: Vec<String>,
    /// Named rule-based checks to run against the response.
    #[serde(default)]
    pub checks: Vec<String>,
}

/// A single message in a pre-seeded conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryMessage {
    /// `"user"` or `"assistant"`.
    pub role: String,
    /// Message text.
    pub content: String,
}

