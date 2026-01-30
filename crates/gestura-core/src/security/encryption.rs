//! Encryption utilities for local data protection
//!
//! Provides AES-256-GCM encryption for protecting sensitive configuration
//! and user data at rest.
//!
//! # Security Notes
//!
//! - Uses AES-256-GCM for authenticated encryption
//! - Nonces are randomly generated and prepended to ciphertext
//! - Keys should be stored securely (e.g., in OS keychain)

use super::SecureStorage;
use crate::error::AppError;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};

/// AES-256-GCM encryptor for protecting sensitive data
pub struct Encryptor {
    key: LessSafeKey,
    rng: SystemRandom,
}

impl Encryptor {
    /// Create a new encryptor with a randomly generated key
    pub fn new() -> Result<Self, AppError> {
        let rng = SystemRandom::new();
        let mut key_bytes = [0u8; 32];
        rng.fill(&mut key_bytes)
            .map_err(|_| AppError::Internal("failed to generate encryption key".to_string()))?;
        Self::from_key(&key_bytes)
    }

    /// Create an encryptor from an existing 32-byte key
    pub fn from_key(key_bytes: &[u8; 32]) -> Result<Self, AppError> {
        let rng = SystemRandom::new();
        let unbound_key = UnboundKey::new(&AES_256_GCM, key_bytes)
            .map_err(|_| AppError::Internal("failed to create encryption key".to_string()))?;
        let key = LessSafeKey::new(unbound_key);
        Ok(Self { key, rng })
    }

    /// Encrypt data, prepending the nonce to the ciphertext
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        let mut nonce_bytes = [0u8; 12];
        self.rng
            .fill(&mut nonce_bytes)
            .map_err(|_| AppError::Internal("failed to generate nonce".to_string()))?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let mut in_out = data.to_vec();
        self.key
            .seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| AppError::Internal("encryption failed".to_string()))?;

        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&in_out);
        Ok(result)
    }

    /// Decrypt data (nonce is expected to be prepended)
    pub fn decrypt(&self, encrypted_data: &[u8]) -> Result<Vec<u8>, AppError> {
        if encrypted_data.len() < 12 {
            return Err(AppError::InvalidInput(
                "encrypted data too short".to_string(),
            ));
        }

        let (nonce_bytes, ciphertext) = encrypted_data.split_at(12);
        let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
            .map_err(|_| AppError::InvalidInput("invalid nonce".to_string()))?;

        let mut in_out = ciphertext.to_vec();
        let plaintext = self
            .key
            .open_in_place(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| AppError::InvalidInput("decryption failed".to_string()))?;
        Ok(plaintext.to_vec())
    }
}

/// Secure configuration manager with encryption and keychain-backed key storage
pub struct SecureConfigManager {
    encryptor: Encryptor,
    #[allow(dead_code)]
    storage: Box<dyn SecureStorage>,
}

impl SecureConfigManager {
    /// Create a new secure config manager
    ///
    /// Loads or generates an encryption key stored in the OS keychain.
    pub async fn new(storage: Box<dyn SecureStorage>) -> Result<Self, AppError> {
        let encryptor = match storage.get_secret("config_encryption_key").await? {
            Some(key_hex) => {
                let key_bytes = hex::decode(key_hex)
                    .map_err(|_| AppError::InvalidInput("invalid key format".to_string()))?;
                if key_bytes.len() != 32 {
                    return Err(AppError::InvalidInput("invalid key length".to_string()));
                }
                let mut key_array = [0u8; 32];
                key_array.copy_from_slice(&key_bytes);
                Encryptor::from_key(&key_array)?
            }
            None => {
                let rng = SystemRandom::new();
                let mut key_bytes = [0u8; 32];
                rng.fill(&mut key_bytes)
                    .map_err(|_| AppError::Internal("failed to generate key".to_string()))?;
                let key_hex = hex::encode(key_bytes);
                storage
                    .store_secret("config_encryption_key", &key_hex)
                    .await?;
                Encryptor::from_key(&key_bytes)?
            }
        };

        Ok(Self { encryptor, storage })
    }

    /// Create with provided storage (for testing)
    pub async fn with_storage(storage: Box<dyn SecureStorage>) -> Result<Self, AppError> {
        Self::new(storage).await
    }

    /// Encrypt serializable data
    pub fn encrypt<T: serde::Serialize>(&self, data: &T) -> Result<Vec<u8>, AppError> {
        let json = serde_json::to_string(data)?;
        self.encryptor.encrypt(json.as_bytes())
    }

    /// Decrypt to deserializable data
    pub fn decrypt<T: serde::de::DeserializeOwned>(&self, encrypted: &[u8]) -> Result<T, AppError> {
        let decrypted = self.encryptor.decrypt(encrypted)?;
        let json = String::from_utf8(decrypted)
            .map_err(|_| AppError::InvalidInput("invalid UTF-8".to_string()))?;
        serde_json::from_str(&json).map_err(AppError::Json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::MockSecureStorage;

    #[test]
    fn test_encryptor_encrypt_decrypt() {
        let encryptor = Encryptor::new().unwrap();
        let plaintext = b"Hello, World!";
        let encrypted = encryptor.encrypt(plaintext).unwrap();
        let decrypted = encryptor.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encryptor_from_key() {
        let key = [0u8; 32];
        let encryptor = Encryptor::from_key(&key).unwrap();
        let plaintext = b"test data";
        let encrypted = encryptor.encrypt(plaintext).unwrap();

        // Decrypt with same key
        let encryptor2 = Encryptor::from_key(&key).unwrap();
        let decrypted = encryptor2.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_decrypt_invalid_data() {
        let encryptor = Encryptor::new().unwrap();
        let result = encryptor.decrypt(&[0u8; 5]);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_secure_config_manager() {
        let storage = Box::new(MockSecureStorage::new());
        let manager = SecureConfigManager::with_storage(storage).await.unwrap();

        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct TestConfig {
            api_key: String,
            enabled: bool,
        }

        let config = TestConfig {
            api_key: "secret123".to_string(),
            enabled: true,
        };

        let encrypted = manager.encrypt(&config).unwrap();
        let decrypted: TestConfig = manager.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, config);
    }
}
