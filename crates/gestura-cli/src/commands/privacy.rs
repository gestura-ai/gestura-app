//! GDPR privacy compliance command

use super::Result;
use crate::PrivacyAction;
use colored::Colorize;
use dialoguer::Confirm;
use gestura_core::get_gdpr_manager;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

pub fn run(action: &PrivacyAction) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;

    match action {
        PrivacyAction::Export { output } => {
            let output_path = output.clone().unwrap_or_else(|| {
                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
                std::path::PathBuf::from(format!("gestura-data-export-{}.json", timestamp))
            });

            println!("{}", "Exporting User Data (GDPR Data Portability)".bold());
            println!("Output file: {}", output_path.display());
            println!();

            let spinner = ProgressBar::new_spinner();
            spinner.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} {msg}")
                    .unwrap(),
            );
            spinner.set_message("Collecting user data...");
            spinner.enable_steady_tick(Duration::from_millis(100));

            rt.block_on(async {
                let gdpr = get_gdpr_manager().await;

                // Get user ID
                let user_id = whoami::username().unwrap_or_else(|_| "unknown".to_string());

                spinner.set_message(format!("Exporting data for user '{}'...", user_id));

                // Export user data using the GDPR manager
                match gdpr.export_user_data(&user_id).await {
                    Ok(data) => {
                        spinner.finish_and_clear();

                        // Add metadata
                        let mut export_data = serde_json::Map::new();
                        export_data.insert("export_date".to_string(),
                            serde_json::Value::String(chrono::Utc::now().to_rfc3339()));
                        export_data.insert("version".to_string(),
                            serde_json::Value::String(gestura_core::VERSION.to_string()));
                        export_data.insert("user_id".to_string(),
                            serde_json::Value::String(user_id.clone()));
                        export_data.insert("data".to_string(), data);

                        // Write to file
                        let json = serde_json::Value::Object(export_data);
                        std::fs::write(&output_path, serde_json::to_string_pretty(&json)?)?;

                        println!("{} Data exported to {}", "✓".green(), output_path.display());
                        println!();
                        println!("This file contains all your personal data stored by Gestura.");
                        println!("You can use this for data portability or to review what data is stored.");
                    }
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("{} Failed to export data: {}", "✗".red(), e);
                    }
                }

                Ok::<(), Box<dyn std::error::Error>>(())
            })?;
        }
        PrivacyAction::Delete { force } => {
            println!("{}", "Delete All User Data".bold().red());
            println!();
            println!("This will {} delete:", "permanently".red().bold());
            println!("  {} Configuration settings", "•".red());
            println!("  {} Cached models and data", "•".red());
            println!("  {} Audit logs", "•".red());
            println!();

            if !force {
                let confirmed = Confirm::new()
                    .with_prompt("Are you sure you want to delete all data?")
                    .default(false)
                    .interact()?;

                if !confirmed {
                    println!("{}", "Deletion cancelled.".yellow());
                    return Ok(());
                }
            }

            let spinner = ProgressBar::new_spinner();
            spinner.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.red} {msg}")
                    .unwrap(),
            );
            spinner.set_message("Deleting user data...");
            spinner.enable_steady_tick(Duration::from_millis(100));

            // Delete config directory
            let config_dir = dirs::config_dir().map(|p| p.join("gestura"));

            if let Some(dir) = config_dir
                && dir.exists()
            {
                spinner.set_message("Removing configuration directory...");
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    spinner.finish_and_clear();
                    eprintln!("{} Failed to delete config directory: {}", "✗".red(), e);
                    return Ok(());
                }
            }

            spinner.finish_and_clear();

            println!("{} All user data has been deleted.", "✓".green());
            println!();
            println!("Gestura will use default settings on next launch.");
        }
        PrivacyAction::Policy => {
            println!("{}", "Data Retention Policy".bold());
            println!();
            println!(
                "Gestura collects and stores the following data {}:",
                "locally".underline()
            );
            println!();
            println!(
                "  {} {}",
                "•".cyan(),
                "Chat sessions and conversation history".bold()
            );
            println!("    Stored indefinitely until manually deleted");
            println!();
            println!("  {} {}", "•".cyan(), "Configuration preferences".bold());
            println!("    API keys, model settings, UI preferences");
            println!();
            println!("  {} {}", "•".cyan(), "Voice recordings".bold());
            println!("    Temporary files deleted after transcription");
            println!();
            println!("  {} {}", "•".cyan(), "Usage telemetry".bold());
            println!("    Only if explicitly enabled; anonymous by default");
            println!();

            let data_dir = dirs::config_dir()
                .map(|p| p.join("gestura"))
                .unwrap_or_else(|| std::path::PathBuf::from("(unknown)"));

            println!("{}", "Data Locations:".underline());
            println!(
                "  Config:  {}",
                data_dir.join("config.toml").display().to_string().cyan()
            );
            println!(
                "  Data:    {}",
                data_dir.join("data").display().to_string().cyan()
            );
            println!(
                "  Logs:    {}",
                data_dir.join("logs").display().to_string().cyan()
            );
            println!();

            println!("{}", "Your Rights (GDPR):".underline());
            println!(
                "  {} Export all your data:  {}",
                "•".green(),
                "gestura privacy export".cyan()
            );
            println!(
                "  {} Delete all your data:  {}",
                "•".green(),
                "gestura privacy delete".cyan()
            );
            println!(
                "  {} View consent status:   {}",
                "•".green(),
                "(coming soon)".dimmed()
            );
            println!();

            println!("{}", "Third-Party Data Sharing:".underline());
            println!("  {} No data is sent to Gestura servers", "✓".green());
            println!(
                "  {} API requests go directly to your configured LLM provider",
                "✓".green()
            );
            println!(
                "  {} Voice data is processed locally (with local Whisper)",
                "✓".green()
            );
        }
    }
    Ok(())
}
