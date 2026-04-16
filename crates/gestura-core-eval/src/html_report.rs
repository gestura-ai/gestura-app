//! Self-contained HTML report generator.
//!
//! [`generate`] renders a single `.html` file with no external dependencies
//! other than Chart.js loaded from CDN.  Open it in any browser, share as a
//! CI artefact, or drop into a GitHub PR comment.
//!
//! Ten tab panels:
//!
//! ⓪ About               → intro, methodology, scenario reference, charts guide
//! ① Overall Leaderboard  — horizontal bar, agents ranked by overall score
//! ② Category Heatmap     — CSS table, agent × category, colour-coded 0→100%
//! ③ Profile Degradation  — grouped bar, quality loss across permission modes
//! ④ Strength Radar       — radar chart, per-category capability fingerprint
//! ⑤ Check Failure Map    — CSS table, agent × check, failure-rate colour
//! ⑥ Latency Comparison   — grouped bar, p50 + p95 per agent
//! ⑦ Variation Matrix     — CSS grid, pass/fail per agent × variation slot
//! ⑧ Response Review      — full prompt + response per variation, agent toggles
//! ⑨ Head to Head         — side-by-side response comparison for any two agents

use std::collections::HashMap;

use crate::comparison::ComparisonReport;
use crate::report::VariationResult;

