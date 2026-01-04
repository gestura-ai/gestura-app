/// Tauri commands for Haptic Harmony Ring simulator functionality

use crate::AppState;
use crate::simulator::{SimulatorInfo, TestResults};
use crate::ble::{TestHapticPattern, SimulatorStatus};
use std::collections::HashMap;
use tauri::State;

/// Get all connected simulators
#[tauri::command]
pub async fn get_simulators(
    state: State<'_, AppState>,
) -> Result<HashMap<String, SimulatorInfo>, String> {
    let _app_state = state.inner();
    
    // Get simulator manager from app state
    // Note: In a real implementation, this would be stored in AppState
    // For now, we'll return a mock response
    let mut simulators = HashMap::new();
    
    simulators.insert(
        "mock-simulator-001".to_string(),
        SimulatorInfo {
            device_id: "mock-simulator-001".to_string(),
            device_name: "Haptic Harmony Ring Simulator".to_string(),
            status: SimulatorStatus::Healthy,
            last_health_check: chrono::Utc::now(),
            connection_time: chrono::Utc::now(),
            metrics: crate::simulator::SimulatorMetrics::default(),
        }
    );
    
    Ok(simulators)
}

/// Scan for available simulators
#[tauri::command]
pub async fn scan_for_simulators(
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let app_state = state.inner();

    // Use the ring manager to scan for simulators
    if let Some(ring_manager) = &app_state.ring_manager {
        ring_manager.scan_for_simulators().await
            .map_err(|e| e.to_string())
    } else {
        Ok(vec!["mock-simulator-001".to_string()])
    }
}

/// Reset a specific simulator
#[tauri::command]
pub async fn reset_simulator(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let app_state = state.inner();
    
    if let Some(ring_manager) = &app_state.ring_manager {
        ring_manager.reset_simulator(&device_id).await
            .map_err(|e| e.to_string())
    } else {
        tracing::info!("Mock: Reset simulator {}", device_id);
        Ok(())
    }
}

/// Send test haptic pattern to simulator
#[tauri::command]
pub async fn send_test_haptic(
    device_id: String,
    pattern_type: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let app_state = state.inner();
    
    let pattern = match pattern_type.as_str() {
        "connectivity" => TestHapticPattern::ConnectivityTest,
        "latency" => TestHapticPattern::LatencyTest,
        "intensity" => TestHapticPattern::IntensityTest { min: 0.1, max: 1.0, steps: 10 },
        "duration" => TestHapticPattern::DurationTest { durations: vec![100, 200, 500, 1000] },
        "complex" => TestHapticPattern::ComplexPattern { 
            pattern: vec![(0.5, 100), (1.0, 200), (0.3, 150)] 
        },
        _ => return Err("Invalid pattern type".to_string()),
    };
    
    if let Some(ring_manager) = &app_state.ring_manager {
        ring_manager.send_test_haptic(&device_id, pattern).await
            .map_err(|e| e.to_string())
    } else {
        tracing::info!("Mock: Send test haptic to {} with pattern {:?}", device_id, pattern);
        Ok(())
    }
}

