//! Developer SDK for Gestura.app
//! Provides APIs and tools for external developers

#[allow(unused_imports)]
use crate::AppError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// SDK version information
#[derive(Debug, Clone, serde::Serialize)]
pub struct SdkVersion {
    pub version: String,
    pub api_version: String,
    pub build_date: String,
    pub features: Vec<String>,
}

/// API endpoint definition
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiEndpoint {
    pub path: String,
    pub method: HttpMethod,
    pub description: String,
    pub parameters: Vec<ApiParameter>,
    pub response_schema: serde_json::Value,
    pub requires_auth: bool,
    pub rate_limit: Option<u32>,
    pub deprecated: bool,
}

/// HTTP methods for API endpoints
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
}

/// API parameter definition
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiParameter {
    pub name: String,
    pub param_type: ParameterType,
    pub description: String,
    pub required: bool,
    pub default_value: Option<serde_json::Value>,
    pub validation: Option<ParameterValidation>,
}

/// Parameter types
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ParameterType {
    String,
    Integer,
    Float,
    Boolean,
    Array,
    Object,
    File,
}

/// Parameter validation rules
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParameterValidation {
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub pattern: Option<String>,
    pub allowed_values: Option<Vec<serde_json::Value>>,
}

/// SDK client configuration
#[derive(Debug, Clone)]
pub struct SdkClientConfig {
    pub api_key: String,
    pub base_url: String,
    pub timeout_seconds: u64,
    pub retry_attempts: u32,
    pub user_agent: String,
}

/// SDK client for external developers
pub struct SdkClient {
    config: SdkClientConfig,
    http_client: reqwest::Client,
    endpoints: Arc<RwLock<HashMap<String, ApiEndpoint>>>,
}

