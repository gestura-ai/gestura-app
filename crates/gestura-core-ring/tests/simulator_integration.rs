use gestura_core_ring::{RingBackend, SimulatorBackend};
use gestura_core_haptics::HapticPattern;
use tokio::time::Duration;

#[tokio::test]
async fn test_simulator_backend_initializes_and_haptic_drops_gracefully() {
    // This integration test verifies that the simulator backend orchestrates correctly
    // Without physically running the Bluetooth daemon, it verifies that the struct and channels 
    // are robust and ready.
    
    let backend = SimulatorBackend::new();
    
    // We expect connection to error out in CI since we have no Bluetooth adapters
    // However, we verify the interface operates transparently
    let _ = backend.connect().await;
    
    let mut rx = backend.subscribe_to_gestures().await;
    
    // Test that invoking a haptic function does not panic when disconnected
    backend.send_haptic(HapticPattern::Confirm, 1.0, 300).await;
    
    let status = backend.get_status().await;
    assert_eq!(status.connection_state, "simulator_disconnected");
    
    // Verify receiver dropping behaves gracefully
    let timeout = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
    assert!(timeout.is_err()); // Ensure it timed out and didn't crash
}