/// Get simulator health status
#[tauri::command]
pub async fn get_simulator_health(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<SimulatorStatus, String> {
    let app_state = state.inner();
    
    if let Some(ring_manager) = &app_state.ring_manager {
        ring_manager.get_simulator_health(&device_id).await
            .map_err(|e| e.to_string())
    } else {
        Ok(SimulatorStatus::Healthy)
    }
}

/// Get connection logs for a simulator
#[tauri::command]
pub async fn get_simulator_logs(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let app_state = state.inner();
    
    if let Some(ring_manager) = &app_state.ring_manager {
        ring_manager.get_connection_logs(&device_id).await
            .map_err(|e| e.to_string())
    } else {
        Ok(vec![
            format!("Mock connection log for {}", device_id),
            "Simulator initialized".to_string(),
            "BLE services started".to_string(),
            "Ready for connections".to_string(),
        ])
    }
}

/// Run comprehensive simulator test
#[tauri::command]
pub async fn run_simulator_test(
    _device_id: String,
    state: State<'_, AppState>,
) -> Result<TestResults, String> {
    let _app_state = state.inner();
    
    // For now, return mock test results
    // In a real implementation, this would use SimulatorTester
    Ok(TestResults {
        connectivity: true,
        latency_ms: 15.5,
        haptic_tests: vec![
            crate::simulator::HapticTestResult {
                pattern: "IntensityTest".to_string(),
                success: true,
                error: None,
            },
            crate::simulator::HapticTestResult {
                pattern: "DurationTest".to_string(),
                success: true,
                error: None,
            },
        ],
    })
}

/// Check if developer mode is enabled
#[tauri::command]
pub async fn is_developer_mode_enabled(
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let app_state = state.inner();
    Ok(app_state.config.developer.developer_mode)
}

/// Toggle developer mode
#[tauri::command]
pub async fn toggle_developer_mode(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _app_state = state.inner();

    // Update configuration
    // Note: In a real implementation, this would persist the config
    tracing::info!("Developer mode {}", if enabled { "enabled" } else { "disabled" });
    
    Ok(())
}

/// Enable/disable simulator support
#[tauri::command]
pub async fn toggle_simulator_support(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _app_state = state.inner();

    tracing::info!("Simulator support {}", if enabled { "enabled" } else { "disabled" });
    
    Ok(())
}

/// Get simulator configuration
#[tauri::command]
pub async fn get_simulator_config(
    state: State<'_, AppState>,
) -> Result<crate::config::SimulatorSettings, String> {
    let app_state = state.inner();
    Ok(app_state.config.developer.simulator.clone())
}

/// Update simulator configuration
#[tauri::command]
pub async fn update_simulator_config(
    config: crate::config::SimulatorSettings,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _app_state = state.inner();

    tracing::info!("Updating simulator config: {:?}", config);
    
    // Note: In a real implementation, this would update and persist the config
    Ok(())
}

/// Auto-discover simulators on localhost
#[tauri::command]
pub async fn auto_discover_simulators(
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let _app_state = state.inner();

    // Mock auto-discovery
    let discovered = vec![
        "localhost:8080".to_string(),
        "localhost:8081".to_string(),
    ];
    
    tracing::info!("Auto-discovered simulators: {:?}", discovered);
    
    Ok(discovered)
}

/// Get simulator performance metrics
#[tauri::command]
pub async fn get_simulator_metrics(
    _device_id: String,
    state: State<'_, AppState>,
) -> Result<crate::simulator::SimulatorMetrics, String> {
    let _app_state = state.inner();
    
    // Return mock metrics
    Ok(crate::simulator::SimulatorMetrics {
        latency_ms: Some(12.3),
        packet_loss_rate: 0.01,
        uptime_seconds: 3600,
        haptic_commands_sent: 150,
        gestures_received: 75,
    })
}

/// Start simulator health monitoring
#[tauri::command]
pub async fn start_health_monitoring(
    device_id: String,
    interval_seconds: u32,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _app_state = state.inner();

    tracing::info!("Starting health monitoring for {} with interval {}s", device_id, interval_seconds);
    
    Ok(())
}

/// Stop simulator health monitoring
#[tauri::command]
pub async fn stop_health_monitoring(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _app_state = state.inner();

    tracing::info!("Stopping health monitoring for {}", device_id);

    Ok(())
}

/// Set window size for the application
#[tauri::command]
pub async fn set_window_size(
    width: f64,
    height: f64,
    window: tauri::Window,
) -> Result<(), String> {
    use tauri::Manager;

    tracing::info!("Setting window size to {}x{}", width, height);

    let size = tauri::LogicalSize::new(width, height);

    window.set_size(size)
        .map_err(|e| {
            tracing::error!("Failed to set window size: {}", e);
            format!("Failed to set window size: {}", e)
        })?;

    tracing::info!("Window size set successfully to {}x{}", width, height);
    Ok(())
}
