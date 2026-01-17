//! MDH (Metadata Harmony) translator for JSON-LD to MCP resource conversion
//! Provides comprehensive JSON-LD processing and MCP resource generation

use crate::AppError;
use crate::mcp::MdhResource;
use std::collections::HashMap;
use std::path::PathBuf;

/// Enhanced MDH translator with caching and validation
pub struct MdhTranslator {
    cache: std::sync::RwLock<HashMap<String, MdhResource>>,
    context_cache: std::sync::RwLock<HashMap<String, serde_json::Value>>,
}

impl Default for MdhTranslator {
    fn default() -> Self {
        Self {
            cache: std::sync::RwLock::new(HashMap::new()),
            context_cache: std::sync::RwLock::new(HashMap::new()),
        }
    }
}

impl MdhTranslator {
    /// Create a new MDH translator
    pub fn new() -> Self {
        Self::default()
    }

    /// Translate JSON-LD file to MCP resource with caching
    pub async fn translate(&self, ld_file: PathBuf) -> Result<MdhResource, AppError> {
        let file_key = ld_file.to_string_lossy().to_string();

        // Check cache first
        if let Ok(cache) = self.cache.read()
            && let Some(cached) = cache.get(&file_key)
        {
            return Ok(cached.clone());
        }

        // Load and process file
        let content = tokio::fs::read_to_string(&ld_file)
            .await
            .map_err(AppError::Io)?;

        let resource = self.process_json_ld(&content, &file_key).await?;

        // Cache result
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(file_key, resource.clone());
        }

        Ok(resource)
    }

    /// Process JSON-LD content
    async fn process_json_ld(
        &self,
        content: &str,
        file_key: &str,
    ) -> Result<MdhResource, AppError> {
        let mut value: serde_json::Value = serde_json::from_str(content).map_err(AppError::Json)?;

        // Extract @context for processing
        let context = self.extract_context(&value)?;

        // Determine resource type and URI
        let resource_type = self.determine_type(&value, file_key)?;
        let uri = format!("mcp://mdh/{}", resource_type);

        // Compact the JSON-LD (remove @context, normalize structure)
        self.compact_json_ld(&mut value, &context)?;

        Ok(MdhResource { uri, data: value })
    }

    /// Extract @context from JSON-LD
    fn extract_context(&self, value: &serde_json::Value) -> Result<serde_json::Value, AppError> {
        match value {
            serde_json::Value::Object(map) => Ok(map
                .get("@context")
                .cloned()
                .unwrap_or(serde_json::Value::Null)),
            _ => Ok(serde_json::Value::Null),
        }
    }

    /// Determine resource type from JSON-LD
    fn determine_type(
        &self,
        value: &serde_json::Value,
        fallback: &str,
    ) -> Result<String, AppError> {
        if let serde_json::Value::Object(map) = value {
            // Try @type first
            if let Some(type_val) = map.get("@type") {
                if let Some(type_str) = type_val.as_str() {
                    return Ok(type_str.to_string());
                }
                if let Some(type_array) = type_val.as_array()
                    && let Some(first_type) = type_array.first().and_then(|v| v.as_str())
                {
                    return Ok(first_type.to_string());
                }
            }

            // Try other type indicators
            if let Some(schema_type) = map.get("type").and_then(|v| v.as_str()) {
                return Ok(schema_type.to_string());
            }
        }

        // Fallback to filename
        let path = PathBuf::from(fallback);
        Ok(path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string())
    }

    /// Compact JSON-LD by removing @context and normalizing
    fn compact_json_ld(
        &self,
        value: &mut serde_json::Value,
        _context: &serde_json::Value,
    ) -> Result<(), AppError> {
        if let serde_json::Value::Object(map) = value {
            // Remove @context
            map.remove("@context");

            // Normalize @id to id
            if let Some(id_val) = map.remove("@id") {
                map.insert("id".to_string(), id_val);
            }

            // Normalize @type to type
            if let Some(type_val) = map.remove("@type") {
                map.insert("type".to_string(), type_val);
            }
        }
        Ok(())
    }

    /// Clear cache
    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.cache.write() {
            cache.clear();
        }
        if let Ok(mut context_cache) = self.context_cache.write() {
            context_cache.clear();
        }
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> (usize, usize) {
        let cache_size = self.cache.read().map(|c| c.len()).unwrap_or(0);
        let context_size = self.context_cache.read().map(|c| c.len()).unwrap_or(0);
        (cache_size, context_size)
    }
}

/// Enhanced MDH translation function with caching
pub async fn mdh_translate_enhanced(ld_file: PathBuf) -> Result<MdhResource, AppError> {
    static TRANSLATOR: std::sync::OnceLock<MdhTranslator> = std::sync::OnceLock::new();
    let translator = TRANSLATOR.get_or_init(MdhTranslator::new);
    translator.translate(ld_file).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_mdh_translation() {
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        let json_ld = r#"{
            "@context": "https://schema.org",
            "@type": "Person",
            "@id": "https://example.com/person/1",
            "name": "John Doe",
            "email": "john@example.com"
        }"#;
        temp_file.write_all(json_ld.as_bytes()).unwrap();

        let translator = MdhTranslator::new();
        let result = translator.translate(temp_file.path().to_path_buf()).await;

        assert!(result.is_ok());
        let resource = result.unwrap();
        assert_eq!(resource.uri, "mcp://mdh/Person");
        assert!(resource.data.get("name").is_some());
        assert!(resource.data.get("@context").is_none()); // Should be removed
    }
}
