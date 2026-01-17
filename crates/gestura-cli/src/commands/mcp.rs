//! MCP server management command

use super::Result;
use crate::McpAction;
use colored::Colorize;
use gestura_core::AppConfig;

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
    }
    Ok(())
}
