//! System Tools for Gestura
//!
//! This module provides output-agnostic system tools that can be used by both
//! the CLI and GUI interfaces. All tools return structured data rather than
//! formatted strings, allowing each interface to present results appropriately.
//!
//! # Tools
//! - [`file`]: File system operations (read, write, edit, search, list, tree)
//! - [`shell`]: Shell command execution
//! - [`git`]: Git repository operations
//! - [`code`]: Code analysis and navigation
//! - [`web`]: Web fetching and search
//! - [`permissions`]: Permission management for tool access
//! - [`registry`]: Tool registry for listing available tools

pub mod code;
pub mod file;
pub mod git;
pub mod permissions;
pub mod policy;
pub mod registry;
pub mod schemas;
pub mod screen;
pub mod shell;
pub mod web;

pub use code::CodeTools;
pub use file::FileTools;
pub use git::GitTools;
pub use permissions::PermissionManager;
pub use registry::{
    ToolDefinition, all_tools, find_tool, looks_like_capabilities_question,
    looks_like_tools_question, render_capabilities, render_tool_detail, render_tools_overview,
};
pub use screen::ScreenTools;
pub use shell::ShellTools;
pub use web::WebTools;

use crate::error::Result;
use std::path::Path;

/// Async shell operations for pipeline integration
pub mod shell_async {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Execute a shell command asynchronously
    pub async fn execute_command(command: &str, cwd: Option<&str>) -> Result<String> {
        execute_command_with_options(command, cwd, None, Some(60)).await
    }

    /// Execute a shell command asynchronously with cwd/env/timeout.
    pub async fn execute_command_with_options(
        command: &str,
        cwd: Option<&str>,
        env: Option<&HashMap<String, String>>,
        timeout_secs: Option<u64>,
    ) -> Result<String> {
        let tools = ShellTools::new();
        let cmd = command.to_string();
        let work_dir = cwd.map(PathBuf::from);
        let env_map = env.cloned();
        let timeout = timeout_secs;

        tokio::task::spawn_blocking(move || {
            tools
                .run_with_options(&cmd, work_dir.as_deref(), env_map.as_ref(), timeout)
                .map(|r| {
                    if r.success {
                        r.stdout
                    } else {
                        format!("Error (exit {}): {}", r.exit_code, r.stderr)
                    }
                })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }
}

/// Async screen capture operations for pipeline
pub mod screen_async {
    use super::*;
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
    pub(crate) enum ScreenshotReturnMode {
        /// Return metadata + file path only.
        Path,
        /// Return metadata + file path + a bounded inline base64 payload.
        InlineBase64,
    }

    /// Options for bounded inline screenshot payloads.
    #[derive(Debug, Clone)]
    pub(crate) struct ScreenshotInlineOptions {
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
                // Start small – a full-screen retina capture at 128×128 JPEG is
                // typically ~2-4 KB base64 which fits comfortably in 1400 chars.
                // The encode function will iteratively halve if it still doesn't fit.
                max_width: Some(128),
                max_height: Some(128),
                // Keep comfortably below pipeline's 2000 char truncation.
                max_base64_chars: 1400,
                max_result_chars: 1800,
            }
        }
    }

    #[derive(Debug, Clone)]
    pub(crate) struct ScreenshotReturnOptions {
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
        result: super::screen::ScreenshotResult,
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

