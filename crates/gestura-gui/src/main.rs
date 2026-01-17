// Gestura desktop app entrypoint (Tauri v2)
//! Gestura desktop app with embedded NATS (Stages 1–3)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use tauri::{Builder, Manager};

use gestura_gui::agents::AgentManager;
#[allow(unused_imports)]
use gestura_gui::commands;
#[allow(unused_imports)]
use gestura_gui::dispatcher::EventDispatcher;
use gestura_gui::hotkeys::register_hotkey;
use gestura_gui::kv::KvStore;
use gestura_gui::{AppConfig, AppState};

#[tokio::main]
async fn main() {
    // Load configuration and spawn embedded NATS
    let config = AppConfig::load();
    let _nats_child = gestura_gui::nats_mq::spawn_nats_server().ok();

    // Try to connect to NATS; continue even if it fails
    let nats_conn = match gestura_gui::nats_mq::connect_with_retry(&config.nats_url).await {
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
    let ring_manager: Option<std::sync::Arc<dyn gestura_gui::ble::RingManager>> =
        Some(std::sync::Arc::from(gestura_gui::ble::create_ring_manager()));

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
        let _ = gestura_gui::nats_mq::subscribe_wildcard(
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
        let _ = gestura_gui::nats_mq::init_jetstream(nc, "agents_state").await;

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
            gestura_gui::api::get_config,
            gestura_gui::api::save_config,
            gestura_gui::api::is_first_run,
            gestura_gui::api::get_config_path,
            gestura_gui::api::list_mcp_tools,
            gestura_gui::api::add_mcp_tool,
            gestura_gui::api::remove_mcp_tool,
            gestura_gui::api::get_mdh_pointers,
            gestura_gui::api::set_mdh_pointer,
            gestura_gui::api::remove_mdh_pointer,
            gestura_gui::api::test_llm,
            gestura_gui::api::test_voice,
            gestura_gui::api::test_ollama_connection,
            gestura_gui::api::list_ollama_models,
            gestura_gui::api::test_local_whisper,
            gestura_gui::api::validate_whisper_model,
            gestura_gui::api::get_whisper_models,
            gestura_gui::api::get_whisper_model_status,
            gestura_gui::api::is_whisper_model_downloaded,
            gestura_gui::api::download_whisper_model,
            gestura_gui::api::run_voice_once,
            gestura_gui::api::get_ui_prefs,
            gestura_gui::api::set_ui_prefs,
            gestura_gui::api::scan_for_rings,
            gestura_gui::api::get_ring_status,
            gestura_gui::api::pair_ring,
            gestura_gui::api::send_haptic_feedback,
            gestura_gui::api::start_gesture_monitoring,
            gestura_gui::api::stop_gesture_monitoring,
            gestura_gui::api::get_system_health,
            gestura_gui::api::get_metrics_summary,
            gestura_gui::api::get_recent_metrics,
            gestura_gui::api::clear_metrics,
            gestura_gui::api::export_user_data,
            gestura_gui::api::delete_user_data,
            gestura_gui::api::get_user_consents,
            gestura_gui::api::register_consent,
            // Chat and agent commands
            gestura_gui::api::process_chat_message,
            gestura_gui::api::process_chat_message_streaming,
            gestura_gui::api::cancel_chat_streaming,
            gestura_gui::api::send_agent_message,
            gestura_gui::api::get_agent_status,
            gestura_gui::api::list_agents,
            // Orchestrator commands
            gestura_gui::api::delegate_task,
            gestura_gui::api::spawn_subagent,
            gestura_gui::api::list_active_tasks,
            gestura_gui::api::cancel_task,
            // Audio device management commands
            gestura_gui::api::list_audio_devices,
            gestura_gui::api::check_microphone_available,
            // Permission management commands
            gestura_gui::api::check_permission,
            gestura_gui::api::request_permission,
            // UI testing commands
            gestura_gui::api::test_open_window,
            gestura_gui::api::capture_window_screenshot,
            gestura_gui::api::validate_window_content,
            gestura_gui::api::get_window_list,
            gestura_gui::api::close_test_windows,
            // Automated testing commands
            gestura_gui::automated_testing::run_automated_tests,
            gestura_gui::automated_testing::test_specific_window,
            // Listening state management
            gestura_gui::api::get_listening_state,
            gestura_gui::api::set_listening_timeout,
            gestura_gui::api::toggle_listening,
            gestura_gui::api::validate_voice_config,
            gestura_gui::api::start_voice_listening,
            gestura_gui::api::stop_voice_listening,
            // Speech processing
            gestura_gui::api::update_speech_config,
            gestura_gui::api::get_speech_status,
            // System permissions
            gestura_gui::api::check_system_permissions,
            // Tray diagnostics
            gestura_gui::api::get_tray_diagnostic_info,
            // Session management
            gestura_gui::api::get_chat_sessions,
            gestura_gui::api::restore_chat_session,
            gestura_gui::api::create_chat_session,
            gestura_gui::api::get_session_counts,
            // Window management
            gestura_gui::commands::set_window_size,
            // Onboarding
            gestura_gui::api::complete_onboarding,
            gestura_gui::api::close_onboarding_window,
            gestura_gui::api::open_system_preferences,
            gestura_gui::api::update_voice_provider,
            gestura_gui::api::update_whisper_model,
            gestura_gui::api::update_llm_provider,
            gestura_gui::api::update_audio_device,
            gestura_gui::api::update_ollama_config,
            // Notification settings
            gestura_gui::api::get_notification_settings,
            gestura_gui::api::update_notification_settings,
            gestura_gui::api::preview_notification_sound,
            gestura_gui::api::set_notification_ring,
            gestura_gui::api::test_notification
        ])
        .setup(move |app| {
            gestura_gui::tray::init_tray(app.handle())?;
            register_hotkey(app.handle(), &config.hotkey_listen);

            // Check if this is the first run and show onboarding window
            if gestura_gui::AppConfig::is_first_run() {
                tracing::info!("First run detected - showing onboarding window");
                // Create a dedicated onboarding window (not the transparent main window)
                if let Err(e) = gestura_gui::window_manager::open_onboarding_window() {
                    tracing::error!("Failed to open onboarding window: {}", e);
                }
            }

            tracing::info!("Gestura app initialized");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Gestura app");
}
