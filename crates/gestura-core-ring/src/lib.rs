use async_trait::async_trait;

pub use gestura_core_gestures::Gesture;

pub use gestura_core_haptics::HapticPattern;

pub use gestura_core_devices::DeviceStatus;

#[async_trait]
pub trait RingBackend: Send + Sync {
    async fn connect(&self) -> Result<(), String>;
    async fn subscribe_to_gestures(&self) -> tokio::sync::broadcast::Receiver<Gesture>;
    async fn send_haptic(&self, pattern: HapticPattern, intensity: f32, duration_ms: u32);
    async fn get_status(&self) -> DeviceStatus;
    /// Releases the connection. Backends that suppressed the device's HID
    /// projection at connect time restore it here (see
    /// `protocol::RingConfig`); default is a no-op so existing backends
    /// remain source-compatible.
    async fn disconnect(&self) {}
}

pub mod backends;
pub mod protocol;

pub use backends::simulator::SimulatorBackend;
