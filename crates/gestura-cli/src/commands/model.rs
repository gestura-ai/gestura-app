//! Model management command

use super::Result;
use crate::{ModelAction, WhisperAction};
use colored::Colorize;
use gestura_core::{AgentPipeline, AgentRequest, AppConfig, AppConfigSecurityExt, RequestSource};
use std::path::PathBuf;

/// Get the models directory
fn get_models_dir() -> PathBuf {
    dirs::data_dir()
        .map(|p| p.join("gestura").join("models"))
        .unwrap_or_else(|| PathBuf::from("models"))
}

/// Check if a whisper model is downloaded
fn is_model_downloaded(model: &str) -> bool {
    let models_dir = get_models_dir();
    let model_file = models_dir.join(format!("ggml-{}.bin", model));
    model_file.exists()
}

/// Get model size info
fn get_model_info(model: &str) -> (&'static str, &'static str) {
    match model {
        "tiny" | "tiny.en" => ("~75MB", "Fastest, least accurate"),
        "base" | "base.en" => ("~140MB", "Good balance of speed/accuracy"),
        "small" | "small.en" => ("~460MB", "Better accuracy"),
        "medium" | "medium.en" => ("~1.5GB", "High accuracy"),
        "large" | "large-v2" | "large-v3" => ("~3GB", "Highest accuracy"),
        _ => ("unknown", "Unknown model"),
    }
}

pub fn run(action: &ModelAction) -> Result<()> {
    match action {
        ModelAction::Whisper { action } => run_whisper(action),
        ModelAction::Test { provider } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let mut config = AppConfig::load();

                // Apply optional CLI provider override in core (thin CLI)
                let _effective = gestura_core::llm_overrides::apply_cli_provider_arg_override(
                    &mut config,
                    provider.as_deref(),
                );

                let provider_name = config.llm.primary.clone();

                println!("Testing LLM connection to: {}", provider_name.cyan());
                println!();

                let spinner = super::spinner::brand_spinner("Connecting...");

                spinner.set_message("Sending test message...");

                let pipeline = AgentPipeline::with_provider_optimized_config(config);
                let request = AgentRequest::new("Say 'OK' and nothing else.")
                    .with_streaming(false)
                    .with_source(RequestSource::CliBasic)
                    .with_tools_enabled(false);

                match pipeline.process_blocking(request).await {
                    Ok(response) => {
                        spinner.finish_and_clear();
                        println!("{} Connection successful!", "✓".green());
                        println!();
                        println!("Provider: {}", provider_name.cyan());
                        println!("Response: {}", response.content.trim().dimmed());
                    }
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("{} Connection failed: {}", "✗".red(), e);
                        std::process::exit(2);
                    }
                }

                Ok::<(), Box<dyn std::error::Error>>(())
            })?;
            Ok(())
        }
        ModelAction::List { provider } => run_list_models(provider.as_deref()),
    }
}

/// List available LLM models for one or all configured providers.
fn run_list_models(provider_filter: Option<&str>) -> Result<()> {
    let config = AppConfig::load();
    let rt = tokio::runtime::Runtime::new()?;

    let providers: Vec<&str> = if let Some(p) = provider_filter {
        vec![p]
    } else {
        // List only providers that have a usable API key configured
        // (ollama is local and does not require an API key).
        let mut ps = Vec::new();
        if config
            .llm
            .openai
            .as_ref()
            .is_some_and(|c| !c.api_key.trim().is_empty())
        {
            ps.push("openai");
        }
        if config
            .llm
            .anthropic
            .as_ref()
            .is_some_and(|c| !c.api_key.trim().is_empty())
        {
            ps.push("anthropic");
        }
        if config
            .llm
            .gemini
            .as_ref()
            .is_some_and(|c| !c.api_key.trim().is_empty())
        {
            ps.push("gemini");
        }
        if config
            .llm
            .grok
            .as_ref()
            .is_some_and(|c| !c.api_key.trim().is_empty())
        {
            ps.push("grok");
        }
        if config.llm.ollama.is_some() {
            ps.push("ollama");
        }
        if ps.is_empty() {
            println!("{}", "No LLM providers configured.".yellow());
            return Ok(());
        }
        ps
    };

    for (i, prov) in providers.iter().enumerate() {
        if i > 0 {
            println!();
        }

        let (api_key, base_url) = api_key_and_base_url_for_provider(&config, prov);
        let key_ref = api_key.as_deref();
        let url_ref = base_url.as_deref();

        let spinner = super::spinner::brand_spinner(format!("Fetching {prov} models…"));

        let models = rt.block_on(gestura_core::list_models_for_provider(
            prov, key_ref, url_ref,
        ));

        spinner.finish_and_clear();

        match models {
            Ok(list) if list.is_empty() => {
                println!(
                    "{} {} — {}",
                    "○".dimmed(),
                    prov.cyan().bold(),
                    "no models found".yellow()
                );
            }
            Ok(list) => {
                println!(
                    "{} {} ({} models)",
                    "●".green(),
                    prov.cyan().bold(),
                    list.len()
                );
                for m in &list {
                    println!("    {} {}", m.id.dimmed(), m.name);
                }
            }
            Err(e) => {
                println!(
                    "{} {} — {}",
                    "✗".red(),
                    prov.cyan().bold(),
                    format!("{e}").red()
                );
            }
        }
    }

    Ok(())
}

