//! Query optimization and indexing for Gestura.app
//! Optimizes database queries and maintains indexes for fast lookups

use crate::AppError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Query optimization statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct QueryStats {
    pub total_queries: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub average_execution_time_ms: f64,
    pub index_usage_count: HashMap<String, usize>,
}

/// Index types for different data structures
#[derive(Debug, Clone)]
pub enum IndexType {
    Hash,
    BTree,
    FullText,
    Composite(Vec<String>),
}

/// Index definition
#[derive(Debug, Clone)]
pub struct IndexDefinition {
    pub name: String,
    pub table: String,
    pub columns: Vec<String>,
    pub index_type: IndexType,
    pub is_unique: bool,
}

/// Query execution plan
#[derive(Debug, Clone, serde::Serialize)]
pub struct QueryPlan {
    pub query_id: String,
    pub estimated_cost: f64,
    pub execution_steps: Vec<ExecutionStep>,
    pub indexes_used: Vec<String>,
    pub estimated_rows: usize,
}

/// Execution step in query plan
#[derive(Debug, Clone, serde::Serialize)]
pub enum ExecutionStep {
    IndexScan { index_name: String, filter: String },
    TableScan { table_name: String },
    Filter { condition: String },
    Sort { columns: Vec<String> },
    Join { join_type: String, condition: String },
    Aggregate { functions: Vec<String> },
}

/// Query optimizer and index manager
pub struct QueryOptimizer {
    indexes: Arc<RwLock<HashMap<String, IndexDefinition>>>,
    query_cache: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    query_stats: Arc<RwLock<QueryStats>>,
    execution_history: Arc<RwLock<Vec<QueryExecution>>>,
    max_cache_size: usize,
    max_history_size: usize,
}

