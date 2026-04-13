use serde::{Deserialize, Serialize};

/// Generic gesture representation agnostic to the underlying hardware source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gesture {
    pub gesture_type: String,
    pub confidence: f32,
    pub acceleration: Option<[f32; 3]>,
    pub gyroscope: Option<[f32; 3]>,
}
