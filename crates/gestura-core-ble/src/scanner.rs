use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use uuid::Uuid;
use std::time::Duration;

/// Initializes the BLE manager, begins a scan targeting the provided `service_uuid`, 
/// and reliably polls until the peripheral is matched or bounds exhaust.
pub async fn find_device_by_service_uuid(
    service_uuid: Uuid, 
    polling_attempts: u8,
    polling_interval: Duration,
) -> Result<(Adapter, Peripheral), String> {
    let manager = Manager::new().await.map_err(|e| format!("Failed to initialize BLE Manager: {}", e))?;
    let adapters = manager.adapters().await.map_err(|e| format!("Failed to get BLE adapters: {}", e))?;
    let adapter = adapters.into_iter().next().ok_or("No Bluetooth adapters found")?;

    adapter
        .start_scan(ScanFilter { services: vec![service_uuid] })
        .await
        .map_err(|e| format!("Failed to start scan: {}", e))?;

    tracing::info!("Scanning for device bounding service UUID {}", service_uuid);
    
    for _ in 0..polling_attempts {
        tokio::time::sleep(polling_interval).await;
        for p in adapter.peripherals().await.unwrap_or_default() {
            if p.properties().await.unwrap_or_default().unwrap_or_default().services.contains(&service_uuid) {
                return Ok((adapter, p));
            }
        }
    }

    Err(format!("Device mapped to BLE service {} not found across bounds", service_uuid))
}
