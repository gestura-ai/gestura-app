//! Execution mode support for agent pipeline
//!
//! This module provides Auto vs Agent mode switching, mode-specific tool permissions,
//! and mode persistence per session. Based on Block Goose architecture patterns.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Execution mode for the agent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ExecutionMode {
    /// Agent mode - interactive conversation with confirmation for dangerous operations
    #[default]
    #[serde(alias = "Chat")]
    Agent,
    /// Auto mode - autonomous tool execution without confirmation
    Auto,
    /// Restricted mode - limited tool access for safety
    Restricted,
}

impl ExecutionMode {
    /// Get a human-readable description of the mode
    pub fn description(&self) -> &'static str {
        match self {
            Self::Agent => "Interactive agent with tool confirmation",
            Self::Auto => "Autonomous execution without confirmation",
            Self::Restricted => "Limited tool access for safety",
        }
    }

    /// Get the short name for UI display
    pub fn short_name(&self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::Auto => "Auto",
            Self::Restricted => "Restricted",
        }
    }

    /// Check if this mode requires confirmation for tool execution
    pub fn requires_confirmation(&self) -> bool {
        match self {
            Self::Agent => true,
            Self::Auto => false,
            Self::Restricted => true,
        }
    }

    /// Check if this mode allows autonomous tool execution
    pub fn allows_autonomous_execution(&self) -> bool {
        match self {
            Self::Agent => false,
            Self::Auto => true,
            Self::Restricted => false,
        }
    }
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.short_name())
    }
}

impl std::str::FromStr for ExecutionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "agent" | "chat" | "interactive" => Ok(Self::Agent),
            "auto" | "autonomous" => Ok(Self::Auto),
            "restricted" | "safe" | "limited" => Ok(Self::Restricted),
            _ => Err(format!("Unknown execution mode: {}", s)),
        }
    }
}

/// Tool permission level for execution modes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolPermission {
    /// Tool is always allowed
    Allowed,
    /// Tool requires confirmation before execution
    RequiresConfirmation,
    /// Tool is blocked in this mode
    Blocked,
}

/// Tool category for permission grouping
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolCategory {
    /// Read-only operations (file read, search, etc.)
    ReadOnly,
    /// Write operations (file write, create, etc.)
    Write,
    /// Shell/command execution
    Shell,
    /// Network operations (web fetch, API calls)
    Network,
    /// System operations (process management, etc.)
    System,
    /// Git operations
    Git,
}

impl ToolCategory {
    /// Get the default permission for this category in a given mode
    pub fn default_permission(&self, mode: ExecutionMode) -> ToolPermission {
        match (self, mode) {
            // Read-only is always allowed
            (Self::ReadOnly, _) => ToolPermission::Allowed,

            // Write operations
            (Self::Write, ExecutionMode::Auto) => ToolPermission::Allowed,
            (Self::Write, ExecutionMode::Agent) => ToolPermission::RequiresConfirmation,
            (Self::Write, ExecutionMode::Restricted) => ToolPermission::Blocked,

            // Shell operations
            (Self::Shell, ExecutionMode::Auto) => ToolPermission::Allowed,
            (Self::Shell, ExecutionMode::Agent) => ToolPermission::RequiresConfirmation,
            (Self::Shell, ExecutionMode::Restricted) => ToolPermission::Blocked,

            // Network operations
            (Self::Network, ExecutionMode::Auto) => ToolPermission::Allowed,
            (Self::Network, ExecutionMode::Agent) => ToolPermission::Allowed,
            (Self::Network, ExecutionMode::Restricted) => ToolPermission::RequiresConfirmation,

            // System operations
            (Self::System, ExecutionMode::Auto) => ToolPermission::RequiresConfirmation,
            (Self::System, ExecutionMode::Agent) => ToolPermission::RequiresConfirmation,
            (Self::System, ExecutionMode::Restricted) => ToolPermission::Blocked,

            // Git operations
            (Self::Git, ExecutionMode::Auto) => ToolPermission::Allowed,
            (Self::Git, ExecutionMode::Agent) => ToolPermission::RequiresConfirmation,
            (Self::Git, ExecutionMode::Restricted) => ToolPermission::Blocked,
        }
    }
}

