//! Security module for encryption, keychain integration, and token management
//! Provides AES encryption for local data and OS keychain integration for secrets

use crate::AppError;
use std::collections::HashMap;

/// Token for MCP authentication
#[derive(Debug, Clone)]
pub struct McpToken {
    pub token: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub scopes: Vec<String>,
}

/// Secure storage interface for sensitive data
#[async_trait::async_trait]
pub trait SecureStorage: Send + Sync {
    /// Store a secret value with the given key
    async fn store_secret(&self, key: &str, value: &str) -> Result<(), AppError>;
    /// Retrieve a secret value by key
    async fn get_secret(&self, key: &str) -> Result<Option<String>, AppError>;
    /// Delete a secret by key
    async fn delete_secret(&self, key: &str) -> Result<(), AppError>;
}

/// Mock secure storage for testing
pub struct MockSecureStorage {
    data: std::sync::RwLock<HashMap<String, String>>,
}

impl Default for MockSecureStorage {
    fn default() -> Self {
        Self {
            data: std::sync::RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl SecureStorage for MockSecureStorage {
    async fn store_secret(&self, key: &str, value: &str) -> Result<(), AppError> {
        let mut data = self
            .data
            .write()
            .map_err(|_| AppError::Io(std::io::Error::other("lock poisoned")))?;
        data.insert(key.to_string(), value.to_string());
        Ok(())
    }

    async fn get_secret(&self, key: &str) -> Result<Option<String>, AppError> {
        let data = self
            .data
            .read()
            .map_err(|_| AppError::Io(std::io::Error::other("lock poisoned")))?;
        Ok(data.get(key).cloned())
    }

    async fn delete_secret(&self, key: &str) -> Result<(), AppError> {
        let mut data = self
            .data
            .write()
            .map_err(|_| AppError::Io(std::io::Error::other("lock poisoned")))?;
        data.remove(key);
        Ok(())
    }
}

/// OS keychain integration (when security feature enabled)
#[cfg(feature = "security")]
pub struct KeychainStorage;

#[cfg(feature = "security")]
#[async_trait::async_trait]
impl SecureStorage for KeychainStorage {
    async fn store_secret(&self, key: &str, value: &str) -> Result<(), AppError> {
        let entry = keyring::Entry::new("gestura", key)
            .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?;
        entry
            .set_password(value)
            .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?;
        Ok(())
    }

    async fn get_secret(&self, key: &str) -> Result<Option<String>, AppError> {
        let entry = keyring::Entry::new("gestura", key)
            .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?;
        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AppError::Io(std::io::Error::other(e.to_string()))),
        }
    }

    async fn delete_secret(&self, key: &str) -> Result<(), AppError> {
        let entry = keyring::Entry::new("gestura", key)
            .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?;
        entry
            .delete_password()
            .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?;
        Ok(())
    }
}

/// Select appropriate secure storage implementation
pub fn create_secure_storage() -> Box<dyn SecureStorage> {
    #[cfg(feature = "security")]
    {
        Box::new(KeychainStorage)
    }
    #[cfg(not(feature = "security"))]
    {
        Box::new(MockSecureStorage::default())
    }
}

/// Encryption utilities for local data
#[cfg(feature = "security")]
pub mod encryption {
    use super::SecureStorage;
    use crate::AppError;
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
    use ring::rand::{SecureRandom, SystemRandom};

    pub struct Encryptor {
        key: LessSafeKey,
        rng: SystemRandom,
    }

    impl Encryptor {
        pub fn new() -> Result<Self, AppError> {
            let rng = SystemRandom::new();
            let mut key_bytes = [0u8; 32];
            rng.fill(&mut key_bytes)
                .map_err(|_| AppError::Io(std::io::Error::other("failed to generate key")))?;
            let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)
                .map_err(|_| AppError::Io(std::io::Error::other("failed to create key")))?;
            let key = LessSafeKey::new(unbound_key);
            Ok(Self { key, rng })
        }

