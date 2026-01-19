//! Unified agent interaction model for gesture/tap/tilt + haptics
//!
//! This module defines the data model and injection points for multi-modal
//! interaction with agents. It enables gesture, tap, tilt events and haptic
//! responsiveness to feed agent context and influence tool selection.

use serde::{Deserialize, Serialize};

/// Types of gestures that can trigger agent interactions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GestureType {
    /// Single tap gesture
    Tap,
    /// Double tap gesture
    DoubleTap,
    /// Long press/hold gesture
    Hold { duration_ms: u64 },
    /// Slide gesture with direction
    Slide { direction: SlideDirection },
    /// Tilt gesture with angle (degrees from vertical)
    Tilt { angle: f32 },
    /// Rotation gesture
    Rotate { degrees: f32 },
    /// Shake gesture
    Shake { intensity: f32 },
}

/// Direction for slide gestures
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlideDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Unified interaction event that can trigger agent actions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionEvent {
    /// Type of interaction
    pub interaction_type: InteractionType,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    /// Timestamp in milliseconds since epoch
    pub timestamp_ms: u64,
    /// Source device identifier (e.g., "ring", "keyboard", "touch")
    pub source: String,
    /// Optional metadata for the interaction
    pub metadata: Option<serde_json::Value>,
}

/// Types of interactions that can trigger agent actions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InteractionType {
    /// Gesture from ring or touch device
    Gesture(GestureType),
    /// Voice command
    Voice {
        text: String,
        language: Option<String>,
    },
    /// Hotkey press
    Hotkey { key: String, modifiers: Vec<String> },
    /// Button press on ring
    Button {
        button_id: u8,
        press_type: ButtonPressType,
    },
}

/// Button press types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonPressType {
    Single,
    Double,
    Long,
}

/// Haptic feedback pattern for agent responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HapticFeedback {
    /// Pattern type
    pub pattern: HapticPattern,
    /// Intensity (0.0 - 1.0)
    pub intensity: f32,
    /// Duration in milliseconds
    pub duration_ms: u32,
    /// Repeat count (0 = single, >0 = repeat n times)
    pub repeat_count: u8,
    /// Delay between repeats in milliseconds
    pub repeat_delay_ms: u32,
}

/// Predefined haptic patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HapticPattern {
    /// Quick click feedback
    Click,
    /// Gentle pulse
    Pulse,
    /// Ramping intensity
    Ramp,
    /// Heartbeat pattern
    Heartbeat,
    /// Notification alert
    Notification,
    /// Error/warning alert
    Alert,
    /// Success confirmation
    Success,
    /// Processing/thinking indicator
    Processing,
    /// Custom pattern ID
    Custom(u8),
}

/// Extended agent context with interaction data
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InteractionContext {
    /// Agent identifier
    pub agent_id: String,
    /// Current interaction that triggered this context
    pub current_interaction: Option<InteractionEvent>,
    /// Recent interaction history (for context)
    pub recent_interactions: Vec<InteractionEvent>,
    /// Suggested haptic feedback for response
    pub suggested_haptic: Option<HapticFeedback>,
    /// Tool selection hints based on interaction
    pub tool_hints: Vec<ToolHint>,
    /// Whether voice response is expected
    pub expects_voice_response: bool,
    /// Session identifier for continuity
    pub session_id: Option<String>,
}

/// Hint for tool selection based on interaction context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolHint {
    /// Tool name or category
    pub tool: String,
    /// Priority weight (higher = more likely to be selected)
    pub priority: f32,
    /// Reason for the hint
    pub reason: String,
}

impl InteractionEvent {
    /// Create a new gesture interaction event
    pub fn gesture(gesture: GestureType, source: &str, confidence: f32) -> Self {
        Self {
            interaction_type: InteractionType::Gesture(gesture),
            confidence,
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            source: source.to_string(),
            metadata: None,
        }
    }

