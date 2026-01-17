//! Git operations tool
//!
//! Provides git operations:
//! - status: Show git status
//! - diff: Show git diff
//! - log: Show git log
//! - commit: Create a commit
//! - undo: Undo last commit
//! - branch: Branch operations
//! - checkout: Checkout branch/file
//! - stash: Stash operations
//! - blame: Show file blame
//! - conflicts: Show merge conflicts
//! - resolve: Resolve conflicts

use super::super::Result;
use colored::Colorize;
use gestura_core::tools::git::{ChangeStatus, GitTools};
use std::path::Path;
use std::sync::OnceLock;

/// Global git tools instance
static GIT_TOOLS: OnceLock<GitTools> = OnceLock::new();

fn get_git_tools() -> &'static GitTools {
    GIT_TOOLS.get_or_init(GitTools::default)
}

/// Git subcommand options
pub enum GitSubcommand {
    Status,
    Diff {
        path: Option<String>,
        staged: bool,
    },
    Log {
        count: Option<usize>,
        oneline: bool,
    },
    Commit {
        message: Option<String>,
        all: bool,
    },
    Undo {
        soft: bool,
    },
    Branch {
        name: Option<String>,
        delete: bool,
    },
    Checkout {
        target: String,
    },
    Stash {
        action: Option<String>,
    },
    Blame {
        path: String,
    },
    Conflicts,
    Resolve {
        path: String,
        strategy: Option<String>,
    },
}

/// Run git subcommand
pub fn run(cmd: GitSubcommand) -> Result<()> {
    match cmd {
        GitSubcommand::Status => run_status(),
        GitSubcommand::Diff { path, staged } => run_diff(path.as_deref(), staged),
        GitSubcommand::Log { count, oneline } => run_log(count, oneline),
        GitSubcommand::Commit { message, all } => run_commit(message.as_deref(), all),
        GitSubcommand::Undo { soft } => run_undo(soft),
        GitSubcommand::Branch { name, delete } => run_branch(name.as_deref(), delete),
        GitSubcommand::Checkout { target } => run_checkout(&target),
        GitSubcommand::Stash { action } => run_stash(action.as_deref()),
        GitSubcommand::Blame { path } => run_blame(&path),
        GitSubcommand::Conflicts => run_conflicts(),
        GitSubcommand::Resolve { path, strategy } => run_resolve(&path, strategy.as_deref()),
    }
}

fn run_status() -> Result<()> {
    println!("{}", "Git Status".bold().underline());
    println!();

    let status = get_git_tools().status()?;

    println!("{} {}", "Branch:".dimmed(), status.branch.cyan());

    if status.is_clean {
        println!("{}", "Working tree clean".green());
    } else {
        if !status.staged.is_empty() {
            println!("\n{}", "Staged:".bold());
            for change in &status.staged {
                let icon = match change.status {
                    ChangeStatus::Added => "A".green(),
                    ChangeStatus::Modified => "M".yellow(),
                    ChangeStatus::Deleted => "D".red(),
                    _ => "?".dimmed(),
                };
                println!("  {} {}", icon, change.path.display());
            }
        }
        if !status.unstaged.is_empty() {
            println!("\n{}", "Unstaged:".bold());
            for change in &status.unstaged {
                let icon = match change.status {
                    ChangeStatus::Modified => "M".yellow(),
                    ChangeStatus::Deleted => "D".red(),
                    _ => "?".dimmed(),
                };
                println!("  {} {}", icon, change.path.display());
            }
        }
        if !status.untracked.is_empty() {
            println!("\n{}", "Untracked:".bold());
            for path in &status.untracked {
                println!("  {} {}", "?".dimmed(), path.display());
            }
        }
    }

    Ok(())
}

fn run_diff(path: Option<&str>, staged: bool) -> Result<()> {
    let path_buf = path.map(Path::new);
    let output = get_git_tools().diff(staged, path_buf)?;

    for line in output.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            println!("{}", line.green());
        } else if line.starts_with('-') && !line.starts_with("---") {
            println!("{}", line.red());
        } else if line.starts_with("@@") {
            println!("{}", line.cyan());
        } else {
            println!("{}", line);
        }
    }

    Ok(())
}

fn run_log(count: Option<usize>, _oneline: bool) -> Result<()> {
    let commits = get_git_tools().log(count, None)?;

    for commit in commits {
        println!(
            "{} {} ({})",
            commit.short_hash.yellow(),
            commit.message,
            commit.author.dimmed()
        );
    }

    Ok(())
}

