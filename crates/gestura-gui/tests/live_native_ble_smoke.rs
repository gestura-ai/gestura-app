use std::time::Duration;

use btleplug::api::{Central, Manager as _, Peripheral as _};
use btleplug::platform::Manager;
use gestura_gui::ble::{BleEvent, RingManager};
use gestura_gui::ble_central::ExternalBleRingManager;
use gestura_gui::haptics::HapticRequest;
use tokio::time::{Instant, timeout};

/// Validates the real app-side BLE central flow against a live native simulator.
///
/// Run explicitly with a simulator already advertising, for example:
/// `cargo test -p gestura-gui --test live_native_ble_smoke -- --ignored --nocapture`
#[tokio::test]
#[ignore = "requires a live haptic-harmony-simulator advertising over native BLE"]
async fn live_native_ble_smoke() {
    log_raw_scan_snapshot().await;

    let manager = ExternalBleRingManager::new()
        .await
        .expect("BLE adapter should be available for live smoke test");

    let simulators = manager
        .scan_for_simulators()
        .await
        .expect("live simulator scan should succeed");
    assert!(
        !simulators.is_empty(),
        "expected at least one live simulator advertising over BLE"
    );

    let device_id = simulators[0].clone();
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
    assert!(
        status.is_simulator,
        "discovered device should be a simulator"
    );
    assert_eq!(status.battery_level, Some(85));
    assert_eq!(status.firmware_version.as_deref(), Some("1.0.0-sim"));

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

    let deadline = Instant::now() + Duration::from_secs(15);
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
    manager
        .reset_simulator(&device_id)
        .await
        .expect("simulator reset should disconnect cleanly");

    assert!(
        saw_gesture,
        "expected a live gesture notification within 15 seconds; trigger one in the simulator while this test runs"
    );
}

async fn log_raw_scan_snapshot() {
    let manager = Manager::new()
        .await
        .expect("raw btleplug manager should initialize");
    let adapters = manager.adapters().await.expect("adapters should be listed");
    let Some(adapter) = adapters.into_iter().next() else {
        println!("RAW_SCAN no adapters available");
        return;
    };

    adapter
        .start_scan(Default::default())
        .await
        .expect("raw scan should start");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let peripherals = adapter
        .peripherals()
        .await
        .expect("peripherals should list");
    println!("RAW_SCAN count={}", peripherals.len());
    for peripheral in peripherals {
        let id = peripheral.id().to_string();
        match peripheral.properties().await {
            Ok(Some(properties)) => println!(
                "RAW_DEVICE id={} local_name={:?} services={:?}",
                id, properties.local_name, properties.services
            ),
            Ok(None) => println!("RAW_DEVICE id={} properties=None", id),
            Err(error) => println!("RAW_DEVICE id={} properties_error={error}", id),
        }
    }
}
