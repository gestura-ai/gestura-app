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
    A2AError, A2AMessage, A2ARequest, A2AResponse, A2ATask, A2ATaskEvent, A2ATaskEventKind,
    AgentCard, Artifact, ArtifactManifestEntry, CreateTaskRequest, RemoteTaskLease,
    RemoteTaskLeaseRequest, TaskArtifactFetchRequest, TaskAuditEvent, TaskHeartbeatRequest,
    TaskProvenance, TaskStatus,
};
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use rand::distributions::Alphanumeric;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyCreateTaskRequest {
    #[serde(flatten)]
    message: A2AMessage,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    parent_task_id: Option<String>,
    #[serde(default, rename = "taskRole")]
    task_role: Option<String>,
    #[serde(default)]
    requested_capabilities: Vec<String>,
    #[serde(default)]
    contract: Option<super::RemoteTaskContract>,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    lease_request: Option<RemoteTaskLeaseRequest>,
    #[serde(default)]
    metadata: HashMap<String, serde_json::Value>,
}

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
    /// Scoped idempotency-key to task-id mapping.
    idempotency_index: std::sync::RwLock<HashMap<String, String>>,
    /// Task-to-caller-profile mapping.
    task_profiles: std::sync::RwLock<HashMap<String, AgentProfile>>,
    /// Broadcast stream for task lifecycle changes.
    event_subscribers: std::sync::Mutex<Vec<std::sync::mpsc::Sender<A2ATaskEvent>>>,
}

