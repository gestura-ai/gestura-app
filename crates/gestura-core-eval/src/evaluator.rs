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
    /// Human-readable explanation.
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
    pub fn evaluate(variation: &EvalVariation, response: &str) -> EvaluationResult {
        let mut results: Vec<CheckResult> = variation
            .checks
            .iter()
            .map(|check_name| Self::run_check(check_name.as_str(), variation, response))
            .collect();

        // Always run the baseline empty check even if not listed.
        if !variation.checks.contains(&"response_not_empty".to_string()) {
            results.insert(0, check_response_not_empty(response));
        }

        let total = results.len() as f32;
        let passed_count = results.iter().filter(|r| r.passed).count() as f32;
        let score = if total > 0.0 { passed_count / total } else { 1.0 };
        let passed = results.iter().all(|r| r.passed);

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
            "builds_on_context" => check_builds_on_context(response),
            "no_external_api_suggestion" => check_no_external_api_suggestion(response),
            "summarizes_provided_content" => check_summarizes_provided_content(response),
            "no_invented_detail" => check_no_invented_detail(response, &v.expected_keywords),
            "root_cause_explained" => check_root_cause_explained(response),
            "suggests_test" => check_suggests_test(response),
            "no_fabricated_live_output" => check_no_fabricated_live_output(response),
            "cites_source_material" => check_cites_source_material(response),
            "confidence_declared" => check_confidence_declared(response),
            other => CheckResult {
                name: other.to_string(),
                passed: false,
                details: format!("Unknown check: '{other}' — add it to evaluator.rs"),
            },
        }
    }
}

// ─── Individual checks ────────────────────────────────────────────────────────

fn pass(name: &str, msg: &str) -> CheckResult {
    CheckResult { name: name.to_string(), passed: true, details: msg.to_string() }
}
fn fail(name: &str, msg: &str) -> CheckResult {
    CheckResult { name: name.to_string(), passed: false, details: msg.to_string() }
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
    let hedges = [
        "may", "might", "could", "often credited", "commonly attributed",
        "though", "however", "contested", "disputed", "debated",
        "some sources", "depending on", "it depends", "not entirely clear",
    ];
    let lower = response.to_lowercase();
    if hedges.iter().any(|h| lower.contains(h)) {
        pass("acknowledges_uncertainty", "Response contains uncertainty hedging language")
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

fn check_builds_on_context(response: &str) -> CheckResult {
    // Minimal heuristic: the response is not just re-explaining from scratch (≥20 words, not a definition).
    let wc = word_count(response);
    let lower = response.to_lowercase();
    let re_explains = lower.starts_with("a ") || lower.starts_with("the ") || lower.starts_with("in ");
    if wc >= 15 && !re_explains {
        pass("builds_on_context", "Response appears to advance the conversation")
    } else if wc >= 15 {
        // Soft pass — length is ok even if it starts with a definition phrase.
        pass("builds_on_context", "Response is substantive (length check passed)")
    } else {
        fail("builds_on_context", "Response is too short to demonstrate context retention")
    }
}

fn check_no_external_api_suggestion(response: &str) -> CheckResult {
    let patterns = ["upload to", "send to", "api.openai", "openai.com", "via cloud", "external service"];
    let lower = response.to_lowercase();
    if patterns.iter().any(|p| lower.contains(p)) {
        fail(
            "no_external_api_suggestion",
            "Response suggests sending data to an external service",
        )
    } else {
        pass("no_external_api_suggestion", "Response respects local-only constraint")
    }
}

fn check_summarizes_provided_content(response: &str) -> CheckResult {
    // The prompt contains "[CONTENT: ...]"; a good response should echo or paraphrase some of it.
    let wc = word_count(response);
    if wc >= 10 {
        pass("summarizes_provided_content", "Response is long enough to be a summary")
    } else {
        fail("summarizes_provided_content", "Response is too short to be a meaningful summary")
    }
}

fn check_no_invented_detail(response: &str, expected_keywords: &[String]) -> CheckResult {
    // If an expected keyword is present, the model answered from the given facts.
    // If not, and the response is assertive, flag it.
    let lower = response.to_lowercase();
    let answered_from_facts = expected_keywords
        .iter()
        .any(|kw| lower.contains(&kw.to_lowercase()));
    let uncertainty_phrases = ["not stated", "not mentioned", "not provided", "i don't know", "unclear"];
    let acknowledged_gap = uncertainty_phrases.iter().any(|p| lower.contains(p));
    if answered_from_facts || acknowledged_gap {
        pass("no_invented_detail", "Response is grounded in provided facts or acknowledges gap")
    } else {
        fail(
            "no_invented_detail",
            "Response may have invented details not present in the provided facts",
        )
    }
}

fn check_root_cause_explained(response: &str) -> CheckResult {
    let markers = [
        "because", "cause", "reason", "happens when", "occurs when",
        "root", "the issue", "the problem", "why", "due to",
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

fn check_confidence_declared(response: &str) -> CheckResult {
    // Accepts explicit confidence, certainty language, or an acknowledgment of what's inferred vs. stated.
    let markers = [
        "directly stated", "explicitly", "inferred", "implied", "not mentioned",
        "unclear", "can confirm", "the document states", "based on",
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

