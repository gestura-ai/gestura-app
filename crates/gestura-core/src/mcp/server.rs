//! MCP JSON-RPC server (transport-agnostic)
//!
//! This module implements the MCP JSON-RPC method dispatch layer while remaining
//! transport-agnostic. CLI/GUI crates can host this server over STDIO, HTTP/SSE,
//! or any other transport by forwarding JSON-RPC requests into [`McpServer`].

use super::integrator::McpIntegrator;
use super::notifications::NotificationSender;
use super::types::{
    InitializeParams, PromptsGetParams, ResourcesReadParams, error_codes, mcp_error_codes,
};
use super::{
    McpLogger, McpNotification, ProgressTracker, PromptRegistry, Resource, ResourcesListResult,
    ResourcesReadResult, SessionManager, Tool, ToolsCallResult, ToolsListResult,
    create_notification_channel, create_session_manager, get_mcp,
};
use crate::error::{AppError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// JSON-RPC request structure.
///
/// MCP uses JSON-RPC 2.0 as its transport envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (must be "2.0")
    pub jsonrpc: String,
    /// The method name (e.g. "tools/list")
    pub method: String,
    /// Optional method parameters
    pub params: Option<serde_json::Value>,
    /// Optional request id (missing for notifications)
    pub id: Option<serde_json::Value>,
}

/// JSON-RPC response structure.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version (always "2.0")
    pub jsonrpc: String,
    /// Result payload
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error payload
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    /// Request id (mirrors request id)
    pub id: Option<serde_json::Value>,
}

/// JSON-RPC error payload.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    /// Numeric error code
    pub code: i32,
    /// Human-readable message
    pub message: String,
    /// Optional error data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Context passed into tool/resource handlers.
#[derive(Debug, Clone)]
pub struct McpRequestContext {
    /// Progress tracker for long-running operations.
    pub progress: Arc<ProgressTracker>,
    /// MCP logger for emitting log notifications.
    pub logger: McpLogger,
}

/// Handler for an MCP tool.
#[async_trait]
pub trait McpToolHandler: Send + Sync {
    /// Execute the tool.
    ///
    /// `auth_token` is the token string (if provided by the client). Tools can
    /// use this for secondary checks (e.g., haptic permission) after the server
    /// validates token well-formedness/expiry.
    async fn call(
        &self,
        arguments: Option<serde_json::Value>,
        auth_token: Option<&str>,
        ctx: McpRequestContext,
    ) -> Result<ToolsCallResult>;
}

/// Handler for an MCP resource.
#[async_trait]
pub trait McpResourceHandler: Send + Sync {
    /// Read the resource contents.
    async fn read(&self, uri: &str, ctx: McpRequestContext) -> Result<ResourcesReadResult>;
}

struct ToolEntry {
    tool: Tool,
    requires_auth: bool,
    handler: Arc<dyn McpToolHandler>,
}

impl std::fmt::Debug for ToolEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolEntry")
            .field("tool", &self.tool)
            .field("requires_auth", &self.requires_auth)
            .finish_non_exhaustive()
    }
}

struct ResourceEntry {
    resource: Resource,
    handler: Arc<dyn McpResourceHandler>,
}

impl std::fmt::Debug for ResourceEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceEntry")
            .field("resource", &self.resource)
            .finish_non_exhaustive()
    }
}

/// Transport-agnostic MCP JSON-RPC server.
///
/// This server owns the MCP protocol method routing and delegates tool/resource
/// execution to registered handlers.
#[derive(Debug)]
pub struct McpServer {
    tools: HashMap<String, ToolEntry>,
    resources: HashMap<String, ResourceEntry>,

    session: Arc<SessionManager>,
    prompts: PromptRegistry,

    progress: Arc<ProgressTracker>,
    logger: McpLogger,
    notification_sender: NotificationSender,
}

impl McpServer {
    /// Create a new MCP server with default lifecycle, prompt registry, and
    /// notification channel.
    pub fn new() -> Self {
        let (notification_sender, _) = create_notification_channel();
        let session = create_session_manager();
        let prompts = PromptRegistry::new();

        let progress = Arc::new(ProgressTracker::new(notification_sender.clone()));
        let logger = McpLogger::new(notification_sender.clone(), Some("gestura".to_string()));

        Self {
            tools: HashMap::new(),
            resources: HashMap::new(),
            session,
            prompts,
            progress,
            logger,
            notification_sender,
        }
    }

