//! Configuration management command

use super::Result;
use crate::ConfigAction;
use colored::Colorize;
use gestura_core::AppConfig;
use gestura_core::AppConfigSecurityExt;
use gestura_core::config_env::{is_secret_key, redact_secret};
use std::path::PathBuf;

/// Get the config file path
fn get_config_path() -> PathBuf {
    // Keep CLI path reporting consistent with gestura-core.
    // Config is persisted at `~/.gestura/config.yaml` (with legacy JSON migration).
    AppConfig::default_path()
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
                let display_value = if is_secret_key(key) {
                    redact_secret(value)
                } else {
                    value.to_string()
                };
                println!("{} {} = {}", "Set".green(), key.cyan(), display_value);
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
                "MCP Servers",
                &[("count", &config.mcp_servers.len().to_string())],
            );
            print_config_section("UI", &[("theme", &config.ui.theme_mode)]);
            print_config_section(
                "Pipeline / Context Management",
                &[
                    (
                        "max_history_messages",
                        &config.pipeline.max_history_messages.to_string(),
                    ),
                    (
                        "auto_compact_threshold",
                        &format!("{}%", config.pipeline.auto_compact_threshold_percent),
                    ),
                    (
                        "compaction_strategy",
                        &format!("{:?}", config.pipeline.compaction_strategy),
                    ),
                    (
                        "max_context_tokens",
                        &if config.pipeline.max_context_tokens == 0 {
                            "auto (provider default)".to_string()
                        } else {
                            config.pipeline.max_context_tokens.to_string()
                        },
                    ),
                    (
                        "log_token_usage",
                        &config.pipeline.log_token_usage.to_string(),
                    ),
                    (
                        "agent_telemetry.enabled",
                        &config.pipeline.agent_telemetry.enabled.to_string(),
                    ),
                    (
                        "agent_telemetry.trace_export.enabled",
                        &config
                            .pipeline
                            .agent_telemetry
                            .trace_export
                            .enabled
                            .to_string(),
                    ),
                    (
                        "agent_telemetry.trace_export.protocol",
                        config
                            .pipeline
                            .agent_telemetry
                            .trace_export
                            .protocol
                            .as_str(),
                    ),
                    (
                        "agent_telemetry.trace_export.endpoint",
                        &config.pipeline.agent_telemetry.trace_export.endpoint,
                    ),
                ],
            );

            // API key status from OS keychain
            let key_status = AppConfig::api_key_keychain_status();
            let key_items: Vec<(&str, String)> = key_status
                .iter()
                .map(|(provider, present)| {
                    let status = if *present {
                        "✓ stored".green().to_string()
                    } else {
                        "✗ not found".red().to_string()
                    };
                    (*provider, status)
                })
                .collect();
            let key_refs: Vec<(&str, &str)> =
                key_items.iter().map(|(p, s)| (*p, s.as_str())).collect();
            print_config_section("API Keys (Keychain)", &key_refs);

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
    config.get(key).or_else(|| match key {
        "llm.openai.api_key" => config
            .llm
            .openai
            .as_ref()
            .map(|c| redact_secret(&c.api_key)),
        "llm.anthropic.api_key" => config
            .llm
            .anthropic
            .as_ref()
            .map(|c| redact_secret(&c.api_key)),
        "llm.grok.api_key" => config.llm.grok.as_ref().map(|c| redact_secret(&c.api_key)),
        "voice.openai_api_key" => config
            .voice
            .openai_api_key
            .as_ref()
            .map(|k| redact_secret(k)),
        "web_search.serpapi_key" => config
            .web_search
            .serpapi_key
            .as_ref()
            .map(|k| redact_secret(k)),
        "web_search.brave_key" => config
            .web_search
            .brave_key
            .as_ref()
            .map(|k| redact_secret(k)),
        _ => None,
    })
}

fn set_config_value(config: &mut AppConfig, key: &str, value: &str) -> bool {
    config.set(key, value)
}
