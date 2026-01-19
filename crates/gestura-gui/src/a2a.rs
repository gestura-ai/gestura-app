//! Agent2Agent (A2A) Protocol Integration
//!
//! This module implements the A2A protocol for agent-to-agent communication.
//! A2A is an open protocol under the Linux Foundation that enables communication
//! and interoperability between opaque agentic applications.
//!
//! # Protocol Decision: A2A vs ACP
//!
//! **Recommendation: Adopt A2A** for the following reasons:
//!
//! 1. **Open Standard**: A2A is under Linux Foundation governance with Apache 2.0 license
//! 2. **JSON-RPC 2.0**: Uses the same protocol as our MCP server implementation
//! 3. **Streaming Support**: Native SSE support matches our streaming architecture
//! 4. **Agent Opacity**: Agents collaborate without exposing internal state/memory
//! 5. **Active Community**: 21k+ GitHub stars, SDKs for Python/Go/JS/Java/.NET
//! 6. **Complements MCP**: A2A for agent-to-agent, MCP for tool access
//!
//! ACP (IBM) was not selected because:
//! - More enterprise/IBM-focused, less community-driven
//! - Not as well-suited for open-source projects
//! - Less active development and adoption
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Gestura Agent System                      │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
//! │  │ A2A Server  │  │ A2A Client  │  │ Agent Card Registry │  │
//! │  │ (incoming)  │  │ (outgoing)  │  │ (discovery)         │  │
//! │  └──────┬──────┘  └──────┬──────┘  └──────────┬──────────┘  │
//! │         │                │                    │              │
//! │         └────────────────┼────────────────────┘              │
//! │                          │                                   │
//! │                    ┌─────▼─────┐                             │
//! │                    │Orchestrator│                            │
//! │                    └─────┬─────┘                             │
//! │                          │                                   │
//! │         ┌────────────────┼────────────────┐                  │
//! │         │                │                │                  │
//! │    ┌────▼────┐     ┌─────▼─────┐    ┌─────▼─────┐           │
//! │    │  NATS   │     │MCP Server │    │  Agents   │           │
//! │    │(events) │     │ (tools)   │    │(spawner)  │           │
//! │    └─────────┘     └───────────┘    └───────────┘           │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// A2A Agent Card (Discovery)
// ============================================================================

/// Agent Card for A2A discovery
/// Describes an agent's capabilities and connection information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    /// Unique agent identifier
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Agent's A2A endpoint URL
    pub url: String,
    /// Protocol version (e.g., "0.3.0")
    pub protocol_version: String,
    /// List of skills this agent provides
    pub skills: Vec<Skill>,
    /// Authentication requirements
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<AuthenticationInfo>,
    /// Supported input modes
    pub default_input_modes: Vec<String>,
    /// Supported output modes
    pub default_output_modes: Vec<String>,
}

/// Skill definition for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    /// Unique skill identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of what this skill does
    pub description: String,
    /// Input modes this skill accepts
    #[serde(default)]
    pub input_modes: Vec<String>,
    /// Output modes this skill produces
    #[serde(default)]
    pub output_modes: Vec<String>,
    /// Example prompts for this skill
    #[serde(default)]
    pub examples: Vec<String>,
}

/// Authentication information for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationInfo {
    /// Authentication schemes supported (e.g., "bearer", "apiKey", "oauth2")
    pub schemes: Vec<String>,
    /// OAuth2 configuration if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth2: Option<OAuth2Config>,
}

/// OAuth2 configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuth2Config {
    /// Authorization endpoint
    pub authorization_url: String,
    /// Token endpoint
    pub token_url: String,
    /// Required scopes
    #[serde(default)]
    pub scopes: Vec<String>,
}

// ============================================================================
// A2A Agent Profile (Authentication & Identity)
// ============================================================================

