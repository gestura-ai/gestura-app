use crate::{DeviceStatus, RingBackend};
use async_trait::async_trait;
use btleplug::api::{CharPropFlags, Characteristic, Peripheral as _};
use btleplug::platform::{Adapter, Peripheral};
use futures::stream::StreamExt;
use gestura_core_gestures::Gesture;
use gestura_core_haptics::HapticPattern;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

const SIMULATOR_SERVICE_UUID: Uuid = Uuid::from_u128(0x12345678_1234_5678_9abc_123456789abc);

/// Raw gestures emitted by the simulator matching the requested schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SimulatorRawGesture {
    Tap {
        intensity: f32,
    },
    DoubleTap,
    Hold {
        start_time: u64,
    },
    Slide {
        direction: SlideDirection,
        distance: u32,
    },
    Tilt {
        angle: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SlideDirection {
    Up,
    Down,
    Left,
    Right,
}

impl SimulatorRawGesture {
    /// Safely normalize the proprietary BLE enum into the generic `Gesture` struct,
    /// explicitly isolating intent parsing to the `gestura-core-intent` crate layer.
    ///
    /// `gesture_type` is always one of a bounded closed set that aligns exactly
    /// with the strings recognised by `gestura-core-intent::gesture_to_action`:
    /// `tap`, `double_tap`, `hold`, `tilt_up`, `tilt_down`, `tilt_left`,
    /// `tilt_right`.
    ///
    /// `Slide` directions are mapped to the corresponding `tilt_*` string so
    /// they route to meaningful actions (`scroll_up`, `scroll_down`, `previous`,
    /// `next`) instead of falling through to `unknown_gesture`.
    ///
    /// Physical `Tilt` direction is derived from the sign of the `angle` field:
    /// non-negative → `tilt_right`, negative → `tilt_left`.
    ///
    /// Numeric values (intensity, distance, angle) are carried in the
    /// `acceleration`/`gyroscope` sensor fields so downstream normalisation
    /// never needs to parse the type string.
    pub fn into_gesture(self) -> Gesture {
        match self {
            Self::Tap { intensity } => Gesture {
                gesture_type: "tap".to_string(),
                // Map tap intensity directly to gesture confidence so
                // downstream intent normalization can weight the signal.
                confidence: intensity.clamp(0.0, 1.0),
                acceleration: None,
                gyroscope: None,
            },
            Self::DoubleTap => Gesture {
                gesture_type: "double_tap".to_string(),
                confidence: 1.0,
                acceleration: None,
                gyroscope: None,
            },
            Self::Hold { .. } => Gesture {
                gesture_type: "hold".to_string(),
                confidence: 1.0,
                acceleration: None,
                gyroscope: None,
            },
            Self::Slide {
                direction,
                distance,
            } => {
                // Map slide directions to the tilt_* strings that
                // gesture_to_action recognises so slides route to meaningful
                // primary actions (scroll_up / scroll_down / previous / next)
                // rather than falling through to "unknown_gesture".
                let tilt_type = match direction {
                    SlideDirection::Up => "tilt_up",
                    SlideDirection::Down => "tilt_down",
                    SlideDirection::Left => "tilt_left",
                    SlideDirection::Right => "tilt_right",
                };
                Gesture {
                    gesture_type: tilt_type.to_string(),
                    confidence: 1.0,
                    // Carry slide distance as the x-axis acceleration component
                    // so downstream can read it without parsing the type string.
                    acceleration: Some([distance as f32, 0.0, 0.0]),
                    gyroscope: None,
                }
            }
            Self::Tilt { angle } => {
                // Derive direction from the sign of the angle so the emitted
                // string is in the recognised set (tilt_right / tilt_left).
                let tilt_type = if angle >= 0.0 {
                    "tilt_right"
                } else {
                    "tilt_left"
                };
                Gesture {
                    gesture_type: tilt_type.to_string(),
                    confidence: 1.0,
                    acceleration: None,
                    // Carry the raw angle in the x-axis gyroscope component so
                    // downstream can read it without parsing the type string.
                    gyroscope: Some([angle, 0.0, 0.0]),
                }
            }
        }
    }
}

pub struct SimulatorBackend {
    tx: broadcast::Sender<Gesture>,
    peripheral: Arc<Mutex<Option<Peripheral>>>,
    tx_char: Arc<Mutex<Option<Characteristic>>>,
    adapter: Arc<Mutex<Option<Adapter>>>,
}

impl Default for SimulatorBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulatorBackend {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);
        Self {
            tx,
            peripheral: Arc::new(Mutex::new(None)),
            tx_char: Arc::new(Mutex::new(None)),
            adapter: Arc::new(Mutex::new(None)),
        }
    }

    async fn find_simulator(&self) -> Result<Peripheral, String> {
        let (adapter, peripheral) = gestura_core_ble::scanner::find_device_by_service_uuid(
            SIMULATOR_SERVICE_UUID,
            10,
            std::time::Duration::from_millis(500),
        )
        .await?;

        *self.adapter.lock().await = Some(adapter);
        Ok(peripheral)
    }

    /// Spawns a background task monitoring notifications from the peripheral
    fn spawn_event_listener(&self, peripheral: Peripheral, characteristic: Characteristic) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Err(e) = peripheral.subscribe(&characteristic).await {
                tracing::error!("Failed to subscribe to gesture characteristics: {}", e);
                return;
            }

            let mut notification_stream = match peripheral.notifications().await {
                Ok(stream) => stream,
                Err(e) => {
                    tracing::error!("Failed to get notification stream: {}", e);
                    return;
                }
            };

            tracing::info!("Started listening for Simulator raw gestures");

            while let Some(data) = notification_stream.next().await {
                if let Ok(raw_str) = String::from_utf8(data.value)
                    && let Ok(raw_gest) = serde_json::from_str::<SimulatorRawGesture>(&raw_str)
                {
                    let gesture = raw_gest.into_gesture();
                    let _ = tx.send(gesture);
                }
            }
        });
    }

    /// Helper for testing environment simulating Tauri invoke fallback.
    pub async fn _untested_tauri_fallback_trigger(&self, raw: SimulatorRawGesture) {
        let _ = self.tx.send(raw.into_gesture());
    }
}

