//! Unified intent normalization layer for Gestura.
//!
//! `gestura-core-intent` converts every input modality—voice transcripts, chat
//! text, ring gesture data (IMU taps/tilts), and future inputs—into one
//! consistent [`Intent`] struct before any agentic processing.
//!
//! ## Design role
//!
//! This crate sits immediately after raw input capture and immediately before
//! the pipeline's planning/execution phase. It ensures that downstream
//! orchestration, tool selection, and response generation all operate on a
//! single, modality-agnostic representation.
//!
//! ## Feature gating
//!
//! Intent normalization is gated behind `advanced-primitives`. When the feature
//! is disabled the [`INTENT_NORMALIZATION_ENABLED`] constant is `false` and the
//! middleware branch in the pipeline constant-folds away, preserving the
//! original agentic loop behavior.
//!
//! ## Stable import paths
//!
//! Application code should import through the facade:
//!
//! - `gestura_core::intent::*`

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use gestura_core_pipeline::RequestSource;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Compile-time flag exported to downstream crates so the normalization branch
/// can constant-fold away when `advanced-primitives` is disabled.
pub const INTENT_NORMALIZATION_ENABLED: bool = cfg!(feature = "advanced-primitives");

// ---------------------------------------------------------------------------
// InputModality
// ---------------------------------------------------------------------------

/// Input modality that produced the raw input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputModality {
    /// Transcribed voice input from microphone / STT.
    Voice,
    /// Typed text input from GUI or CLI.
    Chat,
    /// Gesture input from the Haptic Harmony ring (IMU taps/tilts).
    Gesture,
    /// Any future input modality not yet enumerated.
    Future(String),
}

impl InputModality {
    /// Derive an [`InputModality`] from the pipeline's [`RequestSource`].
    pub fn from_request_source(source: &RequestSource) -> Self {
        match source {
            RequestSource::GuiVoice => Self::Voice,
            RequestSource::GuiText | RequestSource::CliTui | RequestSource::CliBasic => Self::Chat,
            // Orchestrator-initiated requests are treated as chat since they
            // originate from structured text delegation.
            RequestSource::Orchestrator | RequestSource::Unknown => Self::Chat,
        }
    }

    /// Short label for telemetry and metadata hints.
    pub fn label(&self) -> &str {
        match self {
            Self::Voice => "voice",
            Self::Chat => "chat",
            Self::Gesture => "gesture",
            Self::Future(name) => name.as_str(),
        }
    }
}

// ---------------------------------------------------------------------------
// GestureData
// ---------------------------------------------------------------------------

/// Optional IMU gesture data from the Haptic Harmony ring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GestureData {
    /// Gesture type identifier (e.g. `"tap"`, `"double_tap"`, `"tilt_left"`).
    pub gesture_type: String,
    /// Optional raw IMU acceleration values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceleration: Option<[f32; 3]>,
    /// Optional raw IMU gyroscope values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gyroscope: Option<[f32; 3]>,
    /// Gesture confidence from the on-device classifier (0.0–1.0).
    #[serde(default = "default_gesture_confidence")]
    pub confidence: f32,
}

fn default_gesture_confidence() -> f32 {
    0.9
}

// ---------------------------------------------------------------------------
// RawInput
// ---------------------------------------------------------------------------

/// Raw input envelope handed to the normalization layer.
#[derive(Debug, Clone)]
pub struct RawInput {
    /// The primary text payload (transcription, typed message, or gesture label).
    pub text: String,
    /// Detected or declared input modality.
    pub modality: InputModality,
    /// Session identifier if the request is already session-scoped.
    pub session_id: Option<String>,
    /// Optional structured gesture data from the Haptic Harmony ring.
    pub gesture_data: Option<GestureData>,
}

// ---------------------------------------------------------------------------
// Intent
// ---------------------------------------------------------------------------

/// Unified, modality-agnostic intent produced by the normalization layer.
///
/// Every input—voice, chat, gesture, or future modality—is converted into an
/// `Intent` before any agentic processing, ensuring a single consistent
/// representation throughout the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    /// Unique intent identifier (UUID v4).
    pub id: String,
    /// Timestamp when the intent was normalized.
    pub timestamp: DateTime<Utc>,
    /// Input modality that produced this intent.
    pub modality: InputModality,
    /// Original raw input preserved for debugging and audit.
    pub raw_source: String,
    /// Extracted primary action or command.
    pub primary_action: String,
    /// Structured parameters extracted from the input.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub parameters: HashMap<String, serde_json::Value>,
    /// Normalization confidence score (0.0–1.0).
    pub confidence: f32,
    /// Contextual hints derived during normalization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_hints: Vec<String>,
}

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

