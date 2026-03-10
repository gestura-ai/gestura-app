//! Async screen capture operations for pipeline integration
//!
//! Wraps the synchronous [`ScreenTools`] via
//! `tokio::task::spawn_blocking` for use in async contexts (pipeline, GUI).

use crate::error::{AppError, Result};
use crate::screen::{CaptureRegion, ScreenTools, ScreenshotResult};
use base64::Engine;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
struct ScreenshotOutput {
    path: String,
    width: Option<u32>,
    height: Option<u32>,
    format: String,
    mime_type: String,
    timestamp: chrono::DateTime<chrono::Utc>,
    file_size_bytes: u64,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    inline_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inline_mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inline_kind: Option<String>,
}

#[derive(Debug, Serialize)]
struct RecordingStartOutput {
    recording_id: String,
    output_path: String,
    started_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct RecordingStopOutput {
    recording_id: String,
    path: String,
    duration_secs: f64,
    file_size_bytes: u64,
    format: String,
}

/// How the screenshot tool should return its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotReturnMode {
    /// Return metadata + file path only.
    Path,
    /// Return metadata + file path + a bounded inline base64 payload.
    InlineBase64,
}

/// Options for bounded inline screenshot payloads.
#[derive(Debug, Clone)]
pub struct ScreenshotInlineOptions {
    /// Max width for an inline thumbnail (pixels). If `None`, no resize is attempted.
    pub max_width: Option<u32>,
    /// Max height for an inline thumbnail (pixels). If `None`, no resize is attempted.
    pub max_height: Option<u32>,
    /// Maximum base64 character length for the inline payload.
    pub max_base64_chars: usize,
    /// Maximum serialized JSON length for the tool output (to avoid pipeline truncation).
    pub max_result_chars: usize,
}

