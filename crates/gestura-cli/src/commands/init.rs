//! First-time setup wizard command

use super::Result;
use colored::Colorize;
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use gestura_core::{AppConfig, AppConfigSecurityExt};

pub fn run() -> Result<()> {
    println!();
    println!("{}", "Welcome to Gestura!".bold().cyan());
    println!("Let's set up your voice-first AI assistant.");
    println!();

    // Load existing config or create default
    let mut config = AppConfig::load();

    // LLM Provider selection
    let providers = ["openai", "anthropic", "gemini", "grok", "ollama"];
    let provider_labels = [
        "OpenAI",
        "Anthropic",
        "Gemini (Google)",
        "Grok",
        "Ollama (local)",
    ];

    let current_idx = providers
        .iter()
        .position(|p| *p == config.llm.primary)
        .unwrap_or(0);

    let provider_idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Which LLM provider would you like to use?")
        .items(&provider_labels)
        .default(current_idx)
        .interact()?;

    let provider = providers[provider_idx];
    config.llm.primary = provider.to_string();
    println!();

    // API Key (if not Ollama)
    if provider != "ollama" {
        let env_var = match provider {
            "openai" => "OPENAI_API_KEY",
            "anthropic" => "ANTHROPIC_API_KEY",
            "gemini" => "GESTURA_GEMINI_API_KEY",
            "grok" => "GROK_API_KEY",
            _ => "",
        };

        if std::env::var(env_var).is_ok() {
            println!("{} {} is already set.", "✓".green(), env_var.cyan());
        } else {
            let api_key: String = Input::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "Enter your {} API key (or press Enter to skip)",
                    provider_labels[provider_idx]
                ))
                .allow_empty(true)
                .interact_text()?;

            if !api_key.is_empty() {
                println!();
                println!(
                    "{}",
                    "To persist this key, add to your shell profile:".yellow()
                );
                println!("  export {}=\"{}\"", env_var, api_key);
            }
        }
    } else {
        println!(
            "{}",
            "Make sure Ollama is running at http://localhost:11434".dimmed()
        );
    }
    println!();

    // Voice setup
    let enable_voice = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Enable voice input?")
        .default(true)
        .interact()?;

    if enable_voice {
        config.voice.provider = "local".to_string();

        let whisper_models = ["tiny", "base", "small", "medium", "large"];
        let whisper_labels = [
            "tiny (fastest, ~75MB)",
            "base (recommended, ~140MB)",
            "small (~460MB)",
            "medium (~1.5GB)",
            "large (~3GB)",
        ];

        let model_idx = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Which Whisper model size?")
            .items(&whisper_labels)
            .default(1)
            .interact()?;

        let selected_model = whisper_models[model_idx];

        // Set model path
        if let Some(data_dir) = dirs::data_dir() {
            let model_path = data_dir
                .join("gestura")
                .join("models")
                .join(format!("ggml-{}.bin", selected_model));
            config.voice.local_model_path = Some(model_path.to_string_lossy().to_string());
        }

        println!();
        println!("Selected: {}", whisper_labels[model_idx].cyan());
        println!();
        println!("{}", "Note: You may need to download the model:".dimmed());
        println!(
            "  {}",
            format!("gestura model whisper download {}", selected_model).cyan()
        );
    } else {
        config.voice.provider = "none".to_string();
    }
    println!();

    // Hotkey configuration
    let hotkey = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Hotkey for voice activation")
        .default(config.hotkey_listen.clone())
        .interact_text()?;
    config.hotkey_listen = hotkey;
    println!();

    // Save configuration
    match config.save() {
        Ok(()) => {
            println!("{}", "Setup Complete!".green().bold());
            println!();

            if let Some(config_dir) = dirs::config_dir() {
                println!("Configuration saved to:");
                println!(
                    "  {}",
                    config_dir
                        .join("gestura")
                        .join("config.toml")
                        .display()
                        .to_string()
                        .cyan()
                );
            }
        }
        Err(e) => {
            eprintln!(
                "{}: Failed to save configuration: {}",
                "warning".yellow(),
                e
            );
        }
    }

    println!();
    println!("{}", "Get started:".bold());
    println!("  {} - Interactive agent", "gestura agent".cyan());
    println!("  {} - Voice input", "gestura listen".cyan());
    println!(
        "  {} - Single prompt",
        "gestura exec \"Your prompt\"".cyan()
    );
    println!("  {} - Check system status", "gestura health".cyan());
    println!();

    Ok(())
}
