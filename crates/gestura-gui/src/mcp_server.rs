//! MCP server implementation with JSON-RPC transport
//! Provides MCP protocol compliance and tool execution

use crate::AppError;
use crate::haptics::{HapticAuthToken, HapticInterface};
use crate::security::McpToken;
use gestura_core::mcp::{McpIntegrator, get_mcp};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// JSON-RPC request structure
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<serde_json::Value>,
    pub id: Option<serde_json::Value>,
}

/// JSON-RPC response structure
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Option<serde_json::Value>,
}

/// JSON-RPC error structure
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// MCP tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub requires_auth: bool,
}

/// MCP resource definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub mime_type: Option<String>,
}

/// MCP server implementation
pub struct McpServer {
    tools: HashMap<String, McpTool>,
    resources: HashMap<String, McpResource>,
    haptic_interface: Arc<dyn HapticInterface>,
    #[allow(dead_code)]
    auth_tokens: HashMap<String, McpToken>,
}

impl McpServer {
    /// Create a new MCP server
    pub fn new(haptic_interface: Arc<dyn HapticInterface>) -> Self {
        let mut server = Self {
            tools: HashMap::new(),
            resources: HashMap::new(),
            haptic_interface,
            auth_tokens: HashMap::new(),
        };

        // Register built-in tools
        server.register_haptic_tools();
        server.register_ring_resources();

        server
    }

