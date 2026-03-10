//! Audio capture, speech processing, noise reduction, and STT abstractions.
//!
//! `gestura-core-audio` owns the platform-independent audio domain used by the
//! workspace. It covers microphone capture, noise cancellation, speech
//! processing helpers, and speech-to-text provider abstractions that can be
//! consumed by both CLI and GUI entry points.
//!
//! ## Main entry points
//!
//! - `audio_capture`: recording helpers, input-device discovery, and
//!   `AudioCaptureConfig`
//! - `noise_cancellation`: configurable noise reduction processors and stats
//! - `speech`: speech-focused processing utilities
//! - `stt_provider`: the `SttProvider` trait plus concrete providers such as
//!   OpenAI-compatible STT and local Whisper
//!
//! ## Architecture role
//!
//! This crate owns the reusable audio primitives and provider interfaces. GUI
//! permission prompts, tray interactions, and other presentation-layer concerns
//! remain outside this crate.
//!
//! ## Stable import paths
//!
//! Most application code should import these surfaces through:
//!
//! - `gestura_core::audio::*`
//! - `gestura_core::audio_capture::*`
//! - `gestura_core::stt_provider::*`

pub mod audio_capture;
pub mod noise_cancellation;
pub mod speech;
pub mod stt_provider;
