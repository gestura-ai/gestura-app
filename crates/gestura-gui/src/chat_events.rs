//! Chat event emission utilities.
//!
//! This module centralizes how chat streaming events are emitted so we can:
//! - ensure events are *window-scoped* (no accidental global broadcast)
//! - attach `session_id` for defense-in-depth filtering on the frontend
//! - record a small in-memory trace for debugging cross-window leakage.

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use tauri::Emitter;

/// Maximum number of chat event trace entries kept in memory.
const CHAT_EVENT_TRACE_MAX: usize = 500;

/// A single emitted chat event record.
#[derive(Clone, Debug, Serialize)]
pub struct ChatEventTraceEntry {
    /// Unix epoch milliseconds.
    pub ts_ms: u128,
    /// Event name (e.g., `chat-stream-chunk`).
    pub event: String,
    /// Window label that the backend attempted to emit to.
    pub target_window_label: String,
    /// Window label that actually received the event (target or fallback).
    pub emitted_to_window_label: String,
    /// Session id attached to the payload (when available).
    pub session_id: Option<String>,
    /// Truncated JSON payload preview (best-effort).
    pub payload_preview: String,
}

static CHAT_EVENT_TRACE: OnceLock<Mutex<VecDeque<ChatEventTraceEntry>>> = OnceLock::new();

fn trace_store() -> &'static Mutex<VecDeque<ChatEventTraceEntry>> {
    CHAT_EVENT_TRACE.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Clears the in-memory chat event trace.
pub fn clear_chat_event_trace() {
    let mut store = trace_store().lock().unwrap();
    store.clear();
}

/// Returns the most recent chat event trace entries.
///
/// If `max` is `None`, returns up to the most recent `CHAT_EVENT_TRACE_MAX` entries.
pub fn get_chat_event_trace(max: Option<usize>) -> Vec<ChatEventTraceEntry> {
    let store = trace_store().lock().unwrap();
    let take = max.unwrap_or(CHAT_EVENT_TRACE_MAX).min(store.len());
    store.iter().rev().take(take).cloned().collect::<Vec<_>>()
}

/// Adds a `session_id` field to a payload when possible.
///
/// - If `session_id` is `None`, returns the payload unchanged.
/// - If `payload` is a JSON object and it does not already include `session_id`, inserts it.
/// - Otherwise, wraps the payload into `{ session_id, value }`.
pub fn attach_session_id(
    payload: serde_json::Value,
    session_id: Option<&str>,
) -> serde_json::Value {
    let Some(session_id) = session_id else {
        return payload;
    };

    match payload {
        serde_json::Value::Object(mut map) => {
            map.entry("session_id".to_string())
                .or_insert_with(|| serde_json::json!(session_id));
            serde_json::Value::Object(map)
        }
        other => serde_json::json!({
            "session_id": session_id,
            "value": other
        }),
    }
}

/// Emit a chat-related event to a specific window label using `emit_to` (window-scoped).
///
/// If emitting to `target_window_label` fails, this falls back to emitting to
/// `fallback_window_label`.
///
/// Returns the final window label the event was emitted to on success.
pub fn emit_chat_event_to_window(
    app: &tauri::AppHandle,
    target_window_label: &str,
    fallback_window_label: &str,
    event: &str,
    payload: &serde_json::Value,
    session_id: Option<&str>,
) -> Result<String, String> {
    let emitted_to = match app.emit_to(target_window_label, event, payload) {
        Ok(()) => target_window_label.to_string(),
        Err(err) => {
            tracing::warn!(
                event = %event,
                target_window_label = %target_window_label,
                fallback_window_label = %fallback_window_label,
                error = %err,
                "emit_to target failed; falling back"
            );
            app.emit_to(fallback_window_label, event, payload)
                .map_err(|e| format!("emit_to fallback failed: {e}"))?;
            fallback_window_label.to_string()
        }
    };

    // Record a small trace entry to help debug cross-window issues.
    // Note: we truncate by character count to avoid UTF-8 slicing panics.
    let payload_preview = crate::text_utils::truncate_utf8(&payload.to_string(), 240);

    let entry = ChatEventTraceEntry {
        ts_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        event: event.to_string(),
        target_window_label: target_window_label.to_string(),
        emitted_to_window_label: emitted_to.clone(),
        session_id: session_id.map(|s| s.to_string()),
        payload_preview,
    };

    let mut store = trace_store().lock().unwrap();
    store.push_back(entry);
    while store.len() > CHAT_EVENT_TRACE_MAX {
        store.pop_front();
    }

    Ok(emitted_to)
}
