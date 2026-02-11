//! Voice listening command

use super::Result;
use colored::Colorize;
use gestura_core::{
    AudioCaptureConfig, SpeechProcessorCoreExt, get_speech_processor, is_microphone_available,
};
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

pub fn run(transcribe_only: bool, whisper_model: &str) -> Result<()> {
    // Check microphone availability first
    if !is_microphone_available() {
        eprintln!("{}: No microphone available", "error".red());
        eprintln!("Please check your audio input device and permissions.");
        std::process::exit(4); // Permission denied exit code
    }

    println!("{}", "Voice Input Mode".bold());
    println!("Using Whisper model: {}", whisper_model.cyan());
    println!();

    if transcribe_only {
        println!("Mode: {} (not sending to LLM)", "Transcription only".cyan());
    } else {
        println!("Mode: {} (transcription + LLM)", "Voice to AI".cyan());
    }
    println!();

    // Show instructions
    println!("Speak into your microphone. Recording will stop automatically");
    println!(
        "when silence is detected, or press {} to cancel.",
        "Ctrl+C".yellow()
    );
    println!();

    // Create runtime for async execution
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async {
        // Show recording indicator
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        spinner.set_message("Listening... (speak now)");
        spinner.enable_steady_tick(Duration::from_millis(100));

        // Record audio using gestura-core
        let speech_processor = get_speech_processor();
        let audio_config = AudioCaptureConfig {
            device_name: None, // Use default device
            silence_threshold: 0.01,
            silence_timeout_secs: 1.5,
            max_recording_secs: 30,
            wait_for_speech_timeout_secs: 10,
        };

        let result = speech_processor
            .record_audio_to_file(audio_config.device_name.clone())
            .await;
        spinner.finish_and_clear();

        match result {
            Ok((duration, audio_path)) => {
                println!("{} Recorded {:.1}s of audio", "✓".green(), duration);

                // Transcribe using speech processor (whisper-rs or OpenAI Whisper API)
                println!();
                println!("{}", "Transcribing...".dimmed());

                match speech_processor.transcribe_audio(&audio_path).await {
                    Ok(transcription) => {
                        println!("{}", "Transcription:".bold());
                        println!("{}", transcription.text);

                        if !transcribe_only {
                            println!();
                            println!("{}", "Processing with LLM...".dimmed());

                            match speech_processor.process_with_llm(&transcription.text).await {
                                Ok(response) => {
                                    println!("{}", "AI Response:".bold());
                                    println!("{}", response.text);
                                }
                                Err(e) => {
                                    eprintln!("{}: {}", "LLM processing failed".red(), e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("{}: {}", "Transcription failed".red(), e);
                        println!(
                            "{}",
                            "Tip: Configure OpenAI API key for cloud transcription, or enable voice-local feature."
                                .yellow()
                        );
                        println!("Audio file: {}", audio_path.display().to_string().dimmed());
                    }
                }

                // Clean up temp file
                let _ = std::fs::remove_file(&audio_path);

                Ok(())
            }
            Err(e) => {
                eprintln!("{}: {}", "Recording failed".red(), e);
                Err(e.into())
            }
        }
    })
}
