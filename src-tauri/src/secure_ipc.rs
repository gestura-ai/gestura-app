//! Secure IPC communication for agent processes
//! Provides encrypted communication channels between main process and agents

use crate::AppError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Secure IPC message with encryption
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SecureMessage {
    pub agent_id: String,
    pub message_id: String,
    pub encrypted_payload: Vec<u8>,
    pub signature: Vec<u8>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// IPC encryption key manager
pub struct IpcKeyManager {
    agent_keys: Arc<RwLock<HashMap<String, [u8; 32]>>>,
    master_key: [u8; 32],
}

impl IpcKeyManager {
    /// Create a new key manager
    pub fn new() -> Result<Self, AppError> {
        #[allow(unused_assignments)]
        let mut master_key = [0u8; 32];
        
        #[cfg(feature = "security")]
        {
            use ring::rand::{SecureRandom, SystemRandom};
            let rng = SystemRandom::new();
            rng.fill(&mut master_key).map_err(|_| AppError::Io(std::io::Error::new(std::io::ErrorKind::Other, "failed to generate master key")))?;
        }
        #[cfg(not(feature = "security"))]
        {
            // Use a fixed key for testing when security feature is disabled
            master_key = [42u8; 32];
        }

        Ok(Self {
            agent_keys: Arc::new(RwLock::new(HashMap::new())),
            master_key,
        })
    }

    /// Generate a new key for an agent
    pub async fn generate_agent_key(&self, agent_id: &str) -> Result<[u8; 32], AppError> {
        let mut agent_key = [0u8; 32];
        
        #[cfg(feature = "security")]
        {
            use ring::rand::{SecureRandom, SystemRandom};
            let rng = SystemRandom::new();
            rng.fill(&mut agent_key).map_err(|_| AppError::Io(std::io::Error::new(std::io::ErrorKind::Other, "failed to generate agent key")))?;
        }
        #[cfg(not(feature = "security"))]
        {
            // Derive from master key and agent ID for testing
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            self.master_key.hash(&mut hasher);
            agent_id.hash(&mut hasher);
            let hash = hasher.finish();
            agent_key[..8].copy_from_slice(&hash.to_le_bytes());
        }

        let mut keys = self.agent_keys.write().await;
        keys.insert(agent_id.to_string(), agent_key);
        
        tracing::info!("Generated IPC key for agent: {}", agent_id);
        Ok(agent_key)
    }

    /// Get key for an agent
    pub async fn get_agent_key(&self, agent_id: &str) -> Option<[u8; 32]> {
        let keys = self.agent_keys.read().await;
        keys.get(agent_id).copied()
    }

    /// Remove key for an agent
    pub async fn remove_agent_key(&self, agent_id: &str) {
        let mut keys = self.agent_keys.write().await;
        keys.remove(agent_id);
        tracing::info!("Removed IPC key for agent: {}", agent_id);
    }
}

/// Secure IPC channel for agent communication
pub struct SecureIpcChannel {
    key_manager: Arc<IpcKeyManager>,
    channels: Arc<Mutex<HashMap<String, tokio::sync::mpsc::Sender<SecureMessage>>>>,
}

impl SecureIpcChannel {
    /// Create a new secure IPC channel
    pub fn new(key_manager: Arc<IpcKeyManager>) -> Self {
        Self {
            key_manager,
            channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Encrypt a message for an agent
    #[cfg(feature = "security")]
    async fn encrypt_message(&self, agent_id: &str, payload: &[u8]) -> Result<(Vec<u8>, Vec<u8>), AppError> {
        use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
        use ring::rand::{SecureRandom, SystemRandom};

        let key_bytes = self.key_manager.get_agent_key(agent_id).await
            .ok_or_else(|| AppError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "agent key not found")))?;

        let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)
            .map_err(|_| AppError::Io(std::io::Error::new(std::io::ErrorKind::Other, "failed to create key")))?;
        let key = LessSafeKey::new(unbound_key);

        let rng = SystemRandom::new();
        let mut nonce_bytes = [0u8; 12];
        rng.fill(&mut nonce_bytes).map_err(|_| AppError::Io(std::io::Error::new(std::io::ErrorKind::Other, "failed to generate nonce")))?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let mut in_out = payload.to_vec();
        key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| AppError::Io(std::io::Error::new(std::io::ErrorKind::Other, "encryption failed")))?;

        let mut encrypted = nonce_bytes.to_vec();
        encrypted.extend_from_slice(&in_out);

        // Simple signature (in production, use proper HMAC)
        let signature = format!("sig_{}", agent_id).into_bytes();

        Ok((encrypted, signature))
    }

    /// Encrypt a message for an agent (mock version)
    #[cfg(not(feature = "security"))]
    async fn encrypt_message(&self, agent_id: &str, payload: &[u8]) -> Result<(Vec<u8>, Vec<u8>), AppError> {
        // Mock encryption - just base64 encode for testing
        use base64::{Engine as _, engine::general_purpose};
        let encrypted = general_purpose::STANDARD.encode(payload).into_bytes();
        let signature = format!("mock_sig_{}", agent_id).into_bytes();
        Ok((encrypted, signature))
    }

    /// Send a secure message to an agent
    pub async fn send_secure(&self, agent_id: &str, payload: &[u8]) -> Result<(), AppError> {
        let (encrypted_payload, signature) = self.encrypt_message(agent_id, payload).await?;
        
        let message = SecureMessage {
            agent_id: agent_id.to_string(),
            message_id: uuid::Uuid::new_v4().to_string(),
            encrypted_payload,
            signature,
            timestamp: chrono::Utc::now(),
        };

        let channels = self.channels.lock().await;
        if let Some(sender) = channels.get(agent_id) {
            sender.send(message).await.map_err(|_| AppError::Io(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "channel closed")))?;
            tracing::debug!("Sent secure message to agent: {}", agent_id);
        } else {
            return Err(AppError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "agent channel not found")));
        }

        Ok(())
    }

    /// Register a channel for an agent
    pub async fn register_agent(&self, agent_id: &str, sender: tokio::sync::mpsc::Sender<SecureMessage>) -> Result<(), AppError> {
        // Generate key for the agent
        self.key_manager.generate_agent_key(agent_id).await?;
        
        // Register the channel
        let mut channels = self.channels.lock().await;
        channels.insert(agent_id.to_string(), sender);
        
        tracing::info!("Registered secure IPC channel for agent: {}", agent_id);
        Ok(())
    }

    /// Unregister an agent
    pub async fn unregister_agent(&self, agent_id: &str) {
        let mut channels = self.channels.lock().await;
        channels.remove(agent_id);
        self.key_manager.remove_agent_key(agent_id).await;
        tracing::info!("Unregistered secure IPC channel for agent: {}", agent_id);
    }
}

