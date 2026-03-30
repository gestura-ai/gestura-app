//! Global hotkeys integration using the Tauri v2 global-shortcut plugin.
//!
//! The configured shortcuts are intended to match the tray-first UX:
//! - listen toggles voice listening
//! - new session opens a new agent window when onboarding is complete

use std::time::Duration;
use tauri::AppHandle;
#[allow(unused_imports)]
use tauri::Manager as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::AppConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HotkeyAction {
    Listen,
    NewSession,
}

fn normalize_shortcut(shortcut: &str) -> Option<String> {
    let trimmed = shortcut.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn handle_listen_hotkey(app: &AppHandle) {
    // Publish an event to NATS if available
    #[cfg_attr(not(feature = "nats"), allow(unused_variables))]
    if let Some(state) = app.try_state::<crate::AppState>() {
        #[cfg(feature = "nats")]
        if let Some(nc) = &state.nats {
            drop(tokio::spawn({
                let nc = nc.clone();
                async move {
                    let _ = nc
                        .publish("events.hotkey", bytes::Bytes::from_static(b"trigger"))
                        .await;
                    tracing::info!("Listen hotkey triggered");
                }
            }));
        }
    }

    // Prefer routing the hotkey to an active CLI session (if one is running).
    // Fall back to the GUI listening toggle if no CLI server responds quickly.
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let routed =
            gestura_core::hotkey_ipc::try_send_hotkey_trigger_to_cli(Duration::from_millis(150))
                .await
                .unwrap_or(false);

        if !routed {
            crate::tray::toggle_listening_mode(&app_handle);
        }
    });
}

fn handle_new_session_hotkey(_app: &AppHandle) {
    if !crate::tray::is_app_configured() {
        tracing::info!(
            "Ignoring new-session hotkey because onboarding/configuration is incomplete"
        );
        return;
    }

    if let Err(error) = crate::window_manager::create_new_agent_session() {
        tracing::error!(%error, "Failed to create agent session from global hotkey");
    }
}

fn register_action_shortcut(app: &AppHandle, shortcut: &str, action: HotkeyAction) {
    if let Err(error) = app
        .global_shortcut()
        .on_shortcut(shortcut, move |app, _shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }

            match action {
                HotkeyAction::Listen => handle_listen_hotkey(app),
                HotkeyAction::NewSession => handle_new_session_hotkey(app),
            }
        })
    {
        tracing::warn!(shortcut, ?action, %error, "Failed to register global shortcut");
    }
}

/// Registers all configured global shortcuts from the current application config.
///
/// Note: The global-shortcut plugin must be registered in main.rs Builder
/// before calling this function. This function only registers shortcuts
/// and handlers using the already-initialized plugin.
pub fn sync_hotkeys(app: &AppHandle, config: &AppConfig) {
    if let Err(error) = app.global_shortcut().unregister_all() {
        tracing::warn!(%error, "Failed to clear existing global shortcuts before re-registering");
    }

    let listen_shortcut = normalize_shortcut(&config.hotkey_listen);
    let new_session_shortcut = normalize_shortcut(&config.hotkey_new_session);

    if let Some(shortcut) = listen_shortcut.as_deref() {
        register_action_shortcut(app, shortcut, HotkeyAction::Listen);
    }

    match (listen_shortcut.as_deref(), new_session_shortcut.as_deref()) {
        (_, None) => {}
        (Some(listen), Some(new_session)) if listen.eq_ignore_ascii_case(new_session) => {
            tracing::warn!(
                shortcut = new_session,
                "Skipping new-session hotkey registration because it duplicates the listen hotkey"
            );
        }
        (_, Some(shortcut)) => {
            register_action_shortcut(app, shortcut, HotkeyAction::NewSession);
        }
    }
}
