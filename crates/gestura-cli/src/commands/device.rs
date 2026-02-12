//! Audio device management command

use super::Result;
use crate::DeviceAction;
use colored::Colorize;
use gestura_core::list_audio_input_devices;
use std::time::Duration;

pub fn run(action: &DeviceAction) -> Result<()> {
    match action {
        DeviceAction::List => {
            println!("{}", "Audio Input Devices".bold());
            println!();

            let devices = list_audio_input_devices();

            if devices.is_empty() {
                println!("  {}", "(no audio input devices found)".dimmed());
                println!();
                println!("Make sure your microphone is connected and permissions are granted.");
            } else {
                println!("{:4} {}", "ID".underline(), "NAME".underline());

                for (idx, device) in devices.iter().enumerate() {
                    let marker = if device.is_default { "●" } else { " " };
                    let name = if device.is_default {
                        format!("{} (default)", device.name).green().to_string()
                    } else {
                        device.name.clone()
                    };

                    println!(
                        "{} {:3} {}",
                        if device.is_default {
                            marker.green().to_string()
                        } else {
                            marker.to_string()
                        },
                        idx,
                        name
                    );
                }

                println!();
                println!("Total: {} device(s)", devices.len());
            }
        }
        DeviceAction::Scan => {
            println!("{}", "Scanning for audio devices...".cyan());
            println!();

            let spinner = super::spinner::brand_spinner("Enumerating devices...");

            // Small delay to show the spinner
            std::thread::sleep(Duration::from_millis(500));

            let devices = list_audio_input_devices();
            spinner.finish_and_clear();

            println!(
                "{} Found {} audio input device(s)",
                "✓".green(),
                devices.len()
            );
            println!();

            for device in &devices {
                let default_marker = if device.is_default { " (default)" } else { "" };
                println!(
                    "  {} {}{}",
                    "•".cyan(),
                    device.name,
                    default_marker.dimmed()
                );
            }

            if devices.is_empty() {
                println!("  {}", "(no devices found)".dimmed());
            }
        }
        DeviceAction::Connect { device_id } => {
            // For audio devices, "connect" means selecting as the active input
            let devices = list_audio_input_devices();

            // Try to find device by index or name
            let device = if let Ok(idx) = device_id.parse::<usize>() {
                devices.get(idx)
            } else {
                devices
                    .iter()
                    .find(|d| d.name.to_lowercase().contains(&device_id.to_lowercase()))
            };

            match device {
                Some(dev) => {
                    println!("{} Selected audio device: {}", "✓".green(), dev.name.cyan());
                    println!();
                    println!("{}", "Note: Device selection is session-only.".dimmed());
                    println!("To persist, update your config with:");
                    println!(
                        "  {}",
                        format!("gestura config set voice.device \"{}\"", dev.name).cyan()
                    );
                }
                None => {
                    eprintln!("{}: Device '{}' not found", "error".red(), device_id);
                    eprintln!(
                        "Use {} to see available devices.",
                        "gestura device list".cyan()
                    );
                    std::process::exit(2);
                }
            }
        }
        DeviceAction::Disconnect { device_id } => {
            if let Some(id) = device_id {
                println!("Releasing device: {}", id.cyan());
            } else {
                println!("Releasing all audio devices...");
            }
            println!("{} Audio devices released", "✓".green());
            println!();
            println!(
                "{}",
                "Note: Devices will be re-acquired on next recording.".dimmed()
            );
        }
    }
    Ok(())
}
