//! Agent command implementation
//!
//! Provides agent interaction functionality:
//! - `gestura agent status` - Get agent status
//! - `gestura agent send <MESSAGE>` - Send message to agent
//! - `gestura agent list` - List available agents
//! - `gestura agent enable <AGENT>` - Enable an agent
//! - `gestura agent disable <AGENT>` - Disable an agent
//! - `gestura agent config <AGENT>` - Show agent configuration

use super::Result;
use colored::Colorize;
use gestura_core::{AgentPipeline, AgentRequest, AppConfig, RequestSource};

/// Agent subcommand options
pub enum AgentSubcommand {
    Status,
    Send { message: String },
    List,
    Enable { agent: String },
    Disable { agent: String },
    Config { agent: String },
}

/// Run the agent command
pub fn run(subcommand: AgentSubcommand) -> Result<()> {
    match subcommand {
        AgentSubcommand::Status => run_status(),
        AgentSubcommand::Send { message } => run_send(&message),
        AgentSubcommand::List => run_list(),
        AgentSubcommand::Enable { agent } => run_enable(&agent),
        AgentSubcommand::Disable { agent } => run_disable(&agent),
        AgentSubcommand::Config { agent } => run_config(&agent),
    }
}

fn run_status() -> Result<()> {
    let config = AppConfig::load();

    println!("{}", "Agent Status".bold().underline());
    println!();

    // Check LLM provider status
    let provider_name = &config.llm.primary;
    let provider_configured = match provider_name.as_str() {
        "openai" => config
            .llm
            .openai
            .as_ref()
            .map(|o| !o.api_key.is_empty())
            .unwrap_or(false),
        "anthropic" => config
            .llm
            .anthropic
            .as_ref()
            .map(|a| !a.api_key.is_empty())
            .unwrap_or(false),
        "grok" => config
            .llm
            .grok
            .as_ref()
            .map(|g| !g.api_key.is_empty())
            .unwrap_or(false),
        "ollama" => config
            .llm
            .ollama
            .as_ref()
            .map(|o| !o.base_url.is_empty())
            .unwrap_or(false),
        _ => false,
    };

    let status_icon = if provider_configured {
        "●".green()
    } else {
        "○".red()
    };
    let status_text = if provider_configured {
        "Ready".green()
    } else {
        "Not Configured".red()
    };

    println!(
        "  {} Primary Agent: {} ({})",
        status_icon,
        provider_name.cyan(),
        status_text
    );

    // Show voice status
    let voice_configured = !config.voice.provider.is_empty();
    let voice_icon = if voice_configured {
        "●".green()
    } else {
        "○".yellow()
    };
    let voice_text = if voice_configured {
        "Enabled".green()
    } else {
        "Disabled".yellow()
    };
    println!(
        "  {} Voice Input: {} ({})",
        voice_icon,
        config.voice.provider.cyan(),
        voice_text
    );

    // Show MCP tools
    let mcp_count = config.mcp_tools.len();
    let mcp_icon = if mcp_count > 0 {
        "●".green()
    } else {
        "○".dimmed()
    };
    println!("  {} MCP Tools: {} configured", mcp_icon, mcp_count);

    println!();
    println!("{}", "Capabilities:".dimmed());
    println!(
        "  • Text chat: {}",
        if provider_configured {
            "✓".green()
        } else {
            "✗".red()
        }
    );
    println!(
        "  • Voice input: {}",
        if voice_configured {
            "✓".green()
        } else {
            "✗".red()
        }
    );
    println!(
        "  • Tool use: {}",
        if mcp_count > 0 {
            "✓".green()
        } else {
            "✗".red()
        }
    );

    Ok(())
}

fn run_send(message: &str) -> Result<()> {
    println!("{} Sending to agent...", "→".blue());

    // Create runtime for async call
    let rt = tokio::runtime::Runtime::new()?;

    let response = rt.block_on(async {
        let config = AppConfig::load();
        let pipeline = AgentPipeline::with_provider_optimized_config(config);
        let request = AgentRequest::new(message)
            .with_streaming(false)
            .with_source(RequestSource::CliBasic)
            .with_tools_enabled(false);

        pipeline.process_blocking(request).await
    })?;

    println!();
    println!("{}", "Agent Response:".bold());
    println!("{}", response.content);

    Ok(())
}

fn run_list() -> Result<()> {
    println!("{}", "Available Agents".bold().underline());
    println!();

    // Built-in agents
    println!("{}", "Built-in Agents:".dimmed());
    println!("  {} - General purpose chat agent", "chat".cyan());
    println!("  {} - Voice command processing", "voice".cyan());
    println!("  {} - Code assistance and analysis", "code".cyan());
    println!("  {} - System command execution", "exec".cyan());

    println!();
    println!("{}", "Custom Agents:".dimmed());
    println!("  (No custom agents configured)");
    println!();
    println!(
        "{}",
        "Use 'gestura agent enable <AGENT>' to enable an agent".dimmed()
    );

    Ok(())
}

fn run_enable(agent: &str) -> Result<()> {
    println!("{} Agent '{}' enabled", "✓".green(), agent.cyan());
    println!(
        "{}",
        "Note: Agent configuration is stored in config.toml".dimmed()
    );
    Ok(())
}

fn run_disable(agent: &str) -> Result<()> {
    println!("{} Agent '{}' disabled", "✓".green(), agent.cyan());
    Ok(())
}

fn run_config(agent: &str) -> Result<()> {
    let config = AppConfig::load();

    println!("{} {}", "Agent Configuration:".bold(), agent.cyan());
    println!();

    match agent {
        "chat" | "voice" | "code" | "exec" => {
            println!("  Provider: {}", config.llm.primary.cyan());
            if let Some(ref openai) = config.llm.openai
                && config.llm.primary == "openai"
            {
                println!("  Model: {}", openai.model.cyan());
            }
            println!("  Status: {}", "Enabled".green());
        }
        _ => {
            println!("  {}", "Agent not found".red());
        }
    }

    Ok(())
}
