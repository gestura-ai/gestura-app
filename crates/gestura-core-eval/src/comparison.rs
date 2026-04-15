//! Multi-agent comparison engine.
//!
//! [`ComparisonEngine::compare`] takes a `Vec<EvalReport>` produced by the
//! orchestrator and computes a rich [`ComparisonReport`] that answers:
//!
//! * **Who wins overall?** — ranked leaderboard
//! * **Where does each agent excel or fail?** — per-category score matrix
//! * **Is sandboxing a free lunch?** — profile degradation by agent family
//! * **Which specific checks break universally?** — check failure heatmap
//! * **Which agent is fastest?** — p50 / p95 latency summary
//! * **Full granular pass/fail?** — variation matrix
//!
//! [`ComparisonReport::print_text`] renders the leaderboard and category matrix
//! as ASCII tables suitable for terminal display or CI artefacts.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{orchestrator::agent_family, report::EvalReport};

// ─── Output types ─────────────────────────────────────────────────────────────

/// One entry in the ranked leaderboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRank {
    pub agent_id: String,
    pub agent_name: String,
    pub agent_mode: String,
    pub overall_score: f32,
    pub passed_variations: usize,
    pub total_variations: usize,
    pub rank: usize,
}

/// Per-agent, per-category mean scores.
///
/// `scores[agent_id][category]` → mean score (0.0 – 1.0) across all variations
/// in that category.  An absent entry means no scenarios for that category ran.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryMatrix {
    /// Ordered list of agent IDs (matches leaderboard rank order).
    pub agents: Vec<String>,
    /// Sorted list of category names.
    pub categories: Vec<String>,
    /// `scores[agent_id][category]` → mean score.
    pub scores: HashMap<String, HashMap<String, f32>>,
}

/// How much quality degrades across permission modes for one agent family.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FamilyDegradation {
    pub family: String,
    /// Overall score for the full-permission profile.
    pub full: Option<f32>,
    /// Overall score for the iterative profile.
    pub iterative: Option<f32>,
    /// Overall score for the sandboxed profile.
    pub sandboxed: Option<f32>,
    /// `sandboxed - full` (negative = sandboxing hurts quality).
    pub delta_full_sandboxed: Option<f32>,
}

/// Per-agent failure rate for each named check.
///
/// `failure_rates[agent_id][check_name]` → fraction of variations where the
/// check failed (0.0 = never fails, 1.0 = always fails).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckHeatmap {
    pub agents: Vec<String>,
    /// All check names that appeared in at least one variation.
    pub checks: Vec<String>,
    pub failure_rates: HashMap<String, HashMap<String, f32>>,
}

/// Latency statistics for one agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLatency {
    pub agent_id: String,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub max_ms: u64,
    pub mean_ms: u64,
}

/// Full pass/fail grid across all agents × scenarios × variations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariationMatrix {
    pub agents: Vec<String>,
    /// Ordered `"scenario_id/variation_id"` keys.
    pub slots: Vec<String>,
    /// `data[agent_id][slot]` → pass/fail.
    pub data: HashMap<String, HashMap<String, bool>>,
}

/// The complete comparison artefact produced by [`ComparisonEngine`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonReport {
    pub run_id: String,
    pub timestamp: DateTime<Utc>,
    pub leaderboard: Vec<AgentRank>,
    pub category_matrix: CategoryMatrix,
    pub profile_degradation: Vec<FamilyDegradation>,
    pub check_heatmap: CheckHeatmap,
    pub latency_summary: Vec<AgentLatency>,
    pub variation_matrix: VariationMatrix,
    /// Full per-agent reports preserved for drill-down.
    pub agent_reports: Vec<EvalReport>,
}

// ─── Engine ───────────────────────────────────────────────────────────────────

/// Stateless engine that derives a [`ComparisonReport`] from raw per-agent reports.
pub struct ComparisonEngine;

