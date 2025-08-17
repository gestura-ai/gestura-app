//! MCP integration and local MDH (Stage 4 scaffolding)
//! - McpIntegrator trait for tool exposure and dual auth
//! - Local MDH translate function: reads a JSON(-LD) file, validates shape, and emits
//!   an MCP-compatible resource (local-only, offline)

use std::{path::PathBuf, sync::RwLock};
use serde::{Deserialize, Serialize};

use crate::AppError;

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
        .map_err(|e| AppError::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("read mdh file: {e}"))))?;
    let mut value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| AppError::Json(e))?;

    // Basic validation and derive a type for URI
    let mut uri_type = None;
    match &value {
        serde_json::Value::Object(map) => {
            if let Some(t) = map.get("@type").and_then(|v| v.as_str()) { uri_type = Some(t.to_string()); }
        }
        _ => {}
    }
    if uri_type.is_none() {
        // try a nested object or fallback to filename stem
        uri_type = Some(ld_file.file_stem().and_then(|s| s.to_str()).unwrap_or("local").to_string());
    }

    // Simulate compacting by removing @context if present (local-only)
    if let serde_json::Value::Object(map) = &mut value {
        if map.contains_key("@context") { map.remove("@context"); }
    }

    let uri = format!("mcp://mdh/{}", uri_type.unwrap());
    Ok(MdhResource { uri, data: value })
}

/// Minimal integrator for scaffolding; keeps an in-memory list of tools.
pub struct LocalMcp {
    tools: RwLock<Vec<String>>,
}

impl Default for LocalMcp { fn default() -> Self { Self { tools: RwLock::new(Vec::new()) } } }

#[async_trait::async_trait]
impl McpIntegrator for LocalMcp {
    async fn expose_tool(&self, tool: &str) -> Result<(), AppError> {
        let mut guard = self.tools.write().map_err(|e| AppError::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("rwlock: {e}"))))?;
        guard.push(tool.to_string());
        tracing::info!("Exposed MCP tool: {}", tool);
        Ok(())
    }

    async fn authenticate_haptic(&self, _token: &str) -> Result<bool, AppError> {
        // TODO: Implement real token validation
        Ok(true)
    }

    async fn validate_token(&self, _token: &str) -> Result<bool, AppError> {
        // TODO: Implement real token validation
        Ok(true)
    }

    async fn grant_haptic_permission(&self, token: &str) -> Result<(), AppError> {
        tracing::info!("Granted haptic permission for token: {}", token);
        Ok(())
    }
}
