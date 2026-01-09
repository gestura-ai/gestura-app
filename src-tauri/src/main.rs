// Gestura desktop app entrypoint (Tauri v2)
//! Gestura desktop app with embedded NATS (Stages 1–3)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use tauri::{Builder, Manager};

use gestura::agents::AgentManager;
#[allow(unused_imports)]
use gestura::commands;
#[allow(unused_imports)]
use gestura::dispatcher::EventDispatcher;
use gestura::hotkeys::register_hotkey;
use gestura::kv::KvStore;
use gestura::{AppConfig, AppState};

#[tokio::main]
async fn main() {
    // Load configuration and spawn embedded NATS
    let config = AppConfig::load();
    let _nats_child = gestura::nats_mq::spawn_nats_server().ok();

    // Try to connect to NATS; continue even if it fails
    let nats_conn = match gestura::nats_mq::connect_with_retry(&config.nats_url).await {
        Ok(c) => {
            tracing::info!("Connected to NATS at {}", config.nats_url);
            Some(c)
        }
        Err(e) => {
            tracing::warn!("Failed to connect to NATS at {}: {e}", config.nats_url);
            tracing::info!("Continuing without NATS - using in-memory message bus");
            None
        }
    };
    // Attach a KV store if NATS available
    let mut manager = AgentManager::new(AgentManager::default_db_path());
    if nats_conn.is_some() {
        manager.attach_kv(KvStore::new(&config.nats_url, "agents_state"));
    }

    // Create ring manager
    let ring_manager: Option<std::sync::Arc<dyn gestura::ble::RingManager>> =
        Some(std::sync::Arc::from(gestura::ble::create_ring_manager()));

    // Build shared state and wire event forwarding
    let state = AppState {
        nats: nats_conn,
        agents: manager,
        config: config.clone(),
        ring_manager,
    };

    // Initialize event dispatcher and subscribe to NATS events
    #[cfg(feature = "nats")]
    if let Some(nc) = &state.nats {
        let dispatcher = EventDispatcher::new(std::sync::Arc::new(state.agents.clone()));
        let dispatcher_clone = dispatcher.clone();
        let _ = gestura::nats_mq::subscribe_wildcard(
            nc,
            "events.*",
            move |subject: String, data: Vec<u8>| {
                let dispatcher = dispatcher_clone.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = dispatcher.dispatch(&subject, data).await {
                        tracing::error!("Event dispatch error: {}", e);
                    }
                });
            },
        )
        .await;

        // Initialize JetStream KV buckets
        let _ = gestura::nats_mq::init_jetstream(nc, "agents_state").await;

        // Spawn the default agent if missing
        tauri::async_runtime::spawn({
            let agents = state.agents.clone();
            async move {
                agents
                    .spawn_agent("default-agent".into(), "Default".into())
                    .await;
            }
        });
    }

    // Initialize logging
    tracing_subscriber::fmt::init();
    tracing::info!("Starting Gestura app");

    // Run Tauri
    Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            gestura::api::get_config,
            gestura::api::save_config,
            gestura::api::is_first_run,
            gestura::api::get_config_path,
            gestura::api::list_mcp_tools,
            gestura::api::add_mcp_tool,
            gestura::api::remove_mcp_tool,
            gestura::api::get_mdh_pointers,
            gestura::api::set_mdh_pointer,
            gestura::api::remove_mdh_pointer,
            gestura::api::test_llm,
            gestura::api::test_voice,
            gestura::api::test_ollama_connection,
            gestura::api::list_ollama_models,
	            gestura::api::test_local_whisper,
	            gestura::api::validate_whisper_model,
            gestura::api::get_whisper_models,
            gestura::api::get_whisper_model_status,
            gestura::api::is_whisper_model_downloaded,
            gestura::api::download_whisper_model,
            gestura::api::run_voice_once,
            gestura::api::get_ui_prefs,
            gestura::api::set_ui_prefs,
            gestura::api::scan_for_rings,
            gestura::api::get_ring_status,
            gestura::api::pair_ring,
            gestura::api::send_haptic_feedback,
            gestura::api::start_gesture_monitoring,
            gestura::api::stop_gesture_monitoring,
            gestura::api::get_system_health,
            gestura::api::get_metrics_summary,
            gestura::api::get_recent_metrics,
            gestura::api::clear_metrics,
            gestura::api::export_user_data,
            gestura::api::delete_user_data,
            gestura::api::get_user_consents,
            gestura::api::register_consent,
            // Chat and agent commands
            gestura::api::process_chat_message,
            gestura::api::send_agent_message,
            gestura::api::get_agent_status,
            // Audio device management commands
            gestura::api::list_audio_devices,
            gestura::api::check_microphone_available,
            // Permission management commands
            gestura::api::check_permission,
            gestura::api::request_permission,
            // UI testing commands
            gestura::api::test_open_window,
            gestura::api::capture_window_screenshot,
            gestura::api::validate_window_content,
            gestura::api::get_window_list,
            gestura::api::close_test_windows,
            // Automated testing commands
            gestura::automated_testing::run_automated_tests,
            gestura::automated_testing::test_specific_window,
            // Listening state management
            gestura::api::get_listening_state,
            gestura::api::set_listening_timeout,
            gestura::api::toggle_listening,
            gestura::api::validate_voice_config,
            gestura::api::start_voice_listening,
            gestura::api::stop_voice_listening,
            // Speech processing
            gestura::api::update_speech_config,
            gestura::api::get_speech_status,
            // System permissions
            gestura::api::check_system_permissions,
            // Tray diagnostics
            gestura::api::get_tray_diagnostic_info,
            // Session management
            gestura::api::get_chat_sessions,
            gestura::api::restore_chat_session,
            gestura::api::create_chat_session,
            gestura::api::get_session_counts,
            // Window management
            gestura::commands::set_window_size,
            // Onboarding
            gestura::api::complete_onboarding,
            gestura::api::close_onboarding_window,
            gestura::api::open_system_preferences,
            gestura::api::update_voice_provider,
            gestura::api::update_whisper_model,
            gestura::api::update_llm_provider,
            gestura::api::update_audio_device,
            gestura::api::update_ollama_config
        ])
        .setup(move |app| {
            gestura::tray::init_tray(app.handle())?;
            register_hotkey(app.handle(), &config.hotkey_listen);

            // Check if this is the first run and show onboarding window
            if gestura::AppConfig::is_first_run() {
                tracing::info!("First run detected - showing onboarding window");
                // Create a dedicated onboarding window (not the transparent main window)
                if let Err(e) = gestura::window_manager::open_onboarding_window() {
                    tracing::error!("Failed to open onboarding window: {}", e);
                }
            }

            tracing::info!("Gestura app initialized");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Gestura app");
}