impl ComparisonEngine {
    pub fn compare(reports: Vec<EvalReport>) -> ComparisonReport {
        let leaderboard = build_leaderboard(&reports);
        let category_matrix = build_category_matrix(&reports, &leaderboard);
        let profile_degradation = build_profile_degradation(&reports);
        let check_heatmap = build_check_heatmap(&reports, &leaderboard);
        let latency_summary = build_latency_summary(&reports, &leaderboard);
        let variation_matrix = build_variation_matrix(&reports, &leaderboard);

        ComparisonReport {
            run_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            leaderboard,
            category_matrix,
            profile_degradation,
            check_heatmap,
            latency_summary,
            variation_matrix,
            agent_reports: reports,
        }
    }
}

// ─── Builder helpers ──────────────────────────────────────────────────────────

fn build_leaderboard(reports: &[EvalReport]) -> Vec<AgentRank> {
    let mut ranks: Vec<AgentRank> = reports
        .iter()
        .map(|r| AgentRank {
            agent_id: r.agent_id.clone(),
            agent_name: r.agent_name.clone(),
            agent_mode: r.agent_mode.clone(),
            overall_score: r.summary.overall_score,
            passed_variations: r.summary.passed_variations,
            total_variations: r.summary.total_variations,
            rank: 0,
        })
        .collect();

    // Sort descending by overall score, then alphabetically for ties.
    ranks.sort_unstable_by(|a, b| {
        b.overall_score
            .partial_cmp(&a.overall_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.agent_id.cmp(&b.agent_id))
    });

    for (i, r) in ranks.iter_mut().enumerate() {
        r.rank = i + 1;
    }
    ranks
}

fn build_category_matrix(reports: &[EvalReport], leaderboard: &[AgentRank]) -> CategoryMatrix {
    // Collect category → score sums.  Use (sum, count) to compute means.
    let mut raw: HashMap<&str, HashMap<&str, (f32, u32)>> = HashMap::new();

    for report in reports {
        let agent = raw.entry(&report.agent_id).or_default();
        for scenario in &report.scenarios {
            let cat_entry = agent.entry(&scenario.category).or_insert((0.0, 0));
            // Weight by variation count so all variations are equal regardless of
            // how many are in each scenario.
            for v in &scenario.variations {
                cat_entry.0 += v.score;
                cat_entry.1 += 1;
            }
        }
    }

    // Collect unique categories sorted.
    let mut categories: Vec<String> = reports
        .iter()
        .flat_map(|r| r.scenarios.iter().map(|s| s.category.clone()))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    categories.sort_unstable();

    let agents: Vec<String> = leaderboard.iter().map(|r| r.agent_id.clone()).collect();

    let mut scores: HashMap<String, HashMap<String, f32>> = HashMap::new();
    for (agent_id, cat_map) in &raw {
        let entry = scores.entry(agent_id.to_string()).or_default();
        for (cat, (sum, count)) in cat_map {
            if *count > 0 {
                let mean = (sum / *count as f32 * 1000.0).round() / 1000.0;
                entry.insert(cat.to_string(), mean);
            }
        }
    }

    CategoryMatrix { agents, categories, scores }
}

fn build_profile_degradation(reports: &[EvalReport]) -> Vec<FamilyDegradation> {
    // Group by family.
    let mut by_family: HashMap<String, HashMap<String, f32>> = HashMap::new();

    for report in reports {
        let family = agent_family(&report.agent_id).to_string();
        let mode = if report.agent_id.ends_with("-full") {
            "full"
        } else if report.agent_id.ends_with("-iterative") {
            "iterative"
        } else if report.agent_id.ends_with("-sandboxed") {
            "sandboxed"
        } else {
            "full"
        };
        by_family
            .entry(family)
            .or_default()
            .insert(mode.to_string(), report.summary.overall_score);
    }

    let mut families: Vec<String> = by_family.keys().cloned().collect();
    families.sort_unstable();

    families
        .into_iter()
        .map(|family| {
            let modes = &by_family[&family];
            let full = modes.get("full").copied();
            let iterative = modes.get("iterative").copied();
            let sandboxed = modes.get("sandboxed").copied();
            let delta = match (full, sandboxed) {
                (Some(f), Some(s)) => Some((s - f * 1000.0).round() / 1000.0 + (s - f)),
                _ => None,
            };
            // Correct delta computation: sandboxed - full (rounded).
            let delta_full_sandboxed = match (full, sandboxed) {
                (Some(f), Some(s)) => Some(((s - f) * 1000.0).round() / 1000.0),
                _ => None,
            };
            let _ = delta; // discard the incorrect first attempt
            FamilyDegradation { family, full, iterative, sandboxed, delta_full_sandboxed }
        })
        .collect()
}

