//! Noise cancellation and audio enhancement - thin wrapper over gestura_core::audio
//!
//! All noise cancellation logic lives in gestura-core. This module re-exports the
//! core types for GUI usage.

pub use gestura_core::audio::{
    NoiseCancellationConfig, NoiseCancellationProcessor, NoiseReductionStats,
    create_music_noise_canceller, create_speech_noise_canceller,
};
