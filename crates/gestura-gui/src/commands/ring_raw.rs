//! Raw GATT passthrough commands for the ring SDK's Tauri transport.
//!
//! Contract (`sdk/typescript/src/transport/tauri.ts`):
//! ```text
//! invoke("ring_write",       { deviceId, uuid, bytes: number[] }) -> void
//! invoke("ring_read",        { deviceId, uuid })                  -> number[]
//! invoke("ring_subscribe",   { deviceId, uuid })                  -> void
//! invoke("ring_unsubscribe", { deviceId, uuid })                  -> void
//! invoke("ring_active_device")                                    -> string
//! event  "ring-notify" { deviceId, uuid, bytes: number[] }
//! ```
//! Bytes cross the IPC boundary untouched: the SDK's WASM core is the only
//! codec, so nothing here knows what a payload means. The `ring-notify`
//! forwarder is started in `main.rs` from `RingManager::raw_notifications`.

use crate::AppState;
use tauri::State;
use uuid::Uuid;

fn parse_uuid(uuid: &str) -> Result<Uuid, String> {
    Uuid::parse_str(uuid).map_err(|e| format!("invalid characteristic uuid {uuid:?}: {e}"))
}

/// Writes raw bytes to a characteristic (write type chosen from its properties).
#[tauri::command(rename_all = "camelCase")]
pub async fn ring_write(
    device_id: String,
    uuid: String,
    bytes: Vec<u8>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let uuid = parse_uuid(&uuid)?;
    state
        .ring_manager
        .raw_write(&device_id, uuid, bytes)
        .await
        .map_err(|e| e.to_string())
}

/// Reads a characteristic's current value as raw bytes.
#[tauri::command(rename_all = "camelCase")]
pub async fn ring_read(
    device_id: String,
    uuid: String,
    state: State<'_, AppState>,
) -> Result<Vec<u8>, String> {
    let uuid = parse_uuid(&uuid)?;
    state
        .ring_manager
        .raw_read(&device_id, uuid)
        .await
        .map_err(|e| e.to_string())
}

/// Subscribes to a characteristic; notifications arrive as `ring-notify` events.
#[tauri::command(rename_all = "camelCase")]
pub async fn ring_subscribe(
    device_id: String,
    uuid: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let uuid = parse_uuid(&uuid)?;
    state
        .ring_manager
        .raw_subscribe(&device_id, uuid)
        .await
        .map_err(|e| e.to_string())
}

/// Unsubscribes from a characteristic.
#[tauri::command(rename_all = "camelCase")]
pub async fn ring_unsubscribe(
    device_id: String,
    uuid: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let uuid = parse_uuid(&uuid)?;
    state
        .ring_manager
        .raw_unsubscribe(&device_id, uuid)
        .await
        .map_err(|e| e.to_string())
}

/// The connected external ring/simulator the SDK should bind to. Errors when
/// nothing is connected so the SDK falls back to its offline transport.
#[tauri::command]
pub async fn ring_active_device(state: State<'_, AppState>) -> Result<String, String> {
    match state.ring_manager.active_device().await {
        Ok(Some(device_id)) => Ok(device_id),
        Ok(None) => Err("no external ring or simulator is connected — pair one first".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_uuid;

    #[test]
    fn parses_ratified_uuid_and_rejects_garbage() {
        let u = parse_uuid("e3b742d4-51c9-4f0e-9d26-7a48c1f0b9bd").unwrap();
        assert_eq!(u.as_u128(), 0xE3B742D4_51C9_4F0E_9D26_7A48C1F0B9BD);
        assert!(parse_uuid("not-a-uuid").is_err());
    }
}
