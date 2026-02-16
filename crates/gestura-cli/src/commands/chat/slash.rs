//! Shared slash-command helpers for chat (basic + TUI).
//!
//! This module intentionally stays dependency-free (no shell quoting, etc.).

use std::path::PathBuf;

use crate::commands::tools::permissions::permission_manager;

use gestura_core::{
    AppConfig,
    chat_sessions::{ChatSession, SessionPermissionLevel},
    config::{McpScope, McpServerEntry, McpTransportType, infer_transport_from_endpoint},
    context::ContextManager,
    find_tool,
    hooks::{HookCommandTemplate, HookDefinition, HookEvent},
    memory_bank::MemoryBankEntry,
    tasks::{Task, TaskManager, TaskStatus},
    tools::permissions::PermissionScope,
};

// ===================== /hooks =====================

pub(crate) enum HooksOutcome {
    Changed(Vec<String>),
    Unchanged(Vec<String>),
}

impl HooksOutcome {
    pub(crate) fn changed(&self) -> bool {
        matches!(self, HooksOutcome::Changed(_))
    }

    pub(crate) fn into_lines(self) -> Vec<String> {
        match self {
            HooksOutcome::Changed(lines) | HooksOutcome::Unchanged(lines) => lines,
        }
    }
}

pub(crate) fn apply_hooks_subcommand(
    args: &[&str],
    config: &mut AppConfig,
) -> std::result::Result<HooksOutcome, String> {
    let mut lines: Vec<String> = Vec::new();

    let sub = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if sub.is_empty() || sub == "help" || sub == "--help" || sub == "-h" {
        return Ok(HooksOutcome::Unchanged(hooks_usage_lines()));
    }

    match sub.as_str() {
        "show" | "status" => {
            let hooks = &config.hooks;
            lines.push("━━━ Hooks Configuration ━━━".to_string());
            lines.push(String::new());
            lines.push(format!(
                "Enabled: {}",
                if hooks.enabled { "yes" } else { "no" }
            ));
            lines.push(format!("Timeout: {} ms", hooks.timeout_ms));
            lines.push(format!("Max output: {} bytes", hooks.max_output_bytes));
            lines.push(String::new());
            if hooks.allowed_programs.is_empty() {
                lines.push("Allowed programs: (none)".to_string());
            } else {
                lines.push(format!(
                    "Allowed programs: {}",
                    hooks.allowed_programs.join(", ")
                ));
            }
            lines.push(String::new());
            if hooks.hooks.is_empty() {
                lines.push("Hooks: (none)".to_string());
            } else {
                lines.push(format!("Hooks: {} configured", hooks.hooks.len()));
                for h in &hooks.hooks {
                    lines.push(format!(
                        "- {} ({:?}) -> {} {}",
                        h.name,
                        h.event,
                        h.command.program,
                        h.command.args.join(" ")
                    ));
                }
            }
            Ok(HooksOutcome::Unchanged(lines))
        }
        "list" | "ls" => {
            let hooks = &config.hooks;
            lines.push("━━━ Hooks ━━━".to_string());
            lines.push(String::new());
            if hooks.hooks.is_empty() {
                lines.push("No hooks configured.".to_string());
            } else {
                for h in &hooks.hooks {
                    lines.push(format!(
                        "- {} ({:?}) -> {} {}",
                        h.name,
                        h.event,
                        h.command.program,
                        h.command.args.join(" ")
                    ));
                }
            }
            Ok(HooksOutcome::Unchanged(lines))
        }
        "enable" => {
            if config.hooks.enabled {
                lines.push("Hooks already enabled.".to_string());
                Ok(HooksOutcome::Unchanged(lines))
            } else {
                config.hooks.enabled = true;
                lines.push("Enabled hooks.".to_string());
                Ok(HooksOutcome::Changed(lines))
            }
        }
        "disable" => {
            if !config.hooks.enabled {
                lines.push("Hooks already disabled.".to_string());
                Ok(HooksOutcome::Unchanged(lines))
            } else {
                config.hooks.enabled = false;
                lines.push("Disabled hooks.".to_string());
                Ok(HooksOutcome::Changed(lines))
            }
        }
        "allow" => {
            let action = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            match action.as_str() {
                "list" | "ls" | "" => {
                    if config.hooks.allowed_programs.is_empty() {
                        lines.push("Allowed programs: (none)".to_string());
                    } else {
                        lines.push(format!(
                            "Allowed programs ({}): {}",
                            config.hooks.allowed_programs.len(),
                            config.hooks.allowed_programs.join(", ")
                        ));
                    }
                    Ok(HooksOutcome::Unchanged(lines))
                }
                "add" => {
                    let Some(program) = args.get(2).copied() else {
                        return Err("Usage: /hooks allow add <program>".to_string());
                    };
                    if config.hooks.allowed_programs.iter().any(|p| p == program) {
                        lines.push(format!("Program already allow-listed: {program}"));
                        Ok(HooksOutcome::Unchanged(lines))
                    } else {
                        config.hooks.allowed_programs.push(program.to_string());
                        lines.push(format!("Allow-listed program: {program}"));
                        Ok(HooksOutcome::Changed(lines))
                    }
                }
                "remove" | "rm" | "del" | "delete" => {
                    let Some(program) = args.get(2).copied() else {
                        return Err("Usage: /hooks allow remove <program>".to_string());
                    };
                    let before = config.hooks.allowed_programs.len();
                    config.hooks.allowed_programs.retain(|p| p != program);
                    if config.hooks.allowed_programs.len() == before {
                        lines.push(format!("Program not in allow-list: {program}"));
                        Ok(HooksOutcome::Unchanged(lines))
                    } else {
                        lines.push(format!("Removed from allow-list: {program}"));
                        Ok(HooksOutcome::Changed(lines))
                    }
                }
                _ => Err(format!(
                    "Unknown allow subcommand '{action}'. Try: /hooks allow list|add|remove"
                )),
            }
        }
        "set" => {
            let key = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            let value = args.get(2).copied().unwrap_or("");
            if key.is_empty() {
                return Err(
                    "Usage: /hooks set timeout_ms <n> | /hooks set max_output_bytes <n>"
                        .to_string(),
                );
            }
            match key.as_str() {
                "timeout" | "timeout_ms" => {
                    let n: u64 = value
                        .parse()
                        .map_err(|_| "timeout_ms must be an integer".to_string())?;
                    config.hooks.timeout_ms = n;
                    lines.push(format!("Set hooks timeout_ms to {n}"));
                    Ok(HooksOutcome::Changed(lines))
                }
                "max_output" | "max_output_bytes" => {
                    let n: usize = value
                        .parse()
                        .map_err(|_| "max_output_bytes must be an integer".to_string())?;
                    config.hooks.max_output_bytes = n;
                    lines.push(format!("Set hooks max_output_bytes to {n}"));
                    Ok(HooksOutcome::Changed(lines))
                }
                _ => Err(format!(
                    "Unknown key '{key}'. Valid: timeout_ms, max_output_bytes"
                )),
            }
        }
        "create" | "update" => {
            let is_update = sub == "update";
            let Some(name) = args.get(1).copied() else {
                return Err(format!(
                    "Usage: /hooks {} <name> <event> <program> [args...]",
                    if is_update { "update" } else { "create" }
                ));
            };
            let Some(event_str) = args.get(2).copied() else {
                return Err(
                    "Missing <event>. Try: pre_pipeline|post_pipeline|pre_tool|post_tool"
                        .to_string(),
                );
            };
            let Some(program) = args.get(3).copied() else {
                return Err(
                    "Missing <program>. Usage: /hooks create <name> <event> <program> [args...]"
                        .to_string(),
                );
            };
            let event: HookEvent = event_str.parse().map_err(|_: String| {
                format!("Unknown hook event '{event_str}'. Try pre_pipeline|post_pipeline|pre_tool|post_tool")
            })?;
            let cmd = HookCommandTemplate {
                program: program.to_string(),
                args: args
                    .get(4..)
                    .unwrap_or_default()
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect(),
            };

            let idx = config.hooks.hooks.iter().position(|h| h.name == name);
            match (is_update, idx) {
                (false, Some(_)) => Err(format!(
                    "Hook '{name}' already exists. Use /hooks update {name} … or /hooks delete {name}"
                )),
                (true, None) => Err(format!(
                    "Hook '{name}' not found. Use /hooks create {name} …"
                )),
                (true, Some(i)) => {
                    config.hooks.hooks[i].event = event;
                    config.hooks.hooks[i].command = cmd;
                    lines.push(format!("Updated hook: {name}"));
                    lines.push(format!(
                        "Note: program must be allow-listed: /hooks allow add {program}"
                    ));
                    Ok(HooksOutcome::Changed(lines))
                }
                (false, None) => {
                    config.hooks.hooks.push(HookDefinition {
                        name: name.to_string(),
                        event,
                        command: cmd,
                    });
                    lines.push(format!("Created hook: {name}"));
                    lines.push(format!(
                        "Note: program must be allow-listed: /hooks allow add {program}"
                    ));
                    Ok(HooksOutcome::Changed(lines))
                }
            }
        }
        "delete" | "del" | "rm" | "remove" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /hooks delete <name>".to_string());
            };
            let before = config.hooks.hooks.len();
            config.hooks.hooks.retain(|h| h.name != name);
            if config.hooks.hooks.len() == before {
                lines.push(format!("Hook not found: {name}"));
                Ok(HooksOutcome::Unchanged(lines))
            } else {
                lines.push(format!("Deleted hook: {name}"));
                Ok(HooksOutcome::Changed(lines))
            }
        }
        _ => Err(format!(
            "Unknown /hooks subcommand '{sub}'. Try: /hooks help"
        )),
    }
}

