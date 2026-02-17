//! Retry strategies and execution for Gestura (exponential backoff, jitter)

mod retry;

pub use retry::*;
