use serde::{Deserialize, Serialize};

/// Generic status representation for any connected hardware device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub battery: u8,
    pub is_charging: bool,
    pub connection_state: String,
}
