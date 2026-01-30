//! MCP server thin adapter for Gestura GUI
//!
//! This module provides a thin transport layer over gestura-core's MCP server.
//! All JSON-RPC routing and protocol logic lives in gestura-core. This adapter:
//! - Registers GUI-specific tools (haptic feedback)
//! - Provides transport implementations (STDIO, HTTP/SSE)
//! - Adds telemetry hooks

use crate::haptics::{HapticAuthToken, HapticInterface, HapticPattern, HapticRequest};
use async_trait::async_trait;
use gestura_core::McpServer as CoreMcpServer;
use gestura_core::mcp::{
    McpIntegrator, McpNotification, McpRequestContext, McpResourceHandler, McpToolHandler,
    ProgressTracker, Resource, ResourceContent, ResourcesReadResult, SessionManager, Tool,
    ToolResultContent, ToolsCallResult, get_mcp,
};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// Re-export core types for external compatibility
pub use gestura_core::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};

/// GUI MCP server wrapping core's transport-agnostic implementation.
///
/// Registers GUI-specific tools (haptics) and provides transport layers.
pub struct McpServer {
    /// Core MCP server handling all protocol logic
    core: CoreMcpServer,
}

impl McpServer {
    /// Create a new MCP server with GUI-specific haptic tool registered.
    pub fn new(haptic_interface: Arc<dyn HapticInterface>) -> Self {
        let mut core = CoreMcpServer::new();

        // Register haptic tool
        let haptic_tool = Tool {
            name: "send_haptic".to_string(),
            description: Some("Send haptic feedback to the ring".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "enum": ["click", "pulse", "ramp", "heartbeat", "notification", "alert"]},
                    "intensity": {"type": "number", "minimum": 0.0, "maximum": 1.0},
                    "duration_ms": {"type": "integer", "minimum": 1, "maximum": 5000}
                },
                "required": ["pattern", "intensity", "duration_ms"]
            }),
            annotations: None,
        };
        let handler = Arc::new(HapticToolHandler {
            haptic_interface: haptic_interface.clone(),
        });
        core.register_tool(haptic_tool, true, handler);

        // Register ring resource
        let ring_resource = Resource {
            uri: "ring://haptic-harmony/main".to_string(),
            name: "Haptic Harmony Ring".to_string(),
            description: Some("Primary haptic feedback device".to_string()),
            mime_type: Some("application/x-haptic-device".to_string()),
            annotations: None,
        };
        core.register_resource(ring_resource, Arc::new(RingResourceHandler));

        Self { core }
    }

    /// Get the session manager for lifecycle operations.
    pub fn session(&self) -> &Arc<SessionManager> {
        self.core.session()
    }

    /// Get the progress tracker.
    pub fn progress_tracker(&self) -> &Arc<ProgressTracker> {
        self.core.progress_tracker()
    }

    /// Subscribe to MCP notifications.
    pub fn subscribe_notifications(&self) -> tokio::sync::broadcast::Receiver<McpNotification> {
        self.core.subscribe_notifications()
    }

    /// Handle JSON-RPC request with telemetry.
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        // Telemetry: count request
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
        let method = request.method.clone();

        // Delegate to core server
        let response = self.core.handle_request(request).await;

        timer.finish().await;

        // Telemetry: count success/error
        let counter_name = if response.error.is_some() {
            "mcp.requests.errors"
        } else {
            "mcp.requests.success"
        };
        telemetry
            .increment_counter(
                counter_name,
                1.0,
                std::collections::HashMap::from([("method".to_string(), method)]),
            )
            .await;

        response
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

    /// Start HTTP server for MCP with SSE streaming support
    pub async fn start_http_server(self: Arc<Self>, port: u16) -> Result<(), crate::AppError> {
        use axum::{
            Router,
            routing::{get, post},
        };
        use tower_http::cors::{Any, CorsLayer};

        tracing::info!("Starting MCP HTTP server on port {}", port);

        // Clone self for handlers
        let server = self.clone();

        // Configure CORS for cross-origin MCP clients
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
                axum::http::header::ACCEPT,
            ])
            .expose_headers([axum::http::header::CONTENT_TYPE]);

        // Build router with CORS
        let app = Router::new()
            .route("/mcp", post(mcp_handler))
            .route("/mcp/sse", get(sse_handler))
            .route("/health", get(health_handler))
            .layer(cors)
            .with_state(server);

        // Bind and serve
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(crate::AppError::Io)?;

        tracing::info!("MCP HTTP server listening on http://{}", addr);

        axum::serve(listener, app).await.map_err(|e| {
            crate::AppError::Io(std::io::Error::other(format!("Server error: {e}")))
        })?;

        Ok(())
    }
}

// ============================================================================
// Tool and Resource Handlers (implementing core traits)
// ============================================================================

/// Handler for the haptic feedback tool.
///
/// Implements [`McpToolHandler`] to execute haptic commands on the ring device.
struct HapticToolHandler {
    haptic_interface: Arc<dyn HapticInterface>,
}

