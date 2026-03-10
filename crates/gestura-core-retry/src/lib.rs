//! Retry policies, error classification, and retry execution helpers.
//!
//! `gestura-core-retry` provides a small retry domain for transient failures
//! across APIs, tools, and streaming operations.
//!
//! ## Main concepts
//!
//! - `ErrorClass`: classify failures as transient, permanent, or unknown
//! - `RetryPolicy`: reusable retry settings including attempts, backoff, and jitter
//! - `RetryEvent`: structured retry notification payload
//! - `RetryManager`: execute async operations with policy-driven retries
//!
//! ## Built-in policies
//!
//! The crate includes convenience constructors for common domains:
//!
//! - `RetryPolicy::for_api()` / `RetryManager::for_api()`
//! - `RetryPolicy::for_tools()` / `RetryManager::for_tools()`
//! - `RetryPolicy::for_streaming()` / `RetryManager::for_streaming()`
//!
//! ## Stable import paths
//!
//! Most code should import through `gestura_core::retry::*`.

mod retry;

pub use retry::*;
