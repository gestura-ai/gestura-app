//! LLM-as-judge quality evaluator.
//!
//! Calls the Anthropic Messages API synchronously with a structured rubric
//! prompt and returns per-dimension quality scores (1–5) for each response.
//!
//! The judge is entirely optional:
//! * When `ANTHROPIC_API_KEY` is absent the runner skips judging silently.
//! * API failures are non-fatal — `judge()` returns `None` and the rule-based
//!   score is unchanged.
//! * Judge scores are stored alongside rule checks in `VariationResult` and
//!   displayed in the HTML report, but they do **not** affect the pass/fail
//!   determination — that stays deterministic and reproducible.

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::scenario::EvalVariation;

const DEFAULT_JUDGE_MODEL: &str = "claude-haiku-4-5-20251001";
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

// ─── Public types ─────────────────────────────────────────────────────────────

/// Quality ratings returned by the LLM judge for one variation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeScore {
    /// Factual / technical correctness (1–5).
    pub accuracy: u8,
    /// How fully the response addresses the prompt (1–5).
    pub completeness: u8,
    /// Clarity, structure, and appropriate conciseness (1–5).
    pub clarity: u8,
    /// Holistic quality rating — not just an average (1–5).
    pub overall: u8,
    /// One-sentence rationale from the judge.
    pub reasoning: String,
    /// Model ID used as the judge (for transparency).
    pub model: String,
}

impl JudgeScore {
    /// Overall score normalised to 0.0 – 1.0.
    pub fn normalized(&self) -> f32 {
        self.overall as f32 / 5.0
    }
}

// ─── Judge client ─────────────────────────────────────────────────────────────

/// Synchronous LLM judge client backed by the Anthropic Messages API.
#[derive(Clone)]
pub struct LlmJudge {
    api_key: String,
    model:   String,
}

impl LlmJudge {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self { api_key: api_key.into(), model: DEFAULT_JUDGE_MODEL.to_string() }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Construct from `ANTHROPIC_API_KEY`; returns `None` when the var is
    /// absent or empty so callers can treat the judge as optional cleanly.
    pub fn from_env() -> Option<Self> {
        std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .map(Self::new)
    }

    /// Judge a response.  Returns `None` on any error (rate-limit, network
    /// failure, malformed JSON from the judge).  Caller treats `None` as
    /// "judge unavailable" rather than a failure.
    pub fn judge(&self, variation: &EvalVariation, response: &str) -> Option<JudgeScore> {
        if response.trim().is_empty() {
            return None;
        }

        let prompt = build_judge_prompt(variation, response);

        let body = serde_json::json!({
            "model":      self.model,
            "max_tokens": 512,
            "temperature": 0,
            "messages": [{"role": "user", "content": prompt}]
        });

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .ok()?;

        let resp = client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send();

        match resp {
            Err(e) => {
                warn!(error = %e, "LLM judge HTTP request failed");
                None
            }
            Ok(r) if !r.status().is_success() => {
                warn!(status = %r.status(), "LLM judge API returned non-2xx");
                None
            }
            Ok(r) => {
                let json: serde_json::Value = r.json().ok()?;
                let text = json["content"][0]["text"].as_str()?;
                debug!(model = %self.model, "judge response received");
                parse_judge_response(text, &self.model)
            }
        }
    }
}

// ─── Prompt builder ───────────────────────────────────────────────────────────

fn build_judge_prompt(variation: &EvalVariation, response: &str) -> String {
    let mut buf = String::with_capacity(1024);

    if !variation.history.is_empty() {
        buf.push_str("## Conversation History\n");
        for msg in &variation.history {
            let role = if msg.role == "user" { "User" } else { "Assistant" };
            buf.push_str(&format!("**{role}:** {}\n\n", msg.content));
        }
        buf.push('\n');
    }

    buf.push_str(&format!(
        "## Task Prompt\n{}\n\n## Agent Response\n{}\n\n",
        variation.prompt, response
    ));

    buf.push_str(
        "## Evaluation Instructions\n\
         Rate the agent response on these three dimensions (1 = poor, 5 = excellent):\n\n\
         - **accuracy**: Is the information factually correct and technically sound?\n\
         - **completeness**: Does the response fully address what was asked?\n\
         - **clarity**: Is the response well-structured and appropriately concise?\n\n\
         Respond with ONLY valid JSON (no markdown, no text outside the JSON object):\n\
         {\"accuracy\": N, \"completeness\": N, \"clarity\": N, \"overall\": N, \
         \"reasoning\": \"one sentence\"}\n\n\
         The \"overall\" field is your holistic 1–5 rating — not just an average.\n",
    );

    buf
}

// ─── Response parser ──────────────────────────────────────────────────────────

fn parse_judge_response(text: &str, model: &str) -> Option<JudgeScore> {
    // Tolerate occasional markdown fences from the judge.
    let start = text.find('{')?;
    let end   = text.rfind('}')?;
    let json_str = &text[start..=end];

    let v: serde_json::Value = serde_json::from_str(json_str).ok()?;

    let clamp = |field: &str| -> u8 {
        v[field].as_u64().unwrap_or(3).clamp(1, 5) as u8
    };

    Some(JudgeScore {
        accuracy:    clamp("accuracy"),
        completeness: clamp("completeness"),
        clarity:     clamp("clarity"),
        overall:     clamp("overall"),
        reasoning:   v["reasoning"].as_str().unwrap_or("").to_string(),
        model:       model.to_string(),
    })
}
