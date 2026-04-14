//! Self-contained HTML report generator.
//!
//! [`generate`] renders a single `.html` file with no external dependencies
//! other than Chart.js loaded from CDN.  Open it in any browser, share as a
//! CI artefact, or drop into a GitHub PR comment.
//!
//! Eight tab panels:
//!
//! ① Overall Leaderboard  — horizontal bar, agents ranked by overall score
//! ② Category Heatmap     — CSS table, agent × category, colour-coded 0→100%
//! ③ Profile Degradation  — grouped bar, quality loss across permission modes
//! ④ Strength Radar       — radar chart, per-category capability fingerprint
//! ⑤ Check Failure Map    — CSS table, agent × check, failure-rate colour
//! ⑥ Latency Comparison   — grouped bar, p50 + p95 per agent
//! ⑦ Variation Matrix     — CSS grid, pass/fail per agent × variation slot
//! ⑧ Response Review      — full prompt + response per variation, agent toggles

use std::collections::HashMap;

use crate::comparison::ComparisonReport;
use crate::report::VariationResult;

/// Generate a self-contained HTML report from a [`ComparisonReport`].
pub fn generate(report: &ComparisonReport) -> String {
    let data_json      = build_embedded_json(report);
    let meta           = build_meta(report);
    let category_table = build_category_heatmap(report);
    let check_table    = build_check_heatmap(report);
    let variation_grid = build_variation_matrix(report);
    let chart_init     = build_chart_init(report);
    let review_panel   = build_review_panel(report);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Gestura Eval — Comparison Report</title>
<script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.0/dist/chart.umd.min.js"></script>
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
body{{font-family:'SF Mono',ui-monospace,monospace;background:#0d1117;color:#e6edf3;font-size:13px;line-height:1.5}}
header{{padding:1.25rem 2rem;border-bottom:1px solid #21262d;background:#161b22}}
h1{{font-size:1.15rem;font-weight:600;color:#58a6ff;letter-spacing:.01em}}
.meta{{color:#7d8590;font-size:.8rem;margin-top:.3rem}}
.meta span{{margin-right:1.2rem}}
nav{{display:flex;gap:3px;padding:.75rem 2rem;border-bottom:1px solid #21262d;background:#161b22;overflow-x:auto}}
.tab{{background:transparent;border:1px solid transparent;color:#7d8590;padding:.3rem .75rem;cursor:pointer;border-radius:6px;font-family:inherit;font-size:.8rem;white-space:nowrap;transition:all .15s}}
.tab:hover{{color:#e6edf3;border-color:#30363d}}
.tab.active{{background:#21262d;color:#e6edf3;border-color:#58a6ff}}
.panel{{display:none;padding:1.5rem 2rem;max-width:1200px;margin:0 auto}}
.panel.active{{display:block}}
.chart-wrap{{background:#161b22;border:1px solid #21262d;border-radius:8px;padding:1.25rem;margin-bottom:1rem}}
canvas{{max-height:420px}}
h2{{font-size:.9rem;font-weight:600;color:#58a6ff;margin-bottom:.9rem;letter-spacing:.04em;text-transform:uppercase}}

/* Heatmap tables */
.heat{{border-collapse:collapse;width:100%;font-size:.75rem}}
.heat th{{background:#21262d;padding:5px 8px;text-align:center;white-space:nowrap;color:#8b949e;font-weight:600;position:sticky;top:0;z-index:1}}
.heat th.left{{text-align:left}}
.heat td{{padding:4px 8px;text-align:center;border:1px solid #21262d}}
.heat td.agent-name{{text-align:left;font-weight:600;color:#79c0ff;white-space:nowrap;background:#161b22}}
.heat-wrap{{overflow-x:auto}}

/* Variation matrix */
.var-wrap{{overflow-x:auto}}
.var-table{{border-collapse:collapse;font-size:.7rem}}
.var-table th{{background:#21262d;padding:4px 6px;color:#8b949e;white-space:nowrap;font-weight:600}}
.var-table th.left{{text-align:left}}
.var-table td{{padding:2px 4px;text-align:center;border:1px solid #21262d}}
.var-table td.agent-name{{text-align:left;font-weight:600;color:#79c0ff;white-space:nowrap;background:#161b22}}
.dot{{width:16px;height:16px;border-radius:3px;display:inline-block}}
.dot.pass{{background:#238636}}
.dot.fail{{background:#da3633}}
.dot.na{{background:#30363d}}

/* ─── Review panel ─────────────────────────────────────────────────────────── */
.review-controls{{display:flex;align-items:center;flex-wrap:wrap;gap:6px;padding:.6rem .9rem;background:#161b22;border:1px solid #21262d;border-radius:8px;margin-bottom:1rem}}
.review-label{{color:#7d8590;font-size:.75rem;margin-right:2px;white-space:nowrap}}
.toggle-all{{background:#21262d;border:1px solid #30363d;color:#8b949e;padding:3px 9px;border-radius:4px;cursor:pointer;font-family:inherit;font-size:.72rem;transition:all .12s}}
.toggle-all:hover{{border-color:#484f58;color:#e6edf3}}
.ctrl-divider{{color:#30363d;margin:0 4px}}
.agent-pill{{background:#0d1117;border:1px solid #30363d;color:#7d8590;padding:3px 10px;border-radius:12px;cursor:pointer;font-family:inherit;font-size:.72rem;transition:all .12s;white-space:nowrap}}
.agent-pill:hover{{border-color:#484f58;color:#c9d1d9}}
.agent-pill.active{{background:#1c2d3e;border-color:#58a6ff;color:#79c0ff}}
/* Scenario accordion */
.review-scenario{{border:1px solid #21262d;border-radius:6px;overflow:hidden;margin-bottom:.6rem}}
.review-scen-hdr{{display:flex;align-items:center;gap:.6rem;padding:.55rem .9rem;background:#161b22;cursor:pointer;user-select:none;transition:background .1s}}
.review-scen-hdr:hover{{background:#1c2128}}
.scen-toggle{{color:#484f58;font-size:.7rem;width:10px;flex-shrink:0}}
.scen-id{{color:#79c0ff;font-weight:700;font-size:.78rem;min-width:130px}}
.scen-title{{color:#c9d1d9;flex:1;font-size:.82rem}}
.cat-pill{{background:#21262d;border:1px solid #30363d;color:#8b949e;padding:1px 7px;border-radius:10px;font-size:.68rem;white-space:nowrap}}
.scen-pass-summary{{color:#484f58;font-size:.68rem;white-space:nowrap;margin-left:.5rem}}
.review-scen-body{{display:none;padding:.75rem;background:#0d1117;border-top:1px solid #21262d}}
/* Variation blocks */
.review-var{{margin-bottom:1rem}}
.review-var:last-child{{margin-bottom:0}}
.var-prompt{{display:flex;align-items:baseline;gap:.5rem;padding:.45rem .7rem;background:#161b22;border:1px solid #21262d;border-radius:5px;margin-bottom:.55rem;font-size:.8rem}}
.var-label{{color:#58a6ff;font-weight:700;flex-shrink:0}}
.var-prompt-text{{color:#c9d1d9;line-height:1.45}}
/* Response card grid */
.response-grid{{display:flex;flex-wrap:wrap;gap:.55rem;align-items:flex-start}}
.agent-card{{flex:1 1 280px;min-width:240px;max-width:520px;border-radius:6px;border:1px solid #30363d;overflow:hidden;transition:border-color .15s}}
.agent-card.r-pass{{border-color:#1a4731}}
.agent-card.r-fail{{border-color:#4d1f1f}}
.agent-card.r-na{{border-color:#21262d;opacity:.6}}
.agent-card-hdr{{display:flex;justify-content:space-between;align-items:center;padding:5px 9px;background:#21262d;gap:.5rem}}
.card-agent-id{{color:#79c0ff;font-weight:700;font-size:.75rem}}
.card-meta{{display:flex;align-items:center;gap:.4rem}}
.card-score{{font-weight:700;font-size:.75rem}}
.card-score.s-pass{{color:#3fb950}}
.card-score.s-fail{{color:#f85149}}
.card-score.s-na{{color:#7d8590}}
.card-dur{{color:#7d8590;font-size:.68rem}}
.response-body{{padding:8px 10px;background:#0d1117;max-height:240px;overflow-y:auto;white-space:pre-wrap;word-break:break-word;font-size:.76rem;line-height:1.55;color:#c9d1d9}}
.response-empty{{color:#484f58;font-style:italic}}
.response-error{{color:#f85149;font-size:.72rem;padding:6px 10px;background:#2d1a1a;border-top:1px solid #4d1f1f}}
/* Check details */
.checks-bar{{padding:5px 9px;background:#161b22;border-top:1px solid #21262d}}
.checks-toggle{{background:none;border:none;color:#7d8590;font-family:inherit;font-size:.7rem;cursor:pointer;padding:0;display:flex;align-items:center;gap:4px}}
.checks-toggle:hover{{color:#c9d1d9}}
.checks-list{{margin-top:5px;display:none}}
.check-row{{display:flex;gap:6px;font-size:.69rem;padding:2px 0;line-height:1.4}}
.check-row.ck-pass{{color:#3fb950}}
.check-row.ck-fail{{color:#f85149}}
.ck-name{{font-weight:600;min-width:180px;flex-shrink:0}}
.ck-detail{{color:#7d8590}}
.check-row.ck-fail .ck-detail{{color:#e06c75}}
/* Pipeline error badge */
.pipe-error{{padding:4px 9px;background:#2d1a1a;border-top:1px solid #4d1f1f;font-size:.7rem;color:#f85149}}
</style>
</head>
<body>
<header>
  <h1>Gestura Eval — Comparison Report</h1>
  <div class="meta">{meta}</div>
</header>
<nav>
  <button class="tab active" onclick="showTab('leaderboard',this)">① Leaderboard</button>
  <button class="tab" onclick="showTab('category',this)">② Category Heatmap</button>
  <button class="tab" onclick="showTab('degradation',this)">③ Degradation</button>
  <button class="tab" onclick="showTab('radar',this)">④ Radar</button>
  <button class="tab" onclick="showTab('checks',this)">⑤ Check Failures</button>
  <button class="tab" onclick="showTab('latency',this)">⑥ Latency</button>
  <button class="tab" onclick="showTab('matrix',this)">⑦ Variation Matrix</button>
  <button class="tab" onclick="showTab('review',this)">⑧ Response Review</button>
</nav>

<!-- ① Leaderboard -->
<div id="tab-leaderboard" class="panel active">
  <div class="chart-wrap">
    <h2>Overall Leaderboard</h2>
    <canvas id="chart-leaderboard"></canvas>
  </div>
</div>

<!-- ② Category Heatmap -->
<div id="tab-category" class="panel">
  <div class="chart-wrap">
    <h2>Category Score Heatmap</h2>
    <div class="heat-wrap">{category_table}</div>
  </div>
</div>

<!-- ③ Profile Degradation -->
<div id="tab-degradation" class="panel">
  <div class="chart-wrap">
    <h2>Profile Degradation — Quality Loss by Permission Mode</h2>
    <canvas id="chart-degradation"></canvas>
  </div>
</div>

<!-- ④ Strength Radar -->
<div id="tab-radar" class="panel">
  <div class="chart-wrap">
    <h2>Capability Radar — Per-Category Strength</h2>
    <canvas id="chart-radar"></canvas>
  </div>
</div>

<!-- ⑤ Check Failure Map -->
<div id="tab-checks" class="panel">
  <div class="chart-wrap">
    <h2>Check Failure Heatmap</h2>
    <div class="heat-wrap">{check_table}</div>
  </div>
</div>

<!-- ⑥ Latency -->
<div id="tab-latency" class="panel">
  <div class="chart-wrap">
    <h2>Latency Comparison (per-variation wall-clock)</h2>
    <canvas id="chart-latency"></canvas>
  </div>
</div>

<!-- ⑦ Variation Matrix -->
<div id="tab-matrix" class="panel">
  <div class="chart-wrap">
    <h2>Variation Pass / Fail Matrix</h2>
    <div class="var-wrap">{variation_grid}</div>
  </div>
</div>

<!-- ⑧ Response Review -->
<div id="tab-review" class="panel">
{review_panel}
</div>

<script>
const DATA = {data_json};

function showTab(name,btn){{
  document.querySelectorAll('.panel').forEach(p=>p.classList.remove('active'));
  document.querySelectorAll('.tab').forEach(b=>b.classList.remove('active'));
  document.getElementById('tab-'+name).classList.add('active');
  btn.classList.add('active');
}}

Chart.defaults.color='#8b949e';
Chart.defaults.borderColor='#21262d';
Chart.defaults.font.family="'SF Mono',ui-monospace,monospace";

function scoreColor(s){{
  const hue=Math.round(s*120);
  return `hsl(${{hue}},70%,${{s>0.5?40:50}}%)`;
}}

const PALETTE=['#58a6ff','#3fb950','#d29922','#f78166','#bc8cff','#56d364','#e3b341','#ff7b72','#79c0ff','#7ee787','#ffa657','#ff6e96','#a5d6ff','#b3f0a9','#ffd27f'];

{chart_init}

/* ── Review panel ─────────────────────────────────────────────────────────── */
function reviewToggle(btn){{
  const agent=btn.dataset.agent;
  const show=btn.classList.toggle('active');
  document.querySelectorAll(`.agent-card[data-agent="${{agent}}"]`)
    .forEach(el=>el.style.display=show?'':'none');
}}

function reviewToggleAll(show){{
  document.querySelectorAll('.agent-pill').forEach(btn=>{{
    if(show) btn.classList.add('active'); else btn.classList.remove('active');
  }});
  document.querySelectorAll('.agent-card').forEach(el=>el.style.display=show?'':'none');
}}

function toggleScenario(hdr){{
  const body=hdr.nextElementSibling;
  const icon=hdr.querySelector('.scen-toggle');
  const open=body.style.display!=='none';
  body.style.display=open?'none':'block';
  icon.textContent=open?'▶':'▼';
}}

function toggleChecks(btn){{
  const list=btn.nextElementSibling;
  const open=list.style.display!=='none';
  list.style.display=open?'none':'block';
  btn.textContent=open?'▶ checks':'▼ checks';
}}
</script>
</body>
</html>"#,
        meta           = meta,
        category_table = category_table,
        check_table    = check_table,
        variation_grid = variation_grid,
        review_panel   = review_panel,
        data_json      = data_json,
        chart_init     = chart_init,
    )
}

// ─── Meta line ────────────────────────────────────────────────────────────────

fn build_meta(report: &ComparisonReport) -> String {
    format!(
        r#"<span>Run ID: {}</span><span>{}</span><span>{} agents</span><span>{} scenarios</span>"#,
        html_escape(&report.run_id),
        report.timestamp.format("%Y-%m-%d %H:%M UTC"),
        report.leaderboard.len(),
        report.agent_reports.first().map(|r| r.scenarios.len()).unwrap_or(0),
    )
}

// ─── Embedded JSON ────────────────────────────────────────────────────────────

fn build_embedded_json(report: &ComparisonReport) -> String {
    let leaderboard: Vec<serde_json::Value> = report.leaderboard.iter().map(|r| {
        serde_json::json!({
            "agent_id": r.agent_id,
            "overall_score": r.overall_score,
            "rank": r.rank,
        })
    }).collect();

    let degradation: Vec<serde_json::Value> = report.profile_degradation.iter().map(|d| {
        serde_json::json!({
            "family": d.family,
            "full": d.full,
            "iterative": d.iterative,
            "sandboxed": d.sandboxed,
            "delta": d.delta_full_sandboxed,
        })
    }).collect();

    let latency: Vec<serde_json::Value> = report.latency_summary.iter().map(|l| {
        serde_json::json!({
            "agent_id": l.agent_id,
            "p50_ms": l.p50_ms,
            "p95_ms": l.p95_ms,
            "mean_ms": l.mean_ms,
        })
    }).collect();

    let categories = &report.category_matrix.categories;
    let agents = &report.category_matrix.agents;
    let category_data: Vec<serde_json::Value> = agents.iter().map(|agent| {
        let scores: Vec<f64> = categories.iter().map(|cat| {
            report.category_matrix.scores
                .get(agent)
                .and_then(|m| m.get(cat))
                .copied()
                .unwrap_or(0.0) as f64 * 100.0
        }).collect();
        serde_json::json!({"agent_id": agent, "scores": scores})
    }).collect();

    serde_json::json!({
        "leaderboard": leaderboard,
        "profile_degradation": degradation,
        "latency_summary": latency,
        "categories": categories,
        "category_data": category_data,
    }).to_string()
}

// ─── Category heatmap table ───────────────────────────────────────────────────

fn build_category_heatmap(report: &ComparisonReport) -> String {
    let matrix = &report.category_matrix;
    if matrix.agents.is_empty() || matrix.categories.is_empty() {
        return "<p>No data.</p>".to_string();
    }

    let mut html = String::from("<table class='heat'><thead><tr>");
    html.push_str("<th class='left'>Agent</th>");
    for cat in &matrix.categories {
        html.push_str(&format!("<th>{}</th>", html_escape(cat)));
    }
    html.push_str("<th>Mean</th></tr></thead><tbody>");

    for agent in &matrix.agents {
        html.push_str("<tr>");
        html.push_str(&format!("<td class='agent-name'>{}</td>", html_escape(agent)));

        let mut sum = 0.0f32;
        let mut count = 0u32;

        for cat in &matrix.categories {
            let score = matrix.scores.get(agent).and_then(|m| m.get(cat)).copied();
            let (cell, s) = match score {
                Some(s) => (format!("{:.0}%", s * 100.0), s),
                None => ("–".to_string(), 0.0),
            };
            if score.is_some() {
                sum += s;
                count += 1;
            }
            let bg = score_bg_css(score.unwrap_or(0.0), score.is_none());
            html.push_str(&format!("<td style='background:{bg};'>{cell}</td>"));
        }

        let mean_cell = if count > 0 {
            let m = sum / count as f32;
            let bg = score_bg_css(m, false);
            format!("<td style='background:{bg};font-weight:700'>{:.0}%</td>", m * 100.0)
        } else {
            "<td>–</td>".to_string()
        };
        html.push_str(&mean_cell);
        html.push_str("</tr>");
    }

    html.push_str("</tbody></table>");
    html
}

// ─── Check failure heatmap table ──────────────────────────────────────────────

fn build_check_heatmap(report: &ComparisonReport) -> String {
    let hm = &report.check_heatmap;
    if hm.agents.is_empty() || hm.checks.is_empty() {
        return "<p>No data.</p>".to_string();
    }

    let mut html = String::from("<table class='heat'><thead><tr>");
    html.push_str("<th class='left'>Agent</th>");
    for check in &hm.checks {
        html.push_str(&format!("<th>{}</th>", html_escape(check)));
    }
    html.push_str("</tr></thead><tbody>");

    for agent in &hm.agents {
        html.push_str("<tr>");
        html.push_str(&format!("<td class='agent-name'>{}</td>", html_escape(agent)));
        for check in &hm.checks {
            let rate = hm.failure_rates.get(agent).and_then(|m| m.get(check)).copied();
            let (cell, bg) = match rate {
                Some(r) => (format!("{:.0}%", r * 100.0), failure_rate_bg_css(r)),
                None => ("–".to_string(), "transparent".to_string()),
            };
            html.push_str(&format!("<td style='background:{bg};'>{cell}</td>"));
        }
        html.push_str("</tr>");
    }

    html.push_str("</tbody></table>");
    html
}

// ─── Variation matrix ─────────────────────────────────────────────────────────

fn build_variation_matrix(report: &ComparisonReport) -> String {
    let vm = &report.variation_matrix;
    if vm.agents.is_empty() || vm.slots.is_empty() {
        return "<p>No data.</p>".to_string();
    }

    let mut html = String::from("<table class='var-table'><thead><tr>");
    html.push_str("<th class='left'>Agent</th>");
    for slot in &vm.slots {
        let abbrev = abbreviate_slot(slot);
        html.push_str(&format!("<th title='{}'>{}</th>", html_escape(slot), html_escape(&abbrev)));
    }
    html.push_str("<th>%</th></tr></thead><tbody>");

    for agent in &vm.agents {
        html.push_str("<tr>");
        html.push_str(&format!("<td class='agent-name'>{}</td>", html_escape(agent)));

        let mut pass_count = 0u32;
        let total = vm.slots.len() as u32;

        for slot in &vm.slots {
            let passed = vm.data.get(agent).and_then(|m| m.get(slot)).copied();
            match passed {
                Some(true) => {
                    pass_count += 1;
                    html.push_str("<td><span class='dot pass' title='pass'></span></td>");
                }
                Some(false) => {
                    html.push_str("<td><span class='dot fail' title='fail'></span></td>");
                }
                None => {
                    html.push_str("<td><span class='dot na' title='n/a'></span></td>");
                }
            }
        }

        let pct = if total > 0 { pass_count * 100 / total } else { 0 };
        let bg = score_bg_css(pct as f32 / 100.0, false);
        html.push_str(&format!("<td style='background:{bg};font-weight:700'>{pct}%</td>"));
        html.push_str("</tr>");
    }

    html.push_str("</tbody></table>");
    html
}

// ─── Response review panel ────────────────────────────────────────────────────

fn build_review_panel(report: &ComparisonReport) -> String {
    if report.agent_reports.is_empty() {
        return "<p style='color:#7d8590;padding:1rem'>No agent reports available.</p>".to_string();
    }

    // Build lookup: agent_id → scenario_id → variation_id → &VariationResult
    let mut lookup: HashMap<&str, HashMap<&str, HashMap<&str, &VariationResult>>> = HashMap::new();
    for agent_report in &report.agent_reports {
        let agent_map = lookup.entry(agent_report.agent_id.as_str()).or_default();
        for scenario in &agent_report.scenarios {
            let scen_map = agent_map.entry(scenario.scenario_id.as_str()).or_default();
            for variation in &scenario.variations {
                scen_map.insert(variation.variation_id.as_str(), variation);
            }
        }
    }

    // Ordered agent list from leaderboard (rank order, best first).
    let agents: Vec<&str> = report.leaderboard.iter().map(|r| r.agent_id.as_str()).collect();

    // Scenario/variation structure from first agent report.
    let first = &report.agent_reports[0];

    let mut out = String::new();

    // ── Agent filter bar ────────────────────────────────────────────────────
    out.push_str("<div class='review-controls'>");
    out.push_str("<span class='review-label'>Show:</span>");
    out.push_str("<button class='toggle-all' onclick='reviewToggleAll(true)'>All</button>");
    out.push_str("<button class='toggle-all' onclick='reviewToggleAll(false)'>None</button>");
    out.push_str("<span class='ctrl-divider'>|</span>");

    for agent in &agents {
        // Default: show full-permission agents; hide sandboxed / iterative.
        // The user can toggle any on/off from here.
        let is_default_visible = agent.ends_with("-full");
        let active = if is_default_visible { " active" } else { "" };
        out.push_str(&format!(
            "<button class='agent-pill{active}' data-agent='{ae}' onclick='reviewToggle(this)'>{ae}</button>",
            ae = html_escape(agent),
        ));
    }
    out.push_str("</div>");

    // ── Scenario accordions ──────────────────────────────────────────────────
    for scenario in &first.scenarios {
        // Per-agent pass counts for this scenario (shown in header).
        let pass_summary: Vec<String> = agents.iter().map(|a| {
            let passed = lookup.get(*a)
                .and_then(|s| s.get(scenario.scenario_id.as_str()))
                .map(|vars| vars.values().filter(|v| v.passed).count())
                .unwrap_or(0);
            let total = scenario.variations.len();
            format!("{passed}/{total}")
        }).collect();
        let summary_str = agents.iter().zip(pass_summary.iter())
            .map(|(a, s)| format!("{}: {}", short_agent_id(a), s))
            .collect::<Vec<_>>()
            .join("  ");

        out.push_str("<div class='review-scenario'>");
        out.push_str(&format!(
            "<div class='review-scen-hdr' onclick='toggleScenario(this)'>\
               <span class='scen-toggle'>▶</span>\
               <span class='scen-id'>{sid}</span>\
               <span class='scen-title'>{name}</span>\
               <span class='cat-pill'>{cat}</span>\
               <span class='scen-pass-summary'>{summary}</span>\
             </div>",
            sid     = html_escape(&scenario.scenario_id),
            name    = html_escape(&scenario.scenario_name),
            cat     = html_escape(&scenario.category),
            summary = html_escape(&summary_str),
        ));

        out.push_str("<div class='review-scen-body'>");

        for variation in &scenario.variations {
            out.push_str("<div class='review-var'>");

            // Prompt header
            out.push_str(&format!(
                "<div class='var-prompt'>\
                   <span class='var-label'>{vid}</span>\
                   <span class='var-prompt-text'>{prompt}</span>\
                 </div>",
                vid    = html_escape(&variation.variation_id),
                prompt = html_escape(&variation.prompt_preview),
            ));

            out.push_str("<div class='response-grid'>");

            for agent in &agents {
                let var_result = lookup.get(*agent)
                    .and_then(|s| s.get(scenario.scenario_id.as_str()))
                    .and_then(|v| v.get(variation.variation_id.as_str()));

                // Card visibility: only -full agents shown by default.
                let hidden = if agent.ends_with("-full") { "" } else { " style='display:none'" };

                match var_result {
                    Some(vr) => {
                        let (pass_cls, score_cls) = if vr.passed {
                            ("r-pass", "s-pass")
                        } else {
                            ("r-fail", "s-fail")
                        };
                        let score_pct = format!("{:.0}%", vr.score * 100.0);
                        let dur = if vr.duration_ms > 0 {
                            format!("{}ms", vr.duration_ms)
                        } else {
                            String::new()
                        };

                        out.push_str(&format!(
                            "<div class='agent-card {pass_cls}' data-agent='{ae}'{hidden}>",
                            ae = html_escape(agent),
                        ));

                        // Card header
                        out.push_str(&format!(
                            "<div class='agent-card-hdr'>\
                               <span class='card-agent-id'>{ae}</span>\
                               <span class='card-meta'>\
                                 <span class='card-score {score_cls}'>{score_pct}</span>\
                                 <span class='card-dur'>{dur}</span>\
                               </span>\
                             </div>",
                            ae = html_escape(agent),
                        ));

                        // Pipeline error banner (shown above response if present)
                        if let Some(ref err) = vr.pipeline_error {
                            out.push_str(&format!(
                                "<div class='pipe-error'>⚠ {}</div>",
                                html_escape(err)
                            ));
                        }

                        // Response body
                        if vr.response.trim().is_empty() {
                            out.push_str("<div class='response-body'><span class='response-empty'>empty response</span></div>");
                        } else {
                            out.push_str(&format!(
                                "<div class='response-body'>{}</div>",
                                html_escape(&vr.response)
                            ));
                        }

                        // Check details toggle
                        let failed: Vec<_> = vr.checks.iter().filter(|c| !c.passed).collect();
                        let passed_count = vr.checks.len() - failed.len();
                        let check_label = if failed.is_empty() {
                            format!(
                                "<span style='color:#3fb950'>✓ all {} checks passed</span>",
                                passed_count
                            )
                        } else {
                            format!(
                                "<span style='color:#f85149'>✗ {}/{} checks failed</span>",
                                failed.len(),
                                vr.checks.len()
                            )
                        };

                        out.push_str("<div class='checks-bar'>");
                        out.push_str(&format!(
                            "<button class='checks-toggle' onclick='toggleChecks(this)'>▶ checks &nbsp;{check_label}</button>",
                        ));
                        out.push_str("<div class='checks-list'>");
                        for check in &vr.checks {
                            let (row_cls, icon) = if check.passed { ("ck-pass", "✓") } else { ("ck-fail", "✗") };
                            out.push_str(&format!(
                                "<div class='check-row {row_cls}'>\
                                   <span class='ck-name'>{icon} {name}</span>\
                                   <span class='ck-detail'>{detail}</span>\
                                 </div>",
                                name   = html_escape(&check.name),
                                detail = html_escape(&check.details),
                            ));
                        }
                        out.push_str("</div>"); // checks-list
                        out.push_str("</div>"); // checks-bar

                        out.push_str("</div>"); // agent-card
                    }

                    None => {
                        // No data for this agent/variation (skipped profile, etc.)
                        out.push_str(&format!(
                            "<div class='agent-card r-na' data-agent='{ae}'{hidden}>\
                               <div class='agent-card-hdr'>\
                                 <span class='card-agent-id'>{ae}</span>\
                                 <span class='card-score s-na'>–</span>\
                               </div>\
                               <div class='response-body'><span class='response-empty'>no data</span></div>\
                             </div>",
                            ae = html_escape(agent),
                        ));
                    }
                }
            }

            out.push_str("</div>"); // response-grid
            out.push_str("</div>"); // review-var
        }

        out.push_str("</div>"); // review-scen-body
        out.push_str("</div>"); // review-scenario
    }

    out
}

// ─── Chart.js initialisation code ────────────────────────────────────────────

fn build_chart_init(report: &ComparisonReport) -> String {
    let lb  = &report.leaderboard;
    let deg = &report.profile_degradation;
    let lat = &report.latency_summary;
    let cat_matrix = &report.category_matrix;

    let lb_labels  = js_string_array(lb.iter().map(|r| r.agent_id.as_str()));
    let lb_data: Vec<String> = lb.iter().map(|r| format!("{:.1}", r.overall_score * 100.0)).collect();
    let lb_colors  = js_string_array(lb.iter().map(|r| score_js_color(r.overall_score)));

    let deg_labels = js_string_array(deg.iter().map(|d| d.family.as_str()));
    let deg_full:  Vec<String> = deg.iter().map(|d| opt_f32_js(d.full)).collect();
    let deg_iter:  Vec<String> = deg.iter().map(|d| opt_f32_js(d.iterative)).collect();
    let deg_sand:  Vec<String> = deg.iter().map(|d| opt_f32_js(d.sandboxed)).collect();

    let radar_labels   = js_string_array(cat_matrix.categories.iter().map(|s| s.as_str()));
    let radar_agents: Vec<_> = cat_matrix.agents.iter()
        .filter(|a| a.ends_with("-full"))
        .take(6)
        .collect();
    let radar_datasets = build_radar_datasets(report, &radar_agents);

    let lat_labels = js_string_array(lat.iter().map(|l| l.agent_id.as_str()));
    let lat_p50: Vec<String> = lat.iter().map(|l| l.p50_ms.to_string()).collect();
    let lat_p95: Vec<String> = lat.iter().map(|l| l.p95_ms.to_string()).collect();

    format!(
        r#"new Chart(document.getElementById('chart-leaderboard'),{{
  type:'bar',
  data:{{
    labels:{lb_labels},
    datasets:[{{label:'Overall Score (%)',data:[{lb_data}],backgroundColor:{lb_colors},borderWidth:0}}]
  }},
  options:{{
    indexAxis:'y',responsive:true,
    plugins:{{legend:{{display:false}}}},
    scales:{{x:{{min:0,max:100,ticks:{{callback:v=>v+'%'}}}},y:{{ticks:{{font:{{size:11}}}}}}}}
  }}
}});

new Chart(document.getElementById('chart-degradation'),{{
  type:'bar',
  data:{{
    labels:{deg_labels},
    datasets:[
      {{label:'Full',      data:[{deg_full}],backgroundColor:'#238636',borderWidth:0}},
      {{label:'Iterative', data:[{deg_iter}],backgroundColor:'#9e6a03',borderWidth:0}},
      {{label:'Sandboxed', data:[{deg_sand}],backgroundColor:'#da3633',borderWidth:0}},
    ]
  }},
  options:{{responsive:true,scales:{{y:{{min:0,max:100,ticks:{{callback:v=>v+'%'}}}}}}}}
}});

new Chart(document.getElementById('chart-radar'),{{
  type:'radar',
  data:{{
    labels:{radar_labels},
    datasets:[{radar_datasets}]
  }},
  options:{{
    responsive:true,
    scales:{{r:{{min:0,max:100,ticks:{{stepSize:25,callback:v=>v+'%'}}}}}},
    plugins:{{legend:{{position:'bottom'}}}}
  }}
}});

new Chart(document.getElementById('chart-latency'),{{
  type:'bar',
  data:{{
    labels:{lat_labels},
    datasets:[
      {{label:'p50 ms',data:[{lat_p50}],backgroundColor:'#1f6feb',borderWidth:0}},
      {{label:'p95 ms',data:[{lat_p95}],backgroundColor:'#6e40c9',borderWidth:0}},
    ]
  }},
  options:{{responsive:true,scales:{{y:{{beginAtZero:true}}}}}}
}});"#,
        lb_labels      = lb_labels,
        lb_data        = lb_data.join(","),
        lb_colors      = lb_colors,
        deg_labels     = deg_labels,
        deg_full       = deg_full.join(","),
        deg_iter       = deg_iter.join(","),
        deg_sand       = deg_sand.join(","),
        radar_labels   = radar_labels,
        radar_datasets = radar_datasets,
        lat_labels     = lat_labels,
        lat_p50        = lat_p50.join(","),
        lat_p95        = lat_p95.join(","),
    )
}

fn build_radar_datasets(report: &ComparisonReport, agents: &[&String]) -> String {
    let categories = &report.category_matrix.categories;
    agents.iter().enumerate().map(|(i, agent)| {
        let scores: Vec<String> = categories.iter().map(|cat| {
            let s = report.category_matrix.scores
                .get(*agent)
                .and_then(|m| m.get(cat))
                .copied()
                .unwrap_or(0.0);
            format!("{:.1}", s * 100.0)
        }).collect();
        let color = PALETTE_JS[i % PALETTE_JS.len()];
        format!(
            "{{label:{},data:[{}],borderColor:'{}',backgroundColor:'{}33',pointBackgroundColor:'{}',fill:true}}",
            js_string(agent),
            scores.join(","),
            color, color, color,
        )
    }).collect::<Vec<_>>().join(",")
}

// ─── CSS colour helpers ───────────────────────────────────────────────────────

fn score_bg_css(score: f32, empty: bool) -> String {
    if empty { return "transparent".to_string(); }
    let hue = (score * 120.0).round() as u32;
    let l   = if score > 0.5 { 20 } else { 25 };
    format!("hsl({hue},60%,{l}%)")
}

fn failure_rate_bg_css(rate: f32) -> String {
    let hue = ((1.0 - rate) * 120.0).round() as u32;
    let l   = if rate < 0.5 { 20 } else { 25 };
    format!("hsl({hue},60%,{l}%)")
}

fn score_js_color(score: f32) -> &'static str {
    if score >= 0.85 { "#238636" } else if score >= 0.70 { "#9e6a03" } else { "#da3633" }
}

// ─── JS helpers ───────────────────────────────────────────────────────────────

static PALETTE_JS: &[&str] = &[
    "#58a6ff","#3fb950","#d29922","#f78166","#bc8cff",
    "#56d364","#e3b341","#ff7b72","#79c0ff","#7ee787",
];

fn js_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "\\'"))
}

fn js_string_array<'a, I: Iterator<Item = &'a str>>(items: I) -> String {
    let inner: Vec<String> = items.map(|s| js_string(s)).collect();
    format!("[{}]", inner.join(","))
}

fn opt_f32_js(v: Option<f32>) -> String {
    match v {
        Some(f) => format!("{:.1}", f * 100.0),
        None    => "null".to_string(),
    }
}

// ─── Misc helpers ─────────────────────────────────────────────────────────────

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

fn abbreviate_slot(slot: &str) -> String {
    let parts: Vec<&str> = slot.splitn(2, '/').collect();
    match parts.as_slice() {
        [scenario, variation] => {
            let short = scenario.splitn(2, '_').next().unwrap_or(scenario);
            format!("{short}/{variation}")
        }
        _ => slot.to_string(),
    }
}

/// Very short agent label for places where full ID is too wide.
/// `"gestura-full"` → `"g-full"`, `"claude-code-full"` → `"cc-full"`
fn short_agent_id(id: &str) -> String {
    let abbrevs = [
        ("gestura-",     "g-"),
        ("claude-code-", "cc-"),
        ("augment-",     "aug-"),
        ("codex-",       "cx-"),
        ("opencode-",    "oc-"),
    ];
    for (prefix, short) in abbrevs {
        if let Some(rest) = id.strip_prefix(prefix) {
            return format!("{short}{rest}");
        }
    }
    id.to_string()
}