impl Default for ScreenshotInlineOptions {
    fn default() -> Self {
        Self {
            max_width: Some(128),
            max_height: Some(128),
            max_base64_chars: 1400,
            max_result_chars: 1800,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScreenshotReturnOptions {
    pub mode: ScreenshotReturnMode,
    pub inline: ScreenshotInlineOptions,
}

impl Default for ScreenshotReturnOptions {
    fn default() -> Self {
        Self {
            mode: ScreenshotReturnMode::Path,
            inline: ScreenshotInlineOptions::default(),
        }
    }
}

fn mime_type_for_ext(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png".to_string(),
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "gif" => "image/gif".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

fn screenshot_output_json(
    result: ScreenshotResult,
    options: ScreenshotReturnOptions,
) -> Result<String> {
    let path = result.path.clone();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_string();
    let mime_type = mime_type_for_ext(&ext);

    let mut out = ScreenshotOutput {
        path: result.path.display().to_string(),
        width: result.width,
        height: result.height,
        format: result.format,
        mime_type,
        timestamp: result.timestamp,
        file_size_bytes: result.file_size_bytes,
        inline_base64: None,
        inline_mime_type: None,
        inline_kind: None,
    };

    if options.mode == ScreenshotReturnMode::InlineBase64 {
        let (b64, inline_mime, kind) = encode_inline_screenshot(&path, &options.inline)?;
        out.inline_base64 = Some(b64);
        out.inline_mime_type = Some(inline_mime);
        out.inline_kind = Some(kind);

        let max_result_chars = options.inline.max_result_chars.min(2000);
        let json = serde_json::to_string(&out)?;
        if json.len() > max_result_chars {
            return Err(AppError::Io(std::io::Error::other(format!(
                "Inline screenshot tool output is too large ({} chars; max {}). Reduce inline max_width/max_height/max_base64_chars.",
                json.len(),
                max_result_chars
            ))));
        }
        return Ok(json);
    }

    Ok(serde_json::to_string_pretty(&out)?)
}

fn encode_inline_screenshot(
    path: &Path,
    inline: &ScreenshotInlineOptions,
) -> Result<(String, String, String)> {
    const HARD_MAX_BASE64_CHARS: usize = 1700;
    const HARD_MAX_RESULT_CHARS: usize = 2000;
    /// Minimum dimension we'll shrink to before giving up.
    const MIN_THUMB_DIM: u32 = 16;

    let max_base64_chars = inline.max_base64_chars.min(HARD_MAX_BASE64_CHARS);
    let max_result_chars = inline.max_result_chars.min(HARD_MAX_RESULT_CHARS);
    if max_base64_chars < 64 {
        return Err(AppError::Io(std::io::Error::other(
            "inline.max_base64_chars must be >= 64",
        )));
    }
    if max_result_chars < 256 {
        return Err(AppError::Io(std::io::Error::other(
            "inline.max_result_chars must be >= 256",
        )));
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let ext_lc = ext.to_ascii_lowercase();

    // For image formats we can decode, iteratively resize + encode as JPEG
    // (much smaller than PNG for photographic content like screenshots) until
    // the base64 payload fits within the budget.
    if matches!(ext_lc.as_str(), "png" | "jpg" | "jpeg")
        && let Ok(img) = image::open(path)
    {
        let mut w = inline.max_width.unwrap_or(img.width());
        let mut h = inline.max_height.unwrap_or(img.height());

        loop {
            let thumb = img.thumbnail(w, h);

            // Encode as JPEG at quality 60 – much more compact than PNG for
            // real-world screenshots.
            let mut buf = Vec::new();
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 60);
            thumb.write_with_encoder(encoder).map_err(|e| {
                AppError::Io(std::io::Error::other(format!(
                    "Failed to encode inline JPEG thumbnail: {e}"
                )))
            })?;

            let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
            if b64.len() <= max_base64_chars {
                return Ok((b64, "image/jpeg".to_string(), "thumbnail_jpeg".to_string()));
            }

            // Halve dimensions and retry.
            w = (w / 2).max(MIN_THUMB_DIM);
            h = (h / 2).max(MIN_THUMB_DIM);

            if w <= MIN_THUMB_DIM && h <= MIN_THUMB_DIM {
                // Even at minimum size it doesn't fit – give up.
                return Err(AppError::Io(std::io::Error::other(format!(
                    "Inline base64 thumbnail too large even at {}×{} ({} chars; max {}). \
                     Increase max_base64_chars or use return.mode='path'.",
                    w,
                    h,
                    b64.len(),
                    max_base64_chars
                ))));
            }
        }
    }

    // Fallback: base64 the raw file bytes (no resize).
    let bytes = std::fs::read(path)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    if b64.len() > max_base64_chars {
        return Err(AppError::Io(std::io::Error::other(format!(
            "Inline base64 file payload too large ({} chars; max {}). \
             Use a smaller capture region, save as PNG/JPG, or use return.mode='path'.",
            b64.len(),
            max_base64_chars
        ))));
    }

    let mime = mime_type_for_ext(ext);
    Ok((b64, mime, "raw_file".to_string()))
}

/// Capture a screenshot asynchronously.
pub async fn screenshot(
    output_path: &str,
    region: Option<(u32, u32, u32, u32)>,
    display: Option<u32>,
) -> Result<String> {
    screenshot_with_options(
        output_path,
        region,
        display,
        ScreenshotReturnOptions::default(),
    )
    .await
}

/// Capture a screenshot asynchronously with configurable return options.
pub async fn screenshot_with_options(
    output_path: &str,
    region: Option<(u32, u32, u32, u32)>,
    display: Option<u32>,
    options: ScreenshotReturnOptions,
) -> Result<String> {
    let path = output_path.to_string();
    let region_opt = region.map(|(x, y, w, h)| CaptureRegion {
        x,
        y,
        width: w,
        height: h,
    });

    tokio::task::spawn_blocking(move || {
        let tools = ScreenTools::new();
        let result = tools.screenshot(std::path::Path::new(&path), region_opt, display)?;

        screenshot_output_json(result, options)
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}

/// Start screen recording asynchronously.
pub async fn start_recording(
    output_path: &str,
    region: Option<(u32, u32, u32, u32)>,
    display: Option<u32>,
) -> Result<String> {
    let path = output_path.to_string();
    let region_opt = region.map(|(x, y, w, h)| CaptureRegion {
        x,
        y,
        width: w,
        height: h,
    });

    tokio::task::spawn_blocking(move || {
        let tools = ScreenTools::new();
        let result = tools.start_recording(std::path::Path::new(&path), region_opt, display)?;

        let output = RecordingStartOutput {
            recording_id: result.recording_id,
            output_path: result.output_path.display().to_string(),
            started_at: result.started_at,
        };

        Ok(serde_json::to_string_pretty(&output)?)
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}

/// Stop screen recording asynchronously.
pub async fn stop_recording(recording_id: &str) -> Result<String> {
    let id = recording_id.to_string();

    tokio::task::spawn_blocking(move || {
        let tools = ScreenTools::new();
        let result = tools.stop_recording(&id)?;

        let output = RecordingStopOutput {
            recording_id: result.recording_id,
            path: result.path.display().to_string(),
            duration_secs: result.duration_secs,
            file_size_bytes: result.file_size_bytes,
            format: result.format,
        };

        Ok(serde_json::to_string_pretty(&output)?)
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(format!("Task join error: {}", e))))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_jpeg_thumbnail_is_bounded_and_decodable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.png");

        // Create a tiny PNG source image.
        let img = image::RgbaImage::from_fn(32, 32, |x, y| {
            image::Rgba([(x % 255) as u8, (y % 255) as u8, 0, 255])
        });
        img.save_with_format(&path, image::ImageFormat::Png)
            .unwrap();

        let opts = ScreenshotInlineOptions {
            max_width: Some(16),
            max_height: Some(16),
            max_base64_chars: 1700,
            max_result_chars: 2000,
        };
        let (b64, mime, kind) = encode_inline_screenshot(&path, &opts).unwrap();
        // Now encodes as JPEG for compact inline thumbnails.
        assert_eq!(mime, "image/jpeg");
        assert_eq!(kind, "thumbnail_jpeg");
        assert!(b64.len() <= opts.max_base64_chars);

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        // JPEG SOI marker
        assert!(bytes.starts_with(&[0xFF, 0xD8]));
    }

    #[test]
    fn inline_payload_too_small_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.png");
        let img = image::RgbaImage::from_fn(8, 8, |_x, _y| image::Rgba([0, 0, 0, 255]));
        img.save_with_format(&path, image::ImageFormat::Png)
            .unwrap();

        let opts = ScreenshotInlineOptions {
            max_width: Some(8),
            max_height: Some(8),
            max_base64_chars: 64,
            max_result_chars: 2000,
        };
        // Force failure with an absurdly low limit.
        let opts = ScreenshotInlineOptions {
            max_base64_chars: 10,
            ..opts
        };
        assert!(encode_inline_screenshot(&path, &opts).is_err());
    }
}