/// Agent profile for authentication and identity propagation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    /// Unique agent identifier
    pub agent_id: String,
    /// Agent name
    pub name: String,
    /// Agent version
    pub version: String,
    /// Capabilities this agent has
    pub capabilities: Vec<String>,
    /// Authentication token for this agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    /// Token expiration time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Metadata for custom properties
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl AgentProfile {
    /// Create a new agent profile
    pub fn new(agent_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            name: name.into(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities: vec![],
            auth_token: None,
            token_expires_at: None,
            metadata: HashMap::new(),
        }
    }

    /// Add a capability
    pub fn with_capability(mut self, capability: impl Into<String>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    /// Set authentication token
    pub fn with_auth_token(
        mut self,
        token: impl Into<String>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Self {
        self.auth_token = Some(token.into());
        self.token_expires_at = expires_at;
        self
    }

    /// Check if the token is valid (not expired)
    pub fn is_token_valid(&self) -> bool {
        match (&self.auth_token, &self.token_expires_at) {
            (Some(_), Some(expires)) => chrono::Utc::now() < *expires,
            (Some(_), None) => true, // No expiration means always valid
            (None, _) => false,
        }
    }

    /// Generate a new token for this agent
    pub fn generate_token(&mut self, validity_hours: i64) {
        use rand::Rng;
        use rand::distributions::Alphanumeric;
        let token: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(64)
            .map(char::from)
            .collect();
        self.auth_token = Some(token);
        self.token_expires_at = Some(chrono::Utc::now() + chrono::Duration::hours(validity_hours));
    }
}

/// Profile store for managing agent profiles
#[derive(Debug, Default)]
pub struct ProfileStore {
    profiles: std::sync::RwLock<HashMap<String, AgentProfile>>,
}

impl ProfileStore {
    /// Create a new profile store
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a profile
    pub fn store(&self, profile: AgentProfile) {
        let mut profiles = self.profiles.write().unwrap();
        profiles.insert(profile.agent_id.clone(), profile);
    }

    /// Get a profile by agent ID
    pub fn get(&self, agent_id: &str) -> Option<AgentProfile> {
        let profiles = self.profiles.read().unwrap();
        profiles.get(agent_id).cloned()
    }

    /// Validate a token and return the associated profile
    pub fn validate_token(&self, token: &str) -> Option<AgentProfile> {
        let profiles = self.profiles.read().unwrap();
        profiles
            .values()
            .find(|p| p.auth_token.as_deref() == Some(token) && p.is_token_valid())
            .cloned()
    }

    /// List all profiles
    pub fn list(&self) -> Vec<AgentProfile> {
        let profiles = self.profiles.read().unwrap();
        profiles.values().cloned().collect()
    }

    /// Remove a profile
    pub fn remove(&self, agent_id: &str) -> Option<AgentProfile> {
        let mut profiles = self.profiles.write().unwrap();
        profiles.remove(agent_id)
    }
}

// ============================================================================
// A2A Task Management
// ============================================================================

/// Task status in A2A protocol
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// A2A Task representation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ATask {
    /// Unique task identifier
    pub id: String,
    /// Current status
    pub status: TaskStatus,
    /// Task messages (conversation history)
    pub messages: Vec<A2AMessage>,
    /// Output artifacts
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// Metadata
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Message in A2A conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2AMessage {
    /// Role: "user" or "agent"
    pub role: String,
    /// Message parts (text, file, data)
    pub parts: Vec<MessagePart>,
}

/// Part of a message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MessagePart {
    /// Text content
    Text { text: String },
    /// File reference
    File {
        uri: String,
        mime_type: Option<String>,
    },
    /// Structured data
    Data { data: serde_json::Value },
}

/// Output artifact from a task
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    /// Artifact name
    pub name: String,
    /// Artifact parts
    pub parts: Vec<MessagePart>,
}

// ============================================================================
// A2A JSON-RPC Messages
// ============================================================================

/// JSON-RPC 2.0 request for A2A
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ARequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    pub id: serde_json::Value,
}

/// JSON-RPC 2.0 response for A2A
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<A2AError>,
    pub id: serde_json::Value,
}

/// JSON-RPC error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ============================================================================
// A2A Agent Card Registry
// ============================================================================

/// Registry for discovered agent cards
#[derive(Debug, Default)]
pub struct AgentCardRegistry {
    cards: std::sync::RwLock<HashMap<String, AgentCard>>,
}

impl AgentCardRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an agent card
    pub fn register(&self, card: AgentCard) {
        let mut cards = self.cards.write().unwrap();
        cards.insert(card.name.clone(), card);
    }

    /// Get an agent card by name
    pub fn get(&self, name: &str) -> Option<AgentCard> {
        let cards = self.cards.read().unwrap();
        cards.get(name).cloned()
    }

    /// List all registered agents
    pub fn list(&self) -> Vec<AgentCard> {
        let cards = self.cards.read().unwrap();
        cards.values().cloned().collect()
    }

    /// Remove an agent card
    pub fn remove(&self, name: &str) -> Option<AgentCard> {
        let mut cards = self.cards.write().unwrap();
        cards.remove(name)
    }
}

