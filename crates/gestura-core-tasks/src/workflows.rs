//! Workflow management system for prompt templates and automation
//!
//! This module provides a workflow management system that discovers, loads, and executes
//! workflow files (markdown-based prompt templates) from the `.gestura/workflows/` directory.
//!
//! # Architecture
//!
//! ```text
//! .gestura/workflows/
//! ├── code-review.md         # Workflow for code review
//! ├── bug-fix.md             # Workflow for bug fixing
//! └── feature-planning.md    # Workflow for feature planning
//! ```
//!
//! # Workflow File Format
//!
//! Workflows are markdown files with optional YAML frontmatter:
//!
//! ```markdown
//! ---
//! description: Review code for best practices and potential issues
//! tags: [code, review, quality]
//! ---
//!
//! Please review the following code for:
//! - Code quality and best practices
//! - Potential bugs or edge cases
//! - Performance optimizations
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use gestura_core::workflows::WorkflowManager;
//!
//! let manager = WorkflowManager::new();
//! let workflows = manager.list_workflows()?;
//! let content = manager.load_workflow("code-review")?;
//! ```

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// A workflow represents a reusable prompt template
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// Workflow name (derived from filename without extension)
    pub name: String,
    /// Human-readable description (from frontmatter or default)
    pub description: String,
    /// Optional tags for categorization
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The actual prompt content (frontmatter stripped)
    pub content: String,
}

/// Workflow metadata for listing (without loading full content)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInfo {
    /// Workflow name
    pub name: String,
    /// Description
    pub description: String,
    /// Tags
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Error type for workflow operations
#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    /// Workflow not found
    #[error("Workflow not found: {0}")]
    NotFound(String),
    /// Invalid workflow format
    #[error("Invalid workflow format: {0}")]
    InvalidFormat(String),
    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// YAML parsing error
    #[error("YAML parsing error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

/// Workflow manager for discovering and loading workflow files
pub struct WorkflowManager {
    /// Base directory for workflow files
    workflows_dir: PathBuf,
}

impl WorkflowManager {
    /// Create a new workflow manager
    ///
    /// This will use the default workflows directory:
    /// 1. `.gestura/workflows/` in current directory (if exists)
    /// 2. `~/.local/share/gestura/workflows/` (fallback)
    pub fn new() -> Self {
        Self {
            workflows_dir: Self::default_workflows_dir(),
        }
    }