fn build_check_heatmap(reports: &[EvalReport], leaderboard: &[AgentRank]) -> CheckHeatmap {
    // failures[agent_id][check_name] = (fail_count, total_count)
    let mut raw: HashMap<String, HashMap<String, (u32, u32)>> = HashMap::new();

    for report in reports {
        let agent_entry = raw.entry(report.agent_id.clone()).or_default();
        for scenario in &report.scenarios {
            for variation in &scenario.variations {
                for check in &variation.checks {
                    // Skipped checks carry no signal — the agent produced no output.
                    // Including them would make negative checks (e.g. no_price_hallucination)
                    // appear to have a 0% failure rate for broken profiles.
                    if check.skipped {
                        continue;
                    }
                    let entry = agent_entry.entry(check.name.clone()).or_insert((0, 0));
                    entry.1 += 1;
                    if !check.passed {
                        entry.0 += 1;
                    }
                }
            }
        }
    }

    // Collect all unique check names across all agents, sorted.
    let mut checks: Vec<String> = raw
        .values()
        .flat_map(|m| m.keys().cloned())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    checks.sort_unstable();

    let agents: Vec<String> = leaderboard.iter().map(|r| r.agent_id.clone()).collect();

    let failure_rates: HashMap<String, HashMap<String, f32>> = raw
        .into_iter()
        .map(|(agent, check_map)| {
            let rates = check_map
                .into_iter()
                .map(|(check, (fails, total))| {
                    let rate = if total > 0 {
                        (fails as f32 / total as f32 * 1000.0).round() / 1000.0
                    } else {
                        0.0
                    };
                    (check, rate)
                })
                .collect();
            (agent, rates)
        })
        .collect();

    CheckHeatmap { agents, checks, failure_rates }
}

fn build_latency_summary(reports: &[EvalReport], leaderboard: &[AgentRank]) -> Vec<AgentLatency> {
    let mut map: HashMap<String, Vec<u64>> = HashMap::new();

    for report in reports {
        let durations = map.entry(report.agent_id.clone()).or_default();
        for scenario in &report.scenarios {
            for variation in &scenario.variations {
                durations.push(variation.duration_ms);
            }
        }
    }

    leaderboard
        .iter()
        .filter_map(|rank| {
            let mut durations = map.remove(&rank.agent_id)?;
            durations.sort_unstable();
            let n = durations.len();
            if n == 0 {
                return None;
            }
            let p50 = durations[((n as f64 - 1.0) * 0.50).round() as usize];
            let p95 = durations[((n as f64 - 1.0) * 0.95).round() as usize];
            let max = *durations.last().unwrap();
            let mean = (durations.iter().sum::<u64>() as f64 / n as f64).round() as u64;
            Some(AgentLatency {
                agent_id: rank.agent_id.clone(),
                p50_ms: p50,
                p95_ms: p95,
                max_ms: max,
                mean_ms: mean,
            })
        })
        .collect()
}

fn build_variation_matrix(reports: &[EvalReport], leaderboard: &[AgentRank]) -> VariationMatrix {
    // Build ordered slot list from first report (all agents share the same suite).
    let mut slots: Vec<String> = Vec::new();
    if let Some(first) = reports.first() {
        for scenario in &first.scenarios {
            for variation in &scenario.variations {
                slots.push(format!("{}/{}", scenario.scenario_id, variation.variation_id));
            }
        }
    }

    let agents: Vec<String> = leaderboard.iter().map(|r| r.agent_id.clone()).collect();

    let mut data: HashMap<String, HashMap<String, bool>> = HashMap::new();
    for report in reports {
        let agent_map = data.entry(report.agent_id.clone()).or_default();
        for scenario in &report.scenarios {
            for variation in &scenario.variations {
                let key = format!("{}/{}", scenario.scenario_id, variation.variation_id);
                agent_map.insert(key, variation.passed);
            }
        }
    }

    VariationMatrix { agents, slots, data }
}

// ─── Text output ──────────────────────────────────────────────────────────────