    /// Get the lifecycle session manager.
    pub fn session(&self) -> &Arc<SessionManager> {
        &self.session
    }

    /// Get the progress tracker.
    pub fn progress_tracker(&self) -> &Arc<ProgressTracker> {
        &self.progress
    }

    /// Subscribe to MCP notifications.
    pub fn subscribe_notifications(&self) -> tokio::sync::broadcast::Receiver<McpNotification> {
        self.notification_sender.subscribe()
    }

    /// Register an MCP tool.
    pub fn register_tool(
        &mut self,
        tool: Tool,
        requires_auth: bool,
        handler: Arc<dyn McpToolHandler>,
    ) {
        self.tools.insert(
            tool.name.clone(),
            ToolEntry {
                tool,
                requires_auth,
                handler,
            },
        );

        // Best-effort notification; ignore send errors if no subscribers.
        let _ = self
            .notification_sender
            .send(McpNotification::ToolsListChanged);
    }

    /// Register an MCP resource.
    pub fn register_resource(&mut self, resource: Resource, handler: Arc<dyn McpResourceHandler>) {
        self.resources
            .insert(resource.uri.clone(), ResourceEntry { resource, handler });
        let _ = self
            .notification_sender
            .send(McpNotification::ResourcesListChanged);
    }

    /// Handle a single JSON-RPC request.
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            // Lifecycle
            "initialize" => self.handle_initialize(request.params, request.id),
            "notifications/initialized" => self.handle_initialized(request.id),
            "ping" => self.handle_ping(request.id),
            "shutdown" => self.handle_shutdown(request.id),

            // Tools
            "tools/list" => self.handle_list_tools(request.id),
            "tools/call" => self.handle_call_tool(request.params, request.id).await,

            // Resources
            "resources/list" => self.handle_list_resources(request.id),
            "resources/read" => self.handle_read_resource(request.params, request.id).await,

            // Prompts
            "prompts/list" => self.handle_list_prompts(request.id),
            "prompts/get" => self.handle_get_prompt(request.params, request.id),

            // Cancellation
            "notifications/cancelled" => self.handle_cancelled(request.params, request.id),

