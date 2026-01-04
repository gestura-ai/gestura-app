//! JetStream KV wrapper using async-nats.
//! Provides a simple API to put/get values from a bucket.

use std::io;
#[cfg(feature = "nats")]
use async_nats::jetstream::{self, kv};

/// Simple KV store wrapper that connects on each op for robustness.
#[derive(Clone, Debug)]
pub struct KvStore {
    /// NATS server URL
    #[allow(dead_code)]
    url: String,
    /// KV bucket name
    #[allow(dead_code)]
    bucket: String,
}

#[cfg(feature = "nats")]
impl KvStore {
    /// Create a new KV store wrapper.
    pub fn new(url: impl Into<String>, bucket: impl Into<String>) -> Self {
        Self { url: url.into(), bucket: bucket.into() }
    }

    /// Internal: get or create the KV bucket.
    async fn get_or_create(&self, client: async_nats::Client) -> Result<kv::Store, io::Error> {
        let js = jetstream::new(client);
        match js.get_key_value(self.bucket.clone()).await {
            Ok(store) => Ok(store),
            Err(_) => js
                .create_key_value(kv::Config { bucket: self.bucket.clone(), history: 10, ..Default::default() })
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string())),
        }
    }
}


#[cfg(not(feature = "nats"))]
impl KvStore {
    pub fn new(_url: impl Into<String>, _bucket: impl Into<String>) -> Self {
        // placeholder values when NATS is disabled
        Self { url: String::new(), bucket: String::new() }
    }
    pub async fn put(&self, _key: &str, _value: Vec<u8>) -> Result<(), io::Error> { Ok(()) }
    pub async fn get(&self, _key: &str) -> Result<Option<Vec<u8>>, io::Error> { Ok(None) }
}

#[cfg(feature = "nats")]
impl KvStore {
    /// Put a value into the KV bucket under the given key. (async-nats)
    pub async fn put(&self, key: &str, value: Vec<u8>) -> Result<(), io::Error> {
        let client = async_nats::connect(&self.url).await.map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        let store = self.get_or_create(client).await?;
        store.put(key, bytes::Bytes::from(value)).await.map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        Ok(())
    }

    /// Get a value from the KV bucket.
    pub async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, io::Error> {
        let client = async_nats::connect(&self.url).await.map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        let store = self.get_or_create(client).await?;
        let value_opt = store.get(key).await.map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        Ok(value_opt.map(|b| b.to_vec()))
    }
}

