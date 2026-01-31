//! Task management system for tracking agent workflows
//!
//! This module provides a task management system that integrates with the agent loop
//! to track complex workflows, subtasks, and progress throughout conversation sessions.
//!
//! # Architecture
//!
//! ```text
//! .gestura/tasks/
//! ├── {session_id_1}.json    # Tasks for session 1
//! ├── {session_id_2}.json    # Tasks for session 2
//! └── ...
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use gestura_core::tasks::{TaskManager, Task, TaskStatus};
//!
//! let manager = TaskManager::new("/path/to/workspace");
//! let task = manager.create_task("session-123", "Implement feature", "Add new API endpoint", None)?;
//! manager.update_task_status("session-123", &task.id, TaskStatus::InProgress)?;
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Status of a task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Task has not been started
    NotStarted,
    /// Task is currently in progress
    InProgress,
    /// Task has been completed
    Completed,
    /// Task has been cancelled
    Cancelled,
}

/// Source of a task (who created it)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TaskSource {
    /// Created manually by user via the task panel UI
    #[default]
    User,
    /// Created automatically by the agent during processing
    Agent,
    /// Created by the orchestrator for workflow delegation
    Orchestrator,
}

/// A task represents a unit of work to be tracked
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique identifier for this task
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Detailed description
    pub description: String,
    /// Current status
    pub status: TaskStatus,
    /// Parent task ID (for subtasks)
    pub parent_id: Option<String>,
    /// When the task was created
    pub created_at: DateTime<Utc>,
    /// When the task was last updated
    pub updated_at: DateTime<Utc>,
    /// Session ID this task belongs to
    pub session_id: String,
    /// Source of the task (user, agent, or orchestrator)
    #[serde(default)]
    pub source: TaskSource,
    /// ID linking to an orchestrator DelegatedTask (for bidirectional sync)
    #[serde(default)]
    pub orchestrator_task_id: Option<String>,
    /// ID of the agent that created/owns this task
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Additional metadata (tool calls, output, context, etc.)
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

impl Task {
    /// Create a new task (defaults to User source)
    pub fn new(
        session_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        parent_id: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            description: description.into(),
            status: TaskStatus::NotStarted,
            parent_id,
            created_at: now,
            updated_at: now,
            session_id: session_id.into(),
            source: TaskSource::User,
            orchestrator_task_id: None,
            agent_id: None,
            metadata: None,
        }
    }

    /// Create a new task with a specific source
    pub fn new_with_source(
        session_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        parent_id: Option<String>,
        source: TaskSource,
        agent_id: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            description: description.into(),
            status: TaskStatus::NotStarted,
            parent_id,
            created_at: now,
            updated_at: now,
            session_id: session_id.into(),
            source,
            orchestrator_task_id: None,
            agent_id,
            metadata: None,
        }
    }

    /// Create a task from an orchestrator delegated task
    pub fn from_orchestrator_task(
        session_id: impl Into<String>,
        orchestrator_task_id: impl Into<String>,
        agent_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        context: Option<serde_json::Value>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            description: description.into(),
            status: TaskStatus::NotStarted,
            parent_id: None,
            created_at: now,
            updated_at: now,
            session_id: session_id.into(),
            source: TaskSource::Orchestrator,
            orchestrator_task_id: Some(orchestrator_task_id.into()),
            agent_id: Some(agent_id.into()),
            metadata: context,
        }
    }

    /// Set metadata (e.g., tool calls, output)
    pub fn set_metadata(&mut self, metadata: serde_json::Value) {
        self.metadata = Some(metadata);
        self.updated_at = Utc::now();
    }

    /// Update the task status
    pub fn set_status(&mut self, status: TaskStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }

    /// Update the task name
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
        self.updated_at = Utc::now();
    }

    /// Update the task description
    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = description.into();
        self.updated_at = Utc::now();
    }
}