/// Query execution record
#[derive(Debug, Clone)]
pub struct QueryExecution {
    pub query_hash: String,
    pub execution_time_ms: f64,
    pub rows_returned: usize,
    pub indexes_used: Vec<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl QueryOptimizer {
    /// Create a new query optimizer
    pub fn new(max_cache_size: usize, max_history_size: usize) -> Self {
        Self {
            indexes: Arc::new(RwLock::new(HashMap::new())),
            query_cache: Arc::new(RwLock::new(HashMap::new())),
            query_stats: Arc::new(RwLock::new(QueryStats {
                total_queries: 0,
                cache_hits: 0,
                cache_misses: 0,
                average_execution_time_ms: 0.0,
                index_usage_count: HashMap::new(),
            })),
            execution_history: Arc::new(RwLock::new(Vec::new())),
            max_cache_size,
            max_history_size,
        }
    }

    /// Create an index
    pub async fn create_index(&self, definition: IndexDefinition) -> Result<(), AppError> {
        let mut indexes = self.indexes.write().await;
        
        if indexes.contains_key(&definition.name) {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("Index {} already exists", definition.name)
            )));
        }

        // Validate index definition
        if definition.columns.is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Index must have at least one column"
            )));
        }

        indexes.insert(definition.name.clone(), definition.clone());
        tracing::info!("Created index: {} on table: {}", definition.name, definition.table);
        Ok(())
    }

    /// Drop an index
    pub async fn drop_index(&self, index_name: &str) -> Result<(), AppError> {
        let mut indexes = self.indexes.write().await;
        
        if indexes.remove(index_name).is_some() {
            tracing::info!("Dropped index: {}", index_name);
            Ok(())
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Index {} not found", index_name)
            )))
        }
    }

    /// Optimize a query and return execution plan
    pub async fn optimize_query(&self, query: &str) -> Result<QueryPlan, AppError> {
        let query_hash = self.hash_query(query);
        
        // Check if we have a cached plan
        if let Some(cached_plan) = self.get_cached_plan(&query_hash).await {
            return Ok(cached_plan);
        }

        // Analyze query and create execution plan
        let plan = self.analyze_query(query).await?;
        
        // Cache the plan
        self.cache_plan(&query_hash, &plan).await;
        
        Ok(plan)
    }

    /// Execute a query with optimization
    pub async fn execute_query(&self, query: &str) -> Result<serde_json::Value, AppError> {
        let start_time = std::time::Instant::now();
        let query_hash = self.hash_query(query);
        
        // Check query cache first
        if let Some(cached_result) = self.get_cached_result(&query_hash).await {
            self.record_cache_hit().await;
            return Ok(cached_result);
        }

        self.record_cache_miss().await;

        // Get optimized execution plan
        let plan = self.optimize_query(query).await?;
        
        // Execute the query (simplified - in real implementation would execute against actual DB)
        let result = self.execute_plan(&plan).await?;
        
        let execution_time = start_time.elapsed().as_millis() as f64;
        
        // Record execution statistics
        self.record_execution(QueryExecution {
            query_hash: query_hash.clone(),
            execution_time_ms: execution_time,
            rows_returned: self.count_result_rows(&result),
            indexes_used: plan.indexes_used.clone(),
            timestamp: chrono::Utc::now(),
        }).await;

        // Cache the result
        self.cache_result(&query_hash, &result).await;
        
        Ok(result)
    }

    /// Analyze query and create execution plan
    async fn analyze_query(&self, query: &str) -> Result<QueryPlan, AppError> {
        let query_id = uuid::Uuid::new_v4().to_string();
        let mut execution_steps = Vec::new();
        let mut indexes_used = Vec::new();
        let mut estimated_cost = 0.0;
        let mut estimated_rows = 1000; // Default estimate

        // Simple query analysis (in real implementation, use proper SQL parser)
        let query_lower = query.to_lowercase();
        
        // Check for table scans
        if query_lower.contains("select") {
            if let Some(table_name) = self.extract_table_name(&query_lower) {
                // Check if we can use an index
                if let Some(index_name) = self.find_best_index(&table_name, &query_lower).await {
                    execution_steps.push(ExecutionStep::IndexScan {
                        index_name: index_name.clone(),
                        filter: self.extract_where_clause(&query_lower),
                    });
                    indexes_used.push(index_name);
                    estimated_cost += 10.0; // Index scan cost
                    estimated_rows = 100; // Estimated rows from index
                } else {
                    execution_steps.push(ExecutionStep::TableScan {
                        table_name: table_name.clone(),
                    });
                    estimated_cost += 1000.0; // Table scan cost
                }
            }
        }

        // Check for WHERE clause
        if query_lower.contains("where") {
            execution_steps.push(ExecutionStep::Filter {
                condition: self.extract_where_clause(&query_lower),
            });
            estimated_cost += 5.0;
            estimated_rows = (estimated_rows as f64 * 0.1) as usize; // Assume 10% selectivity
        }

        // Check for ORDER BY
        if query_lower.contains("order by") {
            let sort_columns = self.extract_order_by_columns(&query_lower);
            execution_steps.push(ExecutionStep::Sort {
                columns: sort_columns,
            });
            estimated_cost += estimated_rows as f64 * 0.01; // Sort cost
        }

        // Check for GROUP BY or aggregates
        if query_lower.contains("group by") || query_lower.contains("count(") || 
           query_lower.contains("sum(") || query_lower.contains("avg(") {
            execution_steps.push(ExecutionStep::Aggregate {
                functions: self.extract_aggregate_functions(&query_lower),
            });
            estimated_cost += estimated_rows as f64 * 0.005;
            estimated_rows = (estimated_rows as f64 * 0.01) as usize; // Aggregation reduces rows
        }

        Ok(QueryPlan {
            query_id,
            estimated_cost,
            execution_steps,
            indexes_used,
            estimated_rows,
        })
    }

    /// Find the best index for a query
    async fn find_best_index(&self, table_name: &str, query: &str) -> Option<String> {
        let indexes = self.indexes.read().await;
        let mut best_index = None;
        let mut best_score = 0.0;

        for (index_name, index_def) in indexes.iter() {
            if index_def.table != table_name {
                continue;
            }

            let score = self.calculate_index_score(index_def, query);
            if score > best_score {
                best_score = score;
                best_index = Some(index_name.clone());
            }
        }

        best_index
    }

    /// Calculate how well an index matches a query
    fn calculate_index_score(&self, index_def: &IndexDefinition, query: &str) -> f64 {
        let mut score = 0.0;
        
        // Check if index columns are mentioned in WHERE clause
        for column in &index_def.columns {
            if query.contains(column) {
                score += 10.0;
                
                // Bonus for equality conditions
                if query.contains(&format!("{} =", column)) {
                    score += 5.0;
                }
            }
        }

        // Bonus for unique indexes
        if index_def.is_unique {
            score += 2.0;
        }

        // Penalty for composite indexes with unused columns
        if index_def.columns.len() > 1 {
            let unused_columns = index_def.columns.iter()
                .filter(|col| !query.contains(*col))
                .count();
            score -= unused_columns as f64 * 2.0;
        }

        score
    }

    /// Execute a query plan (simplified)
    async fn execute_plan(&self, plan: &QueryPlan) -> Result<serde_json::Value, AppError> {
        // This is a simplified execution - in real implementation would execute against actual database
        let mut result_rows = Vec::new();
        
        for step in &plan.execution_steps {
            match step {
                ExecutionStep::IndexScan { index_name, filter } => {
                    tracing::debug!("Executing index scan on {} with filter: {}", index_name, filter);
                    // Simulate index scan results
                    for i in 0..plan.estimated_rows.min(100) {
                        result_rows.push(serde_json::json!({
                            "id": i,
                            "data": format!("row_{}", i),
                            "source": "index_scan"
                        }));
                    }
                }
                ExecutionStep::TableScan { table_name } => {
                    tracing::debug!("Executing table scan on {}", table_name);
                    // Simulate table scan results
                    for i in 0..plan.estimated_rows.min(1000) {
                        result_rows.push(serde_json::json!({
                            "id": i,
                            "data": format!("row_{}", i),
                            "source": "table_scan"
                        }));
                    }
                }
                ExecutionStep::Filter { condition } => {
                    tracing::debug!("Applying filter: {}", condition);
                    // Simulate filtering (keep 10% of rows)
                    let filtered_count = result_rows.len() / 10;
                    result_rows.truncate(filtered_count);
                }
                ExecutionStep::Sort { columns } => {
                    tracing::debug!("Sorting by columns: {:?}", columns);
                    // Simulate sorting (no actual sorting in this mock)
                }
                ExecutionStep::Aggregate { functions } => {
                    tracing::debug!("Applying aggregates: {:?}", functions);
                    // Simulate aggregation
                    result_rows = vec![serde_json::json!({
                        "count": result_rows.len(),
                        "aggregated": true
                    })];
                }
                ExecutionStep::Join { join_type, condition } => {
                    tracing::debug!("Executing {} join with condition: {}", join_type, condition);
                    // Simulate join (double the rows)
                    let original_count = result_rows.len();
                    for i in 0..original_count {
                        result_rows.push(result_rows[i].clone());
                    }
                }
            }
        }

        Ok(serde_json::json!({
            "rows": result_rows,
            "execution_plan": plan,
            "timestamp": chrono::Utc::now()
        }))
    }

    /// Simple query parsing helpers
    fn extract_table_name(&self, query: &str) -> Option<String> {
        // Very simple table name extraction
        if let Some(from_pos) = query.find("from ") {
            let after_from = &query[from_pos + 5..];
            if let Some(space_pos) = after_from.find(' ') {
                Some(after_from[..space_pos].trim().to_string())
            } else {
                Some(after_from.trim().to_string())
            }
        } else {
            None
        }
    }

    fn extract_where_clause(&self, query: &str) -> String {
        if let Some(where_pos) = query.find("where ") {
            let after_where = &query[where_pos + 6..];
            if let Some(end_pos) = after_where.find(" order by")
                .or_else(|| after_where.find(" group by"))
                .or_else(|| after_where.find(" limit")) {
                after_where[..end_pos].trim().to_string()
            } else {
                after_where.trim().to_string()
            }
        } else {
            String::new()
        }
    }

    fn extract_order_by_columns(&self, query: &str) -> Vec<String> {
        if let Some(order_pos) = query.find("order by ") {
            let after_order = &query[order_pos + 9..];
            let columns_str = if let Some(end_pos) = after_order.find(" limit") {
                &after_order[..end_pos]
            } else {
                after_order
            };
            
            columns_str.split(',')
                .map(|col| col.trim().to_string())
                .collect()
        } else {
            Vec::new()
        }
    }

    fn extract_aggregate_functions(&self, query: &str) -> Vec<String> {
        let mut functions = Vec::new();
        let aggregates = ["count(", "sum(", "avg(", "min(", "max("];
        
        for agg in &aggregates {
            if query.contains(agg) {
                functions.push(agg.trim_end_matches('(').to_string());
            }
        }
        
        functions
    }

    /// Utility functions
    fn hash_query(&self, query: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    fn count_result_rows(&self, result: &serde_json::Value) -> usize {
        result.get("rows")
            .and_then(|rows| rows.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0)
    }

    /// Cache management
    async fn get_cached_plan(&self, _query_hash: &str) -> Option<QueryPlan> {
        // For simplicity, not implementing plan caching in this mock
        None
    }

    async fn cache_plan(&self, _query_hash: &str, _plan: &QueryPlan) {
        // Plan caching would be implemented here
    }

    async fn get_cached_result(&self, query_hash: &str) -> Option<serde_json::Value> {
        let cache = self.query_cache.read().await;
        cache.get(query_hash).cloned()
    }

    async fn cache_result(&self, query_hash: &str, result: &serde_json::Value) {
        let mut cache = self.query_cache.write().await;
        
        if cache.len() >= self.max_cache_size {
            // Simple LRU: remove first entry
            if let Some(first_key) = cache.keys().next().cloned() {
                cache.remove(&first_key);
            }
        }
        
        cache.insert(query_hash.to_string(), result.clone());
    }

    /// Statistics tracking
    async fn record_cache_hit(&self) {
        let mut stats = self.query_stats.write().await;
        stats.cache_hits += 1;
        stats.total_queries += 1;
    }

    async fn record_cache_miss(&self) {
        let mut stats = self.query_stats.write().await;
        stats.cache_misses += 1;
        stats.total_queries += 1;
    }

    async fn record_execution(&self, execution: QueryExecution) {
        // Update statistics
        {
            let mut stats = self.query_stats.write().await;
            
            // Update average execution time
            let total_time = stats.average_execution_time_ms * (stats.total_queries - 1) as f64 + execution.execution_time_ms;
            stats.average_execution_time_ms = total_time / stats.total_queries as f64;
            
            // Update index usage counts
            for index_name in &execution.indexes_used {
                *stats.index_usage_count.entry(index_name.clone()).or_insert(0) += 1;
            }
        }

        // Store execution history
        let mut history = self.execution_history.write().await;
        history.push(execution);
        
        if history.len() > self.max_history_size {
            history.remove(0);
        }
    }

    /// Get query statistics
    pub async fn get_stats(&self) -> QueryStats {
        self.query_stats.read().await.clone()
    }

    /// Get all indexes
    pub async fn get_indexes(&self) -> Vec<IndexDefinition> {
        let indexes = self.indexes.read().await;
        indexes.values().cloned().collect()
    }

    /// Clear query cache
    pub async fn clear_cache(&self) {
        let mut cache = self.query_cache.write().await;
        cache.clear();
        tracing::info!("Query cache cleared");
    }
}

