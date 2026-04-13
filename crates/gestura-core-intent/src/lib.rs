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
///
/// Uses [`find_sentence_boundary`] for dot handling, which avoids truncating
/// on non-sentence dots such as filenames (`foo.rs`), version numbers (`1.5`),
/// and URLs (`example.com`).
fn extract_primary_action(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let end = find_sentence_boundary(trimmed);
    let first_sentence = trimmed[..end].trim();

    // Limit to a reasonable length for the action label.
    first_sentence.chars().take(128).collect()
}

/// Return the byte offset of the first sentence boundary in `text`.
///
/// `!`, `?`, and `\n` are **unconditional** sentence terminators.
///
/// `.` is a sentence terminator **only** when the immediately following
/// character is ASCII whitespace (`' '`, `'\t'`, `'\r'`) or there is no
/// following character (end of string).  This prevents false splits on:
///
/// - filenames       (`foo.rs`, `lib.rs`, `Cargo.toml`)
/// - version numbers (`1.5`, `2.0.1`)
/// - URLs            (`https://example.com/path`)
/// - method calls    (`vec.push()`)
///
/// # Implementation note
///
/// The scan operates on raw bytes.  This is safe for UTF-8 because the
/// sentinel characters (`.`, `!`, `?`, `\n`, space, tab, CR) are all
/// single-byte ASCII values that never appear in the continuation bytes of
/// multi-byte UTF-8 sequences.  The returned index is therefore always a
/// valid UTF-8 character boundary.
fn find_sentence_boundary(text: &str) -> usize {
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'!' | b'?' | b'\n' => return i,
            b'.' => {
                // Treat as sentence end only when followed by whitespace or
                // end-of-string; a word/digit/symbol after the dot means it is
                // part of a filename, number, URL, or similar non-sentence context.
                match bytes.get(i + 1) {
                    None | Some(b' ') | Some(b'\t') | Some(b'\r') => return i,
                    _ => {} // dot is embedded in a word/number — not a sentence end
                }
            }
            _ => {}
        }
    }
    text.len() // no boundary found — whole text is the first "sentence"
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
        // `find_filler_in_original` returns byte ranges measured in the
        // *original* string so that slicing is always at valid UTF-8
        // char boundaries — see its doc-comment for why a naïve
        // `haystack.to_lowercase().find(needle)` is unsafe here.
        let pattern_lower = filler.to_lowercase();
        while let Some((pos, end)) = find_filler_in_original(&result, &pattern_lower) {
            result = format!("{}{}", &result[..pos], &result[end..]);
        }
    }
    // Collapse multiple spaces.
    collapse_whitespace(&result)
}