/// Normalize a [`RawInput`] into a unified [`Intent`].
///
/// This is the single public entry point for intent normalization. It
/// dispatches to modality-specific logic internally.
pub fn normalize_input_to_intent(raw_input: RawInput) -> Intent {
    match &raw_input.modality {
        InputModality::Voice => normalize_voice(raw_input),
        InputModality::Chat => normalize_chat(raw_input),
        InputModality::Gesture => normalize_gesture(raw_input),
        InputModality::Future(_) => normalize_future(raw_input),
    }
}

/// Extract the primary action from the first sentence of text.
fn extract_primary_action(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Take the first sentence (split on sentence-ending punctuation or newline).
    let first_sentence = trimmed
        .split(['.', '!', '?', '\n'])
        .next()
        .unwrap_or(trimmed)
        .trim();

    // Limit to a reasonable length for the action label.
    first_sentence.chars().take(128).collect()
}

// ---- Voice normalization ----

/// Filler words commonly produced by STT engines that carry no semantic value.
const VOICE_FILLER_WORDS: &[&str] = &[
    "um",
    "uh",
    "er",
    "ah",
    "like",
    "you know",
    "so basically",
    "basically",
    "I mean",
    "well",
    "okay so",
    "right so",
];

fn strip_fillers(text: &str) -> String {
    let mut result = text.to_string();
    for filler in VOICE_FILLER_WORDS {
        // Case-insensitive removal; collapse resulting double-spaces.
        let pattern_lower = filler.to_lowercase();
        // Build a simple case-insensitive replacement.
        let lower = result.to_lowercase();
        while let Some(pos) = lower_find(&result, &pattern_lower) {
            let end = pos + filler.len();
            result = format!("{}{}", &result[..pos], &result[end..]);
        }
    }
    // Collapse multiple spaces.
    collapse_whitespace(&result)
}

fn lower_find(haystack: &str, needle: &str) -> Option<usize> {
    let lower = haystack.to_lowercase();
    lower.find(needle)
}

fn collapse_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_was_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_was_space && !result.is_empty() {
                result.push(' ');
            }
            last_was_space = true;
        } else {
            result.push(ch);
            last_was_space = false;
        }
    }
    result.trim().to_string()
}

fn voice_confidence(text: &str) -> f32 {
    let word_count = text.split_whitespace().count();
    if word_count == 0 {
        return 0.0;
    }
    // Heuristic: very short transcripts may be noisy.
    if word_count < 3 {
        0.6
    } else if word_count < 8 {
        0.75
    } else {
        0.85
    }
}

fn normalize_voice(raw: RawInput) -> Intent {
    let cleaned = strip_fillers(&raw.text);
    let primary_action = extract_primary_action(&cleaned);
    let confidence = voice_confidence(&cleaned);

    let mut context_hints = Vec::new();
    context_hints.push("source:voice_transcript".to_string());
    if cleaned.len() < raw.text.len() {
        context_hints.push("fillers_stripped".to_string());
    }

    Intent {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        modality: InputModality::Voice,
        raw_source: raw.text,
        primary_action,
        parameters: HashMap::new(),
        confidence,
        context_hints,
    }
}

// ---- Chat normalization ----

fn normalize_chat(raw: RawInput) -> Intent {
    let trimmed = raw.text.trim().to_string();
    let primary_action = extract_primary_action(&trimmed);

    Intent {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        modality: InputModality::Chat,
        raw_source: raw.text,
        primary_action,
        parameters: HashMap::new(),
        confidence: 0.95,
        context_hints: vec!["source:chat_text".to_string()],
    }
}

// ---- Gesture normalization ----

