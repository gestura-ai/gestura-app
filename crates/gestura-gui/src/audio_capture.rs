//! Audio capture module shim for `gestura-gui`.
//!
//! Core-first architecture rule: audio capture business logic (cpal/VAD/encoding)
//! lives in `gestura-core`. The GUI crate keeps only a small adapter to preserve
//! stable import paths and to apply GUI configuration defaults.

use std::path::Path;
use std::time::Duration;

pub use gestura_core::audio_capture::{
    AudioCaptureConfig, AudioDeviceInfo, is_microphone_available, is_stop_requested,
    list_audio_input_devices, request_stop_recording, reset_stop_flag,
};

/// Record audio from the microphone using the core audio-capture implementation.
///
/// This preserves the legacy GUI signature (`record_audio(duration, output_path)`) while
/// delegating the actual capture/VAD/resampling logic to `gestura-core`.
///
/// The selected input device is resolved from the GUI-visible [`crate::config::AppConfig`]
/// (`voice.audio_device`) and passed to core via [`AudioCaptureConfig`].
pub async fn record_audio(duration: Duration, output_path: &Path) -> Result<f32, crate::AppError> {
    let cfg = crate::config::AppConfig::load();

    let capture_cfg = AudioCaptureConfig {
        device_name: cfg.voice.audio_device.clone(),
        ..Default::default()
    };

    gestura_core::audio_capture::record_audio(duration, output_path, capture_cfg).await
}
