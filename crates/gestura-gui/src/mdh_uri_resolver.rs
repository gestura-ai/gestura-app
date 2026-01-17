//! Dataset URI resolution for MDH (Metadata Harmony)
//! Resolves URIs to actual datasets and handles caching

use crate::AppError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// URI resolution result
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResolvedDataset {
    pub uri: String,
    pub local_path: Option<PathBuf>,
    pub remote_url: Option<String>,
    pub content_type: String,
    pub size: Option<u64>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// URI resolution strategy
#[derive(Debug, Clone)]
pub enum ResolutionStrategy {
    LocalFirst,
    RemoteFirst,
    LocalOnly,
    RemoteOnly,
}

/// Dataset URI resolver
pub struct DatasetUriResolver {
    cache: Arc<RwLock<HashMap<String, ResolvedDataset>>>,
    local_repositories: Arc<RwLock<Vec<PathBuf>>>,
    remote_repositories: Arc<RwLock<Vec<String>>>,
    strategy: ResolutionStrategy,
    cache_ttl: chrono::Duration,
}

impl DatasetUriResolver {
    /// Create a new URI resolver
    pub fn new(strategy: ResolutionStrategy, cache_ttl_hours: i64) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            local_repositories: Arc::new(RwLock::new(Vec::new())),
            remote_repositories: Arc::new(RwLock::new(Vec::new())),
            strategy,
            cache_ttl: chrono::Duration::hours(cache_ttl_hours),
        }
    }

    /// Add a local repository path
    pub async fn add_local_repository(&self, path: PathBuf) {
        let mut repos = self.local_repositories.write().await;
        if !repos.contains(&path) {
            repos.push(path);
        }
    }

    /// Add a remote repository URL
    pub async fn add_remote_repository(&self, url: String) {
        let mut repos = self.remote_repositories.write().await;
        if !repos.contains(&url) {
            repos.push(url);
        }
    }

    /// Resolve a dataset URI
    pub async fn resolve(&self, uri: &str) -> Result<ResolvedDataset, AppError> {
        // Check cache first
        if let Some(cached) = self.get_cached(uri).await {
            return Ok(cached);
        }

        // Resolve based on strategy
        let resolved = match self.strategy {
            ResolutionStrategy::LocalFirst => match self.resolve_local(uri).await {
                Ok(result) => result,
                Err(_) => self.resolve_remote(uri).await?,
            },
            ResolutionStrategy::RemoteFirst => match self.resolve_remote(uri).await {
                Ok(result) => result,
                Err(_) => self.resolve_local(uri).await?,
            },
            ResolutionStrategy::LocalOnly => self.resolve_local(uri).await?,
            ResolutionStrategy::RemoteOnly => self.resolve_remote(uri).await?,
        };

        // Cache the result
        self.cache_result(uri, &resolved).await;

        Ok(resolved)
    }

    /// Resolve URI locally
    async fn resolve_local(&self, uri: &str) -> Result<ResolvedDataset, AppError> {
        let repos = self.local_repositories.read().await;

        for repo_path in repos.iter() {
            // Try different URI-to-path mappings
            let potential_paths = self.uri_to_local_paths(uri, repo_path);

            for path in potential_paths {
                if path.exists() {
                    let metadata = tokio::fs::metadata(&path).await.map_err(AppError::Io)?;

                    let content_type = self.detect_content_type(&path);

                    return Ok(ResolvedDataset {
                        uri: uri.to_string(),
                        local_path: Some(path),
                        remote_url: None,
                        content_type,
                        size: Some(metadata.len()),
                        last_modified: metadata.modified().ok().map(chrono::DateTime::from),
                        metadata: HashMap::new(),
                    });
                }
            }
        }

        Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Dataset not found locally: {}", uri),
        )))
    }

    /// Resolve URI remotely
    async fn resolve_remote(&self, uri: &str) -> Result<ResolvedDataset, AppError> {
        let repos = self.remote_repositories.read().await;

        for repo_url in repos.iter() {
            let full_url = if uri.starts_with("http") {
                uri.to_string()
            } else {
                format!(
                    "{}/{}",
                    repo_url.trim_end_matches('/'),
                    uri.trim_start_matches('/')
                )
            };

            // Try to fetch metadata (HEAD request)
            match self.fetch_remote_metadata(&full_url).await {
                Ok(dataset) => return Ok(dataset),
                Err(_) => continue,
            }
        }

        Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Dataset not found remotely: {}", uri),
        )))
    }

    /// Convert URI to potential local paths
    fn uri_to_local_paths(&self, uri: &str, repo_path: &std::path::Path) -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // Remove protocol if present
        let clean_uri = uri
            .strip_prefix("dataset://")
            .or_else(|| uri.strip_prefix("mdh://"))
            .unwrap_or(uri);

        // Try different path structures
        paths.push(repo_path.join(clean_uri));
        paths.push(repo_path.join(format!("{}.json", clean_uri)));
        paths.push(repo_path.join(format!("{}.jsonld", clean_uri)));
        paths.push(repo_path.join(format!("{}/metadata.json", clean_uri)));

        // Handle hierarchical URIs
        if clean_uri.contains('/') {
            let parts: Vec<&str> = clean_uri.split('/').collect();
            if parts.len() >= 2 {
                let org = parts[0];
                let dataset = parts[1];
                paths.push(repo_path.join(org).join(dataset).join("dataset.json"));
                paths.push(repo_path.join(org).join(format!("{}.json", dataset)));
            }
        }

        paths
    }

    /// Detect content type from file extension
    fn detect_content_type(&self, path: &std::path::Path) -> String {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("json") => "application/json".to_string(),
            Some("jsonld") => "application/ld+json".to_string(),
            Some("ttl") => "text/turtle".to_string(),
            Some("rdf") => "application/rdf+xml".to_string(),
            Some("csv") => "text/csv".to_string(),
            Some("xml") => "application/xml".to_string(),
            _ => "application/octet-stream".to_string(),
        }
    }

    /// Fetch remote metadata
    async fn fetch_remote_metadata(&self, url: &str) -> Result<ResolvedDataset, AppError> {
        // In a real implementation, this would make HTTP requests
        // For now, return a mock result
        tracing::info!("Fetching remote metadata for: {}", url);

        Ok(ResolvedDataset {
            uri: url.to_string(),
            local_path: None,
            remote_url: Some(url.to_string()),
            content_type: "application/ld+json".to_string(),
            size: None,
            last_modified: Some(chrono::Utc::now()),
            metadata: HashMap::from([
                (
                    "source".to_string(),
                    serde_json::Value::String("remote".to_string()),
                ),
                (
                    "status".to_string(),
                    serde_json::Value::String("available".to_string()),
                ),
            ]),
        })
    }

    /// Get cached result
    async fn get_cached(&self, uri: &str) -> Option<ResolvedDataset> {
        let cache = self.cache.read().await;
        if let Some(cached) = cache.get(uri) {
            // Check if cache is still valid
            if let Some(last_modified) = cached.last_modified
                && chrono::Utc::now() - last_modified < self.cache_ttl
            {
                return Some(cached.clone());
            }
        }
        None
    }

    /// Cache resolution result
    async fn cache_result(&self, uri: &str, result: &ResolvedDataset) {
        let mut cache = self.cache.write().await;
        cache.insert(uri.to_string(), result.clone());

        // Limit cache size
        if cache.len() > 1000 {
            // Remove oldest entries (simple LRU approximation)
            let keys_to_remove: Vec<String> = cache.keys().take(100).cloned().collect();
            for key in keys_to_remove {
                cache.remove(&key);
            }
        }
    }

    /// Clear cache
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        tracing::info!("Dataset URI cache cleared");
    }

    /// Get cache statistics
    pub async fn get_cache_stats(&self) -> serde_json::Value {
        let cache = self.cache.read().await;
        let local_repos = self.local_repositories.read().await;
        let remote_repos = self.remote_repositories.read().await;

        serde_json::json!({
            "cache_size": cache.len(),
            "local_repositories": local_repos.len(),
            "remote_repositories": remote_repos.len(),
            "strategy": format!("{:?}", self.strategy),
            "cache_ttl_hours": self.cache_ttl.num_hours()
        })
    }

    /// Batch resolve multiple URIs
    pub async fn batch_resolve(
        &self,
        uris: Vec<String>,
    ) -> HashMap<String, Result<ResolvedDataset, String>> {
        let mut results = HashMap::new();

        // Process in parallel
        let futures: Vec<_> = uris
            .into_iter()
            .map(|uri| {
                let resolver = self;
                async move {
                    let result = resolver.resolve(&uri).await.map_err(|e| e.to_string());
                    (uri, result)
                }
            })
            .collect();

        let resolved = futures::future::join_all(futures).await;
        for (uri, result) in resolved {
            results.insert(uri, result);
        }

        results
    }
}