/// Map well-known gesture types to semantic action labels.
fn gesture_to_action(gesture_type: &str) -> (&'static str, f32) {
    match gesture_type.to_lowercase().as_str() {
        "tap" => ("confirm", 0.9),
        "double_tap" => ("execute", 0.92),
        "triple_tap" => ("cancel", 0.88),
        "tilt_left" => ("previous", 0.85),
        "tilt_right" => ("next", 0.85),
        "tilt_up" => ("scroll_up", 0.8),
        "tilt_down" => ("scroll_down", 0.8),
        "twist_cw" => ("increase", 0.82),
        "twist_ccw" => ("decrease", 0.82),
        "shake" => ("dismiss", 0.78),
        "hold" => ("select", 0.88),
        _ => ("unknown_gesture", 0.5),
    }
}

fn normalize_gesture(raw: RawInput) -> Intent {
    let gesture_type = raw
        .gesture_data
        .as_ref()
        .map(|g| g.gesture_type.as_str())
        .unwrap_or_else(|| raw.text.trim());

    let device_confidence = raw
        .gesture_data
        .as_ref()
        .map(|g| g.confidence)
        .unwrap_or(0.9);

    let (action, mapping_confidence) = gesture_to_action(gesture_type);

    // Combined confidence: device classifier × mapping certainty.
    let confidence = device_confidence * mapping_confidence;

    let mut parameters = HashMap::new();
    parameters.insert(
        "gesture_type".to_string(),
        serde_json::Value::String(gesture_type.to_string()),
    );

    if let Some(ref gesture) = raw.gesture_data {
        if let Some(accel) = gesture.acceleration {
            parameters.insert("acceleration".to_string(), serde_json::json!(accel));
        }
        if let Some(gyro) = gesture.gyroscope {
            parameters.insert("gyroscope".to_string(), serde_json::json!(gyro));
        }
    }

    let mut context_hints = vec!["source:gesture_ring".to_string()];
    if action == "unknown_gesture" {
        context_hints.push("unmapped_gesture".to_string());
    }

    Intent {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        modality: InputModality::Gesture,
        raw_source: raw.text,
        primary_action: action.to_string(),
        parameters,
        confidence,
        context_hints,
    }
}

// ---- Future modality ----