    /// Create a new voice interaction event
    pub fn voice(text: &str, source: &str, confidence: f32) -> Self {
        Self {
            interaction_type: InteractionType::Voice {
                text: text.to_string(),
                language: None,
            },
            confidence,
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            source: source.to_string(),
            metadata: None,
        }
    }

    /// Create a hotkey interaction event
    pub fn hotkey(key: &str, modifiers: Vec<String>, source: &str) -> Self {
        Self {
            interaction_type: InteractionType::Hotkey {
                key: key.to_string(),
                modifiers,
            },
            confidence: 1.0,
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            source: source.to_string(),
            metadata: None,
        }
    }
}

impl HapticFeedback {
    /// Quick click feedback
    pub fn click() -> Self {
        Self {
            pattern: HapticPattern::Click,
            intensity: 0.7,
            duration_ms: 50,
            repeat_count: 0,
            repeat_delay_ms: 0,
        }
    }

    /// Notification feedback
    pub fn notification() -> Self {
        Self {
            pattern: HapticPattern::Notification,
            intensity: 0.8,
            duration_ms: 200,
            repeat_count: 0,
            repeat_delay_ms: 0,
        }
    }

    /// Success confirmation feedback
    pub fn success() -> Self {
        Self {
            pattern: HapticPattern::Success,
            intensity: 0.6,
            duration_ms: 150,
            repeat_count: 1,
            repeat_delay_ms: 100,
        }
    }

    /// Alert/error feedback
    pub fn alert() -> Self {
        Self {
            pattern: HapticPattern::Alert,
            intensity: 1.0,
            duration_ms: 300,
            repeat_count: 2,
            repeat_delay_ms: 150,
        }
    }

    /// Processing/thinking indicator
    pub fn processing() -> Self {
        Self {
            pattern: HapticPattern::Processing,
            intensity: 0.4,
            duration_ms: 100,
            repeat_count: 3,
            repeat_delay_ms: 200,
        }
    }
}

impl InteractionContext {
    /// Create a new interaction context for an agent
    pub fn new(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            ..Default::default()
        }
    }

    /// Set the current interaction and derive tool hints
    pub fn with_interaction(mut self, event: InteractionEvent) -> Self {
        self.tool_hints = derive_tool_hints(&event);
        self.suggested_haptic = suggest_haptic_for_interaction(&event);
        self.expects_voice_response =
            matches!(event.interaction_type, InteractionType::Voice { .. });
        self.current_interaction = Some(event);
        self
    }

    /// Add to recent interaction history
    pub fn push_history(&mut self, event: InteractionEvent) {
        const MAX_HISTORY: usize = 10;
        self.recent_interactions.push(event);
        if self.recent_interactions.len() > MAX_HISTORY {
            self.recent_interactions.remove(0);
        }
    }
}

