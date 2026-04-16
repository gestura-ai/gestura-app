//! Rule-based response evaluator.
//!
//! Each named check is a pure function: `(variation_metadata, response_text) → CheckResult`.
//! All checks are deterministic and require no LLM call, making the harness runnable
//! in offline / dry-run mode.

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::scenario::EvalVariation;

/// Result of a single named check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    /// The check name (mirrors the string in `variation.checks`).
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
    /// `true` when the check was not run because the agent produced an empty
    /// response.  Skipped checks must not be counted toward pass/fail scores or
    /// included in comparison statistics — they carry no signal about agent
    /// capability and negative checks (e.g. `no_price_hallucination`) would
    /// vacuously pass on empty strings, corrupting the check heatmap.
    ///
    /// `#[serde(default)]` keeps existing JSON reports readable without this field.
    #[serde(default)]
    pub skipped: bool,
    /// Human-readable explanation (or skip reason).
    pub details: String,
}

/// Aggregate evaluation result for one variation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationResult {
    /// Individual check results.
    pub checks: Vec<CheckResult>,
    /// Overall pass/fail: all named checks must pass.
    pub passed: bool,
    /// Score: fraction of named checks that passed (0.0 – 1.0).
    pub score: f32,
}

/// Stateless rule-based evaluator.
pub struct RuleEvaluator;

impl RuleEvaluator {
    /// Evaluate a response against all checks declared in `variation`.
    ///
    /// **Empty-response short-circuit:** if the agent returned nothing, only
    /// `response_not_empty` is executed (and it fails).  All other checks are
    /// recorded as [`CheckResult::skipped`] so that negative checks such as
    /// `no_price_hallucination` cannot vacuously pass on an empty string and
    /// corrupt the check heatmap in comparison reports.
    pub fn evaluate(variation: &EvalVariation, response: &str) -> EvaluationResult {
        // ── Short-circuit on empty response ───────────────────────────────────
        let empty_check = check_response_not_empty(response);
        if !empty_check.passed {
            let mut results = vec![empty_check];
            for check_name in &variation.checks {
                if check_name != "response_not_empty" {
                    results.push(skip(check_name));
                }
            }
            return EvaluationResult { checks: results, passed: false, score: 0.0 };
        }

        // ── Normal path: non-empty response ───────────────────────────────────
        let mut results: Vec<CheckResult> = variation
            .checks
            .iter()
            .map(|check_name| Self::run_check(check_name.as_str(), variation, response))
            .collect();

        // Always run the baseline empty check even if not listed.
        if !variation.checks.contains(&"response_not_empty".to_string()) {
            results.insert(0, check_response_not_empty(response));
        }

        // Score is computed only over non-skipped checks.
        let evaluable: Vec<&CheckResult> = results.iter().filter(|r| !r.skipped).collect();
        let total = evaluable.len() as f32;
        let passed_count = evaluable.iter().filter(|r| r.passed).count() as f32;
        let score = if total > 0.0 { passed_count / total } else { 1.0 };
        let passed = evaluable.iter().all(|r| r.passed);

        EvaluationResult { checks: results, passed, score }
    }

    fn run_check(name: &str, v: &EvalVariation, response: &str) -> CheckResult {
        match name {
            "response_not_empty" => check_response_not_empty(response),
            "response_is_concise" => {
                let max = v.max_words.unwrap_or(100);
                check_word_count(response, None, Some(max))
            }
            "response_is_substantive" => {
                let min = v.min_words.unwrap_or(20);
                check_word_count(response, Some(min), None)
            }
            "contains_expected_keyword" => {
                check_contains_keyword(response, &v.expected_keywords)
            }
            "no_forbidden_pattern" => {
                check_no_forbidden_patterns(response, &v.forbidden_patterns)
            }
            "acknowledges_uncertainty" => check_acknowledges_uncertainty(response),
            "no_price_hallucination" => check_no_price_hallucination(response),
            "has_verification_step" => check_has_verification_step(response),
            "has_structured_sections" => check_has_structured_sections(response),
            "builds_on_context" => check_builds_on_context(v, response),
            "no_external_api_suggestion" => check_no_external_api_suggestion(response),
            "summarizes_provided_content" => check_summarizes_provided_content(v, response),
            "no_invented_detail" => check_no_invented_detail(v, response),
            "root_cause_explained" => check_root_cause_explained(response),
            "suggests_test" => check_suggests_test(response),
            "has_recommendation" => check_has_recommendation(response),
            "no_fabricated_live_output" => check_no_fabricated_live_output(response),
            "cites_source_material" => check_cites_source_material(response),
            "confidence_declared" => check_confidence_declared(response),
            other => CheckResult {
                name: other.to_string(),
                passed: false,
                skipped: false,
                details: format!("Unknown check: '{other}' — add it to evaluator.rs"),
            },
        }
    }
}