/// Generate a self-contained HTML report from a [`ComparisonReport`].
pub fn generate(report: &ComparisonReport) -> String {
    let data_json          = build_embedded_json(report);
    let category_table     = build_category_heatmap(report);
    let check_table        = build_check_heatmap(report);
    let variation_grid     = build_variation_matrix(report);
    let chart_init         = build_chart_init(report);
    let review_panel       = build_review_panel(report);
    let about_page         = build_about_page(report);
    let models_cost_panel  = build_models_cost_panel(report);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Gestura Agent Evaluation</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700&display=swap" rel="stylesheet">
<script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.0/dist/chart.umd.min.js"></script>
<style>
/* ─── Reset & tokens ──────────────────────────────────────────────────── */
*{{box-sizing:border-box;margin:0;padding:0}}
:root{{
  --bg:#f8fafc;
  --surface:#ffffff;
  --surface2:#f1f5f9;
  --border:rgba(0,0,0,0.07);
  --border-s:#e2e8f0;
  --text:#1e293b;
  --muted:#64748b;
  --dim:#475569;
}}
body{{font-family:'Inter',system-ui,sans-serif;background:var(--bg);color:var(--text);font-size:14px;line-height:1.6;min-height:100vh;display:flex;flex-direction:column}}
.main-wrap{{flex:1}}

/* ─── Navbar ──────────────────────────────────────────────────────────── */
.navbar{{
  position:sticky;top:0;z-index:200;width:100%;
  border-bottom:1px solid var(--border-s);
  background:rgba(255,255,255,0.92);
  backdrop-filter:blur(14px);-webkit-backdrop-filter:blur(14px);
  box-shadow:0 1px 0 rgba(0,0,0,.04);
}}
.nbi{{
  max-width:1280px;margin:0 auto;padding:0 1.5rem;
  display:flex;align-items:center;justify-content:space-between;
  height:60px;gap:1rem;
}}
.brand{{display:flex;align-items:center;gap:10px;flex-shrink:0}}
.brand-logo{{
  width:34px;height:34px;border-radius:8px;object-fit:cover;
  box-shadow:0 1px 6px rgba(0,0,0,.12);flex-shrink:0;
}}
.brand-titles{{display:flex;flex-direction:column;line-height:1.15}}
.brand-name{{font-size:.95rem;font-weight:700;color:#000;letter-spacing:-.015em}}
.brand-sub{{font-size:.67rem;color:var(--muted);font-weight:400;letter-spacing:.01em}}

/* desktop nav */
.nav-links{{display:flex;align-items:center;gap:2px;margin-left:auto}}
.nb{{
  background:transparent;border:none;cursor:pointer;
  color:var(--dim);font-family:'Inter',sans-serif;font-size:.83rem;font-weight:500;
  padding:.38rem .8rem;border-radius:7px;
  transition:color .18s,background .18s;white-space:nowrap;
}}
.nb:hover{{color:var(--text);background:rgba(0,0,0,.05)}}
.nb.active{{
  background:linear-gradient(90deg,#2563eb,#7c3aed);
  -webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text;
  font-weight:600;
}}

/* dropdown */
.dd{{position:relative}}
.ddt{{
  display:inline-flex;align-items:center;gap:5px;
  background:transparent;border:none;cursor:pointer;
  color:var(--dim);font-family:'Inter',sans-serif;font-size:.83rem;font-weight:500;
  padding:.38rem .8rem;border-radius:7px;
  transition:color .18s,background .18s;white-space:nowrap;
}}
.ddt:hover{{color:var(--text);background:rgba(0,0,0,.05)}}
.ddt.active{{
  background:linear-gradient(90deg,#2563eb,#7c3aed);
  -webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text;
  font-weight:600;
}}
.ddc{{font-size:.58rem;opacity:.6;transition:transform .2s}}
.dd.open .ddc{{transform:rotate(180deg)}}
.ddm{{
  display:none;position:absolute;right:0;top:calc(100% + 6px);
  background:#ffffff;border:1px solid rgba(0,0,0,.1);border-radius:10px;
  padding:.3rem;min-width:220px;
  box-shadow:0 8px 32px rgba(0,0,0,.12);z-index:300;
}}
.dd.open .ddm{{display:block}}
.ddi{{
  display:block;width:100%;text-align:left;
  background:transparent;border:none;cursor:pointer;
  color:var(--dim);font-family:'Inter',sans-serif;font-size:.8rem;font-weight:400;
  padding:.42rem .8rem;border-radius:6px;
  transition:color .12s,background .12s;white-space:nowrap;
}}
.ddi:hover{{color:var(--text);background:rgba(0,0,0,.05)}}
.ddi.active{{color:#2563eb;background:rgba(37,99,235,.08);font-weight:600}}

/* hamburger */
.hbg{{
  display:none;background:transparent;
  border:1px solid rgba(0,0,0,.12);
  color:var(--dim);padding:.3rem .6rem;border-radius:6px;cursor:pointer;
  font-size:1.1rem;line-height:1;
}}
.mob-menu{{display:none;border-top:1px solid var(--border-s);background:var(--surface);padding:.5rem 1rem .75rem}}
.mob-menu.open{{display:block}}
.mbl{{
  display:block;width:100%;text-align:left;
  background:transparent;border:none;cursor:pointer;
  color:var(--dim);font-family:'Inter',sans-serif;font-size:.88rem;font-weight:500;
  padding:.55rem .6rem;border-radius:6px;transition:color .15s,background .15s;
}}
.mbl:hover{{color:var(--text);background:rgba(0,0,0,.05)}}
.mbl.active{{color:#2563eb;font-weight:600}}
.msh{{
  display:flex;align-items:center;justify-content:space-between;
  width:100%;padding:.55rem .6rem;border-radius:6px;
  background:transparent;border:none;cursor:pointer;
  color:var(--dim);font-family:'Inter',sans-serif;font-size:.88rem;font-weight:500;
  transition:color .15s,background .15s;
}}
.msh:hover{{color:var(--text);background:rgba(0,0,0,.05)}}
.msub{{padding-left:.9rem;overflow:hidden;max-height:0;transition:max-height .25s ease}}
.msub.open{{max-height:600px}}
.msb{{
  display:block;width:100%;text-align:left;
  background:transparent;border:none;cursor:pointer;
  color:var(--muted);font-family:'Inter',sans-serif;font-size:.8rem;
  padding:.38rem .6rem;border-radius:6px;transition:color .12s,background .12s;
}}
.msb:hover{{color:var(--text);background:rgba(0,0,0,.04)}}
.msb.active{{color:#2563eb;font-weight:600}}
@media(max-width:768px){{
  .nav-links{{display:none}}
  .hbg{{display:flex;align-items:center}}
}}

/* ─── Main layout ───────────────────────────────────────────────────── */
.panel{{display:none;padding:1.75rem 1.5rem;max-width:1280px;margin:0 auto}}
.panel.active{{display:block}}
.chart-wrap{{background:var(--surface);border:1px solid var(--border-s);border-radius:10px;padding:1.25rem;margin-bottom:1rem}}
canvas{{max-height:420px}}
h2{{font-size:.78rem;font-weight:600;margin-bottom:.9rem;letter-spacing:.07em;text-transform:uppercase;background:linear-gradient(90deg,#2563eb,#7c3aed);-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}

/* ─── Heatmap tables ────────────────────────────────────────────────── */
.heat{{border-collapse:collapse;width:100%;font-size:.75rem}}
.heat th{{background:var(--surface2);padding:5px 8px;text-align:center;white-space:nowrap;color:var(--muted);font-weight:600;position:sticky;top:0;z-index:1}}
.heat th.left{{text-align:left}}
.heat td{{padding:4px 8px;text-align:center;border:1px solid var(--border-s)}}
.heat td.agent-name{{text-align:left;font-weight:600;white-space:nowrap;background:linear-gradient(90deg,#2563eb,#7c3aed);-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}
.heat-wrap{{overflow-x:auto}}

/* ─── Variation matrix ──────────────────────────────────────────────── */
.var-wrap{{overflow-x:auto}}
.var-table{{border-collapse:collapse;font-size:.7rem}}
.var-table th{{background:var(--surface2);padding:4px 6px;color:var(--muted);white-space:nowrap;font-weight:600}}
.var-table th.left{{text-align:left}}
.var-table td{{padding:2px 4px;text-align:center;border:1px solid var(--border-s)}}
.var-table td.agent-name{{text-align:left;font-weight:600;white-space:nowrap;background:linear-gradient(90deg,#2563eb,#7c3aed);-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}
.dot{{width:16px;height:16px;border-radius:3px;display:inline-block}}
.dot.pass{{background:#16a34a}}
.dot.fail{{background:#dc2626}}
.dot.na{{background:#cbd5e1}}

/* ─── Review panel ──────────────────────────────────────────────────── */
.review-controls{{display:flex;align-items:center;flex-wrap:wrap;gap:6px;padding:.6rem .9rem;background:var(--surface);border:1px solid var(--border-s);border-radius:8px;margin-bottom:1rem}}
.review-label{{color:var(--muted);font-size:.75rem;margin-right:2px;white-space:nowrap}}
.toggle-all{{background:var(--surface2);border:1px solid rgba(0,0,0,.1);color:var(--dim);padding:3px 9px;border-radius:4px;cursor:pointer;font-family:'Inter',sans-serif;font-size:.72rem;transition:all .12s}}
.toggle-all:hover{{border-color:rgba(0,0,0,.2);color:var(--text)}}
.ctrl-divider{{color:rgba(0,0,0,.1);margin:0 4px}}
.agent-pill{{background:var(--bg);border:1px solid rgba(0,0,0,.1);color:var(--muted);padding:3px 10px;border-radius:12px;cursor:pointer;font-family:'Inter',sans-serif;font-size:.72rem;transition:all .12s;white-space:nowrap}}
.agent-pill:hover{{border-color:rgba(0,0,0,.2);color:var(--dim)}}
.agent-pill.active{{background:#eff6ff;border-color:#2563eb;color:#2563eb}}
.review-scenario{{border:1px solid var(--border-s);border-radius:8px;overflow:hidden;margin-bottom:.6rem}}
.review-scen-hdr{{display:flex;align-items:center;gap:.6rem;padding:.55rem .9rem;background:var(--surface);cursor:pointer;user-select:none;transition:background .1s}}
.review-scen-hdr:hover{{background:var(--surface2)}}
.scen-toggle{{color:rgba(0,0,0,.2);font-size:.7rem;width:10px;flex-shrink:0}}
.scen-id{{font-weight:700;font-size:.78rem;min-width:130px;background:linear-gradient(90deg,#2563eb,#7c3aed);-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}
.scen-title{{color:var(--text);flex:1;font-size:.82rem}}
.cat-pill{{background:var(--surface2);border:1px solid rgba(0,0,0,.08);color:var(--dim);padding:1px 7px;border-radius:10px;font-size:.68rem;white-space:nowrap}}
.scen-pass-summary{{color:rgba(0,0,0,.3);font-size:.68rem;white-space:nowrap;margin-left:.5rem}}
.trial-badge{{background:var(--surface2);border:1px solid rgba(0,0,0,.08);color:var(--dim);border-radius:8px;font-size:.65rem;padding:0 5px;font-weight:400;cursor:help}}
.trial-block{{border-top:1px solid var(--border-s);padding:.4rem .5rem .2rem}}
.trial-lbl{{color:var(--dim);font-size:.68rem;font-weight:600;margin-bottom:.3rem}}
.review-scen-body{{display:none;padding:.75rem;background:var(--bg);border-top:1px solid var(--border-s)}}
.review-var{{margin-bottom:1rem}}
.review-var:last-child{{margin-bottom:0}}
.var-prompt{{display:flex;align-items:baseline;gap:.5rem;padding:.45rem .7rem;background:var(--surface);border:1px solid var(--border-s);border-radius:5px;margin-bottom:.55rem;font-size:.8rem}}
.var-label{{color:#6366f1;font-weight:700;flex-shrink:0}}
.var-prompt-text{{color:var(--text);line-height:1.45}}
.response-grid{{display:flex;flex-wrap:wrap;gap:.55rem;align-items:flex-start}}
.agent-card{{flex:1 1 280px;min-width:240px;max-width:520px;border-radius:8px;border:1px solid rgba(0,0,0,.08);overflow:hidden;transition:border-color .15s}}
.agent-card.r-pass{{border-color:rgba(22,163,74,.4)}}
.agent-card.r-fail{{border-color:rgba(220,38,38,.35)}}
.agent-card.r-na{{border-color:var(--border-s);opacity:.6}}
.agent-card-hdr{{display:flex;justify-content:space-between;align-items:center;padding:5px 9px;background:var(--surface2);gap:.5rem}}
.card-agent-id{{font-weight:700;font-size:.75rem;background:linear-gradient(90deg,#2563eb,#7c3aed);-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}
.card-meta{{display:flex;align-items:center;gap:.4rem}}
.card-score{{font-weight:700;font-size:.75rem}}
.card-score.s-pass{{color:#16a34a}}
.card-score.s-fail{{color:#dc2626}}
.card-score.s-na{{color:var(--muted)}}
.card-dur{{color:var(--muted);font-size:.68rem}}
.response-body{{padding:8px 10px;background:var(--surface2);max-height:240px;overflow-y:auto;white-space:pre-wrap;word-break:break-word;font-size:.76rem;line-height:1.55;color:var(--text);font-family:'SF Mono',ui-monospace,monospace}}
.response-empty{{color:rgba(0,0,0,.25);font-style:italic}}
.response-error{{color:#dc2626;font-size:.72rem;padding:6px 10px;background:#fef2f2;border-top:1px solid rgba(220,38,38,.2)}}
.checks-bar{{padding:5px 9px;background:var(--surface);border-top:1px solid var(--border-s)}}
.checks-toggle{{background:none;border:none;color:var(--muted);font-family:'Inter',sans-serif;font-size:.7rem;cursor:pointer;padding:0;display:flex;align-items:center;gap:4px}}
.checks-toggle:hover{{color:var(--text)}}
.checks-list{{margin-top:5px;display:none}}
.check-row{{display:flex;gap:6px;font-size:.69rem;padding:2px 0;line-height:1.4}}
.check-row.ck-pass{{color:#16a34a}}
.check-row.ck-fail{{color:#dc2626}}
.ck-name{{font-weight:600;min-width:180px;flex-shrink:0}}
.ck-detail{{color:var(--muted)}}
.check-row.ck-fail .ck-detail{{color:#ef4444}}
.pipe-error{{padding:4px 9px;background:#fef2f2;border-top:1px solid rgba(220,38,38,.2);font-size:.7rem;color:#dc2626}}

/* ─── About page ────────────────────────────────────────────────────── */
.about-grid{{display:grid;grid-template-columns:1fr 1fr;gap:1rem;align-items:start}}
@media(max-width:780px){{.about-grid{{grid-template-columns:1fr}}}}
.about-col{{display:flex;flex-direction:column;gap:1rem}}
.about-card{{background:var(--surface);border:1px solid var(--border-s);border-radius:10px;padding:1.25rem}}
.about-card h2{{margin-bottom:.8rem}}
.about-card p{{color:var(--text);line-height:1.7;margin-bottom:.6rem;font-size:.83rem}}
.about-card ol,.about-card ul{{color:var(--text);line-height:1.8;font-size:.83rem;padding-left:1.2rem}}
.about-card dt{{font-weight:700;font-size:.78rem;margin-top:.6rem;background:linear-gradient(90deg,#2563eb,#7c3aed);-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}
.about-card dd{{color:var(--text);font-size:.78rem;margin-left:1rem;line-height:1.6;margin-bottom:.2rem}}
.about-scen-table{{width:100%;border-collapse:collapse;font-size:.75rem}}
.about-scen-table th{{background:var(--surface2);padding:4px 8px;text-align:left;color:var(--muted);font-weight:600;white-space:nowrap}}
.about-scen-table td{{padding:4px 8px;border-bottom:1px solid var(--border-s);color:var(--text);vertical-align:top}}
.abt-num{{color:rgba(0,0,0,.2);width:24px;text-align:center}}
.abt-id{{font-weight:600;white-space:nowrap;background:linear-gradient(90deg,#2563eb,#7c3aed);-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}
.abt-cat{{color:var(--dim);white-space:nowrap}}

/* ─── Judge badge ───────────────────────────────────────────────────── */
.judge-badge{{display:inline-flex;align-items:center;gap:3px;background:var(--surface2);border:1px solid rgba(0,0,0,.08);border-radius:8px;padding:1px 6px;font-size:.67rem;color:#d97706;cursor:help;white-space:nowrap}}
.judge-badge .jb-star{{font-size:.65rem}}

/* ─── Head-to-Head ──────────────────────────────────────────────────── */
.h2h-controls{{display:flex;align-items:center;flex-wrap:wrap;gap:.6rem;padding:.7rem 1rem;background:var(--surface);border:1px solid var(--border-s);border-radius:8px;margin-bottom:1rem;font-size:.8rem}}
.h2h-select{{background:var(--bg);border:1px solid rgba(0,0,0,.12);color:var(--text);padding:4px 8px;border-radius:4px;font-family:'Inter',sans-serif;font-size:.78rem;cursor:pointer}}
.h2h-select:focus{{outline:none;border-color:#2563eb}}
.h2h-vs{{color:rgba(0,0,0,.25);font-weight:700;padding:0 4px}}
.h2h-scenario{{border:1px solid var(--border-s);border-radius:8px;overflow:hidden;margin-bottom:.6rem}}
.h2h-scen-hdr{{padding:.45rem .9rem;background:var(--surface);cursor:pointer;user-select:none;display:flex;align-items:center;gap:.6rem}}
.h2h-scen-hdr:hover{{background:var(--surface2)}}
.h2h-scen-body{{display:none;border-top:1px solid var(--border-s)}}
.h2h-var{{display:grid;grid-template-columns:1fr 1fr;gap:0;border-bottom:1px solid var(--border-s)}}
.h2h-var:last-child{{border-bottom:none}}
.h2h-side{{padding:.65rem .9rem;border-right:1px solid var(--border-s)}}
.h2h-side:last-child{{border-right:none}}
.h2h-agent-hdr{{display:flex;align-items:center;justify-content:space-between;margin-bottom:.45rem}}
.h2h-agent-id{{font-weight:700;font-size:.75rem;background:linear-gradient(90deg,#2563eb,#7c3aed);-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}
.h2h-score{{font-weight:700;font-size:.75rem}}
.h2h-score.s-pass{{color:#16a34a}}
.h2h-score.s-fail{{color:#dc2626}}
.h2h-score.s-na{{color:var(--muted)}}
.h2h-response{{background:var(--surface2);border-radius:4px;padding:7px 9px;font-size:.75rem;color:var(--text);white-space:pre-wrap;word-break:break-word;max-height:220px;overflow-y:auto;line-height:1.5;border:1px solid var(--border-s);font-family:'SF Mono',ui-monospace,monospace}}
.h2h-response.empty{{color:rgba(0,0,0,.25);font-style:italic}}
.h2h-var-label{{color:#6366f1;font-weight:700;font-size:.72rem;padding:.3rem .9rem;background:var(--surface2);border-bottom:1px solid var(--border-s);border-top:1px solid var(--border-s)}}
.h2h-checks{{margin-top:.4rem;font-size:.68rem}}
.h2h-check{{display:flex;gap:4px;padding:1px 0}}
.h2h-check.ck-pass{{color:#16a34a}}
.h2h-check.ck-fail{{color:#dc2626}}

/* ─── Models & Cost ─────────────────────────────────────────────────────────── */
.cost-table{{width:100%;border-collapse:collapse;font-size:.78rem;margin-bottom:1rem}}
.cost-table th{{background:var(--surface2);padding:6px 10px;text-align:left;color:var(--muted);font-weight:600;white-space:nowrap;border-bottom:2px solid var(--border-s)}}
.cost-table th.right{{text-align:right}}
.cost-table td{{padding:5px 10px;border-bottom:1px solid var(--border-s);vertical-align:top}}
.cost-table td.right{{text-align:right;font-variant-numeric:tabular-nums}}
.cost-table td.model-name{{font-weight:600;color:#000}}
.cost-table tr:last-child td{{border-bottom:none}}
.cost-note{{font-size:.72rem;color:var(--muted);margin-top:.5rem;line-height:1.6}}
.cost-total-row td{{font-weight:700;background:var(--surface2)}}
.cost-hdr{{display:flex;align-items:center;justify-content:space-between;margin-bottom:.9rem}}
.cost-hdr h2{{margin-bottom:0}}
.cost-refresh-btn{{background:transparent;border:1px solid var(--border-s);color:var(--muted);padding:3px 10px;border-radius:5px;cursor:pointer;font-family:'Inter',sans-serif;font-size:.72rem;transition:all .15s}}
.cost-refresh-btn:hover{{border-color:#2563eb;color:#2563eb}}

/* ─── Footer ────────────────────────────────────────────────────────── */
.site-footer{{background:var(--surface);border-top:1px solid var(--border-s);padding:2.5rem 1.5rem 1.5rem;margin-top:0}}
.footer-inner{{max-width:1280px;margin:0 auto;display:flex;flex-wrap:wrap;gap:2rem;justify-content:space-between;align-items:flex-start}}
.footer-brand{{display:flex;align-items:center;gap:10px}}
.footer-logo{{width:32px;height:32px;border-radius:7px;object-fit:cover;box-shadow:0 1px 4px rgba(0,0,0,.1)}}
.footer-brand-name{{font-weight:700;font-size:.9rem;background:linear-gradient(90deg,#2563eb,#7c3aed);-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}
.footer-tagline{{font-size:.72rem;color:var(--muted);margin-top:1px}}
.footer-cols{{display:flex;gap:2.5rem;flex-wrap:wrap}}
.footer-col{{display:flex;flex-direction:column;gap:.4rem}}
.footer-col-title{{font-size:.72rem;font-weight:600;color:#000;text-transform:uppercase;letter-spacing:.06em;margin-bottom:.2rem}}
.footer-link{{color:var(--muted);font-size:.8rem;text-decoration:none;transition:color .15s}}
.footer-link:hover{{color:#2563eb}}
.footer-bottom{{max-width:1280px;margin:.75rem auto 0;padding-top:.75rem;border-top:1px solid var(--border-s);font-size:.72rem;color:var(--muted)}}
</style>
</head>
<body>

<!-- ── Navbar ─────────────────────────────────────────────────────────── -->
<nav class="navbar">
  <div class="nbi">
    <div class="brand">
      <img class="brand-logo" src="https://gestura.app/icon.png" alt="Gestura"
           onerror="this.style.display='none'">
      <div class="brand-titles">
        <span class="brand-name">Gestura Agent Evaluation</span>
        <span class="brand-sub">By Gestura AI</span>
      </div>
    </div>

    <!-- Desktop nav (right-aligned) -->
    <div class="nav-links">
      <button class="nb active" id="nav-about" onclick="showTab('about',this)">About</button>

      <div class="dd" id="dd-dash">
        <button class="ddt" onclick="toggleDd('dd-dash',event)" id="nav-dash">
          Dashboards <span class="ddc">&#9660;</span>
        </button>
        <div class="ddm">
          <button class="ddi" onclick="showTab('leaderboard',this,'dd-dash')">&#9312; Leaderboard</button>
          <button class="ddi" onclick="showTab('category',this,'dd-dash')">&#9313; Category Heatmap</button>
          <button class="ddi" onclick="showTab('degradation',this,'dd-dash')">&#9314; Degradation</button>
          <button class="ddi" onclick="showTab('radar',this,'dd-dash')">&#9315; Radar</button>
          <button class="ddi" onclick="showTab('checks',this,'dd-dash')">&#9316; Check Failures</button>
          <button class="ddi" onclick="showTab('latency',this,'dd-dash')">&#9317; Latency</button>
          <button class="ddi" onclick="showTab('matrix',this,'dd-dash')">&#9318; Variation Matrix</button>
        </div>
      </div>

      <button class="nb" id="nav-review" onclick="showTab('review',this)">Responses</button>
      <button class="nb" id="nav-h2h"    onclick="showTab('h2h',this)">Head-to-Head</button>
      <button class="nb" id="nav-models" onclick="showTab('models',this)">Models &amp; Cost</button>
    </div>

    <button class="hbg" onclick="toggleMob()" aria-label="Menu">&#9776;</button>
  </div>

  <!-- Mobile menu -->
  <div class="mob-menu" id="mob-menu">
    <button class="mbl active" id="mob-about"   onclick="showTab('about',this);closeMob()">About</button>
    <button class="msh" onclick="toggleMobSub(this)">
      <span>Dashboards</span><span style="font-size:.58rem;opacity:.5">&#9660;</span>
    </button>
    <div class="msub" id="msub-dash">
      <button class="msb" onclick="showTab('leaderboard',this,'mob');closeMob()">&#9312; Leaderboard</button>
      <button class="msb" onclick="showTab('category',this,'mob');closeMob()">&#9313; Category Heatmap</button>
      <button class="msb" onclick="showTab('degradation',this,'mob');closeMob()">&#9314; Degradation</button>
      <button class="msb" onclick="showTab('radar',this,'mob');closeMob()">&#9315; Radar</button>
      <button class="msb" onclick="showTab('checks',this,'mob');closeMob()">&#9316; Check Failures</button>
      <button class="msb" onclick="showTab('latency',this,'mob');closeMob()">&#9317; Latency</button>
      <button class="msb" onclick="showTab('matrix',this,'mob');closeMob()">&#9318; Variation Matrix</button>
    </div>
    <button class="mbl" id="mob-review" onclick="showTab('review',this);closeMob()">Responses</button>
    <button class="mbl" id="mob-h2h"    onclick="showTab('h2h',this);closeMob()">Head-to-Head</button>
    <button class="mbl" id="mob-models" onclick="showTab('models',this);closeMob()">Models &amp; Cost</button>
  </div>
</nav>

<div class="main-wrap">
<!-- ① Leaderboard -->
<div id="tab-leaderboard" class="panel">
  <div class="chart-wrap">
    <h2>Overall Leaderboard</h2>
    <canvas id="chart-leaderboard"></canvas>
  </div>
  <div class="chart-wrap">
    <h2>Family Leaderboard</h2>
    <canvas id="chart-family"></canvas>
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
    <h2>Profile Degradation: Quality Loss by Permission Mode</h2>
    <canvas id="chart-degradation"></canvas>
  </div>
</div>

<!-- ④ Strength Radar -->
<div id="tab-radar" class="panel">
  <div class="chart-wrap">
    <h2>Capability Radar: Per-Category Strength</h2>
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

<!-- ⑨ Head to Head -->
<div id="tab-h2h" class="panel">
  <div class="h2h-controls">
    <span style="color:var(--muted);font-size:.75rem">Compare</span>
    <select class="h2h-select" id="h2h-left" onchange="renderH2H()"></select>
    <span class="h2h-vs">vs</span>
    <select class="h2h-select" id="h2h-right" onchange="renderH2H()"></select>
  </div>
  <div id="h2h-body"></div>
</div>

<!-- ⑩ Models & Cost -->
<div id="tab-models" class="panel">
{models_cost_panel}
</div>

<!-- ⓪ About (default) -->
<div id="tab-about" class="panel active">
{about_page}
</div>
</div>

<!-- ── Footer ──────────────────────────────────────────────────────────── -->
<footer class="site-footer">
  <div class="footer-inner">
    <div class="footer-cols">
      <div class="footer-col">
        <div class="footer-col-title">Ecosystem</div>
        <a class="footer-link" href="https://gestura.ai" target="_blank" rel="noopener">Gestura AI</a>
        <a class="footer-link" href="https://gestura.app" target="_blank" rel="noopener">Gestura App</a>
        <a class="footer-link" href="https://haptic-harmony.com" target="_blank" rel="noopener">Haptic Harmony Ring</a>
        <a class="footer-link" href="https://gestura.dev" target="_blank" rel="noopener">Developer Platform</a>
      </div>
    </div>
  </div>
  <div class="footer-bottom">
    <span>© 2026 Gestura AI LLC. All rights reserved.</span>
  </div>
</footer>

<script>
const DATA = {data_json};

// ── Tab navigation ──────────────────────────────────────────────────────────
const DASH_TABS = ['leaderboard','category','degradation','radar','checks','latency','matrix'];

function showTab(name, btn, src) {{
  // Swap panel
  document.querySelectorAll('.panel').forEach(p => p.classList.remove('active'));
  document.getElementById('tab-' + name).classList.add('active');

  if (name === 'models') initModels();

  // Clear all nav active states
  document.querySelectorAll('.nb,.ddt,.ddi,.mbl,.msb').forEach(b => b.classList.remove('active'));

  const isDash = DASH_TABS.includes(name);

  if (src === 'mob') {{
    // Mobile sub-item: mark it active and activate desktop Dashboards toggle
    if (btn) btn.classList.add('active');
    if (isDash) document.getElementById('nav-dash').classList.add('active');
  }} else if (src) {{
    // Desktop dropdown item: mark item + toggle, close dropdown
    if (btn) btn.classList.add('active');
    document.getElementById('nav-dash').classList.add('active');
    document.getElementById(src).classList.remove('open');
  }} else {{
    // Desktop nav-btn (About / Responses / H2H)
    if (btn) btn.classList.add('active');
    // Sync mobile counterpart
    const mob = document.getElementById('mob-' + name);
    if (mob) mob.classList.add('active');
  }}
}}

// ── Dropdown ────────────────────────────────────────────────────────────────
function toggleDd(id, e) {{
  e.stopPropagation();
  document.getElementById(id).classList.toggle('open');
}}
document.addEventListener('click', () => {{
  document.querySelectorAll('.dd.open').forEach(d => d.classList.remove('open'));
}});

// ── Mobile menu ─────────────────────────────────────────────────────────────
function toggleMob() {{ document.getElementById('mob-menu').classList.toggle('open'); }}
function closeMob()  {{ document.getElementById('mob-menu').classList.remove('open'); }}
function toggleMobSub(hdr) {{
  const sub = hdr.nextElementSibling;
  sub.classList.toggle('open');
  const arr = hdr.querySelector('span:last-child');
  if (arr) arr.style.transform = sub.classList.contains('open') ? 'rotate(180deg)' : '';
}}

Chart.defaults.color = '#64748b';
Chart.defaults.borderColor = '#e2e8f0';
Chart.defaults.font.family = "'Inter',system-ui,sans-serif";

function scoreColor(s) {{
  const hue = Math.round(s * 120);
  return `hsl(${{hue}},60%,45%)`;
}}

const PALETTE = ['#60a5fa','#34d399','#fbbf24','#f87171','#a78bfa','#4ade80','#fb923c','#f472b6','#93c5fd','#6ee7b7'];

{chart_init}

/* ── Review panel ───────────────────────────────────────────────────────────── */
function reviewToggle(btn) {{
  const agent = btn.dataset.agent;
  const show  = btn.classList.toggle('active');
  document.querySelectorAll(`.agent-card[data-agent="${{agent}}"]`)
    .forEach(el => el.style.display = show ? '' : 'none');
}}

function reviewToggleAll(show) {{
  document.querySelectorAll('.agent-pill').forEach(b => show ? b.classList.add('active') : b.classList.remove('active'));
  document.querySelectorAll('.agent-card').forEach(el => el.style.display = show ? '' : 'none');
}}

function toggleScenario(hdr) {{
  const body = hdr.nextElementSibling;
  const icon = hdr.querySelector('.scen-toggle');
  const open = body.style.display !== 'none';
  body.style.display = open ? 'none' : 'block';
  icon.textContent   = open ? '▶' : '▼';
}}

function toggleChecks(btn) {{
  const list = btn.nextElementSibling;
  const open = list.style.display !== 'none';
  list.style.display = open ? 'none' : 'block';
  btn.textContent    = open ? '▶ checks' : '▼ checks';
}}

/* ── Head-to-Head ───────────────────────────────────────────────────────────── */
(function initH2H() {{
  const agents = DATA.reports.map(r => r.agent_id);
  const lSel   = document.getElementById('h2h-left');
  const rSel   = document.getElementById('h2h-right');
  agents.forEach(a => {{
    const ol = document.createElement('option'); ol.value = a; ol.textContent = a; lSel.appendChild(ol);
    const or2 = document.createElement('option'); or2.value = a; or2.textContent = a; rSel.appendChild(or2);
  }});
  if (agents.length > 1) rSel.value = agents[1];
  renderH2H();
}})();

function renderH2H() {{
  const la = document.getElementById('h2h-left').value;
  const ra = document.getElementById('h2h-right').value;
  const lReport = DATA.reports.find(r => r.agent_id === la);
  const rReport = DATA.reports.find(r => r.agent_id === ra);
  if (!lReport || !rReport) return;

  function idx(report) {{
    const m = {{}};
    (report.scenarios || []).forEach(s => {{
      s.variations.forEach(v=>{{ m[s.scenario_id+'|'+v.variation_id]=v; }});
    }});
    return m;
  }}
  const lIdx = idx(lReport), rIdx = idx(rReport);

  const scenIds = [...new Set([
    ...(lReport.scenarios || []).map(s => s.scenario_id),
    ...(rReport.scenarios || []).map(s => s.scenario_id)
  ])];

  let html = '';
  scenIds.forEach(sid => {{
    const lScen = (lReport.scenarios || []).find(s => s.scenario_id === sid);
    const rScen = (rReport.scenarios || []).find(s => s.scenario_id === sid);
    const scen  = lScen || rScen;
    const varIds = [...new Set([
      ...(lScen ? lScen.variations.map(v => v.variation_id) : []),
      ...(rScen ? rScen.variations.map(v => v.variation_id) : [])
    ])];

    html += `<div class='h2h-scenario'>
      <div class='h2h-scen-hdr' onclick='this.nextElementSibling.style.display=this.nextElementSibling.style.display==="none"?"block":"none"'>
        <span style='font-weight:700;font-size:.78rem;background:linear-gradient(90deg,#2563eb,#7c3aed);-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text'>${{escH(sid)}}</span>
        <span style='color:var(--text);font-size:.82rem;flex:1;margin-left:.6rem'>${{escH(scen ? scen.scenario_name : '')}}</span>
        <span style='color:rgba(0,0,0,.35);font-size:.7rem'>click to expand</span>
      </div>
      <div style='display:none'>`;

    varIds.forEach(vid => {{
      const lv = lIdx[sid + '|' + vid];
      const rv = rIdx[sid + '|' + vid];
      const prompt = (lv || rv)?.prompt_preview || '';
      html += `<div class='h2h-var-label'>${{escH(vid)}}: ${{escH(prompt)}}</div>
        <div class='h2h-var'>
          ${{h2hSide(la, lv)}}
          ${{h2hSide(ra, rv)}}
        </div>`;
    }});
    html += `</div></div>`;
  }});

  document.getElementById('h2h-body').innerHTML = html ||
    '<p style="color:var(--muted);padding:1rem">No data.</p>';
}}

function h2hSide(agentId, vr) {{
  if (!vr) return `<div class='h2h-side'><div class='h2h-agent-hdr'><span class='h2h-agent-id'>${{escH(agentId)}}</span><span class='h2h-score s-na'>–</span></div><div class='h2h-response empty'>no data</div></div>`;
  const sc  = vr.passed ? 's-pass' : 's-fail';
  const pct = Math.round((vr.score || 0) * 100) + '%';
  const resp = vr.response || '';
  const failChecks = (vr.checks || []).filter(c => !c.passed && !c.skipped);
  const checksHtml = failChecks.length
    ? `<div class='h2h-checks'>${{failChecks.map(c =>
        `<div class='h2h-check ck-fail'><span style='font-weight:600;min-width:160px;display:inline-block'>${{escH(c.name)}}</span> ${{escH(c.details)}}</div>`
      ).join('')}}</div>`
    : '';
  const judgeHtml = vr.judge_score
    ? `<span class='judge-badge' style='margin-top:4px;display:inline-flex'
         title='accuracy ${{vr.judge_score.accuracy}}/5  completeness ${{vr.judge_score.completeness}}/5  clarity ${{vr.judge_score.clarity}}/5. ${{escH(vr.judge_score.reasoning||"")}}'>
         &#9733; ${{vr.judge_score.overall}}/5</span>`
    : '';
  return `<div class='h2h-side'>
    <div class='h2h-agent-hdr'>
      <span class='h2h-agent-id'>${{escH(agentId)}}</span>
      <span class='h2h-score ${{sc}}'>${{pct}}</span>
    </div>
    ${{judgeHtml}}
    <div class='h2h-response${{resp.trim() ? '' : ' empty'}}'>${{resp.trim() ? escH(resp) : 'empty response'}}</div>
    ${{checksHtml}}
  </div>`;
}}

/* ── Models & Cost ─────────────────────────────────────────────────────────── */
let _modelsInitialized = false;
let _costCharts = {{}};

function _costStripSuffix(id) {{
  for (const s of ['-full','-iterative','-sandboxed']) {{
    if (id.endsWith(s)) return id.slice(0, id.length - s.length);
  }}
  return id;
}}

const FALLBACK_PRICING = {{
  'claude-opus-4-6':          {{in:15.0,  out:75.0}},
  'claude-opus-4-5':          {{in:15.0,  out:75.0}},
  'claude-sonnet-4-6':        {{in:3.0,   out:15.0}},
  'claude-sonnet-4-5':        {{in:3.0,   out:15.0}},
  'claude-haiku-4-6':         {{in:0.80,  out:4.0}},
  'claude-haiku-4-5-20251001':{{in:0.80,  out:4.0}},
  'claude-haiku-4-5':         {{in:0.80,  out:4.0}},
}};

function _getPrice(pricing, model) {{
  if (pricing[model]) return pricing[model];
  const base = model.replace(/-\d{{8}}$/, '');
  if (pricing[base]) return pricing[base];
  if (model.includes('opus'))  return {{in:15.0, out:75.0}};
  if (model.includes('haiku')) return {{in:0.80,  out:4.0}};
  return {{in:3.0, out:15.0}};
}}

function _fmtCost(usd) {{
  if (usd < 0.00001) return '&lt;$0.00001';
  if (usd < 0.01)    return '$' + usd.toFixed(5);
  return '$' + usd.toFixed(4);
}}

function _fmtTok(n) {{
  if (n >= 1000000) return (n/1000000).toFixed(1) + 'M';
  if (n >= 1000)    return (n/1000).toFixed(1) + 'K';
  return String(n);
}}

function _renderCostTables(pricing, source) {{
  const cd = DATA.cost_data || [];
  const judgeModel = DATA.judge_model || 'claude-sonnet-4-6';

  // ── Agent table ───────────────────────────────────────────
  let agentHtml = '';
  let totalIn = 0, totalOut = 0, totalCost = 0;
  for (const a of cd) {{
    const p = _getPrice(pricing, a.model);
    const cost = (a.input_tok / 1e6) * p.in + (a.output_tok / 1e6) * p.out;
    totalIn += a.input_tok; totalOut += a.output_tok; totalCost += cost;
    agentHtml += `<tr>
      <td>${{escH(a.agent_id)}}</td>
      <td class="model-name">${{escH(a.model)}}</td>
      <td class="right">${{_fmtTok(a.input_tok)}}</td>
      <td class="right">${{_fmtTok(a.output_tok)}}</td>
      <td class="right">${{_fmtCost(cost)}}</td></tr>`;
  }}
  agentHtml += `<tr class="cost-total-row">
    <td colspan="2">Total</td>
    <td class="right">${{_fmtTok(totalIn)}}</td>
    <td class="right">${{_fmtTok(totalOut)}}</td>
    <td class="right">${{_fmtCost(totalCost)}}</td></tr>`;
  const agentEl = document.getElementById('cost-agent-body');
  if (agentEl) agentEl.innerHTML = agentHtml;

  // ── Model rollup ──────────────────────────────────────────
  const modelMap = {{}};
  for (const a of cd) {{
    if (!modelMap[a.model]) modelMap[a.model] = {{in:0, out:0}};
    modelMap[a.model].in  += a.input_tok;
    modelMap[a.model].out += a.output_tok;
  }}
  let modelHtml = '';
  let grandIn = totalIn, grandOut = totalOut, grandCost = totalCost;
  const hasJudge = cd.some(a => a.has_judge);
  let judgeCost = 0;
  for (const [model, tok] of Object.entries(modelMap).sort()) {{
    const p = _getPrice(pricing, model);
    const cost = (tok.in / 1e6) * p.in + (tok.out / 1e6) * p.out;
    modelHtml += `<tr>
      <td class="model-name">${{escH(model)}}</td>
      <td class="right">${{_fmtTok(tok.in)}}</td>
      <td class="right">${{_fmtTok(tok.out)}}</td>
      <td class="right">${{_fmtCost(cost)}}</td></tr>`;
  }}
  if (hasJudge) {{
    const jp = _getPrice(pricing, judgeModel);
    const jIn  = cd.reduce((s, a) => s + (a.judge_input_tok  || 0), 0);
    const jOut = cd.reduce((s, a) => s + (a.judge_output_tok || 0), 0);
    judgeCost = (jIn / 1e6) * jp.in + (jOut / 1e6) * jp.out;
    grandIn += jIn; grandOut += jOut; grandCost += judgeCost;
    modelHtml += `<tr>
      <td class="model-name">${{escH(judgeModel)}} (judge)</td>
      <td class="right">${{_fmtTok(jIn)}}</td>
      <td class="right">${{_fmtTok(jOut)}}</td>
      <td class="right">${{_fmtCost(judgeCost)}}</td></tr>`;
  }}
  modelHtml += `<tr class="cost-total-row">
    <td>Grand Total</td>
    <td class="right">${{_fmtTok(grandIn)}}</td>
    <td class="right">${{_fmtTok(grandOut)}}</td>
    <td class="right">${{_fmtCost(grandCost)}}</td></tr>`;
  const modelEl = document.getElementById('cost-model-body');
  if (modelEl) modelEl.innerHTML = modelHtml;

  // ── Judge section ─────────────────────────────────────────
  const judgeWrap = document.getElementById('cost-judge-wrap');
  if (judgeWrap) {{
    if (hasJudge) {{
      judgeWrap.style.display = '';
      const jp = _getPrice(pricing, judgeModel);
      let judgeHtml = '';
      for (const a of cd) {{
        if (!a.has_judge) continue;
        const jCost = ((a.judge_input_tok || 0) / 1e6) * jp.in + ((a.judge_output_tok || 0) / 1e6) * jp.out;
        judgeHtml += `<tr>
          <td>${{escH(a.agent_id)}}</td>
          <td class="right">${{_fmtTok(a.judge_input_tok || 0)}}</td>
          <td class="right">${{_fmtTok(a.judge_output_tok || 0)}}</td>
          <td class="right">${{_fmtCost(jCost)}}</td></tr>`;
      }}
      const jIn  = cd.reduce((s, a) => s + (a.judge_input_tok  || 0), 0);
      const jOut = cd.reduce((s, a) => s + (a.judge_output_tok || 0), 0);
      judgeHtml += `<tr class="cost-total-row">
        <td>Total</td>
        <td class="right">${{_fmtTok(jIn)}}</td>
        <td class="right">${{_fmtTok(jOut)}}</td>
        <td class="right">${{_fmtCost(judgeCost)}}</td></tr>`;
      const judgeEl = document.getElementById('cost-judge-body');
      if (judgeEl) judgeEl.innerHTML = judgeHtml;
      const noteEl = document.getElementById('cost-judge-note');
      if (noteEl) noteEl.textContent = 'Judge cost tracked separately and excluded from agent totals. Judge model: ' + judgeModel + '.';
    }}
  }}

  // ── Pricing source badge ──────────────────────────────────
  const badge = document.getElementById('cost-pricing-source');
  if (badge) {{
    if (source === 'openrouter') {{
      badge.innerHTML = '<span style="color:#16a34a;font-weight:500">&#10003; Live pricing from OpenRouter</span>';
    }} else {{
      badge.innerHTML = '<span style="color:var(--muted)">Estimated pricing (hardcoded fallback)</span>';
    }}
  }}

  // ── Cost leaderboard by profile (cheapest first) ──────────
  const profileCosts = cd.map(a => {{
    const p = _getPrice(pricing, a.model);
    return {{id: a.agent_id, cost: (a.input_tok / 1e6) * p.in + (a.output_tok / 1e6) * p.out}};
  }}).sort((a, b) => a.cost - b.cost);
  const profileColors = profileCosts.map((_, i) => `hsl(${{200 + i * 18}},60%,50%)`);
  const profileEl = document.getElementById('chart-cost-profile');
  if (profileEl) {{
    if (_costCharts.profile) _costCharts.profile.destroy();
    _costCharts.profile = new Chart(profileEl, {{
      type:'bar',
      data:{{labels:profileCosts.map(x=>x.id),datasets:[{{label:'Est. Cost (USD)',data:profileCosts.map(x=>x.cost),backgroundColor:profileColors,borderWidth:0}}]}},
      options:{{indexAxis:'y',responsive:true,plugins:{{legend:{{display:false}}}},scales:{{x:{{beginAtZero:true,ticks:{{callback:v=>'$'+v.toFixed(4)}}}},y:{{ticks:{{font:{{size:11}}}}}}}}}}
    }});
  }}

  // ── Cost leaderboard by family (cheapest first) ───────────
  const famCostMap = {{}};
  for (const a of cd) {{
    const fam = _costStripSuffix(a.agent_id);
    if (!famCostMap[fam]) famCostMap[fam] = 0;
    const p = _getPrice(pricing, a.model);
    famCostMap[fam] += (a.input_tok / 1e6) * p.in + (a.output_tok / 1e6) * p.out;
  }}
  const famCostArr = Object.entries(famCostMap).sort((a, b) => a[1] - b[1]);
  const famEl = document.getElementById('chart-cost-family');
  if (famEl) {{
    if (_costCharts.family) _costCharts.family.destroy();
    _costCharts.family = new Chart(famEl, {{
      type:'bar',
      data:{{labels:famCostArr.map(x=>x[0]),datasets:[{{label:'Est. Cost (USD)',data:famCostArr.map(x=>x[1]),backgroundColor:famCostArr.map((_,i)=>`hsl(${{200+i*20}},60%,50%)`),borderWidth:0}}]}},
      options:{{indexAxis:'y',responsive:true,plugins:{{legend:{{display:false}}}},scales:{{x:{{beginAtZero:true,ticks:{{callback:v=>'$'+v.toFixed(4)}}}},y:{{ticks:{{font:{{size:11}}}}}}}}}}
    }});
  }}
}}

async function initModels(force) {{
  if (_modelsInitialized && !force) return;
  _modelsInitialized = true;
  let pricing = Object.assign({{}}, FALLBACK_PRICING);
  let source = 'fallback';
  try {{
    const resp = await fetch('https://openrouter.ai/api/v1/models', {{
      signal: AbortSignal.timeout(6000)
    }});
    if (resp.ok) {{
      const data = await resp.json();
      const ms = (data.data || []).filter(m => m.id && m.id.startsWith('anthropic/'));
      let matched = 0;
      for (const m of ms) {{
        const ourId = m.id.replace('anthropic/', '');
        const inP   = parseFloat((m.pricing || {{}}).prompt     || '0') * 1000000;
        const outP  = parseFloat((m.pricing || {{}}).completion || '0') * 1000000;
        if (inP > 0 && outP > 0) {{ pricing[ourId] = {{in: inP, out: outP}}; matched++; }}
      }}
      if (matched > 0) source = 'openrouter';
    }}
  }} catch (_) {{}}
  _renderCostTables(pricing, source);
}}

function escH(s) {{
  return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}}
</script>
</body>
</html>"#,
        category_table    = category_table,
        check_table       = check_table,
        variation_grid    = variation_grid,
        review_panel      = review_panel,
        about_page        = about_page,
        models_cost_panel = models_cost_panel,
        data_json         = data_json,
        chart_init        = chart_init,
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

    // Slim per-agent report — only what H2H and Response Review JS need.
    // Omits large redundant fields (provider, dry_run, summary) to keep HTML size down.
    let reports: Vec<serde_json::Value> = report.agent_reports.iter().map(|r| {
        let scenarios: Vec<serde_json::Value> = r.scenarios.iter().map(|s| {
            let variations: Vec<serde_json::Value> = s.variations.iter().map(|v| {
                let checks: Vec<serde_json::Value> = v.checks.iter().map(|c| {
                    serde_json::json!({
                        "name": c.name,
                        "passed": c.passed,
                        "skipped": c.skipped,
                        "details": c.details,
                    })
                }).collect();
                let mut vj = serde_json::json!({
                    "variation_id": v.variation_id,
                    "prompt_preview": v.prompt_preview,
                    "response": v.response,
                    "score": v.score,
                    "passed": v.passed,
                    "duration_ms": v.duration_ms,
                    "checks": checks,
                });
                if let Some(ref js) = v.judge_score {
                    vj["judge_score"] = serde_json::json!({
                        "accuracy": js.accuracy,
                        "completeness": js.completeness,
                        "clarity": js.clarity,
                        "overall": js.overall,
                        "reasoning": js.reasoning,
                    });
                }
                vj
            }).collect();
            serde_json::json!({
                "scenario_id": s.scenario_id,
                "scenario_name": s.scenario_name,
                "variations": variations,
            })
        }).collect();
        serde_json::json!({
            "agent_id": r.agent_id,
            "model": r.model,
            "scenarios": scenarios,
        })
    }).collect();

    let judge_model_name = "claude-sonnet-4-6";

    let cost_data: Vec<serde_json::Value> = report.agent_reports.iter().map(|ar| {
        fn est_tokens(text: &str) -> u64 {
            ((text.len() as f64) / 4.0).ceil() as u64
        }

        let mut input_tok = 0u64;
        let mut output_tok = 0u64;
        let mut judge_input_tok = 0u64;
        let mut judge_output_tok = 0u64;

        for sc in &ar.scenarios {
            for vr in &sc.variations {
                let trials = vr.trial_responses.len().max(1);
                input_tok += est_tokens(&vr.prompt_preview) * trials as u64;
                if !vr.trial_responses.is_empty() {
                    for resp in &vr.trial_responses {
                        output_tok += est_tokens(resp);
                    }
                } else {
                    output_tok += est_tokens(&vr.response);
                }
                if let Some(js) = &vr.judge_score {
                    judge_input_tok += est_tokens(&vr.prompt_preview)
                        + est_tokens(&vr.response)
                        + 125;
                    judge_output_tok += est_tokens(&js.reasoning);
                }
            }
        }

        serde_json::json!({
            "agent_id": ar.agent_id,
            "model": ar.model,
            "input_tok": input_tok,
            "output_tok": output_tok,
            "judge_input_tok": judge_input_tok,
            "judge_output_tok": judge_output_tok,
            "has_judge": judge_input_tok > 0,
        })
    }).collect();

    serde_json::json!({
        "leaderboard": leaderboard,
        "profile_degradation": degradation,
        "latency_summary": latency,
        "categories": categories,
        "category_data": category_data,
        "reports": reports,
        "cost_data": cost_data,
        "judge_model": judge_model_name,
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

                        // Score badge: show avg + trial count when >1 trial.
                        let n_trials = vr.trial_scores.len();
                        let score_pct = if n_trials > 1 {
                            let min = vr.trial_scores.iter().cloned().fold(f32::INFINITY, f32::min);
                            let max = vr.trial_scores.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                            format!("{:.0}% avg <span class='trial-badge' title='range {:.0}–{:.0}%'>×{n_trials}</span>",
                                vr.score * 100.0, min * 100.0, max * 100.0)
                        } else {
                            format!("{:.0}%", vr.score * 100.0)
                        };
                        let dur = if vr.duration_ms > 0 {
                            format!("{}ms", vr.duration_ms)
                        } else {
                            String::new()
                        };

                        out.push_str(&format!(
                            "<div class='agent-card {pass_cls}' data-agent='{ae}'{hidden}>",
                            ae = html_escape(agent),
                        ));

                        // Judge badge (only rendered when judge score is present).
                        let judge_badge = if let Some(ref js) = vr.judge_score {
                            let stars = "★".repeat(js.overall as usize)
                                + &"☆".repeat(5usize.saturating_sub(js.overall as usize));
                            format!(
                                "<span class='judge-badge' title='Judge: accuracy {}/5  completeness {}/5  clarity {}/5. {}'>\
                                   <span class='jb-star'>{stars}</span> {}/5\
                                 </span>",
                                js.accuracy, js.completeness, js.clarity,
                                html_escape(&js.reasoning),
                                js.overall,
                            )
                        } else {
                            String::new()
                        };

                        // Card header
                        out.push_str(&format!(
                            "<div class='agent-card-hdr'>\
                               <span class='card-agent-id'>{ae}</span>\
                               <span class='card-meta'>\
                                 {judge_badge}\
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

                        // Response body — single vs. multi-trial layout.
                        if n_trials > 1 {
                            // Show each trial's response in a labelled block.
                            let responses = if vr.trial_responses.is_empty() {
                                std::iter::repeat(vr.response.as_str())
                                    .take(n_trials)
                                    .map(str::to_string)
                                    .collect::<Vec<_>>()
                            } else {
                                vr.trial_responses.clone()
                            };
                            for (i, resp) in responses.iter().enumerate() {
                                let trial_score = vr.trial_scores.get(i).copied().unwrap_or(vr.score);
                                let t_cls = if trial_score >= 0.5 { "s-pass" } else { "s-fail" };
                                out.push_str(&format!(
                                    "<div class='trial-block'>\
                                       <div class='trial-lbl'>Trial {} &nbsp;<span class='{t_cls}'>{:.0}%</span></div>",
                                    i + 1, trial_score * 100.0,
                                ));
                                if resp.trim().is_empty() {
                                    out.push_str("<div class='response-body'><span class='response-empty'>empty response</span></div>");
                                } else {
                                    out.push_str(&format!("<div class='response-body'>{}</div>", html_escape(resp)));
                                }
                                out.push_str("</div>"); // trial-block
                            }
                        } else if vr.response.trim().is_empty() {
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

// ─── About page ───────────────────────────────────────────────────────────────

fn build_about_page(report: &ComparisonReport) -> String {
    let agent_count    = report.leaderboard.len();
    let scenario_count = report.agent_reports.first().map(|r| r.scenarios.len()).unwrap_or(0);
    let timestamp      = report.timestamp.format("%Y-%m-%d %H:%M UTC");
    let run_id         = html_escape(&report.run_id);
    let trials = report.agent_reports.first()
        .and_then(|r| r.scenarios.first())
        .and_then(|s| s.variations.first())
        .map(|v| v.trial_scores.len().max(1))
        .unwrap_or(1);

    let scenarios_html = report.agent_reports.first()
        .map(|r| r.scenarios.iter().enumerate().map(|(i, s)| {
            format!("<tr><td class='abt-num'>{}</td><td class='abt-id'>{}</td><td>{}</td><td class='abt-cat'>{}</td></tr>",
                i + 1,
                html_escape(&s.scenario_id),
                html_escape(&s.scenario_name),
                html_escape(&s.category))
        }).collect::<String>())
        .unwrap_or_default();

    format!(r#"<div class="about-grid">

<!-- Left column -->
<div class="about-col">

<div class="about-card">
<h2>About This Report</h2>
<p>This report compares <strong>{agent_count}</strong> agent profile(s) across <strong>{scenario_count}</strong> evaluation scenario(s) with <strong>{trials}</strong> trial(s) per variation. Each scenario tests a specific capability area using one or more prompt variations, each scored against a rubric of named checks.</p>
<p>Run timestamp: <strong>{timestamp}</strong><br>Run ID: <strong>{run_id}</strong></p>
</div>

<div class="about-card">
<h2>How Evaluations Work</h2>
<ol>
<li>Each scenario contains one or more prompt variations with a rubric of named checks.</li>
<li>The agent CLI is invoked as a subprocess: <code>&lt;binary&gt; [args_prefix...] &quot;&lt;prompt&gt;&quot;</code>. stdout is captured; stderr and exit code are used for error classification.</li>
<li>A rule-based evaluator scores the response against the rubric. Each check is a pattern match, keyword presence, word-count gate, or semantic constraint.</li>
<li>Score = passing checks &divide; total checks (0.0 &ndash; 1.0).</li>
<li>When <code>trials &gt; 1</code>, each variation is run N independent times. The displayed score is the mean across all trial runs; a variation passes if more than half its trials pass (majority vote). Higher trial counts reduce score variance from non-deterministic responses at the cost of added latency and token spend.</li>
<li>When LLM judging is enabled, a judge model scores each response on <strong>accuracy</strong>, <strong>completeness</strong>, and <strong>clarity</strong> (1–5 each) and produces an overall holistic score. Judge scores appear alongside the rule-based score in Response Review and Head-to-Head but do not affect pass/fail thresholds.</li>
</ol>
</div>

<div class="about-card">
<h2>Scoring &amp; Thresholds</h2>
<dl>
<dt>Variation score</dt><dd>Fraction of checks that passed for one prompt/response pair.</dd>
<dt>Scenario score</dt><dd>Average variation score within the scenario.</dd>
<dt>Overall score</dt><dd>Mean variation score across all scenarios and agents.</dd>
<dt>Pass threshold</dt><dd>Configurable per-profile (default: 80% per variation, 100% of variations per scenario).</dd>
<dt>Trials</dt><dd>Number of independent runs per variation. Score is the mean across all trial runs; pass/fail uses majority vote. Trials reduce variance introduced by non-deterministic model responses, useful when a single run is not representative.</dd>
<dt>LLM Judge score</dt><dd>Optional holistic score from a judge model (accuracy / completeness / clarity, each 1–5, plus an overall). Appears as a ★ badge in Response Review and Head-to-Head. Does not affect rule-based pass/fail thresholds; it is a supplemental quality signal.</dd>
</dl>
</div>

<div class="about-card">
<h2>Permission Modes</h2>
<dl>
<dt>Full</dt><dd>Unrestricted tool access, shell execution, file writes, network. Full autonomous task completion.</dd>
<dt>Iterative</dt><dd>Restricted tools; agent pauses at side-effectful actions for human approval before proceeding.</dd>
<dt>Sandboxed</dt><dd>Read-only, no shell, no writes, no network. Reasoning and analysis only.</dd>
</dl>
</div>

</div><!-- /about-col left -->

<!-- Right column -->
<div class="about-col">

<div class="about-card">
<h2>Test Scenarios</h2>
<table class="about-scen-table">
<thead><tr><th>#</th><th>ID</th><th>Name</th><th>Category</th></tr></thead>
<tbody>{scenarios_html}</tbody>
</table>
</div>

<div class="about-card">
<h2>Charts Guide</h2>
<dl>
<dt>&#9312; Overall Leaderboard</dt><dd>Horizontal bar chart ranking all agent profiles by mean score. Includes a Family Leaderboard sub-chart that groups full / iterative / sandboxed bars side by side for a quick cross-tier comparison.</dd>
<dt>&#9313; Category Heatmap</dt><dd>Table of agent &times; category scores, colour-coded green&rarr;red. Reveals which capability areas each agent excels or struggles at.</dd>
<dt>&#9314; Profile Degradation</dt><dd>Grouped bar comparing full/iterative/sandboxed scores within each agent family. Shows quality loss as permissions are restricted.</dd>
<dt>&#9315; Capability Radar</dt><dd>Spider/radar chart overlaying full-permission agents across all categories. Good for seeing each agent&rsquo;s capability fingerprint at a glance.</dd>
<dt>&#9316; Check Failure Map</dt><dd>Table of agent &times; check showing the failure rate for each individual rubric check. Pinpoints which specific behaviours are weakest.</dd>
<dt>&#9317; Latency Comparison</dt><dd>Grouped bar showing p50 and p95 wall-clock response times per agent. Captures both typical and tail latency.</dd>
<dt>&#9318; Variation Matrix</dt><dd>Compact pass/fail grid for every agent &times; variation slot. Shows consistency and which specific variations break agents.</dd>
<dt>Responses</dt><dd>Full prompt + agent response per variation with per-agent toggle filters. Expand the checks list on any card to see every rubric result. When LLM judging was enabled a ★ overall/5 badge appears on each card. Hover it to see the accuracy, completeness, and clarity sub-scores plus the judge&rsquo;s reasoning.</dd>
<dt>Head-to-Head</dt><dd>Side-by-side comparison of any two selected agents across all scenarios and variations. Shows each agent&rsquo;s score, full response text, failing checks, and LLM judge score for every variation. Use it to pinpoint exactly where agents diverge and which agent handles a specific prompt better.</dd>
</dl>
</div>

</div><!-- /about-col right -->

</div><!-- /about-grid -->"#,
        agent_count    = agent_count,
        scenario_count = scenario_count,
        trials         = trials,
        timestamp      = timestamp,
        run_id         = run_id,
        scenarios_html = scenarios_html,
    )
}

// ─── Models & Cost panel ──────────────────────────────────────────────────────

fn build_models_cost_panel(_report: &ComparisonReport) -> String {
    r#"<div class="chart-wrap">
<h2>Cost Leaderboard by Profile</h2>
<canvas id="chart-cost-profile"></canvas>
</div>

<div class="chart-wrap">
<h2>Cost Leaderboard by Family</h2>
<canvas id="chart-cost-family"></canvas>
</div>

<div class="chart-wrap">
<div class="cost-hdr">
  <h2>Cost by Agent Profile</h2>
  <button class="cost-refresh-btn" onclick="initModels(true)">Refresh Pricing</button>
</div>
<table class="cost-table">
<thead><tr><th>Agent</th><th>Model</th><th class="right">Est. Input Tokens</th><th class="right">Est. Output Tokens</th><th class="right">Est. Cost (USD)</th></tr></thead>
<tbody id="cost-agent-body"><tr><td colspan="5" style="color:var(--muted);text-align:center;padding:1rem">Loading...</td></tr></tbody>
</table>
<p class="cost-note">Costs cover all trial runs: input tokens = prompt tokens &times; trial count; output tokens = sum across all trial responses (~4 chars per token). <span id="cost-pricing-source"></span></p>
</div>

<div class="chart-wrap">
<h2>Cost by Model</h2>
<table class="cost-table">
<thead><tr><th>Model</th><th class="right">Est. Input Tokens</th><th class="right">Est. Output Tokens</th><th class="right">Est. Cost (USD)</th></tr></thead>
<tbody id="cost-model-body"></tbody>
</table>
<p class="cost-note">Pricing fetched live from OpenRouter when available; falls back to hardcoded estimates. Judge model costs are shown separately below.</p>
</div>

<div id="cost-judge-wrap" style="display:none" class="chart-wrap">
<h2>LLM Judge Cost</h2>
<table class="cost-table">
<thead><tr><th>Agent</th><th class="right">Est. Input Tokens</th><th class="right">Est. Output Tokens</th><th class="right">Est. Cost (USD)</th></tr></thead>
<tbody id="cost-judge-body"></tbody>
</table>
<p class="cost-note" id="cost-judge-note">Judge cost tracked separately and excluded from agent totals.</p>
</div>"#.to_string()
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

(function(){{
  const suffixes=['-full','-iterative','-sandboxed'];
  function stripSuffix(id){{
    for(const s of suffixes){{if(id.endsWith(s))return id.slice(0,id.length-s.length);}}
    return id;
  }}
  const families=[...new Set(DATA.leaderboard.map(r=>stripSuffix(r.agent_id)))];
  const familyScores=families.map(fam=>{{
    const profiles=DATA.leaderboard.filter(r=>stripSuffix(r.agent_id)===fam);
    if(!profiles.length)return null;
    const mean=profiles.reduce((a,r)=>a+r.overall_score,0)/profiles.length;
    return Math.round(mean*1000)/10;
  }});
  const familyColors=familyScores.map(s=>{{
    if(s===null)return '#94a3b8';
    const hue=Math.round((s/100)*120);
    return `hsl(${{hue}},60%,45%)`;
  }});
  new Chart(document.getElementById('chart-family'),{{
    type:'bar',
    data:{{
      labels:families,
      datasets:[{{label:'Family Score (%)',data:familyScores,backgroundColor:familyColors,borderWidth:0}}]
    }},
    options:{{
      responsive:true,
      plugins:{{legend:{{display:false}}}},
      scales:{{y:{{min:0,max:100,ticks:{{callback:v=>v+'%'}}}}}}
    }}
  }});
}})();

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
    format!("hsl({hue},60%,90%)")
}

fn failure_rate_bg_css(rate: f32) -> String {
    let hue = ((1.0 - rate) * 120.0).round() as u32;
    format!("hsl({hue},60%,90%)")
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
    let inner: Vec<String> = items.map(js_string).collect();
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
            let short = scenario.split('_').next().unwrap_or(scenario);
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
