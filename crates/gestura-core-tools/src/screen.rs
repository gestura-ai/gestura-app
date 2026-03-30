//! Screen capture and recording tool
//!
//! Provides cross-platform screenshot and screen recording capabilities with structured output.
//! All functions return data structures rather than formatted strings.

use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

// ============================================================================
// Bundled ffmpeg resolver
// ============================================================================

/// Resolve the path to the ffmpeg binary to use for screen recording.
///
/// Resolution order (first match wins):
/// 1. `GESTURA_FFMPEG_PATH` environment variable — allows the host app or tests
///    to point at a specific binary.
/// 2. Bundled sidecar placed next to the running executable by the Tauri
///    installer (named `ffmpeg-<target-triple>[.exe]`).
/// 3. `ffmpeg` on the system `PATH` (original behaviour — requires the user to
///    have ffmpeg installed).
fn ffmpeg_binary() -> OsString {
    // 1. Explicit override via env var.
    if let Ok(path) = std::env::var("GESTURA_FFMPEG_PATH")
        && !path.is_empty()
    {
        return path.into();
    }

    // 2. Bundled sidecar next to the running executable.
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for name in bundled_ffmpeg_names() {
            let candidate = dir.join(name);
            if candidate.is_file() {
                tracing::debug!("Using bundled ffmpeg sidecar: {:?}", candidate);
                return candidate.into_os_string();
            }
        }
    }

    // 3. System ffmpeg fallback.
    tracing::debug!(
        "No bundled ffmpeg found; falling back to system ffmpeg. \
         Install ffmpeg or set GESTURA_FFMPEG_PATH to enable screen recording."
    );
    OsString::from("ffmpeg")
}

/// Platform-specific candidate sidecar filenames (Tauri externalBin naming).
///
/// Tauri appends the target triple to the base name supplied in
/// `bundle.externalBin`, so the staged binary must be named
/// `ffmpeg-<triple>[.exe]`.  We probe the most-common triples for each
/// platform so that both architecture-specific and universal builds work.
fn bundled_ffmpeg_names() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &[
            "ffmpeg-universal-apple-darwin",
            "ffmpeg-aarch64-apple-darwin",
            "ffmpeg-x86_64-apple-darwin",
        ]
    }
    #[cfg(target_os = "linux")]
    {
        &[
            "ffmpeg-x86_64-unknown-linux-gnu",
            "ffmpeg-aarch64-unknown-linux-gnu",
            "ffmpeg-x86_64-unknown-linux-musl",
        ]
    }
    #[cfg(target_os = "windows")]
    {
        &[
            "ffmpeg-x86_64-pc-windows-msvc.exe",
            "ffmpeg-i686-pc-windows-msvc.exe",
            "ffmpeg-x86_64-pc-windows-gnu.exe",
            "ffmpeg.exe",
        ]
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        &[]
    }
}

#[cfg(unix)]
use nix::sys::signal::{Signal, kill};
#[cfg(unix)]
use nix::unistd::Pid;

/// Result of capturing a screenshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResult {
    pub path: PathBuf,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub format: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub file_size_bytes: u64,
}

