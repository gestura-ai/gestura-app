//! A2A HTTP Client
//!
//! HTTP client for communicating with A2A-compatible agents.

use super::types::*;
use crate::error::AppError;
use std::time::Duration;

type Result<T> = std::result::Result<T, AppError>;

/// A2A HTTP Client for agent-to-agent communication
pub struct A2AClient {
    client: reqwest::Client,
    auth_token: Option<String>,
}

impl Default for A2AClient {
    fn default() -> Self {
        Self::new()
    }
}

impl A2AClient {
    /// Create a new A2A client
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            client,
            auth_token: None,
        }
    }

    /// Create a client with authentication token
    pub fn with_auth(token: impl Into<String>) -> Self {
        let mut client = Self::new();
        client.auth_token = Some(token.into());
        client
    }

    /// Set authentication token
    pub fn set_auth_token(&mut self, token: impl Into<String>) {
        self.auth_token = Some(token.into());
    }

    /// Send a JSON-RPC request to an A2A endpoint
    async fn send_request(&self, url: &str, request: A2ARequest) -> Result<A2AResponse> {
        let mut req_builder = self.client.post(url).json(&request);

        if let Some(ref token) = self.auth_token {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
        }

        let response = req_builder
            .send()
            .await
            .map_err(|e| AppError::Io(std::io::Error::other(format!("A2A request failed: {e}"))))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AppError::Io(std::io::Error::other(format!(
                "A2A request failed with status {}: {}",
                status, text
            ))));
        }

        response.json::<A2AResponse>().await.map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to parse A2A response: {e}"
            )))
        })
    }

    /// Discover an agent at the given URL
    pub async fn discover(&self, url: &str) -> Result<AgentCard> {
        let request = A2ARequest::new("agent/discover", serde_json::Value::Null);
        let response = self.send_request(url, request).await?;

        if let Some(error) = response.error {
            return Err(AppError::Io(std::io::Error::other(format!(
                "A2A error {}: {}",
                error.code, error.message
            ))));
        }

        let result = response
            .result
            .ok_or_else(|| AppError::Io(std::io::Error::other("No result in A2A response")))?;

        serde_json::from_value(result).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to parse AgentCard: {e}"
            )))
        })
    }

    /// Create a task on a remote agent
    pub async fn create_task(&self, url: &str, message: &str) -> Result<A2ATask> {
        let params = serde_json::json!({
            "role": "user",
            "parts": [{"type": "text", "text": message}]
        });

        let request = A2ARequest::new("task/create", params);
        let response = self.send_request(url, request).await?;

        if let Some(error) = response.error {
            return Err(AppError::Io(std::io::Error::other(format!(
                "A2A error {}: {}",
                error.code, error.message
            ))));
        }

        let result = response
            .result
            .ok_or_else(|| AppError::Io(std::io::Error::other("No result in A2A response")))?;

        serde_json::from_value(result).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to parse A2ATask: {e}"
            )))
        })
    }

    /// Get task status
    pub async fn get_task_status(&self, url: &str, task_id: &str) -> Result<A2ATask> {
        let params = serde_json::json!({"taskId": task_id});
        let request = A2ARequest::new("task/status", params);
        let response = self.send_request(url, request).await?;

        if let Some(error) = response.error {
            return Err(AppError::Io(std::io::Error::other(format!(
                "A2A error {}: {}",
                error.code, error.message
            ))));
        }

        let result = response
            .result
            .ok_or_else(|| AppError::Io(std::io::Error::other("No result in A2A response")))?;

        serde_json::from_value(result).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to parse A2ATask: {e}"
            )))
        })
    }

    /// Cancel a task
    pub async fn cancel_task(&self, url: &str, task_id: &str) -> Result<A2ATask> {
        let params = serde_json::json!({"taskId": task_id});
        let request = A2ARequest::new("task/cancel", params);
        let response = self.send_request(url, request).await?;

        if let Some(error) = response.error {
            return Err(AppError::Io(std::io::Error::other(format!(
                "A2A error {}: {}",
                error.code, error.message
            ))));
        }

        let result = response
            .result
            .ok_or_else(|| AppError::Io(std::io::Error::other("No result in A2A response")))?;

        serde_json::from_value(result).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to parse A2ATask: {e}"
            )))
        })
    }
}