impl ComparisonReport {
    /// Print a human-readable comparison summary to stdout.
    pub fn print_text(&self) {
        use colored::Colorize;

        println!();
        println!("{}", "╔══════════════════════════════════════════════════════════╗".bold());
        println!("{}", "║         GESTURA EVAL — COMPARISON REPORT                 ║".bold());
        println!("{}", "╚══════════════════════════════════════════════════════════╝".bold());
        println!("  Run ID  : {}", self.run_id);
        println!("  Time    : {}", self.timestamp.format("%Y-%m-%d %H:%M:%S UTC"));
        println!("  Agents  : {}", self.leaderboard.len());
        println!();

        self.print_leaderboard();
        self.print_category_matrix();
        self.print_profile_degradation();
        self.print_latency_summary();
    }

    fn print_leaderboard(&self) {
        use colored::Colorize;

        println!("{}", "  LEADERBOARD".bold().underline());
        println!();
        println!(
            "  {:<4} {:<30} {:<14} {:>8}   {:>10}",
            "Rank", "Agent ID", "Mode", "Score", "Variations"
        );
        println!("  {}", "─".repeat(72));

        for rank in &self.leaderboard {
            let score_pct = rank.overall_score * 100.0;
            let score_str = format!("{:.1}%", score_pct);
            let colored_score = if score_pct >= 85.0 {
                score_str.green().to_string()
            } else if score_pct >= 60.0 {
                score_str.yellow().to_string()
            } else {
                score_str.red().to_string()
            };

            println!(
                "  #{:<3} {:<30} {:<14} {:>8}   {}/{}",
                rank.rank,
                rank.agent_id,
                rank.agent_mode,
                colored_score,
                rank.passed_variations,
                rank.total_variations,
            );
        }
        println!();
    }

    fn print_category_matrix(&self) {
        use colored::Colorize;

        let matrix = &self.category_matrix;
        if matrix.categories.is_empty() {
            return;
        }

        println!("{}", "  CATEGORY MATRIX  (mean score per category)".bold().underline());
        println!();

        // Column widths
        let agent_col = 30usize;
        let score_col = 7usize;

        // Header
        print!("  {:<width$}", "Agent", width = agent_col);
        for cat in &matrix.categories {
            // Abbreviate category names to fit
            let abbrev = abbreviate_category(cat);
            print!(" {:>width$}", abbrev, width = score_col);
        }
        println!(" {:>width$}", "MEAN", width = score_col);
        print!("  {}", "─".repeat(agent_col));
        for _ in &matrix.categories {
            print!("{}", "─".repeat(score_col + 1));
        }
        println!("{}", "─".repeat(score_col + 1));

        for agent_id in &matrix.agents {
            print!("  {:<width$}", agent_id, width = agent_col);
            for cat in &matrix.categories {
                let score = matrix.scores
                    .get(agent_id)
                    .and_then(|m| m.get(cat))
                    .copied();
                let cell = match score {
                    Some(s) => {
                        let pct = s * 100.0;
                        let text = format!("{:.0}%", pct);
                        if pct >= 85.0 {
                            text.green().to_string()
                        } else if pct >= 60.0 {
                            text.yellow().to_string()
                        } else {
                            text.red().to_string()
                        }
                    }
                    None => "  -  ".dimmed().to_string(),
                };
                // Pad to score_col width (ANSI codes don't count toward width)
                print!(" {:>width$}", cell, width = score_col);
            }

            // Mean across all categories for this agent
            let scores: Vec<f32> = matrix.categories
                .iter()
                .filter_map(|c| matrix.scores.get(agent_id).and_then(|m| m.get(c)))
                .copied()
                .collect();
            if !scores.is_empty() {
                let mean = scores.iter().sum::<f32>() / scores.len() as f32;
                let mean_str = format!("{:.0}%", mean * 100.0);
                let colored_mean = if mean >= 0.85 {
                    mean_str.green().bold().to_string()
                } else if mean >= 0.60 {
                    mean_str.yellow().bold().to_string()
                } else {
                    mean_str.red().bold().to_string()
                };
                print!(" {:>width$}", colored_mean, width = score_col);
            }
            println!();
        }
        println!();
    }

