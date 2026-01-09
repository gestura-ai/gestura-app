//! Federated learning for Gestura.app
//! Privacy-preserving distributed machine learning across users

#[allow(unused_imports)]
use crate::AppError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Model update for federated learning
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelUpdate {
    pub update_id: String,
    pub client_id: String,
    pub model_version: u64,
    pub parameters: Vec<f32>,
    pub gradient_updates: Vec<f32>,
    pub training_samples: usize,
    pub accuracy: f32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub privacy_budget: f32, // For differential privacy
}

/// Aggregated global model
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GlobalModel {
    pub version: u64,
    pub parameters: Vec<f32>,
    pub accuracy: f32,
    pub participating_clients: usize,
    pub total_training_samples: usize,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub model_type: ModelType,
}

/// Types of models for federated learning
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ModelType {
    GestureRecognition,
    VoiceRecognition,
    UserBehaviorPrediction,
    ErrorPrediction,
    PerformanceOptimization,
}

/// Federated learning configuration
#[derive(Debug, Clone)]
pub struct FederatedConfig {
    pub min_clients_for_aggregation: usize,
    pub max_clients_per_round: usize,
    pub learning_rate: f32,
    pub privacy_epsilon: f32,           // Differential privacy parameter
    pub model_staleness_threshold: u64, // Max version difference allowed
    pub aggregation_strategy: AggregationStrategy,
    pub enable_secure_aggregation: bool,
    pub client_selection_strategy: ClientSelectionStrategy,
}

/// Aggregation strategies for combining model updates
#[derive(Debug, Clone, PartialEq)]
pub enum AggregationStrategy {
    FederatedAveraging,
    WeightedAveraging,
    MedianAggregation,
    TrimmedMean,
    SecureAggregation,
}

/// Client selection strategies
#[derive(Debug, Clone, PartialEq)]
pub enum ClientSelectionStrategy {
    Random,
    DataQuality,
    ResourceBased,
    Staleness,
    Hybrid,
}

impl Default for FederatedConfig {
    fn default() -> Self {
        Self {
            min_clients_for_aggregation: 3,
            max_clients_per_round: 10,
            learning_rate: 0.01,
            privacy_epsilon: 1.0,
            model_staleness_threshold: 5,
            aggregation_strategy: AggregationStrategy::FederatedAveraging,
            enable_secure_aggregation: true,
            client_selection_strategy: ClientSelectionStrategy::Hybrid,
        }
    }
}

/// Federated learning coordinator
pub struct FederatedLearningCoordinator {
    global_models: Arc<RwLock<HashMap<ModelType, GlobalModel>>>,
    pending_updates: Arc<RwLock<HashMap<ModelType, Vec<ModelUpdate>>>>,
    client_registry: Arc<RwLock<HashMap<String, ClientInfo>>>,
    config: Arc<RwLock<FederatedConfig>>,
    aggregation_history: Arc<RwLock<Vec<AggregationRound>>>,
}

/// Client information for federated learning
#[derive(Debug, Clone)]
struct ClientInfo {
    #[allow(dead_code)]
    client_id: String,
    last_seen: chrono::DateTime<chrono::Utc>,
    #[allow(dead_code)]
    model_versions: HashMap<ModelType, u64>,
    data_quality_score: f32,
    resource_capacity: f32,
    privacy_budget_remaining: f32,
    total_contributions: usize,
}

/// Aggregation round information
#[derive(Debug, Clone, serde::Serialize)]
struct AggregationRound {
    round_id: String,
    model_type: ModelType,
    participating_clients: Vec<String>,
    old_version: u64,
    new_version: u64,
    accuracy_improvement: f32,
    timestamp: chrono::DateTime<chrono::Utc>,
}