impl A2AServer {
    /// Create a new A2A server for the given agent card.
    pub fn new(agent_card: AgentCard) -> Self {
        Self {
            agent_card,
            registry: AgentCardRegistry::new(),
            profile_store: ProfileStore::new(),
            tasks: std::sync::RwLock::new(HashMap::new()),
            idempotency_index: std::sync::RwLock::new(HashMap::new()),
            task_profiles: std::sync::RwLock::new(HashMap::new()),
            event_subscribers: std::sync::Mutex::new(Vec::new()),
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
            "task/create"
                | "task/status"
                | "task/cancel"
                | "task/retry"
                | "task/heartbeat"
                | "task/artifacts"
                | "task/artifact"
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
            "task/status" => self.handle_task_status(&request.params, caller_profile.as_ref()),
            "task/cancel" => self.handle_task_cancel(&request.params, caller_profile.as_ref()),
            "task/retry" => self.handle_task_retry(&request.params, caller_profile.as_ref()),
            "task/heartbeat" => {
                self.handle_task_heartbeat(&request.params, caller_profile.as_ref())
            }
            "task/artifacts" => {
                self.handle_task_artifacts(&request.params, caller_profile.as_ref())
            }
            "task/artifact" => self.handle_task_artifact(&request.params, caller_profile.as_ref()),
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

    /// Subscribe to task lifecycle events for streaming adapters.
    pub fn subscribe_events(&self) -> std::sync::mpsc::Receiver<A2ATaskEvent> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.event_subscribers.lock().unwrap().push(sender);
        receiver
    }

    /// List currently known tasks.
    pub fn list_tasks(&self) -> Vec<A2ATask> {
        self.tasks.read().unwrap().values().cloned().collect()
    }

    /// Get a specific task snapshot.
    pub fn get_task(&self, task_id: &str) -> Option<A2ATask> {
        self.tasks.read().unwrap().get(task_id).cloned()
    }

    fn emit_task_event(&self, kind: A2ATaskEventKind, task: &A2ATask) {
        let event = A2ATaskEvent {
            kind,
            task: task.clone(),
            emitted_at: Utc::now(),
        };
        let mut subscribers = self.event_subscribers.lock().unwrap();
        subscribers.retain(|subscriber| subscriber.send(event.clone()).is_ok());
    }

    fn scoped_idempotency_key(caller: Option<&AgentProfile>, idempotency_key: &str) -> String {
        let caller_scope = caller
            .map(|profile| profile.agent_id.as_str())
            .unwrap_or("anonymous");
        format!("{caller_scope}:{idempotency_key}")
    }

    fn lease_from_request(
        &self,
        request: &RemoteTaskLeaseRequest,
        acquired_at: DateTime<Utc>,
    ) -> RemoteTaskLease {
        let ttl_secs = request.ttl_secs.min(i64::MAX as u64) as i64;
        RemoteTaskLease {
            lease_id: uuid::Uuid::new_v4().to_string(),
            holder_agent_id: Some(self.agent_card.name.clone()),
            acquired_at,
            last_heartbeat_at: acquired_at,
            expires_at: acquired_at + Duration::seconds(ttl_secs),
            heartbeat_interval_secs: request.heartbeat_interval_secs,
        }
    }

    fn reconcile_task_lease(task: &mut A2ATask) {
        let Some(lease) = task.lease.as_ref() else {
            return;
        };
        if matches!(
            task.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        ) {
            return;
        }
        if Utc::now() >= lease.expires_at && !matches!(task.status, TaskStatus::Blocked) {
            let expired_at = Utc::now();
            task.status = TaskStatus::Blocked;
            task.status_reason = Some("Remote lease heartbeat expired".to_string());
            task.updated_at = expired_at;
            task.audit_log.push(TaskAuditEvent {
                at: expired_at,
                event: "lease_expired".to_string(),
                detail: Some(
                    "Remote worker heartbeat was not renewed before lease expiry".to_string(),
                ),
            });
        }
    }

    fn ensure_task_owner(
        &self,
        task_id: &str,
        caller: Option<&AgentProfile>,
    ) -> Result<(), A2AError> {
        if self.agent_card.authentication.is_none() {
            return Ok(());
        }
        let owner = self.get_task_caller(task_id).ok_or_else(|| A2AError {
            code: -32003,
            message: format!("Task {task_id} has no registered owner"),
            data: None,
        })?;
        let caller = caller.ok_or_else(|| A2AError {
            code: -32000,
            message: "Authentication required".to_string(),
            data: None,
        })?;
        if caller.agent_id != owner.agent_id {
            return Err(A2AError {
                code: -32004,
                message: format!(
                    "Caller {} is not authorized to mutate task owned by {}",
                    caller.agent_id, owner.agent_id
                ),
                data: None,
            });
        }
        Ok(())
    }

    fn caller_auth_scheme(&self, caller: Option<&AgentProfile>) -> Option<String> {
        caller.and_then(|_| {
            self.agent_card
                .authentication
                .as_ref()
                .and_then(|authentication| authentication.schemes.first().cloned())
        })
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
                serde_json::from_value::<LegacyCreateTaskRequest>(params.clone()).map(|legacy| {
                    CreateTaskRequest {
                        message: legacy.message,
                        run_id: legacy.run_id,
                        parent_task_id: legacy.parent_task_id,
                        role: legacy.task_role,
                        requested_capabilities: legacy.requested_capabilities,
                        contract: legacy.contract,
                        idempotency_key: legacy.idempotency_key,
                        lease_request: legacy.lease_request,
                        metadata: legacy.metadata,
                    }
                })
            })
            .map_err(|e| A2AError {
                code: -32602,
                message: format!("Invalid params: {e}"),
                data: None,
            })?;

        if let Some(idempotency_key) = request.idempotency_key.as_deref() {
            let scoped_key = Self::scoped_idempotency_key(caller, idempotency_key);
            if let Some(existing_task_id) = self
                .idempotency_index
                .read()
                .unwrap()
                .get(&scoped_key)
                .cloned()
            {
                let mut tasks = self.tasks.write().unwrap();
                if let Some(existing_task) = tasks.get_mut(&existing_task_id) {
                    existing_task.updated_at = Utc::now();
                    existing_task.audit_log.push(TaskAuditEvent {
                        at: existing_task.updated_at,
                        event: "idempotent_replay".to_string(),
                        detail: Some(format!(
                            "Duplicate task/create request reused idempotency key {}",
                            idempotency_key
                        )),
                    });
                    return serde_json::to_value(existing_task).map_err(|e| A2AError {
                        code: -32603,
                        message: format!("Serialization error: {e}"),
                        data: None,
                    });
                }
            }
        }

        let task_id = uuid::Uuid::new_v4().to_string();
        let mut metadata = request.metadata;
        let now = Utc::now();