    fn print_profile_degradation(&self) {
        use colored::Colorize;

        if self.profile_degradation.is_empty() {
            return;
        }

        println!("{}", "  PROFILE DEGRADATION  (full → sandboxed quality loss)".bold().underline());
        println!();
        println!(
            "  {:<18} {:>9} {:>10} {:>10} {:>12}",
            "Family", "Full", "Iterative", "Sandboxed", "Δ(sand-full)"
        );
        println!("  {}", "─".repeat(64));

        for d in &self.profile_degradation {
            let fmt_score = |s: Option<f32>| match s {
                Some(v) => {
                    let pct = v * 100.0;
                    let t = format!("{:.1}%", pct);
                    if pct >= 85.0 { t.green().to_string() }
                    else if pct >= 60.0 { t.yellow().to_string() }
                    else { t.red().to_string() }
                }
                None => "   -   ".dimmed().to_string(),
            };

            let delta_str = match d.delta_full_sandboxed {
                Some(v) => {
                    let pct = v * 100.0;
                    let t = format!("{:+.1}%", pct);
                    if pct >= 0.0 { t.green().to_string() } else { t.red().to_string() }
                }
                None => "   -   ".dimmed().to_string(),
            };

            println!(
                "  {:<18} {:>9} {:>10} {:>10} {:>12}",
                d.family,
                fmt_score(d.full),
                fmt_score(d.iterative),
                fmt_score(d.sandboxed),
                delta_str,
            );
        }
        println!();
    }

    fn print_latency_summary(&self) {
        use colored::Colorize;

        if self.latency_summary.is_empty() {
            return;
        }

        println!("{}", "  LATENCY  (subprocess wall-clock time per variation)".bold().underline());
        println!();
        println!(
            "  {:<30} {:>8} {:>8} {:>8} {:>8}",
            "Agent ID", "p50 ms", "p95 ms", "max ms", "mean ms"
        );
        println!("  {}", "─".repeat(66));

        for lat in &self.latency_summary {
            println!(
                "  {:<30} {:>8} {:>8} {:>8} {:>8}",
                lat.agent_id,
                lat.p50_ms,
                lat.p95_ms,
                lat.max_ms,
                lat.mean_ms,
            );
        }
        println!();
    }

    /// Serialize the comparison report to pretty-printed JSON on stdout.
    pub fn print_json(&self) {
        // Omit full agent responses to keep JSON manageable.
        // Callers that need per-agent reports should read the per-agent files.
        let slim = ComparisonReportSlim::from(self);
        println!(
            "{}",
            serde_json::to_string_pretty(&slim)
                .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
        );
    }
}

// ─── Slim serialisation (no agent_reports bloat) ──────────────────────────────

/// Serialisable version without full agent responses (used for JSON stdout).
#[derive(Serialize)]
struct ComparisonReportSlim<'a> {
    run_id: &'a str,
    timestamp: &'a DateTime<Utc>,
    leaderboard: &'a Vec<AgentRank>,
    category_matrix: &'a CategoryMatrix,
    profile_degradation: &'a Vec<FamilyDegradation>,
    check_heatmap: &'a CheckHeatmap,
    latency_summary: &'a Vec<AgentLatency>,
    variation_matrix: &'a VariationMatrix,
}

impl<'a> From<&'a ComparisonReport> for ComparisonReportSlim<'a> {
    fn from(r: &'a ComparisonReport) -> Self {
        Self {
            run_id: &r.run_id,
            timestamp: &r.timestamp,
            leaderboard: &r.leaderboard,
            category_matrix: &r.category_matrix,
            profile_degradation: &r.profile_degradation,
            check_heatmap: &r.check_heatmap,
            latency_summary: &r.latency_summary,
            variation_matrix: &r.variation_matrix,
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Shorten a category name to ≤ 7 chars for table headers.
fn abbreviate_category(cat: &str) -> &str {
    match cat {
        "simple_query"       => "smpl_q",
        "multi_turn"         => "multi",
        "planning"           => "plan",
        "error_handling"     => "err_h",
        "tool_extensibility" => "tools",
        "privacy"            => "priv",
        "context_retention"  => "ctx",
        "long_context"       => "long",
        other                => other,
    }
}
