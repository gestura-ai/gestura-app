//! GDPR compliance features for Gestura.app
//! Provides data export, deletion, consent management, and audit trails

use crate::AppError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// GDPR data categories
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DataCategory {
    PersonalIdentifiers,
    VoiceRecordings,
    BiometricData,
    DeviceData,
    UsageAnalytics,
    ConfigurationData,
    LogData,
}

/// Consent status for data processing
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConsentStatus {
    Granted,
    Denied,
    Withdrawn,
    Pending,
}

/// Consent record
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConsentRecord {
    pub user_id: String,
    pub category: DataCategory,
    pub status: ConsentStatus,
    pub granted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub withdrawn_at: Option<chrono::DateTime<chrono::Utc>>,
    pub purpose: String,
    pub legal_basis: String,
}

/// Data audit entry
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DataAuditEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub user_id: String,
    pub operation: DataOperation,
    pub category: DataCategory,
    pub details: String,
    pub legal_basis: String,
}

/// Data operations for audit trail
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum DataOperation {
    Collect,
    Process,
    Store,
    Access,
    Modify,
    Delete,
    Export,
    Share,
}

/// GDPR compliance manager
pub struct GdprManager {
    consent_records: Arc<RwLock<HashMap<String, Vec<ConsentRecord>>>>,
    audit_trail: Arc<RwLock<Vec<DataAuditEntry>>>,
    data_locations: Arc<RwLock<HashMap<DataCategory, Vec<PathBuf>>>>,
    max_audit_entries: usize,
}

impl GdprManager {
    /// Create a new GDPR manager
    pub fn new(max_audit_entries: usize) -> Self {
        Self {
            consent_records: Arc::new(RwLock::new(HashMap::new())),
            audit_trail: Arc::new(RwLock::new(Vec::new())),
            data_locations: Arc::new(RwLock::new(HashMap::new())),
            max_audit_entries,
        }
    }

    /// Register consent for data processing
    pub async fn register_consent(&self, user_id: String, category: DataCategory, purpose: String, legal_basis: String) -> Result<(), AppError> {
        let consent = ConsentRecord {
            user_id: user_id.clone(),
            category: category.clone(),
            status: ConsentStatus::Granted,
            granted_at: Some(chrono::Utc::now()),
            withdrawn_at: None,
            purpose,
            legal_basis: legal_basis.clone(),
        };

        let mut consents = self.consent_records.write().await;
        consents.entry(user_id.clone()).or_insert_with(Vec::new).push(consent);

        // Audit the consent
        self.audit_data_operation(user_id.clone(), DataOperation::Collect, category, "Consent granted".to_string(), legal_basis).await;

        tracing::info!("Consent registered for user: {}", user_id);
        Ok(())
    }

