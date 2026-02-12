//! Screen capture and recording tool
//!
//! Provides screen operations:
//! - capture: Take a screenshot
//! - record-start: Start screen recording
//! - record-stop: Stop screen recording

use super::super::Result;
use colored::Colorize;
use gestura_core::tools::screen::{CaptureRegion, ScreenTools};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Global screen tools instance
static SCREEN_TOOLS: OnceLock<ScreenTools> = OnceLock::new();

fn get_screen_tools() -> &'static ScreenTools {
    SCREEN_TOOLS.get_or_init(ScreenTools::new)
}

/// Screen subcommand options
pub enum ScreenSubcommand {
    Capture {
        path: PathBuf,
        region: Option<String>, // Format: "x,y,width,height"
        display: Option<u32>,
    },
    RecordStart {
        path: PathBuf,
        region: Option<String>,
        display: Option<u32>,
    },
    RecordStop {
        recording_id: String,
    },
}

/// Run screen subcommand
pub fn run(cmd: ScreenSubcommand) -> Result<()> {
    match cmd {
        ScreenSubcommand::Capture {
            path,
            region,
            display,
        } => run_capture(&path, region.as_deref(), display),
        ScreenSubcommand::RecordStart {
            path,
            region,
            display,
        } => run_record_start(&path, region.as_deref(), display),
        ScreenSubcommand::RecordStop { recording_id } => run_record_stop(&recording_id),
    }
}

fn run_capture(path: &Path, region: Option<&str>, display: Option<u32>) -> Result<()> {
    println!(
        "{} screenshot to {}",
        "Capturing".bold(),
        path.display().to_string().cyan()
    );

    let region_struct = region.and_then(parse_region);
    let result = get_screen_tools().screenshot(path, region_struct, display)?;

    println!("{} Screenshot saved", "✓".green());
    println!("  Path: {}", result.path.display().to_string().cyan());
    println!(
        "  Size: {}x{}",
        result.width.unwrap_or(0),
        result.height.unwrap_or(0)
    );
    println!("  Format: {}", result.format);
    println!("  File size: {} bytes", result.file_size_bytes);

    Ok(())
}

fn run_record_start(path: &Path, region: Option<&str>, display: Option<u32>) -> Result<()> {
    println!(
        "{} screen recording to {}",
        "Starting".bold(),
        path.display().to_string().cyan()
    );

    let region_struct = region.and_then(parse_region);
    let result = get_screen_tools().start_recording(path, region_struct, display)?;

    println!("{} Recording started", "✓".green());
    println!("  Recording ID: {}", result.recording_id.yellow());
    println!(
        "  Output path: {}",
        result.output_path.display().to_string().cyan()
    );
    println!("  Started at: {}", result.started_at);
    println!();
    println!(
        "{}",
        format!(
            "To stop recording, run: gestura tools screen record-stop {}",
            result.recording_id
        )
        .dimmed()
    );

    Ok(())
}

fn run_record_stop(recording_id: &str) -> Result<()> {
    println!("{} recording {}", "Stopping".bold(), recording_id.yellow());

    let result = get_screen_tools().stop_recording(recording_id)?;

    println!("{} Recording stopped", "✓".green());
    println!(
        "  Output path: {}",
        result.path.display().to_string().cyan()
    );
    println!("  Duration: {:.2}s", result.duration_secs);
    println!("  File size: {} bytes", result.file_size_bytes);

    Ok(())
}

/// Parse region string "x,y,width,height" into CaptureRegion
fn parse_region(s: &str) -> Option<CaptureRegion> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() == 4 {
        Some(CaptureRegion {
            x: parts[0].parse().ok()?,
            y: parts[1].parse().ok()?,
            width: parts[2].parse().ok()?,
            height: parts[3].parse().ok()?,
        })
    } else {
        None
    }
}
