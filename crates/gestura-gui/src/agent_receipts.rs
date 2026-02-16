//! Agent receipt tracing utilities.
//!
//! In debug mode, agent windows can emit an `agent-debug-receipt` event back to the backend.
//! This module records those receipts in-memory so we can correlate:
//! - what the backend *attempted* to emit (`agent_events` trace), and
//! - what each webview actually *received* (this module).
//!
//! This is intentionally best-effort and diagnostics-only.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// Maximum number of receipt trace entries kept in memory.
const AGENT_RECEIPT_TRACE_MAX: usize = 1000;

/// A single "receipt" record emitted from a frontend webview.
#[derive(Clone, Debug, Serialize)]
pub struct AgentReceiptTraceEntry {
    /// Unix epoch milliseconds.
    pub ts_ms: u128,
    /// Best-effort window label of the receiving webview.
    pub window_label: Option<String>,
    /// Session id of the receiving webview (typically from `?session_id=...`).
    pub session_id: Option<String>,
    /// Event that was received (e.g., `agent-stream-chunk`).
    pub event_name: String,
    /// Session id embedded in the payload (if present).
    pub incoming_session_id: Option<String>,
    /// Whether the frontend accepted the event for rendering.
    pub accept: bool,
    /// If not accepted, a short reason (e.g., `session_mismatch`).
    pub reason: Option<String>,
    /// Listener mode used by the frontend (e.g., `webview.listen` vs `event.listen`).
    pub listener_mode: Option<String>,
    /// Small preview of the original JSON payload (best-effort).
    pub payload_preview: String,
}

#[derive(Debug, Deserialize)]
struct AgentReceiptWire {
    #[serde(default, alias = "windowLabel")]
    window_label: Option<String>,
    #[serde(default, alias = "sessionId")]
    session_id: Option<String>,
    #[serde(default, alias = "eventName")]
    event_name: Option<String>,
    #[serde(default, alias = "incomingSessionId")]
    incoming_session_id: Option<String>,
    #[serde(default)]
    accept: Option<bool>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default, alias = "listenerMode")]
    listener_mode: Option<String>,
}

static AGENT_RECEIPT_TRACE: OnceLock<Mutex<VecDeque<AgentReceiptTraceEntry>>> = OnceLock::new();

fn trace_store() -> &'static Mutex<VecDeque<AgentReceiptTraceEntry>> {
    AGENT_RECEIPT_TRACE.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Clears the in-memory agent receipt trace.
pub fn clear_agent_receipt_trace() {
    let mut store = trace_store().lock().unwrap();
    store.clear();
}

/// Returns the most recent agent receipt trace entries.
///
/// If `max` is `None`, returns up to the most recent `AGENT_RECEIPT_TRACE_MAX` entries.
pub fn get_agent_receipt_trace(max: Option<usize>) -> Vec<AgentReceiptTraceEntry> {
    let store = trace_store().lock().unwrap();
    let take = max.unwrap_or(AGENT_RECEIPT_TRACE_MAX).min(store.len());
    store.iter().rev().take(take).cloned().collect::<Vec<_>>()
}

/// Records a frontend receipt payload (JSON string) into the in-memory trace.
///
/// The frontend should emit `agent-debug-receipt` with a JSON payload.
pub fn record_agent_receipt_payload(payload: &str) {
    let preview = payload.trim();
    // Note: truncate by character count to avoid UTF-8 slicing panics.
    let payload_preview = crate::text_utils::truncate_utf8(preview, 240);

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    let entry = match serde_json::from_str::<AgentReceiptWire>(payload) {
        Ok(w) => AgentReceiptTraceEntry {
            ts_ms: now_ms,
            window_label: w.window_label,
            session_id: w.session_id,
            event_name: w
                .event_name
                .unwrap_or_else(|| "(missing_event_name)".to_string()),
            incoming_session_id: w.incoming_session_id,
            accept: w.accept.unwrap_or(false),
            reason: w.reason,
            listener_mode: w.listener_mode,
            payload_preview,
        },
        Err(err) => AgentReceiptTraceEntry {
            ts_ms: now_ms,
            window_label: None,
            session_id: None,
            event_name: "(parse_error)".to_string(),
            incoming_session_id: None,
            accept: false,
            reason: Some(format!("parse_error: {err}")),
            listener_mode: None,
            payload_preview,
        },
    };

    let mut store = trace_store().lock().unwrap();
    store.push_back(entry);
    while store.len() > AGENT_RECEIPT_TRACE_MAX {
        store.pop_front();
    }
}

