//! Code analysis and navigation tool
//!
//! Provides code analysis operations with structured output.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Code symbol information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

/// Kind of code symbol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
    Const,
    Static,
    Type,
    Macro,
    Unknown,
}

/// Code statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeStats {
    pub total_files: usize,
    pub total_lines: usize,
    pub code_lines: usize,
    pub comment_lines: usize,
    pub blank_lines: usize,
    pub by_language: HashMap<String, LanguageStats>,
}

/// Statistics for a specific language
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LanguageStats {
    pub files: usize,
    pub lines: usize,
    pub code_lines: usize,
}

/// Dependency information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub version: String,
    pub source: String,
}

/// Lint result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintResult {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub level: LintLevel,
    pub message: String,
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LintLevel {
    Error,
    Warning,
    Info,
    Hint,
}

/// Test result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub duration_ms: u64,
    pub output: Option<String>,
}

/// Code analysis service
pub struct CodeTools {
    #[allow(dead_code)]
    work_dir: Option<PathBuf>,
}

impl Default for CodeTools {
    fn default() -> Self {
        Self::new(None)
    }
}

impl CodeTools {
    pub fn new(work_dir: Option<PathBuf>) -> Self {
        Self { work_dir }
    }

    /// Get code statistics for a directory
    pub fn stats(&self, path: &Path) -> Result<CodeStats> {
        let mut stats = CodeStats {
            total_files: 0,
            total_lines: 0,
            code_lines: 0,
            comment_lines: 0,
            blank_lines: 0,
            by_language: HashMap::new(),
        };

        self.collect_stats(path, &mut stats)?;
        Ok(stats)
    }

    fn collect_stats(&self, path: &Path, stats: &mut CodeStats) -> Result<()> {
        if path.is_file() {
            self.analyze_file(path, stats)?;
        } else if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let entry_path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();

                // Skip hidden and common non-source directories
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }

                self.collect_stats(&entry_path, stats)?;
            }
        }
        Ok(())
    }

    fn analyze_file(&self, path: &Path, stats: &mut CodeStats) -> Result<()> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = match ext {
            "rs" => "Rust",
            "ts" | "tsx" => "TypeScript",
            "js" | "jsx" => "JavaScript",
            "py" => "Python",
            "go" => "Go",
            "java" => "Java",
            "c" | "h" => "C",
            "cpp" | "hpp" | "cc" => "C++",
            "md" => "Markdown",
            "json" => "JSON",
            "toml" => "TOML",
            "yaml" | "yml" => "YAML",
            _ => return Ok(()),
        };

        if let Ok(content) = fs::read_to_string(path) {
            let lines: Vec<&str> = content.lines().collect();
            let total = lines.len();
            let blank = lines.iter().filter(|l| l.trim().is_empty()).count();
            let comments = lines
                .iter()
                .filter(|l| {
                    let t = l.trim();
                    t.starts_with("//")
                        || t.starts_with('#')
                        || t.starts_with("/*")
                        || t.starts_with('*')
                })
                .count();
            let code = total.saturating_sub(blank + comments);

            stats.total_files += 1;
            stats.total_lines += total;
            stats.blank_lines += blank;
            stats.comment_lines += comments;
            stats.code_lines += code;

            let lang_stats = stats.by_language.entry(lang.to_string()).or_default();
            lang_stats.files += 1;
            lang_stats.lines += total;
            lang_stats.code_lines += code;
        }
        Ok(())
    }
}