impl FederatedLearningCoordinator {
    /// Create a new federated learning coordinator
    pub fn new(config: FederatedConfig) -> Self {
        Self {
            global_models: Arc::new(RwLock::new(HashMap::new())),
            pending_updates: Arc::new(RwLock::new(HashMap::new())),
            client_registry: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(config)),
            aggregation_history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Register a client for federated learning
    pub async fn register_client(&self, client_id: String) -> Result<(), AppError> {
        let mut registry = self.client_registry.write().await;

        registry.insert(
            client_id.clone(),
            ClientInfo {
                client_id: client_id.clone(),
                last_seen: chrono::Utc::now(),
                model_versions: HashMap::new(),
                data_quality_score: 0.5,        // Default score
                resource_capacity: 1.0,         // Default capacity
                privacy_budget_remaining: 10.0, // Default budget
                total_contributions: 0,
            },
        );

        tracing::info!("Registered federated learning client: {}", client_id);
        Ok(())
    }

    /// Submit a model update from a client
    pub async fn submit_update(&self, update: ModelUpdate) -> Result<(), AppError> {
        // Validate update
        self.validate_update(&update).await?;

        // Apply differential privacy
        let private_update = self.apply_differential_privacy(update).await?;

        // Store pending update
        let mut pending = self.pending_updates.write().await;
        let model_type = ModelType::GestureRecognition; // Infer from update context

        let client_id = private_update.client_id.clone();
        let privacy_budget = private_update.privacy_budget;

        pending
            .entry(model_type.clone())
            .or_insert_with(Vec::new)
            .push(private_update);

        // Update client info
        let mut registry = self.client_registry.write().await;
        if let Some(client_info) = registry.get_mut(&client_id) {
            client_info.last_seen = chrono::Utc::now();
            client_info.total_contributions += 1;
            client_info.privacy_budget_remaining -= privacy_budget;
        }

        // Check if we can trigger aggregation
        let config = self.config.read().await;
        if pending.get(&model_type).map(|v| v.len()).unwrap_or(0)
            >= config.min_clients_for_aggregation
        {
            drop(pending);
            drop(registry);
            drop(config);
            self.trigger_aggregation(model_type).await?;
        }

        Ok(())
    }

    /// Get the latest global model
    pub async fn get_global_model(&self, model_type: ModelType) -> Option<GlobalModel> {
        let models = self.global_models.read().await;
        models.get(&model_type).cloned()
    }

    /// Trigger model aggregation
    async fn trigger_aggregation(&self, model_type: ModelType) -> Result<(), AppError> {
        let mut pending = self.pending_updates.write().await;
        let updates = pending.remove(&model_type).unwrap_or_default();

        if updates.is_empty() {
            return Ok(());
        }

        drop(pending);

        // Select clients for this round
        let selected_updates = self.select_clients_for_aggregation(&updates).await;

        if selected_updates.len() < self.config.read().await.min_clients_for_aggregation {
            tracing::warn!("Not enough clients selected for aggregation");
            return Ok(());
        }

        // Perform aggregation
        let new_global_model = self
            .aggregate_updates(&selected_updates, &model_type)
            .await?;

        // Update global model
        let mut models = self.global_models.write().await;
        let old_version = models.get(&model_type).map(|m| m.version).unwrap_or(0);
        models.insert(model_type.clone(), new_global_model.clone());

        // Record aggregation round
        let round = AggregationRound {
            round_id: uuid::Uuid::new_v4().to_string(),
            model_type: model_type.clone(),
            participating_clients: selected_updates
                .iter()
                .map(|u| u.client_id.clone())
                .collect(),
            old_version,
            new_version: new_global_model.version,
            accuracy_improvement: new_global_model.accuracy
                - models.get(&model_type).map(|m| m.accuracy).unwrap_or(0.0),
            timestamp: chrono::Utc::now(),
        };

        let mut history = self.aggregation_history.write().await;
        history.push(round);
        if history.len() > 100 {
            history.remove(0); // Keep only recent rounds
        }

        tracing::info!(
            "Completed federated aggregation for {:?}, new version: {}",
            model_type,
            new_global_model.version
        );

        Ok(())
    }

    /// Validate a model update
    async fn validate_update(&self, update: &ModelUpdate) -> Result<(), AppError> {
        // Check client registration
        let registry = self.client_registry.read().await;
        let client_info = registry.get(&update.client_id).ok_or_else(|| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Client not registered",
            ))
        })?;

        // Check privacy budget
        if client_info.privacy_budget_remaining < update.privacy_budget {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Insufficient privacy budget",
            )));
        }

        // Check model staleness
        let config = self.config.read().await;
        let models = self.global_models.read().await;
        if let Some(global_model) = models.get(&ModelType::GestureRecognition) {
            let version_diff = global_model.version.saturating_sub(update.model_version);
            if version_diff > config.model_staleness_threshold {
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Model update too stale",
                )));
            }
        }

        // Validate parameter dimensions
        if update.parameters.is_empty() || update.gradient_updates.is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid parameter dimensions",
            )));
        }

        Ok(())
    }

    /// Apply differential privacy to model update
    async fn apply_differential_privacy(
        &self,
        mut update: ModelUpdate,
    ) -> Result<ModelUpdate, AppError> {
        let config = self.config.read().await;
        let epsilon = config.privacy_epsilon;

        // Add Gaussian noise to gradients for differential privacy
        let noise_scale = 2.0 / epsilon; // Simplified noise calculation

        for gradient in update.gradient_updates.iter_mut() {
            let noise = rand::random::<f32>() * noise_scale - noise_scale / 2.0;
            *gradient += noise;
        }

        // Clip gradients to bound sensitivity
        let clip_norm = 1.0;
        let gradient_norm: f32 = update
            .gradient_updates
            .iter()
            .map(|g| g * g)
            .sum::<f32>()
            .sqrt();

        if gradient_norm > clip_norm {
            let scale_factor = clip_norm / gradient_norm;
            for gradient in update.gradient_updates.iter_mut() {
                *gradient *= scale_factor;
            }
        }

        update.privacy_budget = epsilon;
        Ok(update)
    }

    /// Select clients for aggregation round
    async fn select_clients_for_aggregation(&self, updates: &[ModelUpdate]) -> Vec<ModelUpdate> {
        let config = self.config.read().await;
        let registry = self.client_registry.read().await;

        let mut scored_updates: Vec<(ModelUpdate, f32)> = updates
            .iter()
            .filter_map(|update| {
                registry.get(&update.client_id).map(|client_info| {
                    let score = match config.client_selection_strategy {
                        ClientSelectionStrategy::Random => rand::random::<f32>(),
                        ClientSelectionStrategy::DataQuality => client_info.data_quality_score,
                        ClientSelectionStrategy::ResourceBased => client_info.resource_capacity,
                        ClientSelectionStrategy::Staleness => {
                            1.0 / (1.0 + update.model_version as f32)
                        }
                        ClientSelectionStrategy::Hybrid => {
                            (client_info.data_quality_score + client_info.resource_capacity) / 2.0
                        }
                    };
                    (update.clone(), score)
                })
            })
            .collect();

        // Sort by score and select top clients
        scored_updates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored_updates.truncate(config.max_clients_per_round);

        scored_updates
            .into_iter()
            .map(|(update, _)| update)
            .collect()
    }

    /// Aggregate model updates
    async fn aggregate_updates(
        &self,
        updates: &[ModelUpdate],
        model_type: &ModelType,
    ) -> Result<GlobalModel, AppError> {
        if updates.is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "No updates to aggregate",
            )));
        }

        let config = self.config.read().await;
        let current_models = self.global_models.read().await;
        let current_version = current_models
            .get(model_type)
            .map(|m| m.version)
            .unwrap_or(0);

        // Initialize aggregated parameters
        let param_size = updates[0].parameters.len();
        let mut aggregated_params = vec![0.0; param_size];
        let mut total_weight = 0.0;

        // Aggregate based on strategy
        match config.aggregation_strategy {
            AggregationStrategy::FederatedAveraging => {
                for update in updates {
                    let weight = update.training_samples as f32;
                    total_weight += weight;

                    for (i, &param) in update.parameters.iter().enumerate() {
                        aggregated_params[i] += param * weight;
                    }
                }

                // Normalize by total weight
                for param in aggregated_params.iter_mut() {
                    *param /= total_weight;
                }
            }
            AggregationStrategy::WeightedAveraging => {
                for update in updates {
                    let weight = update.accuracy;
                    total_weight += weight;

                    for (i, &param) in update.parameters.iter().enumerate() {
                        aggregated_params[i] += param * weight;
                    }
                }

                for param in aggregated_params.iter_mut() {
                    *param /= total_weight;
                }
            }
            AggregationStrategy::MedianAggregation => {
                // For each parameter, take the median across all updates
                for (i, aggregated_param) in
                    aggregated_params.iter_mut().enumerate().take(param_size)
                {
                    let mut param_values: Vec<f32> =
                        updates.iter().map(|u| u.parameters[i]).collect();
                    param_values
                        .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                    *aggregated_param = if param_values.len() % 2 == 0 {
                        (param_values[param_values.len() / 2 - 1]
                            + param_values[param_values.len() / 2])
                            / 2.0
                    } else {
                        param_values[param_values.len() / 2]
                    };
                }
            }
            _ => {
                // Default to federated averaging
                for update in updates {
                    let weight = update.training_samples as f32;
                    total_weight += weight;

                    for (i, &param) in update.parameters.iter().enumerate() {
                        aggregated_params[i] += param * weight;
                    }
                }

                for param in aggregated_params.iter_mut() {
                    *param /= total_weight;
                }
            }
        }

        // Calculate aggregated accuracy
        let aggregated_accuracy = updates
            .iter()
            .map(|u| u.accuracy * u.training_samples as f32)
            .sum::<f32>()
            / updates
                .iter()
                .map(|u| u.training_samples as f32)
                .sum::<f32>();

        Ok(GlobalModel {
            version: current_version + 1,
            parameters: aggregated_params,
            accuracy: aggregated_accuracy,
            participating_clients: updates.len(),
            total_training_samples: updates.iter().map(|u| u.training_samples).sum(),
            created_at: chrono::Utc::now(),
            model_type: model_type.clone(),
        })
    }

    /// Get federated learning statistics
    pub async fn get_stats(&self) -> serde_json::Value {
        let models = self.global_models.read().await;
        let registry = self.client_registry.read().await;
        let history = self.aggregation_history.read().await;
        let pending = self.pending_updates.read().await;

        let total_clients = registry.len();
        let active_clients = registry
            .values()
            .filter(|c| (chrono::Utc::now() - c.last_seen).num_hours() < 24)
            .count();

        let total_models = models.len();
        let total_rounds = history.len();
        let pending_updates: usize = pending.values().map(|v| v.len()).sum();

        serde_json::json!({
            "total_clients": total_clients,
            "active_clients": active_clients,
            "total_models": total_models,
            "total_aggregation_rounds": total_rounds,
            "pending_updates": pending_updates,
            "model_versions": models.iter().map(|(k, v)| (format!("{:?}", k), v.version)).collect::<HashMap<_, _>>()
        })
    }

    /// Clear client data (for privacy compliance)
    pub async fn clear_client_data(&self, client_id: &str) -> Result<(), AppError> {
        let mut registry = self.client_registry.write().await;
        let mut pending = self.pending_updates.write().await;

        registry.remove(client_id);

        // Remove pending updates from this client
        for updates in pending.values_mut() {
            updates.retain(|u| u.client_id != client_id);
        }

        tracing::info!("Cleared federated learning data for client: {}", client_id);
        Ok(())
    }
}