// ============================================================================
// A2A Server (Incoming Requests)
// ============================================================================

/// A2A Server for handling incoming agent-to-agent requests
pub struct A2AServer {
    /// This agent's card
    pub agent_card: AgentCard,
    /// Registry of known agents
    pub registry: AgentCardRegistry,
    /// Profile store for authentication
    pub profile_store: ProfileStore,
    /// Active tasks
    tasks: std::sync::RwLock<HashMap<String, A2ATask>>,
    /// Task to caller profile mapping
    task_profiles: std::sync::RwLock<HashMap<String, AgentProfile>>,
}

impl A2AServer {
    /// Create a new A2A server
    pub fn new(agent_card: AgentCard) -> Self {
        Self {
            agent_card,
            registry: AgentCardRegistry::new(),
            profile_store: ProfileStore::new(),
            tasks: std::sync::RwLock::new(HashMap::new()),
            task_profiles: std::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Handle an incoming JSON-RPC request with optional authentication
    pub fn handle_request(&self, request: A2ARequest) -> A2AResponse {
        self.handle_request_with_auth(request, None)
    }

    /// Handle an incoming JSON-RPC request with authentication token
    pub fn handle_request_with_auth(
        &self,
        request: A2ARequest,
        auth_token: Option<&str>,
    ) -> A2AResponse {
        // Validate token if provided
        let caller_profile = auth_token.and_then(|token| self.profile_store.validate_token(token));

        // Check if authentication is required for this method
        let requires_auth = matches!(request.method.as_str(), "task/create" | "task/cancel");
        if requires_auth && self.agent_card.authentication.is_some() && caller_profile.is_none() {
            return A2AResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(A2AError {
                    code: -32000,
                    message: "Authentication required".to_string(),
                    data: None,
                }),
                id: request.id,
            };
        }

        let result = match request.method.as_str() {
            "agent/discover" => self.handle_discover(),
            "task/create" => self.handle_task_create(&request.params, caller_profile.as_ref()),
            "task/status" => self.handle_task_status(&request.params),
            "task/cancel" => self.handle_task_cancel(&request.params),
            "profile/register" => self.handle_profile_register(&request.params),
            "profile/validate" => self.handle_profile_validate(&request.params),
            _ => Err(A2AError {
                code: -32601,
                message: format!("Method not found: {}", request.method),
                data: None,
            }),
        };

        match result {
            Ok(value) => A2AResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(value),
                error: None,
                id: request.id,
            },
            Err(error) => A2AResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(error),
                id: request.id,
            },
        }
    }

    /// Get the caller profile for a task
    pub fn get_task_caller(&self, task_id: &str) -> Option<AgentProfile> {
        let profiles = self.task_profiles.read().unwrap();
        profiles.get(task_id).cloned()
    }

    /// Handle agent/discover request
    fn handle_discover(&self) -> Result<serde_json::Value, A2AError> {
        serde_json::to_value(&self.agent_card).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {}", e),
            data: None,
        })
    }

    /// Handle task/create request
    fn handle_task_create(
        &self,
        params: &serde_json::Value,
        caller: Option<&AgentProfile>,
    ) -> Result<serde_json::Value, A2AError> {
        let message: A2AMessage = serde_json::from_value(params.clone()).map_err(|e| A2AError {
            code: -32602,
            message: format!("Invalid params: {}", e),
            data: None,
        })?;

        let task_id = uuid::Uuid::new_v4().to_string();
        let mut metadata = HashMap::new();

        // Store caller profile info in task metadata
        if let Some(profile) = caller {
            metadata.insert(
                "caller_agent_id".to_string(),
                serde_json::json!(profile.agent_id),
            );
            metadata.insert("caller_name".to_string(), serde_json::json!(profile.name));

            // Store full profile for later retrieval
            let mut profiles = self.task_profiles.write().unwrap();
            profiles.insert(task_id.clone(), profile.clone());
        }

        let task = A2ATask {
            id: task_id.clone(),
            status: TaskStatus::Pending,
            messages: vec![message],
            artifacts: vec![],
            metadata,
        };

        {
            let mut tasks = self.tasks.write().unwrap();
            tasks.insert(task_id.clone(), task.clone());
        }

        tracing::info!(task_id = %task_id, caller = ?caller.map(|p| &p.agent_id), "A2A task created");
        serde_json::to_value(&task).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {}", e),
            data: None,
        })
    }

    /// Handle profile/register request
    fn handle_profile_register(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, A2AError> {
        let profile: AgentProfile =
            serde_json::from_value(params.clone()).map_err(|e| A2AError {
                code: -32602,
                message: format!("Invalid profile: {}", e),
                data: None,
            })?;

        let agent_id = profile.agent_id.clone();
        self.profile_store.store(profile);

        tracing::info!(agent_id = %agent_id, "A2A profile registered");
        Ok(serde_json::json!({"success": true, "agentId": agent_id}))
    }

    /// Handle profile/validate request
    fn handle_profile_validate(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, A2AError> {
        let token = params
            .get("token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| A2AError {
                code: -32602,
                message: "Missing token parameter".to_string(),
                data: None,
            })?;

        match self.profile_store.validate_token(token) {
            Some(profile) => {
                tracing::debug!(agent_id = %profile.agent_id, "Token validated");
                Ok(serde_json::json!({
                    "valid": true,
                    "agentId": profile.agent_id,
                    "name": profile.name,
                    "capabilities": profile.capabilities
                }))
            }
            None => Ok(serde_json::json!({"valid": false})),
        }
    }

    /// Handle task/status request
    fn handle_task_status(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, A2AError> {
        let task_id = params
            .get("taskId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| A2AError {
                code: -32602,
                message: "Missing taskId parameter".to_string(),
                data: None,
            })?;

        let tasks = self.tasks.read().unwrap();
        let task = tasks.get(task_id).ok_or_else(|| A2AError {
            code: -32001,
            message: format!("Task not found: {}", task_id),
            data: None,
        })?;

        serde_json::to_value(task).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {}", e),
            data: None,
        })
    }

    /// Handle task/cancel request
    fn handle_task_cancel(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, A2AError> {
        let task_id = params
            .get("taskId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| A2AError {
                code: -32602,
                message: "Missing taskId parameter".to_string(),
                data: None,
            })?;

        let mut tasks = self.tasks.write().unwrap();
        let task = tasks.get_mut(task_id).ok_or_else(|| A2AError {
            code: -32001,
            message: format!("Task not found: {}", task_id),
            data: None,
        })?;

        task.status = TaskStatus::Cancelled;
        tracing::info!(task_id = %task_id, "A2A task cancelled");

        serde_json::to_value(task).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {}", e),
            data: None,
        })
    }

    /// Update task status (for internal use)
    pub fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<(), String> {
        let mut tasks = self.tasks.write().unwrap();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Task not found: {}", task_id))?;
        task.status = status;
        Ok(())
    }

    /// Add artifact to task (for internal use)
    pub fn add_artifact(&self, task_id: &str, artifact: Artifact) -> Result<(), String> {
        let mut tasks = self.tasks.write().unwrap();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Task not found: {}", task_id))?;
        task.artifacts.push(artifact);
        Ok(())
    }
}

