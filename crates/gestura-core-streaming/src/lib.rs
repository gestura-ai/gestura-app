//! Streaming LLM provider support for Gestura.
//!
//! This crate provides streaming capabilities for LLM responses, enabling
//! real-time token-by-token delivery to the frontend with cancellation support.

pub mod cancellation;
pub mod config;
pub mod streaming;

pub use cancellation::{STREAM_CANCELLATIONS, StreamCancellationRegistry};
pub use config::{
    AnthropicProviderConfig, GeminiProviderConfig, GrokProviderConfig, OllamaProviderConfig,
    OpenAiProviderConfig, StreamingConfig,
};
pub use streaming::*;
