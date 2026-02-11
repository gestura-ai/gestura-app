//! Streaming cancellation registry.
//!
//! This module centralizes cancellation-token storage semantics for streaming requests.
//! Frontends (GUI/CLI) are responsible for choosing an appropriate cancel key that
//! prevents cross-window or cross-session leakage. The core owns:
//! - token storage
//! - collision semantics (replacing an existing token cancels the old one)
//! - cancellation (remove + cancel)

use std::collections::HashMap;
use std::sync::Mutex;

use crate::streaming::CancellationToken;

/// In-memory registry of streaming cancellation tokens.
#[derive(Debug, Default)]
pub struct StreamCancellationRegistry {
    tokens: Mutex<HashMap<String, CancellationToken>>,
}

impl StreamCancellationRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Register a token for `key`.
    ///
    /// If an existing token is registered under the same key, it is cancelled and replaced.
    pub fn register(&self, key: String, token: CancellationToken) {
        let mut map = self
            .tokens
            .lock()
            .expect("stream cancellation registry poisoned");
        if let Some(prev) = map.insert(key, token) {
            prev.cancel();
        }
    }

    /// Cancel the token for `key`.
    ///
    /// Returns `true` if a token existed and was cancelled.
    pub fn cancel(&self, key: &str) -> bool {
        let mut map = self
            .tokens
            .lock()
            .expect("stream cancellation registry poisoned");
        if let Some(token) = map.remove(key) {
            token.cancel();
            true
        } else {
            false
        }
    }

    /// Remove the token for `key` without cancelling it.
    ///
    /// This is used for cleanup after a stream completes normally.
    pub fn remove(&self, key: &str) {
        let mut map = self
            .tokens
            .lock()
            .expect("stream cancellation registry poisoned");
        map.remove(key);
    }

    /// Returns `true` if the registry currently contains `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        let map = self
            .tokens
            .lock()
            .expect("stream cancellation registry poisoned");
        map.contains_key(key)
    }
}

/// Global cancellation registry used by frontends that need process-wide cancellation.
pub static STREAM_CANCELLATIONS: std::sync::LazyLock<StreamCancellationRegistry> =
    std::sync::LazyLock::new(StreamCancellationRegistry::new);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_replaces_and_cancels_previous() {
        let reg = StreamCancellationRegistry::new();
        let key = "k".to_string();
        let t1 = CancellationToken::new();
        let t2 = CancellationToken::new();

        reg.register(key.clone(), t1.clone());
        reg.register(key.clone(), t2);

        assert!(t1.is_cancelled());
        assert!(reg.contains_key(&key));
    }

    #[test]
    fn cancel_removes_and_cancels() {
        let reg = StreamCancellationRegistry::new();
        let key = "k2".to_string();
        let t = CancellationToken::new();
        reg.register(key.clone(), t.clone());

        assert!(reg.cancel(&key));
        assert!(t.is_cancelled());
        assert!(!reg.contains_key(&key));
        assert!(!reg.cancel(&key));
    }
}
