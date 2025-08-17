// Gestura desktop app entrypoint (Tauri v2)
//! Gestura desktop app with embedded NATS (Stages 1–3)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Builder;
#[allow(unused_imports)]
use std::sync::Arc;

use gestura_app::{AppConfig, AppState};
use gestura_app::agents::AgentManager;
use gestura_app::kv::KvStore;
use gestura_app::hotkeys::register_hotkey;
use gestura_app::commands;
#[allow(unused_imports)]
use gestura_app::dispatcher::EventDispatcher;
#[cfg(not(feature = "nats"))]
// use gestura_app::nats_mq as _; // intentionally unused import removed





#[tokio::main]
async fn main() {
    // Load configuration and spawn embedded NATS
    let config = AppConfig::load();
    let _nats_child = gestura_app::nats_mq::spawn_nats_server().ok();

    // Try to connect to NATS; continue even if it fails
    let nats_conn = match gestura_app::nats_mq::connect_with_retry(&config.nats_url).await {
        Ok(c) => {
            tracing::info!("Connected to NATS at {}", config.nats_url);
            Some(c)
        },
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
    let ring_manager: Option<std::sync::Arc<dyn gestura_app::ble::RingManager>> = Some(std::sync::Arc::from(gestura_app::ble::create_ring_manager()));

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
        let _ = gestura_app::nats_mq::subscribe_wildcard(nc, "events.*", move |subject: String, data: Vec<u8>| {
            let dispatcher = dispatcher_clone.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = dispatcher.dispatch(&subject, data).await {
                    tracing::error!("Event dispatch error: {}", e);
                }
            });
        }).await;

        // Initialize JetStream KV buckets
        let _ = gestura_app::nats_mq::init_jetstream(nc, "agents_state").await;

        // Spawn the default agent if missing
        tauri::async_runtime::spawn({
            let agents = state.agents.clone();
            async move {
                agents.spawn_agent("default-agent".into(), "Default".into()).await;
            }
        });
    }

    // Initialize logging
    tracing_subscriber::fmt::init();
    tracing::info!("Starting Gestura app");

    // Run Tauri
    Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            gestura_app::api::get_config,
            gestura_app::api::save_config,
            gestura_app::api::list_mcp_tools,
            gestura_app::api::add_mcp_tool,
            gestura_app::api::remove_mcp_tool,
            gestura_app::api::get_mdh_pointers,
            gestura_app::api::set_mdh_pointer,
            gestura_app::api::remove_mdh_pointer,
            gestura_app::api::test_llm,
            gestura_app::api::test_voice,
            gestura_app::api::run_voice_once,
            gestura_app::api::get_ui_prefs,
            gestura_app::api::set_ui_prefs,
            gestura_app::api::scan_for_rings,
            gestura_app::api::get_ring_status,
            gestura_app::api::pair_ring,
            gestura_app::api::send_haptic_feedback,
            gestura_app::api::start_gesture_monitoring,
            gestura_app::api::stop_gesture_monitoring,
            gestura_app::api::get_system_health,
            gestura_app::api::get_metrics_summary,
            gestura_app::api::get_recent_metrics,
            gestura_app::api::clear_metrics,
            gestura_app::api::export_user_data,
            gestura_app::api::delete_user_data,
            gestura_app::api::get_user_consents,
            gestura_app::api::register_consent,
            // Chat and agent commands
            gestura_app::api::send_agent_message,
            gestura_app::api::get_agent_status,
            // Permission management commands
            gestura_app::api::check_permission,
            gestura_app::api::request_permission,
            // UI testing commands
            gestura_app::api::test_open_window,
            gestura_app::api::capture_window_screenshot,
            gestura_app::api::validate_window_content,
            gestura_app::api::get_window_list,
            gestura_app::api::close_test_windows,
            // Automated testing commands
            gestura_app::automated_testing::run_automated_tests,
            gestura_app::automated_testing::test_specific_window,
            // Listening state management
            gestura_app::api::get_listening_state,
            gestura_app::api::set_listening_timeout,
            gestura_app::api::toggle_listening,
            // Speech processing
            gestura_app::api::update_speech_config,
            gestura_app::api::get_speech_status,
            // System permissions
            gestura_app::api::check_system_permissions,
            // Tray diagnostics
            gestura_app::api::get_tray_diagnostic_info,
            // Window management
            gestura_app::commands::set_window_size
        ])
        .setup(move |app| {
            gestura_app::tray::init_tray(app.handle())?;
            register_hotkey(app.handle(), &config.hotkey_listen);
            tracing::info!("Gestura app initialized");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Gestura app");
}