fn normalize_future(raw: RawInput) -> Intent {
    let primary_action = extract_primary_action(&raw.text);

    Intent {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        modality: raw.modality.clone(),
        raw_source: raw.text,
        primary_action,
        parameters: HashMap::new(),
        confidence: 0.7,
        context_hints: vec!["source:future_modality".to_string()],
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_produces_valid_intent() {
        let raw = RawInput {
            text: "Um, like, please create a new file called foo.rs".to_string(),
            modality: InputModality::Voice,
            session_id: Some("session-1".to_string()),
            gesture_data: None,
        };

        let intent = normalize_input_to_intent(raw);

        assert_eq!(intent.modality, InputModality::Voice);
        assert!(!intent.id.is_empty());
        assert!(!intent.primary_action.is_empty());
        // Fillers should be stripped from the action.
        assert!(
            !intent.primary_action.to_lowercase().contains("um,"),
            "Filler 'um' should be stripped"
        );
        assert!(intent.confidence > 0.0 && intent.confidence <= 1.0);
        assert!(
            intent
                .context_hints
                .contains(&"source:voice_transcript".to_string())
        );
        assert!(intent.raw_source.contains("Um")); // raw preserved
    }

    #[test]
    fn chat_produces_valid_intent() {
        let raw = RawInput {
            text: "Refactor the authentication module to use OAuth2".to_string(),
            modality: InputModality::Chat,
            session_id: None,
            gesture_data: None,
        };

        let intent = normalize_input_to_intent(raw);

        assert_eq!(intent.modality, InputModality::Chat);
        assert!(!intent.id.is_empty());
        assert_eq!(
            intent.primary_action,
            "Refactor the authentication module to use OAuth2"
        );
        assert!(
            (intent.confidence - 0.95).abs() < f32::EPSILON,
            "Chat confidence should be 0.95"
        );
        assert!(
            intent
                .context_hints
                .contains(&"source:chat_text".to_string())
        );
    }

    #[test]
    fn gesture_produces_valid_intent() {
        let raw = RawInput {
            text: "double_tap".to_string(),
            modality: InputModality::Gesture,
            session_id: Some("session-2".to_string()),
            gesture_data: Some(GestureData {
                gesture_type: "double_tap".to_string(),
                acceleration: Some([0.1, 9.8, 0.3]),
                gyroscope: None,
                confidence: 0.95,
            }),
        };

        let intent = normalize_input_to_intent(raw);

        assert_eq!(intent.modality, InputModality::Gesture);
        assert_eq!(intent.primary_action, "execute");
        assert!(intent.confidence > 0.8);
        assert!(intent.parameters.contains_key("gesture_type"));
        assert!(intent.parameters.contains_key("acceleration"));
        assert!(
            intent
                .context_hints
                .contains(&"source:gesture_ring".to_string())
        );
    }

    #[test]
    fn gesture_without_data_falls_back_to_text() {
        let raw = RawInput {
            text: "tap".to_string(),
            modality: InputModality::Gesture,
            session_id: None,
            gesture_data: None,
        };

        let intent = normalize_input_to_intent(raw);
        assert_eq!(intent.primary_action, "confirm");
    }

    #[test]
    fn unknown_gesture_has_low_confidence() {
        let raw = RawInput {
            text: "backflip".to_string(),
            modality: InputModality::Gesture,
            session_id: None,
            gesture_data: Some(GestureData {
                gesture_type: "backflip".to_string(),
                acceleration: None,
                gyroscope: None,
                confidence: 0.9,
            }),
        };

        let intent = normalize_input_to_intent(raw);
        assert_eq!(intent.primary_action, "unknown_gesture");
        assert!(intent.confidence < 0.6);
        assert!(
            intent
                .context_hints
                .contains(&"unmapped_gesture".to_string())
        );
    }

    #[test]
    fn future_modality_passes_through() {
        let raw = RawInput {
            text: "Neural signal: focus next element".to_string(),
            modality: InputModality::Future("neural".to_string()),
            session_id: None,
            gesture_data: None,
        };

        let intent = normalize_input_to_intent(raw);
        assert_eq!(intent.modality, InputModality::Future("neural".to_string()));
        assert!(!intent.primary_action.is_empty());
        assert!((intent.confidence - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn voice_chat_gesture_produce_equivalent_structs() {
        // All three produce the same Intent shape — the struct fields are
        // always populated regardless of modality.
        let voice = normalize_input_to_intent(RawInput {
            text: "hello world".to_string(),
            modality: InputModality::Voice,
            session_id: None,
            gesture_data: None,
        });
        let chat = normalize_input_to_intent(RawInput {
            text: "hello world".to_string(),
            modality: InputModality::Chat,
            session_id: None,
            gesture_data: None,
        });
        let gesture = normalize_input_to_intent(RawInput {
            text: "tap".to_string(),
            modality: InputModality::Gesture,
            session_id: None,
            gesture_data: None,
        });

        // All should have non-empty required fields.
        for intent in [&voice, &chat, &gesture] {
            assert!(!intent.id.is_empty());
            assert!(!intent.primary_action.is_empty());
            assert!(intent.confidence > 0.0);
            assert!(!intent.context_hints.is_empty());
        }
    }

    #[test]
    fn modality_from_request_source() {
        assert_eq!(
            InputModality::from_request_source(&RequestSource::GuiVoice),
            InputModality::Voice
        );
        assert_eq!(
            InputModality::from_request_source(&RequestSource::GuiText),
            InputModality::Chat
        );
        assert_eq!(
            InputModality::from_request_source(&RequestSource::CliTui),
            InputModality::Chat
        );
        assert_eq!(
            InputModality::from_request_source(&RequestSource::Orchestrator),
            InputModality::Chat
        );
    }

    #[test]
    fn intent_serialization_roundtrip() {
        let intent = normalize_input_to_intent(RawInput {
            text: "Build the project".to_string(),
            modality: InputModality::Chat,
            session_id: Some("s-1".to_string()),
            gesture_data: None,
        });

        let json = serde_json::to_string(&intent).expect("serialize");
        let parsed: Intent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.id, intent.id);
        assert_eq!(parsed.primary_action, intent.primary_action);
        assert_eq!(parsed.modality, intent.modality);
    }

    #[test]
    fn empty_text_has_zero_voice_confidence() {
        let intent = normalize_input_to_intent(RawInput {
            text: "".to_string(),
            modality: InputModality::Voice,
            session_id: None,
            gesture_data: None,
        });

        assert!((intent.confidence - 0.0).abs() < f32::EPSILON);
    }
}