/// Find `needle` (already lower-cased ASCII) inside `haystack`
/// case-insensitively, returning `(start_byte, end_byte)` measured in the
/// **original** `haystack`.
///
/// # Why not `haystack.to_lowercase().find(needle)`?
///
/// `to_lowercase()` can change the UTF-8 byte length of a character.
/// For example, U+0130 `İ` (2 bytes) lowercases to `i` + U+0307 combining
/// dot (3 bytes). A byte offset found inside the lowercased copy therefore
/// does not point to the same position in the original string — using it
/// directly to slice `haystack` can panic (non-char boundary) or silently
/// produce wrong output.
///
/// This function iterates over `haystack.char_indices()` so that every
/// position it returns is a guaranteed valid char boundary of `haystack`,
/// regardless of what case-folding does to individual characters.
fn find_filler_in_original(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return Some((0, 0));
    }

    // Collect the needle chars once (needle is always lower-cased ASCII).
    let needle_chars: Vec<char> = needle.chars().collect();
    let needle_len = needle_chars.len();

    // Collect (byte_offset, char) pairs from the *original* string.
    // Using char_indices guarantees that `start` and `end` are always on
    // valid UTF-8 char boundaries of `haystack`.
    let chars: Vec<(usize, char)> = haystack.char_indices().collect();

    'outer: for i in 0..chars.len() {
        if chars.len() - i < needle_len {
            break;
        }
        for j in 0..needle_len {
            let hc = chars[i + j].1;
            let nc = needle_chars[j]; // already lower-case
            // Lower-case the haystack char and compare the full scalar
            // sequence so multi-scalar lower-case expansions (e.g. 'ﬁ' →
            // 'f','i') never accidentally match a single needle char.
            if !hc.to_lowercase().eq(std::iter::once(nc)) {
                continue 'outer;
            }
        }
        let start = chars[i].0;
        let end = chars
            .get(i + needle_len)
            .map_or(haystack.len(), |&(b, _)| b);
        return Some((start, end));
    }
    None
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

    // -----------------------------------------------------------------------
    // strip_fillers / find_filler_in_original
    // -----------------------------------------------------------------------

    #[test]
    fn strip_fillers_removes_ascii_filler_case_insensitively() {
        // All-ASCII path: basic case-insensitive removal.
        // Note: only the exact filler token is removed; adjacent punctuation
        // (commas, etc.) is preserved by design.
        assert_eq!(
            strip_fillers("Um please open the file"),
            "please open the file"
        );
        assert_eq!(
            strip_fillers("like please open the file"),
            "please open the file"
        );
        assert_eq!(strip_fillers("UM like uh do it"), "do it");
    }

    #[test]
    fn strip_fillers_preserves_non_filler_content() {
        let input = "Create a new Rust project";
        assert_eq!(strip_fillers(input), input);
    }

    #[test]
    fn strip_fillers_with_non_ascii_prefix_does_not_panic() {
        // U+0130 İ (2 UTF-8 bytes) lowercases to 'i' + U+0307 (3 bytes).
        // A naïve `haystack.to_lowercase().find(needle)` returns an offset
        // inside the lowercased string that is 1 byte ahead of the real
        // position in the original, potentially slicing in the middle of the
        // İ codepoint and causing a panic.  The fixed implementation must not
        // panic and must remove the filler word correctly.
        let input = "İ um please do this";
        let result = strip_fillers(input);
        // The filler "um" must be removed; the non-ASCII prefix must survive.
        assert!(
            !result.contains("um"),
            "filler 'um' should be removed, got: {result:?}"
        );
        assert!(
            result.contains('İ'),
            "non-ASCII prefix should be preserved, got: {result:?}"
        );
    }

    #[test]
    fn strip_fillers_with_non_ascii_interleaved_does_not_panic() {
        // Mix of non-ASCII chars around a filler word.
        // "Ñoño" contains no filler substrings, so only "um" is stripped.
        // (Note: "über" is intentionally avoided here because it contains
        // "er", which IS in VOICE_FILLER_WORDS and would also be removed.)
        let input = "Ñoño um test";
        let result = strip_fillers(input);
        assert!(
            !result.contains("um"),
            "filler 'um' should be removed, got: {result:?}"
        );
        assert!(
            result.contains("Ñoño"),
            "non-ASCII word should survive, got: {result:?}"
        );
    }

    #[test]
    fn find_filler_returns_valid_original_byte_range() {
        // Verify that the returned (start, end) range is always a valid slice
        // of the original haystack, even with a non-ASCII prefix.
        let haystack = "İ um test"; // İ = 2 bytes
        let needle = "um";
        let (start, end) = find_filler_in_original(haystack, needle).expect("should find 'um'");
        // Slicing at these offsets must not panic.
        let before = &haystack[..start];
        let after = &haystack[end..];
        assert!(before.contains('İ'));
        assert_eq!(after.trim(), "test");
    }

    #[test]
    fn find_filler_returns_none_when_not_present() {
        assert!(find_filler_in_original("hello world", "um").is_none());
    }

    #[test]
    fn find_filler_empty_needle_returns_zero_range() {
        assert_eq!(find_filler_in_original("hello", ""), Some((0, 0)));
    }

    // -----------------------------------------------------------------------
    // extract_primary_action / find_sentence_boundary
    // -----------------------------------------------------------------------

    #[test]
    fn extract_primary_action_does_not_split_on_filename_dot() {
        // Dots embedded in filenames must NOT be treated as sentence terminators.
        assert_eq!(
            extract_primary_action("please create foo.rs and add tests"),
            "please create foo.rs and add tests",
        );
        assert_eq!(
            extract_primary_action("open lib.rs for editing"),
            "open lib.rs for editing",
        );
        assert_eq!(
            extract_primary_action("edit Cargo.toml to add the dependency"),
            "edit Cargo.toml to add the dependency",
        );
    }

    #[test]
    fn extract_primary_action_does_not_split_on_version_number() {
        assert_eq!(
            extract_primary_action("upgrade to version 1.5 of the SDK"),
            "upgrade to version 1.5 of the SDK",
        );
        assert_eq!(
            extract_primary_action("pin gestura-core to 2.0.1 in Cargo.toml"),
            "pin gestura-core to 2.0.1 in Cargo.toml",
        );
    }

    #[test]
    fn extract_primary_action_does_not_split_on_url() {
        assert_eq!(
            extract_primary_action("visit https://example.com/path for the docs"),
            "visit https://example.com/path for the docs",
        );
    }

    #[test]
    fn extract_primary_action_does_not_split_on_method_call_dot() {
        assert_eq!(
            extract_primary_action("call vec.push() and return the result"),
            "call vec.push() and return the result",
        );
    }

    #[test]
    fn extract_primary_action_splits_on_sentence_ending_dot() {
        // A dot followed by a space IS a sentence terminator.
        assert_eq!(
            extract_primary_action("Fix the bug. Add tests afterwards."),
            "Fix the bug",
        );
    }

    #[test]
    fn extract_primary_action_splits_on_dot_at_end_of_string() {
        // A dot at the very end of the string (no following char) is a sentence end.
        // The result preserves the filename inside the sentence up to the final dot.
        assert_eq!(extract_primary_action("Check file.rs."), "Check file.rs",);
    }

    #[test]
    fn extract_primary_action_splits_on_exclamation_and_question() {
        assert_eq!(
            extract_primary_action("Do it now! Please hurry."),
            "Do it now",
        );
        assert_eq!(
            extract_primary_action("What should I do? Maybe this."),
            "What should I do",
        );
    }

    #[test]
    fn extract_primary_action_splits_on_newline() {
        assert_eq!(
            extract_primary_action("First line\nSecond line"),
            "First line",
        );
    }

    #[test]
    fn extract_primary_action_no_punctuation_returns_whole_text() {
        let input = "update the authentication module to use OAuth2";
        assert_eq!(extract_primary_action(input), input);
    }

    #[test]
    fn extract_primary_action_caps_at_128_chars() {
        let long_input = "a".repeat(200);
        let result = extract_primary_action(&long_input);
        assert_eq!(result.chars().count(), 128);
    }

    #[test]
    fn find_sentence_boundary_dot_before_non_ascii_is_not_a_boundary() {
        // A multi-byte UTF-8 char after '.' (e.g. 'ü' = 0xC3 0xBC) must not
        // be mistaken for whitespace; the continuation byte 0xBF is > 0x7F.
        assert_eq!(extract_primary_action("foo.über alles"), "foo.über alles",);
    }
}