/// Derive tool hints based on interaction type
fn derive_tool_hints(event: &InteractionEvent) -> Vec<ToolHint> {
    let mut hints = Vec::new();

    match &event.interaction_type {
        InteractionType::Gesture(gesture) => match gesture {
            GestureType::DoubleTap => {
                hints.push(ToolHint {
                    tool: "quick_action".to_string(),
                    priority: 0.9,
                    reason: "Double tap suggests quick action intent".to_string(),
                });
            }
            GestureType::Hold { duration_ms } if *duration_ms > 1000 => {
                hints.push(ToolHint {
                    tool: "context_menu".to_string(),
                    priority: 0.8,
                    reason: "Long hold suggests context menu or detailed action".to_string(),
                });
            }
            GestureType::Slide { direction } => {
                let tool = match direction {
                    SlideDirection::Up => "scroll_up",
                    SlideDirection::Down => "scroll_down",
                    SlideDirection::Left => "navigate_back",
                    SlideDirection::Right => "navigate_forward",
                };
                hints.push(ToolHint {
                    tool: tool.to_string(),
                    priority: 0.7,
                    reason: format!("Slide {:?} gesture", direction),
                });
            }
            GestureType::Shake { intensity } if *intensity > 0.5 => {
                hints.push(ToolHint {
                    tool: "cancel".to_string(),
                    priority: 0.85,
                    reason: "Shake gesture suggests cancel/undo intent".to_string(),
                });
            }
            _ => {}
        },
        InteractionType::Voice { text, .. } => {
            // Voice interactions prioritize voice-friendly tools
            hints.push(ToolHint {
                tool: "voice_response".to_string(),
                priority: 0.9,
                reason: "Voice input expects voice output".to_string(),
            });
            if text.to_lowercase().contains("show") || text.to_lowercase().contains("display") {
                hints.push(ToolHint {
                    tool: "visual_display".to_string(),
                    priority: 0.7,
                    reason: "Voice command requests visual output".to_string(),
                });
            }
        }
        InteractionType::Hotkey { key, modifiers } => {
            if modifiers.contains(&"Ctrl".to_string()) || modifiers.contains(&"Cmd".to_string()) {
                hints.push(ToolHint {
                    tool: "keyboard_shortcut".to_string(),
                    priority: 0.8,
                    reason: format!("Hotkey {} with modifiers", key),
                });
            }
        }
        InteractionType::Button { press_type, .. } => match press_type {
            ButtonPressType::Long => {
                hints.push(ToolHint {
                    tool: "voice_input".to_string(),
                    priority: 0.9,
                    reason: "Long button press activates voice input".to_string(),
                });
            }
            ButtonPressType::Double => {
                hints.push(ToolHint {
                    tool: "quick_action".to_string(),
                    priority: 0.8,
                    reason: "Double button press for quick action".to_string(),
                });
            }
            _ => {}
        },
    }

    hints
}

/// Suggest haptic feedback based on interaction type
fn suggest_haptic_for_interaction(event: &InteractionEvent) -> Option<HapticFeedback> {
    match &event.interaction_type {
        InteractionType::Gesture(GestureType::Tap) => Some(HapticFeedback::click()),
        InteractionType::Gesture(GestureType::DoubleTap) => Some(HapticFeedback {
            pattern: HapticPattern::Click,
            intensity: 0.8,
            duration_ms: 40,
            repeat_count: 1,
            repeat_delay_ms: 50,
        }),
        InteractionType::Gesture(GestureType::Hold { .. }) => Some(HapticFeedback::notification()),
        InteractionType::Voice { .. } => Some(HapticFeedback::processing()),
        InteractionType::Button {
            press_type: ButtonPressType::Long,
            ..
        } => Some(HapticFeedback::notification()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gesture_event_creation() {
        let event = InteractionEvent::gesture(GestureType::Tap, "ring", 0.95);
        assert!(matches!(
            event.interaction_type,
            InteractionType::Gesture(GestureType::Tap)
        ));
        assert_eq!(event.source, "ring");
        assert_eq!(event.confidence, 0.95);
    }

    #[test]
    fn test_voice_event_creation() {
        let event = InteractionEvent::voice("hello world", "microphone", 0.9);
        if let InteractionType::Voice { text, .. } = &event.interaction_type {
            assert_eq!(text, "hello world");
        } else {
            panic!("Expected Voice interaction type");
        }
    }

    #[test]
    fn test_interaction_context_with_hints() {
        let event = InteractionEvent::gesture(GestureType::DoubleTap, "ring", 0.9);
        let ctx = InteractionContext::new("test-agent").with_interaction(event);

        assert!(!ctx.tool_hints.is_empty());
        assert!(ctx.tool_hints.iter().any(|h| h.tool == "quick_action"));
    }

    #[test]
    fn test_haptic_feedback_presets() {
        let click = HapticFeedback::click();
        assert_eq!(click.pattern, HapticPattern::Click);
        assert_eq!(click.repeat_count, 0);

        let alert = HapticFeedback::alert();
        assert_eq!(alert.pattern, HapticPattern::Alert);
        assert_eq!(alert.repeat_count, 2);
    }
}