fn hooks_usage_lines() -> Vec<String> {
    vec![
        "Hooks commands:".to_string(),
        "  /hooks                     (interactive browser)".to_string(),
        "  /hooks show                (print config)".to_string(),
        "  /hooks enable|disable".to_string(),
        "  /hooks allow list".to_string(),
        "  /hooks allow add <program>".to_string(),
        "  /hooks allow remove <program>".to_string(),
        "  /hooks list".to_string(),
        "  /hooks create <name> <event> <program> [args...]".to_string(),
        "  /hooks update <name> <event> <program> [args...]".to_string(),
        "  /hooks delete <name>".to_string(),
        "  /hooks set timeout_ms <n>".to_string(),
        "  /hooks set max_output_bytes <n>".to_string(),
        "Events: pre_pipeline | post_pipeline | pre_tool | post_tool".to_string(),
    ]
}

// ===================== /permissions =====================

pub(crate) struct PermissionsOutcome {
    pub(crate) lines: Vec<String>,
    pub(crate) changed_permissions: bool,
    pub(crate) session_changed: bool,
}

pub(crate) fn run_permissions_subcommand(
    args: &[&str],
    session: &mut ChatSession,
) -> std::result::Result<PermissionsOutcome, String> {
    let sub = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if sub.is_empty() || sub == "help" || sub == "--help" || sub == "-h" {
        return Ok(PermissionsOutcome {
            lines: permissions_usage_lines(),
            changed_permissions: false,
            session_changed: false,
        });
    }

    match sub.as_str() {
        "list" | "ls" => {
            let perms = permission_manager()
                .list()
                .map_err(|e| format!("Failed to list permissions: {e}"))?;
            let mut lines = vec!["━━━ Granted Permissions ━━━".to_string(), String::new()];
            if perms.is_empty() {
                lines.push("No tool permissions have been granted.".to_string());
            } else {
                for perm in &perms {
                    let scope_str = match &perm.scope {
                        gestura_core::PermissionScope::Global => "global".to_string(),
                        gestura_core::PermissionScope::Path(p) => format!("path:{p}"),
                        gestura_core::PermissionScope::Command(c) => format!("cmd:{c}"),
                    };
                    let expires = perm
                        .expires_at
                        .map(|e| e.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "never".to_string());
                    lines.push(format!(
                        "  {}:{} [{}] expires: {}",
                        perm.tool, perm.action, scope_str, expires
                    ));
                }
            }
            lines.push(String::new());
            lines.push("Try: /permissions grant <tool.action> [scope]".to_string());
            lines.push("Try: /permissions revoke <tool.action>".to_string());
            Ok(PermissionsOutcome {
                lines,
                changed_permissions: false,
                session_changed: false,
            })
        }
        "grant" => {
            let (tool, action, scope) = parse_permission_grant_args(args)?;
            permission_manager()
                .grant(&tool, &action, scope, None)
                .map_err(|e| format!("Failed to grant permission: {e}"))?;
            Ok(PermissionsOutcome {
                lines: vec![format!("Granted permission: {tool}.{action}")],
                changed_permissions: true,
                session_changed: false,
            })
        }
        "revoke" => {
            let (tool, action) = parse_permission_tool_action(args.get(1..).unwrap_or_default())?;
            let count = permission_manager()
                .revoke(&tool, &action)
                .map_err(|e| format!("Failed to revoke permission: {e}"))?;
            let msg = if count > 0 {
                format!("Revoked permission: {tool}.{action} ({count} removed)")
            } else {
                format!("No matching permission found: {tool}.{action}")
            };
            Ok(PermissionsOutcome {
                lines: vec![msg],
                changed_permissions: count > 0,
                session_changed: false,
            })
        }
        "reset" => {
            let count = permission_manager()
                .reset()
                .map_err(|e| format!("Failed to reset permissions: {e}"))?;
            Ok(PermissionsOutcome {
                lines: vec![format!("Reset permissions ({count} removed)")],
                changed_permissions: count > 0,
                session_changed: false,
            })
        }
        "check" => {
            let (tool, action, target) = parse_permission_check_args(args)?;
            let check = permission_manager()
                .check(&tool, &action, target.as_deref())
                .map_err(|e| format!("Failed to check permission: {e}"))?;

            let mut lines = Vec::new();
            let target_str = target.as_deref().unwrap_or("-");
            if check.allowed {
                lines.push(format!("ALLOWED: {tool}.{action} [{target_str}]"));
            } else {
                lines.push(format!("DENIED: {tool}.{action} [{target_str}]"));
                lines.push(format!("Reason: {}", check.reason));
            }
            Ok(PermissionsOutcome {
                lines,
                changed_permissions: false,
                session_changed: false,
            })
        }
        "audit" => {
            // /permissions audit [clear]
            let action = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            if action == "clear" {
                let removed = permission_manager()
                    .clear_audit_log()
                    .map_err(|e| format!("Failed to clear audit log: {e}"))?;
                return Ok(PermissionsOutcome {
                    lines: vec![format!("Cleared permission audit log ({removed} entries)")],
                    changed_permissions: false,
                    session_changed: false,
                });
            }

            let log = permission_manager()
                .audit_log()
                .map_err(|e| format!("Failed to load audit log: {e}"))?;
            if log.is_empty() {
                return Ok(PermissionsOutcome {
                    lines: vec!["Permission audit log is empty.".to_string()],
                    changed_permissions: false,
                    session_changed: false,
                });
            }

            let mut lines = vec!["━━━ Permission Audit Log ━━━".to_string(), String::new()];
            for entry in log.iter().rev().take(20) {
                let status = if entry.allowed { "✓" } else { "✗" };
                let res = entry.resource.as_deref().unwrap_or("-");
                lines.push(format!(
                    "  {status} {}:{} [{res}] - {}",
                    entry.tool, entry.action, entry.reason
                ));
            }
            if log.len() > 20 {
                lines.push(format!("  ... and {} more entries", log.len() - 20));
            }
            Ok(PermissionsOutcome {
                lines,
                changed_permissions: false,
                session_changed: false,
            })
        }
        "level" => {
            let action = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            match action.as_str() {
                "" | "show" => {
                    let current = session
                        .state
                        .tool_settings
                        .as_ref()
                        .map(|s| s.permission_level)
                        .unwrap_or_default();
                    Ok(PermissionsOutcome {
                        lines: vec![format!("Session permission level: {current}")],
                        changed_permissions: false,
                        session_changed: false,
                    })
                }
                "set" => {
                    let Some(level_str) = args.get(2).copied() else {
                        return Err(
                            "Usage: /permissions level set <sandbox|restricted|full>".to_string(),
                        );
                    };
                    let level: SessionPermissionLevel = level_str.parse()
                        .map_err(|_: String| format!("Unknown permission level '{level_str}'"))?;
                    let settings = session.state.tool_settings.get_or_insert_with(Default::default);
                    let changed = settings.permission_level != level;
                    settings.permission_level = level;
                    Ok(PermissionsOutcome {
                        lines: vec![format!("Set session permission level -> {level}")],
                        changed_permissions: false,
                        session_changed: changed,
                    })
                }
                _ => Err(
                    "Usage: /permissions level [show] | /permissions level set <sandbox|restricted|full>"
                        .to_string(),
                ),
            }
        }
        _ => Err(format!(
            "Unknown /permissions subcommand '{sub}'. Try: /permissions help"
        )),
    }
}

