// Gestura desktop app entrypoint (Tauri v2)
//! Gestura desktop app with embedded NATS (Stages 1–3)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use tauri::{Builder, Manager, RunEvent};

use gestura_gui::agents::AgentManager;
#[allow(unused_imports)]
use gestura_gui::commands;
#[allow(unused_imports)]
use gestura_gui::dispatcher::EventDispatcher;
use gestura_gui::hotkeys::register_hotkey;
use gestura_gui::kv::KvStore;
use gestura_gui::{AppConfig, AppConfigSecurityExt, AppState};

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
    let orchestrator = Arc::new(gestura_gui::orchestrator::AgentOrchestrator::new(
        manager.clone(),
        config.clone(),
    ));

    let state = AppState {
        nats: nats_conn,
        agents: manager,
        config: config.clone(),
        orchestrator,
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

    // Build Tauri
    let app = Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            gestura_gui::api::get_config,
            gestura_gui::api::save_config,
            gestura_gui::api::is_first_run,
            gestura_gui::api::get_config_path,
            gestura_gui::api::list_builtin_tools,
            gestura_gui::api::list_mcp_tools,
            gestura_gui::api::add_mcp_tool,
            gestura_gui::api::remove_mcp_tool,
            // MCP Discovery Manager commands
            gestura_gui::api::init_mcp_servers,
            gestura_gui::api::list_discovered_mcp_tools,
            gestura_gui::api::get_mcp_server_status,
            gestura_gui::api::register_mcp_server,
            gestura_gui::api::unregister_mcp_server,
            // MCP Client Runtime commands
            gestura_gui::api::connect_mcp_server,
            gestura_gui::api::disconnect_mcp_server,
            gestura_gui::api::list_connected_mcp_servers,
            gestura_gui::api::list_mcp_client_tools,
            gestura_gui::api::call_mcp_tool,
            gestura_gui::api::get_mdh_pointers,
            gestura_gui::api::set_mdh_pointer,
            gestura_gui::api::remove_mdh_pointer,
            // Knowledge management commands
            gestura_gui::api::add_knowledge_entry,
            gestura_gui::api::list_knowledge_entries,
            gestura_gui::api::search_knowledge,
            gestura_gui::api::test_llm,
            gestura_gui::api::enhance_prompt,
            gestura_gui::api::test_voice,
            gestura_gui::api::test_ollama_connection,
            gestura_gui::api::list_ollama_models,
            gestura_gui::api::list_openai_models,
            gestura_gui::api::list_openai_stt_models,
            gestura_gui::api::list_anthropic_models,
            gestura_gui::api::list_grok_models,
            gestura_gui::api::test_local_whisper,
            gestura_gui::api::validate_whisper_model,
            gestura_gui::api::get_whisper_models,
            gestura_gui::api::get_whisper_model_status,
            gestura_gui::api::is_whisper_model_downloaded,
            gestura_gui::api::download_whisper_model,
            gestura_gui::api::run_voice_once,
            gestura_gui::api::get_ui_prefs,
            gestura_gui::api::set_ui_prefs,
            gestura_gui::api::get_system_theme,
            gestura_gui::api::scan_for_rings,
            gestura_gui::api::get_ring_status,
            gestura_gui::api::pair_ring,
            gestura_gui::api::send_haptic_feedback,
            gestura_gui::api::start_gesture_monitoring,
            gestura_gui::api::stop_gesture_monitoring,
            gestura_gui::api::get_nats_status,
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
            gestura_gui::api::resume_chat_streaming,
            // Tool confirmation (Restricted mode pause/resume)
            gestura_gui::api::approve_tool_confirmation,
            gestura_gui::api::resolve_tool_confirmation_decision,
            gestura_gui::api::deny_tool_confirmation,
            // Chat diagnostics
            gestura_gui::api::get_chat_event_trace,
            gestura_gui::api::clear_chat_event_trace,
            gestura_gui::api::record_chat_receipt,
            gestura_gui::api::get_chat_receipt_trace,
            gestura_gui::api::clear_chat_receipt_trace,
            gestura_gui::api::run_chat_isolation_probe,
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
            // Window management commands
            gestura_gui::api::open_config_window,
            // UI testing commands
            gestura_gui::api::test_open_window,
            gestura_gui::api::capture_window_screenshot,
            gestura_gui::api::validate_window_content,
            gestura_gui::api::get_window_list,
            gestura_gui::api::close_test_windows,
            // Screen capture commands
            gestura_gui::api::capture_screenshot,
            gestura_gui::api::start_screen_recording,
            gestura_gui::api::stop_screen_recording,
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
            gestura_gui::api::get_session_history,
            gestura_gui::api::get_session_workspace,
            gestura_gui::api::get_session_workspace_by_id,
            gestura_gui::api::set_session_workspace,
            gestura_gui::api::pick_workspace_directory,
            // Session convenience actions
            gestura_gui::api::open_shell_for_session,
            // Shell process control (inline shell console)
            gestura_gui::api::shell_process_stop,
            gestura_gui::api::shell_process_pause,
            gestura_gui::api::shell_process_resume,
            gestura_gui::api::shell_process_rerun_info,
            // Session LLM config (session-scoped, doesn't modify global config)
            gestura_gui::api::get_session_llm_config,
            gestura_gui::api::set_session_llm_provider,
            gestura_gui::api::set_session_llm_model,
            gestura_gui::api::clear_session_llm_config,
            gestura_gui::api::get_effective_llm_config,
            // Session Voice/STT config (session-scoped, doesn't modify global config)
            gestura_gui::api::get_session_voice_config,
            gestura_gui::api::set_session_voice_provider,
            gestura_gui::api::set_session_voice_model,
            gestura_gui::api::clear_session_voice_config,
            // Session tool and permission settings
            gestura_gui::api::get_session_tool_settings,
            gestura_gui::api::set_session_permission_level,
            gestura_gui::api::set_session_tool_enabled,
            gestura_gui::api::is_session_tool_enabled,
            gestura_gui::api::is_session_action_allowed,
            gestura_gui::api::session_requires_confirmation,
            // Task management
            gestura_gui::api::create_task,
            gestura_gui::api::update_task_status,
            gestura_gui::api::update_task,
            gestura_gui::api::delete_task,
            gestura_gui::api::list_tasks,
            gestura_gui::api::get_task_hierarchy,
            gestura_gui::api::break_down_requirements,
            // Knowledge management
            gestura_gui::api::list_knowledge_items,
            gestura_gui::api::get_knowledge_item,
            gestura_gui::api::set_knowledge_enabled,
            gestura_gui::api::get_enabled_knowledge,
            // Simulator management
            gestura_gui::commands::get_simulators,
            gestura_gui::commands::scan_for_simulators,
            gestura_gui::commands::reset_simulator,
            gestura_gui::commands::send_test_haptic,
            gestura_gui::commands::get_simulator_health,
            gestura_gui::commands::get_simulator_logs,
            gestura_gui::commands::run_simulator_test,
            gestura_gui::commands::is_developer_mode_enabled,
            gestura_gui::commands::toggle_developer_mode,
            gestura_gui::commands::toggle_simulator_support,
            gestura_gui::commands::get_simulator_config,
            gestura_gui::commands::update_simulator_config,
            gestura_gui::commands::auto_discover_simulators,
            gestura_gui::commands::get_simulator_metrics,
            gestura_gui::commands::start_health_monitoring,
            gestura_gui::commands::stop_health_monitoring,
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
            gestura_gui::api::test_notification,
            // Secure secret management
            gestura_gui::api::store_secret,
            gestura_gui::api::get_secret,
            gestura_gui::api::delete_secret,
            gestura_gui::api::is_keychain_available,
            gestura_gui::api::store_api_key,
            gestura_gui::api::get_api_key,
            gestura_gui::api::delete_api_key,
            gestura_gui::api::has_api_key,
            gestura_gui::api::get_available_llm_providers,
            gestura_gui::api::migrate_api_keys_to_keychain,
            // Hooks settings
            gestura_gui::api::get_hooks_settings,
            gestura_gui::api::set_hooks_settings,
            gestura_gui::api::set_hooks_enabled,
            // Checkpoints
            gestura_gui::api::list_session_checkpoints,
            gestura_gui::api::restore_session_checkpoint,
            // Tool permission grants
            gestura_gui::api::list_tool_permission_grants,
            gestura_gui::api::get_permission_audit_log,
            gestura_gui::api::revoke_tool_permission,
            // Global permission settings
            gestura_gui::api::get_global_permission_settings,
            gestura_gui::api::set_global_permission_settings,
            gestura_gui::api::set_default_permission_level
        ])
        .setup(move |app| {
            // Extend the asset-protocol scope so the webview can load screenshots
            // and other artifacts stored under ~/.gestura/ (session workspaces).
            if let Some(home) = dirs::home_dir() {
                let gestura_dir = home.join(".gestura");
                if let Err(e) = app
                    .asset_protocol_scope()
                    .allow_directory(&gestura_dir, true)
                {
                    tracing::warn!(
                        "Failed to add {} to asset protocol scope: {}",
                        gestura_dir.display(),
                        e
                    );
                }
                tracing::info!("Asset protocol scope extended to {}", gestura_dir.display());
            }

            // Attach the GUI observer for core orchestrator task lifecycle events.
            //
            // This keeps the orchestrator core-owned (tauri-free) while still enabling
            // task panel syncing in the GUI.
            let orchestrator = app.state::<AppState>().orchestrator.clone();
            let observer = Arc::new(gestura_gui::orchestrator::TauriTaskObserver::new(
                app.handle().clone(),
            ));
            tauri::async_runtime::spawn(async move {
                orchestrator.set_observer(observer).await;
            });

            gestura_gui::tray::init_tray(app.handle())?;
            register_hotkey(app.handle(), &config.hotkey_listen);

            // Check if this is the first run and show onboarding window
            if gestura_gui::AppConfig::is_first_run() {
                tracing::info!("First run detected - showing onboarding window");
                // Create a dedicated onboarding window (the app is tray-first and does not
                // create a default "main" window at startup).
                if let Err(e) = gestura_gui::window_manager::open_onboarding_window() {
                    tracing::error!("Failed to open onboarding window: {}", e);
                }
            }

            tracing::info!("Gestura app initialized");
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Gestura app");

    // Tray-first behavior:
    // - Closing the last window should NOT terminate the process.
    // - Explicit Quit/Exit (tray menu) should terminate.
    app.run(|_app_handle, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            // Only prevent exiting when we successfully created a tray icon.
            // If the tray failed to initialize, allow exit so the app doesn't become
            // un-quit-able.
            let tray_ok = gestura_gui::tray::is_tray_running();

            if tray_ok && !gestura_gui::app_lifecycle::is_exit_requested() {
                tracing::info!(
                    "Exit requested while in tray-first mode (likely last window closed); preventing exit"
                );
                api.prevent_exit();
            }
        }
    });
}
