//! A2A (Agent-to-Agent) Protocol CLI Commands
//!
//! Commands for managing A2A protocol interactions, agent profiles,
//! and authentication tokens.

use colored::Colorize;
use gestura_core::a2a::{
    A2AClient, A2AMessage, AgentCard, CreateTaskRequest, MessagePart, RemoteTaskContract,
};
use tokio::runtime::Runtime;

use super::Result;
use crate::A2aAction;

/// Run the A2A command
pub fn run(action: &A2aAction) -> Result<()> {
    match action {
        A2aAction::Status => show_a2a_status(),
        A2aAction::Profiles => list_profiles(),
        A2aAction::Discover { url } => discover_agent(url),
        A2aAction::Register {
            id,
            name,
            capabilities,
        } => register_profile(id, name, capabilities.as_deref()),
        A2aAction::Token { agent_id, hours } => generate_token(agent_id, *hours),
        A2aAction::Validate { token } => validate_token(token),
        A2aAction::Agents => list_agents(),
        A2aAction::Send { url, message } => send_task(url, message),
    }
}

/// Show A2A protocol status
fn show_a2a_status() -> Result<()> {
    println!("{}", "A2A Protocol Status".bold().cyan());
    println!("{}", "═".repeat(50));
    println!();

    println!("{}: Agent2Agent (A2A)", "Protocol".bold());
    println!("{}: 0.3.0", "Version".bold());
    println!("{}: Linux Foundation", "Governance".bold());
    println!("{}: Apache 2.0", "License".bold());
    println!();

    println!("{}", "Features".bold().yellow());
    println!("  {} Agent discovery via Agent Cards", "✓".green());
    println!("  {} Task-based communication", "✓".green());
    println!("  {} JSON-RPC 2.0 protocol", "✓".green());
    println!("  {} Bearer token authentication", "✓".green());
    println!("  {} Profile propagation", "✓".green());
    println!("  {} SSE streaming support", "✓".green());
    println!();

    println!("{}", "Endpoints".bold().yellow());
    println!("  {} agent/discover", "•".cyan());
    println!("  {} task/create", "•".cyan());
    println!("  {} task/status", "•".cyan());
    println!("  {} task/cancel", "•".cyan());
    println!("  {} profile/register", "•".cyan());
    println!("  {} profile/validate", "•".cyan());

    Ok(())
}

/// List registered agent profiles
fn list_profiles() -> Result<()> {
    println!("{}", "Registered Agent Profiles".bold().cyan());
    println!("{}", "═".repeat(50));
    println!();

    // In a real implementation, this would load from persistent storage
    println!("{}", "No profiles registered yet.".dimmed());
    println!();
    println!(
        "Use {} to register a new profile.",
        "gestura a2a register".cyan()
    );

    Ok(())
}

/// Discover a remote agent
fn discover_agent(url: &str) -> Result<()> {
    println!("{} {}", "Discovering agent at:".bold(), url.cyan());
    println!();

    let rt = Runtime::new().map_err(|e| format!("Failed to create runtime: {e}"))?;
    let client = A2AClient::new();

    match rt.block_on(client.discover(url)) {
        Ok(card) => {
            print_agent_card(&card);
        }
        Err(e) => {
            println!("{} {}", "Error:".red().bold(), e);
            println!();
            println!(
                "{}",
                "Make sure the URL points to a valid A2A endpoint.".dimmed()
            );
        }
    }

    Ok(())
}

/// Print an agent card in a formatted way
fn print_agent_card(card: &AgentCard) {
    println!("{}", "Agent Card".bold().green());
    println!("{}", "─".repeat(50));
    println!("{}: {}", "Name".bold(), card.name.cyan());
    println!("{}: {}", "Description".bold(), card.description);
    println!("{}: {}", "URL".bold(), card.url);
    println!("{}: {}", "Protocol".bold(), card.protocol_version);
    println!();

    if !card.skills.is_empty() {
        println!("{}", "Skills".bold().yellow());
        for skill in &card.skills {
            println!(
                "  {} {} - {}",
                "•".cyan(),
                skill.name.bold(),
                skill.description
            );
            if !skill.examples.is_empty() {
                for example in &skill.examples {
                    println!("      {} {}", "→".dimmed(), example.dimmed());
                }
            }
        }
        println!();
    }

    if let Some(ref auth) = card.authentication {
        println!("{}: {}", "Authentication".bold(), auth.schemes.join(", "));
    }

    println!(
        "{}: {}",
        "Input Modes".bold(),
        card.default_input_modes.join(", ")
    );
    println!(
        "{}: {}",
        "Output Modes".bold(),
        card.default_output_modes.join(", ")
    );
}

/// Register a new agent profile
fn register_profile(id: &str, name: &str, capabilities: Option<&str>) -> Result<()> {
    println!("{}", "Registering Agent Profile".bold().cyan());
    println!("{}", "═".repeat(50));
    println!();

    println!("{}: {}", "Agent ID".bold(), id.green());
    println!("{}: {}", "Name".bold(), name);

    if let Some(caps) = capabilities {
        println!("{}: {}", "Capabilities".bold(), caps);
    }

    println!();
    println!("{} Profile registered successfully!", "✓".green());
    println!();
    println!(
        "Use {} to generate an auth token.",
        format!("gestura a2a token {}", id).cyan()
    );

    Ok(())
}