fn permissions_usage_lines() -> Vec<String> {
    vec![
        "Permissions commands:".to_string(),
        "  /permissions                    (interactive browser/overlay)".to_string(),
        "  /permissions list".to_string(),
        "  /permissions grant <tool.action> [scope]".to_string(),
        "  /permissions grant <tool> <action> [scope]".to_string(),
        "  /permissions revoke <tool.action>".to_string(),
        "  /permissions revoke <tool> <action>".to_string(),
        "  /permissions reset".to_string(),
        "  /permissions check <read|write|shell|fetch|tool.action> [target]".to_string(),
        "  /permissions check <tool> <action> [target]".to_string(),
        "  /permissions audit [clear]".to_string(),
        "  /permissions level [show]".to_string(),
        "  /permissions level set <sandbox|restricted|full>".to_string(),
        "Scope: omit for global; start with '/' for path scope; otherwise command substring scope"
            .to_string(),
    ]
}

fn parse_permission_grant_args(args: &[&str]) -> Result<(String, String, PermissionScope), String> {
    // /permissions grant <tool.action> [scope]
    // /permissions grant <tool> <action> [scope]
    let rest = args.get(1..).unwrap_or_default();
    let (tool, action, scope_str) = match rest {
        [perm] => {
            let (tool, action) = parse_permission_tool_action(&[*perm])?;
            (tool, action, None)
        }
        [perm, scope] if perm.contains('.') => {
            let (tool, action) = parse_permission_tool_action(&[*perm])?;
            (tool, action, Some(*scope))
        }
        [tool, action] => ((*tool).to_string(), (*action).to_string(), None),
        [tool, action, scope, ..] => ((*tool).to_string(), (*action).to_string(), Some(*scope)),
        _ => {
            return Err(
                "Usage: /permissions grant <tool.action> [scope] OR /permissions grant <tool> <action> [scope]"
                    .to_string(),
            );
        }
    };

    let scope = scope_str
        .map(|s| s.parse::<PermissionScope>().unwrap())
        .unwrap_or(PermissionScope::Global);
    Ok((tool, action, scope))
}

fn parse_permission_tool_action(args: &[&str]) -> Result<(String, String), String> {
    // Accept either: ["tool.action"] or ["tool", "action"]
    match args {
        [one] => {
            let parts: Vec<&str> = one.splitn(2, '.').collect();
            if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
                return Err("Expected 'tool.action' (e.g. 'file.read')".to_string());
            }
            Ok((parts[0].to_string(), parts[1].to_string()))
        }
        [tool, action] => Ok(((*tool).to_string(), (*action).to_string())),
        _ => Err("Expected 'tool.action' or '<tool> <action>'".to_string()),
    }
}

fn parse_permission_check_args(args: &[&str]) -> Result<(String, String, Option<String>), String> {
    // /permissions check <friendly|tool.action> [target]
    // /permissions check <tool> <action> [target]
    let rest = args.get(1..).unwrap_or_default();
    match rest {
        [] => Err(
            "Usage: /permissions check <read|write|shell|fetch|tool.action> [target]".to_string(),
        ),
        [action] => {
            let (tool, action) = map_check_action(action);
            Ok((tool, action, None))
        }
        [action, target] if action.contains('.') => {
            let (tool, action) = parse_permission_tool_action(&[*action])?;
            Ok((tool, action, Some((*target).to_string())))
        }
        [tool, action] if find_tool(tool).is_some() => {
            Ok(((*tool).to_string(), (*action).to_string(), None))
        }
        [tool, action, target, ..] if find_tool(tool).is_some() => Ok((
            (*tool).to_string(),
            (*action).to_string(),
            Some((*target).to_string()),
        )),
        [friendly, target] => {
            let (tool, action) = map_check_action(friendly);
            Ok((tool, action, Some((*target).to_string())))
        }
        _ => Err(
            "Usage: /permissions check <read|write|shell|fetch|tool.action> [target]".to_string(),
        ),
    }
}

fn map_check_action(action: &str) -> (String, String) {
    match action {
        "read" => ("file".to_string(), "read".to_string()),
        "write" => ("file".to_string(), "write".to_string()),
        "delete" => ("file".to_string(), "delete".to_string()),
        "run" | "exec" | "shell" => ("shell".to_string(), "run".to_string()),
        "sudo" => ("shell".to_string(), "sudo".to_string()),
        "git-read" => ("git".to_string(), "read".to_string()),
        "git-write" | "commit" | "push" => ("git".to_string(), "write".to_string()),
        "fetch" | "get" => ("web".to_string(), "fetch".to_string()),
        "post" => ("web".to_string(), "post".to_string()),
        "lint" => ("code".to_string(), "lint".to_string()),
        "test" => ("code".to_string(), "test".to_string()),
        other => {
            let parts: Vec<&str> = other.splitn(2, '.').collect();
            if parts.len() == 2 {
                (parts[0].to_string(), parts[1].to_string())
            } else {
                ("unknown".to_string(), other.to_string())
            }
        }
    }
}

// ===================== /task(s) =====================

#[derive(Debug)]
pub(crate) struct TasksOutcome {
    pub(crate) lines: Vec<String>,
    pub(crate) changed: bool,
}

