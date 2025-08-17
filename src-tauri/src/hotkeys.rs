//! Global hotkeys integration using the Tauri v2 global-shortcut plugin.
//! This registers the configured hotkey to toggle/show the main window.

use tauri::{AppHandle, Manager};
#[allow(unused_imports)]
use tauri::Manager as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Code, Shortcut};

/// Registers the global hotkey from configuration.
/// If registration fails, logs a warning but does not crash the app.
pub fn register_hotkey(app: &AppHandle, hotkey: &str) {
    // Parse a simple "Ctrl+Space" style hotkey into modifiers + code
    // For production, consider a more robust parser; this covers our default.
    let mut mods = None;
    let mut code = Code::Space;
    let norm = hotkey.to_ascii_lowercase();
    if norm.contains("ctrl+") || norm.contains("control+") { mods = Some(Modifiers::CONTROL); }
    if norm.contains("alt+") { mods = Some(mods.unwrap_or(Modifiers::empty()) | Modifiers::ALT); }
    if norm.contains("shift+") { mods = Some(mods.unwrap_or(Modifiers::empty()) | Modifiers::SHIFT); }
    if norm.contains("space") { code = Code::Space; }

    let shortcut = Shortcut::new(mods, code);

    // Register the shortcut using the plugin API
    if let Err(e) = app.global_shortcut().register(shortcut.clone()) {
        eprintln!("[hotkeys] failed to register global shortcut: {e}");
        return;
    }

    // Install a handler to publish to NATS and focus the window
    if let Err(e) = app
        .plugin(tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, sc, ev| {
                if sc == &shortcut {
                    use tauri_plugin_global_shortcut::ShortcutState;
                    match ev.state() {
                        ShortcutState::Pressed => {
                            // Publish an event to NATS if available
                            #[cfg_attr(not(feature = "nats"), allow(unused_variables))]
                            if let Some(state) = app.try_state::<crate::AppState>() {
                                #[cfg(feature = "nats")]
                                if let Some(nc) = &state.nats {
                                    let _ = tokio::spawn({
                                        let nc = nc.clone();
                                        async move {
                                            let _ = nc.publish("events.hotkey", bytes::Bytes::from_static(b"trigger")).await;
                                            tracing::info!("Hotkey triggered");
                                        }
                                    });
                                }
                            }
                            if let Some(win) = app.get_webview_window("main") { let _ = win.show(); let _ = win.set_focus(); }
                        }
                        ShortcutState::Released => {}
                    }
                }
            })
            .build()
        )
    {
        eprintln!("[hotkeys] failed to enable handler: {e}");
    }
}

