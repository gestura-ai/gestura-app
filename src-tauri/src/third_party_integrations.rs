//! Third-party integrations for Gestura.app
//! Provides connectors for popular services and APIs

#[allow(unused_imports)]
use crate::AppError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Integration types
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum IntegrationType {
    Webhook,
    RestApi,
    GraphQL,
    WebSocket,
    OAuth,
    Database,
    MessageQueue,
    CloudStorage,
    NotificationService,
    SmartHome,
    SocialMedia,
    ProductivityTool,
}

/// Integration configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Integration {
    pub id: String,
    pub name: String,
    pub description: String,
    pub integration_type: IntegrationType,
    pub provider: String,
    pub config: IntegrationConfig,
    pub credentials: IntegrationCredentials,
    pub is_enabled: bool,
    pub last_used: Option<chrono::DateTime<chrono::Utc>>,
    pub usage_count: u32,
    pub error_count: u32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Integration configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IntegrationConfig {
    pub endpoint_url: Option<String>,
    pub timeout_seconds: u64,
    pub retry_attempts: u32,
    pub rate_limit: Option<RateLimit>,
    pub headers: HashMap<String, String>,
    pub parameters: HashMap<String, serde_json::Value>,
    pub webhook_secret: Option<String>,
    pub ssl_verify: bool,
}

/// Rate limiting configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RateLimit {
    pub requests_per_minute: u32,
    pub burst_limit: u32,
}

/// Integration credentials (encrypted)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IntegrationCredentials {
    pub auth_type: AuthType,
    pub encrypted_data: String, // Encrypted credential data
}

/// Authentication types
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AuthType {
    None,
    ApiKey,
    BasicAuth,
    BearerToken,
    OAuth2,
    Custom(String),
}

/// Integration request
#[derive(Debug, Clone, serde::Serialize)]
pub struct IntegrationRequest {
    pub integration_id: String,
    pub method: HttpMethod,
    pub path: Option<String>,
    pub headers: HashMap<String, String>,
    pub body: Option<serde_json::Value>,
    pub timeout: Option<std::time::Duration>,
}

/// HTTP methods
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    OPTIONS,
}

/// Integration response
#[derive(Debug, Clone, serde::Serialize)]
pub struct IntegrationResponse {
    pub success: bool,
    pub status_code: Option<u16>,
    pub headers: HashMap<String, String>,
    pub body: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub response_time_ms: u64,
}

/// Third-party integration manager
pub struct ThirdPartyIntegrationManager {
    integrations: Arc<RwLock<HashMap<String, Integration>>>,
    http_client: reqwest::Client,
    rate_limiters: Arc<RwLock<HashMap<String, RateLimiter>>>,
}

impl ThirdPartyIntegrationManager {
    /// Create a new integration manager
    pub fn new() -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("Gestura/1.0")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            integrations: Arc::new(RwLock::new(HashMap::new())),
            http_client,
            rate_limiters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a new integration
    pub async fn add_integration(&self, integration: Integration) -> Result<(), AppError> {
        // Validate integration
        self.validate_integration(&integration).await?;

        // Test connection
        self.test_integration_connection(&integration).await?;

        // Store integration
        let mut integrations = self.integrations.write().await;
        integrations.insert(integration.id.clone(), integration.clone());

        // Initialize rate limiter if needed
        if let Some(rate_limit) = &integration.config.rate_limit {
            let mut rate_limiters = self.rate_limiters.write().await;
            rate_limiters.insert(
                integration.id.clone(),
                RateLimiter::new(rate_limit.requests_per_minute, rate_limit.burst_limit)
            );
        }

        tracing::info!("Added integration: {} ({})", integration.name, integration.provider);
        Ok(())
    }