    /// Register haptic feedback tools
    fn register_haptic_tools(&mut self) {
        let haptic_tool = McpTool {
            name: "send_haptic".to_string(),
            description: "Send haptic feedback to the ring".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "enum": ["click", "pulse", "ramp"]},
                    "intensity": {"type": "number", "minimum": 0.0, "maximum": 1.0},
                    "duration_ms": {"type": "integer", "minimum": 1, "maximum": 5000}
                },
                "required": ["pattern", "intensity", "duration_ms"]
            }),
            requires_auth: true,
        };
        self.tools.insert("send_haptic".to_string(), haptic_tool);
    }

    /// Register ring as MCP resource
    fn register_ring_resources(&mut self) {
        let ring_resource = McpResource {
            uri: "ring://haptic-harmony/main".to_string(),
            name: "Haptic Harmony Ring".to_string(),
            description: "Primary haptic feedback device".to_string(),
            mime_type: Some("application/x-haptic-device".to_string()),
        };
        self.resources
            .insert("ring://haptic-harmony/main".to_string(), ring_resource);
    }

    /// Handle JSON-RPC request with audit logging
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        // Audit the request
        let telemetry = crate::telemetry::get_telemetry_manager().await;
        telemetry
            .increment_counter(
                "mcp.requests.total",
                1.0,
                std::collections::HashMap::from([("method".to_string(), request.method.clone())]),
            )
            .await;

        let timer = crate::telemetry::start_timer("mcp.request.duration")
            .with_tag("method".to_string(), request.method.clone());

        let response = match request.method.as_str() {
            "tools/list" => self.handle_list_tools(request.id).await,
            "tools/call" => self.handle_call_tool(request.params, request.id).await,
            "resources/list" => self.handle_list_resources(request.id).await,
            "resources/read" => self.handle_read_resource(request.params, request.id).await,
            _ => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: "Method not found".to_string(),
                    data: None,
                }),
                id: request.id,
            },
        };

        // Complete the timer
        timer.finish().await;

        // Audit response status
        if response.error.is_some() {
            telemetry
                .increment_counter(
                    "mcp.requests.errors",
                    1.0,
                    std::collections::HashMap::from([("method".to_string(), request.method)]),
                )
                .await;
        } else {
            telemetry
                .increment_counter(
                    "mcp.requests.success",
                    1.0,
                    std::collections::HashMap::from([("method".to_string(), request.method)]),
                )
                .await;
        }

        response
    }

    /// Handle tools/list request
    async fn handle_list_tools(&self, id: Option<serde_json::Value>) -> JsonRpcResponse {
        let tools: Vec<&McpTool> = self.tools.values().collect();
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::to_value(tools).unwrap_or_default()),
            error: None,
            id,
        }
    }

    /// Handle tools/call request
    async fn handle_call_tool(
        &self,
        params: Option<serde_json::Value>,
        id: Option<serde_json::Value>,
    ) -> JsonRpcResponse {
        // Extract tool name, arguments, and auth token
        let (tool_name, args, auth_token) = match params {
            Some(serde_json::Value::Object(map)) => {
                let name = map
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let arguments = map.get("arguments").cloned().unwrap_or_default();
                // Extract auth token from params (MCP clients should include this)
                let token = map
                    .get("auth_token")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                (name, arguments, token)
            }
            _ => {
                return JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Invalid params".to_string(),
                        data: None,
                    }),
                    id,
                };
            }
        };

        // Execute tool with auth token
        match self.execute_tool(&tool_name, args, auth_token.as_deref()).await {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(result),
                error: None,
                id,
            },
            Err(e) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: -32603,
                    message: e.to_string(),
                    data: None,
                }),
                id,
            },
        }
    }

    /// Execute a specific tool with optional auth token
    async fn execute_tool(
        &self,
        tool_name: &str,
        args: serde_json::Value,
        auth_token: Option<&str>,
    ) -> Result<serde_json::Value, AppError> {
        // Check if tool requires authentication
        if let Some(tool) = self.tools.get(tool_name) {
            if tool.requires_auth {
                // Validate auth token using gestura-core MCP module
                let token = auth_token.ok_or_else(|| {
                    AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Authentication required: missing auth_token in request params",
                    ))
                })?;

                let mcp = get_mcp();
                let is_valid = mcp.validate_token(token).await?;
                if !is_valid {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Invalid or expired authentication token",
                    )));
                }

                tracing::info!("MCP tool '{}' authenticated successfully", tool_name);
            }
        }

        match tool_name {
            "send_haptic" => self.execute_haptic_tool(args, auth_token).await,
            _ => Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Tool not found",
            ))),
        }
    }

    /// Execute haptic feedback tool with sandboxing and proper auth validation
    async fn execute_haptic_tool(
        &self,
        args: serde_json::Value,
        auth_token: Option<&str>,
    ) -> Result<serde_json::Value, AppError> {
        // Validate tool execution parameters
        self.validate_tool_execution("send_haptic", &args)
            .await
            .map_err(|e| AppError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)))?;

        // Parse haptic request
        let pattern_str = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("click");
        let intensity = args
            .get("intensity")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5) as f32;
        let duration_ms = args
            .get("duration_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as u32;

        let pattern = match pattern_str {
            "click" => crate::haptics::HapticPattern::Click,
            "pulse" => crate::haptics::HapticPattern::Pulse,
            "ramp" => crate::haptics::HapticPattern::Ramp,
            "heartbeat" => crate::haptics::HapticPattern::Heartbeat,
            "notification" => crate::haptics::HapticPattern::Notification,
            "alert" => crate::haptics::HapticPattern::Alert,
            _ => crate::haptics::HapticPattern::Click,
        };

        let request = crate::haptics::HapticRequest {
            pattern,
            intensity,
            duration_ms,
            repeat_count: 0,
            repeat_delay_ms: 0,
        };

        // Execute with sandbox constraints (timeout)
        let sandbox_config = crate::sandbox::create_default_sandbox("mcp-tool");
        let timeout_duration = std::time::Duration::from_secs(sandbox_config.max_cpu_time_secs);

        // Validate haptic permission using MCP dual-auth
        // The token was already validated in execute_tool, now check haptic-specific permission
        let mcp = get_mcp();
        if let Some(token) = auth_token {
            let has_haptic_permission = mcp.authenticate_haptic(token).await?;
            if !has_haptic_permission {
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Haptic permission not granted for this token. Request haptic permission first.",
                )));
            }
        }

        // Create auth token for haptic interface (validated above)
        let haptic_auth = HapticAuthToken(auth_token.unwrap_or("validated").to_string());

        match tokio::time::timeout(
            timeout_duration,
            self.haptic_interface.send(&haptic_auth, &request),
        )
        .await
        {
            Ok(Ok(_)) => Ok(serde_json::json!({
                "status": "success",
                "message": "Haptic feedback sent",
                "pattern": pattern_str,
                "intensity": intensity,
                "duration_ms": duration_ms
            })),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Tool execution timeout",
            ))),
        }
    }

    /// Validate tool execution parameters
    async fn validate_tool_execution(
        &self,
        name: &str,
        arguments: &serde_json::Value,
    ) -> Result<(), String> {
        match name {
            "send_haptic" => {
                // Validate haptic parameters
                let intensity = arguments
                    .get("intensity")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.5);

                if !(0.0..=1.0).contains(&intensity) {
                    return Err("Intensity must be between 0.0 and 1.0".to_string());
                }

                let duration_ms = arguments
                    .get("duration_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(100);

                if duration_ms > 5000 {
                    return Err("Duration cannot exceed 5000ms".to_string());
                }

                let pattern = arguments
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("click");

                if ![
                    "click",
                    "pulse",
                    "ramp",
                    "heartbeat",
                    "notification",
                    "alert",
                ]
                .contains(&pattern)
                {
                    return Err("Invalid haptic pattern".to_string());
                }

                Ok(())
            }
            _ => Err(format!("Unknown tool: {}", name)),
        }
    }

    /// Handle resources/list request
    async fn handle_list_resources(&self, id: Option<serde_json::Value>) -> JsonRpcResponse {
        let resources: Vec<&McpResource> = self.resources.values().collect();
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::to_value(resources).unwrap_or_default()),
            error: None,
            id,
        }
    }

    /// Handle resources/read request
    async fn handle_read_resource(
        &self,
        params: Option<serde_json::Value>,
        id: Option<serde_json::Value>,
    ) -> JsonRpcResponse {
        let uri = match params
            .as_ref()
            .and_then(|p| p.get("uri"))
            .and_then(|v| v.as_str())
        {
            Some(uri) => uri.to_string(),
            None => {
                return JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: "Missing uri parameter".to_string(),
                        data: None,
                    }),
                    id,
                };
            }
        };

        // Return resource data
        if let Some(resource) = self.resources.get(&uri) {
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::to_value(resource).unwrap_or_default()),
                error: None,
                id,
            }
        } else {
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: -32603,
                    message: "Resource not found".to_string(),
                    data: None,
                }),
                id,
            }
        }
    }

    /// Start STDIO transport for MCP
    pub async fn start_stdio_transport(&self) -> Result<(), crate::AppError> {
        tracing::info!("Starting MCP STDIO transport");

        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    tracing::info!("STDIO transport closed");
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    // Parse JSON-RPC request
                    match serde_json::from_str::<JsonRpcRequest>(trimmed) {
                        Ok(request) => {
                            let response = self.handle_request(request).await;
                            let response_json = serde_json::to_string(&response)
                                .unwrap_or_else(|_| r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"Internal error"},"id":null}"#.to_string());

                            if let Err(e) = stdout.write_all(response_json.as_bytes()).await {
                                tracing::error!("Failed to write STDIO response: {}", e);
                                break;
                            }
                            if let Err(e) = stdout.write_all(b"\n").await {
                                tracing::error!("Failed to write STDIO newline: {}", e);
                                break;
                            }
                            if let Err(e) = stdout.flush().await {
                                tracing::error!("Failed to flush STDIO: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to parse JSON-RPC request: {}", e);
                            let error_response = JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                result: None,
                                error: Some(JsonRpcError {
                                    code: -32700,
                                    message: "Parse error".to_string(),
                                    data: None,
                                }),
                                id: None,
                            };
                            let error_json = serde_json::to_string(&error_response).unwrap();
                            let _ = stdout.write_all(error_json.as_bytes()).await;
                            let _ = stdout.write_all(b"\n").await;
                            let _ = stdout.flush().await;
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("STDIO read error: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Start HTTP server for MCP
    pub async fn start_http_server(&self, port: u16) -> Result<(), crate::AppError> {
        tracing::info!("Starting MCP HTTP server on port {}", port);
        // TODO: Implement actual HTTP server with warp or axum
        Ok(())
    }
}