/// Result of starting a screen recording
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStartResult {
    pub recording_id: String,
    pub output_path: PathBuf,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

/// Result of stopping a screen recording
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStopResult {
    pub recording_id: String,
    pub path: PathBuf,
    pub duration_secs: f64,
    pub file_size_bytes: u64,
    pub format: String,
}

/// Region to capture (optional)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Screen capture and recording service
#[derive(Debug)]
struct ActiveRecording {
    child: std::process::Child,
    output_path: PathBuf,
    started_at: chrono::DateTime<chrono::Utc>,
}

pub struct ScreenTools {
    active_recordings: Arc<Mutex<HashMap<String, ActiveRecording>>>,
}

impl Default for ScreenTools {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenTools {
    fn shared_recordings() -> Arc<Mutex<HashMap<String, ActiveRecording>>> {
        static RECORDINGS: OnceLock<Arc<Mutex<HashMap<String, ActiveRecording>>>> = OnceLock::new();
        RECORDINGS
            .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
            .clone()
    }

    fn ensure_parent_dir(path: &Path) -> Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    pub fn new() -> Self {
        Self {
            active_recordings: Self::shared_recordings(),
        }
    }

    /// Capture a screenshot
    ///
    /// # Arguments
    /// * `output_path` - Where to save the screenshot
    /// * `region` - Optional region to capture (None = full screen)
    /// * `display` - Optional display number (None = primary display)
    pub fn screenshot(
        &self,
        output_path: &Path,
        region: Option<CaptureRegion>,
        display: Option<u32>,
    ) -> Result<ScreenshotResult> {
        Self::ensure_parent_dir(output_path)?;

        #[cfg(target_os = "macos")]
        {
            self.screenshot_macos(output_path, region, display)
        }

        #[cfg(target_os = "linux")]
        {
            self.screenshot_linux(output_path, region, display)
        }

        #[cfg(target_os = "windows")]
        {
            self.screenshot_windows(output_path, region, display)
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Screenshot not supported on this platform",
            )))
        }
    }

    /// Start screen recording
    ///
    /// # Arguments
    /// * `output_path` - Where to save the recording
    /// * `region` - Optional region to record (None = full screen)
    /// * `display` - Optional display number (None = primary display)
    pub fn start_recording(
        &self,
        output_path: &Path,
        region: Option<CaptureRegion>,
        display: Option<u32>,
    ) -> Result<RecordingStartResult> {
        Self::ensure_parent_dir(output_path)?;

        #[cfg(target_os = "macos")]
        {
            self.start_recording_macos(output_path, region, display)
        }

        #[cfg(target_os = "linux")]
        {
            self.start_recording_linux(output_path, region, display)
        }

        #[cfg(target_os = "windows")]
        {
            self.start_recording_windows(output_path, region, display)
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Screen recording not supported on this platform",
            )))
        }
    }

    /// Stop an active screen recording
    pub fn stop_recording(&self, recording_id: &str) -> Result<RecordingStopResult> {
        #[cfg(target_os = "macos")]
        {
            self.stop_recording_macos(recording_id)
        }

        #[cfg(target_os = "linux")]
        {
            self.stop_recording_linux(recording_id)
        }

        #[cfg(target_os = "windows")]
        {
            self.stop_recording_windows(recording_id)
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Screen recording not supported on this platform",
            )))
        }
    }

    // ============================================================================
    // macOS Implementation
    // ============================================================================

    #[cfg(target_os = "macos")]
    fn screenshot_macos(
        &self,
        output_path: &Path,
        region: Option<CaptureRegion>,
        display: Option<u32>,
    ) -> Result<ScreenshotResult> {
        let mut cmd = Command::new("screencapture");

        // Add region if specified
        if let Some(r) = region {
            cmd.arg("-R");
            cmd.arg(format!("{},{},{},{}", r.x, r.y, r.width, r.height));
        }

        // Add display if specified
        if let Some(d) = display {
            cmd.arg("-D");
            cmd.arg(d.to_string());
        }

        // Output path
        cmd.arg(output_path);

        let output = cmd.output().map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to execute screencapture: {}",
                e
            )))
        })?;

        if !output.status.success() {
            return Err(AppError::Io(std::io::Error::other(format!(
                "screencapture failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ))));
        }

        let metadata = std::fs::metadata(output_path)?;
        let file_size_bytes = metadata.len();

        // Try to get image dimensions using sips
        let (width, height) = self
            .get_image_dimensions_macos(output_path)
            .unwrap_or((None, None));

        Ok(ScreenshotResult {
            path: output_path.to_path_buf(),
            width,
            height,
            format: output_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("png")
                .to_string(),
            timestamp: chrono::Utc::now(),
            file_size_bytes,
        })
    }

    #[cfg(target_os = "macos")]
    fn get_image_dimensions_macos(&self, path: &Path) -> Option<(Option<u32>, Option<u32>)> {
        let output = Command::new("sips")
            .arg("-g")
            .arg("pixelWidth")
            .arg("-g")
            .arg("pixelHeight")
            .arg(path)
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut width = None;
        let mut height = None;

        for line in stdout.lines() {
            if line.contains("pixelWidth:") {
                width = line.split_whitespace().last()?.parse().ok();
            } else if line.contains("pixelHeight:") {
                height = line.split_whitespace().last()?.parse().ok();
            }
        }

        Some((width, height))
    }

    #[cfg(target_os = "macos")]
    fn start_recording_macos(
        &self,
        output_path: &Path,
        region: Option<CaptureRegion>,
        _display: Option<u32>,
    ) -> Result<RecordingStartResult> {
        // Use ffmpeg for screen recording on macOS (bundled sidecar preferred)
        let mut cmd = Command::new(ffmpeg_binary());
        cmd.arg("-f").arg("avfoundation");

        // Input device (screen capture)
        cmd.arg("-i").arg("1:none"); // Capture screen 1, no audio

        // Add region filter if specified
        if let Some(r) = region {
            cmd.arg("-filter:v");
            cmd.arg(format!("crop={}:{}:{}:{}", r.width, r.height, r.x, r.y));
        }

        // Output settings
        cmd.arg("-c:v").arg("libx264");
        cmd.arg("-preset").arg("ultrafast");
        cmd.arg("-y"); // Overwrite output file
        cmd.arg(output_path);

        let child = cmd.spawn().map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to start ffmpeg: {}",
                e
            )))
        })?;

        let recording_id = uuid::Uuid::new_v4().to_string();

        let started_at = chrono::Utc::now();
        if let Ok(mut recordings) = self.active_recordings.lock() {
            recordings.insert(
                recording_id.clone(),
                ActiveRecording {
                    child,
                    output_path: output_path.to_path_buf(),
                    started_at,
                },
            );
        }

        Ok(RecordingStartResult {
            recording_id,
            output_path: output_path.to_path_buf(),
            started_at,
        })
    }

    #[cfg(target_os = "macos")]
    fn stop_recording_macos(&self, recording_id: &str) -> Result<RecordingStopResult> {
        let mut handle = if let Ok(mut recordings) = self.active_recordings.lock() {
            recordings.remove(recording_id).ok_or_else(|| {
                AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Recording not found: {}", recording_id),
                ))
            })?
        } else {
            return Err(AppError::Io(std::io::Error::other(
                "Failed to access recordings",
            )));
        };

        // Send SIGINT to ffmpeg to stop recording gracefully
        #[cfg(unix)]
        {
            let pid = Pid::from_raw(handle.child.id() as i32);
            let _ = kill(pid, Signal::SIGINT);
        }

        // Wait for process to finish
        let _ = handle.child.wait()?;

        let duration_ms = (chrono::Utc::now() - handle.started_at).num_milliseconds();
        let duration_secs = (duration_ms.max(0) as f64) / 1000.0;

        let metadata = std::fs::metadata(&handle.output_path)?;
        let file_size_bytes = metadata.len();

        Ok(RecordingStopResult {
            recording_id: recording_id.to_string(),
            path: handle.output_path.clone(),
            duration_secs,
            file_size_bytes,
            format: handle
                .output_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp4")
                .to_string(),
        })
    }

    // ============================================================================
    // Linux Implementation
    // ============================================================================

    #[cfg(target_os = "linux")]
    fn screenshot_linux(
        &self,
        output_path: &Path,
        region: Option<CaptureRegion>,
        _display: Option<u32>,
    ) -> Result<ScreenshotResult> {
        // Try xdg-desktop-portal first (Wayland-compatible)
        if self
            .try_screenshot_portal(output_path, region.clone())
            .is_ok()
        {
            return self.get_screenshot_result(output_path);
        }

        // Fallback to scrot for X11
        let mut cmd = Command::new("scrot");

        if let Some(r) = region {
            cmd.arg("-a");
            cmd.arg(format!("{},{},{},{}", r.x, r.y, r.width, r.height));
        }

        cmd.arg(output_path);

        let output = cmd.output().map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to execute scrot (install scrot or use Wayland with xdg-desktop-portal): {}",
                e
            )))
        })?;

        if !output.status.success() {
            return Err(AppError::Io(std::io::Error::other(format!(
                "scrot failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ))));
        }

        self.get_screenshot_result(output_path)
    }

    #[cfg(target_os = "linux")]
    fn try_screenshot_portal(
        &self,
        output_path: &Path,
        _region: Option<CaptureRegion>,
    ) -> Result<()> {
        // Use grim for Wayland screenshot (works with xdg-desktop-portal)
        let output = Command::new("grim")
            .arg(output_path)
            .output()
            .map_err(|e| {
                AppError::Io(std::io::Error::other(format!("grim not available: {}", e)))
            })?;

        if !output.status.success() {
            return Err(AppError::Io(std::io::Error::other("grim failed")));
        }

        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn get_screenshot_result(&self, output_path: &Path) -> Result<ScreenshotResult> {
        let metadata = std::fs::metadata(output_path)?;
        let file_size_bytes = metadata.len();

        // Try to get dimensions using imagemagick identify
        let (width, height) = self
            .get_image_dimensions_linux(output_path)
            .unwrap_or((None, None));

        Ok(ScreenshotResult {
            path: output_path.to_path_buf(),
            width,
            height,
            format: output_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("png")
                .to_string(),
            timestamp: chrono::Utc::now(),
            file_size_bytes,
        })
    }

    #[cfg(target_os = "linux")]
    fn get_image_dimensions_linux(&self, path: &Path) -> Option<(Option<u32>, Option<u32>)> {
        let output = Command::new("identify")
            .arg("-format")
            .arg("%w %h")
            .arg(path)
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.split_whitespace().collect();

        if parts.len() >= 2 {
            let width = parts[0].parse().ok();
            let height = parts[1].parse().ok();
            Some((width, height))
        } else {
            None
        }
    }

    #[cfg(target_os = "linux")]
    fn start_recording_linux(
        &self,
        output_path: &Path,
        region: Option<CaptureRegion>,
        _display: Option<u32>,
    ) -> Result<RecordingStartResult> {
        // Use wf-recorder for Wayland or ffmpeg for X11
        let mut cmd = if self.is_wayland() {
            let mut c = Command::new("wf-recorder");

            if let Some(r) = region {
                c.arg("-g");
                c.arg(format!("{},{} {}x{}", r.x, r.y, r.width, r.height));
            }

            c.arg("-f");
            c.arg(output_path);
            c
        } else {
            // X11 fallback using bundled/system ffmpeg
            let mut c = Command::new(ffmpeg_binary());
            c.arg("-f").arg("x11grab");
            c.arg("-i").arg(":0.0");

            if let Some(r) = region {
                c.arg("-video_size");
                c.arg(format!("{}x{}", r.width, r.height));
                c.arg("-grab_x").arg(r.x.to_string());
                c.arg("-grab_y").arg(r.y.to_string());
            }

            c.arg("-c:v").arg("libx264");
            c.arg("-preset").arg("ultrafast");
            c.arg("-y");
            c.arg(output_path);
            c
        };

        let child = cmd.spawn().map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to start screen recording: {}",
                e
            )))
        })?;

        let recording_id = uuid::Uuid::new_v4().to_string();

        let started_at = chrono::Utc::now();
        if let Ok(mut recordings) = self.active_recordings.lock() {
            recordings.insert(
                recording_id.clone(),
                ActiveRecording {
                    child,
                    output_path: output_path.to_path_buf(),
                    started_at,
                },
            );
        }

        Ok(RecordingStartResult {
            recording_id,
            output_path: output_path.to_path_buf(),
            started_at,
        })
    }

    #[cfg(target_os = "linux")]
    fn is_wayland(&self) -> bool {
        std::env::var("WAYLAND_DISPLAY").is_ok()
    }

    #[cfg(target_os = "linux")]
    fn stop_recording_linux(&self, recording_id: &str) -> Result<RecordingStopResult> {
        let mut handle = if let Ok(mut recordings) = self.active_recordings.lock() {
            recordings.remove(recording_id).ok_or_else(|| {
                AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Recording not found: {}", recording_id),
                ))
            })?
        } else {
            return Err(AppError::Io(std::io::Error::other(
                "Failed to access recordings",
            )));
        };

        // Send SIGINT to stop recording gracefully
        let pid = Pid::from_raw(handle.child.id() as i32);
        let _ = kill(pid, Signal::SIGINT);

        let _ = handle.child.wait()?;

        let duration_ms = (chrono::Utc::now() - handle.started_at).num_milliseconds();
        let duration_secs = (duration_ms.max(0) as f64) / 1000.0;

        let metadata = std::fs::metadata(&handle.output_path)?;
        let file_size_bytes = metadata.len();

        Ok(RecordingStopResult {
            recording_id: recording_id.to_string(),
            path: handle.output_path.clone(),
            duration_secs,
            file_size_bytes,
            format: handle
                .output_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp4")
                .to_string(),
        })
    }

    // ============================================================================
    // Windows Implementation
    // ============================================================================

    #[cfg(target_os = "windows")]
    fn screenshot_windows(
        &self,
        output_path: &Path,
        region: Option<CaptureRegion>,
        _display: Option<u32>,
    ) -> Result<ScreenshotResult> {
        // Use PowerShell to capture screenshot
        let ps_script = if let Some(r) = region {
            format!(
                r#"Add-Type -AssemblyName System.Windows.Forms,System.Drawing;
                $bounds = New-Object Drawing.Rectangle {},{},{},{};
                $bmp = New-Object Drawing.Bitmap $bounds.Width,$bounds.Height;
                $graphics = [Drawing.Graphics]::FromImage($bmp);
                $graphics.CopyFromScreen($bounds.Location, [Drawing.Point]::Empty, $bounds.Size);
                $bmp.Save('{}');
                $graphics.Dispose();
                $bmp.Dispose();"#,
                r.x,
                r.y,
                r.width,
                r.height,
                output_path.display()
            )
        } else {
            format!(
                r#"Add-Type -AssemblyName System.Windows.Forms,System.Drawing;
                $screen = [System.Windows.Forms.Screen]::PrimaryScreen;
                $bounds = $screen.Bounds;
                $bmp = New-Object Drawing.Bitmap $bounds.Width,$bounds.Height;
                $graphics = [Drawing.Graphics]::FromImage($bmp);
                $graphics.CopyFromScreen($bounds.Location, [Drawing.Point]::Empty, $bounds.Size);
                $bmp.Save('{}');
                $graphics.Dispose();
                $bmp.Dispose();"#,
                output_path.display()
            )
        };

        let output = Command::new("powershell")
            .arg("-NoProfile")
            .arg("-Command")
            .arg(&ps_script)
            .output()
            .map_err(|e| {
                AppError::Io(std::io::Error::other(format!(
                    "Failed to execute PowerShell: {}",
                    e
                )))
            })?;

        if !output.status.success() {
            return Err(AppError::Io(std::io::Error::other(format!(
                "PowerShell screenshot failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ))));
        }

        let metadata = std::fs::metadata(output_path)?;
        let file_size_bytes = metadata.len();

        Ok(ScreenshotResult {
            path: output_path.to_path_buf(),
            width: None, // Could parse from PowerShell output
            height: None,
            format: output_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("png")
                .to_string(),
            timestamp: chrono::Utc::now(),
            file_size_bytes,
        })
    }

    #[cfg(target_os = "windows")]
    fn start_recording_windows(
        &self,
        output_path: &Path,
        _region: Option<CaptureRegion>,
        _display: Option<u32>,
    ) -> Result<RecordingStartResult> {
        // Use bundled/system ffmpeg for Windows screen recording
        let mut cmd = Command::new(ffmpeg_binary());
        cmd.arg("-f").arg("gdigrab");
        cmd.arg("-i").arg("desktop");
        cmd.arg("-c:v").arg("libx264");
        cmd.arg("-preset").arg("ultrafast");
        cmd.arg("-y");
        cmd.arg(output_path);

        let child = cmd.spawn().map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to start ffmpeg for screen recording (bundled binary not found and \
                 ffmpeg is not on PATH — set GESTURA_FFMPEG_PATH if needed): {}",
                e
            )))
        })?;

        let recording_id = uuid::Uuid::new_v4().to_string();

        let started_at = chrono::Utc::now();
        if let Ok(mut recordings) = self.active_recordings.lock() {
            recordings.insert(
                recording_id.clone(),
                ActiveRecording {
                    child,
                    output_path: output_path.to_path_buf(),
                    started_at,
                },
            );
        }

        Ok(RecordingStartResult {
            recording_id,
            output_path: output_path.to_path_buf(),
            started_at,
        })
    }

    #[cfg(target_os = "windows")]
    fn stop_recording_windows(&self, recording_id: &str) -> Result<RecordingStopResult> {
        let mut handle = if let Ok(mut recordings) = self.active_recordings.lock() {
            recordings.remove(recording_id).ok_or_else(|| {
                AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Recording not found: {}", recording_id),
                ))
            })?
        } else {
            return Err(AppError::Io(std::io::Error::other(
                "Failed to access recordings",
            )));
        };

        // Kill the process (Windows doesn't have SIGINT)
        handle.child.kill()?;
        let _ = handle.child.wait()?;

        let duration_ms = (chrono::Utc::now() - handle.started_at).num_milliseconds();
        let duration_secs = (duration_ms.max(0) as f64) / 1000.0;

        let metadata = std::fs::metadata(&handle.output_path)?;
        let file_size_bytes = metadata.len();

        Ok(RecordingStopResult {
            recording_id: recording_id.to_string(),
            path: handle.output_path.clone(),
            duration_secs,
            file_size_bytes,
            format: handle
                .output_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp4")
                .to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_screen_tools_creation() {
        let tools = ScreenTools::new();
        // Just verify we can create the tools struct
        assert!(std::mem::size_of_val(&tools) > 0);
    }

    #[test]
    fn screen_tools_share_global_recording_registry() {
        let a = ScreenTools::new();
        let b = ScreenTools::new();
        assert!(Arc::ptr_eq(&a.active_recordings, &b.active_recordings));
    }

    #[test]
    fn test_capture_region_serialization() {
        let region = CaptureRegion {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };
        let json = serde_json::to_string(&region).unwrap();
        assert!(json.contains("800"));
        assert!(json.contains("600"));
    }

    #[test]
    fn test_screenshot_result_serialization() {
        let result = ScreenshotResult {
            path: PathBuf::from("/tmp/test.png"),
            width: Some(1920),
            height: Some(1080),
            format: "png".to_string(),
            timestamp: chrono::Utc::now(),
            file_size_bytes: 12345,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("1920"));
        assert!(json.contains("1080"));
        assert!(json.contains("png"));
    }
}
