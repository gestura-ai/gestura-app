//! Tool policy helpers.
//!
//! This module centralizes tool write-classification and permission-level decisions.
//!
//! The pipeline, CLI, and GUI should rely on these helpers rather than duplicating
//! their own logic for determining whether a tool call is:
//! - blocked (e.g. Sandbox write),
//! - requires confirmation (e.g. Restricted write), or
//! - allowed.

use gestura_core_foundation::permissions::PermissionLevel;

/// A user-facing description of a tool confirmation prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolConfirmationInfo {
    /// Human-readable description of what the tool intends to do.
    pub description: String,
    /// Risk level hint (0-3) for UI severity.
    pub risk_level: u8,
    /// Category label used by the UI.
    pub category: String,
}

/// Decision for a tool call at a given permission level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallDecision {
    /// The tool call is allowed to execute.
    Allowed,
    /// The tool call requires user confirmation before it can execute.
    RequiresConfirmation(ToolConfirmationInfo),
    /// The tool call is blocked and will not execute.
    Blocked { reason: String },
}

/// Evaluation of a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPolicyEvaluation {
    /// Whether the tool call is classified as a write/side-effecting operation.
    pub is_write_operation: bool,
    /// The policy decision.
    pub decision: ToolCallDecision,
}

/// Evaluate a tool call against a [`PermissionLevel`].
///
/// Behavior is intentionally aligned with the pipeline runtime gating:
/// - Sandbox blocks write operations
/// - Restricted requires confirmation for write operations
/// - Full allows all operations
pub fn evaluate_tool_call(
    permission_level: PermissionLevel,
    tool_name: &str,
    arguments: &str,
) -> ToolPolicyEvaluation {
    let is_write = is_write_operation(tool_name, arguments);

    if permission_level.blocks(is_write) {
        let reason = format!(
            "Tool '{}' blocked: write operations are not allowed in Sandbox mode",
            tool_name
        );
        return ToolPolicyEvaluation {
            is_write_operation: is_write,
            decision: ToolCallDecision::Blocked { reason },
        };
    }

    if permission_level.requires_confirmation(is_write) {
        // Maintain existing UI semantics.
        let info = ToolConfirmationInfo {
            description: format!("Tool '{}' wants to perform a write operation", tool_name),
            risk_level: 2,
            category: "write".to_string(),
        };

        return ToolPolicyEvaluation {
            is_write_operation: is_write,
            decision: ToolCallDecision::RequiresConfirmation(info),
        };
    }

    ToolPolicyEvaluation {
        is_write_operation: is_write,
        decision: ToolCallDecision::Allowed,
    }
}

/// Return whether an action should be allowed for a session at the given permission level.
///
/// This helper exists for UI layers (CLI/GUI) that want to pre-flight an operation using a
/// coarse “write vs read” flag.
///
/// The pipeline remains the source of truth for runtime enforcement; this function is a
/// convenience wrapper around [`PermissionLevel::blocks`].
pub fn is_action_allowed(permission_level: PermissionLevel, is_write_operation: bool) -> bool {
    !permission_level.blocks(is_write_operation)
}

/// Return whether an action should require confirmation for a session at the given permission level.
///
/// This is primarily used by UI layers to decide whether to show a confirmation prompt before
/// sending a request/tool call.
pub fn requires_confirmation(permission_level: PermissionLevel, is_write_operation: bool) -> bool {
    permission_level.requires_confirmation(is_write_operation)
}