            _ => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: error_codes::METHOD_NOT_FOUND,
                    message: "Method not found".to_string(),
                    data: None,
                }),
                id: request.id,
            },
        }
    }

    fn ctx(&self) -> McpRequestContext {
        McpRequestContext {
            progress: self.progress.clone(),
            logger: self.logger.clone(),
        }
    }

    fn handle_initialize(
        &self,
        params: Option<serde_json::Value>,
        id: Option<serde_json::Value>,
    ) -> JsonRpcResponse {
        let init_params: InitializeParams = match params {
            Some(p) => match serde_json::from_value(p) {
                Ok(v) => v,
                Err(e) => {
                    return JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: None,
                        error: Some(JsonRpcError {
                            code: error_codes::INVALID_PARAMS,
                            message: format!("Invalid params: {e}"),
                            data: None,
                        }),
                        id,
                    };
                }
            },
            None => {
                return JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: "Missing initialize params".to_string(),
                        data: None,
                    }),
                    id,
                };
            }
        };

        match self.session.initialize(init_params) {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::to_value(result).unwrap_or_default()),
                error: None,
                id,
            },
            Err(message) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: mcp_error_codes::ALREADY_INITIALIZED,
                    message,
                    data: None,
                }),
                id,
            },
        }
    }

    fn handle_initialized(&self, id: Option<serde_json::Value>) -> JsonRpcResponse {
        match self.session.initialized() {
            Ok(()) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::json!({})),
                error: None,
                id,
            },
            Err(message) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: mcp_error_codes::NOT_INITIALIZED,
                    message,
                    data: None,
                }),
                id,
            },
        }
    }

    fn handle_ping(&self, id: Option<serde_json::Value>) -> JsonRpcResponse {
        let result = self.session.ping();
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::to_value(result).unwrap_or_default()),
            error: None,
            id,
        }
    }

    fn handle_shutdown(&self, id: Option<serde_json::Value>) -> JsonRpcResponse {
        match self.session.shutdown() {
            Ok(()) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::json!({})),
                error: None,
                id,
            },
            Err(message) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: error_codes::INTERNAL_ERROR,
                    message,
                    data: None,
                }),
                id,
            },
        }
    }

    fn handle_list_tools(&self, id: Option<serde_json::Value>) -> JsonRpcResponse {
        let tools = self.tools.values().map(|t| t.tool.clone()).collect();
        let result = ToolsListResult {
            tools,
            next_cursor: None,
        };
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::to_value(result).unwrap_or_default()),
            error: None,
            id,
        }
    }

    async fn handle_call_tool(
        &self,
        params: Option<serde_json::Value>,
        id: Option<serde_json::Value>,
    ) -> JsonRpcResponse {
        #[derive(Debug, Deserialize)]
        struct ToolsCallWithAuth {
            name: String,
            #[serde(default)]
            arguments: Option<serde_json::Value>,
            #[serde(default, alias = "authToken")]
            auth_token: Option<String>,
        }

        let parsed: ToolsCallWithAuth = match params {
            Some(p) => match serde_json::from_value(p) {
                Ok(v) => v,
                Err(e) => {
                    return JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: None,
                        error: Some(JsonRpcError {
                            code: error_codes::INVALID_PARAMS,
                            message: format!("Invalid params: {e}"),
                            data: None,
                        }),
                        id,
                    };
                }
            },
            None => {
                return JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                    id,
                };
            }
        };

        let Some(entry) = self.tools.get(&parsed.name) else {
            return JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: mcp_error_codes::TOOL_NOT_FOUND,
                    message: format!("Tool not found: {}", parsed.name),
                    data: None,
                }),
                id,
            };
        };

        // Optional auth validation (used by some tools such as haptics).
        if entry.requires_auth {
            let Some(token) = parsed.auth_token.as_deref() else {
                return JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: "Authentication required: missing auth_token in request params"
                            .to_string(),
                        data: None,
                    }),
                    id,
                };
            };
            match get_mcp().validate_token(token).await {
                Ok(true) => {}
                Ok(false) => {
                    return JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: None,
                        error: Some(JsonRpcError {
                            code: error_codes::INVALID_PARAMS,
                            message: "Invalid or expired authentication token".to_string(),
                            data: None,
                        }),
                        id,
                    };
                }
                Err(e) => {
                    return JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: None,
                        error: Some(Self::to_jsonrpc_error(e)),
                        id,
                    };
                }
            }
        }

        match entry
            .handler
            .call(parsed.arguments, parsed.auth_token.as_deref(), self.ctx())
            .await
        {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::to_value(result).unwrap_or_default()),
                error: None,
                id,
            },
            Err(e) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(Self::to_jsonrpc_error(e)),
                id,
            },
        }
    }

    fn handle_list_resources(&self, id: Option<serde_json::Value>) -> JsonRpcResponse {
        let resources = self
            .resources
            .values()
            .map(|r| r.resource.clone())
            .collect();
        let result = ResourcesListResult {
            resources,
            next_cursor: None,
        };
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::to_value(result).unwrap_or_default()),
            error: None,
            id,
        }
    }

    async fn handle_read_resource(
        &self,
        params: Option<serde_json::Value>,
        id: Option<serde_json::Value>,
    ) -> JsonRpcResponse {
        let parsed: ResourcesReadParams = match params {
            Some(p) => match serde_json::from_value(p) {
                Ok(v) => v,
                Err(e) => {
                    return JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: None,
                        error: Some(JsonRpcError {
                            code: error_codes::INVALID_PARAMS,
                            message: format!("Invalid params: {e}"),
                            data: None,
                        }),
                        id,
                    };
                }
            },
            None => {
                return JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                    id,
                };
            }
        };

        let Some(entry) = self.resources.get(&parsed.uri) else {
            return JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: mcp_error_codes::RESOURCE_NOT_FOUND,
                    message: format!("Resource not found: {}", parsed.uri),
                    data: None,
                }),
                id,
            };
        };

        match entry.handler.read(&parsed.uri, self.ctx()).await {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::to_value(result).unwrap_or_default()),
                error: None,
                id,
            },
            Err(e) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(Self::to_jsonrpc_error(e)),
                id,
            },
        }
    }

    fn handle_list_prompts(&self, id: Option<serde_json::Value>) -> JsonRpcResponse {
        let result = super::types::PromptsListResult {
            prompts: self.prompts.list(),
            next_cursor: None,
        };
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::to_value(result).unwrap_or_default()),
            error: None,
            id,
        }
    }

    fn handle_get_prompt(
        &self,
        params: Option<serde_json::Value>,
        id: Option<serde_json::Value>,
    ) -> JsonRpcResponse {
        let parsed: PromptsGetParams = match params {
            Some(p) => match serde_json::from_value(p) {
                Ok(v) => v,
                Err(e) => {
                    return JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: None,
                        error: Some(JsonRpcError {
                            code: error_codes::INVALID_PARAMS,
                            message: format!("Invalid params: {e}"),
                            data: None,
                        }),
                        id,
                    };
                }
            },
            None => {
                return JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: "Missing params".to_string(),
                        data: None,
                    }),
                    id,
                };
            }
        };

        match self.prompts.get(&parsed.name, parsed.arguments.as_ref()) {
            Some(result) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::to_value(result).unwrap_or_default()),
                error: None,
                id,
            },
            None => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: mcp_error_codes::PROMPT_NOT_FOUND,
                    message: format!("Prompt not found: {}", parsed.name),
                    data: None,
                }),
                id,
            },
        }
    }

    fn handle_cancelled(
        &self,
        params: Option<serde_json::Value>,
        id: Option<serde_json::Value>,
    ) -> JsonRpcResponse {
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CancelParams {
            request_id: String,
            #[serde(default)]
            reason: Option<String>,
        }

        if let Some(p) = params
            && let Ok(parsed) = serde_json::from_value::<CancelParams>(p)
        {
            self.progress
                .cancel_operation(&parsed.request_id, parsed.reason.clone());
            self.logger
                .info(format!("Request {} cancelled", parsed.request_id));
        }

        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::json!({})),
            error: None,
            id,
        }
    }

    fn to_jsonrpc_error(err: AppError) -> JsonRpcError {
        // Conservative mapping: many errors are represented as INTERNAL_ERROR.
        let (code, message) = match err {
            AppError::InvalidInput(msg) => (error_codes::INVALID_PARAMS, msg),
            AppError::NotFound(msg) => (mcp_error_codes::RESOURCE_NOT_FOUND, msg),
            AppError::PermissionDenied(msg) => (error_codes::INVALID_PARAMS, msg),
            other => (error_codes::INTERNAL_ERROR, other.to_string()),
        };
        JsonRpcError {
            code,
            message,
            data: None,
        }
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::ToolResultContent;

    struct EchoTool;

    #[async_trait]
    impl McpToolHandler for EchoTool {
        async fn call(
            &self,
            arguments: Option<serde_json::Value>,
            _auth_token: Option<&str>,
            _ctx: McpRequestContext,
        ) -> Result<ToolsCallResult> {
            Ok(ToolsCallResult {
                content: vec![ToolResultContent::Text {
                    text: arguments
                        .unwrap_or_else(|| serde_json::json!({}))
                        .to_string(),
                }],
                is_error: None,
            })
        }
    }

    #[tokio::test]
    async fn tools_list_and_call_round_trip() {
        let mut server = McpServer::new();
        server.register_tool(
            Tool {
                name: "echo".to_string(),
                description: Some("Echo input".to_string()),
                input_schema: serde_json::json!({"type":"object"}),
                annotations: None,
            },
            false,
            Arc::new(EchoTool),
        );

        let list = server
            .handle_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                method: "tools/list".to_string(),
                params: None,
                id: Some(serde_json::json!(1)),
            })
            .await;
        assert!(list.error.is_none());
        assert!(list.result.is_some());

        let call = server
            .handle_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                method: "tools/call".to_string(),
                params: Some(serde_json::json!({"name":"echo","arguments":{"a":1}})),
                id: Some(serde_json::json!(2)),
            })
            .await;
        assert!(call.error.is_none());
        assert!(call.result.is_some());
    }
}
