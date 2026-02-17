//! Smart caching for context data
//!
//! Provides LRU-style caching with TTL for context data that can be
//! expensive to compute or fetch.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Default cache TTL in seconds
const DEFAULT_TTL_SECS: u64 = 300; // 5 minutes
/// Maximum cache entries
const MAX_CACHE_SIZE: usize = 100;

/// A cached entry with TTL
#[derive(Debug, Clone)]
pub struct CacheEntry<T> {
    /// The cached value
    pub value: T,
    /// When this entry was created
    pub created_at: Instant,
    /// Time-to-live for this entry
    pub ttl: Duration,
    /// Number of times this entry was accessed
    pub access_count: u32,
    /// Last access time
    pub last_accessed: Instant,
}

impl<T: Clone> CacheEntry<T> {
    /// Create a new cache entry
    pub fn new(value: T, ttl: Duration) -> Self {
        let now = Instant::now();
        Self {
            value,
            created_at: now,
            ttl,
            access_count: 0,
            last_accessed: now,
        }
    }

    /// Check if this entry is expired
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.ttl
    }

    /// Mark as accessed
    pub fn touch(&mut self) {
        self.access_count += 1;
        self.last_accessed = Instant::now();
    }
}

/// Smart cache for context data
pub struct ContextCache<T> {
    /// Internal storage
    entries: RwLock<HashMap<String, CacheEntry<T>>>,
    /// Default TTL for entries
    default_ttl: Duration,
    /// Maximum number of entries
    max_size: usize,
}

impl<T: Clone> ContextCache<T> {
    /// Create a new cache with default settings
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            default_ttl: Duration::from_secs(DEFAULT_TTL_SECS),
            max_size: MAX_CACHE_SIZE,
        }
    }

    /// Create with custom TTL
    pub fn with_ttl(ttl_secs: u64) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            default_ttl: Duration::from_secs(ttl_secs),
            max_size: MAX_CACHE_SIZE,
        }
    }

    /// Get a value from the cache
    pub fn get(&self, key: &str) -> Option<T> {
        let mut entries = self.entries.write().ok()?;
        if let Some(entry) = entries.get_mut(key) {
            if entry.is_expired() {
                entries.remove(key);
                return None;
            }
            entry.touch();
            return Some(entry.value.clone());
        }
        None
    }

    /// Insert a value into the cache
    pub fn insert(&self, key: impl Into<String>, value: T) {
        self.insert_with_ttl(key, value, self.default_ttl);
    }

    /// Insert with custom TTL
    pub fn insert_with_ttl(&self, key: impl Into<String>, value: T, ttl: Duration) {
        if let Ok(mut entries) = self.entries.write() {
            // Evict if needed
            if entries.len() >= self.max_size {
                self.evict_oldest(&mut entries);
            }
            entries.insert(key.into(), CacheEntry::new(value, ttl));
        }
    }

    /// Remove a value from the cache
    pub fn remove(&self, key: &str) -> Option<T> {
        self.entries
            .write()
            .ok()
            .and_then(|mut e| e.remove(key))
            .map(|e| e.value)
    }

    /// Clear all entries
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.write() {
            entries.clear();
        }
    }

    /// Get the number of entries
    pub fn len(&self) -> usize {
        self.entries.read().map(|e| e.len()).unwrap_or(0)
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Evict expired entries
    pub fn evict_expired(&self) {
        if let Ok(mut entries) = self.entries.write() {
            entries.retain(|_, entry| !entry.is_expired());
        }
    }

    /// Evict the oldest entry
    fn evict_oldest(&self, entries: &mut HashMap<String, CacheEntry<T>>) {
        // Find the oldest entry by last access time
        if let Some(oldest_key) = entries
            .iter()
            .min_by_key(|(_, e)| e.last_accessed)
            .map(|(k, _)| k.clone())
        {
            entries.remove(&oldest_key);
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let entries = self.entries.read().ok();
        let (size, total_accesses, expired) = entries
            .map(|e| {
                let expired = e.values().filter(|v| v.is_expired()).count();
                let total: u32 = e.values().map(|v| v.access_count).sum();
                (e.len(), total, expired)
            })
            .unwrap_or((0, 0, 0));

        CacheStats {
            size,
            max_size: self.max_size,
            total_accesses,
            expired_count: expired,
        }
    }
}

impl<T: Clone> Default for ContextCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    /// Current number of entries
    pub size: usize,
    /// Maximum allowed entries
    pub max_size: usize,
    /// Total access count across all entries
    pub total_accesses: u32,
    /// Number of currently expired entries (pending cleanup)
    pub expired_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_cache_basic() {
        let cache: ContextCache<String> = ContextCache::new();
        cache.insert("key1", "value1".to_string());
        cache.insert("key2", "value2".to_string());

        assert_eq!(cache.get("key1"), Some("value1".to_string()));
        assert_eq!(cache.get("key2"), Some("value2".to_string()));
        assert_eq!(cache.get("key3"), None);
    }

    #[test]
    fn test_cache_expiry() {
        let cache: ContextCache<String> = ContextCache::with_ttl(1);
        cache.insert("key1", "value1".to_string());

        assert_eq!(cache.get("key1"), Some("value1".to_string()));

        // Wait for expiry
        thread::sleep(Duration::from_millis(1100));

        assert_eq!(cache.get("key1"), None);
    }

    #[test]
    fn test_cache_remove() {
        let cache: ContextCache<String> = ContextCache::new();
        cache.insert("key1", "value1".to_string());

        assert_eq!(cache.remove("key1"), Some("value1".to_string()));
        assert_eq!(cache.get("key1"), None);
    }

    #[test]
    fn test_cache_clear() {
        let cache: ContextCache<String> = ContextCache::new();
        cache.insert("key1", "value1".to_string());
        cache.insert("key2", "value2".to_string());

        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_stats() {
        let cache: ContextCache<String> = ContextCache::new();
        cache.insert("key1", "value1".to_string());
        cache.get("key1");
        cache.get("key1");

        let stats = cache.stats();
        assert_eq!(stats.size, 1);
        assert_eq!(stats.total_accesses, 2);
    }
}