/// A list of tasks for a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskList {
    /// Session ID
    pub session_id: String,
    /// All tasks in this session
    pub tasks: Vec<Task>,
    /// Optional "current task" pointer for UI focus and checkpoint/rewind.
    ///
    /// This is persisted alongside the task list so the UI can restore the
    /// user's current focus when resuming a session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<String>,
}

impl TaskList {
    /// Create a new task list for a session
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            tasks: Vec::new(),
            current_task_id: None,
        }
    }

    /// Get the currently focused task id.
    pub fn current_task_id(&self) -> Option<&str> {
        self.current_task_id.as_deref()
    }

    /// Set or clear the current task pointer.
    ///
    /// If `task_id` is `Some`, the id must exist in this task list.
    pub fn set_current_task_id(&mut self, task_id: Option<String>) -> Result<(), TaskError> {
        if let Some(ref id) = task_id {
            if self.find_task(id.as_str()).is_none() {
                return Err(TaskError::InvalidInput(format!(
                    "current_task_id '{id}' does not exist in task list"
                )));
            }
        }

        self.current_task_id = task_id;
        Ok(())
    }

    /// Add a task to the list
    pub fn add_task(&mut self, task: Task) {
        self.tasks.push(task);
    }

    /// Find a task by ID
    pub fn find_task(&self, task_id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == task_id)
    }

    /// Find a task by ID (mutable)
    pub fn find_task_mut(&mut self, task_id: &str) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|t| t.id == task_id)
    }

    /// Remove a task by ID
    pub fn remove_task(&mut self, task_id: &str) -> Option<Task> {
        if let Some(pos) = self.tasks.iter().position(|t| t.id == task_id) {
            if self.current_task_id.as_deref() == Some(task_id) {
                self.current_task_id = None;
            }
            Some(self.tasks.remove(pos))
        } else {
            None
        }
    }

    /// Get all root tasks (tasks without a parent)
    pub fn root_tasks(&self) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| t.parent_id.is_none())
            .collect()
    }

    /// Get all subtasks of a given task
    pub fn subtasks(&self, parent_id: &str) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| t.parent_id.as_deref() == Some(parent_id))
            .collect()
    }
}

/// Error type for task operations
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    /// Task not found
    #[error("Task not found: {0}")]
    NotFound(String),
    /// Invalid input
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Task manager for persisting and managing tasks
pub struct TaskManager {
    /// Base directory for task files
    base_dir: PathBuf,
    /// In-memory cache of task lists by session ID
    cache: std::sync::RwLock<HashMap<String, TaskList>>,
}

impl TaskManager {
    /// Create a new task manager with the given base directory
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into().join(".gestura").join("tasks");
        Self {
            base_dir,
            cache: std::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Get the path to a session's task file
    fn task_file_path(&self, session_id: &str) -> PathBuf {
        self.base_dir.join(format!("{}.json", session_id))
    }

    /// Load tasks for a session from disk
    fn load_from_disk(&self, session_id: &str) -> Result<TaskList, TaskError> {
        let path = self.task_file_path(session_id);
        if !path.exists() {
            return Ok(TaskList::new(session_id));
        }

        let content = fs::read_to_string(&path)?;
        let task_list: TaskList = serde_json::from_str(&content)?;
        Ok(task_list)
    }

    /// Save tasks for a session to disk
    fn save_to_disk(&self, task_list: &TaskList) -> Result<(), TaskError> {
        // Ensure directory exists
        fs::create_dir_all(&self.base_dir)?;

        let path = self.task_file_path(&task_list.session_id);
        let content = serde_json::to_string_pretty(task_list)?;
        fs::write(&path, content)?;
        Ok(())
    }

    /// Get or load task list for a session
    fn get_or_load(&self, session_id: &str) -> Result<TaskList, TaskError> {
        // Check cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(task_list) = cache.get(session_id) {
                return Ok(task_list.clone());
            }
        }

        // Load from disk
        let task_list = self.load_from_disk(session_id)?;