/// Configuration for execution mode behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeConfig {
    /// Current execution mode
    pub mode: ExecutionMode,
    /// Custom tool overrides (tool_name -> permission)
    pub tool_overrides: std::collections::HashMap<String, ToolPermission>,
    /// Whether to persist mode across sessions
    pub persist_mode: bool,
    /// Auto-switch to Agent mode after errors
    pub auto_fallback_on_error: bool,
}

impl Default for ModeConfig {
    fn default() -> Self {
        Self {
            mode: ExecutionMode::Agent,
            tool_overrides: std::collections::HashMap::new(),
            persist_mode: true,
            auto_fallback_on_error: true,
        }
    }
}

impl ModeConfig {
    /// Create a new config with the specified mode
    pub fn with_mode(mode: ExecutionMode) -> Self {
        Self {
            mode,
            ..Default::default()
        }
    }

    /// Get the effective permission for a tool
    pub fn get_tool_permission(&self, tool_name: &str, category: ToolCategory) -> ToolPermission {
        // Check for explicit override first
        if let Some(permission) = self.tool_overrides.get(tool_name) {
            return *permission;
        }
        // Fall back to category default
        category.default_permission(self.mode)
    }

    /// Set a custom permission for a specific tool
    pub fn set_tool_override(&mut self, tool_name: impl Into<String>, permission: ToolPermission) {
        self.tool_overrides.insert(tool_name.into(), permission);
    }

    /// Remove a custom permission override
    pub fn remove_tool_override(&mut self, tool_name: &str) {
        self.tool_overrides.remove(tool_name);
    }

    /// Check if a tool is allowed (Allowed or RequiresConfirmation)
    pub fn is_tool_allowed(&self, tool_name: &str, category: ToolCategory) -> bool {
        matches!(
            self.get_tool_permission(tool_name, category),
            ToolPermission::Allowed | ToolPermission::RequiresConfirmation
        )
    }

    /// Check if a tool requires confirmation
    pub fn tool_requires_confirmation(&self, tool_name: &str, category: ToolCategory) -> bool {
        matches!(
            self.get_tool_permission(tool_name, category),
            ToolPermission::RequiresConfirmation
        )
    }
}

/// Manager for execution mode state
#[derive(Debug, Clone)]
pub struct ModeManager {
    config: ModeConfig,
    /// Blocked tools for the current session
    session_blocked_tools: HashSet<String>,
    /// Tools that have been confirmed this session (skip re-confirmation)
    confirmed_tools: HashSet<String>,
}

impl ModeManager {
    /// Create a new mode manager with default config
    pub fn new() -> Self {
        Self::with_config(ModeConfig::default())
    }

    /// Create a new mode manager with custom config
    pub fn with_config(config: ModeConfig) -> Self {
        Self {
            config,
            session_blocked_tools: HashSet::new(),
            confirmed_tools: HashSet::new(),
        }
    }

    /// Get the current execution mode
    pub fn mode(&self) -> ExecutionMode {
        self.config.mode
    }

    /// Get the current config
    pub fn config(&self) -> &ModeConfig {
        &self.config
    }

    /// Set the execution mode
    pub fn set_mode(&mut self, mode: ExecutionMode) {
        self.config.mode = mode;
        // Clear confirmed tools when mode changes
        self.confirmed_tools.clear();
    }

    /// Check if a tool can be executed
    pub fn can_execute_tool(&self, tool_name: &str, category: ToolCategory) -> ToolExecutionCheck {
        // Check session blocks first
        if self.session_blocked_tools.contains(tool_name) {
            return ToolExecutionCheck::Blocked {
                reason: "Tool was blocked for this session".to_string(),
            };
        }

        let permission = self.config.get_tool_permission(tool_name, category);
        match permission {
            ToolPermission::Allowed => ToolExecutionCheck::Allowed,
            ToolPermission::RequiresConfirmation => {
                if self.confirmed_tools.contains(tool_name) {
                    ToolExecutionCheck::Allowed
                } else {
                    ToolExecutionCheck::RequiresConfirmation
                }
            }
            ToolPermission::Blocked => ToolExecutionCheck::Blocked {
                reason: format!(
                    "Tool '{}' is blocked in {} mode",
                    tool_name,
                    self.config.mode.short_name()
                ),
            },
        }
    }

    /// Mark a tool as confirmed for this session
    pub fn confirm_tool(&mut self, tool_name: impl Into<String>) {
        self.confirmed_tools.insert(tool_name.into());
    }

