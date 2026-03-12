use crate::ble::{SimulatorStatus, TestHapticPattern};
use crate::simulator::{SimulatorInfo, SimulatorTester, TestResults};
/// Tauri commands for Haptic Harmony Ring simulator functionality
use crate::{AppConfigSecurityExt, AppState};
use std::collections::HashMap;
use tauri::State;

async fn persist_developer_settings(
    settings: crate::config::DeveloperSettings,
) -> Result<(), String> {
    let mut config = crate::AppConfig::load_async().await;
    config.developer = settings;
    config.save_async().await.map_err(|e| e.to_string())
}

/// Get all connected simulators
#[tauri::command]
pub async fn get_simulators(
    state: State<'_, AppState>,
) -> Result<HashMap<String, SimulatorInfo>, String> {
    Ok(state.simulator_manager.get_simulators().await)
}

/// Scan for available simulators
#[tauri::command]
pub async fn scan_for_simulators(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    state
        .simulator_manager
        .scan_for_simulators()
        .await
        .map_err(|e| e.to_string())
}

/// Reset a specific simulator
#[tauri::command]
pub async fn reset_simulator(device_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state
        .ring_manager
        .reset_simulator(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Send test haptic pattern to simulator
#[tauri::command]
pub async fn send_test_haptic(
    device_id: String,
    pattern_type: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let pattern = match pattern_type.as_str() {
        "connectivity" => TestHapticPattern::ConnectivityTest,
        "latency" => TestHapticPattern::LatencyTest,
        "intensity" => TestHapticPattern::IntensityTest {
            min: 0.1,
            max: 1.0,
            steps: 10,
        },
        "duration" => TestHapticPattern::DurationTest {
            durations: vec![100, 200, 500, 1000],
        },
        "complex" => TestHapticPattern::ComplexPattern {
            pattern: vec![(0.5, 100), (1.0, 200), (0.3, 150)],
        },
        _ => return Err("Invalid pattern type".to_string()),
    };

    state
        .simulator_manager
        .send_test_haptic(&device_id, pattern)
        .await
        .map_err(|e| e.to_string())
}

/// Get simulator health status
#[tauri::command]
pub async fn get_simulator_health(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<SimulatorStatus, String> {
    state
        .simulator_manager
        .get_simulator_health(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Get connection logs for a simulator
#[tauri::command]
pub async fn get_simulator_logs(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    state
        .simulator_manager
        .get_connection_logs(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Run comprehensive simulator test
#[tauri::command]
pub async fn run_simulator_test(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<TestResults, String> {
    SimulatorTester::new(state.simulator_manager.clone())
        .run_comprehensive_test(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Check if developer mode is enabled
#[tauri::command]
pub async fn is_developer_mode_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.simulator_manager.is_developer_mode_enabled().await)
}

/// Toggle developer mode
#[tauri::command]
pub async fn toggle_developer_mode(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.simulator_manager.set_developer_mode(enabled).await;
    persist_developer_settings(state.simulator_manager.get_settings().await).await?;
    tracing::info!(
        "Developer mode {}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

/// Enable/disable simulator support
#[tauri::command]
pub async fn toggle_simulator_support(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .simulator_manager
        .set_simulator_support(enabled)
        .await
        .map_err(|e| e.to_string())?;
    persist_developer_settings(state.simulator_manager.get_settings().await).await?;
    tracing::info!(
        "Simulator support {}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

/// Get simulator configuration
#[tauri::command]
pub async fn get_simulator_config(
    state: State<'_, AppState>,
) -> Result<crate::config::SimulatorSettings, String> {
    Ok(state.simulator_manager.get_simulator_config().await)
}

/// Update simulator configuration
#[tauri::command]
pub async fn update_simulator_config(
    config: crate::config::SimulatorSettings,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tracing::info!("Updating simulator config: {:?}", config);
    state
        .simulator_manager
        .update_simulator_config(config)
        .await;
    persist_developer_settings(state.simulator_manager.get_settings().await).await?;
    Ok(())
}

/// Auto-discover simulators on localhost
#[tauri::command]
pub async fn auto_discover_simulators(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let discovered = state
        .simulator_manager
        .scan_for_simulators()
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!("Auto-discovered simulators: {:?}", discovered);
    Ok(discovered)
}

/// Get simulator performance metrics
#[tauri::command]
pub async fn get_simulator_metrics(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<crate::simulator::SimulatorMetrics, String> {
    tracing::debug!(device_id = %device_id, "Fetching simulator metrics");
    state
        .simulator_manager
        .get_simulator_metrics(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Start simulator health monitoring
#[tauri::command]
pub async fn start_health_monitoring(
    _device_id: String,
    interval_seconds: u32,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .simulator_manager
        .set_health_check_interval(interval_seconds)
        .await;
    state
        .simulator_manager
        .start()
        .await
        .map_err(|e| e.to_string())?;
    persist_developer_settings(state.simulator_manager.get_settings().await).await?;
    tracing::info!(
        "Starting health monitoring with interval {}s",
        interval_seconds
    );
    Ok(())
}

/// Stop simulator health monitoring
#[tauri::command]
pub async fn stop_health_monitoring(
    _device_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.simulator_manager.stop().await;
    tracing::info!("Stopping simulator health monitoring");
    Ok(())
}

/// Set window size for the application
#[tauri::command]
pub async fn set_window_size(width: f64, height: f64, window: tauri::Window) -> Result<(), String> {
    tracing::info!("Setting window size to {}x{}", width, height);

    let size = tauri::LogicalSize::new(width, height);

    window.set_size(size).map_err(|e| {
        tracing::error!("Failed to set window size: {}", e);
        format!("Failed to set window size: {}", e)
    })?;

    tracing::info!("Window size set successfully to {}x{}", width, height);
    Ok(())
}