impl SdkClient {
    /// Create a new SDK client
    pub fn new(config: SdkClientConfig) -> Result<Self, AppError> {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_seconds))
            .user_agent(&config.user_agent)
            .build()
            .map_err(|e| AppError::Io(std::io::Error::other(e)))?;

        Ok(Self {
            config,
            http_client,
            endpoints: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Initialize SDK client with available endpoints
    pub async fn initialize(&self) -> Result<(), AppError> {
        // Load available endpoints
        let endpoints = self.discover_endpoints().await?;

        let mut endpoints_map = self.endpoints.write().await;
        for endpoint in endpoints {
            endpoints_map.insert(endpoint.path.clone(), endpoint);
        }

        tracing::info!(
            "SDK client initialized with {} endpoints",
            endpoints_map.len()
        );
        Ok(())
    }

    /// Discover available API endpoints
    async fn discover_endpoints(&self) -> Result<Vec<ApiEndpoint>, AppError> {
        // In real implementation, would query the API for available endpoints
        Ok(vec![
            ApiEndpoint {
                path: "/api/v1/voice/recognize".to_string(),
                method: HttpMethod::POST,
                description: "Recognize speech from audio data".to_string(),
                parameters: vec![
                    ApiParameter {
                        name: "audio_data".to_string(),
                        param_type: ParameterType::File,
                        description: "Audio file to process".to_string(),
                        required: true,
                        default_value: None,
                        validation: None,
                    },
                    ApiParameter {
                        name: "language".to_string(),
                        param_type: ParameterType::String,
                        description: "Language code (e.g., 'en-US')".to_string(),
                        required: false,
                        default_value: Some(serde_json::Value::String("en-US".to_string())),
                        validation: Some(ParameterValidation {
                            min_length: Some(2),
                            max_length: Some(10),
                            min_value: None,
                            max_value: None,
                            pattern: Some(r"^[a-z]{2}-[A-Z]{2}$".to_string()),
                            allowed_values: None,
                        }),
                    },
                ],
                response_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"},
                        "confidence": {"type": "number"},
                        "language": {"type": "string"}
                    }
                }),
                requires_auth: true,
                rate_limit: Some(100),
                deprecated: false,
            },
            ApiEndpoint {
                path: "/api/v1/gestures/recognize".to_string(),
                method: HttpMethod::POST,
                description: "Recognize gesture from sensor data".to_string(),
                parameters: vec![ApiParameter {
                    name: "sensor_data".to_string(),
                    param_type: ParameterType::Array,
                    description: "Array of sensor readings".to_string(),
                    required: true,
                    default_value: None,
                    validation: None,
                }],
                response_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "gesture": {"type": "string"},
                        "confidence": {"type": "number"}
                    }
                }),
                requires_auth: true,
                rate_limit: Some(200),
                deprecated: false,
            },
            ApiEndpoint {
                path: "/api/v1/ring/status".to_string(),
                method: HttpMethod::GET,
                description: "Get Haptic Harmony ring status".to_string(),
                parameters: vec![],
                response_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connected": {"type": "boolean"},
                        "battery_level": {"type": "number"},
                        "signal_strength": {"type": "number"}
                    }
                }),
                requires_auth: true,
                rate_limit: Some(60),
                deprecated: false,
            },
            ApiEndpoint {
                path: "/api/v1/analytics/usage".to_string(),
                method: HttpMethod::GET,
                description: "Get usage analytics".to_string(),
                parameters: vec![ApiParameter {
                    name: "days".to_string(),
                    param_type: ParameterType::Integer,
                    description: "Number of days to include".to_string(),
                    required: false,
                    default_value: Some(serde_json::Value::Number(serde_json::Number::from(7))),
                    validation: Some(ParameterValidation {
                        min_length: None,
                        max_length: None,
                        min_value: Some(1.0),
                        max_value: Some(365.0),
                        pattern: None,
                        allowed_values: None,
                    }),
                }],
                response_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "total_events": {"type": "number"},
                        "unique_users": {"type": "number"},
                        "usage_patterns": {"type": "object"}
                    }
                }),
                requires_auth: true,
                rate_limit: Some(10),
                deprecated: false,
            },
        ])
    }

    /// Make API request
    pub async fn request(
        &self,
        endpoint_path: &str,
        parameters: HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, AppError> {
        let endpoints = self.endpoints.read().await;
        let endpoint = endpoints.get(endpoint_path).ok_or_else(|| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Endpoint not found: {}", endpoint_path),
            ))
        })?;

        // Validate parameters
        self.validate_parameters(endpoint, &parameters)?;

        // Build request URL
        let url = format!(
            "{}{}",
            self.config.base_url.trim_end_matches('/'),
            endpoint_path
        );

        // Build request
        let mut request_builder = match endpoint.method {
            HttpMethod::GET => self.http_client.get(&url),
            HttpMethod::POST => self.http_client.post(&url),
            HttpMethod::PUT => self.http_client.put(&url),
            HttpMethod::DELETE => self.http_client.delete(&url),
            HttpMethod::PATCH => self.http_client.patch(&url),
        };

        // Add authentication
        if endpoint.requires_auth {
            request_builder =
                request_builder.header("Authorization", format!("Bearer {}", self.config.api_key));
        }

        // Add parameters
        match endpoint.method {
            HttpMethod::GET => {
                // Add as query parameters
                let mut query_params = Vec::new();
                for (key, value) in parameters {
                    if let Some(str_value) = value.as_str() {
                        query_params.push((key, str_value.to_string()));
                    } else {
                        query_params.push((key, value.to_string()));
                    }
                }
                request_builder = request_builder.query(&query_params);
            }
            _ => {
                // Add as JSON body
                request_builder = request_builder.json(&parameters);
            }
        }

        // Execute request
        let response = request_builder
            .send()
            .await
            .map_err(|e| AppError::Io(std::io::Error::other(e)))?;

        if response.status().is_success() {
            let json_response = response.json::<serde_json::Value>().await.map_err(|e| {
                AppError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            })?;
            Ok(json_response)
        } else {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(AppError::Io(std::io::Error::other(format!(
                "API request failed: {}",
                error_text
            ))))
        }
    }

    /// Validate request parameters
    fn validate_parameters(
        &self,
        endpoint: &ApiEndpoint,
        parameters: &HashMap<String, serde_json::Value>,
    ) -> Result<(), AppError> {
        for param in &endpoint.parameters {
            if param.required && !parameters.contains_key(&param.name) {
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Required parameter missing: {}", param.name),
                )));
            }

            if let Some(value) = parameters.get(&param.name) {
                self.validate_parameter_value(param, value)?;
            }
        }

        Ok(())
    }

    /// Validate individual parameter value
    fn validate_parameter_value(
        &self,
        param: &ApiParameter,
        value: &serde_json::Value,
    ) -> Result<(), AppError> {
        // Type validation
        match param.param_type {
            ParameterType::String => {
                if !value.is_string() {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Parameter '{}' must be a string", param.name),
                    )));
                }
            }
            ParameterType::Integer => {
                if !value.is_i64() {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Parameter '{}' must be an integer", param.name),
                    )));
                }
            }
            ParameterType::Float => {
                if !value.is_f64() && !value.is_i64() {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Parameter '{}' must be a number", param.name),
                    )));
                }
            }
            ParameterType::Boolean => {
                if !value.is_boolean() {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Parameter '{}' must be a boolean", param.name),
                    )));
                }
            }
            ParameterType::Array => {
                if !value.is_array() {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Parameter '{}' must be an array", param.name),
                    )));
                }
            }
            ParameterType::Object => {
                if !value.is_object() {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Parameter '{}' must be an object", param.name),
                    )));
                }
            }
            ParameterType::File => {
                // File validation would be more complex in real implementation
                if !value.is_string() {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Parameter '{}' must be a file path", param.name),
                    )));
                }
            }
        }

        // Additional validation rules
        if let Some(validation) = &param.validation {
            if let Some(str_value) = value.as_str() {
                if let Some(min_len) = validation.min_length
                    && str_value.len() < min_len
                {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Parameter '{}' too short (min: {})", param.name, min_len),
                    )));
                }
                if let Some(max_len) = validation.max_length
                    && str_value.len() > max_len
                {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Parameter '{}' too long (max: {})", param.name, max_len),
                    )));
                }
            }

            if let Some(num_value) = value.as_f64() {
                if let Some(min_val) = validation.min_value
                    && num_value < min_val
                {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Parameter '{}' too small (min: {})", param.name, min_val),
                    )));
                }
                if let Some(max_val) = validation.max_value
                    && num_value > max_val
                {
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Parameter '{}' too large (max: {})", param.name, max_val),
                    )));
                }
            }
        }

        Ok(())
    }

    /// Get available endpoints
    pub async fn get_endpoints(&self) -> Vec<ApiEndpoint> {
        let endpoints = self.endpoints.read().await;
        endpoints.values().cloned().collect()
    }

    /// Get SDK version information
    pub fn get_version(&self) -> SdkVersion {
        SdkVersion {
            version: "1.0.0".to_string(),
            api_version: "v1".to_string(),
            build_date: "2024-01-01".to_string(),
            features: vec![
                "Voice Recognition".to_string(),
                "Gesture Recognition".to_string(),
                "Ring Integration".to_string(),
                "Analytics".to_string(),
                "Plugin System".to_string(),
                "Custom Gestures".to_string(),
                "Scripting".to_string(),
                "Third-party Integrations".to_string(),
            ],
        }
    }
}

