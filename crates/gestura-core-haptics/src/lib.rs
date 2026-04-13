use serde::{Deserialize, Serialize};

/// High-level semantic haptic patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HapticPattern {
    Confirm,
    Error,
    Tick,
    DoubleTick,
    Waveform(Vec<u8>),
}
