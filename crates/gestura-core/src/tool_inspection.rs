//! Tool Inspection Manager
//!
//! Provides unified tool inspection, permission checking, and confirmation flow.
//! Integrates execution mode permissions with persistent permission storage.
//! Based on Block Goose's ToolInspectionManager pattern.

use crate::error::AppError;
use crate::execution_mode::{ExecutionMode, ModeManager, ToolCategory, ToolExecutionCheck};
use crate::tools::permissions::{PermissionManager, PermissionScope};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Tool metadata for inspection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    /// Tool name
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Tool category for permission grouping
    pub category: ToolCategory,
    /// Whether this tool has side effects
    pub has_side_effects: bool,
    /// Risk level (0-10, higher = more dangerous)
    pub risk_level: u8,
    /// Required capabilities (e.g., "filesystem", "network")
    pub required_capabilities: Vec<String>,
}

impl ToolMetadata {
    /// Create metadata for a read-only tool
    pub fn read_only(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            category: ToolCategory::ReadOnly,
            has_side_effects: false,
            risk_level: 0,
            required_capabilities: vec![],
        }
    }

    /// Create metadata for a write tool
    pub fn write(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            category: ToolCategory::Write,
            has_side_effects: true,
            risk_level: 3,
            required_capabilities: vec!["filesystem".to_string()],
        }
    }

    /// Create metadata for a shell tool
    pub fn shell(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            category: ToolCategory::Shell,
            has_side_effects: true,
            risk_level: 7,
            required_capabilities: vec!["shell".to_string()],
        }
    }

    /// Create metadata for a network tool
    pub fn network(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            category: ToolCategory::Network,
            has_side_effects: false,
            risk_level: 2,
            required_capabilities: vec!["network".to_string()],
        }
    }

    /// Create metadata for a git tool
    pub fn git(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            category: ToolCategory::Git,
            has_side_effects: true,
            risk_level: 5,
            required_capabilities: vec!["git".to_string()],
        }
    }
}

/// Result of tool inspection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectionResult {
    /// Tool name
    pub tool_name: String,
    /// Whether execution is allowed
    pub allowed: bool,
    /// Whether confirmation is required
    pub requires_confirmation: bool,
    /// Reason for the decision
    pub reason: String,
    /// Tool metadata if available
    pub metadata: Option<ToolMetadata>,
    /// Suggested confirmation message
    pub confirmation_message: Option<String>,
}