    /// Create a workflow manager with a custom directory
    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            workflows_dir: dir.into(),
        }
    }

    /// Get the default workflows directory
    ///
    /// Precedence:
    /// 1. `.gestura/workflows/` in current directory (if exists)
    /// 2. `~/.local/share/gestura/workflows/` (fallback)
    pub fn default_workflows_dir() -> PathBuf {
        let current = PathBuf::from(".gestura/workflows");
        if current.exists() {
            return current;
        }

        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("gestura")
            .join("workflows")
    }

    /// Get the workflows directory path
    pub fn workflows_dir(&self) -> &Path {
        &self.workflows_dir
    }

    /// List all available workflows (metadata only, no content)
    pub fn list_workflows(&self) -> Result<Vec<WorkflowInfo>, WorkflowError> {
        let mut workflows = Vec::new();

        if !self.workflows_dir.exists() {
            return Ok(workflows);
        }

        for entry in fs::read_dir(&self.workflows_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Only process .md files
            if path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| WorkflowError::InvalidFormat("Invalid filename".to_string()))?
                .to_string();

            // Read file to extract frontmatter
            let content = fs::read_to_string(&path)?;
            let (description, tags) = Self::parse_frontmatter(&content);

            workflows.push(WorkflowInfo {
                name,
                description,
                tags,
            });
        }

        // Sort by name for consistent ordering
        workflows.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(workflows)
    }

    /// Load a workflow by name
    ///
    /// The name should be the filename without the `.md` extension.
    /// For example, to load `code-review.md`, use `load_workflow("code-review")`.
    pub fn load_workflow(&self, name: &str) -> Result<Workflow, WorkflowError> {
        let filename = if name.ends_with(".md") {
            name.to_string()
        } else {
            format!("{}.md", name)
        };

        let path = self.workflows_dir.join(&filename);
        if !path.exists() {
            return Err(WorkflowError::NotFound(name.to_string()));
        }

        let content = fs::read_to_string(&path)?;
        let (description, tags) = Self::parse_frontmatter(&content);
        let content = Self::strip_frontmatter(&content);

        Ok(Workflow {
            name: name.trim_end_matches(".md").to_string(),
            description,
            tags,
            content,
        })
    }

    /// Parse frontmatter from workflow content
    ///
    /// Returns (description, tags) tuple
    fn parse_frontmatter(content: &str) -> (String, Vec<String>) {
        let mut description = "No description".to_string();
        let mut tags = Vec::new();

        if let Some(stripped) = content.strip_prefix("---") {
            if let Some(end_idx) = stripped.find("---") {
                let frontmatter = &stripped[..end_idx];

                // Parse YAML frontmatter
                if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter) {
                    if let Some(desc) = yaml.get("description").and_then(|v| v.as_str()) {
                        description = desc.to_string();
                    }
                    if let Some(tag_list) = yaml.get("tags").and_then(|v| v.as_sequence()) {
                        tags = tag_list
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();
                    }
                }
            }
        } else {
            // No frontmatter, try to extract description from first line
            if let Some(first_line) = content.lines().find(|l| !l.trim().is_empty())
                && first_line.starts_with("description:")
            {
                description = first_line
                    .trim_start_matches("description:")
                    .trim()
                    .to_string();
            }
        }

        (description, tags)
    }

    /// Strip frontmatter from workflow content
    fn strip_frontmatter(content: &str) -> String {
        if let Some(stripped) = content.strip_prefix("---")
            && let Some(end_idx) = stripped.find("---")
        {
            return stripped[end_idx + 3..].trim().to_string();
        }
        content.to_string()
    }

    /// Check if a workflow exists
    pub fn workflow_exists(&self, name: &str) -> bool {
        let filename = if name.ends_with(".md") {
            name.to_string()
        } else {
            format!("{}.md", name)
        };
        self.workflows_dir.join(filename).exists()
    }

    /// Create the workflows directory if it doesn't exist
    pub fn ensure_workflows_dir(&self) -> Result<(), WorkflowError> {
        if !self.workflows_dir.exists() {
            fs::create_dir_all(&self.workflows_dir)?;
        }
        Ok(())
    }
}

impl Default for WorkflowManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_workflow_manager_list_empty() {
        let temp_dir = TempDir::new().unwrap();
        let manager = WorkflowManager::with_dir(temp_dir.path());

        let workflows = manager.list_workflows().unwrap();
        assert_eq!(workflows.len(), 0);
    }

    #[test]
    fn test_workflow_manager_load_workflow() {
        let temp_dir = TempDir::new().unwrap();
        let manager = WorkflowManager::with_dir(temp_dir.path());

        // Create a test workflow
        manager.ensure_workflows_dir().unwrap();
        let workflow_path = temp_dir.path().join("test.md");
        fs::write(
            &workflow_path,
            "---\ndescription: Test workflow\ntags: [test, example]\n---\n\nTest content",
        )
        .unwrap();

        let workflow = manager.load_workflow("test").unwrap();
        assert_eq!(workflow.name, "test");
        assert_eq!(workflow.description, "Test workflow");
        assert_eq!(workflow.tags, vec!["test", "example"]);
        assert_eq!(workflow.content, "Test content");
    }

    #[test]
    fn test_workflow_manager_list_workflows() {
        let temp_dir = TempDir::new().unwrap();
        let manager = WorkflowManager::with_dir(temp_dir.path());

        manager.ensure_workflows_dir().unwrap();
        fs::write(
            temp_dir.path().join("workflow1.md"),
            "---\ndescription: First workflow\n---\n\nContent 1",
        )
        .unwrap();
        fs::write(
            temp_dir.path().join("workflow2.md"),
            "---\ndescription: Second workflow\n---\n\nContent 2",
        )
        .unwrap();

        let workflows = manager.list_workflows().unwrap();
        assert_eq!(workflows.len(), 2);
        assert_eq!(workflows[0].name, "workflow1");
        assert_eq!(workflows[1].name, "workflow2");
    }
}
