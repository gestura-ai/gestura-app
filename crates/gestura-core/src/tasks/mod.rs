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
}

impl Task {
    /// Create a new task
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
        }
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
}

impl TaskList {
    /// Create a new task list for a session
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            tasks: Vec::new(),
        }
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
