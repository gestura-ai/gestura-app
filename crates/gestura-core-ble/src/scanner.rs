use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use std::time::Duration;
use uuid::Uuid;

/// Initializes the BLE manager, begins a scan targeting the provided `service_uuid`,
/// and reliably polls until the peripheral is matched or bounds exhaust.
pub async fn find_device_by_service_uuid(
    service_uuid: Uuid,
    polling_attempts: u8,
    polling_interval: Duration,
) -> Result<(Adapter, Peripheral), String> {
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
            services: vec![service_uuid],
        })
        .await
        .map_err(|e| format!("Failed to start scan: {}", e))?;

    tracing::info!("Scanning for device bounding service UUID {}", service_uuid);

    for _ in 0..polling_attempts {
        tokio::time::sleep(polling_interval).await;

        let peripherals = match adapter.peripherals().await {
            Ok(ps) => ps,
            Err(e) => {
                tracing::warn!(
                    "Failed to list peripherals during BLE scan (will retry): {}",
                    e
                );
                continue;
            }
        };

        for p in peripherals {
            let properties = match p.properties().await {
                Ok(Some(props)) => props,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(
                        "Failed to read peripheral properties (skipping peripheral): {}",
                        e
                    );
                    continue;
                }
            };
            if properties.services.contains(&service_uuid) {
                let _ = adapter.stop_scan().await;
                return Ok((adapter, p));
            }
        }
    }

    let _ = adapter.stop_scan().await;

    Err(format!(
        "Device mapped to BLE service {} not found across bounds",
        service_uuid
    ))
}
