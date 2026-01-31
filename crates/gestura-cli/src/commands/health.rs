//! System health and diagnostics command

use super::Result;
use colored::Colorize;
use gestura_core::{AppConfig, is_microphone_available, list_audio_input_devices};
use std::path::PathBuf;

/// Get the config file path
fn get_config_path() -> PathBuf {
    // Keep CLI health diagnostics consistent with gestura-core.
    AppConfig::default_path()
}

pub fn run() -> Result<()> {
    println!("{}", "Gestura System Health".bold());
    println!();

    let config = AppConfig::load();
    let mut issues: Vec<String> = Vec::new();

    // Version info
    println!("{}", "Version".underline());
    println!("  CLI Version:  {}", gestura_core::VERSION);
    println!("  Core Version: {}", gestura_core::VERSION);
    println!();

    // Configuration
    println!("{}", "Configuration".underline());
    let config_path = get_config_path();
    if config_path.exists() {
        println!("  Config File:  {} {}", config_path.display(), "✓".green());
    } else {
        println!(
            "  Config File:  {} {}",
            config_path.display(),
            "(using defaults)".yellow()
        );
    }
    println!("  LLM Provider: {}", config.llm.primary.cyan());
    println!("  Voice:        {}", config.voice.provider.cyan());
    println!();

    // LLM Providers
    println!("{}", "LLM Providers".underline());
    let openai_ok = check_provider(
        "OpenAI",
        "OPENAI_API_KEY",
        config
            .llm
            .openai
            .as_ref()
            .map(|o| !o.api_key.is_empty())
            .unwrap_or(false),
    );
    let anthropic_ok = check_provider(
        "Anthropic",
        "ANTHROPIC_API_KEY",
        config
            .llm
            .anthropic
            .as_ref()
            .map(|a| !a.api_key.is_empty())
            .unwrap_or(false),
    );
    let grok_ok = check_provider(
        "Grok",
        "GROK_API_KEY",
        config
            .llm
            .grok
            .as_ref()
            .map(|g| !g.api_key.is_empty())
            .unwrap_or(false),
    );

    // Check Ollama connectivity
    let ollama_url = config
        .llm
        .ollama
        .as_ref()
        .map(|o| o.base_url.clone())
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    print!("  Ollama:       ");
    // Simple check - just report configured URL
    println!("{} ({})", "configured".dimmed(), ollama_url.dimmed());
    println!();

    // Check if primary provider is configured
    let primary_ok = match config.llm.primary.as_str() {
        "openai" => openai_ok,
        "anthropic" => anthropic_ok,
        "grok" => grok_ok,
        "ollama" => true, // Assume Ollama is available locally
        _ => false,
    };
    if !primary_ok {
        issues.push(format!(
            "Primary LLM provider '{}' is not configured",
            config.llm.primary
        ));
    }

    // Voice
    println!("{}", "Voice Processing".underline());
    #[cfg(feature = "voice-local")]
    {
        println!("  Local Whisper: enabled {}", "✓".green());
        // Check if model exists
        if let Some(model_path) = &config.voice.local_model_path {
            let path = PathBuf::from(model_path);
            if path.exists() {
                println!(
                    "  Model File:    {} {}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    "✓".green()
                );
            } else {
                println!("  Model File:    {} {}", model_path, "(not found)".yellow());
                issues.push(format!("Whisper model not found at '{}'", model_path));
            }
        } else {
            println!("  Model File:    {}", "(not configured)".yellow());
        }
    }
    #[cfg(not(feature = "voice-local"))]
    println!("  Local Whisper: {} {}", "disabled", "○".dimmed());
    println!();

    // Audio
    println!("{}", "Audio".underline());
    if is_microphone_available() {
        println!("  Microphone:   available {}", "✓".green());
        let devices = list_audio_input_devices();
        if !devices.is_empty() {
            println!("  Input Devices: {}", devices.len());
            for device in devices.iter().take(3) {
                let marker = if device.is_default { " (default)" } else { "" };
                println!("    • {}{}", device.name, marker.dimmed());
            }
            if devices.len() > 3 {
                println!(
                    "    {} more...",
                    format!("+ {}", devices.len() - 3).dimmed()
                );
            }
        } else {
            println!("  Input Devices: {}", "none found".yellow());
        }
    } else {
        println!("  Microphone:   not available {}", "✗".red());
        issues.push("No microphone available".to_string());
    }
    println!();

    // MCP
    println!("{}", "MCP Integration".underline());
    if !config.mcp_tools.is_empty() {
        println!("  Tools:        {} configured", config.mcp_tools.len());
        for tool in config.mcp_tools.iter().take(3) {
            println!("    • {}", tool.name);
        }
    } else {
        println!("  Tools:        {}", "none configured".dimmed());
    }
    println!();

    // Overall status
    if issues.is_empty() {
        println!("{}", "Status: Ready ✓".green().bold());
    } else {
        println!("{}", "Status: Issues Found".yellow().bold());
        println!();
        for issue in &issues {
            println!("  {} {}", "!".yellow(), issue);
        }
    }

    Ok(())
}

fn check_provider(name: &str, env_var: &str, config_has_key: bool) -> bool {
    let env_ok = std::env::var(env_var).is_ok();
    let configured = env_ok || config_has_key;

    if configured {
        let source = if env_ok { "(env)" } else { "(config)" };
        println!(
            "  {:11} configured {} {}",
            format!("{}:", name),
            "✓".green(),
            source.dimmed()
        );
    } else {
        println!(
            "  {:11} not configured {}",
            format!("{}:", name),
            "○".dimmed()
        );
    }
    configured
}