fn run_commit(message: Option<&str>, all: bool) -> Result<()> {
    let msg = message.unwrap_or("AI-generated commit");
    let commit = get_git_tools().commit(msg, all)?;

    println!("{} {}", "✓".green(), "Commit created".bold());
    println!("  {} {}", commit.short_hash.yellow(), commit.message);

    Ok(())
}

fn run_undo(soft: bool) -> Result<()> {
    let output = get_git_tools().undo(soft)?;

    println!("{} {}", "✓".green(), "Last commit undone".bold());
    if !output.is_empty() {
        println!("{}", output.dimmed());
    }

    Ok(())
}

fn run_branch(name: Option<&str>, delete: bool) -> Result<()> {
    match (name, delete) {
        (Some(n), true) => {
            // For delete, we still need direct git command - not in core yet
            std::process::Command::new("git")
                .args(["branch", "-d", n])
                .output()?;
            println!("{} Deleted branch: {}", "✓".green(), n.cyan());
        }
        (Some(n), false) => {
            // Create branch via checkout -b
            get_git_tools().checkout(n, true)?;
            println!("{} Created branch: {}", "✓".green(), n.cyan());
        }
        (None, _) => {
            let branches = get_git_tools().branches(true)?;
            println!("{}", "Branches".bold().underline());
            for branch in branches {
                if branch.is_current {
                    println!("{} {}", "*".green(), branch.name.green());
                } else if branch.is_remote {
                    println!("  {}", branch.name.dimmed());
                } else {
                    println!("  {}", branch.name);
                }
            }
        }
    }
    Ok(())
}

fn run_checkout(target: &str) -> Result<()> {
    let output = get_git_tools().checkout(target, false)?;
    println!("{} Checked out: {}", "✓".green(), target.cyan());
    if !output.is_empty() {
        println!("{}", output.dimmed());
    }
    Ok(())
}

fn run_stash(action: Option<&str>) -> Result<()> {
    match action {
        Some("pop") => {
            get_git_tools().stash(true, None)?;
            println!("{} Stash popped", "✓".green());
        }
        Some("list") | None => {
            // List stashes - not in core yet, use direct command
            let output = std::process::Command::new("git")
                .args(["stash", "list"])
                .output()?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.is_empty() {
                println!("{}", "No stashes".dimmed());
            } else {
                println!("{}", stdout);
            }
        }
        Some("drop") => {
            std::process::Command::new("git")
                .args(["stash", "drop"])
                .output()?;
            println!("{} Stash dropped", "✓".green());
        }
        Some(other) => {
            get_git_tools().stash(false, Some(other))?;
            println!("{} Stash {}", "✓".green(), other);
        }
    }
    Ok(())
}

fn run_blame(path: &str) -> Result<()> {
    let output = get_git_tools().blame(Path::new(path), None)?;
    println!("{}", output);
    Ok(())
}

fn run_conflicts() -> Result<()> {
    // Check for conflicts - not in core yet
    let output = std::process::Command::new("git")
        .args(["diff", "--name-only", "--diff-filter=U"])
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.is_empty() {
        println!("{}", "No merge conflicts".green());
    } else {
        println!("{}", "Files with conflicts:".bold().underline());
        for line in stdout.lines() {
            println!("  {} {}", "⚠".yellow(), line);
        }
    }

    Ok(())
}

fn run_resolve(path: &str, strategy: Option<&str>) -> Result<()> {
    match strategy {
        Some("ours") => {
            std::process::Command::new("git")
                .args(["checkout", "--ours", path])
                .output()?;
            std::process::Command::new("git")
                .args(["add", path])
                .output()?;
            println!("{} Resolved {} using 'ours'", "✓".green(), path.cyan());
        }
        Some("theirs") => {
            std::process::Command::new("git")
                .args(["checkout", "--theirs", path])
                .output()?;
            std::process::Command::new("git")
                .args(["add", path])
                .output()?;
            println!("{} Resolved {} using 'theirs'", "✓".green(), path.cyan());
        }
        _ => {
            println!("{}", "Conflict resolution strategies:".bold());
            println!("  {} - Keep our version", "ours".cyan());
            println!("  {} - Keep their version", "theirs".cyan());
            println!();
            println!(
                "Usage: gestura tools git resolve {} --strategy <ours|theirs>",
                path
            );
        }
    }

    Ok(())
}