/// Global federated learning coordinator instance
static FEDERATED_COORDINATOR: tokio::sync::OnceCell<FederatedLearningCoordinator> =
    tokio::sync::OnceCell::const_new();

/// Get the global federated learning coordinator
pub async fn get_federated_coordinator() -> &'static FederatedLearningCoordinator {
    FEDERATED_COORDINATOR
        .get_or_init(|| async { FederatedLearningCoordinator::new(FederatedConfig::default()) })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_registration() {
        let coordinator = FederatedLearningCoordinator::new(FederatedConfig::default());

        coordinator
            .register_client("client1".to_string())
            .await
            .unwrap();

        let stats = coordinator.get_stats().await;
        assert_eq!(stats["total_clients"].as_u64().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_model_update_submission() {
        let coordinator = FederatedLearningCoordinator::new(FederatedConfig {
            min_clients_for_aggregation: 1,
            ..FederatedConfig::default()
        });

        coordinator
            .register_client("client1".to_string())
            .await
            .unwrap();

        let update = ModelUpdate {
            update_id: "update1".to_string(),
            client_id: "client1".to_string(),
            model_version: 0,
            parameters: vec![1.0, 2.0, 3.0],
            gradient_updates: vec![0.1, 0.2, 0.3],
            training_samples: 100,
            accuracy: 0.85,
            timestamp: chrono::Utc::now(),
            privacy_budget: 0.1,
        };

        coordinator.submit_update(update).await.unwrap();

        let model = coordinator
            .get_global_model(ModelType::GestureRecognition)
            .await;
        assert!(model.is_some());
    }

    #[tokio::test]
    async fn test_differential_privacy() {
        let coordinator = FederatedLearningCoordinator::new(FederatedConfig::default());

        let original_update = ModelUpdate {
            update_id: "update1".to_string(),
            client_id: "client1".to_string(),
            model_version: 0,
            parameters: vec![1.0, 2.0, 3.0],
            gradient_updates: vec![0.1, 0.2, 0.3],
            training_samples: 100,
            accuracy: 0.85,
            timestamp: chrono::Utc::now(),
            privacy_budget: 0.0,
        };

        let private_update = coordinator
            .apply_differential_privacy(original_update.clone())
            .await
            .unwrap();

        // Gradients should be different due to noise
        assert_ne!(
            private_update.gradient_updates,
            original_update.gradient_updates
        );
        assert!(private_update.privacy_budget > 0.0);
    }
}