// ============================================================================
// A2A Client (Outgoing Requests)
// ============================================================================

/// A2A Client for making requests to external agents
#[derive(Debug, Clone)]
pub struct A2AClient {
    /// HTTP client
    client: reqwest::Client,
    /// Authentication token (if any)
    auth_token: Option<String>,
    /// Agent profile for this client
    profile: Option<AgentProfile>,
}

impl A2AClient {
    /// Create a new A2A client
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            auth_token: None,
            profile: None,
        }
    }

    /// Create a client with authentication token
    pub fn with_auth(token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            auth_token: Some(token),
            profile: None,
        }
    }

    /// Create a client with a full agent profile
    pub fn with_profile(profile: AgentProfile) -> Self {
        let auth_token = profile.auth_token.clone();
        Self {
            client: reqwest::Client::new(),
            auth_token,
            profile: Some(profile),
        }
    }

    /// Get the current profile
    pub fn profile(&self) -> Option<&AgentProfile> {
        self.profile.as_ref()
    }

    /// Set the authentication token
    pub fn set_auth_token(&mut self, token: String) {
        self.auth_token = Some(token);
    }

    /// Register this client's profile with a remote agent
    pub async fn register_profile(
        &self,
        agent_url: &str,
        profile: &AgentProfile,
    ) -> Result<(), String> {
        let request = A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "profile/register".to_string(),
            params: serde_json::to_value(profile).map_err(|e| e.to_string())?,
            id: serde_json::json!(1),
        };

        let response = self.send_request(agent_url, &request).await?;

        if response.error.is_some() {
            return Err(response.error.unwrap().message);
        }

        Ok(())
    }

    /// Validate a token with a remote agent
    pub async fn validate_token(
        &self,
        agent_url: &str,
        token: &str,
    ) -> Result<Option<AgentProfile>, String> {
        let request = A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "profile/validate".to_string(),
            params: serde_json::json!({"token": token}),
            id: serde_json::json!(1),
        };

        let response = self.send_request(agent_url, &request).await?;

        let result = response.result.ok_or_else(|| {
            response
                .error
                .map(|e| e.message)
                .unwrap_or_else(|| "Unknown error".to_string())
        })?;

        let valid = result
            .get("valid")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !valid {
            return Ok(None);
        }

        // Reconstruct a minimal profile from the response
        let agent_id = result
            .get("agentId")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let name = result
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let capabilities: Vec<String> = result
            .get("capabilities")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let mut profile = AgentProfile::new(agent_id, name);
        for cap in capabilities {
            profile = profile.with_capability(cap);
        }

        Ok(Some(profile))
    }

    /// Discover an agent's capabilities
    pub async fn discover(&self, agent_url: &str) -> Result<AgentCard, String> {
        let request = A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "agent/discover".to_string(),
            params: serde_json::Value::Null,
            id: serde_json::json!(1),
        };

        let response = self.send_request(agent_url, &request).await?;

        response
            .result
            .ok_or_else(|| {
                response
                    .error
                    .map(|e| e.message)
                    .unwrap_or_else(|| "Unknown error".to_string())
            })
            .and_then(|v| serde_json::from_value(v).map_err(|e| format!("Parse error: {}", e)))
    }

    /// Create a task on a remote agent
    pub async fn create_task(
        &self,
        agent_url: &str,
        message: A2AMessage,
    ) -> Result<A2ATask, String> {
        let request = A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/create".to_string(),
            params: serde_json::to_value(&message).map_err(|e| e.to_string())?,
            id: serde_json::json!(1),
        };

        let response = self.send_request(agent_url, &request).await?;

        response
            .result
            .ok_or_else(|| {
                response
                    .error
                    .map(|e| e.message)
                    .unwrap_or_else(|| "Unknown error".to_string())
            })
            .and_then(|v| serde_json::from_value(v).map_err(|e| format!("Parse error: {}", e)))
    }

    /// Get task status from a remote agent
    pub async fn get_task_status(&self, agent_url: &str, task_id: &str) -> Result<A2ATask, String> {
        let request = A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/status".to_string(),
            params: serde_json::json!({"taskId": task_id}),
            id: serde_json::json!(1),
        };

        let response = self.send_request(agent_url, &request).await?;

        response
            .result
            .ok_or_else(|| {
                response
                    .error
                    .map(|e| e.message)
                    .unwrap_or_else(|| "Unknown error".to_string())
            })
            .and_then(|v| serde_json::from_value(v).map_err(|e| format!("Parse error: {}", e)))
    }

    /// Cancel a task on a remote agent
    pub async fn cancel_task(&self, agent_url: &str, task_id: &str) -> Result<A2ATask, String> {
        let request = A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/cancel".to_string(),
            params: serde_json::json!({"taskId": task_id}),
            id: serde_json::json!(1),
        };

        let response = self.send_request(agent_url, &request).await?;

        response
            .result
            .ok_or_else(|| {
                response
                    .error
                    .map(|e| e.message)
                    .unwrap_or_else(|| "Unknown error".to_string())
            })
            .and_then(|v| serde_json::from_value(v).map_err(|e| format!("Parse error: {}", e)))
    }

    /// Send a JSON-RPC request to an agent
    async fn send_request(
        &self,
        agent_url: &str,
        request: &A2ARequest,
    ) -> Result<A2AResponse, String> {
        let mut req = self.client.post(agent_url).json(request);

        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let response = req.send().await.map_err(|e| format!("HTTP error: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()));
        }

        response
            .json::<A2AResponse>()
            .await
            .map_err(|e| format!("Parse error: {}", e))
    }
}