/// Determine if a tool operation is a write operation based on tool name and arguments.
///
/// Write operations include:
/// - shell/bash/execute: commands that appear to modify state (best-effort classifier)
/// - file: write/edit operations (or implicit write when `content` exists)
/// - git: operations that modify repository state (commit/push/etc)
///
/// This classifier is intentionally conservative.
pub fn is_write_operation(tool_name: &str, arguments: &str) -> bool {
    match tool_name {
        // Screen capture / recording is privacy-sensitive and produces artifacts on disk.
        // Treat as write/side-effecting so it is blocked in Sandbox and requires confirmation
        // in Restricted.
        "screenshot" | "screen_record" => true,

        // Shell commands can be read-only (e.g. `pwd`, `ls`). Use a conservative classifier.
        "shell" | "bash" | "execute" => is_shell_command_write_operation(arguments),

        // File operations depend on the operation type.
        // IMPORTANT: Keep this aligned with `execute_file_tool` and the tool schema.
        "file" | "write_file" | "edit_file" => {
            if matches!(tool_name, "write_file" | "edit_file") {
                return true;
            }

            if let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) {
                // Mirror `execute_file_tool` defaulting behavior.
                let op = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        if args.get("content").is_some() {
                            "write"
                        } else {
                            "read"
                        }
                    });
                matches!(op, "write" | "edit")
            } else {
                // If we can't parse, assume write for safety.
                true
            }
        }
        "read_file" => false,

        // Git operations depend on the operation type.
        "git" => {
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) {
                let op = args.get("operation").and_then(|v| v.as_str()).unwrap_or("");
                matches!(
                    op,
                    "commit"
                        | "push"
                        | "pull"
                        | "checkout"
                        | "merge"
                        | "rebase"
                        | "reset"
                        | "stash"
                        | "branch"
                        | "add"
                        | "rm"
                )
            } else {
                false
            }
        }

        // Web tools are always read-only (fetch / search).
        "web" | "web_search" => false,

        // Code tool: most operations are read-only analysis, but batch_edit writes to disk
        // and lint/test spawn subprocesses that can modify state (fix, test output artifacts).
        "code" => {
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) {
                let op = args.get("operation").and_then(|v| v.as_str()).unwrap_or("");
                matches!(op, "batch_edit" | "lint" | "test")
            } else {
                false
            }
        }

        // MCP manager: read operations (search/evaluate/info/list) are safe;
        // write operations (install/enable/disable/remove) modify .mcp.json on disk.
        "mcp" => {
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) {
                let op = args.get("operation").and_then(|v| v.as_str()).unwrap_or("");
                matches!(op, "install" | "enable" | "disable" | "remove")
            } else {
                false
            }
        }

        // Unknown tools are considered read-only by default.
        _ => false,
    }
}