            // Always enforce an upper bound compatible with the pipeline's tool-result truncation.
            let max_result_chars = options.inline.max_result_chars.min(2000);
            let json = serde_json::to_string(&out)?;
            if json.len() > max_result_chars {
                return Err(crate::error::AppError::Io(std::io::Error::other(format!(
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
            return Err(crate::error::AppError::Io(std::io::Error::other(
                "inline.max_base64_chars must be >= 64",
            )));
        }
        if max_result_chars < 256 {
            return Err(crate::error::AppError::Io(std::io::Error::other(
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
                    crate::error::AppError::Io(std::io::Error::other(format!(
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
                    return Err(crate::error::AppError::Io(std::io::Error::other(format!(
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
            return Err(crate::error::AppError::Io(std::io::Error::other(format!(
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
    pub(crate) async fn screenshot_with_options(
        output_path: &str,
        region: Option<(u32, u32, u32, u32)>,
        display: Option<u32>,
        options: ScreenshotReturnOptions,
    ) -> Result<String> {
        let path = output_path.to_string();
        let region_opt = region.map(|(x, y, w, h)| super::screen::CaptureRegion {
            x,
            y,
            width: w,
            height: h,
        });

        tokio::task::spawn_blocking(move || {
            let tools = super::screen::ScreenTools::new();
            let result = tools.screenshot(std::path::Path::new(&path), region_opt, display)?;

            screenshot_output_json(result, options)
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// Start screen recording asynchronously.
    pub async fn start_recording(
        output_path: &str,
        region: Option<(u32, u32, u32, u32)>,
        display: Option<u32>,
    ) -> Result<String> {
        let path = output_path.to_string();
        let region_opt = region.map(|(x, y, w, h)| super::screen::CaptureRegion {
            x,
            y,
            width: w,
            height: h,
        });

        tokio::task::spawn_blocking(move || {
            let tools = super::screen::ScreenTools::new();
            let result = tools.start_recording(std::path::Path::new(&path), region_opt, display)?;

            let output = RecordingStartOutput {
                recording_id: result.recording_id,
                output_path: result.output_path.display().to_string(),
                started_at: result.started_at,
            };

            Ok(serde_json::to_string_pretty(&output)?)
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// Stop screen recording asynchronously.
    pub async fn stop_recording(recording_id: &str) -> Result<String> {
        let id = recording_id.to_string();

        tokio::task::spawn_blocking(move || {
            let tools = super::screen::ScreenTools::new();
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
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
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
            // This may or may not fit depending on PNG compression; force failure with an absurdly low limit.
            let opts = ScreenshotInlineOptions {
                max_base64_chars: 10,
                ..opts
            };
            assert!(encode_inline_screenshot(&path, &opts).is_err());
        }
    }
}

/// Async file operations for pipeline
pub mod file_async {
    use super::*;
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    struct ListDirOutput {
        path: String,
        show_hidden: bool,
        truncated: bool,
        entries: Vec<super::file::FileEntry>,
    }

    #[derive(Debug, Serialize)]
    struct TreeDirOutput {
        path: String,
        max_depth: Option<usize>,
        show_hidden: bool,
        tree: super::file::TreeNode,
    }

    #[derive(Debug, Serialize)]
    struct EditFileOutput {
        path: String,
        replacements: usize,
    }

    #[derive(Debug, Serialize)]
    struct SearchOutput {
        pattern: String,
        path: String,
        recursive: bool,
        truncated: bool,
        matches: Vec<super::file::SearchMatch>,
    }

    /// Read a file asynchronously.
    pub async fn read_file(path: &str) -> Result<String> {
        read_file_range(path, None, None).await
    }

    /// Read a file asynchronously with an optional line range.
    pub async fn read_file_range(
        path: &str,
        start_line: Option<usize>,
        end_line: Option<usize>,
    ) -> Result<String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            tools
                .read(Path::new(&path), start_line, end_line)
                .map(|r| r.content)
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// Write to a file asynchronously
    pub async fn write_file(path: &str, content: &str) -> Result<()> {
        let path = path.to_string();
        let content = content.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            tools.write(Path::new(&path), &content).map(|_| ())
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// Edit a file asynchronously by replacing `old_str` with `new_str`.
    ///
    /// Returns a small JSON payload with the number of replacements.
    pub async fn edit_file(path: &str, old_str: &str, new_str: &str) -> Result<String> {
        let path = path.to_string();
        let old_str = old_str.to_string();
        let new_str = new_str.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            let res = tools.edit(Path::new(&path), &old_str, &new_str)?;

            let out = EditFileOutput {
                path,
                replacements: res.replacements,
            };
            serde_json::to_string_pretty(&out).map_err(|e| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "Failed to serialize edit output: {e}"
                )))
            })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// List directory entries asynchronously.
    ///
    /// Returns a JSON string containing the entries.
    pub async fn list_dir(
        path: &str,
        show_hidden: bool,
        max_entries: Option<usize>,
    ) -> Result<String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            let mut entries = tools.list(Path::new(&path), show_hidden)?;
            let max = max_entries.unwrap_or(200);
            let truncated = entries.len() > max;
            if truncated {
                entries.truncate(max);
            }

            let out = ListDirOutput {
                path,
                show_hidden,
                truncated,
                entries,
            };
            serde_json::to_string_pretty(&out).map_err(|e| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "Failed to serialize list output: {e}"
                )))
            })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// Build a directory tree asynchronously.
    ///
    /// Returns a JSON string containing the tree.
    pub async fn tree_dir(
        path: &str,
        max_depth: Option<usize>,
        show_hidden: bool,
    ) -> Result<String> {
        let path = path.to_string();
        let show_hidden_flag = show_hidden;
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            let tree = tools.tree(Path::new(&path), max_depth, show_hidden_flag)?;
            let out = TreeDirOutput {
                path,
                max_depth,
                show_hidden: show_hidden_flag,
                tree,
            };
            serde_json::to_string_pretty(&out).map_err(|e| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "Failed to serialize tree output: {e}"
                )))
            })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }

    /// Search files for a pattern asynchronously.
    ///
    /// Returns a JSON string containing the matches.
    pub async fn search_files(
        pattern: &str,
        path: &str,
        recursive: bool,
        max_matches: Option<usize>,
    ) -> Result<String> {
        let pattern = pattern.to_string();
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = FileTools::new();
            let mut matches = tools.search(&pattern, Path::new(&path), recursive)?;
            let max = max_matches.unwrap_or(200);
            let truncated = matches.len() > max;
            if truncated {
                matches.truncate(max);
            }

            let out = SearchOutput {
                pattern,
                path,
                recursive,
                truncated,
                matches,
            };
            serde_json::to_string_pretty(&out).map_err(|e| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "Failed to serialize search output: {e}"
                )))
            })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }
}

/// Async code operations for pipeline
pub mod code_async {
    use super::*;
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    struct StatsOutput {
        path: String,
        stats: super::code::CodeStats,
    }

    /// Compute code statistics asynchronously.
    ///
    /// Returns a JSON string with the stats.
    pub async fn stats_dir(path: &str) -> Result<String> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let tools = CodeTools::default();
            let stats = tools.stats(Path::new(&path))?;
            let out = StatsOutput { path, stats };
            serde_json::to_string_pretty(&out).map_err(|e| {
                crate::error::AppError::Io(std::io::Error::other(format!(
                    "Failed to serialize code stats output: {e}"
                )))
            })
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {e}")))
        })?
    }
}

/// Async git operations for pipeline
pub mod git_async {
    use super::*;

    /// Execute a git operation asynchronously
    pub async fn execute_git(operation: &str, path: &str) -> Result<String> {
        let op = operation.to_string();
        let work_path = path.to_string();

        tokio::task::spawn_blocking(move || {
            let tools = GitTools::new(Some(std::path::PathBuf::from(&work_path)));
            match op.as_str() {
                "status" => tools.status().map(|s| {
                    format!(
                        "Branch: {}\nStaged: {} files\nUnstaged: {} files\nUntracked: {} files",
                        s.branch,
                        s.staged.len(),
                        s.unstaged.len(),
                        s.untracked.len()
                    )
                }),
                "log" => tools.log(Some(10), None).map(|commits| {
                    commits
                        .iter()
                        .map(|c| format!("{} - {} ({})", c.short_hash, c.message, c.author))
                        .collect::<Vec<_>>()
                        .join("\n")
                }),
                "diff" => tools.diff(false, None),
                "diff-staged" => tools.diff(true, None),
                "branches" => tools.branches(false).map(|branches| {
                    branches
                        .iter()
                        .map(|b| {
                            if b.is_current {
                                format!("* {}", b.name)
                            } else {
                                format!("  {}", b.name)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }),
                _ => Err(crate::error::AppError::Io(std::io::Error::other(format!(
                    "Unknown git operation: {}",
                    op
                )))),
            }
        })
        .await
        .map_err(|e| {
            crate::error::AppError::Io(std::io::Error::other(format!("Task join error: {}", e)))
        })?
    }
}