pub(crate) fn run_tasks_subcommand(
    args: &[&str],
    manager: &TaskManager,
    session_id: &str,
) -> std::result::Result<TasksOutcome, String> {
    let sub = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if sub.is_empty() || sub == "help" || sub == "--help" || sub == "-h" {
        return Ok(TasksOutcome {
            lines: tasks_usage_lines(),
            changed: false,
        });
    }

    match sub.as_str() {
        "list" | "ls" => {
            let hierarchy = manager
                .get_hierarchy(session_id)
                .map_err(|e| format!("Failed to load tasks: {e}"))?;
            let lines = format_task_hierarchy(&hierarchy);
            Ok(TasksOutcome {
                lines,
                changed: false,
            })
        }
        "create" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /task create <name> [description...]".to_string());
            };
            let desc = args.get(2..).unwrap_or_default().join(" ");
            let task = manager
                .create_task(session_id, name, desc, None)
                .map_err(|e| format!("Failed to create task: {e}"))?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Created task {}: {}",
                    short_id(&task.id),
                    task.name
                )],
                changed: true,
            })
        }
        "create-sub" | "sub" => {
            let Some(parent_spec) = args.get(1).copied() else {
                return Err(
                    "Usage: /task create-sub <parent_id> <name> [description...]".to_string(),
                );
            };
            let Some(name) = args.get(2).copied() else {
                return Err(
                    "Usage: /task create-sub <parent_id> <name> [description...]".to_string(),
                );
            };
            let desc = args.get(3..).unwrap_or_default().join(" ");
            let parent_id = resolve_task_id_spec(manager, session_id, parent_spec)?;
            let task = manager
                .create_task(session_id, name, desc, Some(parent_id.clone()))
                .map_err(|e| format!("Failed to create subtask: {e}"))?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Created subtask {} under {}",
                    short_id(&task.id),
                    short_id(&parent_id)
                )],
                changed: true,
            })
        }
        "show" => {
            let Some(spec) = args.get(1).copied() else {
                return Err("Usage: /task show <id>".to_string());
            };
            let task_id = resolve_task_id_spec(manager, session_id, spec)?;
            let tasks = manager
                .list_tasks(session_id)
                .map_err(|e| format!("Failed to list tasks: {e}"))?;
            let task = tasks
                .iter()
                .find(|t| t.id == task_id)
                .ok_or_else(|| "Task not found".to_string())?;
            Ok(TasksOutcome {
                lines: format_task_details(task),
                changed: false,
            })
        }
        "status" => {
            let Some(spec) = args.get(1).copied() else {
                return Err(
                    "Usage: /task status <id> <not_started|in_progress|completed|cancelled>"
                        .to_string(),
                );
            };
            let Some(status_str) = args.get(2).copied() else {
                return Err(
                    "Usage: /task status <id> <not_started|in_progress|completed|cancelled>"
                        .to_string(),
                );
            };
            let task_id = resolve_task_id_spec(manager, session_id, spec)?;
            let status: TaskStatus = status_str
                .parse()
                .map_err(|_: String| format!("Unknown status '{status_str}'"))?;
            manager
                .update_task_status(session_id, &task_id, status)
                .map_err(|e| format!("Failed to update status: {e}"))?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Set task {} status -> {:?}",
                    short_id(&task_id),
                    status
                )],
                changed: true,
            })
        }
        "update" => {
            let Some(spec) = args.get(1).copied() else {
                return Err("Usage: /task update <id> name|desc <value...>".to_string());
            };
            let Some(field) = args.get(2).copied() else {
                return Err("Usage: /task update <id> name|desc <value...>".to_string());
            };
            let value = args.get(3..).unwrap_or_default().join(" ");
            if value.trim().is_empty() {
                return Err("Update value cannot be empty".to_string());
            }
            let task_id = resolve_task_id_spec(manager, session_id, spec)?;
            match field.to_ascii_lowercase().as_str() {
                "name" => manager
                    .update_task(session_id, &task_id, Some(value), None)
                    .map_err(|e| format!("Failed to update task: {e}"))?,
                "desc" | "description" => manager
                    .update_task(session_id, &task_id, None, Some(value))
                    .map_err(|e| format!("Failed to update task: {e}"))?,
                _ => return Err("Field must be 'name' or 'desc'".to_string()),
            }
            Ok(TasksOutcome {
                lines: vec![format!("Updated task {}", short_id(&task_id))],
                changed: true,
            })
        }
        "delete" | "del" | "rm" | "remove" => {
            // Destructive: require explicit confirmation.
            // Accept either order:
            //   /task delete --confirmed <id>
            //   /task delete <id> --confirmed
            let mut confirmed = false;
            let mut spec: Option<&str> = None;
            for a in args.iter().skip(1).copied() {
                if a == "--confirmed" {
                    confirmed = true;
                } else if spec.is_none() {
                    spec = Some(a);
                } else {
                    return Err("Usage: /task delete --confirmed <id>".to_string());
                }
            }

            if !confirmed {
                return Err(
                    "Refusing to delete without confirmation. Use: /task delete --confirmed <id>"
                        .to_string(),
                );
            }
            let Some(spec) = spec else {
                return Err("Usage: /task delete --confirmed <id>".to_string());
            };

            let task_id = resolve_task_id_spec(manager, session_id, spec)?;
            let deleted = manager
                .delete_task(session_id, &task_id)
                .map_err(|e| format!("Failed to delete task: {e}"))?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Deleted task {}: {}",
                    short_id(&deleted.id),
                    deleted.name
                )],
                changed: true,
            })
        }
        "current" => {
            let action = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            match action.as_str() {
                "" | "show" => {
                    let cur = manager
                        .get_current_task_id(session_id)
                        .map_err(|e| format!("Failed to read current task: {e}"))?;
                    Ok(TasksOutcome {
                        lines: vec![match cur {
                            Some(id) => format!("Current task: {}", short_id(&id)),
                            None => "Current task: (none)".to_string(),
                        }],
                        changed: false,
                    })
                }
                "set" => {
                    let Some(spec) = args.get(2).copied() else {
                        return Err("Usage: /task current set <id>".to_string());
                    };
                    let task_id = resolve_task_id_spec(manager, session_id, spec)?;
                    manager
                        .set_current_task_id(session_id, Some(task_id.clone()))
                        .map_err(|e| format!("Failed to set current task: {e}"))?;
                    Ok(TasksOutcome {
                        lines: vec![format!("Set current task -> {}", short_id(&task_id))],
                        changed: true,
                    })
                }
                "clear" | "unset" => {
                    manager
                        .set_current_task_id(session_id, None)
                        .map_err(|e| format!("Failed to clear current task: {e}"))?;
                    Ok(TasksOutcome {
                        lines: vec!["Cleared current task".to_string()],
                        changed: true,
                    })
                }
                _ => Err(
                    "Usage: /task current [show] | /task current set <id> | /task current clear"
                        .to_string(),
                ),
            }
        }
        "dep" | "deps" | "dependency" => {
            let action = args.get(1).copied().unwrap_or("").to_ascii_lowercase();
            if action.as_str() != "add" {
                return Err("Usage: /task dep add <task_id> <blocked_by_id>".to_string());
            }
            let Some(task_spec) = args.get(2).copied() else {
                return Err("Usage: /task dep add <task_id> <blocked_by_id>".to_string());
            };
            let Some(blocked_by_spec) = args.get(3).copied() else {
                return Err("Usage: /task dep add <task_id> <blocked_by_id>".to_string());
            };
            let task_id = resolve_task_id_spec(manager, session_id, task_spec)?;
            let blocked_by_id = resolve_task_id_spec(manager, session_id, blocked_by_spec)?;
            manager
                .add_task_dependency(session_id, &task_id, &blocked_by_id)
                .map_err(|e| format!("Failed to add dependency: {e}"))?;
            Ok(TasksOutcome {
                lines: vec![format!(
                    "Added dependency: {} blocked by {}",
                    short_id(&task_id),
                    short_id(&blocked_by_id)
                )],
                changed: true,
            })
        }
        _ => Err(format!("Unknown /task subcommand '{sub}'. Try: /task help")),
    }
}

fn tasks_usage_lines() -> Vec<String> {
    vec![
        "Task commands:".to_string(),
        "  /tasks                    (interactive browser in TUI / basic mode)".to_string(),
        "  /task                     (alias for /tasks when no args)".to_string(),
        "  /task list".to_string(),
        "  /task create <name> [description...]".to_string(),
        "  /task create-sub <parent_id> <name> [description...]".to_string(),
        "  /task show <id>".to_string(),
        "  /task update <id> name <new name...>".to_string(),
        "  /task update <id> desc <new description...>".to_string(),
        "  /task status <id> <not_started|in_progress|completed|cancelled>".to_string(),
        "  /task delete --confirmed <id>".to_string(),
        "  /task current [show]".to_string(),
        "  /task current set <id>".to_string(),
        "  /task current clear".to_string(),
        "  /task dep add <task_id> <blocked_by_id>".to_string(),
        "IDs can be full UUIDs or unique prefixes. Use '.' to refer to current task.".to_string(),
    ]
}

fn resolve_task_id_spec(
    manager: &TaskManager,
    session_id: &str,
    spec: &str,
) -> std::result::Result<String, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("Task id cannot be empty".to_string());
    }
    let current = manager
        .get_current_task_id(session_id)
        .map_err(|e| format!("Failed to read current task: {e}"))?;
    let tasks = manager
        .list_tasks(session_id)
        .map_err(|e| format!("Failed to list tasks: {e}"))?;
    resolve_task_id_from_list(spec, &tasks, current.as_deref())
}

fn resolve_task_id_from_list(
    spec: &str,
    tasks: &[Task],
    current_id: Option<&str>,
) -> std::result::Result<String, String> {
    if spec == "." || spec.eq_ignore_ascii_case("current") {
        return current_id
            .map(|s| s.to_string())
            .ok_or_else(|| "No current task set".to_string());
    }

    // Exact match wins.
    if tasks.iter().any(|t| t.id == spec) {
        return Ok(spec.to_string());
    }

    let matches: Vec<&Task> = tasks.iter().filter(|t| t.id.starts_with(spec)).collect();
    match matches.len() {
        0 => Err(format!("No task id matches prefix '{spec}'")),
        1 => Ok(matches[0].id.clone()),
        _ => {
            let mut ids: Vec<String> = matches.iter().take(8).map(|t| short_id(&t.id)).collect();
            if matches.len() > 8 {
                ids.push("…".to_string());
            }
            Err(format!(
                "Ambiguous task prefix '{spec}' (matches: {})",
                ids.join(", ")
            ))
        }
    }
}

fn format_task_hierarchy(hierarchy: &[(Task, Vec<Task>)]) -> Vec<String> {
    if hierarchy.is_empty() {
        return vec![
            "No tasks yet. Create one with: /task create <name> [description...]".to_string(),
        ];
    }
    let mut lines = vec!["━━━ Tasks ━━━".to_string(), String::new()];
    for (root, subs) in hierarchy {
        lines.push(format!(
            "{} {}  {}",
            status_icon(root.status),
            short_id(&root.id),
            root.name
        ));
        for t in subs {
            lines.push(format!(
                "  {} {}  {}",
                status_icon(t.status),
                short_id(&t.id),
                t.name
            ));
        }
    }
    lines
}

