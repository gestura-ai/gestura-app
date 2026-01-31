//! Compile-time regression tests for `gestura-gui` re-export shims.
//!
//! These tests ensure that the GUI maintains stable import paths (`gestura_gui::gdpr`,
//! `gestura_gui::audio_capture`) while business logic lives in `gestura-core`.

/// Ensure key re-export symbols resolve (compile-time guard).
#[test]
fn gdpr_and_audio_capture_reexports_resolve() {
    // GDPR shim
    let _ = gestura_gui::gdpr::get_gdpr_manager;
    let _ = gestura_gui::gdpr::GdprManager::new;

    // Audio capture shim
    let _ = gestura_gui::audio_capture::record_audio;
    let _ = gestura_gui::audio_capture::AudioCaptureConfig::default;
    let _ = gestura_gui::audio_capture::list_audio_input_devices;
    let _ = gestura_gui::audio_capture::request_stop_recording;
    let _ = gestura_gui::audio_capture::reset_stop_flag;
}
