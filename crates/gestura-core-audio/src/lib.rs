//! Gestura Core Audio
//!
//! Audio capture, noise cancellation, speech processing, and speech-to-text
//! provider abstractions. This domain crate contains the platform-independent
//! audio pipeline used by both GUI and CLI entry points.

pub mod audio_capture;
pub mod noise_cancellation;
pub mod speech;
pub mod stt_provider;
