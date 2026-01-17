//! Shell command execution tool
//!
//! Provides shell command execution:
//! - run: Execute a shell command
//! - test: Test a command without executing
//! - history: Show command history
//! - last: Show last command output

use super::super::Result;
use colored::Colorize;
use gestura_core::tools::shell::ShellTools;
use std::sync::OnceLock;

/// Global shell tools instance
static SHELL_TOOLS: OnceLock<ShellTools> = OnceLock::new();

fn get_shell_tools() -> &'static ShellTools {
    SHELL_TOOLS.get_or_init(ShellTools::new)
}

/// Shell subcommand options
pub enum ShellSubcommand {
    Run {
        command: String,
        timeout: Option<u64>,
        quiet: bool,
    },
    Test {
        command: String,
    },
    History {
        limit: Option<usize>,
    },
    Last,
}

/// Run shell subcommand
pub fn run(cmd: ShellSubcommand) -> Result<()> {
    match cmd {
        ShellSubcommand::Run {
            command,
            timeout,
            quiet,
        } => run_command(&command, timeout, quiet),
        ShellSubcommand::Test { command } => run_test(&command),
        ShellSubcommand::History { limit } => run_history(limit),
        ShellSubcommand::Last => run_last(),
    }
}

fn run_command(command: &str, timeout_secs: Option<u64>, quiet: bool) -> Result<()> {
    if !quiet {
        println!("{} {}", "Running:".bold(), command.cyan());
        println!("{}", "─".repeat(60).dimmed());
    }

    let result = get_shell_tools().run(command, timeout_secs)?;

    // Print stdout
    if !result.stdout.is_empty() {
        print!("{}", result.stdout);
    }

    // Print stderr
    if !result.stderr.is_empty() {
        eprint!("{}", result.stderr.red());
    }

    if !quiet {
        println!("{}", "─".repeat(60).dimmed());
        let status_icon = if result.success {
            "✓".green()
        } else {
            "✗".red()
        };
        println!(
            "{} Exit code: {} (took {:.2}s)",
            status_icon,
            result.exit_code,
            result.duration_ms as f64 / 1000.0
        );
    }

    if !result.success {
        std::process::exit(result.exit_code);
    }

    Ok(())
}

fn run_test(command: &str) -> Result<()> {
    let result = get_shell_tools().test(command)?;

    println!("{}", "Command Analysis".bold().underline());
    println!();
    println!("  {} {}", "Command:".dimmed(), command.cyan());

    if result.valid {
        println!("  {} {}", "Syntax:".dimmed(), "Valid".green());
    } else {
        println!("  {} {}", "Syntax:".dimmed(), "Invalid".red());
        if let Some(err) = result.error {
            println!("  {} {}", "Error:".dimmed(), err.trim().red());
        }
    }

    println!();
    println!("{}", "Use 'gestura tools shell run' to execute".dimmed());

    Ok(())
}

fn run_history(limit: Option<usize>) -> Result<()> {
    let history = get_shell_tools().history(limit)?;

    println!("{}", "Command History".bold().underline());
    println!();

    if history.is_empty() {
        println!("{}", "(No commands in history)".dimmed());
    } else {
        for entry in history {
            let status_icon = if entry.exit_code == 0 {
                "✓".green()
            } else {
                "✗".red()
            };
            println!(
                "{} {} ({}ms)",
                status_icon,
                entry.command.cyan(),
                entry.duration_ms
            );
        }
    }

    Ok(())
}

fn run_last() -> Result<()> {
    println!("{}", "Last Command Output".bold().underline());
    println!();

    match get_shell_tools().last()? {
        Some(result) => {
            println!("  {} {}", "Command:".dimmed(), result.command.cyan());
            println!("  {} {}", "Exit code:".dimmed(), result.exit_code);
            println!();
            if !result.stdout.is_empty() {
                println!("{}", "stdout:".bold());
                print!("{}", result.stdout);
            }
            if !result.stderr.is_empty() {
                println!("{}", "stderr:".bold());
                eprint!("{}", result.stderr.red());
            }
        }
        None => {
            println!("{}", "(No previous command)".dimmed());
        }
    }

    Ok(())
}
