//! A2A Protocol Types
//!
//! Core types for the Agent-to-Agent protocol.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn is_false(value: &bool) -> bool {
    !*value
}

/// A2A JSON-RPC Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ARequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
    pub id: serde_json::Value,
}

impl A2ARequest {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
            id: serde_json::json!(uuid::Uuid::new_v4().to_string()),
        }
    }
}

/// A2A JSON-RPC Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<A2AError>,
    pub id: serde_json::Value,
}

/// A2A Error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Agent Card for A2A discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub url: String,
    pub protocol_version: String,
    pub skills: Vec<Skill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<AuthenticationInfo>,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_rpc_methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_task_features: Vec<String>,
}

/// Skill definition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub input_modes: Vec<String>,
    #[serde(default)]
    pub output_modes: Vec<String>,
    #[serde(default)]
    pub examples: Vec<String>,
}

/// Authentication info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationInfo {
    pub schemes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth2: Option<OAuth2Config>,
}

/// OAuth2 configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuth2Config {
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// A2A Task
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ATask {
    pub id: String,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    pub messages: Vec<A2AMessage>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub requested_capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<RemoteTaskContract>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease: Option<RemoteTaskLease>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<RemoteTaskProgress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<TaskProvenance>,
    #[serde(default)]
    pub audit_log: Vec<TaskAuditEvent>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Rich remote task create request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskRequest {
    pub message: A2AMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub requested_capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<RemoteTaskContract>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease_request: Option<RemoteTaskLeaseRequest>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl CreateTaskRequest {
    pub fn from_message(message: A2AMessage) -> Self {
        Self {
            message,
            run_id: None,
            parent_task_id: None,
            role: None,
            requested_capabilities: Vec::new(),
            contract: None,
            idempotency_key: None,
            lease_request: None,
            metadata: HashMap::new(),
        }
    }
}

/// Structured contract shared with a remote worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTaskContract {
    pub objective: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub deliverables: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>,
}

/// Requested lease semantics for a remote task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTaskLeaseRequest {
    /// Requested lease time-to-live in seconds.
    pub ttl_secs: u64,
    /// Expected heartbeat cadence in seconds.
    pub heartbeat_interval_secs: u64,
}

/// Active lease state for a remote task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTaskLease {
    /// Stable lease identifier.
    pub lease_id: String,
    /// Agent currently holding the lease.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holder_agent_id: Option<String>,
    /// When the lease was acquired.
    pub acquired_at: DateTime<Utc>,
    /// Most recent heartbeat timestamp.
    pub last_heartbeat_at: DateTime<Utc>,
    /// Lease expiry timestamp.
    pub expires_at: DateTime<Utc>,
    /// Configured heartbeat interval in seconds.
    pub heartbeat_interval_secs: u64,
}

/// Progress snapshot reported by a remote worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTaskProgress {
    /// Optional current stage or step name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// Optional human-readable message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Percent complete from 0-100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    /// Snapshot timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Heartbeat/update request for a remote task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskHeartbeatRequest {
    /// Task identifier to update.
    pub task_id: String,
    /// Optional status transition reported by the worker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    /// Optional status reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    /// Optional progress snapshot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<RemoteTaskProgress>,
    /// Optional artifacts produced since the last heartbeat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    /// Optional new lease ttl in seconds; if absent, reuse the prior ttl window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extend_lease_secs: Option<u64>,
}

/// Remote provenance metadata for a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProvenance {
    /// Authenticated caller identifier if one was established.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_agent_id: Option<String>,
    /// Human-friendly caller name if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_name: Option<String>,
    /// Caller software version if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_version: Option<String>,
    /// Caller-advertised capabilities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caller_capabilities: Vec<String>,
    /// Whether the caller was authenticated when the task was accepted.
    #[serde(default, skip_serializing_if = "is_false")]
    pub authenticated: bool,
    /// Authentication scheme used to authenticate the caller.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_scheme: Option<String>,
}

/// Audit entry recorded for task lifecycle changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAuditEvent {
    pub at: DateTime<Utc>,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Task status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Blocked,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// A2A Message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2AMessage {
    pub role: String,
    pub parts: Vec<MessagePart>,
}

/// Message part (text, file, or data)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MessagePart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "file")]
    File {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    #[serde(rename = "data")]
    Data { data: serde_json::Value },
}

/// Artifact produced by a task
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub name: String,
    pub parts: Vec<MessagePart>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Summary metadata for an artifact without its full payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactManifestEntry {
    pub name: String,
    #[serde(default)]
    pub part_count: usize,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Request to fetch a specific artifact for a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskArtifactFetchRequest {
    pub task_id: String,
    pub artifact_name: String,
}

/// Event emitted when an A2A task changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum A2ATaskEventKind {
    Created,
    Updated,
    Completed,
    Failed,
    Cancelled,
    ArtifactAdded,
}

/// Streamable task event payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ATaskEvent {
    pub kind: A2ATaskEventKind,
    pub task: A2ATask,
    pub emitted_at: DateTime<Utc>,
}