/// SDK manager for handling multiple clients and API keys
pub struct SdkManager {
    clients: Arc<RwLock<HashMap<String, SdkClient>>>,
    api_keys: Arc<RwLock<HashMap<String, ApiKeyInfo>>>,
}

/// API key information
#[derive(Debug, Clone)]
struct ApiKeyInfo {
    #[allow(dead_code)]
    key: String,
    #[allow(dead_code)]
    name: String,
    permissions: Vec<String>,
    #[allow(dead_code)]
    rate_limit: u32,
    #[allow(dead_code)]
    created_at: chrono::DateTime<chrono::Utc>,
    last_used: Option<chrono::DateTime<chrono::Utc>>,
    usage_count: u32,
}

impl Default for SdkManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SdkManager {
    /// Create a new SDK manager
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Generate a new API key
    pub async fn generate_api_key(
        &self,
        name: String,
        permissions: Vec<String>,
    ) -> Result<String, AppError> {
        let api_key = format!("gsk_{}", uuid::Uuid::new_v4().to_string().replace('-', ""));

        let key_info = ApiKeyInfo {
            key: api_key.clone(),
            name,
            permissions,
            rate_limit: 1000, // Default rate limit
            created_at: chrono::Utc::now(),
            last_used: None,
            usage_count: 0,
        };

        let mut api_keys = self.api_keys.write().await;
        api_keys.insert(api_key.clone(), key_info);

        tracing::info!("Generated new API key: {}", &api_key[..12]);
        Ok(api_key)
    }