        // Update cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(session_id.to_string(), task_list.clone());
        }

        Ok(task_list)
    }

    /// Update cache and save to disk
    fn update_and_save(&self, task_list: TaskList) -> Result<(), TaskError> {
        // Save to disk first
        self.save_to_disk(&task_list)?;

        // Update cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(task_list.session_id.clone(), task_list);
        }

        Ok(())
    }

    /// Load the full task list for a session.
    ///
    /// This returns the persisted state (or an empty list if none exists yet).
    /// It is used by checkpoint/rewind to snapshot and restore task state.
    pub fn load_task_list(&self, session_id: &str) -> Result<TaskList, TaskError> {
        self.get_or_load(session_id)
    }

    /// Replace the persisted task list for a session.
    ///
    /// This is primarily used by checkpoint/rewind to restore a previous task
    /// state.
    pub fn replace_task_list(&self, task_list: TaskList) -> Result<(), TaskError> {
        self.update_and_save(task_list)
    }

    /// Set or clear the current task pointer for a session.
    ///
    /// If `task_id` is `Some`, it must exist in the session's task list.
    pub fn set_current_task_id(
        &self,
        session_id: &str,
        task_id: Option<String>,
    ) -> Result<(), TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        task_list.set_current_task_id(task_id)?;
        self.update_and_save(task_list)
    }

    /// Get the current task pointer for a session.
    pub fn get_current_task_id(&self, session_id: &str) -> Result<Option<String>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        Ok(task_list.current_task_id.clone())
    }

    /// Create a new task
    pub fn create_task(
        &self,
        session_id: &str,
        name: impl Into<String>,
        description: impl Into<String>,
        parent_id: Option<String>,
    ) -> Result<Task, TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = Task::new(session_id, name, description, parent_id);
        task_list.add_task(task.clone());
        self.update_and_save(task_list)?;
        Ok(task)
    }

    /// Update a task's status
    pub fn update_task_status(
        &self,
        session_id: &str,
        task_id: &str,
        status: TaskStatus,
    ) -> Result<(), TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = task_list
            .find_task_mut(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        task.set_status(status);
        self.update_and_save(task_list)?;
        Ok(())
    }

    /// Update a task's name and description
    pub fn update_task(
        &self,
        session_id: &str,
        task_id: &str,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<(), TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = task_list
            .find_task_mut(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;

        if let Some(name) = name {
            task.set_name(name);
        }
        if let Some(description) = description {
            task.set_description(description);
        }

        self.update_and_save(task_list)?;
        Ok(())
    }

    /// Delete a task
    pub fn delete_task(&self, session_id: &str, task_id: &str) -> Result<Task, TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = task_list
            .remove_task(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        self.update_and_save(task_list)?;
        Ok(task)
    }

    /// List all tasks for a session
    pub fn list_tasks(&self, session_id: &str) -> Result<Vec<Task>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        Ok(task_list.tasks.clone())
    }

    /// Get task hierarchy for a session
    pub fn get_hierarchy(&self, session_id: &str) -> Result<Vec<(Task, Vec<Task>)>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        let mut hierarchy = Vec::new();

        for root in task_list.root_tasks() {
            let subtasks = task_list.subtasks(&root.id).into_iter().cloned().collect();
            hierarchy.push((root.clone(), subtasks));
        }

        Ok(hierarchy)
    }

    /// Create a task from an agent (during LLM processing)
    pub fn create_agent_task(
        &self,
        session_id: &str,
        name: impl Into<String>,
        description: impl Into<String>,
        agent_id: Option<String>,
        parent_id: Option<String>,
    ) -> Result<Task, TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = Task::new_with_source(
            session_id,
            name,
            description,
            parent_id,
            TaskSource::Agent,
            agent_id,
        );
        task_list.add_task(task.clone());
        self.update_and_save(task_list)?;
        Ok(task)
    }

    /// Create a task from an orchestrator delegated task
    pub fn create_orchestrator_task(
        &self,
        session_id: &str,
        orchestrator_task_id: impl Into<String>,
        agent_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        context: Option<serde_json::Value>,
    ) -> Result<Task, TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = Task::from_orchestrator_task(
            session_id,
            orchestrator_task_id,
            agent_id,
            name,
            description,
            context,
        );
        task_list.add_task(task.clone());
        self.update_and_save(task_list)?;
        Ok(task)
    }

    /// Find a task by its orchestrator_task_id
    pub fn find_by_orchestrator_id(
        &self,
        session_id: &str,
        orchestrator_task_id: &str,
    ) -> Result<Option<Task>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        Ok(task_list
            .tasks
            .iter()
            .find(|t| t.orchestrator_task_id.as_deref() == Some(orchestrator_task_id))
            .cloned())
    }

    /// Update a task's metadata
    pub fn update_task_metadata(
        &self,
        session_id: &str,
        task_id: &str,
        metadata: serde_json::Value,
    ) -> Result<(), TaskError> {
        let mut task_list = self.get_or_load(session_id)?;
        let task = task_list
            .find_task_mut(task_id)
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        task.set_metadata(metadata);
        self.update_and_save(task_list)?;
        Ok(())
    }

    /// Get a specific task by ID
    pub fn get_task(&self, session_id: &str, task_id: &str) -> Result<Option<Task>, TaskError> {
        let task_list = self.get_or_load(session_id)?;
        Ok(task_list.find_task(task_id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_task_creation() {
        let task = Task::new("session-123", "Test Task", "Test description", None);
        assert_eq!(task.session_id, "session-123");
        assert_eq!(task.name, "Test Task");
        assert_eq!(task.description, "Test description");
        assert_eq!(task.status, TaskStatus::NotStarted);
        assert!(task.parent_id.is_none());
        assert!(!task.id.is_empty());
    }

    #[test]
    fn test_task_status_update() {
        let mut task = Task::new("session-123", "Test Task", "Test description", None);
        let original_updated_at = task.updated_at;

        // Small delay to ensure timestamp changes
        std::thread::sleep(std::time::Duration::from_millis(10));

        task.set_status(TaskStatus::InProgress);
        assert_eq!(task.status, TaskStatus::InProgress);
        assert!(task.updated_at > original_updated_at);
    }

    #[test]
    fn test_task_list_operations() {
        let mut list = TaskList::new("session-123");
        assert_eq!(list.tasks.len(), 0);
        assert!(list.current_task_id().is_none());

        let task1 = Task::new("session-123", "Task 1", "Description 1", None);
        let task2 = Task::new(
            "session-123",
            "Task 2",
            "Description 2",
            Some(task1.id.clone()),
        );

        list.add_task(task1.clone());
        list.add_task(task2.clone());
        assert_eq!(list.tasks.len(), 2);

        // Setting current task requires it to exist
        assert!(list.set_current_task_id(Some(task1.id.clone())).is_ok());
        assert_eq!(list.current_task_id(), Some(task1.id.as_str()));
        assert!(matches!(
            list.set_current_task_id(Some("does-not-exist".to_string())),
            Err(TaskError::InvalidInput(_))
        ));

        // Test find
        let found = list.find_task(&task1.id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Task 1");

        // Test root tasks
        let roots = list.root_tasks();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, task1.id);

        // Test subtasks
        let subtasks = list.subtasks(&task1.id);
        assert_eq!(subtasks.len(), 1);
        assert_eq!(subtasks[0].id, task2.id);

        // Test remove
        let removed = list.remove_task(&task1.id);
        assert!(removed.is_some());
        assert_eq!(list.tasks.len(), 1);
        // removing the current task clears the pointer
        assert!(list.current_task_id().is_none());
    }

    #[test]
    fn test_task_current_pointer_roundtrip() {
        let temp_dir = TempDir::new().unwrap();

        let session_id = "session-123";
        let task_id = {
            let manager = TaskManager::new(temp_dir.path());
            let task = manager
                .create_task(session_id, "Test Task", "Test description", None)
                .unwrap();
            manager
                .set_current_task_id(session_id, Some(task.id.clone()))
                .unwrap();
            task.id
        };

        let manager = TaskManager::new(temp_dir.path());
        let loaded = manager.get_current_task_id(session_id).unwrap();
        assert_eq!(loaded, Some(task_id));
    }

    #[test]
    fn test_task_replace_task_list() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let session_id = "session-123";
        let task = Task::new(session_id, "Task 1", "Description 1", None);

        let mut list = TaskList::new(session_id);
        list.add_task(task.clone());
        list.set_current_task_id(Some(task.id.clone())).unwrap();

        manager.replace_task_list(list).unwrap();

        let loaded = manager.load_task_list(session_id).unwrap();
        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(loaded.current_task_id(), Some(task.id.as_str()));
    }

    #[test]
    fn test_task_manager_create_and_list() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let task = manager
            .create_task("session-123", "Test Task", "Test description", None)
            .unwrap();

        assert_eq!(task.name, "Test Task");

        let tasks = manager.list_tasks("session-123").unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, task.id);
    }

    #[test]
    fn test_task_manager_persistence() {
        let temp_dir = TempDir::new().unwrap();

        // Create task with first manager instance
        {
            let manager = TaskManager::new(temp_dir.path());
            manager
                .create_task("session-123", "Test Task", "Test description", None)
                .unwrap();
        }

        // Load with second manager instance
        {
            let manager = TaskManager::new(temp_dir.path());
            let tasks = manager.list_tasks("session-123").unwrap();
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0].name, "Test Task");
        }
    }

    #[test]
    fn test_task_manager_update_status() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let task = manager
            .create_task("session-123", "Test Task", "Test description", None)
            .unwrap();

        manager
            .update_task_status("session-123", &task.id, TaskStatus::InProgress)
            .unwrap();

        let tasks = manager.list_tasks("session-123").unwrap();
        assert_eq!(tasks[0].status, TaskStatus::InProgress);
    }

    #[test]
    fn test_task_manager_update_task() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let task = manager
            .create_task("session-123", "Test Task", "Test description", None)
            .unwrap();

        manager
            .update_task(
                "session-123",
                &task.id,
                Some("Updated Task".to_string()),
                Some("Updated description".to_string()),
            )
            .unwrap();

        let tasks = manager.list_tasks("session-123").unwrap();
        assert_eq!(tasks[0].name, "Updated Task");
        assert_eq!(tasks[0].description, "Updated description");
    }

    #[test]
    fn test_task_manager_delete() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let task = manager
            .create_task("session-123", "Test Task", "Test description", None)
            .unwrap();

        let deleted = manager.delete_task("session-123", &task.id).unwrap();
        assert_eq!(deleted.id, task.id);

        let tasks = manager.list_tasks("session-123").unwrap();
        assert_eq!(tasks.len(), 0);
    }

    #[test]
    fn test_task_manager_hierarchy() {
        let temp_dir = TempDir::new().unwrap();
        let manager = TaskManager::new(temp_dir.path());

        let root = manager
            .create_task("session-123", "Root Task", "Root description", None)
            .unwrap();

        manager
            .create_task(
                "session-123",
                "Subtask 1",
                "Subtask description",
                Some(root.id.clone()),
            )
            .unwrap();

        manager
            .create_task(
                "session-123",
                "Subtask 2",
                "Subtask description",
                Some(root.id.clone()),
            )
            .unwrap();

        let hierarchy = manager.get_hierarchy("session-123").unwrap();
        assert_eq!(hierarchy.len(), 1);
        assert_eq!(hierarchy[0].0.id, root.id);
        assert_eq!(hierarchy[0].1.len(), 2);
    }
}
