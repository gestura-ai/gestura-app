//! Configuration management command

use super::Result;
use crate::ConfigAction;
use colored::Colorize;
use gestura_core::AppConfig;
use std::path::PathBuf;

/// Get the config file path
fn get_config_path() -> PathBuf {
    dirs::config_dir()
        .map(|p| p.join("gestura").join("config.json"))
        .unwrap_or_else(|| PathBuf::from("config.json"))
}

pub fn run(action: &ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Get { key } => {
            let config = AppConfig::load();
            let value = get_config_value(&config, key);
            match value {
                Some(v) => println!("{}", v),
                None => {
                    eprintln!("{}: Unknown config key: {}", "error".red(), key);
                    std::process::exit(2);
                }
            }
        }
        ConfigAction::Set { key, value } => {
            let mut config = AppConfig::load();
            if set_config_value(&mut config, key, value) {
                if let Err(e) = config.save() {
                    eprintln!("{}: Failed to save config: {}", "error".red(), e);
                    std::process::exit(2);
                }
                println!("{} {} = {}", "Set".green(), key.cyan(), value);
            } else {
                eprintln!(
                    "{}: Unknown or read-only config key: {}",
                    "error".red(),
                    key
                );
                std::process::exit(2);
            }
        }
        ConfigAction::List => {
            let config = AppConfig::load();
            println!("{}", "Current Configuration".bold());
            println!();
            print_config_section("LLM", &[("primary", &config.llm.primary)]);
            print_config_section(
                "Voice",
                &[
                    ("provider", &config.voice.provider),
                    (
                        "local_model",
                        config
                            .voice
                            .local_model_path
                            .as_deref()
                            .unwrap_or("(not set)"),
                    ),
                ],
            );
            print_config_section(
                "MCP Tools",
                &[("count", &config.mcp_tools.len().to_string())],
            );
            print_config_section("UI", &[("theme", &config.ui.theme_mode)]);
            println!();
            println!(
                "Config file: {}",
                get_config_path().display().to_string().dimmed()
            );
        }
        ConfigAction::Edit => {
            let config_path = get_config_path();

            if !config_path.exists() {
                // Create default config file if it doesn't exist
                let config = AppConfig::default();
                if let Err(e) = config.save() {
                    eprintln!("{}: Failed to create config file: {}", "error".red(), e);
                    std::process::exit(2);
                }
                println!(
                    "{} Created default config at {}",
                    "✓".green(),
                    config_path.display()
                );
            }

            let editor = std::env::var("EDITOR")
                .or_else(|_| std::env::var("VISUAL"))
                .unwrap_or_else(|_| {
                    if cfg!(target_os = "macos") {
                        "open -e".to_string()
                    } else {
                        "nano".to_string()
                    }
                });

            println!("Opening {} in {}", config_path.display(), editor.cyan());

            let status = if editor.contains(' ') {
                let parts: Vec<&str> = editor.split_whitespace().collect();
                std::process::Command::new(parts[0])
                    .args(&parts[1..])
                    .arg(&config_path)
                    .status()?
            } else {
                std::process::Command::new(&editor)
                    .arg(&config_path)
                    .status()?
            };

            if !status.success() {
                eprintln!("{}: Editor exited with error", "warning".yellow());
            }
        }
        ConfigAction::Reset => {
            println!("{}", "Resetting configuration to defaults...".yellow());
            let config = AppConfig::default();
            if let Err(e) = config.save() {
                eprintln!("{}: Failed to save config: {}", "error".red(), e);
                std::process::exit(2);
            }
            println!("{} Configuration reset to defaults", "✓".green());
        }
    }

    Ok(())
}

fn print_config_section(name: &str, items: &[(&str, &str)]) {
    println!("  {}", name.underline());
    for (key, value) in items {
        println!("    {}: {}", key.cyan(), value);
    }
    println!();
}

fn get_config_value(config: &AppConfig, key: &str) -> Option<String> {
    match key {
        "llm.primary" => Some(config.llm.primary.clone()),
        "voice.provider" => Some(config.voice.provider.clone()),
        "voice.local_model_path" => Some(config.voice.local_model_path.clone().unwrap_or_default()),
        "voice.audio_device" => Some(config.voice.audio_device.clone().unwrap_or_default()),
        "ui.theme_mode" => Some(config.ui.theme_mode.clone()),
        "hotkey_listen" => Some(config.hotkey_listen.clone()),
        "nats_url" => Some(config.nats_url.clone()),
        _ => None,
    }
}

fn set_config_value(config: &mut AppConfig, key: &str, value: &str) -> bool {
    match key {
        "llm.primary" => {
            config.llm.primary = value.to_string();
            true
        }
        "voice.provider" => {
            config.voice.provider = value.to_string();
            true
        }
        "voice.local_model_path" => {
            config.voice.local_model_path = Some(value.to_string());
            true
        }
        "voice.audio_device" => {
            config.voice.audio_device = Some(value.to_string());
            true
        }
        "ui.theme_mode" => {
            config.ui.theme_mode = value.to_string();
            true
        }
        "hotkey_listen" => {
            config.hotkey_listen = value.to_string();
            true
        }
        "nats_url" => {
            config.nats_url = value.to_string();
            true
        }
        _ => false,
    }
}