    /// Validate API key
    pub async fn validate_api_key(&self, api_key: &str) -> Result<Vec<String>, AppError> {
        let mut api_keys = self.api_keys.write().await;

        if let Some(key_info) = api_keys.get_mut(api_key) {
            key_info.last_used = Some(chrono::Utc::now());
            key_info.usage_count += 1;
            Ok(key_info.permissions.clone())
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Invalid API key",
            )))
        }
    }

    /// Create SDK client
    pub async fn create_client(
        &self,
        api_key: String,
        base_url: String,
    ) -> Result<String, AppError> {
        // Validate API key
        self.validate_api_key(&api_key).await?;

        let config = SdkClientConfig {
            api_key: api_key.clone(),
            base_url,
            timeout_seconds: 30,
            retry_attempts: 3,
            user_agent: "Gestura-SDK/1.0".to_string(),
        };

        let client = SdkClient::new(config)?;
        client.initialize().await?;

        let client_id = uuid::Uuid::new_v4().to_string();
        let mut clients = self.clients.write().await;
        clients.insert(client_id.clone(), client);

        Ok(client_id)
    }

    /// Get SDK statistics
    pub async fn get_stats(&self) -> serde_json::Value {
        let clients = self.clients.read().await;
        let api_keys = self.api_keys.read().await;

        let total_clients = clients.len();
        let total_api_keys = api_keys.len();
        let total_usage: u32 = api_keys.values().map(|k| k.usage_count).sum();

        serde_json::json!({
            "total_clients": total_clients,
            "total_api_keys": total_api_keys,
            "total_usage": total_usage
        })
    }
}

/// Global SDK manager instance
static SDK_MANAGER: tokio::sync::OnceCell<SdkManager> = tokio::sync::OnceCell::const_new();

/// Get the global SDK manager
pub async fn get_sdk_manager() -> &'static SdkManager {
    SDK_MANAGER
        .get_or_init(|| async { SdkManager::new() })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sdk_manager() {
        let manager = SdkManager::new();

        let api_key = manager
            .generate_api_key(
                "Test App".to_string(),
                vec!["voice".to_string(), "gestures".to_string()],
            )
            .await
            .unwrap();

        let permissions = manager.validate_api_key(&api_key).await.unwrap();
        assert_eq!(permissions.len(), 2);
    }

    #[test]
    fn test_parameter_validation() {
        let param = ApiParameter {
            name: "test".to_string(),
            param_type: ParameterType::String,
            description: "Test parameter".to_string(),
            required: true,
            default_value: None,
            validation: Some(ParameterValidation {
                min_length: Some(3),
                max_length: Some(10),
                min_value: None,
                max_value: None,
                pattern: None,
                allowed_values: None,
            }),
        };

        let config = SdkClientConfig {
            api_key: "test".to_string(),
            base_url: "http://localhost".to_string(),
            timeout_seconds: 30,
            retry_attempts: 3,
            user_agent: "test".to_string(),
        };

        let client = SdkClient::new(config).unwrap();

        // Valid value
        let valid_value = serde_json::Value::String("hello".to_string());
        assert!(
            client
                .validate_parameter_value(&param, &valid_value)
                .is_ok()
        );

        // Invalid value (too short)
        let invalid_value = serde_json::Value::String("hi".to_string());
        assert!(
            client
                .validate_parameter_value(&param, &invalid_value)
                .is_err()
        );
    }
}
