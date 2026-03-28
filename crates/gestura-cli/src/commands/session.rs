//! Session management command
//!
//! Manages agent sessions (conversation history) stored locally.

use super::Result;
use crate::SessionAction;
use chrono::Utc;
use colored::Colorize;
use gestura_core::agent_sessions::{
    AgentSession, AgentSessionStore, FileAgentSessionStore, SessionFilter, SessionInfo,
};

/// Return the canonical core-backed agent session store.
fn session_store() -> FileAgentSessionStore {
    FileAgentSessionStore::new_default()
}

fn list_sessions_or_exit(store: &FileAgentSessionStore) -> Vec<SessionInfo> {
    match store.list(SessionFilter::All) {
        Ok(sessions) => sessions,
        Err(error) => {
            eprintln!("{}: Failed to list sessions: {error}", "error".red());
            std::process::exit(2);
        }
    }
}

fn load_session_or_exit(store: &FileAgentSessionStore, session_id: &str) -> AgentSession {
    match store.load(session_id) {
        Ok(session) => session,
        Err(_) => {
            eprintln!("{}: Session '{}' not found", "error".red(), session_id);
            std::process::exit(2);
        }
    }
}

fn resolve_resume_session_id(store: &FileAgentSessionStore, requested: &str) -> String {
    if requested == "last" {
        match store.load_last() {
            Ok(Some(session)) => session.id,
            Ok(None) => {
                eprintln!("{}: No sessions found", "error".red());
                std::process::exit(2);
            }
            Err(error) => {
                eprintln!("{}: Failed to load last session: {error}", "error".red());
                std::process::exit(2);
            }
        }
    } else {
        requested.to_string()
    }
}

fn humanize_last_active(last_active: chrono::DateTime<chrono::Utc>) -> String {
    let secs = Utc::now()
        .signed_duration_since(last_active)
        .num_seconds()
        .max(0);

    if secs < 60 {
        "just now".to_string()
    } else if secs < 3_600 {
        format!("{} min ago", secs / 60)
    } else if secs < 86_400 {
        format!("{} hours ago", secs / 3_600)
    } else {
        format!("{} days ago", secs / 86_400)
    }
}

pub fn run(action: &SessionAction) -> Result<()> {
    let store = session_store();

    match action {
        SessionAction::List { limit } => {
            println!("{}", "Agent Sessions".bold());
            println!();

            let sessions = list_sessions_or_exit(&store);

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

                for session in sessions.iter().take(display_count) {
                    let time_str = humanize_last_active(session.last_active);
                    println!("{:20} {}", session.id.cyan(), time_str.dimmed());
                }

                println!();
                println!("Total: {} session(s)", sessions.len());

                if sessions.len() > display_count {
                    println!("Use {} to see more.", "--limit".cyan());
                }
            }
        }
        SessionAction::Resume { session } => {
            let session_id = resolve_resume_session_id(&store, session);
            load_session_or_exit(&store, &session_id);

            println!("Resuming session: {}", session_id.cyan());
            println!();
            println!("{}", "To resume in agent mode, run:".dimmed());
            println!(
                "  {}",
                format!("gestura agent --resume --session {}", session_id).cyan()
            );
        }
        SessionAction::Fork { session } => {
            let session = load_session_or_exit(&store, session);
            let forked = session.fork();

            if let Err(error) = store.save(&forked) {
                eprintln!("{}: Failed to fork session: {error}", "error".red());
                std::process::exit(2);
            }

            println!("{} Forked session: {}", "✓".green(), session.id.cyan());
            println!("New session ID: {}", forked.id.cyan());
        }
        SessionAction::Delete { session } => match store.delete(session) {
            Ok(true) => {
                println!("{} Deleted session: {}", "✓".green(), session.cyan());
            }
            Ok(false) => {
                eprintln!("{}: Session '{}' not found", "error".red(), session);
                std::process::exit(2);
            }
            Err(error) => {
                eprintln!("{}: Failed to delete session: {error}", "error".red());
                std::process::exit(2);
            }
        },
    }
    Ok(())
}