/// Extract the API key and optional base URL for a provider from config.
fn api_key_and_base_url_for_provider(
    config: &AppConfig,
    provider: &str,
) -> (Option<String>, Option<String>) {
    match provider {
        "openai" => {
            let cfg = config.llm.openai.as_ref();
            (
                cfg.map(|c| c.api_key.clone()).filter(|k| !k.is_empty()),
                cfg.and_then(|c| c.base_url.clone()),
            )
        }
        "anthropic" => {
            let cfg = config.llm.anthropic.as_ref();
            (
                cfg.map(|c| c.api_key.clone()).filter(|k| !k.is_empty()),
                cfg.and_then(|c| c.base_url.clone()),
            )
        }
        "gemini" => {
            let cfg = config.llm.gemini.as_ref();
            (
                cfg.map(|c| c.api_key.clone()).filter(|k| !k.is_empty()),
                cfg.and_then(|c| c.base_url.clone()),
            )
        }
        "grok" => {
            let cfg = config.llm.grok.as_ref();
            (
                cfg.map(|c| c.api_key.clone()).filter(|k| !k.is_empty()),
                cfg.and_then(|c| c.base_url.clone()),
            )
        }
        "ollama" => {
            let cfg = config.llm.ollama.as_ref();
            (None, cfg.map(|c| c.base_url.clone()))
        }
        _ => (None, None),
    }
}

fn run_whisper(action: &WhisperAction) -> Result<()> {
    // Ensure models directory exists
    let models_dir = get_models_dir();
    std::fs::create_dir_all(&models_dir).ok();

    match action {
        WhisperAction::List => {
            println!("{}", "Whisper Models".bold());
            println!();

            let models = ["tiny", "base", "small", "medium", "large"];

            for model in models {
                let downloaded = is_model_downloaded(model);
                let (size, desc) = get_model_info(model);
                let status = if downloaded {
                    "●".green()
                } else {
                    "○".dimmed()
                };
                let name = if downloaded {
                    model.green().to_string()
                } else {
                    model.to_string()
                };

                println!("  {} {:8} - {} ({})", status, name, desc, size);
            }

            println!();
            println!(
                "{} = downloaded, {} = not downloaded",
                "●".green(),
                "○".dimmed()
            );
            println!();
            println!(
                "Models directory: {}",
                models_dir.display().to_string().dimmed()
            );

            // Show current active model
            let config = AppConfig::load();
            if let Some(model_path) = &config.voice.local_model_path {
                println!("Active model: {}", model_path.cyan());
            }
        }
        WhisperAction::Download { model } => {
            println!("Downloading Whisper model: {}", model.cyan());
            println!();

            if is_model_downloaded(model) {
                println!("{} Model '{}' is already downloaded.", "✓".green(), model);
                return Ok(());
            }

            let (size, _) = get_model_info(model);
            println!("Size: {}", size);
            println!("Destination: {}", models_dir.display());
            println!();

            // Model download URL (using Hugging Face)
            let url = format!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{}.bin",
                model
            );

            println!("Download URL: {}", url.dimmed());
            println!();
            println!("{}", "To download manually:".yellow());
            println!(
                "  curl -L -o {} \"{}\"",
                models_dir.join(format!("ggml-{}.bin", model)).display(),
                url
            );
            println!();
            println!("{}", "(Automatic download not yet implemented)".yellow());
        }
        WhisperAction::Use { model } => {
            if !is_model_downloaded(model) {
                eprintln!("{}: Model '{}' is not downloaded.", "error".red(), model);
                eprintln!(
                    "Run: {} first",
                    format!("gestura model whisper download {}", model).cyan()
                );
                std::process::exit(2);
            }

            let model_path = models_dir.join(format!("ggml-{}.bin", model));

            // Update config
            let mut config = AppConfig::load();
            config.update_local_whisper_model_path(model_path.to_string_lossy().to_string());

            if let Err(e) = config.save() {
                eprintln!("{}: Failed to save config: {}", "error".red(), e);
                std::process::exit(2);
            }

            println!(
                "{} Active Whisper model set to: {}",
                "✓".green(),
                model.cyan()
            );
        }
    }
    Ok(())
}