fn format_task_details(task: &Task) -> Vec<String> {
    let mut lines = vec!["━━━ Task ━━━".to_string(), String::new()];
    lines.push(format!("ID: {}", task.id));
    lines.push(format!("Name: {}", task.name));
    lines.push(format!("Status: {:?}", task.status));
    if let Some(parent) = &task.parent_id {
        lines.push(format!("Parent: {}", short_id(parent)));
    }
    if !task.blocked_by.is_empty() {
        lines.push(format!(
            "Blocked by: {}",
            task.blocked_by
                .iter()
                .map(|id| short_id(id))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    lines.push(String::new());
    lines.push("Description:".to_string());
    lines.push(task.description.clone());
    lines
}

fn status_icon(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::NotStarted => "[ ]",
        TaskStatus::InProgress => "[/]",
        TaskStatus::Completed => "[x]",
        TaskStatus::Cancelled => "[-]",
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

// ===================== /memory =====================

#[derive(Debug)]
pub(crate) struct MemoryOutcome {
    pub(crate) lines: Vec<String>,
    pub(crate) changed: bool,
    pub(crate) live_action: Option<MemoryLiveAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MemoryLiveAction {
    List,
    Search { query: String, limit: usize },
    Save { entry: MemoryBankEntry },
    ClearAll,
    Delete { file_path: PathBuf },
}

pub(crate) fn run_memory_subcommand(
    args: &[&str],
    session: &ChatSession,
) -> std::result::Result<MemoryOutcome, String> {
    let sub = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if sub.is_empty() || sub == "help" || sub == "--help" || sub == "-h" {
        return Ok(MemoryOutcome {
            lines: memory_usage_lines(),
            changed: false,
            live_action: None,
        });
    }

    let workspace_dir = session.workspace_dir().ok_or_else(|| {
        "No workspace directory configured. Cannot access memory bank.".to_string()
    })?;

    match sub.as_str() {
        "list" | "ls" => Ok(MemoryOutcome {
            lines: vec!["Listing memory bank entries…".to_string()],
            changed: false,
            live_action: Some(MemoryLiveAction::List),
        }),
        "search" => {
            // Flags: --limit <n>
            let mut limit: usize = 10;
            let mut query_parts: Vec<&str> = Vec::new();

            let mut i = 1;
            while i < args.len() {
                match args[i] {
                    "--limit" | "-l" => {
                        let Some(v) = args.get(i + 1).copied() else {
                            return Err("Usage: /memory search <query> [--limit <n>]".to_string());
                        };
                        limit = v
                            .parse::<usize>()
                            .map_err(|_| format!("Invalid --limit value: '{v}'"))?;
                        i += 2;
                    }
                    other => {
                        query_parts.push(other);
                        i += 1;
                    }
                }
            }

            let query = query_parts.join(" ").trim().to_string();
            if query.is_empty() {
                return Err("Usage: /memory search <query> [--limit <n>]".to_string());
            }

            Ok(MemoryOutcome {
                lines: vec![format!("Searching memory bank for '{query}'…")],
                changed: false,
                live_action: Some(MemoryLiveAction::Search { query, limit }),
            })
        }
        "save" => {
            // Flags:
            // - --summary <text>
            // - --category/-c <name>
            // - --last <n>
            let mut summary_override: Option<String> = None;
            let mut category: Option<String> = None;
            let mut last_n: Option<usize> = None;

            let mut i = 1;
            while i < args.len() {
                match args[i] {
                    "--summary" => {
                        let Some(v) = args.get(i + 1).copied() else {
                            return Err("Usage: /memory save [--summary <text>] [--category <name>] [--last <n>]".to_string());
                        };
                        summary_override = Some(v.to_string());
                        i += 2;
                    }
                    "--category" | "-c" => {
                        let Some(v) = args.get(i + 1).copied() else {
                            return Err("Usage: /memory save [--summary <text>] [--category <name>] [--last <n>]".to_string());
                        };
                        category = Some(v.to_string());
                        i += 2;
                    }
                    "--last" => {
                        let Some(v) = args.get(i + 1).copied() else {
                            return Err("Usage: /memory save [--summary <text>] [--category <name>] [--last <n>]".to_string());
                        };
                        last_n = Some(
                            v.parse::<usize>()
                                .map_err(|_| format!("Invalid --last value: '{v}'"))?,
                        );
                        i += 2;
                    }
                    other => {
                        return Err(format!(
                            "Unknown flag for /memory save: '{other}'. Try: --summary, --category, --last"
                        ));
                    }
                }
            }

            let mut history: Vec<String> = session
                .state
                .messages
                .iter()
                .map(|m| m.content.clone())
                .collect();
            if let Some(n) = last_n {
                if n == 0 {
                    history.clear();
                } else if history.len() > n {
                    history = history.split_off(history.len().saturating_sub(n));
                }
            }

            if history.is_empty() {
                return Err("No conversation history to save.".to_string());
            }

            let summary = summary_override
                .unwrap_or_else(|| ContextManager::new().summarize_history(&history));
            let content = history.join("\n\n");

            let entry = MemoryBankEntry {
                timestamp: chrono::Utc::now(),
                session_id: session.id.clone(),
                category,
                summary,
                content,
                file_path: None,
            };

            Ok(MemoryOutcome {
                lines: vec!["Saving conversation to memory bank…".to_string()],
                changed: true,
                live_action: Some(MemoryLiveAction::Save { entry }),
            })
        }
        "clear" => {
            let confirmed = args.contains(&"--confirmed");
            if !confirmed {
                return Err(
                    "Refusing to clear without confirmation. Use: /memory clear --confirmed"
                        .to_string(),
                );
            }
            Ok(MemoryOutcome {
                lines: vec!["Clearing memory bank…".to_string()],
                changed: true,
                live_action: Some(MemoryLiveAction::ClearAll),
            })
        }
        "delete" => {
            let mut confirmed = false;
            let mut path_arg: Option<&str> = None;
            for a in args.iter().skip(1).copied() {
                if a == "--confirmed" {
                    confirmed = true;
                } else {
                    path_arg = Some(a);
                }
            }

            if !confirmed {
                return Err(
                    "Refusing to delete without confirmation. Use: /memory delete --confirmed <path>"
                        .to_string(),
                );
            }

            let Some(path_str) = path_arg else {
                return Err("Usage: /memory delete --confirmed <path>".to_string());
            };

            let input_path = std::path::Path::new(path_str);
            let resolved = if input_path.is_absolute() {
                input_path.to_path_buf()
            } else {
                workspace_dir.join(input_path)
            };

            Ok(MemoryOutcome {
                lines: vec![format!("Deleting memory entry: {path_str}")],
                changed: true,
                live_action: Some(MemoryLiveAction::Delete {
                    file_path: resolved,
                }),
            })
        }
        other => Err(format!(
            "Unknown /memory subcommand: '{other}'. Try: list, search, save, clear, delete"
        )),
    }
}

fn memory_usage_lines() -> Vec<String> {
    vec![
        "━━━ /memory ━━━".to_string(),
        String::new(),
        "Interactive: /memory".to_string(),
        String::new(),
        "Commands:".to_string(),
        "  /memory list".to_string(),
        "  /memory search <query> [--limit <n>]".to_string(),
        "  /memory save [--summary <text>] [--category <name>] [--last <n>]".to_string(),
        "  /memory delete --confirmed <path>".to_string(),
        "  /memory clear --confirmed".to_string(),
        String::new(),
        "Destructive actions require --confirmed (or use the interactive UI).".to_string(),
    ]
}

// ===================== /mcp =====================

#[derive(Debug)]
pub(crate) struct McpOutcome {
    pub(crate) lines: Vec<String>,
    pub(crate) changed: bool,
    /// Live actions that must be executed by a caller that has a Tokio runtime
    /// and access to the MCP registry.
    pub(crate) live_action: Option<McpLiveAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum McpLiveAction {
    Status,
    Tools { server: Option<String> },
    Connect { name: String },
    Disconnect { name: String },
}

pub(crate) fn run_mcp_subcommand(
    args: &[&str],
    config: &mut AppConfig,
) -> std::result::Result<McpOutcome, String> {
    let sub = args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if sub.is_empty() || sub == "help" || sub == "--help" || sub == "-h" {
        return Ok(McpOutcome {
            lines: mcp_usage_lines(),
            changed: false,
            live_action: None,
        });
    }

    match sub.as_str() {
        "list" | "ls" => Ok(McpOutcome {
            lines: mcp_list_lines(config),
            changed: false,
            live_action: None,
        }),
        "get" | "show" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp get <name>".to_string());
            };
            let srv = config
                .find_mcp_server(name)
                .ok_or_else(|| format!("MCP server '{name}' not found"))?;
            Ok(McpOutcome {
                lines: mcp_server_details_lines(srv),
                changed: false,
                live_action: None,
            })
        }
        "enable" | "disable" => {
            let Some(name) = args.get(1).copied() else {
                return Err(format!("Usage: /mcp {sub} <name>"));
            };
            let enabled = sub == "enable";
            let srv = config
                .find_mcp_server_mut(name)
                .ok_or_else(|| format!("MCP server '{name}' not found"))?;
            let changed = srv.enabled != enabled;
            srv.enabled = enabled;
            Ok(McpOutcome {
                lines: vec![format!(
                    "{} MCP server: {name}",
                    if enabled { "Enabled" } else { "Disabled" }
                )],
                changed,
                live_action: None,
            })
        }
        "remove" | "rm" | "delete" | "del" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp remove <name>".to_string());
            };
            let before = config.mcp_servers.len();
            config.mcp_servers.retain(|s| s.name != name);
            let changed = config.mcp_servers.len() != before;
            if changed {
                Ok(McpOutcome {
                    lines: vec![format!("Removed MCP server: {name}")],
                    changed: true,
                    live_action: None,
                })
            } else {
                Ok(McpOutcome {
                    lines: vec![format!("MCP server not found: {name}")],
                    changed: false,
                    live_action: None,
                })
            }
        }
        "add" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp add <name> [endpoint] [flags...]".to_string());
            };
            if config.mcp_servers.iter().any(|s| s.name == name) {
                return Err(format!(
                    "MCP server '{name}' already exists. Use: /mcp edit {name} ..."
                ));
            }

            let (pos_endpoint, rest) = split_positional_endpoint(args.get(2..).unwrap_or_default());
            let parsed = McpParsedArgs::parse(rest)?;
            let transport = parsed
                .transport
                .or_else(|| infer_transport_from_endpoint(pos_endpoint.as_deref()))
                .unwrap_or(McpTransportType::Stdio);

            let mut entry = McpServerEntry {
                name: name.to_string(),
                transport,
                enabled: parsed.enabled.unwrap_or(true),
                scope: parsed.scope.unwrap_or(McpScope::User),
                timeout_secs: parsed.timeout_secs.unwrap_or(30),
                auto_reconnect: parsed.auto_reconnect.unwrap_or(true),
                ..McpServerEntry::default()
            };

            apply_transport_specific_add_fields(&mut entry, transport, &parsed, pos_endpoint)?;
            validate_mcp_entry(&entry)?;

            config.mcp_servers.push(entry);
            Ok(McpOutcome {
                lines: vec![format!("Added MCP server: {name}")],
                changed: true,
                live_action: None,
            })
        }
        "edit" | "update" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp edit <name> [endpoint] [flags...]".to_string());
            };

            let (pos_endpoint, rest) = split_positional_endpoint(args.get(2..).unwrap_or_default());
            let parsed = McpParsedArgs::parse(rest)?;

            let srv = config
                .find_mcp_server_mut(name)
                .ok_or_else(|| format!("MCP server '{name}' not found"))?;
            let before = srv.clone();
            apply_mcp_edit_patch(srv, &parsed, pos_endpoint)?;
            validate_mcp_entry(srv)?;
            let changed = *srv != before;

            Ok(McpOutcome {
                lines: vec![format!("Updated MCP server: {name}")],
                changed,
                live_action: None,
            })
        }
        // Live / runtime-backed operations. We parse + validate inputs here,
        // but a caller must actually execute them.
        "status" => Ok(McpOutcome {
            lines: vec!["Fetching MCP status...".to_string()],
            changed: false,
            live_action: Some(McpLiveAction::Status),
        }),
        "tools" => {
            let server = args.get(1).copied().map(|s| s.to_string());
            Ok(McpOutcome {
                lines: vec!["Fetching MCP tools...".to_string()],
                changed: false,
                live_action: Some(McpLiveAction::Tools { server }),
            })
        }
        "connect" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp connect <name>".to_string());
            };
            // Ensure server exists (and surfaces a good error message) even though
            // the actual connect is performed by the caller.
            let _ = config
                .find_mcp_server(name)
                .ok_or_else(|| format!("MCP server '{name}' not found"))?;
            Ok(McpOutcome {
                lines: vec![format!("Connecting to MCP server: {name}...")],
                changed: false,
                live_action: Some(McpLiveAction::Connect {
                    name: name.to_string(),
                }),
            })
        }
        "disconnect" => {
            let Some(name) = args.get(1).copied() else {
                return Err("Usage: /mcp disconnect <name>".to_string());
            };
            Ok(McpOutcome {
                lines: vec![format!("Disconnecting from MCP server: {name}...")],
                changed: false,
                live_action: Some(McpLiveAction::Disconnect {
                    name: name.to_string(),
                }),
            })
        }
        _ => Err(format!("Unknown /mcp subcommand '{sub}'. Try: /mcp help")),
    }
}

