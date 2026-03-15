use std::{env, time::Duration};

use btleplug::api::{Central, Manager as _, Peripheral as _};
use btleplug::platform::Manager;
use gestura_gui::ble::{BleEvent, RingManager};
use gestura_gui::ble_central::ExternalBleRingManager;
use gestura_gui::haptics::HapticRequest;
use tokio::time::{Instant, timeout};

const DEFAULT_SCAN_SECS: u64 = 3;
const DEFAULT_GESTURE_TIMEOUT_SECS: u64 = 15;

#[derive(Debug, Clone)]
struct LiveBleSmokeConfig {
    target_device_id: Option<String>,
    target_name_substring: Option<String>,
    scan_secs: u64,
    gesture_timeout_secs: u64,
    require_gesture: bool,
    expected_battery: Option<u8>,
    expected_firmware: Option<String>,
}

#[derive(Debug, Clone)]
struct RawPeripheralSummary {
    id: String,
    local_name: Option<String>,
    services: Vec<String>,
}

impl LiveBleSmokeConfig {
    fn from_env() -> Self {
        Self {
            target_device_id: env::var("GESTURA_LIVE_BLE_DEVICE_ID").ok(),
            target_name_substring: env::var("GESTURA_LIVE_BLE_NAME_SUBSTRING").ok(),
            scan_secs: parse_u64_env("GESTURA_LIVE_BLE_SCAN_SECS", DEFAULT_SCAN_SECS),
            gesture_timeout_secs: parse_u64_env(
                "GESTURA_LIVE_BLE_GESTURE_TIMEOUT_SECS",
                DEFAULT_GESTURE_TIMEOUT_SECS,
            ),
            require_gesture: parse_bool_env("GESTURA_LIVE_BLE_REQUIRE_GESTURE", true),
            expected_battery: env::var("GESTURA_LIVE_BLE_EXPECT_BATTERY")
                .ok()
                .and_then(|value| value.parse().ok()),
            expected_firmware: env::var("GESTURA_LIVE_BLE_EXPECT_FIRMWARE").ok(),
        }
    }
}

/// Validates the real app-side BLE central flow against a live native simulator.
///
/// Run explicitly with a simulator already advertising, for example:
/// `cargo test -p gestura-gui --test live_native_ble_smoke -- --ignored --nocapture`
///
/// Recommended env vars for cross-device validation:
/// - `GESTURA_LIVE_BLE_DEVICE_ID=<exact peripheral id>`
/// - `GESTURA_LIVE_BLE_NAME_SUBSTRING="Ring Simulator"`
/// - `GESTURA_LIVE_BLE_REQUIRE_GESTURE=false` when gesture injection is manual/unavailable
/// - `GESTURA_LIVE_BLE_EXPECT_BATTERY=85`
/// - `GESTURA_LIVE_BLE_EXPECT_FIRMWARE=1.0.0-sim`
#[tokio::test]
#[ignore = "requires a live haptic-harmony-simulator advertising over native BLE"]
async fn live_native_ble_smoke() {
    let config = LiveBleSmokeConfig::from_env();
    println!("LIVE_BLE_CONFIG {config:?}");
    let raw_devices = collect_raw_scan_snapshot(config.scan_secs).await;

    let manager = ExternalBleRingManager::new()
        .await
        .expect("BLE adapter should be available for live smoke test");

    let rings = manager
        .scan_for_rings()
        .await
        .expect("live ring scan should succeed");
    println!("LIVE_RING_SCAN ids={rings:?}");

    let simulators = manager
        .scan_for_simulators()
        .await
        .expect("live simulator scan should succeed");
    println!("LIVE_SIMULATOR_SCAN ids={simulators:?}");

    let device_id = resolve_target_device(&config, &rings, &simulators, &raw_devices);
    println!("LIVE_BLE_TARGET {device_id}");

    manager
        .pair_ring(&device_id)
        .await
        .expect("pairing with live simulator should succeed");

    let status = manager
        .get_ring_status(&device_id)
        .await
        .expect("status read should succeed")
        .expect("paired device should report status");
    assert!(status.is_connected, "paired simulator should be connected");

    if simulators.contains(&device_id) {
        assert!(
            status.is_simulator,
            "scan_for_simulators classified the target as a simulator, but status did not"
        );
    } else {
        println!(
            "LIVE_BLE_NOTE target was selected explicitly for cross-device validation and was not returned by scan_for_simulators"
        );
    }

    if let Some(expected_battery) = config.expected_battery {
        assert_eq!(status.battery_level, Some(expected_battery));
    } else if status.is_simulator {
        assert!(
            status.battery_level.is_some(),
            "expected simulator status to expose a battery level"
        );
    }

    if let Some(expected_firmware) = config.expected_firmware.as_deref() {
        assert_eq!(status.firmware_version.as_deref(), Some(expected_firmware));
    } else if status.is_simulator {
        assert!(
            status.firmware_version.is_some(),
            "expected simulator status to expose a firmware version"
        );
    }

    manager
        .send_haptic(&device_id, HapticRequest::click())
        .await
        .expect("haptic command should write successfully");

    let logs = manager
        .get_connection_logs(&device_id)
        .await
        .expect("connection logs should be readable");
    assert!(
        logs.iter()
            .any(|entry| entry.contains("Sent haptic command")),
        "expected connection logs to include the haptic write"
    );

    let (event_tx, mut event_rx) = tokio::sync::broadcast::channel(32);
    manager
        .start_gesture_monitoring(&device_id, event_tx)
        .await
        .expect("gesture monitoring subscription should succeed");

    let deadline = Instant::now() + Duration::from_secs(config.gesture_timeout_secs);
    let mut saw_gesture = false;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match timeout(remaining, event_rx.recv()).await {
            Ok(Ok(BleEvent::GestureDetected(_))) => {
                saw_gesture = true;
                break;
            }
            Ok(Ok(_)) => {}
            Ok(Err(error)) => panic!("gesture monitoring channel failed: {error}"),
            Err(_) => break,
        }
    }

    manager
        .stop_gesture_monitoring(&device_id)
        .await
        .expect("gesture monitoring should stop cleanly");

    if status.is_simulator {
        manager
            .reset_simulator(&device_id)
            .await
            .expect("simulator reset should disconnect cleanly");
    } else {
        println!(
            "LIVE_BLE_NOTE skipping simulator reset because target was not classified as a simulator"
        );
    }

    if config.require_gesture {
        assert!(
            saw_gesture,
            "expected a live gesture notification within the configured timeout; for cross-device runs either trigger a gesture manually or set GESTURA_LIVE_BLE_REQUIRE_GESTURE=false"
        );
    }
}

