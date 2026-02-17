//! NATS messaging queue utilities for Gestura.
//!
//! Provides embedded NATS server spawning, client connection, publish/subscribe,
//! and JetStream KV bucket initialization.
//!
//! ## Feature gates
//!
//! The `nats` feature enables the actual async-nats implementation.
//! When disabled, stub types and no-op functions are provided.
//!
//! ## Stable import paths
//!
//! Most code should import through `gestura_core::nats_mq::*`.

mod nats_mq;

pub use nats_mq::*;