#[async_trait]
impl RingBackend for SimulatorBackend {
    async fn connect(&self) -> Result<(), String> {
        tracing::info!("SimulatorBackend initializing connection sequence");
        let peripheral = self.find_simulator().await?;

        peripheral
            .connect()
            .await
            .map_err(|e| format!("Connection failed: {}", e))?;
        peripheral
            .discover_services()
            .await
            .map_err(|e| format!("Service discovery failed: {}", e))?;

        let chars = peripheral.characteristics();
        // Just find a characteristic we can subscribe to/write to for gestures
        // In a real device we explicitly target a rx/tx uuid pair.
        let notify_char = chars
            .iter()
            .find(|c| c.properties.contains(CharPropFlags::NOTIFY));
        let write_char = chars
            .iter()
            .find(|c| {
                c.properties.contains(CharPropFlags::WRITE)
                    || c.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
            })
            .cloned();

        *self.peripheral.lock().await = Some(peripheral.clone());
        *self.tx_char.lock().await = write_char;

        if let Some(c) = notify_char {
            self.spawn_event_listener(peripheral, c.clone());
        } else {
            return Err("Simulator connected but no NOTIFY characteristic found; gestures will never be emitted.".to_string());
        }

        tracing::info!("SimulatorBackend successfully fully bound BLE channel");
        Ok(())
    }

    async fn subscribe_to_gestures(&self) -> tokio::sync::broadcast::Receiver<Gesture> {
        self.tx.subscribe()
    }

    async fn send_haptic(&self, pattern: HapticPattern, intensity: f32, duration_ms: u32) {
        tracing::debug!(
            "SimulatorBackend sending haptic: {:?} (int: {}, dur: {}ms)",
            pattern,
            intensity,
            duration_ms
        );

        let p_lock = self.peripheral.lock().await;
        let c_lock = self.tx_char.lock().await;

        if let (Some(peripheral), Some(char)) = (&*p_lock, &*c_lock) {
            let command_json = serde_json::json!({
                "command": "trigger_haptic",
                "pattern": pattern,
                "intensity": intensity,
                "duration_ms": duration_ms
            })
            .to_string();

            // Choose the write type the characteristic actually supports.
            // Blindly using WithoutResponse on a WRITE-only characteristic
            // produces a silent failure; inspect the flags first.
            let write_type = if char
                .properties
                .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
            {
                btleplug::api::WriteType::WithoutResponse
            } else {
                btleplug::api::WriteType::WithResponse
            };

            if let Err(e) = peripheral
                .write(char, command_json.as_bytes(), write_type)
                .await
            {
                tracing::warn!(
                    pattern = ?pattern,
                    "Failed to send haptic feedback via BLE write: {}",
                    e
                );
            }
        }
    }

    async fn get_status(&self) -> DeviceStatus {
        let is_connected = self.peripheral.lock().await.is_some();
        DeviceStatus {
            battery: 100,
            is_charging: false,
            connection_state: if is_connected {
                "simulator_ble_connected".to_string()
            } else {
                "simulator_disconnected".to_string()
            },
        }
    }
}
