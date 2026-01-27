//! NATS MQ utilities - thin wrapper over gestura_core::nats_mq
//!
//! This module provides re-exports from gestura-core's nats_mq module.
//! All NATS connection, publish/subscribe, and JetStream logic lives in core.

// Re-export core NATS MQ types and functions
pub use gestura_core::nats_mq::{
    Connection, DispatchEvent, connect_nats, connect_with_retry, init_jetstream, publish_json,
    spawn_nats_server, subjects, subscribe, subscribe_wildcard,
};

#[cfg(feature = "nats")]
pub use gestura_core::nats_mq::NatsHealthMonitor;
