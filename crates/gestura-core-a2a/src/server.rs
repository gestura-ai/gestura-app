//! A2A server + authentication helpers.
//!
//! This module contains the in-memory server-side implementation of the A2A JSON-RPC
//! protocol along with basic agent identity/authentication primitives.
//!
//! ## Design
//! - Transport-agnostic: HTTP/SSE adapters live in shells (GUI/CLI), but the protocol
//!   routing and request/response handling lives in `gestura-core`.
//! - Lightweight auth: bearer token validation is performed against an in-memory
//!   `ProfileStore` (suitable for local adapters and tests).

use super::{
    A2AError, A2AMessage, A2ARequest, A2AResponse, A2ATask, AgentCard, Artifact, CreateTaskRequest,
    TaskAuditEvent, TaskProvenance, TaskStatus,
};
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use rand::distributions::Alphanumeric;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Agent profile for authentication and identity propagation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    /// Unique agent identifier.
    pub agent_id: String,
    /// Human-readable agent name.
    pub name: String,
    /// Agent version.
    pub version: String,
    /// Capabilities this agent advertises.
    pub capabilities: Vec<String>,
    /// Authentication token for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    /// Token expiration time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<DateTime<Utc>>,
    /// Metadata for custom properties.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl AgentProfile {
    /// Create a new agent profile.
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

    /// Add a capability string.
    pub fn with_capability(mut self, capability: impl Into<String>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    /// Set the authentication token (and optional expiry) for this profile.
    pub fn with_auth_token(
        mut self,
        token: impl Into<String>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Self {
        self.auth_token = Some(token.into());
        self.token_expires_at = expires_at;
        self
    }

    /// Return whether the currently attached token exists and is not expired.
    pub fn is_token_valid(&self) -> bool {
        match (&self.auth_token, &self.token_expires_at) {
            (Some(_), Some(expires)) => Utc::now() < *expires,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// Generate a new bearer token for this agent and set `token_expires_at`.
    ///
    /// The generated token is URL-safe alphanumeric and intended for use as a
    /// `Authorization: Bearer <token>` credential.
    pub fn generate_token(&mut self, validity_hours: i64) {
        let token: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(64)
            .map(char::from)
            .collect();
        self.auth_token = Some(token);
        self.token_expires_at = Some(Utc::now() + Duration::hours(validity_hours));
    }
}

/// Return whether a token is well-formed for Gestura's current bearer-token scheme.
///
/// This is **not** a full validation step (it does not check expiry or revocation),
/// but it can be used by shells to provide a quick, offline sanity check.
pub fn is_token_well_formed(token: &str) -> bool {
    token.len() >= 32 && token.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Profile store for managing agent profiles.
#[derive(Debug, Default)]
pub struct ProfileStore {
    profiles: std::sync::RwLock<HashMap<String, AgentProfile>>,
}

impl ProfileStore {
    /// Create a new in-memory profile store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store/overwrite a profile keyed by `agent_id`.
    pub fn store(&self, profile: AgentProfile) {
        let mut profiles = self.profiles.write().unwrap();
        profiles.insert(profile.agent_id.clone(), profile);
    }

    /// Retrieve a profile by `agent_id`.
    pub fn get(&self, agent_id: &str) -> Option<AgentProfile> {
        let profiles = self.profiles.read().unwrap();
        profiles.get(agent_id).cloned()
    }

    /// Validate a bearer token and return the associated profile (if present and valid).
    pub fn validate_token(&self, token: &str) -> Option<AgentProfile> {
        let profiles = self.profiles.read().unwrap();
        profiles
            .values()
            .find(|p| p.auth_token.as_deref() == Some(token) && p.is_token_valid())
            .cloned()
    }

    /// List all stored profiles.
    pub fn list(&self) -> Vec<AgentProfile> {
        let profiles = self.profiles.read().unwrap();
        profiles.values().cloned().collect()
    }

    /// Remove a profile by `agent_id`.
    pub fn remove(&self, agent_id: &str) -> Option<AgentProfile> {
        let mut profiles = self.profiles.write().unwrap();
        profiles.remove(agent_id)
    }
}

/// In-memory registry for discovered/known agent cards.
#[derive(Debug, Default)]
pub struct AgentCardRegistry {
    cards: std::sync::RwLock<HashMap<String, AgentCard>>,
}

impl AgentCardRegistry {
    /// Create a new registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an agent card.
    pub fn register(&self, card: AgentCard) {
        let mut cards = self.cards.write().unwrap();
        cards.insert(card.name.clone(), card);
    }

    /// Get an agent card by name.
    pub fn get(&self, name: &str) -> Option<AgentCard> {
        let cards = self.cards.read().unwrap();
        cards.get(name).cloned()
    }

    /// List all registered agents.
    pub fn list(&self) -> Vec<AgentCard> {
        let cards = self.cards.read().unwrap();
        cards.values().cloned().collect()
    }

    /// Remove an agent card.
    pub fn remove(&self, name: &str) -> Option<AgentCard> {
        let mut cards = self.cards.write().unwrap();
        cards.remove(name)
    }
}

/// A2A server for handling incoming JSON-RPC requests.
///
/// This is a **protocol** server (request router + in-memory task store), not a
/// network server. A shell crate is responsible for exposing it over HTTP/SSE.
pub struct A2AServer {
    /// This agent's card.
    pub agent_card: AgentCard,
    /// Registry of known agents.
    pub registry: AgentCardRegistry,
    /// Profile store for bearer token authentication.
    pub profile_store: ProfileStore,
    /// Active tasks.
    tasks: std::sync::RwLock<HashMap<String, A2ATask>>,
    /// Task-to-caller-profile mapping.
    task_profiles: std::sync::RwLock<HashMap<String, AgentProfile>>,
}

impl A2AServer {
    /// Create a new A2A server for the given agent card.
    pub fn new(agent_card: AgentCard) -> Self {
        Self {
            agent_card,
            registry: AgentCardRegistry::new(),
            profile_store: ProfileStore::new(),
            tasks: std::sync::RwLock::new(HashMap::new()),
            task_profiles: std::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Handle an incoming JSON-RPC request.
    pub fn handle_request(&self, request: A2ARequest) -> A2AResponse {
        self.handle_request_with_auth(request, None)
    }

    /// Handle an incoming JSON-RPC request with an optional bearer token.
    pub fn handle_request_with_auth(
        &self,
        request: A2ARequest,
        auth_token: Option<&str>,
    ) -> A2AResponse {
        // Validate token if provided.
        let caller_profile = auth_token.and_then(|token| self.profile_store.validate_token(token));

        // Auth is currently required for task creation/cancellation if the agent
        // card advertises an auth scheme.
        let requires_auth = matches!(
            request.method.as_str(),
            "task/create" | "task/cancel" | "task/retry"
        );
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
            "task/retry" => self.handle_task_retry(&request.params),
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

    /// Get the caller profile for a task (if known).
    pub fn get_task_caller(&self, task_id: &str) -> Option<AgentProfile> {
        let profiles = self.task_profiles.read().unwrap();
        profiles.get(task_id).cloned()
    }

    fn handle_discover(&self) -> Result<serde_json::Value, A2AError> {
        serde_json::to_value(&self.agent_card).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {e}"),
            data: None,
        })
    }

    fn handle_task_create(
        &self,
        params: &serde_json::Value,
        caller: Option<&AgentProfile>,
    ) -> Result<serde_json::Value, A2AError> {
        let request = serde_json::from_value::<CreateTaskRequest>(params.clone())
            .or_else(|_| {
                serde_json::from_value::<A2AMessage>(params.clone())
                    .map(CreateTaskRequest::from_message)
            })
            .map_err(|e| A2AError {
                code: -32602,
                message: format!("Invalid params: {e}"),
                data: None,
            })?;

        let task_id = uuid::Uuid::new_v4().to_string();
        let mut metadata = request.metadata;
        let now = Utc::now();

        let provenance = if let Some(profile) = caller {
            metadata.insert(
                "caller_agent_id".to_string(),
                serde_json::json!(profile.agent_id),
            );
            metadata.insert("caller_name".to_string(), serde_json::json!(profile.name));

            let mut profiles = self.task_profiles.write().unwrap();
            profiles.insert(task_id.clone(), profile.clone());

            Some(TaskProvenance {
                caller_agent_id: Some(profile.agent_id.clone()),
                caller_name: Some(profile.name.clone()),
                caller_version: Some(profile.version.clone()),
            })
        } else {
            None
        };

        let task = A2ATask {
            id: task_id.clone(),
            status: TaskStatus::Pending,
            status_reason: Some("Accepted by remote agent".to_string()),
            messages: vec![request.message],
            artifacts: vec![],
            retry_count: 0,
            run_id: request.run_id,
            parent_task_id: request.parent_task_id,
            role: request.role,
            requested_capabilities: request.requested_capabilities,
            contract: request.contract,
            provenance,
            audit_log: vec![TaskAuditEvent {
                at: now,
                event: "created".to_string(),
                detail: Some("Remote task accepted".to_string()),
            }],
            created_at: now,
            updated_at: now,
            metadata,
        };

        {
            let mut tasks = self.tasks.write().unwrap();
            tasks.insert(task_id.clone(), task.clone());
        }

        tracing::info!(task_id = %task_id, caller = ?caller.map(|p| &p.agent_id), "A2A task created");
        serde_json::to_value(&task).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {e}"),
            data: None,
        })
    }

    fn handle_profile_register(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, A2AError> {
        let profile: AgentProfile =
            serde_json::from_value(params.clone()).map_err(|e| A2AError {
                code: -32602,
                message: format!("Invalid profile: {e}"),
                data: None,
            })?;

        let agent_id = profile.agent_id.clone();
        self.profile_store.store(profile);

        tracing::info!(agent_id = %agent_id, "A2A profile registered");
        Ok(serde_json::json!({"success": true, "agentId": agent_id}))
    }

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
            Some(profile) => Ok(serde_json::json!({
                "valid": true,
                "agentId": profile.agent_id,
                "name": profile.name,
                "capabilities": profile.capabilities,
            })),
            None => Ok(serde_json::json!({"valid": false})),
        }
    }

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
            message: format!("Task not found: {task_id}"),
            data: None,
        })?;

        serde_json::to_value(task).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {e}"),
            data: None,
        })
    }

    fn handle_task_retry(&self, params: &serde_json::Value) -> Result<serde_json::Value, A2AError> {
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
            message: format!("Task not found: {task_id}"),
            data: None,
        })?;

        task.status = TaskStatus::Pending;
        task.status_reason = Some("Retry requested".to_string());
        task.retry_count += 1;
        task.updated_at = Utc::now();
        task.audit_log.push(TaskAuditEvent {
            at: task.updated_at,
            event: "retried".to_string(),
            detail: Some(format!("Retry count is now {}", task.retry_count)),
        });

        serde_json::to_value(task).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {e}"),
            data: None,
        })
    }

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
            message: format!("Task not found: {task_id}"),
            data: None,
        })?;

        task.status = TaskStatus::Cancelled;
        task.status_reason = Some("Cancelled by caller".to_string());
        task.updated_at = Utc::now();
        task.audit_log.push(TaskAuditEvent {
            at: task.updated_at,
            event: "cancelled".to_string(),
            detail: Some("Remote caller cancelled task".to_string()),
        });
        tracing::info!(task_id = %task_id, "A2A task cancelled");

        serde_json::to_value(task).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {e}"),
            data: None,
        })
    }

    /// Update task status (for internal use).
    pub fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<(), String> {
        let mut tasks = self.tasks.write().unwrap();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Task not found: {task_id}"))?;
        task.status = status;
        task.updated_at = Utc::now();
        task.status_reason = Some(format!("Updated to {:?}", task.status));
        task.audit_log.push(TaskAuditEvent {
            at: task.updated_at,
            event: "status_updated".to_string(),
            detail: Some(format!("Task status changed to {:?}", task.status)),
        });
        Ok(())
    }

    /// Add an artifact to a task (for internal use).
    pub fn add_artifact(&self, task_id: &str, artifact: Artifact) -> Result<(), String> {
        let mut tasks = self.tasks.write().unwrap();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Task not found: {task_id}"))?;
        task.artifacts.push(artifact);
        task.updated_at = Utc::now();
        task.audit_log.push(TaskAuditEvent {
            at: task.updated_at,
            event: "artifact_added".to_string(),
            detail: Some("Artifact attached to remote task".to_string()),
        });
        Ok(())
    }
}

/// Create a default agent card for Gestura.
pub fn create_gestura_agent_card(base_url: &str) -> AgentCard {
    use super::{AuthenticationInfo, Skill};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{A2ARequest, MessagePart};

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
        let server = A2AServer::new(card);

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
        // Create a card without authentication for testing basic lifecycle.
        let card = AgentCard {
            name: "test-agent".to_string(),
            description: "Test agent".to_string(),
            url: "http://localhost:8080/a2a".to_string(),
            protocol_version: "0.3.0".to_string(),
            skills: vec![],
            authentication: None,
            default_input_modes: vec!["text".to_string()],
            default_output_modes: vec!["text".to_string()],
        };
        let server = A2AServer::new(card);

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

        let status_request = A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/status".to_string(),
            params: serde_json::json!({"taskId": task.id}),
            id: serde_json::json!(2),
        };

        let status_response = server.handle_request(status_request);
        assert!(status_response.result.is_some());

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
