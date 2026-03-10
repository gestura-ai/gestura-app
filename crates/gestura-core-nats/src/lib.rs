//! NATS messaging helpers, server lifecycle, and health monitoring.
//!
//! `gestura-core-nats` provides the optional NATS messaging integration used by
//! Gestura for local or distributed event transport.
//!
//! ## Main entry points
//!
//! - `connect_nats` / `connect_with_retry`: connection helpers
//! - `spawn_nats_server`: embedded/local server process startup
//! - `publish_json`, `subscribe`, `subscribe_wildcard`: messaging primitives
//! - `init_jetstream`: JetStream KV bootstrap helper
//! - `DispatchEvent`: typed event envelope used by messaging flows
//! - `NatsHealthMonitor`: health and reconnection helper when the feature is enabled
//!
//! ## Feature gates
//!
//! The `nats` feature enables the actual `async-nats` implementation. When it
//! is disabled, the crate exposes compatible stub/no-op behavior so dependent
//! code can compile without pretending messaging is active.
//!
//! ## Stable import paths
//!
//! Most code should import through `gestura_core::nats_mq::*`.

mod nats_mq;

pub use nats_mq::*;
