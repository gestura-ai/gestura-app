//! Permissions management tool
//!
//! Provides permission management:
//! - list: List current permissions
//! - grant: Grant a permission
//! - revoke: Revoke a permission
//! - reset: Reset to defaults
//! - check: Check if action is allowed

use super::super::Result;
use colored::Colorize;
use gestura_core::tools::permissions::{PermissionManager, PermissionScope};
use std::sync::OnceLock;

/// Global permission manager instance
static PERMISSION_MANAGER: OnceLock<PermissionManager> = OnceLock::new();

fn get_permission_manager() -> &'static PermissionManager {
    PERMISSION_MANAGER.get_or_init(PermissionManager::new)
}

/// Permissions subcommand options
pub enum PermissionsSubcommand {
    List,
    Grant {
        permission: String,
        scope: Option<String>,
    },
    Revoke {
        permission: String,
    },
    Reset,
    Check {
        action: String,
        target: Option<String>,
    },
}

/// Run permissions subcommand
pub fn run(cmd: PermissionsSubcommand) -> Result<()> {
    match cmd {
        PermissionsSubcommand::List => run_list(),
        PermissionsSubcommand::Grant { permission, scope } => {
            run_grant(&permission, scope.as_deref())
        }
        PermissionsSubcommand::Revoke { permission } => run_revoke(&permission),
        PermissionsSubcommand::Reset => run_reset(),
        PermissionsSubcommand::Check { action, target } => run_check(&action, target.as_deref()),
    }
}

fn run_list() -> Result<()> {
    println!("{}", "Permissions".bold().underline());
    println!();

    let perms = get_permission_manager().list()?;

    if perms.is_empty() {
        println!("{}", "(No permissions granted)".dimmed());
    } else {
        // Group by tool
        let mut current_tool = String::new();
        let mut sorted = perms;
        sorted.sort_by(|a, b| (&a.tool, &a.action).cmp(&(&b.tool, &b.action)));

        for perm in sorted {
            if perm.tool != current_tool {
                if !current_tool.is_empty() {
                    println!();
                }
                println!("{}", format!("{}:", perm.tool.to_uppercase()).dimmed());
                current_tool = perm.tool.clone();
            }

            let scope_info = match &perm.scope {
                PermissionScope::Global => "global".to_string(),
                PermissionScope::Path(p) => format!("path: {}", p),
                PermissionScope::Command(c) => format!("cmd: {}", c),
            };

            println!(
                "  {} {:20} ({})",
                "✓".green(),
                perm.action.cyan(),
                scope_info.dimmed()
            );
        }
    }

    println!();
    println!(
        "{}",
        "Use 'gestura tools permissions grant <tool.action>' to add".dimmed()
    );

    Ok(())
}

fn run_grant(permission: &str, scope: Option<&str>) -> Result<()> {
    // Parse permission as "tool.action"
    let parts: Vec<&str> = permission.splitn(2, '.').collect();
    if parts.len() != 2 {
        println!(
            "{} Invalid permission format. Use 'tool.action' (e.g., 'file.read')",
            "✗".red()
        );
        return Ok(());
    }

    let (tool, action) = (parts[0], parts[1]);
    let perm_scope = match scope {
        Some(s) if s.starts_with('/') => PermissionScope::Path(s.to_string()),
        Some(s) => PermissionScope::Command(s.to_string()),
        None => PermissionScope::Global,
    };

    get_permission_manager().grant(tool, action, perm_scope.clone(), None)?;

    let scope_msg = match &perm_scope {
        PermissionScope::Global => String::new(),
        PermissionScope::Path(p) => format!(" (path: {})", p),
        PermissionScope::Command(c) => format!(" (cmd: {})", c),
    };
    println!(
        "{} Granted: {}{}",
        "✓".green(),
        permission.cyan(),
        scope_msg
    );

    Ok(())
}

fn run_revoke(permission: &str) -> Result<()> {
    let parts: Vec<&str> = permission.splitn(2, '.').collect();
    if parts.len() != 2 {
        println!(
            "{} Invalid permission format. Use 'tool.action' (e.g., 'file.read')",
            "✗".red()
        );
        return Ok(());
    }

    let (tool, action) = (parts[0], parts[1]);
    let count = get_permission_manager().revoke(tool, action)?;

    if count > 0 {
        println!(
            "{} Revoked: {} ({} permission(s))",
            "✓".green(),
            permission.cyan(),
            count
        );
    } else {
        println!(
            "{} No matching permission found: {}",
            "⚠".yellow(),
            permission
        );
    }

    Ok(())
}

fn run_reset() -> Result<()> {
    let count = get_permission_manager().reset()?;

    println!("{} Permissions reset ({} removed)", "✓".green(), count);
    println!();
    run_list()?;

    Ok(())
}

fn run_check(action: &str, target: Option<&str>) -> Result<()> {
    // Map action to tool.action
    let (tool, action_name) = match action {
        "read" => ("file", "read"),
        "write" => ("file", "write"),
        "delete" => ("file", "delete"),
        "run" | "exec" | "shell" => ("shell", "run"),
        "sudo" => ("shell", "sudo"),
        "git-read" => ("git", "read"),
        "git-write" | "commit" | "push" => ("git", "write"),
        "fetch" | "get" => ("web", "fetch"),
        "post" => ("web", "post"),
        "lint" => ("code", "lint"),
        "test" => ("code", "test"),
        other => {
            let parts: Vec<&str> = other.splitn(2, '.').collect();
            if parts.len() == 2 {
                (parts[0], parts[1])
            } else {
                ("unknown", other)
            }
        }
    };

    let target_info = target.map(|t| format!(" on '{}'", t)).unwrap_or_default();
    let check = get_permission_manager().check(tool, action_name, target)?;

    if check.allowed {
        println!(
            "{} Action '{}'{} is {}",
            "✓".green(),
            action.cyan(),
            target_info,
            "ALLOWED".green().bold()
        );
    } else {
        println!(
            "{} Action '{}'{} is {}",
            "✗".red(),
            action.cyan(),
            target_info,
            "DENIED".red().bold()
        );
        println!("  {}", check.reason.dimmed());
        println!();
        println!(
            "{}",
            format!(
                "Grant with: gestura tools permissions grant {}.{}",
                tool, action_name
            )
            .dimmed()
        );
    }

    Ok(())
}