impl InspectionResult {
    /// Create an allowed result
    pub fn allowed(tool_name: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            allowed: true,
            requires_confirmation: false,
            reason: "Tool execution allowed".to_string(),
            metadata: None,
            confirmation_message: None,
        }
    }

    /// Create a result requiring confirmation
    pub fn needs_confirmation(tool_name: impl Into<String>, message: impl Into<String>) -> Self {
        let name = tool_name.into();
        Self {
            tool_name: name.clone(),
            allowed: true,
            requires_confirmation: true,
            reason: "Tool requires user confirmation".to_string(),
            metadata: None,
            confirmation_message: Some(message.into()),
        }
    }

    /// Create a blocked result
    pub fn blocked(tool_name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            allowed: false,
            requires_confirmation: false,
            reason: reason.into(),
            metadata: None,
            confirmation_message: None,
        }
    }

    /// Add metadata to the result
    pub fn with_metadata(mut self, metadata: ToolMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Confirmation request for user approval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmationRequest {
    /// Unique request ID
    pub id: String,
    /// Tool name
    pub tool_name: String,
    /// Tool arguments (for display)
    pub arguments: String,
    /// Human-readable description of what will happen
    pub description: String,
    /// Risk level (0-10)
    pub risk_level: u8,
    /// Whether to remember this decision
    pub remember_decision: bool,
}

/// User's response to a confirmation request
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfirmationResponse {
    /// Allow this execution
    Allow,
    /// Allow and remember for this session
    AllowSession,
    /// Allow and remember permanently
    AllowAlways,
    /// Deny this execution
    Deny,
    /// Deny and block for this session
    DenySession,
}

/// Tool Inspection Manager
///
/// Provides unified tool inspection, permission checking, and confirmation flow.
/// Integrates:
/// - `ModeManager` for execution mode-based permissions
/// - `PermissionManager` for persistent permission storage
/// - Tool metadata registry for categorization
pub struct ToolInspectionManager {
    /// Mode manager for execution mode permissions
    mode_manager: Arc<RwLock<ModeManager>>,
    /// Permission manager for persistent permissions
    permission_manager: Arc<PermissionManager>,
    /// Tool metadata registry
    tool_registry: RwLock<HashMap<String, ToolMetadata>>,
    /// Pending confirmation requests
    pending_confirmations: RwLock<HashMap<String, ConfirmationRequest>>,
}

impl Default for ToolInspectionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolInspectionManager {
    /// Create a new tool inspection manager
    pub fn new() -> Self {
        let manager = Self {
            mode_manager: Arc::new(RwLock::new(ModeManager::new())),
            permission_manager: Arc::new(PermissionManager::new()),
            tool_registry: RwLock::new(HashMap::new()),
            pending_confirmations: RwLock::new(HashMap::new()),
        };
        manager.register_builtin_tools();
        manager
    }

    /// Create with custom mode manager
    pub fn with_mode_manager(mode_manager: ModeManager) -> Self {
        let manager = Self {
            mode_manager: Arc::new(RwLock::new(mode_manager)),
            permission_manager: Arc::new(PermissionManager::new()),
            tool_registry: RwLock::new(HashMap::new()),
            pending_confirmations: RwLock::new(HashMap::new()),
        };
        manager.register_builtin_tools();
        manager
    }

    /// Register built-in tool metadata
    fn register_builtin_tools(&self) {
        let tools = vec![
            ToolMetadata::read_only("read_file", "Read contents of a file"),
            ToolMetadata::read_only("list_directory", "List files in a directory"),
            ToolMetadata::read_only("search_files", "Search for files by pattern"),
            ToolMetadata::write("write_file", "Write content to a file"),
            ToolMetadata::write("create_file", "Create a new file"),
            ToolMetadata::write("delete_file", "Delete a file"),
            ToolMetadata::shell("shell", "Execute a shell command"),
            ToolMetadata::shell("bash", "Execute a bash command"),
            ToolMetadata::shell("execute", "Execute a command"),
            ToolMetadata::network("web_search", "Search the web"),
            ToolMetadata::network("web_fetch", "Fetch a web page"),
            ToolMetadata::git("git", "Execute git commands"),
            ToolMetadata::git("git_status", "Get git repository status"),
            ToolMetadata::git("git_commit", "Create a git commit"),
            ToolMetadata::git("git_push", "Push to remote repository"),
        ];

        if let Ok(mut registry) = self.tool_registry.write() {
            for tool in tools {
                registry.insert(tool.name.clone(), tool);
            }
        }
    }

    /// Register a custom tool
    pub fn register_tool(&self, metadata: ToolMetadata) {
        if let Ok(mut registry) = self.tool_registry.write() {
            registry.insert(metadata.name.clone(), metadata);
        }
    }

    /// Get tool metadata
    pub fn get_tool_metadata(&self, tool_name: &str) -> Option<ToolMetadata> {
        self.tool_registry
            .read()
            .ok()
            .and_then(|r| r.get(tool_name).cloned())
    }

    /// Get the current execution mode
    pub fn current_mode(&self) -> ExecutionMode {
        self.mode_manager
            .read()
            .map(|m| m.mode())
            .unwrap_or_default()
    }

    /// Set the execution mode
    pub fn set_mode(&self, mode: ExecutionMode) {
        if let Ok(mut manager) = self.mode_manager.write() {
            manager.set_mode(mode);
        }
    }

    /// Inspect a tool before execution
    ///
    /// Returns an `InspectionResult` indicating whether the tool can be executed,
    /// requires confirmation, or is blocked.
    pub fn inspect_tool(
        &self,
        tool_name: &str,
        arguments: Option<&str>,
    ) -> Result<InspectionResult, AppError> {
        // Get tool metadata
        let metadata = self.get_tool_metadata(tool_name);
        let category = metadata
            .as_ref()
            .map(|m| m.category)
            .unwrap_or(ToolCategory::Shell); // Default to Shell (most restrictive)

        // Check mode-based permissions
        let mode_check = self
            .mode_manager
            .read()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?
            .can_execute_tool(tool_name, category);

        match mode_check {
            ToolExecutionCheck::Allowed => {
                // Also check persistent permissions
                let perm_check = self
                    .permission_manager
                    .check(tool_name, "execute", arguments)?;
                if perm_check.allowed {
                    Ok(InspectionResult::allowed(tool_name))
                } else {
                    // Mode allows but no persistent permission - still allowed
                    Ok(InspectionResult::allowed(tool_name))
                }
            }
            ToolExecutionCheck::RequiresConfirmation => {
                // Check if we have a persistent permission that allows it
                let perm_check = self
                    .permission_manager
                    .check(tool_name, "execute", arguments)?;
                if perm_check.allowed {
                    Ok(InspectionResult::allowed(tool_name))
                } else {
                    let message = self.build_confirmation_message(tool_name, arguments, &metadata);
                    let mut result = InspectionResult::needs_confirmation(tool_name, message);
                    if let Some(meta) = metadata {
                        result = result.with_metadata(meta);
                    }
                    Ok(result)
                }
            }
            ToolExecutionCheck::Blocked { reason } => {
                let mut result = InspectionResult::blocked(tool_name, reason);
                if let Some(meta) = metadata {
                    result = result.with_metadata(meta);
                }
                Ok(result)
            }
        }
    }

    /// Build a confirmation message for the user
    fn build_confirmation_message(
        &self,
        tool_name: &str,
        arguments: Option<&str>,
        metadata: &Option<ToolMetadata>,
    ) -> String {
        let desc = metadata
            .as_ref()
            .map(|m| m.description.as_str())
            .unwrap_or("Execute tool");
        let args_preview = arguments
            .map(|a| {
                if a.len() > 100 {
                    format!("{}...", &a[..100])
                } else {
                    a.to_string()
                }
            })
            .unwrap_or_default();

        format!(
            "Allow '{}' to {}?\n\nArguments: {}",
            tool_name, desc, args_preview
        )
    }

    /// Create a confirmation request
    pub fn create_confirmation_request(
        &self,
        tool_name: &str,
        arguments: &str,
    ) -> ConfirmationRequest {
        let metadata = self.get_tool_metadata(tool_name);
        let id = uuid::Uuid::new_v4().to_string();
        let description = self.build_confirmation_message(tool_name, Some(arguments), &metadata);
        let risk_level = metadata.as_ref().map(|m| m.risk_level).unwrap_or(5);

        let request = ConfirmationRequest {
            id: id.clone(),
            tool_name: tool_name.to_string(),
            arguments: arguments.to_string(),
            description,
            risk_level,
            remember_decision: false,
        };

        // Store pending request
        if let Ok(mut pending) = self.pending_confirmations.write() {
            pending.insert(id, request.clone());
        }

        request
    }

    /// Handle a confirmation response
    pub fn handle_confirmation(
        &self,
        request_id: &str,
        response: ConfirmationResponse,
    ) -> Result<bool, AppError> {
        // Get and remove the pending request
        let request = self
            .pending_confirmations
            .write()
            .ok()
            .and_then(|mut p| p.remove(request_id));

        let Some(request) = request else {
            return Err(AppError::Io(std::io::Error::other(format!(
                "No pending confirmation with id: {}",
                request_id
            ))));
        };

        match response {
            ConfirmationResponse::Allow => {
                // One-time allow, no persistence
                Ok(true)
            }
            ConfirmationResponse::AllowSession => {
                // Remember for this session
                if let Ok(mut manager) = self.mode_manager.write() {
                    manager.confirm_tool(&request.tool_name);
                }
                Ok(true)
            }
            ConfirmationResponse::AllowAlways => {
                // Persist permission
                self.permission_manager.grant(
                    &request.tool_name,
                    "execute",
                    PermissionScope::Global,
                    None,
                )?;
                Ok(true)
            }
            ConfirmationResponse::Deny => {
                // One-time deny
                Ok(false)
            }
            ConfirmationResponse::DenySession => {
                // Block for this session
                if let Ok(mut manager) = self.mode_manager.write() {
                    manager.block_tool_for_session(&request.tool_name);
                }
                Ok(false)
            }
        }
    }

    /// Get pending confirmation requests
    pub fn pending_requests(&self) -> Vec<ConfirmationRequest> {
        self.pending_confirmations
            .read()
            .map(|p| p.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Clear all pending confirmations
    pub fn clear_pending(&self) {
        if let Ok(mut pending) = self.pending_confirmations.write() {
            pending.clear();
        }
    }

    /// List all registered tools
    pub fn list_tools(&self) -> Vec<ToolMetadata> {
        self.tool_registry
            .read()
            .map(|r| r.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Get tools by category
    pub fn tools_by_category(&self, category: ToolCategory) -> Vec<ToolMetadata> {
        self.tool_registry
            .read()
            .map(|r| {
                r.values()
                    .filter(|t| t.category == category)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_metadata_factories() {
        let read = ToolMetadata::read_only("test", "Test tool");
        assert_eq!(read.category, ToolCategory::ReadOnly);
        assert!(!read.has_side_effects);
        assert_eq!(read.risk_level, 0);

        let shell = ToolMetadata::shell("bash", "Bash shell");
        assert_eq!(shell.category, ToolCategory::Shell);
        assert!(shell.has_side_effects);
        assert_eq!(shell.risk_level, 7);
    }

    #[test]
    fn test_inspection_result_factories() {
        let allowed = InspectionResult::allowed("test");
        assert!(allowed.allowed);
        assert!(!allowed.requires_confirmation);

        let needs_confirm = InspectionResult::needs_confirmation("test", "Please confirm");
        assert!(needs_confirm.allowed);
        assert!(needs_confirm.requires_confirmation);

        let blocked = InspectionResult::blocked("test", "Not allowed");
        assert!(!blocked.allowed);
        assert!(!blocked.requires_confirmation);
    }

    #[test]
    fn test_tool_inspection_manager_creation() {
        let manager = ToolInspectionManager::new();
        assert_eq!(manager.current_mode(), ExecutionMode::Chat);

        // Should have built-in tools registered
        let tools = manager.list_tools();
        assert!(!tools.is_empty());
        assert!(manager.get_tool_metadata("read_file").is_some());
        assert!(manager.get_tool_metadata("shell").is_some());
    }

    #[test]
    fn test_inspect_read_only_tool() {
        let manager = ToolInspectionManager::new();

        // Read-only tools should be allowed in all modes
        let result = manager.inspect_tool("read_file", None).unwrap();
        assert!(result.allowed);
        assert!(!result.requires_confirmation);
    }

    #[test]
    fn test_inspect_shell_tool_chat_mode() {
        let manager = ToolInspectionManager::new();

        // Shell tools require confirmation in Chat mode
        let result = manager.inspect_tool("shell", Some("ls -la")).unwrap();
        assert!(result.allowed);
        assert!(result.requires_confirmation);
    }

    #[test]
    fn test_inspect_shell_tool_auto_mode() {
        let manager = ToolInspectionManager::new();
        manager.set_mode(ExecutionMode::Auto);

        // Shell tools are allowed without confirmation in Auto mode
        let result = manager.inspect_tool("shell", Some("ls -la")).unwrap();
        assert!(result.allowed);
        assert!(!result.requires_confirmation);
    }

    #[test]
    fn test_inspect_shell_tool_restricted_mode() {
        let manager = ToolInspectionManager::new();
        manager.set_mode(ExecutionMode::Restricted);

        // Shell tools are blocked in Restricted mode
        let result = manager.inspect_tool("shell", Some("ls -la")).unwrap();
        assert!(!result.allowed);
    }

    #[test]
    fn test_confirmation_flow() {
        let manager = ToolInspectionManager::new();

        // Create confirmation request
        let request = manager.create_confirmation_request("shell", "rm -rf /tmp/test");
        assert!(!request.id.is_empty());
        assert_eq!(request.tool_name, "shell");

        // Should be in pending
        assert_eq!(manager.pending_requests().len(), 1);

        // Handle confirmation
        let allowed = manager
            .handle_confirmation(&request.id, ConfirmationResponse::AllowSession)
            .unwrap();
        assert!(allowed);

        // Should be removed from pending
        assert!(manager.pending_requests().is_empty());

        // Tool should now be allowed without confirmation
        let result = manager.inspect_tool("shell", None).unwrap();
        assert!(result.allowed);
        assert!(!result.requires_confirmation);
    }

    #[test]
    fn test_tools_by_category() {
        let manager = ToolInspectionManager::new();

        let read_tools = manager.tools_by_category(ToolCategory::ReadOnly);
        assert!(!read_tools.is_empty());
        assert!(
            read_tools
                .iter()
                .all(|t| t.category == ToolCategory::ReadOnly)
        );

        let shell_tools = manager.tools_by_category(ToolCategory::Shell);
        assert!(!shell_tools.is_empty());
        assert!(
            shell_tools
                .iter()
                .all(|t| t.category == ToolCategory::Shell)
        );
    }
}
