//! MCP server management command (Claude Code compatible)
//!
//! Provides CLI commands for inspecting and managing MCP servers. The surface
//! mirrors `claude mcp …` so users can replace `claude` → `gestura` in scripts.

use super::Result;
use crate::McpAction;
use colored::Colorize;
use gestura_core::AppConfig;
use gestura_core::config::{McpScope, McpServerEntry, McpTransportType};
use gestura_core::mcp::{PROTOCOL_VERSION, PromptRegistry, SessionManager};
use std::collections::HashMap;

pub fn run(action: &McpAction) -> Result<()> {
    match action {
        // ── list ────────────────────────────────────────────────────────
        McpAction::List => {
            let config = AppConfig::load();

            println!("{}", "MCP Servers".bold());
            println!();

            if config.mcp_servers.is_empty() {
                println!("  {}", "(no MCP servers configured)".dimmed());
                println!();
                println!(
                    "Add a server with: {}",
                    "gestura mcp add <name> <command_or_url>".cyan()
                );
            } else {
                println!(
                    "{:20} {:8} {:8} {}",
                    "NAME".underline(),
                    "TYPE".underline(),
                    "SCOPE".underline(),
                    "ENDPOINT / COMMAND".underline()
                );
                for srv in &config.mcp_servers {
                    let status = if srv.enabled { "✓" } else { "○" };
                    let display = match srv.transport {
                        McpTransportType::Stdio => {
                            let cmd = srv.command.as_deref().unwrap_or("");
                            let args = srv.args.join(" ");
                            format!("{} {}", cmd, args).trim().to_string()
                        }
                        _ => srv.url.clone().unwrap_or_default(),
                    };
                    println!(
                        "{} {:18} {:8} {:8} {}",
                        status,
                        srv.name.cyan(),
                        format!("{}", srv.transport).dimmed(),
                        format!("{}", srv.scope).dimmed(),
                        display.dimmed()
                    );
                }
                println!();
                println!("Total: {} server(s)", config.mcp_servers.len());
            }
        }

        // ── add ─────────────────────────────────────────────────────────
        McpAction::Add {
            name,
            command_or_url,
            transport,
            scope,
            env,
            header,
            args,
        } => {
            let mut config = AppConfig::load();

            if config.mcp_servers.iter().any(|t| t.name == *name) {
                eprintln!("{}: MCP server '{}' already exists", "error".red(), name);
                eprintln!("Use {} first.", "gestura mcp remove".cyan());
                std::process::exit(2);
            }

            let transport_type: McpTransportType = transport.parse().unwrap_or_else(|e: String| {
                eprintln!("{}: {}", "error".red(), e);
                std::process::exit(2);
            });
            let scope_val: McpScope = scope.parse().unwrap_or_else(|e: String| {
                eprintln!("{}: {}", "error".red(), e);
                std::process::exit(2);
            });

            let env_map: HashMap<String, String> = env
                .iter()
                .filter_map(|kv| {
                    kv.split_once('=')
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                })
                .collect();

            let headers_map: HashMap<String, String> = header
                .iter()
                .filter_map(|h| {
                    h.split_once(':')
                        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                })
                .collect();

            let entry = match transport_type {
                McpTransportType::Stdio => McpServerEntry {
                    name: name.clone(),
                    transport: transport_type,
                    enabled: true,
                    command: Some(command_or_url.clone()),
                    args: args.clone(),
                    env: env_map,
                    scope: scope_val,
                    ..McpServerEntry::default()
                },
                McpTransportType::Http | McpTransportType::Sse => McpServerEntry {
                    name: name.clone(),
                    transport: transport_type,
                    enabled: true,
                    url: Some(command_or_url.clone()),
                    headers: headers_map,
                    scope: scope_val,
                    ..McpServerEntry::default()
                },
            };

            config.mcp_servers.push(entry);

            if let Err(e) = config.save() {
                eprintln!("{}: Failed to save config: {}", "error".red(), e);
                std::process::exit(2);
            }

            println!(
                "{} Added MCP server: {} ({})",
                "✓".green(),
                name.cyan(),
                transport
            );
        }

        // ── add-json ────────────────────────────────────────────────────
        McpAction::AddJson { name, json } => {
            let mut config = AppConfig::load();

            if config.mcp_servers.iter().any(|t| t.name == *name) {
                eprintln!("{}: MCP server '{}' already exists", "error".red(), name);
                std::process::exit(2);
            }

            let mut entry: McpServerEntry = serde_json::from_str(json).unwrap_or_else(|e| {
                eprintln!("{}: Invalid JSON: {}", "error".red(), e);
                std::process::exit(2);
            });
            entry.name = name.clone();

            config.mcp_servers.push(entry);

            if let Err(e) = config.save() {
                eprintln!("{}: Failed to save config: {}", "error".red(), e);
                std::process::exit(2);
            }

            println!(
                "{} Added MCP server from JSON: {}",
                "✓".green(),
                name.cyan()
            );
        }

        // ── get ─────────────────────────────────────────────────────────
        McpAction::Get { name } => {
            let config = AppConfig::load();
            match config.mcp_servers.iter().find(|t| t.name == *name) {
                Some(srv) => {
                    println!("{}", srv.name.bold());
                    println!(
                        "  {} {}",
                        "Transport:".dimmed(),
                        format!("{}", srv.transport).cyan()
                    );
                    println!(
                        "  {} {}",
                        "Enabled:".dimmed(),
                        if srv.enabled {
                            "yes".green()
                        } else {
                            "no".red()
                        }
                    );
                    println!("  {} {}", "Scope:".dimmed(), srv.scope);
                    println!("  {} {}s", "Timeout:".dimmed(), srv.timeout_secs);
                    println!("  {} {}", "Auto-reconnect:".dimmed(), srv.auto_reconnect);
                    match srv.transport {
                        McpTransportType::Stdio => {
                            println!(
                                "  {} {}",
                                "Command:".dimmed(),
                                srv.command.as_deref().unwrap_or("(none)")
                            );
                            if !srv.args.is_empty() {
                                println!("  {} {:?}", "Args:".dimmed(), srv.args);
                            }
                            if !srv.env.is_empty() {
                                println!("  {}", "Env:".dimmed());
                                for (k, v) in &srv.env {
                                    println!("    {}={}", k, v);
                                }
                            }
                        }
                        _ => {
                            println!(
                                "  {} {}",
                                "URL:".dimmed(),
                                srv.url.as_deref().unwrap_or("(none)")
                            );
                            if !srv.headers.is_empty() {
                                println!("  {}", "Headers:".dimmed());
                                for (k, v) in &srv.headers {
                                    println!("    {}: {}", k, v);
                                }
                            }
                        }
                    }
                }
                None => {
                    eprintln!("{}: MCP server '{}' not found", "error".red(), name);
                    std::process::exit(2);
                }
            }
        }

        // ── remove ──────────────────────────────────────────────────────
        McpAction::Remove { name } => {
            let mut config = AppConfig::load();
            let original_len = config.mcp_servers.len();
            config.mcp_servers.retain(|t| t.name != *name);

            if config.mcp_servers.len() == original_len {
                eprintln!("{}: MCP server '{}' not found", "error".red(), name);
                std::process::exit(2);
            }

            if let Err(e) = config.save() {
                eprintln!("{}: Failed to save config: {}", "error".red(), e);
                std::process::exit(2);
            }

            println!("{} Removed MCP server: {}", "✓".green(), name.cyan());
        }

        // ── enable ──────────────────────────────────────────────────────
        McpAction::Enable { name } => {
            let mut config = AppConfig::load();
            match config.mcp_servers.iter_mut().find(|t| t.name == *name) {
                Some(srv) => {
                    srv.enabled = true;
                    if let Err(e) = config.save() {
                        eprintln!("{}: Failed to save config: {}", "error".red(), e);
                        std::process::exit(2);
                    }
                    println!("{} Enabled MCP server: {}", "✓".green(), name.cyan());
                }
                None => {
                    eprintln!("{}: MCP server '{}' not found", "error".red(), name);
                    std::process::exit(2);
                }
            }
        }

        // ── disable ─────────────────────────────────────────────────────
        McpAction::Disable { name } => {
            let mut config = AppConfig::load();
            match config.mcp_servers.iter_mut().find(|t| t.name == *name) {
                Some(srv) => {
                    srv.enabled = false;
                    if let Err(e) = config.save() {
                        eprintln!("{}: Failed to save config: {}", "error".red(), e);
                        std::process::exit(2);
                    }
                    println!("{} Disabled MCP server: {}", "✓".green(), name.cyan());
                }
                None => {
                    eprintln!("{}: MCP server '{}' not found", "error".red(), name);
                    std::process::exit(2);
                }
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