    /// Withdraw consent for data processing
    pub async fn withdraw_consent(&self, user_id: &str, category: &DataCategory) -> Result<(), AppError> {
        let mut consents = self.consent_records.write().await;
        
        if let Some(user_consents) = consents.get_mut(user_id) {
            for consent in user_consents.iter_mut() {
                if consent.category == *category && consent.status == ConsentStatus::Granted {
                    consent.status = ConsentStatus::Withdrawn;
                    consent.withdrawn_at = Some(chrono::Utc::now());
                    
                    // Audit the withdrawal
                    self.audit_data_operation(
                        user_id.to_string(), 
                        DataOperation::Modify, 
                        category.clone(), 
                        "Consent withdrawn".to_string(), 
                        consent.legal_basis.clone()
                    ).await;
                    
                    tracing::info!("Consent withdrawn for user: {} category: {:?}", user_id, category);
                    return Ok(());
                }
            }
        }

        Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Consent record not found"
        )))
    }

    /// Check if user has given consent for a data category
    pub async fn has_consent(&self, user_id: &str, category: &DataCategory) -> bool {
        let consents = self.consent_records.read().await;
        
        if let Some(user_consents) = consents.get(user_id) {
            user_consents.iter().any(|c| c.category == *category && c.status == ConsentStatus::Granted)
        } else {
            false
        }
    }

    /// Export all user data (GDPR Article 20)
    pub async fn export_user_data(&self, user_id: &str) -> Result<serde_json::Value, AppError> {
        // Audit the export request
        self.audit_data_operation(
            user_id.to_string(),
            DataOperation::Export,
            DataCategory::PersonalIdentifiers,
            "Data export requested".to_string(),
            "GDPR Article 20".to_string()
        ).await;

        let mut export_data = serde_json::Map::new();

        // Export consent records
        let consents = self.consent_records.read().await;
        if let Some(user_consents) = consents.get(user_id) {
            export_data.insert("consents".to_string(), serde_json::to_value(user_consents)?);
        }

        // Export configuration data
        let config = crate::AppConfig::load();
        export_data.insert("configuration".to_string(), serde_json::to_value(&config)?);

        // Export audit trail for this user
        let audit_entries = self.get_user_audit_trail(user_id).await;
        export_data.insert("audit_trail".to_string(), serde_json::to_value(&audit_entries)?);

        // Export voice data locations (metadata only)
        let data_locations = self.data_locations.read().await;
        if let Some(voice_locations) = data_locations.get(&DataCategory::VoiceRecordings) {
            let voice_metadata: Vec<_> = voice_locations.iter()
                .map(|path| serde_json::json!({
                    "path": path.to_string_lossy(),
                    "category": "voice_recording"
                }))
                .collect();
            export_data.insert("voice_data_locations".to_string(), serde_json::Value::Array(voice_metadata));
        }

        tracing::info!("Data export completed for user: {}", user_id);
        Ok(serde_json::Value::Object(export_data))
    }

    /// Delete all user data (GDPR Article 17 - Right to be forgotten)
    pub async fn delete_user_data(&self, user_id: &str, verify: bool) -> Result<Vec<String>, AppError> {
        let mut deleted_items = Vec::new();

        // Audit the deletion request
        self.audit_data_operation(
            user_id.to_string(),
            DataOperation::Delete,
            DataCategory::PersonalIdentifiers,
            "Data deletion requested".to_string(),
            "GDPR Article 17".to_string()
        ).await;

        // Delete consent records
        let mut consents = self.consent_records.write().await;
        if consents.remove(user_id).is_some() {
            deleted_items.push("Consent records".to_string());
        }
        drop(consents);

        // Delete data files
        let data_locations = self.data_locations.read().await;
        for (category, locations) in data_locations.iter() {
            for path in locations {
                if path.to_string_lossy().contains(user_id) {
                    if verify {
                        // In verify mode, just check what would be deleted
                        deleted_items.push(format!("{:?}: {}", category, path.display()));
                    } else {
                        // Actually delete the file
                        match tokio::fs::remove_file(path).await {
                            Ok(_) => {
                                deleted_items.push(format!("{:?}: {}", category, path.display()));
                                tracing::info!("Deleted file: {}", path.display());
                            }
                            Err(e) => {
                                tracing::error!("Failed to delete file {}: {}", path.display(), e);
                            }
                        }
                    }
                }
            }
        }

        // Remove user from audit trail (anonymize)
        if !verify {
            let mut audit_trail = self.audit_trail.write().await;
            for entry in audit_trail.iter_mut() {
                if entry.user_id == user_id {
                    entry.user_id = "[DELETED]".to_string();
                }
            }
        }

        if verify {
            tracing::info!("Data deletion verification completed for user: {}", user_id);
        } else {
            tracing::info!("Data deletion completed for user: {}", user_id);
        }

        Ok(deleted_items)
    }

    /// Register data location for tracking
    pub async fn register_data_location(&self, category: DataCategory, path: PathBuf) {
        let mut locations = self.data_locations.write().await;
        locations.entry(category).or_insert_with(Vec::new).push(path);
    }

    /// Audit data operation
    pub async fn audit_data_operation(&self, user_id: String, operation: DataOperation, category: DataCategory, details: String, legal_basis: String) {
        let entry = DataAuditEntry {
            timestamp: chrono::Utc::now(),
            user_id,
            operation,
            category,
            details,
            legal_basis,
        };

        let mut audit_trail = self.audit_trail.write().await;
        audit_trail.push(entry);

        // Trim audit trail if needed
        if audit_trail.len() > self.max_audit_entries {
            audit_trail.remove(0);
        }
    }

    /// Get audit trail for a specific user
    pub async fn get_user_audit_trail(&self, user_id: &str) -> Vec<DataAuditEntry> {
        let audit_trail = self.audit_trail.read().await;
        audit_trail.iter()
            .filter(|entry| entry.user_id == user_id)
            .cloned()
            .collect()
    }

    /// Get full audit trail
    pub async fn get_audit_trail(&self, limit: Option<usize>) -> Vec<DataAuditEntry> {
        let audit_trail = self.audit_trail.read().await;
        if let Some(limit) = limit {
            audit_trail.iter().rev().take(limit).cloned().collect()
        } else {
            audit_trail.clone()
        }
    }

    /// Get consent status for user
    pub async fn get_user_consents(&self, user_id: &str) -> Vec<ConsentRecord> {
        let consents = self.consent_records.read().await;
        consents.get(user_id).cloned().unwrap_or_default()
    }

    /// Generate privacy report
    pub async fn generate_privacy_report(&self) -> serde_json::Value {
        let consents = self.consent_records.read().await;
        let audit_trail = self.audit_trail.read().await;
        let data_locations = self.data_locations.read().await;

        let total_users = consents.len();
        let total_consents: usize = consents.values().map(|v| v.len()).sum();
        let total_audit_entries = audit_trail.len();
        let total_data_locations: usize = data_locations.values().map(|v| v.len()).sum();

        serde_json::json!({
            "generated_at": chrono::Utc::now(),
            "summary": {
                "total_users": total_users,
                "total_consents": total_consents,
                "total_audit_entries": total_audit_entries,
                "total_data_locations": total_data_locations
            },
            "consent_breakdown": {
                "granted": consents.values().flatten().filter(|c| c.status == ConsentStatus::Granted).count(),
                "withdrawn": consents.values().flatten().filter(|c| c.status == ConsentStatus::Withdrawn).count(),
                "denied": consents.values().flatten().filter(|c| c.status == ConsentStatus::Denied).count(),
                "pending": consents.values().flatten().filter(|c| c.status == ConsentStatus::Pending).count()
            },
            "data_categories": data_locations.keys().collect::<Vec<_>>()
        })
    }
}

/// Global GDPR manager instance
static GDPR_MANAGER: tokio::sync::OnceCell<GdprManager> = tokio::sync::OnceCell::const_new();

/// Get the global GDPR manager
pub async fn get_gdpr_manager() -> &'static GdprManager {
    GDPR_MANAGER.get_or_init(|| async {
        GdprManager::new(50000)
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_gdpr_manager() {
        let manager = GdprManager::new(1000);
        
        // Test consent registration
        manager.register_consent(
            "test-user".to_string(),
            DataCategory::VoiceRecordings,
            "Voice processing".to_string(),
            "User consent".to_string()
        ).await.unwrap();
        
        // Test consent check
        assert!(manager.has_consent("test-user", &DataCategory::VoiceRecordings).await);
        
        // Test consent withdrawal
        manager.withdraw_consent("test-user", &DataCategory::VoiceRecordings).await.unwrap();
        assert!(!manager.has_consent("test-user", &DataCategory::VoiceRecordings).await);
        
        // Test data export
        let export = manager.export_user_data("test-user").await.unwrap();
        assert!(export.get("consents").is_some());
    }
}