fn mcp_usage_lines() -> Vec<String> {
    vec![
        "MCP commands:".to_string(),
        "  /mcp                     (interactive browser/overlay)".to_string(),
        "  /mcp list".to_string(),
        "  /mcp get <name>".to_string(),
        "  /mcp add <name> [endpoint] [flags...]".to_string(),
        "  /mcp edit <name> [endpoint] [flags...]".to_string(),
        "  /mcp remove <name>".to_string(),
        "  /mcp enable <name>".to_string(),
        "  /mcp disable <name>".to_string(),
        "  /mcp status              (requires runtime)".to_string(),
        "  /mcp tools [server]      (requires runtime)".to_string(),
        "  /mcp connect <name>      (requires runtime)".to_string(),
        "  /mcp disconnect <name>   (requires runtime)".to_string(),
        "".to_string(),
        "Flags (add/edit):".to_string(),
        "  --transport|-t stdio|http|sse".to_string(),
        "  --scope|-s user|project|local".to_string(),
        "  --timeout <secs>".to_string(),
        "  --auto-reconnect | --no-auto-reconnect".to_string(),
        "  --enabled | --disabled".to_string(),
        "".to_string(),
        "Stdio flags:".to_string(),
        "  --command <cmd>   (or positional endpoint)".to_string(),
        "  --arg <value>     (repeatable)".to_string(),
        "  --env KEY=VALUE   (repeatable)".to_string(),
        "".to_string(),
        "HTTP/SSE flags:".to_string(),
        "  --url <url>       (or positional endpoint)".to_string(),
        "  --header K:V      (repeatable)".to_string(),
        "".to_string(),
        "Edit-only helpers:".to_string(),
        "  --clear-args | --clear-env | --clear-headers".to_string(),
    ]
}

fn mcp_list_lines(config: &AppConfig) -> Vec<String> {
    if config.mcp_servers.is_empty() {
        return vec![
            "No MCP servers configured.".to_string(),
            "Add one with: /mcp (interactive)".to_string(),
        ];
    }
    let mut lines = vec!["━━━ MCP Servers ━━━".to_string(), String::new()];
    for srv in &config.mcp_servers {
        let status = if srv.enabled { "✓" } else { "○" };
        lines.push(format!(
            "{status} {:<20} [{:<5}] {:<7} {}",
            srv.name,
            format!("{}", srv.transport),
            format!("{}", srv.scope),
            mcp_endpoint_hint(srv)
        ));
    }
    lines
}

fn mcp_endpoint_hint(srv: &McpServerEntry) -> String {
    match srv.transport {
        McpTransportType::Stdio => {
            let cmd = srv.command.as_deref().unwrap_or("(no command)");
            let args = if srv.args.is_empty() {
                "".to_string()
            } else {
                format!(" {}", srv.args.join(" "))
            };
            format!("{}{}", cmd, args)
        }
        McpTransportType::Http | McpTransportType::Sse => {
            srv.url.as_deref().unwrap_or("(no url)").to_string()
        }
    }
}

fn mcp_server_details_lines(srv: &McpServerEntry) -> Vec<String> {
    let mut lines = vec!["━━━ MCP Server ━━━".to_string(), String::new()];
    lines.push(format!("Name:           {}", srv.name));
    lines.push(format!("Transport:      {}", srv.transport));
    lines.push(format!("Scope:          {}", srv.scope));
    lines.push(format!(
        "Enabled:        {}",
        if srv.enabled { "yes" } else { "no" }
    ));
    lines.push(format!("Timeout:        {}s", srv.timeout_secs));
    lines.push(format!(
        "Auto-reconnect: {}",
        if srv.auto_reconnect { "yes" } else { "no" }
    ));
    lines.push(String::new());
    match srv.transport {
        McpTransportType::Stdio => {
            lines.push(format!(
                "Command:        {}",
                srv.command.as_deref().unwrap_or("(none)")
            ));
            if !srv.args.is_empty() {
                lines.push(format!("Args:           {}", srv.args.join(" ")));
            }
            if !srv.env.is_empty() {
                lines.push(format!("Env:            {} vars", srv.env.len()));
            }
        }
        McpTransportType::Http | McpTransportType::Sse => {
            lines.push(format!(
                "URL:            {}",
                srv.url.as_deref().unwrap_or("(none)")
            ));
            if !srv.headers.is_empty() {
                lines.push(format!("Headers:        {}", srv.headers.len()));
            }
        }
    }
    lines
}