    /// Block a tool for this session
    pub fn block_tool_for_session(&mut self, tool_name: impl Into<String>) {
        self.session_blocked_tools.insert(tool_name.into());
    }

    /// Clear session state (confirmed and blocked tools)
    pub fn clear_session_state(&mut self) {
        self.confirmed_tools.clear();
        self.session_blocked_tools.clear();
    }

    /// Get list of tools that require confirmation
    pub fn pending_confirmations(&self) -> Vec<&String> {
        self.session_blocked_tools.iter().collect()
    }
}

impl Default for ModeManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of checking if a tool can be executed
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolExecutionCheck {
    /// Tool can be executed immediately
    Allowed,
    /// Tool requires user confirmation before execution
    RequiresConfirmation,
    /// Tool is blocked and cannot be executed
    Blocked { reason: String },
}

impl ToolExecutionCheck {
    /// Check if execution is allowed (either immediately or after confirmation)
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    /// Check if confirmation is required
    pub fn requires_confirmation(&self) -> bool {
        matches!(self, Self::RequiresConfirmation)
    }

    /// Check if execution is blocked
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_mode_defaults() {
        let mode = ExecutionMode::default();
        assert_eq!(mode, ExecutionMode::Agent);
        assert!(mode.requires_confirmation());
        assert!(!mode.allows_autonomous_execution());
    }

    #[test]
    fn test_execution_mode_from_str() {
        assert_eq!(
            "agent".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Agent
        );
        // "chat" is kept as an alias for backward compatibility
        assert_eq!(
            "chat".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Agent
        );
        assert_eq!(
            "auto".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Auto
        );
        assert_eq!(
            "restricted".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Restricted
        );
        assert!("invalid".parse::<ExecutionMode>().is_err());
    }

    #[test]
    fn test_tool_category_permissions() {
        // Read-only is always allowed
        assert_eq!(
            ToolCategory::ReadOnly.default_permission(ExecutionMode::Agent),
            ToolPermission::Allowed
        );
        assert_eq!(
            ToolCategory::ReadOnly.default_permission(ExecutionMode::Restricted),
            ToolPermission::Allowed
        );

        // Shell requires confirmation in Agent mode
        assert_eq!(
            ToolCategory::Shell.default_permission(ExecutionMode::Agent),
            ToolPermission::RequiresConfirmation
        );

        // Shell is blocked in Restricted mode
        assert_eq!(
            ToolCategory::Shell.default_permission(ExecutionMode::Restricted),
            ToolPermission::Blocked
        );
    }

    #[test]
    fn test_mode_config_overrides() {
        let mut config = ModeConfig::with_mode(ExecutionMode::Agent);

        // Default: shell requires confirmation
        assert_eq!(
            config.get_tool_permission("run_shell", ToolCategory::Shell),
            ToolPermission::RequiresConfirmation
        );

        // Override: allow specific shell command
        config.set_tool_override("run_shell", ToolPermission::Allowed);
        assert_eq!(
            config.get_tool_permission("run_shell", ToolCategory::Shell),
            ToolPermission::Allowed
        );
    }

    #[test]
    fn test_mode_manager_confirmation() {
        let mut manager = ModeManager::new();

        // Shell requires confirmation in Agent mode
        let check = manager.can_execute_tool("run_shell", ToolCategory::Shell);
        assert!(check.requires_confirmation());

        // After confirmation, it's allowed
        manager.confirm_tool("run_shell");
        let check = manager.can_execute_tool("run_shell", ToolCategory::Shell);
        assert!(check.is_allowed());

        // Changing mode clears confirmations
        manager.set_mode(ExecutionMode::Auto);
        let check = manager.can_execute_tool("run_shell", ToolCategory::Shell);
        assert!(check.is_allowed()); // Auto mode allows without confirmation
    }

    #[test]
    fn test_mode_manager_session_block() {
        let mut manager = ModeManager::with_config(ModeConfig::with_mode(ExecutionMode::Auto));

        // Initially allowed
        let check = manager.can_execute_tool("dangerous_tool", ToolCategory::System);
        assert!(!check.is_blocked());

        // Block for session
        manager.block_tool_for_session("dangerous_tool");
        let check = manager.can_execute_tool("dangerous_tool", ToolCategory::System);
        assert!(check.is_blocked());

        // Clear session state
        manager.clear_session_state();
        let check = manager.can_execute_tool("dangerous_tool", ToolCategory::System);
        assert!(!check.is_blocked());
    }
}
