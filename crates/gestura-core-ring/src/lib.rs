use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gesture {
    pub gesture_type: String,
    pub confidence: f32,
    pub acceleration: Option<[f32; 3]>,
    pub gyroscope: Option<[f32; 3]>,
}

pub use gestura_core_haptics::HapticPattern;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RingStatus {
    pub battery: u8,
    pub cpt_charging: bool,
    pub connection_state: String,
}

#[async_trait]
pub trait RingBackend: Send + Sync {
    async fn connect(&self) -> Result<(), String>;
    async fn subscribe_to_gestures(&self) -> tokio::sync::broadcast::Receiver<Gesture>;
    async fn send_haptic(&self, pattern: HapticPattern, intensity: f32, duration_ms: u32);
    async fn get_status(&self) -> RingStatus;
}

pub mod backends;

pub use backends::simulator::SimulatorBackend;