fn split_positional_endpoint<'a>(rest: &'a [&'a str]) -> (Option<String>, &'a [&'a str]) {
    let Some(first) = rest.first().copied() else {
        return (None, rest);
    };
    if first.starts_with('-') {
        (None, rest)
    } else {
        (Some(first.to_string()), &rest[1..])
    }
}

#[derive(Default, Debug, Clone)]
struct McpParsedArgs {
    transport: Option<McpTransportType>,
    scope: Option<McpScope>,
    timeout_secs: Option<u64>,
    auto_reconnect: Option<bool>,
    enabled: Option<bool>,

    command: Option<String>,
    url: Option<String>,
    args: Option<Vec<String>>,
    env: std::collections::HashMap<String, String>,
    headers: std::collections::HashMap<String, String>,

    clear_args: bool,
    clear_env: bool,
    clear_headers: bool,
}

impl McpParsedArgs {
    fn parse(rest: &[&str]) -> Result<Self, String> {
        let mut out = Self::default();
        let mut i = 0;
        while i < rest.len() {
            match rest[i] {
                "--transport" | "-t" => {
                    let val = rest.get(i + 1).copied().ok_or_else(|| {
                        "Missing value for --transport. Try: --transport stdio|http|sse".to_string()
                    })?;
                    out.transport = Some(parse_mcp_transport(val)?);
                    i += 2;
                }
                "--scope" | "-s" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --scope".to_string())?;
                    out.scope = Some(parse_mcp_scope(val)?);
                    i += 2;
                }
                "--timeout" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --timeout".to_string())?;
                    out.timeout_secs = Some(val.parse::<u64>().map_err(|_| {
                        "--timeout must be an integer number of seconds".to_string()
                    })?);
                    i += 2;
                }
                "--auto-reconnect" => {
                    out.auto_reconnect = Some(true);
                    i += 1;
                }
                "--no-auto-reconnect" => {
                    out.auto_reconnect = Some(false);
                    i += 1;
                }
                "--enabled" => {
                    out.enabled = Some(true);
                    i += 1;
                }
                "--disabled" => {
                    out.enabled = Some(false);
                    i += 1;
                }
                "--command" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --command".to_string())?;
                    out.command = Some(val.to_string());
                    i += 2;
                }
                "--url" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --url".to_string())?;
                    out.url = Some(val.to_string());
                    i += 2;
                }
                "--arg" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --arg".to_string())?;
                    out.args.get_or_insert_with(Vec::new).push(val.to_string());
                    i += 2;
                }
                "--env" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --env".to_string())?;
                    let (k, v) = parse_key_value(val, '=')
                        .ok_or_else(|| "--env must be KEY=VALUE".to_string())?;
                    out.env.insert(k, v);
                    i += 2;
                }
                "--header" => {
                    let val = rest
                        .get(i + 1)
                        .copied()
                        .ok_or_else(|| "Missing value for --header".to_string())?;
                    let (k, v) = parse_key_value(val, ':')
                        .or_else(|| parse_key_value(val, '='))
                        .ok_or_else(|| "--header must be 'Key: Value'".to_string())?;
                    out.headers.insert(k, v);
                    i += 2;
                }
                "--clear-args" => {
                    out.clear_args = true;
                    i += 1;
                }
                "--clear-env" => {
                    out.clear_env = true;
                    i += 1;
                }
                "--clear-headers" => {
                    out.clear_headers = true;
                    i += 1;
                }
                other if other.starts_with('-') => {
                    return Err(format!("Unknown flag '{other}'. Try: /mcp help"));
                }
                other => {
                    return Err(format!(
                        "Unexpected positional argument '{other}'. If you meant an endpoint, it must come immediately after the name."
                    ));
                }
            }
        }
        Ok(out)
    }
}

fn parse_key_value(input: &str, sep: char) -> Option<(String, String)> {
    let (k, v) = input.split_once(sep)?;
    let k = k.trim();
    let v = v.trim();
    if k.is_empty() {
        return None;
    }
    Some((k.to_string(), v.to_string()))
}

fn parse_mcp_transport(s: &str) -> Result<McpTransportType, String> {
    s.parse::<McpTransportType>()
        .map_err(|e| format!("Invalid transport '{s}': {e}"))
}

fn parse_mcp_scope(s: &str) -> Result<McpScope, String> {
    s.parse::<McpScope>()
        .map_err(|e| format!("Invalid scope '{s}': {e}"))
}

fn apply_transport_specific_add_fields(
    entry: &mut McpServerEntry,
    transport: McpTransportType,
    parsed: &McpParsedArgs,
    pos_endpoint: Option<String>,
) -> Result<(), String> {
    match transport {
        McpTransportType::Stdio => {
            entry.command = parsed.command.clone().or(pos_endpoint);
            entry.args = parsed.args.clone().unwrap_or_default();
            entry.env = parsed.env.clone();
        }
        McpTransportType::Http | McpTransportType::Sse => {
            entry.url = parsed.url.clone().or(pos_endpoint);
            entry.headers = parsed.headers.clone();
        }
    }
    Ok(())
}

fn apply_mcp_edit_patch(
    srv: &mut McpServerEntry,
    parsed: &McpParsedArgs,
    pos_endpoint: Option<String>,
) -> Result<(), String> {
    if let Some(scope) = parsed.scope {
        srv.scope = scope;
    }
    if let Some(timeout) = parsed.timeout_secs {
        srv.timeout_secs = timeout;
    }
    if let Some(enabled) = parsed.enabled {
        srv.enabled = enabled;
    }
    if let Some(ar) = parsed.auto_reconnect {
        srv.auto_reconnect = ar;
    }

    if let Some(new_transport) = parsed.transport
        && srv.transport != new_transport
    {
        srv.transport = new_transport;
        // Clear fields that are irrelevant in the new transport.
        match new_transport {
            McpTransportType::Stdio => {
                srv.url = None;
                srv.headers.clear();
            }
            McpTransportType::Http | McpTransportType::Sse => {
                srv.command = None;
                srv.args.clear();
                srv.env.clear();
            }
        }
    }

    // Transport-specific updates
    match srv.transport {
        McpTransportType::Stdio => {
            if let Some(cmd) = &parsed.command {
                srv.command = Some(cmd.clone());
            } else if let Some(ep) = pos_endpoint {
                srv.command = Some(ep);
            }
            if parsed.clear_args {
                srv.args.clear();
            }
            if let Some(args) = &parsed.args {
                srv.args = args.clone();
            }
            if parsed.clear_env {
                srv.env.clear();
            }
            for (k, v) in &parsed.env {
                srv.env.insert(k.clone(), v.clone());
            }
        }
        McpTransportType::Http | McpTransportType::Sse => {
            if let Some(url) = &parsed.url {
                srv.url = Some(url.clone());
            } else if let Some(ep) = pos_endpoint {
                srv.url = Some(ep);
            }
            if parsed.clear_headers {
                srv.headers.clear();
            }
            for (k, v) in &parsed.headers {
                srv.headers.insert(k.clone(), v.clone());
            }
        }
    }

    Ok(())
}