/// Global URI resolver instance
static URI_RESOLVER: tokio::sync::OnceCell<DatasetUriResolver> = tokio::sync::OnceCell::const_new();

/// Get the global URI resolver
pub async fn get_uri_resolver() -> &'static DatasetUriResolver {
    URI_RESOLVER
        .get_or_init(|| async {
            let resolver = DatasetUriResolver::new(ResolutionStrategy::LocalFirst, 24);

            // Add default repositories
            resolver
                .add_local_repository(std::env::current_dir().unwrap().join("datasets"))
                .await;
            resolver
                .add_remote_repository("https://datasets.gestura.ai".to_string())
                .await;

            resolver
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_uri_resolver() {
        let temp_dir = TempDir::new().unwrap();
        let resolver = DatasetUriResolver::new(ResolutionStrategy::LocalFirst, 1);

        // Add temp directory as repository
        resolver
            .add_local_repository(temp_dir.path().to_path_buf())
            .await;

        // Create a test dataset file
        let dataset_path = temp_dir.path().join("test-dataset.json");
        tokio::fs::write(&dataset_path, r#"{"name": "test"}"#)
            .await
            .unwrap();

        // Test resolution
        let result = resolver.resolve("test-dataset").await;
        assert!(result.is_ok());

        let dataset = result.unwrap();
        assert_eq!(dataset.uri, "test-dataset");
        assert!(dataset.local_path.is_some());
        assert_eq!(dataset.content_type, "application/json");
    }

    #[tokio::test]
    async fn test_cache() {
        let resolver = DatasetUriResolver::new(ResolutionStrategy::RemoteOnly, 1);
        resolver
            .add_remote_repository("https://example.com".to_string())
            .await;

        // First resolution (will be cached)
        let result1 = resolver.resolve("test-uri").await;
        assert!(result1.is_ok());

        // Second resolution (should use cache)
        let result2 = resolver.resolve("test-uri").await;
        assert!(result2.is_ok());

        // Results should be identical
        assert_eq!(result1.unwrap().uri, result2.unwrap().uri);
    }
}