// ─── Individual checks ────────────────────────────────────────────────────────

fn pass(name: &str, msg: &str) -> CheckResult {
    CheckResult { name: name.to_string(), passed: true, skipped: false, details: msg.to_string() }
}
fn fail(name: &str, msg: &str) -> CheckResult {
    CheckResult { name: name.to_string(), passed: false, skipped: false, details: msg.to_string() }
}
fn skip(name: &str) -> CheckResult {
    CheckResult {
        name: name.to_string(),
        passed: false,
        skipped: true,
        details: "skipped — response was empty; cannot evaluate".to_string(),
    }
}

fn check_response_not_empty(response: &str) -> CheckResult {
    if response.trim().is_empty() {
        fail("response_not_empty", "Response is empty or whitespace-only")
    } else {
        pass("response_not_empty", "Response contains content")
    }
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn check_word_count(response: &str, min: Option<usize>, max: Option<usize>) -> CheckResult {
    let wc = word_count(response);
    let name = if min.is_some() { "response_is_substantive" } else { "response_is_concise" };
    if let Some(m) = min
        && wc < m
    {
        return fail(name, &format!("Response has {wc} words; expected ≥{m}"));
    }
    if let Some(m) = max
        && wc > m
    {
        return fail(name, &format!("Response has {wc} words; expected ≤{m}"));
    }
    pass(name, &format!("Word count {wc} is within bounds"))
}

fn check_contains_keyword(response: &str, keywords: &[String]) -> CheckResult {
    if keywords.is_empty() {
        return pass("contains_expected_keyword", "No keywords required");
    }
    let lower = response.to_lowercase();
    for kw in keywords {
        if lower.contains(&kw.to_lowercase()) {
            return pass("contains_expected_keyword", &format!("Found keyword '{kw}'"));
        }
    }
    fail(
        "contains_expected_keyword",
        &format!("None of {:?} found in response", keywords),
    )
}

fn check_no_forbidden_patterns(response: &str, patterns: &[String]) -> CheckResult {
    for pat in patterns {
        if let Ok(re) = Regex::new(pat)
            && re.is_match(response)
        {
            return fail("no_forbidden_pattern", &format!("Forbidden pattern '{pat}' matched"));
        }
    }
    pass("no_forbidden_pattern", "No forbidden patterns matched")
}

fn check_acknowledges_uncertainty(response: &str) -> CheckResult {
    // Only match phrases that are meaningful epistemic signals.
    // Removed "may", "might", "could", "though", "however", "based on" —
    // these are grammatical filler present in virtually every response and
    // produce near-zero variance across agents.
    let hedges = [
        // Attribution phrasing (historical/factual uncertainty)
        "often credited", "commonly attributed", "generally attributed",
        "widely credited", "widely regarded", "typically credited",
        "credited with", "generally considered",
        // Explicit epistemic markers
        "contested", "disputed", "debated", "not entirely clear",
        "some sources", "depending on", "it depends",
        // Contrast phrasing (e.g. "while other inventors…")
        "while other", "while some",
        // Document-grounded absence markers (long-context scenarios)
        "not mentioned", "not stated", "not listed", "not specified",
        "no mention", "doesn't mention", "does not mention",
    ];
    let lower = response.to_lowercase();
    if hedges.iter().any(|h| lower.contains(h)) {
        pass("acknowledges_uncertainty", "Response contains meaningful uncertainty hedging language")
    } else {
        fail(
            "acknowledges_uncertainty",
            "Response presents information without appropriate uncertainty hedging",
        )
    }
}

fn check_no_price_hallucination(response: &str) -> CheckResult {
    // Allow prices if they're marked as approximate or illustrative.
    let lower = response.to_lowercase();
    let ok_markers = ["approximately", "roughly", "around", "check", "verify", "varies", "estimate"];
    // Simple price pattern: currency symbol followed by digits.
    let price_re = Regex::new(r"[\$€£¥]\s*[0-9]").unwrap();
    if price_re.is_match(response) && !ok_markers.iter().any(|m| lower.contains(m)) {
        fail(
            "no_price_hallucination",
            "Response contains specific prices without verification disclaimer",
        )
    } else {
        pass("no_price_hallucination", "No unqualified price assertions found")
    }
}

fn check_has_verification_step(response: &str) -> CheckResult {
    let markers = [
        "verify", "check", "confirm", "book", "look up", "search", "visit",
        "official", "website", "recommended to", "you should check",
    ];
    let lower = response.to_lowercase();
    if markers.iter().any(|m| lower.contains(m)) {
        pass("has_verification_step", "Response includes a verification prompt")
    } else {
        fail(
            "has_verification_step",
            "Response does not direct the user to verify live data",
        )
    }
}

fn check_has_structured_sections(response: &str) -> CheckResult {
    // Accepts markdown headers, numbered lists, or bold section labels.
    let re = Regex::new(r"(?m)(^#{1,3} .+|^\d+\.\s+\S|^\*\*\S)").unwrap();
    if re.is_match(response) {
        pass("has_structured_sections", "Response contains structured sections")
    } else {
        fail("has_structured_sections", "Response lacks structured sections (headers or numbered lists)")
    }
}

fn check_builds_on_context(v: &EvalVariation, response: &str) -> CheckResult {
    if v.history.is_empty() {
        // No history to anchor against — fall back to a length check.
        return if word_count(response) >= 15 {
            pass("builds_on_context", "Response is substantive (no conversation history to anchor against)")
        } else {
            fail("builds_on_context", "Response is too short to demonstrate context retention")
        };
    }

    // Common stop words that carry no topical signal.
    let stop_words = [
        "about", "after", "also", "been", "before", "being", "between",
        "could", "each", "from", "have", "here", "into", "just", "like",
        "might", "more", "other", "over", "should", "some", "than", "that",
        "their", "them", "there", "these", "they", "this", "those", "through",
        "very", "were", "what", "when", "where", "which", "while", "will",
        "with", "would", "your",
    ];

    // Collect unique content words (len > 4) from every history message.
    let mut history_keywords: std::collections::HashSet<String> = std::collections::HashSet::new();
    for msg in &v.history {
        let lower_msg = msg.content.to_lowercase();
        for word in lower_msg.split(|c: char| !c.is_alphanumeric()) {
            if word.len() > 4 && !stop_words.iter().any(|&s| s == word) {
                history_keywords.insert(word.to_string());
            }
        }
    }

    let lower = response.to_lowercase();
    let matches = history_keywords.iter().filter(|kw| lower.contains(kw.as_str())).count();

    if matches >= 3 {
        pass(
            "builds_on_context",
            &format!("Response references {matches} term(s) from conversation history"),
        )
    } else {
        fail(
            "builds_on_context",
            &format!(
                "Response references only {matches} term(s) from conversation history; \
                 expected ≥ 3 — response may not be building on prior context"
            ),
        )
    }
}

fn check_no_external_api_suggestion(response: &str) -> CheckResult {
    // Affirmative patterns that suggest data is being sent out.
    let patterns = ["upload to", "send to", "api.openai", "openai.com", "via cloud", "external service"];
    let lower = response.to_lowercase();

    for pat in &patterns {
        if let Some(idx) = lower.find(pat) {
            // Check whether the match is in a negation context — scan up to 60
            // characters before the match for denial words.  This prevents
            // false positives like "without sending to any external services".
            let window_start = idx.saturating_sub(60);
            let window = &lower[window_start..idx];
            let negated = ["without", "not ", "no ", "never", "avoiding", "instead of"]
                .iter()
                .any(|neg| window.contains(neg));
            if !negated {
                return fail(
                    "no_external_api_suggestion",
                    "Response suggests sending data to an external service",
                );
            }
        }
    }
    pass("no_external_api_suggestion", "Response respects local-only constraint")
}

fn check_summarizes_provided_content(v: &EvalVariation, response: &str) -> CheckResult {
    // Extract the [CONTENT: ...] block from the prompt and verify the response
    // echoes or paraphrases at least 2 of its meaningful words.
    let prompt = &v.prompt;
    if let Some(block_start) = prompt.find("[CONTENT:") {
        let inner_start = block_start + "[CONTENT:".len();
        if let Some(end_offset) = prompt[inner_start..].find(']') {
            let content = &prompt[inner_start..inner_start + end_offset];
            let content_lower = content.to_lowercase();

            let stop_words = [
                "about", "after", "also", "been", "before", "being", "each",
                "from", "have", "into", "just", "like", "more", "other", "over",
                "some", "than", "that", "their", "them", "there", "these", "they",
                "this", "those", "with", "were", "without", "using",
            ];

            // Unique meaningful words from the content block.
            let mut content_words: std::collections::HashSet<&str> =
                std::collections::HashSet::new();
            for word in content_lower.split(|c: char| !c.is_alphanumeric()) {
                if word.len() > 3 && !stop_words.iter().any(|&s| s == word) {
                    content_words.insert(word);
                }
            }

            let lower = response.to_lowercase();
            let matches = content_words.iter().filter(|&&w| lower.contains(w)).count();

            return if matches >= 2 {
                pass(
                    "summarizes_provided_content",
                    &format!("Response references {matches} term(s) from the [CONTENT] block"),
                )
            } else {
                fail(
                    "summarizes_provided_content",
                    &format!(
                        "Response contains only {matches} term(s) from the [CONTENT] block; \
                         expected ≥ 2 — response may not be summarizing the provided content"
                    ),
                )
            };
        }
    }

    // No [CONTENT:] block found — fall back to a length check.
    if word_count(response) >= 10 {
        pass("summarizes_provided_content", "Response is long enough to be a summary (no CONTENT block in prompt)")
    } else {
        fail("summarizes_provided_content", "Response is too short to be a meaningful summary")
    }
}

fn check_no_invented_detail(v: &EvalVariation, response: &str) -> CheckResult {
    let lower = response.to_lowercase();
    let expected_keywords = &v.expected_keywords;

    // Check 1 (existing): Response is grounded in the provided facts.
    let answered_from_facts = expected_keywords
        .iter()
        .any(|kw| lower.contains(&kw.to_lowercase()));
    let uncertainty_phrases = [
        "not stated", "not mentioned", "not provided", "i don't know",
        "unclear", "not listed", "not specified",
    ];
    let acknowledged_gap = uncertainty_phrases.iter().any(|p| lower.contains(p));

    if !answered_from_facts && !acknowledged_gap {
        return fail(
            "no_invented_detail",
            "Response may have invented details not present in the provided facts",
        );
    }

    // Check 2 (new): Detect fabricated years.
    // If the response contains a 4-digit year (19xx or 20xx) that was not
    // present anywhere in the prompt or history, flag it as a likely hallucination.
    let year_re = Regex::new(r"\b(19|20)\d{2}\b").unwrap();

    let mut source_text = v.prompt.clone();
    for msg in &v.history {
        source_text.push(' ');
        source_text.push_str(&msg.content);
    }

    let source_years: std::collections::HashSet<&str> = year_re
        .find_iter(&source_text)
        .map(|m| m.as_str())
        .collect();

    for m in year_re.find_iter(response) {
        let year = m.as_str();
        if !source_years.contains(year) {
            return fail(
                "no_invented_detail",
                &format!(
                    "Response contains year '{year}' not present in the source context — \
                     possible hallucinated detail"
                ),
            );
        }
    }

    pass("no_invented_detail", "Response is grounded in provided facts or acknowledges gap")
}

fn check_root_cause_explained(response: &str) -> CheckResult {
    let markers = [
        // Causal connectives
        "because", "cause", "reason", "due to", "results in",
        // Temporal / conditional framing
        "happens when", "occurs when", "triggered when", "triggered by",
        // Conditional failure explanation (code comments / prose)
        "panics if", "crashes if", "fails if", "fails when",
        "is missing", "does not exist", "not present",
        // Direct naming of the problem
        "root", "the issue", "the problem", "the bug", "why",
        // Error/exception language (code error explanations)
        "raises", "thrown", "throws", "exception", "error occurs",
        // Fix-consequence language
        "prevents", "avoids", "this stops",
    ];
    let lower = response.to_lowercase();
    if markers.iter().any(|m| lower.contains(m)) {
        pass("root_cause_explained", "Response explains the root cause")
    } else {
        fail("root_cause_explained", "Response does not explain why the error occurs")
    }
}

fn check_suggests_test(response: &str) -> CheckResult {
    let markers = ["test", "assert", "verify", "check", "try", "example", "run"];
    let lower = response.to_lowercase();
    if markers.iter().any(|m| lower.contains(m)) {
        pass("suggests_test", "Response includes a testing suggestion")
    } else {
        fail("suggests_test", "Response does not suggest any way to verify the fix")
    }
}

fn check_no_fabricated_live_output(response: &str) -> CheckResult {
    // If the model presents live data as fact (specific temperature, exact stock price),
    // without labeling it mock/placeholder, fail.
    let live_markers = ["current temperature is", "stock price is $", "live data shows"];
    let mock_labels = ["mock", "placeholder", "example output", "hypothetical", "for illustration"];
    let lower = response.to_lowercase();
    let has_live = live_markers.iter().any(|m| lower.contains(m));
    let has_mock_label = mock_labels.iter().any(|m| lower.contains(m));
    if has_live && !has_mock_label {
        fail(
            "no_fabricated_live_output",
            "Response presents live/real-time data without a mock/placeholder label",
        )
    } else {
        pass("no_fabricated_live_output", "No unlabeled live output detected")
    }
}

fn check_cites_source_material(response: &str) -> CheckResult {
    let markers = ["document", "passage", "states", "according", "mentioned", "the text", "provided"];
    let lower = response.to_lowercase();
    if markers.iter().any(|m| lower.contains(m)) {
        pass("cites_source_material", "Response references the provided source material")
    } else {
        fail("cites_source_material", "Response does not anchor its answer to the provided document")
    }
}

fn check_has_recommendation(response: &str) -> CheckResult {
    // Checks that the agent commits to a concrete recommendation rather than
    // giving a pure trade-off list with no conclusion.  Appropriate for system
    // design questions that explicitly ask "which would you recommend?".
    let markers = [
        "recommend", "suggestion", "suggest ", "go with", "prefer ",
        "would choose", "better suited", "better fit", "better choice",
        "i would use", "i'd use", "start with", "opt for",
    ];
    let lower = response.to_lowercase();
    if markers.iter().any(|m| lower.contains(m)) {
        pass("has_recommendation", "Response includes a concrete recommendation")
    } else {
        fail(
            "has_recommendation",
            "Response does not commit to a recommendation — lists trade-offs without a conclusion",
        )
    }
}

fn check_confidence_declared(response: &str) -> CheckResult {
    // Accepts explicit confidence, certainty language, or an acknowledgment of what's inferred vs. stated.
    let markers = [
        // Explicit grounding in provided material
        "directly stated", "the document states", "based on", "according to",
        "as described", "as stated", "as mentioned",
        // Epistemic hedging
        "explicitly", "inferred", "implied", "not mentioned", "unclear",
        // Confidence assertion
        "can confirm",
    ];
    let lower = response.to_lowercase();
    if markers.iter().any(|m| lower.contains(m)) {
        pass("confidence_declared", "Response distinguishes stated vs. inferred content")
    } else {
        fail(
            "confidence_declared",
            "Response does not declare confidence level or distinguish explicit from inferred",
        )
    }
}

