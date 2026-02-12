//! Global hotkeys integration using the Tauri v2 global-shortcut plugin.
//!
//! The listen hotkey is intended to **toggle voice listening**, matching the
//! exact behavior of the tray menu item "Start Listening" / "Stop Listening".

#[allow(unused_imports)]
use tauri::Manager as _;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// Registers the global hotkey from configuration.
/// If registration fails, logs a warning but does not crash the app.
///
/// Note: The global-shortcut plugin must be registered in main.rs Builder
/// before calling this function. This function only registers the shortcut
/// and sets up the handler using the already-initialized plugin.
pub fn register_hotkey(app: &AppHandle, hotkey: &str) {
    // Parse a simple "Ctrl+Space" style hotkey into modifiers + code
    // For production, consider a more robust parser; this covers our default.
    let mut mods = None;
    let mut code = Code::Space;
    let norm = hotkey.to_ascii_lowercase();
    if norm.contains("ctrl+") || norm.contains("control+") {
        mods = Some(Modifiers::CONTROL);
    }
    if norm.contains("alt+") {
        mods = Some(mods.unwrap_or(Modifiers::empty()) | Modifiers::ALT);
    }
    if norm.contains("shift+") {
        mods = Some(mods.unwrap_or(Modifiers::empty()) | Modifiers::SHIFT);
    }
    if norm.contains("space") {
        code = Code::Space;
    }

    let shortcut = Shortcut::new(mods, code);

    // Register the shortcut with a handler using the plugin API
    // The plugin was already initialized in main.rs, so we just register the shortcut here
    if let Err(e) = app
        .global_shortcut()
        .on_shortcut(shortcut, move |app, sc, ev| {
            if sc == &shortcut {
                match ev.state() {
                    ShortcutState::Pressed => {
                        // Publish an event to NATS if available
                        #[cfg_attr(not(feature = "nats"), allow(unused_variables))]
                        if let Some(state) = app.try_state::<crate::AppState>() {
                            #[cfg(feature = "nats")]
                            if let Some(nc) = &state.nats {
                                drop(tokio::spawn({
                                    let nc = nc.clone();
                                    async move {
                                        let _ = nc
                                            .publish(
                                                "events.hotkey",
                                                bytes::Bytes::from_static(b"trigger"),
                                            )
                                            .await;
                                        tracing::info!("Hotkey triggered");
                                    }
                                }));
                            }
                        }

                        // IMPORTANT: Do not open any chat window here.
                        // Hotkey must behave exactly like tray "Start Listening".
                        crate::tray::toggle_listening_mode(app);
                    }
                    ShortcutState::Released => {}
                }
            }
        })
    {
        eprintln!("[hotkeys] failed to register global shortcut: {e}");
    }
}
