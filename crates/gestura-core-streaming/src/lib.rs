//! Streaming response types, config, and cancellation support for Gestura.
//!
//! `gestura-core-streaming` provides the shared building blocks for incremental
//! LLM response delivery across the workspace. It defines the streaming event
//! model surfaced to UIs, the decoupled streaming configuration types used at
//! runtime, and the cancellation registry that coordinates interruption of
//! in-flight streams.
//!
//! ## Responsibilities
//!
//! - `streaming`: core streaming events, provider-specific streaming helpers,
//!   token-usage updates, tool-call chunks, shell output chunks, and status
//!   notifications used by CLI/GUI frontends
//! - `config`: a portable `StreamingConfig` model derived from application
//!   configuration without tying the streaming crate directly to `AppConfig`
//! - `cancellation`: cooperative cancellation-token registry for active streams
//!
//! ## Architecture boundary
//!
//! This crate owns the streaming protocol and runtime primitives. Higher-level
//! orchestration—such as deciding when to start a stream, how to bridge it into
//! sessions, and how to mix streaming with tool execution—remains in
//! `gestura-core`.
//!
//! ## Stable import paths
//!
//! Most consumers should import through the facade re-exports:
//!
//! - `gestura_core::streaming::*`
//! - `gestura_core::stream_cancellation::*`
//!
//! ## Why the config is separate
//!
//! `StreamingConfig` mirrors only the subset of provider configuration needed
//! for streaming. This keeps the crate portable, easier to test, and less
//! tightly coupled to the larger application configuration surface.

pub mod cancellation;
pub mod config;
pub mod streaming;

pub use cancellation::{STREAM_CANCELLATIONS, StreamCancellationRegistry};
pub use config::{
    AnthropicProviderConfig, GeminiProviderConfig, GrokProviderConfig, OllamaProviderConfig,
    OpenAiProviderConfig, StreamingConfig,
};
pub use streaming::*;