impl Default for A2AClient {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Create a default agent card for Gestura
pub fn create_gestura_agent_card(base_url: &str) -> AgentCard {
    AgentCard {
        name: "gestura".to_string(),
        description: "Voice-powered agentic workflows with MCP integration".to_string(),
        url: format!("{}/a2a", base_url),
        protocol_version: "0.3.0".to_string(),
        skills: vec![
            Skill {
                id: "voice-command".to_string(),
                name: "Voice Command".to_string(),
                description: "Process voice commands and execute workflows".to_string(),
                input_modes: vec!["text".to_string(), "audio".to_string()],
                output_modes: vec!["text".to_string()],
                examples: vec!["Run the build".to_string(), "Open the settings".to_string()],
            },
            Skill {
                id: "tool-execution".to_string(),
                name: "Tool Execution".to_string(),
                description: "Execute MCP tools and return results".to_string(),
                input_modes: vec!["text".to_string()],
                output_modes: vec!["text".to_string(), "data".to_string()],
                examples: vec![
                    "Send haptic feedback".to_string(),
                    "Check ring battery".to_string(),
                ],
            },
        ],
        authentication: Some(AuthenticationInfo {
            schemes: vec!["bearer".to_string()],
            oauth2: None,
        }),
        default_input_modes: vec!["text".to_string()],
        default_output_modes: vec!["text".to_string()],
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_card_serialization() {
        let card = create_gestura_agent_card("http://localhost:8080");
        let json = serde_json::to_string(&card).unwrap();
        assert!(json.contains("gestura"));
        assert!(json.contains("voice-command"));
    }

    #[test]
    fn test_a2a_server_discover() {
        let card = create_gestura_agent_card("http://localhost:8080");
        let server = A2AServer::new(card.clone());

        let request = A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "agent/discover".to_string(),
            params: serde_json::Value::Null,
            id: serde_json::json!(1),
        };

        let response = server.handle_request(request);
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn test_a2a_server_task_lifecycle() {
        // Create a card without authentication for testing basic lifecycle
        let card = AgentCard {
            name: "test-agent".to_string(),
            description: "Test agent".to_string(),
            url: "http://localhost:8080/a2a".to_string(),
            protocol_version: "0.3.0".to_string(),
            skills: vec![],
            authentication: None, // No auth required for this test
            default_input_modes: vec!["text".to_string()],
            default_output_modes: vec!["text".to_string()],
        };
        let server = A2AServer::new(card);

        // Create task
        let create_request = A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/create".to_string(),
            params: serde_json::json!({
                "role": "user",
                "parts": [{"type": "text", "text": "Hello agent"}]
            }),
            id: serde_json::json!(1),
        };

        let create_response = server.handle_request(create_request);
        assert!(create_response.result.is_some());

        let task: A2ATask = serde_json::from_value(create_response.result.unwrap()).unwrap();
        assert_eq!(task.status, TaskStatus::Pending);

        // Get status
        let status_request = A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/status".to_string(),
            params: serde_json::json!({"taskId": task.id}),
            id: serde_json::json!(2),
        };

        let status_response = server.handle_request(status_request);
        assert!(status_response.result.is_some());

        // Cancel task
        let cancel_request = A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/cancel".to_string(),
            params: serde_json::json!({"taskId": task.id}),
            id: serde_json::json!(3),
        };

        let cancel_response = server.handle_request(cancel_request);
        assert!(cancel_response.result.is_some());

        let cancelled_task: A2ATask =
            serde_json::from_value(cancel_response.result.unwrap()).unwrap();
        assert_eq!(cancelled_task.status, TaskStatus::Cancelled);
    }

    #[test]
    fn test_agent_card_registry() {
        let registry = AgentCardRegistry::new();
        let card = create_gestura_agent_card("http://localhost:8080");

        registry.register(card.clone());
        assert_eq!(registry.list().len(), 1);

        let retrieved = registry.get("gestura").unwrap();
        assert_eq!(retrieved.name, "gestura");

        registry.remove("gestura");
        assert!(registry.get("gestura").is_none());
    }

    #[test]
    fn test_message_part_serialization() {
        let text_part = MessagePart::Text {
            text: "Hello".to_string(),
        };
        let json = serde_json::to_string(&text_part).unwrap();
        assert!(json.contains("text"));
        assert!(json.contains("Hello"));

        let file_part = MessagePart::File {
            uri: "file:///test.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
        };
        let json = serde_json::to_string(&file_part).unwrap();
        assert!(json.contains("file"));
        assert!(json.contains("file:///test.txt"));
    }
}