#[async_trait]
impl McpToolHandler for HapticToolHandler {
    async fn call(
        &self,
        arguments: Option<serde_json::Value>,
        auth_token: Option<&str>,
        _ctx: McpRequestContext,
    ) -> gestura_core::Result<ToolsCallResult> {
        let args = arguments.unwrap_or_default();

        // Parse haptic parameters
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

        // Validate parameters
        if !(0.0..=1.0).contains(&intensity) {
            return Ok(ToolsCallResult {
                content: vec![ToolResultContent::Text {
                    text: "Error: intensity must be between 0.0 and 1.0".to_string(),
                }],
                is_error: Some(true),
            });
        }
        if duration_ms > 5000 {
            return Ok(ToolsCallResult {
                content: vec![ToolResultContent::Text {
                    text: "Error: duration_ms cannot exceed 5000".to_string(),
                }],
                is_error: Some(true),
            });
        }

        let pattern = match pattern_str {
            "click" => HapticPattern::Click,
            "pulse" => HapticPattern::Pulse,
            "ramp" => HapticPattern::Ramp,
            "heartbeat" => HapticPattern::Heartbeat,
            "notification" => HapticPattern::Notification,
            "alert" => HapticPattern::Alert,
            _ => {
                return Ok(ToolsCallResult {
                    content: vec![ToolResultContent::Text {
                        text: format!("Error: invalid pattern '{}'", pattern_str),
                    }],
                    is_error: Some(true),
                });
            }
        };

        // Validate haptic permission using MCP dual-auth
        let mcp = get_mcp();
        if let Some(token) = auth_token {
            let has_haptic_permission = mcp
                .authenticate_haptic(token)
                .await
                .map_err(|e| gestura_core::AppError::Io(std::io::Error::other(e.to_string())))?;
            if !has_haptic_permission {
                return Ok(ToolsCallResult {
                    content: vec![ToolResultContent::Text {
                        text: "Error: haptic permission not granted for this token".to_string(),
                    }],
                    is_error: Some(true),
                });
            }
        }

        let request = HapticRequest {
            pattern,
            intensity,
            duration_ms,
            repeat_count: 0,
            repeat_delay_ms: 0,
        };

        // Execute with sandbox timeout
        let sandbox_config = crate::sandbox::create_default_sandbox("mcp-tool");
        let timeout_duration = std::time::Duration::from_secs(sandbox_config.max_cpu_time_secs);

        let haptic_auth = HapticAuthToken(auth_token.unwrap_or("validated").to_string());

        match tokio::time::timeout(
            timeout_duration,
            self.haptic_interface.send(&haptic_auth, &request),
        )
        .await
        {
            Ok(Ok(_)) => Ok(ToolsCallResult {
                content: vec![ToolResultContent::Text {
                    text: serde_json::json!({
                        "status": "success",
                        "message": "Haptic feedback sent",
                        "pattern": pattern_str,
                        "intensity": intensity,
                        "duration_ms": duration_ms
                    })
                    .to_string(),
                }],
                is_error: None,
            }),
            Ok(Err(e)) => Ok(ToolsCallResult {
                content: vec![ToolResultContent::Text {
                    text: format!("Error: {}", e),
                }],
                is_error: Some(true),
            }),
            Err(_) => Ok(ToolsCallResult {
                content: vec![ToolResultContent::Text {
                    text: "Error: tool execution timeout".to_string(),
                }],
                is_error: Some(true),
            }),
        }
    }
}

/// Handler for the ring resource.
///
/// Implements [`McpResourceHandler`] to return ring device information.
struct RingResourceHandler;

#[async_trait]
impl McpResourceHandler for RingResourceHandler {
    async fn read(
        &self,
        uri: &str,
        _ctx: McpRequestContext,
    ) -> gestura_core::Result<ResourcesReadResult> {
        Ok(ResourcesReadResult {
            contents: vec![ResourceContent {
                uri: uri.to_string(),
                mime_type: Some("application/json".to_string()),
                text: Some(
                    serde_json::json!({
                        "device": "Haptic Harmony Ring",
                        "status": "connected",
                        "capabilities": ["haptic_feedback", "vibration_patterns"],
                        "battery_level": 85
                    })
                    .to_string(),
                ),
                blob: None,
            }],
        })
    }
}

// ============================================================================
// HTTP Handlers
// ============================================================================

/// Health check endpoint
async fn health_handler() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "status": "ok",
        "service": "gestura-mcp",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// MCP JSON-RPC handler
async fn mcp_handler(
    axum::extract::State(server): axum::extract::State<Arc<McpServer>>,
    axum::Json(request): axum::Json<JsonRpcRequest>,
) -> impl axum::response::IntoResponse {
    let response = server.handle_request(request).await;
    axum::Json(response)
}

/// SSE handler for streaming notifications
async fn sse_handler(
    axum::extract::State(server): axum::extract::State<Arc<McpServer>>,
) -> axum::response::sse::Sse<
    impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    use futures::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;

    let receiver = server.subscribe_notifications();
    let stream = BroadcastStream::new(receiver).filter_map(|result| async move {
        match result {
            Ok(notification) => {
                let data = serde_json::to_string(&notification).ok()?;
                Some(Ok(axum::response::sse::Event::default().data(data)))
            }
            Err(_) => None,
        }
    });

    axum::response::sse::Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}