/// Global query optimizer instance
static QUERY_OPTIMIZER: tokio::sync::OnceCell<QueryOptimizer> = tokio::sync::OnceCell::const_new();

/// Get the global query optimizer
pub async fn get_query_optimizer() -> &'static QueryOptimizer {
    QUERY_OPTIMIZER.get_or_init(|| async {
        QueryOptimizer::new(1000, 10000)
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_index_creation() {
        let optimizer = QueryOptimizer::new(100, 1000);
        
        let index_def = IndexDefinition {
            name: "idx_user_email".to_string(),
            table: "users".to_string(),
            columns: vec!["email".to_string()],
            index_type: IndexType::Hash,
            is_unique: true,
        };
        
        optimizer.create_index(index_def).await.unwrap();
        
        let indexes = optimizer.get_indexes().await;
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].name, "idx_user_email");
    }

    #[tokio::test]
    async fn test_query_optimization() {
        let optimizer = QueryOptimizer::new(100, 1000);
        
        // Create an index
        let index_def = IndexDefinition {
            name: "idx_users_id".to_string(),
            table: "users".to_string(),
            columns: vec!["id".to_string()],
            index_type: IndexType::BTree,
            is_unique: true,
        };
        optimizer.create_index(index_def).await.unwrap();
        
        // Optimize a query
        let query = "SELECT * FROM users WHERE id = 123";
        let plan = optimizer.optimize_query(query).await.unwrap();
        
        assert!(!plan.execution_steps.is_empty());
        assert!(plan.estimated_cost > 0.0);
    }

    #[tokio::test]
    async fn test_query_execution() {
        let optimizer = QueryOptimizer::new(100, 1000);
        
        let query = "SELECT * FROM users WHERE name = 'test'";
        let result = optimizer.execute_query(query).await.unwrap();
        
        assert!(result.get("rows").is_some());
        assert!(result.get("execution_plan").is_some());
        
        // Test caching
        let result2 = optimizer.execute_query(query).await.unwrap();
        assert_eq!(result, result2);
        
        let stats = optimizer.get_stats().await;
        assert_eq!(stats.total_queries, 2);
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(stats.cache_misses, 1);
    }
}