/// Factory function to create secure IPC components
pub fn create_secure_ipc() -> Result<(Arc<IpcKeyManager>, SecureIpcChannel), AppError> {
    let key_manager = Arc::new(IpcKeyManager::new()?);
    let channel = SecureIpcChannel::new(key_manager.clone());
    Ok((key_manager, channel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_key_generation() {
        let key_manager = IpcKeyManager::new().unwrap();
        
        let key1 = key_manager.generate_agent_key("agent1").await.unwrap();
        let key2 = key_manager.generate_agent_key("agent2").await.unwrap();
        
        // Keys should be different
        assert_ne!(key1, key2);
        
        // Should be able to retrieve keys
        assert_eq!(key_manager.get_agent_key("agent1").await, Some(key1));
        assert_eq!(key_manager.get_agent_key("agent2").await, Some(key2));
    }

    #[tokio::test]
    async fn test_secure_channel() {
        let (_key_manager, channel) = create_secure_ipc().unwrap();
        
        // Create a mock channel for testing
        let (sender, mut receiver) = tokio::sync::mpsc::channel(10);
        
        // Register agent
        channel.register_agent("test-agent", sender).await.unwrap();
        
        // Send secure message
        let test_payload = b"secret message";
        channel.send_secure("test-agent", test_payload).await.unwrap();
        
        // Receive message
        let message = receiver.recv().await.unwrap();
        assert_eq!(message.agent_id, "test-agent");
        assert!(!message.encrypted_payload.is_empty());
        assert!(!message.signature.is_empty());
    }
}
