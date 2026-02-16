//! Deterministic diagnostics probe for multi-window agent isolation.
//!
//! This module provides a backend-driven probe that emits an `agent-probe` event to
//! multiple agent windows and correlates backend emission trace entries with
//! frontend receipt trace entries. The intent is to make cross-window leakage
//! reproducible and debuggable without relying on real LLM calls.

use serde::Serialize;
use tauri::Manager;

/// The event name used for probe traffic.
pub const AGENT_PROBE_EVENT: &str = "agent-probe";

/// A selected agent window used by the probe.
#[derive(Clone, Debug, Serialize)]
pub struct ProbeWindow {
    pub label: String,
    pub session_id: Option<String>,
}

/// A single leakage finding detected by the probe analysis.
#[derive(Clone, Debug, Serialize)]
pub struct ProbeLeakageFinding {
    pub window_label: Option<String>,
    pub session_id: Option<String>,
    pub incoming_session_id: Option<String>,
    pub accept: bool,
    pub reason: Option<String>,
}

/// Summary analysis of the probe run.
#[derive(Clone, Debug, Serialize)]
pub struct AgentIsolationProbeAnalysis {
    /// Whether the probe observed the expected isolation behavior.
    pub ok: bool,
    /// Any warnings that may indicate instrumentation gaps.
    pub warnings: Vec<String>,
    /// Receipts that indicate possible leakage.
    pub leakage: Vec<ProbeLeakageFinding>,
    /// Receipt counts by window label.
    pub receipt_counts: Vec<ProbeReceiptCount>,
}

/// Receipt counts for the probe event grouped by window label.
#[derive(Clone, Debug, Serialize)]
pub struct ProbeReceiptCount {
    pub window_label: String,
    pub total: usize,
    pub accepted: usize,
    pub ignored: usize,
}

/// Full report returned to the frontend.
#[derive(Clone, Debug, Serialize)]
pub struct AgentIsolationProbeReport {
    pub probe_id: String,
    pub selected_windows: Vec<ProbeWindow>,
    pub emit_trace: Vec<crate::agent_events::AgentEventTraceEntry>,
    pub receipt_trace: Vec<crate::agent_receipts::AgentReceiptTraceEntry>,
    pub analysis: AgentIsolationProbeAnalysis,
}

/// Run a deterministic multi-window agent isolation probe.
///
/// Behavior:
/// - Clears the backend traces (emit + receipt).
/// - Selects two agent windows (labels matching `agent-...`).
/// - Emits `agent-probe` events to each window using window-scoped `emit_to`.
/// - Waits briefly for frontend receipts to arrive.
/// - Returns traces and an analysis summary.
pub async fn run_agent_isolation_probe(
    app: tauri::AppHandle,
) -> Result<AgentIsolationProbeReport, String> {
    crate::agent_events::clear_agent_event_trace();
    crate::agent_receipts::clear_agent_receipt_trace();

    let mut agent_labels: Vec<String> = app
        .webview_windows()
        .keys()
        .filter(|l| l.starts_with("agent-"))
        .cloned()
        .collect();
    agent_labels.sort();

    if agent_labels.len() < 2 {
        return Err(
            "Agent isolation probe requires at least two open agent session windows (labels like agent-{session_id})."
                .to_string(),
        );
    }

    // Deterministically pick the first two.
    let a_label = agent_labels[0].clone();
    let b_label = agent_labels[1].clone();

    let a_session = crate::window_manager::get_session_id_for_window_label(&a_label);
    let b_session = crate::window_manager::get_session_id_for_window_label(&b_label);

    let selected_windows = vec![
        ProbeWindow {
            label: a_label.clone(),
            session_id: a_session.clone(),
        },
        ProbeWindow {
            label: b_label.clone(),
            session_id: b_session.clone(),
        },
    ];

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let probe_id = format!("probe-{now_ms}");

    // Emit a probe event to each target window. We deliberately set the fallback
    // label equal to the target label to avoid emitting to any other window.
    for (idx, w) in selected_windows.iter().enumerate() {
        let payload = serde_json::json!({
            "probe_id": probe_id,
            "probe_seq": idx + 1,
            "target_window_label": w.label,
            "note": "agent isolation probe"
        });
        let payload = crate::agent_events::attach_session_id(payload, w.session_id.as_deref());
        crate::agent_events::emit_agent_event_to_window(
            &app,
            &w.label,
            &w.label,
            AGENT_PROBE_EVENT,
            &payload,
            w.session_id.as_deref(),
        )?;
    }

    // Give the frontend a moment to record receipts (invoke is async).
    tokio::time::sleep(tokio::time::Duration::from_millis(600)).await;

    let emit_trace = crate::agent_events::get_agent_event_trace(Some(200));
    let receipt_trace = crate::agent_receipts::get_agent_receipt_trace(Some(400));
    let analysis = analyze_probe(&selected_windows, &receipt_trace);

    Ok(AgentIsolationProbeReport {
        probe_id,
        selected_windows,
        emit_trace,
        receipt_trace,
        analysis,
    })
}