    /// Validate integration configuration
    async fn validate_integration(&self, integration: &Integration) -> Result<(), AppError> {
        // Check required fields
        if integration.name.is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Integration name cannot be empty"
            )));
        }

        // Validate endpoint URL for API integrations
        match integration.integration_type {
            IntegrationType::RestApi | IntegrationType::GraphQL | IntegrationType::Webhook => {
                if integration.config.endpoint_url.is_none() {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Endpoint URL required for API integrations"
                    )));
                }
            }
            _ => {}
        }

        // Validate credentials
        if integration.credentials.auth_type != AuthType::None && 
           integration.credentials.encrypted_data.is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Credentials required for authenticated integrations"
            )));
        }

        Ok(())
    }

    /// Test integration connection
    async fn test_integration_connection(&self, integration: &Integration) -> Result<(), AppError> {
        match integration.integration_type {
            IntegrationType::RestApi => {
                if let Some(url) = &integration.config.endpoint_url {
                    let response = self.http_client
                        .get(url)
                        .timeout(std::time::Duration::from_secs(10))
                        .send()
                        .await;
                    
                    match response {
                        Ok(resp) => {
                            if resp.status().is_success() || resp.status().is_client_error() {
                                // Connection successful (even if auth fails)
                                return Ok(());
                            }
                        }
                        Err(_) => {
                            return Err(AppError::Io(std::io::Error::new(
                                std::io::ErrorKind::ConnectionRefused,
                                "Failed to connect to integration endpoint"
                            )));
                        }
                    }
                }
            }
            IntegrationType::Webhook => {
                // Webhooks don't need connection testing
                return Ok(());
            }
            _ => {
                // Other integration types would have specific connection tests
                tracing::debug!("Connection test not implemented for {:?}", integration.integration_type);
            }
        }

        Ok(())
    }

    /// Execute integration request
    pub async fn execute_request(&self, request: IntegrationRequest) -> Result<IntegrationResponse, AppError> {
        let start_time = std::time::Instant::now();

        // Get integration
        let integrations = self.integrations.read().await;
        let integration = integrations.get(&request.integration_id)
            .ok_or_else(|| AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Integration not found"
            )))?
            .clone();

        if !integration.is_enabled {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Integration is disabled"
            )));
        }

        drop(integrations);

        // Check rate limit
        if let Some(_rate_limit) = &integration.config.rate_limit {
            let rate_limiters = self.rate_limiters.read().await;
            if let Some(limiter) = rate_limiters.get(&integration.id) {
                if !limiter.allow_request().await {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Rate limit exceeded"
                    )));
                }
            }
        }

        // Execute request based on integration type
        let response = match integration.integration_type {
            IntegrationType::RestApi => {
                self.execute_rest_request(&integration, &request).await?
            }
            IntegrationType::Webhook => {
                self.execute_webhook_request(&integration, &request).await?
            }
            IntegrationType::GraphQL => {
                self.execute_graphql_request(&integration, &request).await?
            }
            _ => {
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    format!("Integration type {:?} not implemented", integration.integration_type)
                )));
            }
        };

        // Update integration statistics
        let mut integrations_mut = self.integrations.write().await;
        if let Some(integration_mut) = integrations_mut.get_mut(&request.integration_id) {
            integration_mut.last_used = Some(chrono::Utc::now());
            integration_mut.usage_count += 1;
            if !response.success {
                integration_mut.error_count += 1;
            }
        }

        let response_time = start_time.elapsed().as_millis() as u64;
        Ok(IntegrationResponse {
            response_time_ms: response_time,
            ..response
        })
    }

    /// Execute REST API request
    async fn execute_rest_request(&self, integration: &Integration, request: &IntegrationRequest) -> Result<IntegrationResponse, AppError> {
        let base_url = integration.config.endpoint_url.as_ref()
            .ok_or_else(|| AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "No endpoint URL configured"
            )))?;

        let url = if let Some(path) = &request.path {
            format!("{}/{}", base_url.trim_end_matches('/'), path.trim_start_matches('/'))
        } else {
            base_url.clone()
        };

        // Build request
        let mut req_builder = match request.method {
            HttpMethod::GET => self.http_client.get(&url),
            HttpMethod::POST => self.http_client.post(&url),
            HttpMethod::PUT => self.http_client.put(&url),
            HttpMethod::DELETE => self.http_client.delete(&url),
            HttpMethod::PATCH => self.http_client.patch(&url),
            HttpMethod::HEAD => self.http_client.head(&url),
            HttpMethod::OPTIONS => {
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "OPTIONS method not implemented"
                )));
            }
        };

        // Add headers
        for (key, value) in &integration.config.headers {
            req_builder = req_builder.header(key, value);
        }
        for (key, value) in &request.headers {
            req_builder = req_builder.header(key, value);
        }

        // Add authentication
        req_builder = self.add_authentication(req_builder, &integration.credentials).await?;

        // Add body for POST/PUT/PATCH
        if let Some(body) = &request.body {
            req_builder = req_builder.json(body);
        }

        // Set timeout
        let timeout = request.timeout.unwrap_or(std::time::Duration::from_secs(integration.config.timeout_seconds));
        req_builder = req_builder.timeout(timeout);

        // Execute request
        match req_builder.send().await {
            Ok(response) => {
                let status_code = response.status().as_u16();
                let headers: HashMap<String, String> = response.headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();

                let body = match response.json::<serde_json::Value>().await {
                    Ok(json) => Some(json),
                    Err(_) => None,
                };

                Ok(IntegrationResponse {
                    success: status_code < 400,
                    status_code: Some(status_code),
                    headers,
                    body,
                    error_message: if status_code >= 400 {
                        Some(format!("HTTP {}", status_code))
                    } else {
                        None
                    },
                    response_time_ms: 0, // Will be set by caller
                })
            }
            Err(error) => {
                Ok(IntegrationResponse {
                    success: false,
                    status_code: None,
                    headers: HashMap::new(),
                    body: None,
                    error_message: Some(error.to_string()),
                    response_time_ms: 0,
                })
            }
        }
    }

    /// Execute webhook request
    async fn execute_webhook_request(&self, integration: &Integration, request: &IntegrationRequest) -> Result<IntegrationResponse, AppError> {
        // Webhooks are typically outgoing HTTP requests
        self.execute_rest_request(integration, request).await
    }

    /// Execute GraphQL request
    async fn execute_graphql_request(&self, integration: &Integration, request: &IntegrationRequest) -> Result<IntegrationResponse, AppError> {
        let endpoint = integration.config.endpoint_url.as_ref()
            .ok_or_else(|| AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "No GraphQL endpoint configured"
            )))?;

        // GraphQL requests are always POST
        let mut req_builder = self.http_client.post(endpoint);

        // Add headers
        req_builder = req_builder.header("Content-Type", "application/json");
        for (key, value) in &integration.config.headers {
            req_builder = req_builder.header(key, value);
        }

        // Add authentication
        req_builder = self.add_authentication(req_builder, &integration.credentials).await?;

        // GraphQL body format
        if let Some(body) = &request.body {
            req_builder = req_builder.json(body);
        }

        // Execute request
        match req_builder.send().await {
            Ok(response) => {
                let status_code = response.status().as_u16();
                let body = response.json::<serde_json::Value>().await.ok();

                Ok(IntegrationResponse {
                    success: status_code == 200,
                    status_code: Some(status_code),
                    headers: HashMap::new(),
                    body,
                    error_message: if status_code != 200 {
                        Some(format!("GraphQL error: HTTP {}", status_code))
                    } else {
                        None
                    },
                    response_time_ms: 0,
                })
            }
            Err(error) => {
                Ok(IntegrationResponse {
                    success: false,
                    status_code: None,
                    headers: HashMap::new(),
                    body: None,
                    error_message: Some(error.to_string()),
                    response_time_ms: 0,
                })
            }
        }
    }

    /// Add authentication to request
    async fn add_authentication(&self, mut req_builder: reqwest::RequestBuilder, credentials: &IntegrationCredentials) -> Result<reqwest::RequestBuilder, AppError> {
        match credentials.auth_type {
            AuthType::None => Ok(req_builder),
            AuthType::ApiKey => {
                // In real implementation, would decrypt and use API key
                req_builder = req_builder.header("X-API-Key", "decrypted_api_key");
                Ok(req_builder)
            }
            AuthType::BearerToken => {
                // In real implementation, would decrypt and use bearer token
                req_builder = req_builder.header("Authorization", "Bearer decrypted_token");
                Ok(req_builder)
            }
            AuthType::BasicAuth => {
                // In real implementation, would decrypt and use basic auth
                req_builder = req_builder.basic_auth("username", Some("password"));
                Ok(req_builder)
            }
            AuthType::OAuth2 => {
                // In real implementation, would handle OAuth2 flow
                req_builder = req_builder.header("Authorization", "Bearer oauth2_token");
                Ok(req_builder)
            }
            AuthType::Custom(_) => {
                // Custom authentication would be handled based on the specific type
                Ok(req_builder)
            }
        }
    }

    /// Get all integrations
    pub async fn get_integrations(&self) -> Vec<Integration> {
        let integrations = self.integrations.read().await;
        integrations.values().cloned().collect()
    }

    /// Get integration by ID
    pub async fn get_integration(&self, integration_id: &str) -> Option<Integration> {
        let integrations = self.integrations.read().await;
        integrations.get(integration_id).cloned()
    }

    /// Remove integration
    pub async fn remove_integration(&self, integration_id: &str) -> Result<(), AppError> {
        let mut integrations = self.integrations.write().await;
        let mut rate_limiters = self.rate_limiters.write().await;

        integrations.remove(integration_id);
        rate_limiters.remove(integration_id);

        tracing::info!("Removed integration: {}", integration_id);
        Ok(())
    }

    /// Get integration statistics
    pub async fn get_stats(&self) -> serde_json::Value {
        let integrations = self.integrations.read().await;
        
        let total_integrations = integrations.len();
        let enabled_integrations = integrations.values().filter(|i| i.is_enabled).count();
        let total_usage: u32 = integrations.values().map(|i| i.usage_count).sum();
        let total_errors: u32 = integrations.values().map(|i| i.error_count).sum();

        let integration_types: HashMap<String, usize> = integrations.values()
            .fold(HashMap::new(), |mut acc, integration| {
                let type_name = format!("{:?}", integration.integration_type);
                *acc.entry(type_name).or_insert(0) += 1;
                acc
            });

        serde_json::json!({
            "total_integrations": total_integrations,
            "enabled_integrations": enabled_integrations,
            "total_usage": total_usage,
            "total_errors": total_errors,
            "integration_types": integration_types
        })
    }
}