        pub fn from_key(key_bytes: &[u8; 32]) -> Result<Self, AppError> {
            let rng = SystemRandom::new();
            let unbound_key = UnboundKey::new(&AES_256_GCM, key_bytes)
                .map_err(|_| AppError::Io(std::io::Error::other("failed to create key")))?;
            let key = LessSafeKey::new(unbound_key);
            Ok(Self { key, rng })
        }

        pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
            let mut nonce_bytes = [0u8; 12];
            self.rng
                .fill(&mut nonce_bytes)
                .map_err(|_| AppError::Io(std::io::Error::other("failed to generate nonce")))?;
            let nonce = Nonce::assume_unique_for_key(nonce_bytes);

            let mut in_out = data.to_vec();
            self.key
                .seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
                .map_err(|_| AppError::Io(std::io::Error::other("encryption failed")))?;

            let mut result = nonce_bytes.to_vec();
            result.extend_from_slice(&in_out);
            Ok(result)
        }

        pub fn decrypt(&self, encrypted_data: &[u8]) -> Result<Vec<u8>, AppError> {
            if encrypted_data.len() < 12 {
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "encrypted data too short",
                )));
            }

            let (nonce_bytes, ciphertext) = encrypted_data.split_at(12);
            let nonce = Nonce::try_assume_unique_for_key(nonce_bytes).map_err(|_| {
                AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid nonce",
                ))
            })?;

            let mut in_out = ciphertext.to_vec();
            let plaintext = self
                .key
                .open_in_place(nonce, Aad::empty(), &mut in_out)
                .map_err(|_| {
                    AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "decryption failed",
                    ))
                })?;
            Ok(plaintext.to_vec())
        }
    }

    /// Secure configuration manager with encryption
    pub struct SecureConfigManager {
        encryptor: Encryptor,
        #[allow(dead_code)]
        storage: Box<dyn SecureStorage>,
    }

    impl SecureConfigManager {
        pub async fn new() -> Result<Self, AppError> {
            let storage = super::create_secure_storage();

            // Try to load existing key or generate new one
            let encryptor = match storage.get_secret("config_encryption_key").await? {
                Some(key_hex) => {
                    let key_bytes = hex::decode(key_hex).map_err(|_| {
                        AppError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "invalid key format",
                        ))
                    })?;
                    if key_bytes.len() != 32 {
                        return Err(AppError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "invalid key length",
                        )));
                    }
                    let mut key_array = [0u8; 32];
                    key_array.copy_from_slice(&key_bytes);
                    Encryptor::from_key(&key_array)?
                }
                None => {
                    let encryptor = Encryptor::new()?;
                    // Store the key for future use (in production, derive from user password)
                    let rng = SystemRandom::new();
                    let mut key_bytes = [0u8; 32];
                    rng.fill(&mut key_bytes).map_err(|_| {
                        AppError::Io(std::io::Error::other("failed to generate key"))
                    })?;
                    let key_hex = hex::encode(key_bytes);
                    storage
                        .store_secret("config_encryption_key", &key_hex)
                        .await?;
                    encryptor
                }
            };

            Ok(Self { encryptor, storage })
        }

        pub async fn encrypt_config(&self, config: &crate::AppConfig) -> Result<Vec<u8>, AppError> {
            let json = serde_json::to_string(config).map_err(AppError::Json)?;
            self.encryptor.encrypt(json.as_bytes())
        }

        pub async fn decrypt_config(
            &self,
            encrypted_data: &[u8],
        ) -> Result<crate::AppConfig, AppError> {
            let decrypted = self.encryptor.decrypt(encrypted_data)?;
            let json = String::from_utf8(decrypted).map_err(|_| {
                AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid utf8",
                ))
            })?;
            serde_json::from_str(&json).map_err(AppError::Json)
        }
    }
}
