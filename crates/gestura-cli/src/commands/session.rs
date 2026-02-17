//! Session management command
//!
//! Manages agent sessions (conversation history) stored locally.

use super::Result;
use crate::SessionAction;
use colored::Colorize;
use std::path::PathBuf;

/// Get the sessions directory
fn get_sessions_dir() -> PathBuf {
    dirs::data_dir()
        .map(|p| p.join("gestura").join("sessions"))
        .unwrap_or_else(|| PathBuf::from("sessions"))
}

/// List session files in the sessions directory
fn list_session_files() -> Vec<(String, std::time::SystemTime)> {
    let sessions_dir = get_sessions_dir();
    if !sessions_dir.exists() {
        return Vec::new();
    }

    let mut sessions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata()
                && metadata.is_file()
                && let Some(name) = entry.file_name().to_str()
                && name.ends_with(".json")
            {
                let session_id = name.trim_end_matches(".json").to_string();
                if let Ok(modified) = metadata.modified() {
                    sessions.push((session_id, modified));
                }
            }
        }
    }

    // Sort by modification time (newest first)
    sessions.sort_by(|a, b| b.1.cmp(&a.1));
    sessions
}

pub fn run(action: &SessionAction) -> Result<()> {
    match action {
        SessionAction::List { limit } => {
            println!("{}", "Agent Sessions".bold());
            println!();

            let sessions = list_session_files();

            if sessions.is_empty() {
                println!("  {}", "(no sessions found)".dimmed());
                println!();
                println!("Start a new session with: {}", "gestura agent".cyan());
            } else {
                let display_count = (*limit).min(sessions.len());
                println!(
                    "{:20} {}",
                    "SESSION ID".underline(),
                    "LAST MODIFIED".underline()
                );

                for (session_id, modified) in sessions.iter().take(display_count) {
                    let time_str = if let Ok(duration) = modified.elapsed() {
                        let secs = duration.as_secs();
                        if secs < 60 {
                            "just now".to_string()
                        } else if secs < 3600 {
                            format!("{} min ago", secs / 60)
                        } else if secs < 86400 {
                            format!("{} hours ago", secs / 3600)
                        } else {
                            format!("{} days ago", secs / 86400)
                        }
                    } else {
                        "unknown".to_string()
                    };

                    println!("{:20} {}", session_id.cyan(), time_str.dimmed());
                }

                println!();
                println!("Total: {} session(s)", sessions.len());

                if sessions.len() > display_count {
                    println!("Use {} to see more.", "--limit".cyan());
                }
            }
        }
        SessionAction::Resume { session } => {
            let sessions = list_session_files();

            let session_id = if session == "last" {
                if let Some((id, _)) = sessions.first() {
                    id.clone()
                } else {
                    eprintln!("{}: No sessions found", "error".red());
                    std::process::exit(2);
                }
            } else {
                session.clone()
            };

            let session_file = get_sessions_dir().join(format!("{}.json", session_id));
            if !session_file.exists() {
                eprintln!("{}: Session '{}' not found", "error".red(), session_id);
                std::process::exit(2);
            }

            println!("Resuming session: {}", session_id.cyan());
            println!();
            println!("{}", "To resume in agent mode, run:".dimmed());
            println!(
                "  {}",
                format!("gestura agent --resume --session {}", session_id).cyan()
            );
        }
        SessionAction::Fork { session } => {
            let session_file = get_sessions_dir().join(format!("{}.json", session));
            if !session_file.exists() {
                eprintln!("{}: Session '{}' not found", "error".red(), session);
                std::process::exit(2);
            }

            // Generate new session ID
            let new_id = format!(
                "{}-fork-{}",
                session,
                chrono::Utc::now().format("%Y%m%d%H%M%S")
            );
            let new_file = get_sessions_dir().join(format!("{}.json", new_id));

            // Copy session file
            if let Err(e) = std::fs::copy(&session_file, &new_file) {
                eprintln!("{}: Failed to fork session: {}", "error".red(), e);
                std::process::exit(2);
            }

            println!("{} Forked session: {}", "✓".green(), session.cyan());
            println!("New session ID: {}", new_id.cyan());
        }
        SessionAction::Delete { session } => {
            let session_file = get_sessions_dir().join(format!("{}.json", session));
            if !session_file.exists() {
                eprintln!("{}: Session '{}' not found", "error".red(), session);
                std::process::exit(2);
            }

            if let Err(e) = std::fs::remove_file(&session_file) {
                eprintln!("{}: Failed to delete session: {}", "error".red(), e);
                std::process::exit(2);
            }

            println!("{} Deleted session: {}", "✓".green(), session.cyan());
        }
    }
    Ok(())
}