/// Conservatively determine whether a shell tool call is likely to perform a write.
///
/// This function is intentionally biased toward safety: if we can't confidently
/// classify a command as read-only, we treat it as a write operation.
pub fn is_shell_command_write_operation(arguments: &str) -> bool {
    let command: String = match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(args) => args
            .get("command")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| arguments.to_string()),
        Err(_) => arguments.to_string(),
    };

    let cmd = command.trim();
    if cmd.is_empty() {
        return true;
    }

    // If the command uses shell control operators or redirection, treat as write.
    // This avoids having to fully parse multi-command pipelines.
    let suspicious_tokens = [">>", ">", "<", "|", ";", "&&", "||", "\n", "\r", "`", "$("];
    if suspicious_tokens.iter().any(|t| cmd.contains(t)) {
        return true;
    }

    // Extract the executable name (first token).
    let first = cmd.split_whitespace().next().unwrap_or("");

    // Allowlist of common read-only commands we want to work in Restricted mode.
    // Anything not in this list is treated as write.
    let is_allowlisted_read = matches!(
        first,
        "pwd"
            | "ls"
            | "cat"
            | "head"
            | "tail"
            | "wc"
            | "stat"
            | "file"
            | "which"
            | "whoami"
            | "uname"
            | "echo"
            | "date"
            | "env"
            | "printenv"
            | "rg"
            | "grep"
    );

    !is_allowlisted_read
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluate_blocks_writes_in_sandbox() {
        let write = serde_json::json!({"operation": "write", "path": "foo.txt", "content": "hi"})
            .to_string();

        let eval = evaluate_tool_call(PermissionLevel::Sandbox, "file", &write);
        assert!(eval.is_write_operation);
        assert!(matches!(eval.decision, ToolCallDecision::Blocked { .. }));
    }

    #[test]
    fn evaluate_requires_confirmation_for_writes_in_restricted() {
        let write = serde_json::json!({"operation": "write", "path": "foo.txt", "content": "hi"})
            .to_string();

        let eval = evaluate_tool_call(PermissionLevel::Restricted, "file", &write);
        assert!(eval.is_write_operation);
        assert!(matches!(
            eval.decision,
            ToolCallDecision::RequiresConfirmation(_)
        ));
    }

    #[test]
    fn evaluate_allows_reads_in_sandbox() {
        let read = serde_json::json!({"operation": "read", "path": "foo.txt"}).to_string();

        let eval = evaluate_tool_call(PermissionLevel::Sandbox, "file", &read);
        assert!(!eval.is_write_operation);
        assert_eq!(eval.decision, ToolCallDecision::Allowed);
    }

    #[test]
    fn evaluate_blocks_screen_capture_in_sandbox() {
        let args = serde_json::json!({"output_path": "./artifacts/screen.png"}).to_string();

        let eval = evaluate_tool_call(PermissionLevel::Sandbox, "screenshot", &args);
        assert!(eval.is_write_operation);
        assert!(matches!(eval.decision, ToolCallDecision::Blocked { .. }));
    }

    #[test]
    fn evaluate_requires_confirmation_for_screen_capture_in_restricted() {
        let args = serde_json::json!({"output_path": "./artifacts/screen.png"}).to_string();

        let eval = evaluate_tool_call(PermissionLevel::Restricted, "screenshot", &args);
        assert!(eval.is_write_operation);
        assert!(matches!(
            eval.decision,
            ToolCallDecision::RequiresConfirmation(_)
        ));
    }

    #[test]
    fn evaluate_blocks_screen_record_in_sandbox() {
        let args = serde_json::json!({"operation": "start", "output_path": "./artifacts/rec.mp4"})
            .to_string();

        let eval = evaluate_tool_call(PermissionLevel::Sandbox, "screen_record", &args);
        assert!(eval.is_write_operation);
        assert!(matches!(eval.decision, ToolCallDecision::Blocked { .. }));
    }

    #[test]
    fn coarse_helpers_match_permission_level_semantics() {
        assert!(is_action_allowed(PermissionLevel::Sandbox, false));
        assert!(!is_action_allowed(PermissionLevel::Sandbox, true));

        assert!(is_action_allowed(PermissionLevel::Restricted, false));
        assert!(is_action_allowed(PermissionLevel::Restricted, true));

        assert!(requires_confirmation(PermissionLevel::Restricted, true));
        assert!(!requires_confirmation(PermissionLevel::Restricted, false));

        assert!(is_action_allowed(PermissionLevel::Full, true));
        assert!(!requires_confirmation(PermissionLevel::Full, true));
    }

    // ── Code tool policy ──────────────────────────────────────────────────────

    #[test]
    fn code_batch_edit_is_write_operation() {
        let args = serde_json::json!({"operation": "batch_edit", "edits": []}).to_string();
        assert!(is_write_operation("code", &args));
    }

    #[test]
    fn code_batch_edit_blocked_in_sandbox() {
        let args = serde_json::json!({"operation": "batch_edit", "edits": []}).to_string();
        let eval = evaluate_tool_call(PermissionLevel::Sandbox, "code", &args);
        assert!(eval.is_write_operation);
        assert!(matches!(eval.decision, ToolCallDecision::Blocked { .. }));
    }

    #[test]
    fn code_batch_edit_requires_confirmation_in_restricted() {
        let args = serde_json::json!({"operation": "batch_edit", "edits": []}).to_string();
        let eval = evaluate_tool_call(PermissionLevel::Restricted, "code", &args);
        assert!(eval.is_write_operation);
        assert!(matches!(
            eval.decision,
            ToolCallDecision::RequiresConfirmation(_)
        ));
    }

    #[test]
    fn code_lint_is_write_operation() {
        let args = serde_json::json!({"operation": "lint", "path": "."}).to_string();
        assert!(is_write_operation("code", &args));
    }

    #[test]
    fn code_test_is_write_operation() {
        let args = serde_json::json!({"operation": "test", "path": "."}).to_string();
        assert!(is_write_operation("code", &args));
    }

    #[test]
    fn code_glob_is_read_only() {
        let args = serde_json::json!({"operation": "glob", "pattern": "**/*.rs"}).to_string();
        assert!(!is_write_operation("code", &args));
    }

    #[test]
    fn code_grep_is_read_only_in_sandbox() {
        let args =
            serde_json::json!({"operation": "grep", "pattern": "fn main", "path": "."}).to_string();
        let eval = evaluate_tool_call(PermissionLevel::Sandbox, "code", &args);
        assert!(!eval.is_write_operation);
        assert_eq!(eval.decision, ToolCallDecision::Allowed);
    }

    #[test]
    fn code_symbols_is_read_only() {
        let args = serde_json::json!({"operation": "symbols", "path": "src/main.rs"}).to_string();
        assert!(!is_write_operation("code", &args));
    }

    // ── MCP manager tool policy ───────────────────────────────────────────────

    #[test]
    fn mcp_install_is_write_operation() {
        let args =
            serde_json::json!({"operation": "install", "server_id": "io.github.test/server"})
                .to_string();
        assert!(is_write_operation("mcp", &args));
    }

    #[test]
    fn mcp_install_blocked_in_sandbox() {
        let args =
            serde_json::json!({"operation": "install", "server_id": "io.github.test/server"})
                .to_string();
        let eval = evaluate_tool_call(PermissionLevel::Sandbox, "mcp", &args);
        assert!(eval.is_write_operation);
        assert!(matches!(eval.decision, ToolCallDecision::Blocked { .. }));
    }

    #[test]
    fn mcp_install_requires_confirmation_in_restricted() {
        let args =
            serde_json::json!({"operation": "install", "server_id": "io.github.test/server"})
                .to_string();
        let eval = evaluate_tool_call(PermissionLevel::Restricted, "mcp", &args);
        assert!(eval.is_write_operation);
        assert!(matches!(
            eval.decision,
            ToolCallDecision::RequiresConfirmation(_)
        ));
    }

    #[test]
    fn mcp_enable_is_write_operation() {
        let args = serde_json::json!({"operation": "enable", "name": "my-server"}).to_string();
        assert!(is_write_operation("mcp", &args));
    }

    #[test]
    fn mcp_disable_is_write_operation() {
        let args = serde_json::json!({"operation": "disable", "name": "my-server"}).to_string();
        assert!(is_write_operation("mcp", &args));
    }

    #[test]
    fn mcp_remove_is_write_operation() {
        let args = serde_json::json!({"operation": "remove", "name": "my-server"}).to_string();
        assert!(is_write_operation("mcp", &args));
    }

    #[test]
    fn mcp_search_is_read_only() {
        let args = serde_json::json!({"operation": "search", "query": "filesystem"}).to_string();
        assert!(!is_write_operation("mcp", &args));
    }

    #[test]
    fn mcp_search_allowed_in_sandbox() {
        let args = serde_json::json!({"operation": "search", "query": "filesystem"}).to_string();
        let eval = evaluate_tool_call(PermissionLevel::Sandbox, "mcp", &args);
        assert!(!eval.is_write_operation);
        assert_eq!(eval.decision, ToolCallDecision::Allowed);
    }

    #[test]
    fn mcp_evaluate_is_read_only() {
        let args =
            serde_json::json!({"operation": "evaluate", "server_id": "io.github.test/server"})
                .to_string();
        assert!(!is_write_operation("mcp", &args));
    }

    #[test]
    fn mcp_list_is_read_only() {
        let args = serde_json::json!({"operation": "list"}).to_string();
        assert!(!is_write_operation("mcp", &args));
    }
}
