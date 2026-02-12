//! Brand-themed CLI spinner using the arc pattern with a blue→purple gradient.
//!
//! Each frame of the arc spinner (`◜ ◠ ◝ ◞ ◡ ◟`) is colored at a different
//! point along the Gestura brand gradient:
//!
//! ```text
//! rgb(96, 165, 250)  →  rgb(192, 132, 252)
//!       blue-400            purple-400
//! ```
//!
//! The gradient rotates with the spinner, creating a smooth color sweep effect.

use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

/// Arc spinner frames (from spinoff's "arc" pattern).
const ARC_FRAMES: [&str; 6] = ["◜", "◠", "◝", "◞", "◡", "◟"];

/// Brand gradient endpoints (matches the website's blue→purple gradient).
const GRADIENT_START: (u8, u8, u8) = (96, 165, 250); // blue-400
const GRADIENT_END: (u8, u8, u8) = (192, 132, 252); // purple-400

/// Number of gradient steps for the rotating color sweep.
/// We generate more steps than frames so the color appears to travel smoothly.
const GRADIENT_STEPS: usize = 12;

/// Tick interval in milliseconds (arc default from spinoff is 100ms).
const TICK_MS: u64 = 80;

/// Wrap a string in ANSI 24-bit true-color foreground escape codes.
fn ansi_fg(r: u8, g: u8, b: u8, text: &str) -> String {
    format!("\x1b[38;2;{r};{g};{b}m{text}\x1b[0m")
}

/// Linearly interpolate between two RGB colors at position `t` ∈ [0.0, 1.0].
fn lerp_rgb(start: (u8, u8, u8), end: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    let r = (start.0 as f64 + (end.0 as f64 - start.0 as f64) * t).round() as u8;
    let g = (start.1 as f64 + (end.1 as f64 - start.1 as f64) * t).round() as u8;
    let b = (start.2 as f64 + (end.2 as f64 - start.2 as f64) * t).round() as u8;
    (r, g, b)
}

/// Build the full set of gradient-colored tick strings.
///
/// We generate `GRADIENT_STEPS` frames by cycling through the arc characters
/// while sweeping the color from blue → purple → blue (ping-pong).
fn build_tick_strings() -> Vec<String> {
    let mut ticks = Vec::with_capacity(GRADIENT_STEPS + 1);

    for i in 0..GRADIENT_STEPS {
        // Ping-pong: 0→1→0 over the full cycle for a smooth loop.
        let t_raw = i as f64 / GRADIENT_STEPS as f64;
        let t = if t_raw <= 0.5 {
            t_raw * 2.0
        } else {
            (1.0 - t_raw) * 2.0
        };

        let (r, g, b) = lerp_rgb(GRADIENT_START, GRADIENT_END, t);
        let frame = ARC_FRAMES[i % ARC_FRAMES.len()];
        ticks.push(ansi_fg(r, g, b, frame));
    }

    // Final tick (shown when spinner finishes) — use a dim version of the last frame.
    ticks.push(ansi_fg(
        GRADIENT_START.0,
        GRADIENT_START.1,
        GRADIENT_START.2,
        " ",
    ));

    ticks
}

/// Create a brand-themed spinner with the given initial message.
///
/// Uses the arc pattern (`◜ ◠ ◝ ◞ ◡ ◟`) with a rotating blue→purple gradient
/// that matches the Gestura website brand.
///
/// # Example
///
/// ```ignore
/// let spinner = brand_spinner("Connecting...");
/// // ... do work ...
/// spinner.finish_and_clear();
/// ```
pub fn brand_spinner(msg: impl Into<std::borrow::Cow<'static, str>>) -> ProgressBar {
    let ticks = build_tick_strings();
    let tick_refs: Vec<&str> = ticks.iter().map(String::as_str).collect();

    let style = ProgressStyle::default_spinner()
        .tick_strings(&tick_refs)
        .template("{spinner} {msg}")
        .expect("valid spinner template");

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(style);
    spinner.set_message(msg);
    spinner.enable_steady_tick(Duration::from_millis(TICK_MS));
    spinner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_strings_have_correct_count() {
        let ticks = build_tick_strings();
        // GRADIENT_STEPS frames + 1 final tick
        assert_eq!(ticks.len(), GRADIENT_STEPS + 1);
    }

    #[test]
    fn tick_strings_contain_ansi_escapes() {
        let ticks = build_tick_strings();
        for tick in &ticks {
            assert!(tick.contains("\x1b[38;2;"), "missing ANSI color: {tick}");
            assert!(tick.contains("\x1b[0m"), "missing ANSI reset: {tick}");
        }
    }

    #[test]
    fn gradient_endpoints_are_correct() {
        let start = lerp_rgb(GRADIENT_START, GRADIENT_END, 0.0);
        let end = lerp_rgb(GRADIENT_START, GRADIENT_END, 1.0);
        assert_eq!(start, GRADIENT_START);
        assert_eq!(end, GRADIENT_END);
    }
}
