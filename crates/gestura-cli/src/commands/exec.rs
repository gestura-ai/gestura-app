//! Execute single prompt command

use super::Result;
use colored::Colorize;
use gestura_core::{AgentPipeline, AgentRequest, AppConfig, AppConfigSecurityExt, RequestSource};
use std::io::{self, IsTerminal, Read};
use std::path::Path;

pub fn run(prompt: Option<&str>, file: Option<&Path>, model: Option<&str>) -> Result<()> {
    // Get prompt from argument, file, or stdin
    let prompt_text = if let Some(p) = prompt {
        p.to_string()
    } else if let Some(f) = file {
        std::fs::read_to_string(f)?
    } else if !io::stdin().is_terminal() {
        // Read from stdin if piped
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else {
        return Err(
            "No prompt provided. Use positional argument, --file, or pipe to stdin.".into(),
        );
    };

    if prompt_text.trim().is_empty() {
        return Err(
            "No prompt provided. Use positional argument, --file, or pipe to stdin.".into(),
        );
    }

    // Load config and apply optional CLI model override in core
    let mut config = AppConfig::load();
    let effective = gestura_core::llm_overrides::apply_cli_model_arg_overrides(&mut config, model);
    if !effective.model.trim().is_empty() {
        tracing::debug!(
            provider = %effective.provider,
            model = %effective.model,
            "Using CLI model override"
        );
    }

    // Create runtime for async execution
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async {
        tracing::debug!(
            "Sending prompt via AgentPipeline to {} provider",
            effective.provider
        );

        let pipeline = AgentPipeline::with_provider_optimized_config(config);
        let request = AgentRequest::new(prompt_text)
            .with_streaming(false)
            .with_source(RequestSource::CliBasic)
            .with_tools_enabled(false);

        match pipeline.process_blocking(request).await {
            Ok(response) => {
                // Output response to stdout (no formatting for piping)
                println!("{}", response.content);
                Ok(())
            }
            Err(e) => {
                eprintln!("{}: {}", "LLM Error".red(), e);
                Err(e.into())
            }
        }
    })
}