/// Analyze receipt trace entries for probe-related leakage signals.
fn analyze_probe(
    selected_windows: &[ProbeWindow],
    receipt_trace: &[crate::agent_receipts::AgentReceiptTraceEntry],
) -> AgentIsolationProbeAnalysis {
    let mut warnings = Vec::new();
    let mut leakage = Vec::new();

    let targets: std::collections::HashSet<&str> =
        selected_windows.iter().map(|w| w.label.as_str()).collect();

    let probe_receipts: Vec<&crate::agent_receipts::AgentReceiptTraceEntry> = receipt_trace
        .iter()
        .filter(|r| r.event_name == AGENT_PROBE_EVENT)
        .collect();

    if probe_receipts.is_empty() {
        warnings.push(
            "No probe receipts were recorded. This likely means agent.html is not listening to `agent-probe` or receipt recording is disabled."
                .to_string(),
        );
    }

    // Count receipts by window label.
    let mut counts: std::collections::BTreeMap<String, (usize, usize, usize)> =
        std::collections::BTreeMap::new();
    for r in &probe_receipts {
        let label = r
            .window_label
            .clone()
            .unwrap_or_else(|| "(unknown_window)".to_string());
        let entry = counts.entry(label).or_insert((0, 0, 0));
        entry.0 += 1;
        if r.accept {
            entry.1 += 1;
        } else {
            entry.2 += 1;
        }

        // Leakage signal 1: receipt on a non-target window label.
        if r.window_label
            .as_deref()
            .filter(|wl| !targets.contains(*wl))
            .is_some()
        {
            leakage.push(ProbeLeakageFinding {
                window_label: r.window_label.clone(),
                session_id: r.session_id.clone(),
                incoming_session_id: r.incoming_session_id.clone(),
                accept: r.accept,
                reason: r.reason.clone(),
            });
        }

        // Leakage signal 2: this window received an event tagged for a different session.
        match (&r.session_id, &r.incoming_session_id) {
            (Some(sid), Some(incoming)) if sid != incoming => {
                leakage.push(ProbeLeakageFinding {
                    window_label: r.window_label.clone(),
                    session_id: r.session_id.clone(),
                    incoming_session_id: r.incoming_session_id.clone(),
                    accept: r.accept,
                    reason: r.reason.clone(),
                });
            }
            _ => {}
        }
    }

    // Missing receipt per selected window is an instrumentation gap or delivery failure.
    for w in selected_windows {
        let label = w.label.clone();
        let total = counts.get(&label).map(|t| t.0).unwrap_or(0);
        if total == 0 {
            warnings.push(format!(
                "No `agent-probe` receipt recorded for window {label}. Listener may not be attached yet, or receipt logging is disabled."
            ));
        }
    }

    let receipt_counts = counts
        .into_iter()
        .map(
            |(window_label, (total, accepted, ignored))| ProbeReceiptCount {
                window_label,
                total,
                accepted,
                ignored,
            },
        )
        .collect::<Vec<_>>();

    let ok = leakage.is_empty() && warnings.is_empty();

    AgentIsolationProbeAnalysis {
        ok,
        warnings,
        leakage,
        receipt_counts,
    }
}