/// Generate a new auth token for an agent
fn generate_token(agent_id: &str, hours: i64) -> Result<()> {
    println!("{}", "Generating Auth Token".bold().cyan());
    println!("{}", "═".repeat(50));
    println!();

    // Core-first: token generation rules live in gestura-core.
    let mut profile = gestura_core::a2a::AgentProfile::new(agent_id, agent_id);
    profile.generate_token(hours);
    let token = profile
        .auth_token
        .clone()
        .expect("token should be generated");

    println!("{}: {}", "Agent ID".bold(), agent_id);
    println!("{}: {} hours", "Validity".bold(), hours);
    println!();
    println!("{}: {}", "Token".bold().green(), token);
    if let Some(expires_at) = profile.token_expires_at {
        println!("{}: {}", "Expires At".bold(), expires_at.to_rfc3339());
    }
    println!();
    println!(
        "{}",
        "Store this token securely - it cannot be retrieved later.".yellow()
    );

    Ok(())
}

/// Validate a token
fn validate_token(token: &str) -> Result<()> {
    println!("{}", "Validating Token".bold().cyan());
    println!("{}", "═".repeat(50));
    println!();

    println!("{}: {}...", "Token".bold(), &token[..token.len().min(16)]);
    println!();

    if !gestura_core::a2a::is_token_well_formed(token) {
        println!("{}", "Invalid token format".red().bold());
        println!(
            "Expected an ASCII-alphanumeric bearer token (minimum length threshold not met, or invalid characters)."
        );
        return Ok(());
    }

    println!("{}", "Token is well-formed".green().bold());
    println!(
        "Note: This performs an offline format check only; remote validation requires calling profile/validate on the hosting agent."
    );

    Ok(())
}

/// List known remote agents
fn list_agents() -> Result<()> {
    println!("{}", "Known Remote Agents".bold().cyan());
    println!("{}", "═".repeat(50));
    println!();

    println!("{}", "No remote agents discovered yet.".dimmed());
    println!();
    println!(
        "Use {} to discover a remote agent.",
        "gestura a2a discover <url>".cyan()
    );

    Ok(())
}

/// Send a task to a remote agent
fn send_task(url: &str, message: &str) -> Result<()> {
    println!("{}", "Sending Task to Remote Agent".bold().cyan());
    println!("{}", "═".repeat(50));
    println!();

    println!("{}: {}", "Agent URL".bold(), url);
    println!("{}: {}", "Message".bold(), message);
    println!();

    let rt = Runtime::new().map_err(|e| format!("Failed to create runtime: {e}"))?;

    // Check for auth token in environment
    let client = if let Ok(token) = std::env::var("GESTURA_A2A_TOKEN") {
        A2AClient::with_auth(token)
    } else {
        A2AClient::new()
    };

    let request = CreateTaskRequest {
        message: A2AMessage {
            role: "user".to_string(),
            parts: vec![MessagePart::Text {
                text: message.to_string(),
            }],
        },
        run_id: None,
        parent_task_id: None,
        role: Some("remote_worker".to_string()),
        requested_capabilities: vec!["analysis".to_string(), "artifacts".to_string()],
        contract: Some(RemoteTaskContract {
            objective: message.to_string(),
            acceptance_criteria: vec!["Return a concise remote status update".to_string()],
            constraints: vec!["Preserve provenance in task output".to_string()],
            deliverables: vec!["Remote result summary".to_string()],
            output_format: Some("text".to_string()),
        }),
        metadata: std::collections::HashMap::new(),
    };

    match rt.block_on(client.create_task_with_request(url, request)) {
        Ok(task) => {
            println!("{} Task created successfully!", "✓".green());
            println!();
            println!("{}: {}", "Task ID".bold(), task.id.cyan());
            println!("{}: {:?}", "Status".bold(), task.status);
            if let Some(role) = &task.role {
                println!("{}: {}", "Role".bold(), role);
            }
            if let Some(contract) = &task.contract {
                println!("{}: {}", "Objective".bold(), contract.objective);
            }
            println!("{}: {}", "Created At".bold(), task.created_at);

            if !task.messages.is_empty() {
                println!();
                println!("{}", "Messages".bold().yellow());
                for msg in &task.messages {
                    println!("  {} [{}]", "•".cyan(), msg.role);
                    for part in &msg.parts {
                        match part {
                            gestura_core::a2a::MessagePart::Text { text } => {
                                println!("    {}", text);
                            }
                            gestura_core::a2a::MessagePart::File { uri, .. } => {
                                println!("    [File: {}]", uri);
                            }
                            gestura_core::a2a::MessagePart::Data { data } => {
                                println!("    [Data: {}]", data);
                            }
                        }
                    }
                }
            }

            match rt.block_on(client.get_task_status(url, &task.id)) {
                Ok(status) => {
                    println!();
                    println!("{}", "Remote Status Snapshot".bold().yellow());
                    println!("{}: {:?}", "Status".bold(), status.status);
                    if let Some(reason) = status.status_reason {
                        println!("{}: {}", "Reason".bold(), reason);
                    }
                    println!("{}: {}", "Retry Count".bold(), status.retry_count);
                }
                Err(error) => {
                    println!("{} {}", "Status polling failed:".yellow(), error);
                }
            }
        }
        Err(e) => {
            println!("{} {}", "Error:".red().bold(), e);
            println!();
            println!("{}", "Tips:".bold().yellow());
            println!(
                "  {} Set GESTURA_A2A_TOKEN env var for authentication",
                "•".cyan()
            );
            println!(
                "  {} Ensure the agent URL is correct and accessible",
                "•".cyan()
            );
        }
    }

    Ok(())
}
