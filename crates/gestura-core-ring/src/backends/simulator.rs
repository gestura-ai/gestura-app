use crate::{Gesture, RingBackend, RingStatus};
use async_trait::async_trait;
use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, CharPropFlags,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::stream::StreamExt;
use gestura_core_haptics::HapticPattern;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

const SIMULATOR_SERVICE_UUID: Uuid = Uuid::from_u128(0x12345678_1234_5678_9abc_123456789abc);

/// Raw gestures emitted by the simulator matching the requested schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SimulatorRawGesture {
    Tap { intensity: f32 },
    DoubleTap,
    Hold { start_time: u64 },
    Slide { direction: SlideDirection, distance: u32 },
    Tilt { angle: f32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SlideDirection {
    Up,
    Down,
    Left,
    Right,
}

impl SimulatorRawGesture {
    /// Safely normalize the proprietary BLE enum into the generic `Gesture` struct 
    /// explicitly isolating intent parsing to the `gestura-core-intent` crate layer.
    pub fn into_gesture(self) -> Gesture {
        let (gesture_type, confidence) = match self {
            Self::Tap { intensity } => (format!("tap_{}", intensity), 1.0),
            Self::DoubleTap => ("double_tap".to_string(), 1.0),
            Self::Hold { .. } => ("hold".to_string(), 1.0),
            Self::Slide { direction, distance } => {
                let dir_str = match direction {
                    SlideDirection::Up => "up",
                    SlideDirection::Down => "down",
                    SlideDirection::Left => "left",
                    SlideDirection::Right => "right",
                };
                (format!("slide_{}_{}", dir_str, distance), 1.0)
            }
            Self::Tilt { angle } => (format!("tilt_{}", angle), 1.0),
        };

        Gesture {
            gesture_type,
            confidence,
            acceleration: None,
            gyroscope: None,
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
        let manager = Manager::new()
            .await
            .map_err(|e| format!("Failed to initialize BLE Manager: {}", e))?;
        
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| format!("Failed to get BLE adapters: {}", e))?;
            
        let adapter = adapters
            .into_iter()
            .next()
            .ok_or("No Bluetooth adapters found")?;

        adapter
            .start_scan(ScanFilter {
                services: vec![SIMULATOR_SERVICE_UUID],
            })
            .await
            .map_err(|e| format!("Failed to start scan: {}", e))?;

        tracing::info!("Scanning for Simulator bounds on service UUID...");
        
        // Polling loop to find the peripheral
        for _ in 0..10 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            for p in adapter.peripherals().await.unwrap_or_default() {
                if p.properties()
                    .await
                    .unwrap_or_default()
                    .unwrap_or_default()
                    .services
                    .contains(&SIMULATOR_SERVICE_UUID)
                {
                    *self.adapter.lock().await = Some(adapter);
                    return Ok(p);
                }
            }
        }

        Err("Simulator BLE peripheral not found".to_string())
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
                if let Ok(raw_str) = String::from_utf8(data.value) {
                    if let Ok(raw_gest) = serde_json::from_str::<SimulatorRawGesture>(&raw_str) {
                        let gesture = raw_gest.into_gesture();
                        let _ = tx.send(gesture);
                    }
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
        
        peripheral.connect().await.map_err(|e| format!("Connection failed: {}", e))?;
        peripheral.discover_services().await.map_err(|e| format!("Service discovery failed: {}", e))?;
        
        let chars = peripheral.characteristics();
        // Just find a characteristic we can subscribe to/write to for gestures
        // In a real device we explicitly target a rx/tx uuid pair. 
        let notify_char = chars.iter().find(|c| c.properties.contains(CharPropFlags::NOTIFY));
        let write_char = chars.iter().find(|c| c.properties.contains(CharPropFlags::WRITE) || c.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)).cloned();

        *self.peripheral.lock().await = Some(peripheral.clone());
        *self.tx_char.lock().await = write_char;

        if let Some(c) = notify_char {
            self.spawn_event_listener(peripheral, c.clone());
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
            pattern, intensity, duration_ms
        );
        
        let p_lock = self.peripheral.lock().await;
        let c_lock = self.tx_char.lock().await;
        
        if let (Some(peripheral), Some(char)) = (&*p_lock, &*c_lock) {
            let command_json = serde_json::json!({
                "command": "trigger_haptic",
                "pattern": pattern,
                "intensity": intensity,
                "duration_ms": duration_ms
            }).to_string();
            
            // Using WriteWithoutResponse primarily since usually it's faster for haptics.
            let _ = peripheral.write(char, command_json.as_bytes(), btleplug::api::WriteType::WithoutResponse).await;
        }
    }
    
    async fn get_status(&self) -> RingStatus {
        let is_connected = self.peripheral.lock().await.is_some();
        RingStatus {
            battery: 100,
            cpt_charging: false,
            connection_state: if is_connected { "simulator_ble_connected".to_string() } else { "simulator_disconnected".to_string() },
        }
    }
}