fn resolve_target_device(
    config: &LiveBleSmokeConfig,
    rings: &[String],
    simulators: &[String],
    raw_devices: &[RawPeripheralSummary],
) -> String {
    if let Some(device_id) = config.target_device_id.as_ref() {
        return device_id.clone();
    }

    if let Some(substring) = config.target_name_substring.as_ref() {
        let needle = substring.to_ascii_lowercase();
        if let Some(device) = raw_devices.iter().find(|device| {
            device.id.to_ascii_lowercase().contains(&needle)
                || device
                    .local_name
                    .as_deref()
                    .map(|name| name.to_ascii_lowercase().contains(&needle))
                    .unwrap_or(false)
        }) {
            return device.id.clone();
        }

        panic!(
            "GESTURA_LIVE_BLE_NAME_SUBSTRING={substring:?} did not match any raw scan result; raw devices={raw_devices:?}"
        );
    }

    if let Some(device_id) = simulators.first() {
        return device_id.clone();
    }

    panic!(
        "expected at least one live simulator advertising over BLE; scan_for_rings={rings:?}; raw_devices={raw_devices:?}. On macOS, same-host CoreBluetooth central+peripheral validation is not authoritative—prefer a second device or explicit GESTURA_LIVE_BLE_DEVICE_ID / GESTURA_LIVE_BLE_NAME_SUBSTRING for cross-device runs."
    );
}

async fn collect_raw_scan_snapshot(scan_secs: u64) -> Vec<RawPeripheralSummary> {
    let manager = Manager::new()
        .await
        .expect("raw btleplug manager should initialize");
    let adapters = manager.adapters().await.expect("adapters should be listed");
    let Some(adapter) = adapters.into_iter().next() else {
        println!("RAW_SCAN no adapters available");
        return Vec::new();
    };

    adapter
        .start_scan(Default::default())
        .await
        .expect("raw scan should start");
    tokio::time::sleep(Duration::from_secs(scan_secs)).await;

    let peripherals = adapter
        .peripherals()
        .await
        .expect("peripherals should list");
    println!("RAW_SCAN count={}", peripherals.len());

    let mut summaries = Vec::new();
    for peripheral in peripherals {
        let id = peripheral.id().to_string();
        match peripheral.properties().await {
            Ok(Some(properties)) => {
                let summary = RawPeripheralSummary {
                    id,
                    local_name: properties.local_name,
                    services: properties
                        .services
                        .into_iter()
                        .map(|uuid| uuid.to_string())
                        .collect(),
                };
                println!(
                    "RAW_DEVICE id={} local_name={:?} services={:?}",
                    summary.id, summary.local_name, summary.services
                );
                summaries.push(summary);
            }
            Ok(None) => println!("RAW_DEVICE id={} properties=None", id),
            Err(error) => println!("RAW_DEVICE id={} properties_error={error}", id),
        }
    }

    summaries
}

fn parse_bool_env(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn parse_u64_env(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
