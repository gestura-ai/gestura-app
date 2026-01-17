//! Execute single prompt command

use super::Result;
use colored::Colorize;
use gestura_core::{AgentContext, AppConfig, select_provider};
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

    // Load config and optionally override model
    let mut config = AppConfig::load();
    if let Some(m) = model {
        // Parse model string - could be "provider:model" or just "model"
        if let Some((provider, model_name)) = m.split_once(':') {
            config.llm.primary = provider.to_string();
            // Update the model in the appropriate provider config
            match provider {
                "openai" => {
                    if let Some(ref mut openai) = config.llm.openai {
                        openai.model = model_name.to_string();
                    }
                }
                "anthropic" => {
                    if let Some(ref mut anthropic) = config.llm.anthropic {
                        anthropic.model = model_name.to_string();
                    }
                }
                _ => {}
            }
        }
        tracing::debug!("Using model: {}", m);
    }

    // Create runtime for async execution
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async {
        let provider = select_provider(
            &config,
            &AgentContext {
                agent_id: "cli-exec".into(),
            },
        );

        tracing::debug!("Sending prompt to {} provider", config.llm.primary);

        match provider.call(&prompt_text).await {
            Ok(response) => {
                // Output response to stdout (no formatting for piping)
                println!("{}", response);
                Ok(())
            }
            Err(e) => {
                eprintln!("{}: {}", "LLM Error".red(), e);
                Err(e.into())
            }
        }
    })
}
