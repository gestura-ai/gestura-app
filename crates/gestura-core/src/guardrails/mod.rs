//! Project guardrails discovery and prompt injection helpers.
//!
//! Gestura supports repository-specific instruction files ("guardrails") to keep the
//! agent aligned with a project's conventions and safety constraints.
//!
//! This module is deliberately conservative:
//! - It only loads guardrails when a request includes an explicit workspace root.
//! - It reads a bounded amount of content and truncates by characters.
//! - Missing/invalid files are treated as "no guardrails" rather than errors.

use std::path::{Path, PathBuf};

use crate::config::ProjectGuardrailsSettings;

/// The source file used for guardrails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardrailsSource {
    /// `.gestura/guardrails` within the workspace.
    DotGesturaGuardrails,
    /// `AGENTS.md` at the workspace root.
    AgentsMd,
}

impl GuardrailsSource {
    /// Get the relative path (within workspace) for this source.
    pub fn relative_path(&self) -> &'static str {
        match self {
            GuardrailsSource::DotGesturaGuardrails => ".gestura/guardrails",
            GuardrailsSource::AgentsMd => "AGENTS.md",
        }
    }
}

/// Loaded project guardrails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectGuardrails {
    /// Which file we loaded.
    pub source: GuardrailsSource,
    /// Absolute path to the loaded file.
    pub path: PathBuf,
    /// Guardrails content (possibly truncated).
    pub content: String,
    /// Whether the content was truncated.
    pub truncated: bool,
}

/// Load project guardrails for the given workspace.
///
/// Precedence:
/// 1. `.gestura/guardrails`
/// 2. `AGENTS.md`
///
/// Returns `None` when guardrails are disabled, when `workspace_dir` is missing,
/// when no guardrails file is found, or when a read error occurs.
pub fn load_project_guardrails(
    workspace_dir: &Path,
    settings: &ProjectGuardrailsSettings,
) -> Option<ProjectGuardrails> {
    if !settings.enabled {
        return None;
    }

    let candidates = [
        GuardrailsSource::DotGesturaGuardrails,
        GuardrailsSource::AgentsMd,
    ];

    for source in candidates {
        let path = workspace_dir.join(source.relative_path());
        if !path.is_file() {
            continue;
        }

        match read_guardrails_file_limited(&path, settings.max_chars) {
            Ok((content, truncated)) => {
                // Empty content is treated as absent to avoid adding noise.
                if content.trim().is_empty() {
                    return None;
                }

                return Some(ProjectGuardrails {
                    source,
                    path,
                    content,
                    truncated,
                });
            }
            Err(err) => {
                tracing::debug!(path = %path.display(), error = %err, "failed to read guardrails file");
                return None;
            }
        }
    }

    None
}

/// Read a text file and return at most `max_chars` characters.
///
/// This uses a conservative byte cap to avoid loading arbitrarily large files.
fn read_guardrails_file_limited(path: &Path, max_chars: usize) -> std::io::Result<(String, bool)> {
    use std::io::Read;

    let max_bytes = max_chars
        .saturating_mul(4)
        .min(512 * 1024) // 512KiB hard cap
        .max(1);

    let f = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    f.take(max_bytes as u64).read_to_end(&mut buf)?;

    let text = String::from_utf8_lossy(&buf).to_string();

    let mut iter = text.chars();
    let content: String = iter.by_ref().take(max_chars).collect();
    let truncated = iter.next().is_some();

    Ok((content, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    #[test]
    fn loads_dot_gestura_guardrails_over_agents_md() {
        let temp = tempdir().unwrap();
        std::fs::write(temp.path().join("AGENTS.md"), "agents\n").unwrap();
        std::fs::create_dir_all(temp.path().join(".gestura")).unwrap();
        std::fs::write(temp.path().join(".gestura/guardrails"), "guardrails\n").unwrap();

        let settings = ProjectGuardrailsSettings::default();
        let loaded = load_project_guardrails(temp.path(), &settings).expect("should load");

        assert_eq!(loaded.source, GuardrailsSource::DotGesturaGuardrails);
        assert!(loaded.content.contains("guardrails"));
    }

    #[test]
    fn truncates_by_chars() {
        let temp = tempdir().unwrap();
        std::fs::write(temp.path().join("AGENTS.md"), "abcdefghij").unwrap();

        let settings = ProjectGuardrailsSettings {
            enabled: true,
            max_chars: 5,
        };
        let loaded = load_project_guardrails(temp.path(), &settings).expect("should load");

        assert_eq!(loaded.source, GuardrailsSource::AgentsMd);
        assert_eq!(loaded.content, "abcde");
        assert!(loaded.truncated);
    }
}