        let provenance = if let Some(profile) = caller {
            metadata.insert(
                "caller_agent_id".to_string(),
                serde_json::json!(profile.agent_id),
            );
            metadata.insert("caller_name".to_string(), serde_json::json!(profile.name));
            metadata.insert(
                "caller_version".to_string(),
                serde_json::json!(profile.version),
            );
            metadata.insert(
                "caller_capabilities".to_string(),
                serde_json::json!(profile.capabilities),
            );
            metadata.insert("caller_authenticated".to_string(), serde_json::json!(true));
            if let Some(auth_scheme) = self.caller_auth_scheme(caller) {
                metadata.insert(
                    "caller_auth_scheme".to_string(),
                    serde_json::json!(auth_scheme),
                );
            }

            let mut profiles = self.task_profiles.write().unwrap();
            profiles.insert(task_id.clone(), profile.clone());

            Some(TaskProvenance {
                caller_agent_id: Some(profile.agent_id.clone()),
                caller_name: Some(profile.name.clone()),
                caller_version: Some(profile.version.clone()),
                caller_capabilities: profile.capabilities.clone(),
                authenticated: true,
                auth_scheme: self.caller_auth_scheme(caller),
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
            idempotency_key: request.idempotency_key.clone(),
            lease: request
                .lease_request
                .as_ref()
                .map(|lease_request| self.lease_from_request(lease_request, now)),
            progress: None,
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
        if let Some(idempotency_key) = request.idempotency_key {
            let scoped_key = Self::scoped_idempotency_key(caller, &idempotency_key);
            self.idempotency_index
                .write()
                .unwrap()
                .insert(scoped_key, task_id.clone());
        }
        self.emit_task_event(A2ATaskEventKind::Created, &task);

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
        caller: Option<&AgentProfile>,
    ) -> Result<serde_json::Value, A2AError> {
        let task_id = params
            .get("taskId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| A2AError {
                code: -32602,
                message: "Missing taskId parameter".to_string(),
                data: None,
            })?;

        self.ensure_task_owner(task_id, caller)?;
        let mut tasks = self.tasks.write().unwrap();
        let task = tasks.get_mut(task_id).ok_or_else(|| A2AError {
            code: -32001,
            message: format!("Task not found: {task_id}"),
            data: None,
        })?;
        let previous_status = task.status;
        Self::reconcile_task_lease(task);
        if task.status != previous_status {
            self.emit_task_event(A2ATaskEventKind::Updated, task);
        }

        serde_json::to_value(task).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {e}"),
            data: None,
        })
    }

    fn handle_task_artifacts(
        &self,
        params: &serde_json::Value,
        caller: Option<&AgentProfile>,
    ) -> Result<serde_json::Value, A2AError> {
        let task_id = params
            .get("taskId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| A2AError {
                code: -32602,
                message: "Missing taskId parameter".to_string(),
                data: None,
            })?;
        self.ensure_task_owner(task_id, caller)?;
        let tasks = self.tasks.read().unwrap();
        let task = tasks.get(task_id).ok_or_else(|| A2AError {
            code: -32001,
            message: format!("Task not found: {task_id}"),
            data: None,
        })?;
        let manifest = task
            .artifacts
            .iter()
            .map(|artifact| ArtifactManifestEntry {
                name: artifact.name.clone(),
                part_count: artifact.parts.len(),
                metadata: artifact.metadata.clone(),
            })
            .collect::<Vec<_>>();
        serde_json::to_value(manifest).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {e}"),
            data: None,
        })
    }

    fn handle_task_artifact(
        &self,
        params: &serde_json::Value,
        caller: Option<&AgentProfile>,
    ) -> Result<serde_json::Value, A2AError> {
        let request =
            serde_json::from_value::<TaskArtifactFetchRequest>(params.clone()).map_err(|e| {
                A2AError {
                    code: -32602,
                    message: format!("Invalid artifact fetch request: {e}"),
                    data: None,
                }
            })?;
        self.ensure_task_owner(&request.task_id, caller)?;
        let tasks = self.tasks.read().unwrap();
        let task = tasks.get(&request.task_id).ok_or_else(|| A2AError {
            code: -32001,
            message: format!("Task not found: {}", request.task_id),
            data: None,
        })?;
        let artifact = task
            .artifacts
            .iter()
            .find(|artifact| artifact.name == request.artifact_name)
            .cloned()
            .ok_or_else(|| A2AError {
                code: -32005,
                message: format!(
                    "Artifact '{}' not found for task {}",
                    request.artifact_name, request.task_id
                ),
                data: None,
            })?;
        serde_json::to_value(artifact).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {e}"),
            data: None,
        })
    }

    fn handle_task_retry(
        &self,
        params: &serde_json::Value,
        caller: Option<&AgentProfile>,
    ) -> Result<serde_json::Value, A2AError> {
        let task_id = params
            .get("taskId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| A2AError {
                code: -32602,
                message: "Missing taskId parameter".to_string(),
                data: None,
            })?;

        self.ensure_task_owner(task_id, caller)?;
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
        self.emit_task_event(A2ATaskEventKind::Updated, task);

        serde_json::to_value(task).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {e}"),
            data: None,
        })
    }

    fn handle_task_heartbeat(
        &self,
        params: &serde_json::Value,
        caller: Option<&AgentProfile>,
    ) -> Result<serde_json::Value, A2AError> {
        let heartbeat =
            serde_json::from_value::<TaskHeartbeatRequest>(params.clone()).map_err(|e| {
                A2AError {
                    code: -32602,
                    message: format!("Invalid heartbeat request: {e}"),
                    data: None,
                }
            })?;

        self.ensure_task_owner(&heartbeat.task_id, caller)?;
        let mut tasks = self.tasks.write().unwrap();
        let task = tasks.get_mut(&heartbeat.task_id).ok_or_else(|| A2AError {
            code: -32001,
            message: format!("Task not found: {}", heartbeat.task_id),
            data: None,
        })?;

        let now = Utc::now();
        let Some(lease) = task.lease.as_mut() else {
            return Err(A2AError {
                code: -32002,
                message: format!("Task {} does not have an active lease", heartbeat.task_id),
                data: None,
            });
        };

        lease.last_heartbeat_at = now;
        let extend_secs = heartbeat
            .extend_lease_secs
            .unwrap_or(lease.heartbeat_interval_secs)
            .min(i64::MAX as u64) as i64;
        lease.expires_at = now + Duration::seconds(extend_secs);

        if let Some(status) = heartbeat.status {
            task.status = status;
        }
        if heartbeat.status_reason.is_some() {
            task.status_reason = heartbeat.status_reason;
        }
        if heartbeat.progress.is_some() {
            task.progress = heartbeat.progress;
        }
        if !heartbeat.artifacts.is_empty() {
            task.artifacts.extend(heartbeat.artifacts);
        }
        task.updated_at = now;
        task.audit_log.push(TaskAuditEvent {
            at: now,
            event: "heartbeat".to_string(),
            detail: Some(format!(
                "Lease renewed until {}",
                lease.expires_at.to_rfc3339()
            )),
        });
        self.emit_task_event(A2ATaskEventKind::Updated, task);

        serde_json::to_value(task).map_err(|e| A2AError {
            code: -32603,
            message: format!("Serialization error: {e}"),
            data: None,
        })
    }

    fn handle_task_cancel(
        &self,
        params: &serde_json::Value,
        caller: Option<&AgentProfile>,
    ) -> Result<serde_json::Value, A2AError> {
        let task_id = params
            .get("taskId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| A2AError {
                code: -32602,
                message: "Missing taskId parameter".to_string(),
                data: None,
            })?;

        self.ensure_task_owner(task_id, caller)?;
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
        self.emit_task_event(A2ATaskEventKind::Cancelled, task);
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
        let kind = match task.status {
            TaskStatus::Completed => A2ATaskEventKind::Completed,
            TaskStatus::Failed => A2ATaskEventKind::Failed,
            TaskStatus::Cancelled => A2ATaskEventKind::Cancelled,
            _ => A2ATaskEventKind::Updated,
        };
        self.emit_task_event(kind, task);
        Ok(())
    }

    /// Update task status and message for internal execution bridges.
    pub fn update_task_status_with_reason(
        &self,
        task_id: &str,
        status: TaskStatus,
        reason: impl Into<String>,
    ) -> Result<(), String> {
        let reason = reason.into();
        let mut tasks = self.tasks.write().unwrap();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Task not found: {task_id}"))?;
        task.status = status;
        task.status_reason = Some(reason.clone());
        task.updated_at = Utc::now();
        task.audit_log.push(TaskAuditEvent {
            at: task.updated_at,
            event: "status_updated".to_string(),
            detail: Some(reason),
        });
        let kind = match task.status {
            TaskStatus::Completed => A2ATaskEventKind::Completed,
            TaskStatus::Failed => A2ATaskEventKind::Failed,
            TaskStatus::Cancelled => A2ATaskEventKind::Cancelled,
            _ => A2ATaskEventKind::Updated,
        };
        self.emit_task_event(kind, task);
        Ok(())
    }

    /// Report in-flight task progress for internal execution bridges.
    pub fn update_task_progress(
        &self,
        task_id: &str,
        progress: super::RemoteTaskProgress,
    ) -> Result<(), String> {
        let mut tasks = self.tasks.write().unwrap();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Task not found: {task_id}"))?;
        task.progress = Some(progress);
        task.updated_at = Utc::now();
        task.audit_log.push(TaskAuditEvent {
            at: task.updated_at,
            event: "progress_updated".to_string(),
            detail: Some("Remote execution progress updated".to_string()),
        });
        self.emit_task_event(A2ATaskEventKind::Updated, task);
        Ok(())
    }

    /// Append a message to a task for final result delivery.
    pub fn add_message(&self, task_id: &str, message: A2AMessage) -> Result<(), String> {
        let mut tasks = self.tasks.write().unwrap();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Task not found: {task_id}"))?;
        task.messages.push(message);
        task.updated_at = Utc::now();
        task.audit_log.push(TaskAuditEvent {
            at: task.updated_at,
            event: "message_added".to_string(),
            detail: Some("Message appended to remote task transcript".to_string()),
        });
        self.emit_task_event(A2ATaskEventKind::Updated, task);
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
        self.emit_task_event(A2ATaskEventKind::ArtifactAdded, task);
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
        supported_rpc_methods: vec![
            "agent/discover".to_string(),
            "task/create".to_string(),
            "task/status".to_string(),
            "task/cancel".to_string(),
            "task/retry".to_string(),
            "task/heartbeat".to_string(),
            "task/artifacts".to_string(),
            "task/artifact".to_string(),
            "profile/register".to_string(),
            "profile/validate".to_string(),
        ],
        supported_task_features: vec![
            "idempotency".to_string(),
            "leases".to_string(),
            "progress".to_string(),
            "artifacts".to_string(),
            "sse-events".to_string(),
            "provenance".to_string(),
            "authenticated-mutations".to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{A2ARequest, AuthenticationInfo, MessagePart};
    use std::collections::HashMap;

    fn test_card(authentication: Option<AuthenticationInfo>) -> AgentCard {
        AgentCard {
            name: "test-agent".to_string(),
            description: "Test agent".to_string(),
            url: "http://localhost:8080/a2a".to_string(),
            protocol_version: "0.3.0".to_string(),
            skills: vec![],
            authentication,
            default_input_modes: vec!["text".to_string()],
            default_output_modes: vec!["text".to_string()],
            supported_rpc_methods: vec![
                "agent/discover".to_string(),
                "task/create".to_string(),
                "task/status".to_string(),
                "task/cancel".to_string(),
                "task/retry".to_string(),
                "task/heartbeat".to_string(),
                "task/artifacts".to_string(),
                "task/artifact".to_string(),
            ],
            supported_task_features: vec![
                "idempotency".to_string(),
                "leases".to_string(),
                "progress".to_string(),
                "artifacts".to_string(),
                "sse-events".to_string(),
                "provenance".to_string(),
                "authenticated-mutations".to_string(),
            ],
        }
    }

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
        let server = A2AServer::new(test_card(None));

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
    fn test_task_create_reuses_idempotency_key_for_same_caller_scope() {
        let server = A2AServer::new(test_card(None));

        let first = server.handle_request(A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/create".to_string(),
            params: serde_json::json!({
                "role": "user",
                "parts": [{"type": "text", "text": "Do the remote task"}],
                "idempotencyKey": "idem-123"
            }),
            id: serde_json::json!(1),
        });
        let second = server.handle_request(A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/create".to_string(),
            params: serde_json::json!({
                "role": "user",
                "parts": [{"type": "text", "text": "Do the remote task again"}],
                "idempotencyKey": "idem-123"
            }),
            id: serde_json::json!(2),
        });

        let first_task: A2ATask = serde_json::from_value(first.result.unwrap()).unwrap();
        let second_task: A2ATask = serde_json::from_value(second.result.unwrap()).unwrap();
        assert_eq!(first_task.id, second_task.id);
        assert!(
            second_task
                .audit_log
                .iter()
                .any(|event| event.event == "idempotent_replay")
        );
    }

    #[test]
    fn test_task_heartbeat_updates_progress_and_lease() {
        let server = A2AServer::new(test_card(None));

        let create_response = server.handle_request(A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/create".to_string(),
            params: serde_json::json!({
                "role": "user",
                "parts": [{"type": "text", "text": "Track leased progress"}],
                "leaseRequest": {"ttlSecs": 30, "heartbeatIntervalSecs": 5}
            }),
            id: serde_json::json!(1),
        });
        let task: A2ATask = serde_json::from_value(create_response.result.unwrap()).unwrap();
        let original_expiry = task.lease.as_ref().unwrap().expires_at;

        let heartbeat_response = server.handle_request(A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/heartbeat".to_string(),
            params: serde_json::json!({
                "taskId": task.id,
                "status": "running",
                "statusReason": "Remote worker is executing",
                "extendLeaseSecs": 60,
                "progress": {
                    "stage": "compile",
                    "message": "Running compile phase",
                    "percent": 40,
                    "updatedAt": Utc::now()
                }
            }),
            id: serde_json::json!(2),
        });
        let updated_task: A2ATask =
            serde_json::from_value(heartbeat_response.result.unwrap()).unwrap();

        assert_eq!(updated_task.status, TaskStatus::Running);
        assert_eq!(
            updated_task
                .progress
                .as_ref()
                .and_then(|progress| progress.percent),
            Some(40)
        );
        assert!(updated_task.lease.as_ref().unwrap().expires_at > original_expiry);
    }

    #[test]
    fn test_task_status_blocks_when_lease_expires() {
        let server = A2AServer::new(test_card(None));

        let create_response = server.handle_request(A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/create".to_string(),
            params: serde_json::json!({
                "role": "user",
                "parts": [{"type": "text", "text": "Expire quickly"}],
                "leaseRequest": {"ttlSecs": 0, "heartbeatIntervalSecs": 1}
            }),
            id: serde_json::json!(1),
        });
        let task: A2ATask = serde_json::from_value(create_response.result.unwrap()).unwrap();

        let status_response = server.handle_request(A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/status".to_string(),
            params: serde_json::json!({"taskId": task.id}),
            id: serde_json::json!(2),
        });
        let blocked_task: A2ATask =
            serde_json::from_value(status_response.result.unwrap()).unwrap();

        assert_eq!(blocked_task.status, TaskStatus::Blocked);
        assert_eq!(
            blocked_task.status_reason.as_deref(),
            Some("Remote lease heartbeat expired")
        );
    }

    #[test]
    fn test_authenticated_mutations_reject_non_owner_and_artifact_fetch_succeeds_for_owner() {
        let server = A2AServer::new(test_card(Some(AuthenticationInfo {
            schemes: vec!["bearer".to_string()],
            oauth2: None,
        })));
        let mut owner = AgentProfile::new("owner-agent", "Owner");
        owner.generate_token(1);
        let owner_token = owner.auth_token.clone().unwrap();
        server.profile_store.store(owner.clone());

        let mut intruder = AgentProfile::new("intruder-agent", "Intruder");
        intruder.generate_token(1);
        let intruder_token = intruder.auth_token.clone().unwrap();
        server.profile_store.store(intruder.clone());

        let create_response = server.handle_request_with_auth(
            A2ARequest {
                jsonrpc: "2.0".to_string(),
                method: "task/create".to_string(),
                params: serde_json::json!({
                    "role": "user",
                    "parts": [{"type": "text", "text": "Protected task"}],
                    "leaseRequest": {"ttlSecs": 30, "heartbeatIntervalSecs": 5}
                }),
                id: serde_json::json!(1),
            },
            Some(&owner_token),
        );
        let task: A2ATask = serde_json::from_value(create_response.result.unwrap()).unwrap();
        assert_eq!(
            task.provenance
                .as_ref()
                .and_then(|value| value.caller_agent_id.as_deref()),
            Some("owner-agent")
        );
        assert_eq!(
            task.provenance.as_ref().map(|value| value.authenticated),
            Some(true)
        );
        assert_eq!(
            task.provenance
                .as_ref()
                .and_then(|value| value.auth_scheme.as_deref()),
            Some("bearer")
        );
        server
            .add_artifact(
                &task.id,
                Artifact {
                    name: "result.txt".to_string(),
                    parts: vec![MessagePart::Text {
                        text: "artifact body".to_string(),
                    }],
                    metadata: HashMap::new(),
                },
            )
            .unwrap();

        let unauthorized = server.handle_request_with_auth(
            A2ARequest {
                jsonrpc: "2.0".to_string(),
                method: "task/cancel".to_string(),
                params: serde_json::json!({"taskId": task.id}),
                id: serde_json::json!(2),
            },
            Some(&intruder_token),
        );
        assert!(unauthorized.error.is_some());

        let unauthorized_status = server.handle_request_with_auth(
            A2ARequest {
                jsonrpc: "2.0".to_string(),
                method: "task/status".to_string(),
                params: serde_json::json!({"taskId": task.id}),
                id: serde_json::json!(3),
            },
            Some(&intruder_token),
        );
        assert!(unauthorized_status.error.is_some());

        let unauthorized_manifest = server.handle_request_with_auth(
            A2ARequest {
                jsonrpc: "2.0".to_string(),
                method: "task/artifacts".to_string(),
                params: serde_json::json!({"taskId": task.id}),
                id: serde_json::json!(4),
            },
            Some(&intruder_token),
        );
        assert!(unauthorized_manifest.error.is_some());

        let artifact_response = server.handle_request_with_auth(
            A2ARequest {
                jsonrpc: "2.0".to_string(),
                method: "task/artifact".to_string(),
                params: serde_json::json!({"taskId": task.id, "artifactName": "result.txt"}),
                id: serde_json::json!(5),
            },
            Some(&owner_token),
        );
        let artifact: Artifact = serde_json::from_value(artifact_response.result.unwrap()).unwrap();
        assert_eq!(artifact.name, "result.txt");

        let status_response = server.handle_request_with_auth(
            A2ARequest {
                jsonrpc: "2.0".to_string(),
                method: "task/status".to_string(),
                params: serde_json::json!({"taskId": task.id}),
                id: serde_json::json!(6),
            },
            Some(&owner_token),
        );
        assert!(status_response.error.is_none());

        let manifest_response = server.handle_request_with_auth(
            A2ARequest {
                jsonrpc: "2.0".to_string(),
                method: "task/artifacts".to_string(),
                params: serde_json::json!({"taskId": task.id}),
                id: serde_json::json!(7),
            },
            Some(&owner_token),
        );
        let manifest: Vec<ArtifactManifestEntry> =
            serde_json::from_value(manifest_response.result.unwrap()).unwrap();
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest[0].name, "result.txt");
    }

    #[test]
    fn test_task_events_and_artifact_manifest_are_emitted() {
        let server = A2AServer::new(test_card(None));
        let events = server.subscribe_events();
        let create_response = server.handle_request(A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/create".to_string(),
            params: serde_json::json!({
                "role": "user",
                "parts": [{"type": "text", "text": "Stream updates"}]
            }),
            id: serde_json::json!(1),
        });
        let task: A2ATask = serde_json::from_value(create_response.result.unwrap()).unwrap();
        let created = events.try_recv().unwrap();
        assert!(matches!(created.kind, A2ATaskEventKind::Created));

        server
            .add_artifact(
                &task.id,
                Artifact {
                    name: "summary.md".to_string(),
                    parts: vec![MessagePart::Text {
                        text: "# done".to_string(),
                    }],
                    metadata: HashMap::from([(
                        "mimeType".to_string(),
                        serde_json::json!("text/markdown"),
                    )]),
                },
            )
            .unwrap();
        let artifact_event = events.try_recv().unwrap();
        assert!(matches!(
            artifact_event.kind,
            A2ATaskEventKind::ArtifactAdded
        ));

        let manifest_response = server.handle_request(A2ARequest {
            jsonrpc: "2.0".to_string(),
            method: "task/artifacts".to_string(),
            params: serde_json::json!({"taskId": task.id}),
            id: serde_json::json!(2),
        });
        let manifest: Vec<ArtifactManifestEntry> =
            serde_json::from_value(manifest_response.result.unwrap()).unwrap();
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest[0].name, "summary.md");
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
