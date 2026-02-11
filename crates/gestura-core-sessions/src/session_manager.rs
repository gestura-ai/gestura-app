//! Session management for authentication and authorization
//! Handles user sessions, tokens, and access control

use gestura_core_foundation::AppError;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;

/// User session information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserSession {
    pub session_id: String,
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,
    pub created_at: SystemTime,
    pub last_accessed: SystemTime,
    pub expires_at: SystemTime,
    pub is_active: bool,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Authentication token
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthToken {
    pub token: String,
    pub token_type: TokenType,
    pub session_id: String,
    pub expires_at: SystemTime,
    pub scopes: Vec<String>,
}

/// Token types
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TokenType {
    Bearer,
    ApiKey,
    Refresh,
    Temporary,
}

/// Session manager
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<String, UserSession>>>,
    tokens: Arc<RwLock<HashMap<String, AuthToken>>>,
    default_session_duration: Duration,
    max_sessions_per_user: usize,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(default_session_duration: Duration, max_sessions_per_user: usize) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            tokens: Arc::new(RwLock::new(HashMap::new())),
            default_session_duration,
            max_sessions_per_user,
        }
    }

    /// Create a new user session
    pub async fn create_session(
        &self,
        user_id: String,
        username: String,
        email: Option<String>,
        roles: Vec<String>,
    ) -> Result<UserSession, AppError> {
        let session_id = self.generate_session_id();
        let now = SystemTime::now();
        let expires_at = now + self.default_session_duration;

        // Check if user has too many active sessions
        self.cleanup_user_sessions(&user_id).await?;

        let session = UserSession {
            session_id: session_id.clone(),
            user_id: user_id.clone(),
            username,
            email,
            roles,
            permissions: Vec::new(), // Will be populated based on roles
            created_at: now,
            last_accessed: now,
            expires_at,
            is_active: true,
            metadata: HashMap::new(),
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.clone(), session.clone());

        tracing::info!("Created session {} for user {}", session_id, user_id);
        Ok(session)
    }

    /// Get session by ID
    pub async fn get_session(&self, session_id: &str) -> Option<UserSession> {
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.get_mut(session_id) {
            // Check if session is expired
            if SystemTime::now() > session.expires_at || !session.is_active {
                sessions.remove(session_id);
                return None;
            }

            // Update last accessed time
            session.last_accessed = SystemTime::now();
            Some(session.clone())
        } else {
            None
        }
    }

    /// Validate session and return user info
    pub async fn validate_session(&self, session_id: &str) -> Result<UserSession, AppError> {
        self.get_session(session_id).await.ok_or_else(|| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Invalid or expired session",
            ))
        })
    }

    /// Create authentication token
    pub async fn create_token(
        &self,
        session_id: &str,
        token_type: TokenType,
        scopes: Vec<String>,
        duration: Option<Duration>,
    ) -> Result<AuthToken, AppError> {
        // Validate session exists
        let session = self.validate_session(session_id).await?;

        let token = self.generate_token();
        let expires_at = SystemTime::now() + duration.unwrap_or(Duration::from_secs(3600));

        let auth_token = AuthToken {
            token: token.clone(),
            token_type,
            session_id: session.session_id,
            expires_at,
            scopes,
        };

        let mut tokens = self.tokens.write().await;
        tokens.insert(token.clone(), auth_token.clone());

        tracing::info!("Created token for session {}", session_id);
        Ok(auth_token)
    }

    /// Validate authentication token
    pub async fn validate_token(&self, token: &str) -> Result<(UserSession, AuthToken), AppError> {
        let tokens = self.tokens.read().await;

        if let Some(auth_token) = tokens.get(token) {
            // Check if token is expired
            if SystemTime::now() > auth_token.expires_at {
                drop(tokens);
                self.revoke_token(token).await?;
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Token expired",
                )));
            }

            // Get associated session
            let session = self.validate_session(&auth_token.session_id).await?;
            Ok((session, auth_token.clone()))
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Invalid token",
            )))
        }
    }

    /// Revoke authentication token
    pub async fn revoke_token(&self, token: &str) -> Result<(), AppError> {
        let mut tokens = self.tokens.write().await;
        if tokens.remove(token).is_some() {
            tracing::info!("Revoked token: {}", &token[..8]);
            Ok(())
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Token not found",
            )))
        }
    }

    /// End user session
    pub async fn end_session(&self, session_id: &str) -> Result<(), AppError> {
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.get_mut(session_id) {
            session.is_active = false;

            // Revoke all tokens for this session
            let mut tokens = self.tokens.write().await;
            let tokens_to_remove: Vec<String> = tokens
                .iter()
                .filter(|(_, token)| token.session_id == session_id)
                .map(|(token_str, _)| token_str.clone())
                .collect();

            for token in tokens_to_remove {
                tokens.remove(&token);
            }

            tracing::info!("Ended session: {}", session_id);
            Ok(())
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Session not found",
            )))
        }
    }

    /// Get all active sessions for a user
    pub async fn get_user_sessions(&self, user_id: &str) -> Vec<UserSession> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|session| session.user_id == user_id && session.is_active)
            .cloned()
            .collect()
    }

    /// Cleanup old sessions for a user (keep only the most recent ones)
    async fn cleanup_user_sessions(&self, user_id: &str) -> Result<(), AppError> {
        let mut sessions = self.sessions.write().await;
        let mut user_sessions: Vec<_> = sessions
            .iter()
            .filter(|(_, session)| session.user_id == user_id && session.is_active)
            .map(|(id, session)| (id.clone(), session.last_accessed))
            .collect();

        if user_sessions.len() >= self.max_sessions_per_user {
            // Sort by last accessed time (oldest first)
            user_sessions.sort_by_key(|(_, last_accessed)| *last_accessed);

            // Remove oldest sessions
            let sessions_to_remove = user_sessions.len() - self.max_sessions_per_user + 1;
            for (session_id, _) in user_sessions.iter().take(sessions_to_remove) {
                sessions.remove(session_id);
                tracing::info!("Removed old session {} for user {}", session_id, user_id);
            }
        }

        Ok(())
    }

    /// Generate unique session ID
    fn generate_session_id(&self) -> String {
        format!("sess_{}", uuid::Uuid::new_v4().to_string().replace('-', ""))
    }

    /// Generate authentication token
    fn generate_token(&self) -> String {
        format!("tok_{}", uuid::Uuid::new_v4().to_string().replace('-', ""))
    }

    /// Get session statistics
    pub async fn get_stats(&self) -> serde_json::Value {
        let sessions = self.sessions.read().await;
        let tokens = self.tokens.read().await;

        let active_sessions = sessions.values().filter(|s| s.is_active).count();
        let total_sessions = sessions.len();
        let total_tokens = tokens.len();

        serde_json::json!({
            "active_sessions": active_sessions,
            "total_sessions": total_sessions,
            "total_tokens": total_tokens,
            "default_session_duration_hours": self.default_session_duration.as_secs() / 3600,
            "max_sessions_per_user": self.max_sessions_per_user
        })
    }
}

/// Global session manager instance
static SESSION_MANAGER: tokio::sync::OnceCell<SessionManager> = tokio::sync::OnceCell::const_new();

/// Get the global session manager
pub async fn get_session_manager() -> &'static SessionManager {
    SESSION_MANAGER
        .get_or_init(|| async { SessionManager::new(Duration::from_secs(24 * 3600), 5) })
        .await
}
