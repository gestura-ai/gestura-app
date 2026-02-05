//! Application lifecycle coordination for the tray-first GUI.
//!
//! In a tray-first Tauri app we often want to **keep the process alive** when the last
//! window is closed, while still allowing an explicit “Quit/Exit” action to terminate.
//!
//! Tauri emits a `RunEvent::ExitRequested` when the OS / runtime intends to exit the
//! app (e.g. last window closed). We pair that with an explicit exit request flag.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::AppHandle;

static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Mark that the app is intentionally exiting (e.g. from tray “Exit Gestura”).
///
/// Once this is set, the `RunEvent::ExitRequested` handler should allow exit.
pub fn request_exit(app: &AppHandle, code: i32) {
    EXIT_REQUESTED.store(true, Ordering::SeqCst);
    app.exit(code);
}

/// Returns true when the app has been asked to exit intentionally.
pub fn is_exit_requested() -> bool {
    EXIT_REQUESTED.load(Ordering::SeqCst)
}

#[cfg(test)]
pub(crate) fn reset_exit_requested_for_tests() {
    EXIT_REQUESTED.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_requested_flag_defaults_false_and_can_be_set() {
        reset_exit_requested_for_tests();
        assert!(!is_exit_requested());

        EXIT_REQUESTED.store(true, Ordering::SeqCst);
        assert!(is_exit_requested());
    }
}