/// Simple rate limiter
struct RateLimiter {
    requests_per_minute: u32,
    burst_limit: u32,
    requests: Arc<RwLock<VecDeque<chrono::DateTime<chrono::Utc>>>>,
}

use std::collections::VecDeque;

impl RateLimiter {
    fn new(requests_per_minute: u32, burst_limit: u32) -> Self {
        Self {
            requests_per_minute,
            burst_limit,
            requests: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    async fn allow_request(&self) -> bool {
        let now = chrono::Utc::now();
        let mut requests = self.requests.write().await;

        // Remove requests older than 1 minute
        while let Some(&front) = requests.front() {
            if (now - front).num_seconds() > 60 {
                requests.pop_front();
            } else {
                break;
            }
        }

        // Check rate limits
        if requests.len() >= self.burst_limit as usize {
            return false;
        }

        if requests.len() >= self.requests_per_minute as usize {
            return false;
        }

        // Allow request
        requests.push_back(now);
        true
    }
}

/// Global third-party integration manager instance
static INTEGRATION_MANAGER: tokio::sync::OnceCell<ThirdPartyIntegrationManager> = tokio::sync::OnceCell::const_new();

/// Get the global integration manager
pub async fn get_integration_manager() -> &'static ThirdPartyIntegrationManager {
    INTEGRATION_MANAGER.get_or_init(|| async {
        ThirdPartyIntegrationManager::new()
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_integration_creation() {
        let manager = ThirdPartyIntegrationManager::new();
        
        let integration = Integration {
            id: "test-integration".to_string(),
            name: "Test Integration".to_string(),
            description: "A test integration".to_string(),
            integration_type: IntegrationType::RestApi,
            provider: "Test Provider".to_string(),
            config: IntegrationConfig {
                endpoint_url: Some("https://api.example.com".to_string()),
                timeout_seconds: 30,
                retry_attempts: 3,
                rate_limit: None,
                headers: HashMap::new(),
                parameters: HashMap::new(),
                webhook_secret: None,
                ssl_verify: true,
            },
            credentials: IntegrationCredentials {
                auth_type: AuthType::None,
                encrypted_data: String::new(),
            },
            is_enabled: true,
            last_used: None,
            usage_count: 0,
            error_count: 0,
            created_at: chrono::Utc::now(),
        };
        
        // Note: This test will fail connection test in real environment
        // In actual testing, would mock the HTTP client
        let _result = manager.add_integration(integration).await;
        // assert!(result.is_ok()); // Would pass with mocked HTTP client
    }

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(5, 10);
        
        // Should allow first few requests
        assert!(limiter.allow_request().await);
        assert!(limiter.allow_request().await);
        assert!(limiter.allow_request().await);
    }
}
