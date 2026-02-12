//! MCP Integrator - Token management and tool exposure
//! Provides McpIntegrator trait and LocalMcp implementation for
//! tool exposure, dual authentication, and MDH translation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::{path::PathBuf, sync::RwLock};

use crate::error::AppError;

/// Token information with expiration and permissions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    /// The token string
    pub token: String,
    /// When the token was created
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the token expires
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Whether haptic permission is granted
    pub haptic_permission: bool,
    /// Client identifier
    pub client_id: String,
    /// Scopes granted to this token
    pub scopes: Vec<String>,
}

/// Result of local MDH translation suitable for MCP usage
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MdhResource {
    /// A synthetic MCP URI derived from JSON-LD @type or file stem
    pub uri: String,
    /// A compacted JSON payload (local-only)
    pub data: serde_json::Value,
}

/// Unifies MCP operations with dual authentication
#[async_trait::async_trait]
pub trait McpIntegrator: Send + Sync {
    /// Expose a tool name for use by clients
    async fn expose_tool(&self, tool: &str) -> Result<(), AppError>;
    /// Perform dual auth (app approval + MCP token)
    async fn authenticate_haptic(&self, token: &str) -> Result<bool, AppError>;
    /// Validate MCP token
    async fn validate_token(&self, token: &str) -> Result<bool, AppError>;
    /// Register haptic permission for token
    async fn grant_haptic_permission(&self, token: &str) -> Result<(), AppError>;
}

/// Local-only MDH translate: read JSON file, validate minimal JSON-LD shape, and create URI
/// NOTE: In production we will use json-ld-rs to expand/compact; here we simulate locally
pub fn mdh_translate(ld_file: PathBuf) -> Result<MdhResource, AppError> {
    let content = std::fs::read_to_string(&ld_file)
        .map_err(|e| AppError::Io(std::io::Error::other(format!("read mdh file: {e}"))))?;
    let mut value: serde_json::Value = serde_json::from_str(&content).map_err(AppError::Json)?;

    // Basic validation and derive a type for URI
    let mut uri_type = None;
    if let serde_json::Value::Object(map) = &value
        && let Some(t) = map.get("@type").and_then(|v| v.as_str())
    {
        uri_type = Some(t.to_string());
    }
    if uri_type.is_none() {
        // try a nested object or fallback to filename stem
        uri_type = Some(
            ld_file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("local")
                .to_string(),
        );
    }

    // Simulate compacting by removing @context if present (local-only)
    if let serde_json::Value::Object(map) = &mut value
        && map.contains_key("@context")
    {
        map.remove("@context");
    }

    let uri = format!("mcp://mdh/{}", uri_type.unwrap());
    Ok(MdhResource { uri, data: value })
}

/// MCP integrator with token storage and validation
pub struct LocalMcp {
    tools: RwLock<Vec<String>>,
    tokens: RwLock<HashMap<String, TokenInfo>>,
}

impl Default for LocalMcp {
    fn default() -> Self {
        Self {
            tools: RwLock::new(Vec::new()),
            tokens: RwLock::new(HashMap::new()),
        }
    }
}

impl LocalMcp {
    /// Create a new LocalMcp instance
    pub fn new() -> Self {
        Self::default()
    }

    /// Generate a new token for a client
    pub fn generate_token(
        &self,
        client_id: &str,
        scopes: Vec<String>,
        duration_hours: i64,
    ) -> Result<TokenInfo, AppError> {
        use rand::Rng;
        use rand::distributions::Alphanumeric;

        let token: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        let now = chrono::Utc::now();
        let expires_at = now + chrono::Duration::hours(duration_hours);

        let token_info = TokenInfo {
            token: token.clone(),
            created_at: now,
            expires_at,
            haptic_permission: false,
            client_id: client_id.to_string(),
            scopes,
        };

        let mut tokens = self
            .tokens
            .write()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("rwlock: {e}"))))?;
        tokens.insert(token.clone(), token_info.clone());

        tracing::info!(
            "Generated token for client {}: expires at {}",
            client_id,
            expires_at
        );

        Ok(token_info)
    }

    /// Clean up expired tokens
    pub fn cleanup_expired_tokens(&self) -> Result<usize, AppError> {
        let mut tokens = self
            .tokens
            .write()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("rwlock: {e}"))))?;

        let now = chrono::Utc::now();
        let initial_count = tokens.len();
        tokens.retain(|_, info| info.expires_at > now);
        let removed = initial_count - tokens.len();

        if removed > 0 {
            tracing::info!("Cleaned up {} expired tokens", removed);
        }

        Ok(removed)
    }

    /// Get token info if valid
    pub fn get_token_info(&self, token: &str) -> Result<Option<TokenInfo>, AppError> {
        let tokens = self
            .tokens
            .read()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("rwlock: {e}"))))?;

        if let Some(info) = tokens.get(token)
            && info.expires_at > chrono::Utc::now()
        {
            return Ok(Some(info.clone()));
        }

        Ok(None)
    }

    /// List all active tokens (for admin purposes)
    pub fn list_active_tokens(&self) -> Result<Vec<TokenInfo>, AppError> {
        let tokens = self
            .tokens
            .read()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("rwlock: {e}"))))?;

        let now = chrono::Utc::now();
        Ok(tokens
            .values()
            .filter(|info| info.expires_at > now)
            .cloned()
            .collect())
    }
}

#[async_trait::async_trait]
impl McpIntegrator for LocalMcp {
    async fn expose_tool(&self, tool: &str) -> Result<(), AppError> {
        let mut guard = self
            .tools
            .write()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("rwlock: {e}"))))?;
        guard.push(tool.to_string());
        tracing::info!("Exposed MCP tool: {}", tool);
        Ok(())
    }

    async fn authenticate_haptic(&self, token: &str) -> Result<bool, AppError> {
        // Validate token and check haptic permission
        if let Some(info) = self.get_token_info(token)? {
            if info.haptic_permission {
                tracing::info!(
                    "Haptic authentication successful for client: {}",
                    info.client_id
                );
                return Ok(true);
            }
            tracing::warn!(
                "Haptic permission not granted for client: {}",
                info.client_id
            );
            return Ok(false);
        }

        tracing::warn!("Invalid or expired token for haptic authentication");
        Ok(false)
    }

    async fn validate_token(&self, token: &str) -> Result<bool, AppError> {
        // Check if token exists and is not expired
        if let Some(info) = self.get_token_info(token)? {
            tracing::debug!("Token validated for client: {}", info.client_id);
            return Ok(true);
        }

        tracing::debug!("Token validation failed: invalid or expired");
        Ok(false)
    }

    async fn grant_haptic_permission(&self, token: &str) -> Result<(), AppError> {
        let mut tokens = self
            .tokens
            .write()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("rwlock: {e}"))))?;

        if let Some(info) = tokens.get_mut(token) {
            if info.expires_at > chrono::Utc::now() {
                info.haptic_permission = true;
                tracing::info!("Granted haptic permission for client: {}", info.client_id);
                return Ok(());
            }
            return Err(AppError::Io(std::io::Error::other("Token expired")));
        }

        Err(AppError::Io(std::io::Error::other("Token not found")))
    }
}

/// Global MCP instance
static MCP_INSTANCE: std::sync::OnceLock<LocalMcp> = std::sync::OnceLock::new();

/// Get the global MCP instance
pub fn get_mcp() -> &'static LocalMcp {
    MCP_INSTANCE.get_or_init(LocalMcp::new)
}
