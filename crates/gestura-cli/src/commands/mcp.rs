//! MCP server management command
//! Provides CLI commands for MCP protocol inspection and server management

use super::Result;
use crate::McpAction;
use colored::Colorize;
use gestura_core::AppConfig;
use gestura_core::mcp::{PROTOCOL_VERSION, PromptRegistry, SessionManager};

pub fn run(action: &McpAction) -> Result<()> {
    match action {
        McpAction::List => {
            let config = AppConfig::load();

            println!("{}", "MCP (Model Context Protocol) Tools".bold());
            println!();

            if config.mcp_tools.is_empty() {
                println!("  {}", "(no MCP tools configured)".dimmed());
                println!();
                println!(
                    "Add a tool with: {}",
                    "gestura mcp add <name> <endpoint>".cyan()
                );
            } else {
                println!("{:20} {}", "NAME".underline(), "ENDPOINT".underline());
                for tool in &config.mcp_tools {
                    println!("{:20} {}", tool.name.cyan(), tool.endpoint.dimmed());
                }
                println!();
                println!("Total: {} tool(s)", config.mcp_tools.len());
            }
        }
        McpAction::Add { name, command } => {
            let mut config = AppConfig::load();

            // Check if already exists
            if config.mcp_tools.iter().any(|t| t.name == *name) {
                eprintln!("{}: MCP tool '{}' already exists", "error".red(), name);
                eprintln!("Use {} to update it.", "gestura mcp remove".cyan());
                std::process::exit(2);
            }

            // Add new tool (command is used as endpoint)
            config.mcp_tools.push(gestura_core::config::McpTool {
                name: name.clone(),
                endpoint: command.clone(),
            });

            if let Err(e) = config.save() {
                eprintln!("{}: Failed to save config: {}", "error".red(), e);
                std::process::exit(2);
            }

            println!("{} Added MCP tool: {}", "✓".green(), name.cyan());
            println!("Endpoint: {}", command);
        }
        McpAction::Remove { name } => {
            let mut config = AppConfig::load();

            let original_len = config.mcp_tools.len();
            config.mcp_tools.retain(|t| t.name != *name);

            if config.mcp_tools.len() == original_len {
                eprintln!("{}: MCP tool '{}' not found", "error".red(), name);
                std::process::exit(2);
            }

            if let Err(e) = config.save() {
                eprintln!("{}: Failed to save config: {}", "error".red(), e);
                std::process::exit(2);
            }

            println!("{} Removed MCP tool: {}", "✓".green(), name.cyan());
        }
        McpAction::Enable { name } => {
            // In the current McpTool structure, there's no enabled field
            // Just verify the tool exists
            let config = AppConfig::load();
            if config.mcp_tools.iter().any(|t| t.name == *name) {
                println!("{} MCP tool '{}' is active", "✓".green(), name.cyan());
            } else {
                eprintln!("{}: MCP tool '{}' not found", "error".red(), name);
                std::process::exit(2);
            }
        }
        McpAction::Disable { name } => {
            // In the current McpTool structure, there's no enabled field
            // Suggest using remove instead
            let config = AppConfig::load();
            if config.mcp_tools.iter().any(|t| t.name == *name) {
                println!(
                    "{} To disable a tool, use: {}",
                    "Note:".yellow(),
                    format!("gestura mcp remove {}", name).cyan()
                );
            } else {
                eprintln!("{}: MCP tool '{}' not found", "error".red(), name);
                std::process::exit(2);
            }
        }
        McpAction::Status => {
            show_mcp_status();
        }
        McpAction::Prompts => {
            show_mcp_prompts();
        }
        McpAction::Capabilities => {
            show_mcp_capabilities();
        }
    }
    Ok(())
}

/// Display MCP protocol status
fn show_mcp_status() {
    println!("{}", "MCP Protocol Status".bold());
    println!();

    // Protocol info
    println!(
        "  {} {}",
        "Protocol Version:".dimmed(),
        PROTOCOL_VERSION.cyan()
    );
    println!("  {} {}", "Server Name:".dimmed(), "gestura".cyan());
    println!(
        "  {} {}",
        "Server Version:".dimmed(),
        env!("CARGO_PKG_VERSION").cyan()
    );
    println!();

    // Session state (create a temporary session manager to check state)
    let session = SessionManager::new();
    println!("  {} {:?}", "Session State:".dimmed(), session.state());
    println!();

    // Transport info
    println!("{}", "Transports".bold());
    println!("  {} {}", "STDIO:".dimmed(), "✓ Available".green());
    println!("  {} {}", "HTTP/SSE:".dimmed(), "○ Planned".yellow());
    println!();

    // Features
    println!("{}", "Protocol Features".bold());
    println!(
        "  {} {}",
        "Lifecycle:".dimmed(),
        "✓ initialize, ping, shutdown".green()
    );
    println!("  {} {}", "Tools:".dimmed(), "✓ list, call".green());
    println!("  {} {}", "Resources:".dimmed(), "✓ list, read".green());
    println!("  {} {}", "Prompts:".dimmed(), "✓ list, get".green());
    println!(
        "  {} {}",
        "Notifications:".dimmed(),
        "✓ progress, logging, cancelled".green()
    );
}

/// Display available prompts
fn show_mcp_prompts() {
    let registry = PromptRegistry::new();
    let prompts = registry.list();

    println!("{}", "Available MCP Prompts".bold());
    println!();

    if prompts.is_empty() {
        println!("  {}", "(no prompts registered)".dimmed());
    } else {
        for prompt in &prompts {
            println!("  {} {}", "•".cyan(), prompt.name.bold());
            if let Some(desc) = &prompt.description {
                println!("    {}", desc.dimmed());
            }
            if let Some(args) = &prompt.arguments {
                println!("    {}:", "Arguments".underline());
                for arg in args {
                    let required = if arg.required {
                        " (required)".red()
                    } else {
                        "".normal()
                    };
                    println!("      {} {}{}", "-".dimmed(), arg.name, required);
                    if let Some(desc) = &arg.description {
                        println!("        {}", desc.dimmed());
                    }
                }
            }
            println!();
        }
    }
    println!("Total: {} prompt(s)", prompts.len());
}

/// Display server capabilities
fn show_mcp_capabilities() {
    println!("{}", "MCP Server Capabilities".bold());
    println!();

    // Tools capability
    println!("{}", "Tools".bold().underline());
    println!("  {} {}", "list_changed:".dimmed(), "true".green());
    println!();

    // Resources capability
    println!("{}", "Resources".bold().underline());
    println!("  {} {}", "subscribe:".dimmed(), "false".yellow());
    println!("  {} {}", "list_changed:".dimmed(), "true".green());
    println!();

    // Prompts capability
    println!("{}", "Prompts".bold().underline());
    println!("  {} {}", "list_changed:".dimmed(), "true".green());
    println!();

    // Logging capability
    println!("{}", "Logging".bold().underline());
    println!("  {} {}", "enabled:".dimmed(), "true".green());
    println!();

    // Client capabilities (what we can accept)
    println!("{}", "Client Features Supported".bold().underline());
    println!(
        "  {} {}",
        "Sampling:".dimmed(),
        "○ Not implemented".yellow()
    );
    println!("  {} {}", "Roots:".dimmed(), "○ Not implemented".yellow());
    println!(
        "  {} {}",
        "Elicitation:".dimmed(),
        "○ Not implemented".yellow()
    );
}
