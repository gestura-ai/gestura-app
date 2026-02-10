//! Gestura Core Foundation
//!
//! This crate contains shared primitives that multiple `gestura-core-*` domain crates
//! can depend on without pulling in the full `gestura-core` facade.

pub mod error;
pub mod permissions;

pub use error::{AppError, Result};
pub use permissions::PermissionLevel;