fn validate_mcp_entry(entry: &McpServerEntry) -> Result<(), String> {
    if entry.name.trim().is_empty() {
        return Err("MCP server name cannot be empty".to_string());
    }
    if entry.timeout_secs == 0 {
        return Err("timeout_secs must be > 0".to_string());
    }

    match entry.transport {
        McpTransportType::Stdio => {
            let cmd = entry.command.as_deref().unwrap_or("").trim();
            if cmd.is_empty() {
                return Err("stdio transport requires a non-empty command".to_string());
            }
            if entry.url.is_some() {
                return Err("stdio transport cannot set url".to_string());
            }
            if !entry.headers.is_empty() {
                return Err("stdio transport cannot set headers".to_string());
            }
        }
        McpTransportType::Http | McpTransportType::Sse => {
            let url = entry.url.as_deref().unwrap_or("").trim();
            if url.is_empty() {
                return Err("http/sse transport requires a non-empty url".to_string());
            }
            if entry.command.is_some() {
                return Err("http/sse transport cannot set command".to_string());
            }
            if !entry.args.is_empty() {
                return Err("http/sse transport cannot set args".to_string());
            }
            if !entry.env.is_empty() {
                return Err("http/sse transport cannot set env".to_string());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gestura_core::chat_sessions::MessageSource;
    use uuid::Uuid;

    fn new_test_session() -> ChatSession {
        let base = std::env::temp_dir()
            .join("gestura-slash-tests")
            .join(Uuid::new_v4().to_string());
        std::fs::create_dir_all(&base).unwrap();

        ChatSession::new_with_workspace(base, None).unwrap()
    }

    #[test]
    fn memory_help_does_not_require_workspace() {
        let mut session = new_test_session();
        session.state.workspace_dir = None;

        let out = run_memory_subcommand(&["help"], &session).unwrap();
        assert!(out.live_action.is_none());
        assert!(out.lines.iter().any(|l| l.contains("/memory")));
    }

    #[test]
    fn memory_list_requires_workspace() {
        let mut session = new_test_session();
        session.state.workspace_dir = None;

        let err = run_memory_subcommand(&["list"], &session).unwrap_err();
        assert!(err.contains("No workspace directory"));
    }

    #[test]
    fn memory_search_parses_query_and_limit() {
        let session = new_test_session();

        let out =
            run_memory_subcommand(&["search", "hello", "world", "--limit", "5"], &session).unwrap();
        assert_eq!(
            out.live_action,
            Some(MemoryLiveAction::Search {
                query: "hello world".to_string(),
                limit: 5
            })
        );

        let err = run_memory_subcommand(&["search", "--limit", "5"], &session).unwrap_err();
        assert!(err.contains("Usage: /memory search"));
    }

    #[test]
    fn memory_save_validates_history_and_last_n() {
        let mut session = new_test_session();

        let err = run_memory_subcommand(&["save"], &session).unwrap_err();
        assert!(err.contains("No conversation history"));

        session.add_user_message("u1", MessageSource::Text);
        session.add_assistant_message("a1", None);

        let out = run_memory_subcommand(
            &[
                "save",
                "--summary",
                "sum",
                "--category",
                "cat",
                "--last",
                "1",
            ],
            &session,
        )
        .unwrap();
        assert!(out.changed);

        let act = out.live_action.unwrap();
        match act {
            MemoryLiveAction::Save { entry } => {
                assert_eq!(entry.session_id, session.id);
                assert_eq!(entry.summary, "sum");
                assert_eq!(entry.category, Some("cat".to_string()));
                assert!(entry.content.contains("a1"));
                assert!(!entry.content.contains("u1"));
            }
            other => panic!("expected Save live action, got {other:?}"),
        }

        let err = run_memory_subcommand(&["save", "--last", "0"], &session).unwrap_err();
        assert!(err.contains("No conversation history"));
    }

    #[test]
    fn memory_clear_and_delete_require_confirmed_and_resolve_paths() {
        let session = new_test_session();

        let err = run_memory_subcommand(&["clear"], &session).unwrap_err();
        assert!(err.contains("--confirmed"));

        let out = run_memory_subcommand(&["clear", "--confirmed"], &session).unwrap();
        assert_eq!(out.live_action, Some(MemoryLiveAction::ClearAll));

        let err = run_memory_subcommand(&["delete", "foo.json"], &session).unwrap_err();
        assert!(err.contains("Refusing to delete"));

        let err = run_memory_subcommand(&["delete", "--confirmed"], &session).unwrap_err();
        assert!(err.contains("Usage: /memory delete"));

        let ws = session.workspace_dir().unwrap().clone();
        let out =
            run_memory_subcommand(&["delete", "--confirmed", "rel/path.json"], &session).unwrap();
        match out.live_action {
            Some(MemoryLiveAction::Delete { file_path }) => {
                assert_eq!(file_path, ws.join("rel/path.json"));
            }
            other => panic!("expected Delete live action, got {other:?}"),
        }

        let abs = std::env::temp_dir().join("gestura-abs-delete.json");
        let abs_str = abs.to_string_lossy().to_string();
        let out =
            run_memory_subcommand(&["delete", "--confirmed", abs_str.as_str()], &session).unwrap();
        match out.live_action {
            Some(MemoryLiveAction::Delete { file_path }) => {
                assert_eq!(file_path, abs);
            }
            other => panic!("expected Delete live action, got {other:?}"),
        }
    }

    #[test]
    fn tasks_delete_requires_confirmed() {
        use gestura_core::tasks::TaskManager;

        let base = std::env::temp_dir()
            .join("gestura-slash-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&base).unwrap();

        let manager = TaskManager::new(&base);
        let session_id = "session-slash-test";
        let task = manager
            .create_task(session_id, "TestTask", "desc", None)
            .unwrap();

        let err =
            run_tasks_subcommand(&["delete", task.id.as_str()], &manager, session_id).unwrap_err();
        assert!(err.contains("--confirmed"));

        let out = run_tasks_subcommand(
            &["delete", "--confirmed", task.id.as_str()],
            &manager,
            session_id,
        )
        .unwrap();
        assert!(out.changed);
        assert!(out.lines.join("\n").contains("Deleted task"));
    }

    #[test]
    fn hook_event_parsing_accepts_variants() {
        assert_eq!(
            "pre_pipeline".parse::<HookEvent>().ok(),
            Some(HookEvent::PrePipeline)
        );
        assert_eq!(
            "pre-pipeline".parse::<HookEvent>().ok(),
            Some(HookEvent::PrePipeline)
        );
        assert_eq!(
            "PreTool".parse::<HookEvent>().ok(),
            Some(HookEvent::PreTool)
        );
        assert_eq!(
            "post tool".parse::<HookEvent>().ok(),
            Some(HookEvent::PostTool)
        );
        assert_eq!("nope".parse::<HookEvent>().ok(), None);
    }

    #[test]
    fn resolve_task_id_from_list_supports_prefix_and_current() {
        let mut t1 = Task::new("session-1", "A", "", None);
        t1.id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string();

        let mut t2 = t1.clone();
        t2.id = "bbbbbbbb-1111-2222-3333-444444444444".to_string();

        let mut t3 = t1.clone();
        t3.id = "bbbbbbbb-9999-8888-7777-666666666666".to_string();

        let tasks = vec![t1.clone(), t2.clone(), t3.clone()];
        let resolved = resolve_task_id_from_list("aaaa", &tasks, None).unwrap();
        assert_eq!(resolved, t1.id);

        let resolved = resolve_task_id_from_list(".", &tasks, Some(&t2.id)).unwrap();
        assert_eq!(resolved, t2.id);

        let err = resolve_task_id_from_list("b", &tasks, None).unwrap_err();
        assert!(err.contains("Ambiguous"));
    }

    #[test]
    fn permissions_parsing_accepts_tool_action_and_tool_dot_action() {
        let (tool, action) = parse_permission_tool_action(&["file.read"]).unwrap();
        assert_eq!(tool, "file");
        assert_eq!(action, "read");

        let (tool, action) = parse_permission_tool_action(&["shell", "run"]).unwrap();
        assert_eq!(tool, "shell");
        assert_eq!(action, "run");
    }

    #[test]
    fn permission_level_parsing_accepts_variants() {
        assert_eq!(
            "full-permissions".parse::<SessionPermissionLevel>().ok(),
            Some(SessionPermissionLevel::Full)
        );
        assert_eq!(
            "restricted".parse::<SessionPermissionLevel>().ok(),
            Some(SessionPermissionLevel::Restricted)
        );
        assert_eq!(
            "sandbox".parse::<SessionPermissionLevel>().ok(),
            Some(SessionPermissionLevel::Sandbox)
        );
        assert_eq!("nope".parse::<SessionPermissionLevel>().ok(), None);
    }

    #[test]
    fn mcp_add_validates_transport_specific_requirements() {
        let mut cfg = AppConfig::default();

        // Missing command for stdio.
        let err =
            run_mcp_subcommand(&["add", "srv1", "--transport", "stdio"], &mut cfg).unwrap_err();
        assert!(err.contains("requires"));

        // Valid stdio add.
        let out = run_mcp_subcommand(
            &[
                "add",
                "srv1",
                "--transport",
                "stdio",
                "--command",
                "npx",
                "--arg",
                "-y",
            ],
            &mut cfg,
        )
        .unwrap();
        assert!(out.changed);
        assert_eq!(cfg.mcp_servers.len(), 1);

        // Duplicate name.
        let err = run_mcp_subcommand(
            &["add", "srv1", "--transport", "http", "--url", "https://x"],
            &mut cfg,
        )
        .unwrap_err();
        assert!(err.contains("already exists"));
    }
}
