//! Code analysis and navigation tool
//!
//! Provides code analysis operations with structured output.

use crate::error::{AppError, Result};
use crate::shell::CommandResult;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Instant;
use toml::Value;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// Repository map output.
///
/// This is intended to be presentation-agnostic. Callers (CLI/GUI) can render
/// the information however they like.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryMap {
    /// Root that was analyzed.
    pub root: PathBuf,
    /// Maximum directory depth included in the map.
    pub max_depth: usize,
    /// File extension -> count.
    ///
    /// Files without an extension use the key `(none)`.
    pub file_types: HashMap<String, usize>,
    /// Common "key" files found at the root.
    pub key_files_found: Vec<String>,
}

/// A single reference hit (line-level match).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceHit {
    pub path: PathBuf,
    pub line: usize,
    pub content: String,
}

/// A single definition hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinitionHit {
    pub kind: SymbolKind,
    pub name: String,
    pub path: PathBuf,
    pub line: usize,
    pub content: String,
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

/// A group of dependencies from a single manifest section (e.g. `[dependencies]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyGroup {
    /// Section name as it appears in the manifest (e.g. `dependencies`).
    pub section: String,
    /// Dependencies listed under that section.
    pub dependencies: Vec<Dependency>,
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

/// A single file match returned by a glob search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobMatch {
    /// Absolute path to the matched file.
    pub path: PathBuf,
    /// Path relative to the search root, using forward slashes.
    pub relative_path: String,
}

/// A single line match returned by a grep search, with optional surrounding context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrepMatch {
    /// File the match was found in.
    pub path: PathBuf,
    /// 1-based line number of the matching line.
    pub line: usize,
    /// The matching line content.
    pub content: String,
    /// Lines before the match: `(1-based line number, line content)`.
    pub context_before: Vec<(usize, String)>,
    /// Lines after the match: `(1-based line number, line content)`.
    pub context_after: Vec<(usize, String)>,
}

/// A single entry in a [`CodeTools::batch_read`] response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchReadEntry {
    /// The path that was requested.
    pub path: String,
    /// File content, or `None` if the read failed.
    pub content: Option<String>,
    /// Number of lines in the file (0 on error).
    pub line_count: usize,
    /// Error message if the read failed.
    pub error: Option<String>,
}

/// A single str-replace edit operation for [`CodeTools::batch_edit`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditOp {
    /// Path of the file to edit.
    pub path: String,
    /// Exact string to find.
    pub old_str: String,
    /// Replacement string.
    pub new_str: String,
}

/// Result of applying a single [`EditOp`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditOpResult {
    /// The path that was edited.
    pub path: String,
    /// Whether the operation succeeded.
    pub success: bool,
    /// Number of replacements made (0 when `success` is false).
    pub replacements: usize,
    /// Error message when `success` is false.
    pub error: Option<String>,
}

/// A lightweight symbol entry for file outlines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineNode {
    pub name: String,
    pub kind: SymbolKind,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column.
    pub column: usize,
}

/// Code analysis service
pub struct CodeTools {
    /// Working directory for relative path resolution.
    /// Used to resolve relative paths in code analysis operations.
    work_dir: Option<PathBuf>,
}

impl Default for CodeTools {
    fn default() -> Self {
        Self::new(None)
    }
}

impl CodeTools {
    /// Create a new [`CodeTools`].
    ///
    /// If `work_dir` is set, relative paths passed to methods will be resolved
    /// against it.
    pub fn new(work_dir: Option<PathBuf>) -> Self {
        Self { work_dir }
    }

    /// Resolve a path, making it absolute if relative and work_dir is set.
    /// Returns the original path if it's already absolute or no work_dir is configured.
    pub fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(ref work_dir) = self.work_dir {
            work_dir.join(path)
        } else {
            path.to_path_buf()
        }
    }

    /// Get the configured working directory
    pub fn work_dir(&self) -> Option<&Path> {
        self.work_dir.as_deref()
    }

    /// Get code statistics for a directory.
    /// Resolves relative paths using the configured work_dir.
    pub fn stats(&self, path: &Path) -> Result<CodeStats> {
        let resolved_path = self.resolve_path(path);
        let mut stats = CodeStats {
            total_files: 0,
            total_lines: 0,
            code_lines: 0,
            comment_lines: 0,
            blank_lines: 0,
            by_language: HashMap::new(),
        };

        self.collect_stats(&resolved_path, &mut stats)?;
        Ok(stats)
    }

    /// Generate a repository map for `root` up to `max_depth`.
    ///
    /// Hidden entries and common non-source directories (`target`, `node_modules`)
    /// are skipped.
    pub fn repository_map(&self, root: &Path, max_depth: usize) -> Result<RepositoryMap> {
        let resolved_root = self.resolve_path(root);
        let mut file_types: HashMap<String, usize> = HashMap::new();
        Self::count_files_by_extension(&resolved_root, &mut file_types, max_depth, 0)?;

        let key_files = [
            "README.md",
            "Cargo.toml",
            "package.json",
            "pyproject.toml",
            "Makefile",
            "Justfile",
            ".gitignore",
            "LICENSE",
        ];

        let mut key_files_found = Vec::new();
        for file in key_files {
            let file_path = resolved_root.join(file);
            if file_path.exists() {
                key_files_found.push(file.to_string());
            }
        }

        Ok(RepositoryMap {
            root: resolved_root,
            max_depth,
            file_types,
            key_files_found,
        })
    }

    /// Extract top-level Rust-like symbols from a single file.
    ///
    /// This is a lightweight, regex-based approach meant for quick inspection.
    /// It is not a full parser.
    pub fn symbols(&self, path: &Path) -> Result<Vec<Symbol>> {
        let path = self.resolve_path(path);
        let content = fs::read_to_string(&path)?;

        let mut out = Vec::new();
        for (kind, re) in symbol_patterns().iter() {
            for cap in re.captures_iter(&content) {
                let name = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
                if name.is_empty() {
                    continue;
                }

                // Determine line/column based on the match start.
                let start = cap.get(1).map(|m| m.start()).unwrap_or(0);
                let prefix = &content[..start];
                let line = prefix.lines().count().max(1);
                let col = prefix
                    .lines()
                    .last()
                    .map(|l| l.chars().count() + 1)
                    .unwrap_or(1);

                out.push(Symbol {
                    name,
                    kind: *kind,
                    path: path.clone(),
                    line,
                    column: col,
                });
            }
        }

        Ok(out)
    }

    /// Find references to `symbol` under `root`.
    ///
    /// This performs a simple word-boundary search (`\bSYMBOL\b`) and returns
    /// line-level hits.
    pub fn references(&self, symbol: &str, root: &Path) -> Result<Vec<ReferenceHit>> {
        let root = self.resolve_path(root);
        let pattern = format!(r"\b{}\b", regex::escape(symbol));
        let re = Regex::new(&pattern).map_err(|e| {
            crate::error::AppError::InvalidInput(format!("Invalid symbol regex: {e}"))
        })?;

        let mut hits = Vec::new();
        Self::search_references(&root, &re, &mut hits)?;
        Ok(hits)
    }

    /// Find the first definition of `symbol` under `root`.
    ///
    /// The search is regex-based (functions/structs/enums/types/consts). If multiple
    /// definitions exist, the first encountered in directory traversal order is returned.
    pub fn definition(&self, symbol: &str, root: &Path) -> Result<Option<DefinitionHit>> {
        let root = self.resolve_path(root);

        let patterns: Vec<(SymbolKind, Regex)> = vec![
            (
                SymbolKind::Function,
                Regex::new(&format!(
                    r"(?m)^(?:pub\s+)?(?:async\s+)?fn\s+{}\s*[<(]",
                    regex::escape(symbol)
                ))
                .map_err(|e| {
                    crate::error::AppError::InvalidInput(format!("Invalid definition regex: {e}"))
                })?,
            ),
            (
                SymbolKind::Struct,
                Regex::new(&format!(
                    r"(?m)^(?:pub\s+)?struct\s+{}\s*[<{{]",
                    regex::escape(symbol)
                ))
                .map_err(|e| {
                    crate::error::AppError::InvalidInput(format!("Invalid definition regex: {e}"))
                })?,
            ),
            (
                SymbolKind::Enum,
                Regex::new(&format!(
                    r"(?m)^(?:pub\s+)?enum\s+{}\s*[<{{]",
                    regex::escape(symbol)
                ))
                .map_err(|e| {
                    crate::error::AppError::InvalidInput(format!("Invalid definition regex: {e}"))
                })?,
            ),
            (
                SymbolKind::Type,
                Regex::new(&format!(
                    r"(?m)^(?:pub\s+)?type\s+{}\s*=",
                    regex::escape(symbol)
                ))
                .map_err(|e| {
                    crate::error::AppError::InvalidInput(format!("Invalid definition regex: {e}"))
                })?,
            ),
            (
                SymbolKind::Const,
                Regex::new(&format!(
                    r"(?m)^(?:pub\s+)?const\s+{}\s*:",
                    regex::escape(symbol)
                ))
                .map_err(|e| {
                    crate::error::AppError::InvalidInput(format!("Invalid definition regex: {e}"))
                })?,
            ),
        ];

        Self::find_definition(&root, symbol, &patterns)
    }

    /// Parse Rust/Cargo dependencies from a `Cargo.toml` at `root`.
    ///
    /// If `root` is a directory, this looks for `root/Cargo.toml`.
    /// If `root` is a file, it is treated as the manifest.
    pub fn cargo_dependencies(&self, root: &Path) -> Result<Vec<DependencyGroup>> {
        let root = self.resolve_path(root);
        let manifest_path = if root.is_dir() {
            root.join("Cargo.toml")
        } else {
            root.clone()
        };
        if !manifest_path.exists() {
            return Err(AppError::NotFound(format!(
                "Cargo.toml not found at {}",
                manifest_path.display()
            )));
        }

        let content = fs::read_to_string(&manifest_path)?;
        let parsed: Value = content.parse()?;
        let Some(table) = parsed.as_table() else {
            return Err(AppError::InvalidInput(
                "Cargo.toml is not a table".to_string(),
            ));
        };

        let sections = ["dependencies", "dev-dependencies", "build-dependencies"];
        let mut out = Vec::new();
        for section in sections {
            let Some(deps_table) = table.get(section).and_then(|v| v.as_table()) else {
                continue;
            };

            let mut deps = Vec::new();
            for (name, value) in deps_table {
                let (version, source) = match value {
                    Value::String(v) => (v.clone(), "crates.io".to_string()),
                    Value::Table(t) => {
                        let version = t
                            .get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        let source = if t
                            .get("workspace")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        {
                            "workspace".to_string()
                        } else if let Some(p) = t.get("path").and_then(|v| v.as_str()) {
                            format!("path:{p}")
                        } else if let Some(g) = t.get("git").and_then(|v| v.as_str()) {
                            format!("git:{g}")
                        } else if let Some(r) = t.get("registry").and_then(|v| v.as_str()) {
                            format!("registry:{r}")
                        } else {
                            "crates.io".to_string()
                        };

                        (version, source)
                    }
                    _ => ("".to_string(), "unknown".to_string()),
                };

                deps.push(Dependency {
                    name: name.clone(),
                    version,
                    source,
                });
            }

            deps.sort_by(|a, b| a.name.cmp(&b.name));
            out.push(DependencyGroup {
                section: section.to_string(),
                dependencies: deps,
            });
        }

        Ok(out)
    }

    /// Run `cargo clippy` within `root` and return captured stdout/stderr.
    ///
    /// This is intended for local developer tooling (CLI) and should not be used
    /// for untrusted inputs.
    pub fn cargo_clippy(&self, root: &Path, fix: bool) -> Result<CommandResult> {
        let mut args = vec!["clippy"];
        if fix {
            args.push("--fix");
        }
        self.run_cargo(root, &args)
    }

    /// Run `cargo test` within `root` and return captured stdout/stderr.
    ///
    /// The optional `filter` is passed as the standard cargo test filter argument.
    pub fn cargo_test(&self, root: &Path, filter: Option<&str>) -> Result<CommandResult> {
        let mut args = vec!["test"];
        if let Some(f) = filter {
            args.push(f);
        }
        self.run_cargo(root, &args)
    }

    /// Run a `cargo` subcommand with the given args in `root`.
    fn run_cargo(&self, root: &Path, args: &[&str]) -> Result<CommandResult> {
        let root = self.resolve_path(root);
        let start = Instant::now();

        let output = Command::new("cargo")
            .args(args)
            .current_dir(&root)
            .output()
            .map_err(AppError::Io)?;

        let duration_ms = start.elapsed().as_millis() as u64;
        let exit_code = output.status.code().unwrap_or(-1);

        Ok(CommandResult {
            command: format!("cargo {}", args.join(" ")),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code,
            success: output.status.success(),
            duration_ms,
        })
    }

    /// Find all files whose paths (relative to `root`) match the glob `pattern`.
    ///
    /// Supports `**` (any path depth), `*` (any filename chars), `?` (one char).
    /// Hidden directories and common build directories (`target`, `node_modules`) are skipped.
    pub fn glob_search(
        &self,
        pattern: &str,
        root: &Path,
        max_results: usize,
    ) -> Result<Vec<GlobMatch>> {
        let root = self.resolve_path(root);
        let regex_str = glob_to_regex_string(pattern);
        let re = Regex::new(&regex_str).map_err(|e| {
            AppError::InvalidInput(format!("Invalid glob pattern '{pattern}': {e}"))
        })?;
        let mut out = Vec::new();
        Self::walk_for_glob(&root, &root, &re, max_results, &mut out)?;
        Ok(out)
    }

    /// Search file contents under `root` for lines matching the regex `pattern`.
    ///
    /// - `file_glob`: optional glob to restrict which files are searched (e.g. `*.rs`)
    /// - `context_lines`: number of context lines to include before and after each match
    /// - `case_sensitive`: whether the match is case-sensitive
    /// - `max_matches`: maximum number of [`GrepMatch`] entries to return
    pub fn grep(
        &self,
        pattern: &str,
        root: &Path,
        file_glob: Option<&str>,
        context_lines: usize,
        case_sensitive: bool,
        max_matches: usize,
    ) -> Result<Vec<GrepMatch>> {
        let root = self.resolve_path(root);
        let pattern_str = if case_sensitive {
            pattern.to_string()
        } else {
            format!("(?i){pattern}")
        };
        let re = Regex::new(&pattern_str).map_err(|e| {
            AppError::InvalidInput(format!("Invalid grep pattern '{pattern}': {e}"))
        })?;
        let file_re: Option<Regex> =
            match file_glob {
                Some(g) => {
                    let s = glob_to_regex_string(g);
                    Some(Regex::new(&s).map_err(|e| {
                        AppError::InvalidInput(format!("Invalid file glob '{g}': {e}"))
                    })?)
                }
                None => None,
            };
        let mut out = Vec::new();
        Self::walk_for_grep(
            &root,
            &re,
            file_re.as_ref(),
            context_lines,
            max_matches,
            &mut out,
        )?;
        Ok(out)
    }

    /// Read multiple files in one call.
    ///
    /// Each path is resolved through the configured `work_dir`. Failures are
    /// captured per-entry rather than aborting the batch.
    pub fn batch_read(&self, paths: &[&str]) -> Vec<BatchReadEntry> {
        paths
            .iter()
            .map(|p| {
                let resolved = self.resolve_path(Path::new(p));
                match fs::read_to_string(&resolved) {
                    Ok(content) => {
                        let line_count = content.lines().count();
                        BatchReadEntry {
                            path: p.to_string(),
                            content: Some(content),
                            line_count,
                            error: None,
                        }
                    }
                    Err(e) => BatchReadEntry {
                        path: p.to_string(),
                        content: None,
                        line_count: 0,
                        error: Some(e.to_string()),
                    },
                }
            })
            .collect()
    }

    /// Apply multiple str-replace edits across files in one call.
    ///
    /// Each [`EditOp`] replaces all occurrences of `old_str` with `new_str` in
    /// the target file.  Failures are captured per-entry rather than aborting
    /// the batch, so callers must inspect [`EditOpResult::success`] for each entry.
    pub fn batch_edit(&self, edits: &[EditOp]) -> Vec<EditOpResult> {
        edits
            .iter()
            .map(|op| {
                let resolved = self.resolve_path(Path::new(&op.path));
                match fs::read_to_string(&resolved) {
                    Ok(content) => {
                        let replacements = content.matches(op.old_str.as_str()).count();
                        if replacements == 0 {
                            return EditOpResult {
                                path: op.path.clone(),
                                success: false,
                                replacements: 0,
                                error: Some(format!("old_str not found in '{}'", op.path)),
                            };
                        }
                        let new_content = content.replace(op.old_str.as_str(), op.new_str.as_str());
                        match fs::write(&resolved, new_content) {
                            Ok(()) => EditOpResult {
                                path: op.path.clone(),
                                success: true,
                                replacements,
                                error: None,
                            },
                            Err(e) => EditOpResult {
                                path: op.path.clone(),
                                success: false,
                                replacements: 0,
                                error: Some(e.to_string()),
                            },
                        }
                    }
                    Err(e) => EditOpResult {
                        path: op.path.clone(),
                        success: false,
                        replacements: 0,
                        error: Some(e.to_string()),
                    },
                }
            })
            .collect()
    }

    /// Return a structured outline of all top-level symbols in `path`.
    ///
    /// This is a lightweight wrapper around [`Self::symbols`] that strips the
    /// absolute path from each entry so the result is presentation-friendly.
    pub fn outline(&self, path: &Path) -> Result<Vec<OutlineNode>> {
        let syms = self.symbols(path)?;
        Ok(syms
            .into_iter()
            .map(|s| OutlineNode {
                name: s.name,
                kind: s.kind,
                line: s.line,
                column: s.column,
            })
            .collect())
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

    /// Return `true` if a directory entry name should be skipped during filesystem traversal.
    ///
    /// This filters out hidden entries and common build/dependency directories.
    fn should_skip_name(name: &str) -> bool {
        name.starts_with('.') || name == "target" || name == "node_modules"
    }

    /// Recursively count files by extension under `path` up to `max_depth`.
    ///
    /// This is a traversal helper used by [`Self::repository_map`].
    fn count_files_by_extension(
        path: &Path,
        counts: &mut HashMap<String, usize>,
        max_depth: usize,
        depth: usize,
    ) -> Result<()> {
        if depth > max_depth || !path.is_dir() {
            return Ok(());
        }

        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let entry_path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if Self::should_skip_name(&name) {
                continue;
            }

            if entry_path.is_file() {
                let ext = entry_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("(none)")
                    .to_string();
                *counts.entry(ext).or_insert(0) += 1;
            } else if entry_path.is_dir() {
                Self::count_files_by_extension(&entry_path, counts, max_depth, depth + 1)?;
            }
        }
        Ok(())
    }

    /// Recursively search for `re` under `path` and append line hits into `out`.
    ///
    /// This is a traversal helper used by [`Self::references`].
    fn search_references(path: &Path, re: &Regex, out: &mut Vec<ReferenceHit>) -> Result<()> {
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(path) {
                for (idx, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        out.push(ReferenceHit {
                            path: path.to_path_buf(),
                            line: idx + 1,
                            content: line.trim().to_string(),
                        });
                    }
                }
            }
        } else if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if Self::should_skip_name(&name) {
                    continue;
                }
                Self::search_references(&p, re, out)?;
            }
        }
        Ok(())
    }

    /// Recursively search for the first definition matching `patterns` under `path`.
    ///
    /// This is a traversal helper used by [`Self::definition`].
    fn find_definition(
        path: &Path,
        symbol: &str,
        patterns: &[(SymbolKind, Regex)],
    ) -> Result<Option<DefinitionHit>> {
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(path) {
                for (line_num, line) in content.lines().enumerate() {
                    for (kind, pattern) in patterns {
                        if pattern.is_match(line) {
                            return Ok(Some(DefinitionHit {
                                kind: *kind,
                                name: symbol.to_string(),
                                path: path.to_path_buf(),
                                line: line_num + 1,
                                content: line.to_string(),
                            }));
                        }
                    }
                }
            }
        } else if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if Self::should_skip_name(&name) {
                    continue;
                }
                if let Some(hit) = Self::find_definition(&p, symbol, patterns)? {
                    return Ok(Some(hit));
                }
            }
        }
        Ok(None)
    }

    /// Recursive traversal helper for [`Self::glob_search`].
    fn walk_for_glob(
        root: &Path,
        current: &Path,
        re: &Regex,
        limit: usize,
        out: &mut Vec<GlobMatch>,
    ) -> Result<()> {
        if out.len() >= limit {
            return Ok(());
        }
        if current.is_file() {
            let rel = current.strip_prefix(root).unwrap_or(current);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if re.is_match(&rel_str) {
                out.push(GlobMatch {
                    path: current.to_path_buf(),
                    relative_path: rel_str,
                });
            }
        } else if current.is_dir() {
            let mut entries: Vec<_> = fs::read_dir(current)?.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                if out.len() >= limit {
                    break;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if Self::should_skip_name(&name) {
                    continue;
                }
                Self::walk_for_glob(root, &entry.path(), re, limit, out)?;
            }
        }
        Ok(())
    }

    /// Recursive traversal helper for [`Self::grep`].
    fn walk_for_grep(
        path: &Path,
        re: &Regex,
        file_re: Option<&Regex>,
        context_lines: usize,
        limit: usize,
        out: &mut Vec<GrepMatch>,
    ) -> Result<()> {
        if out.len() >= limit {
            return Ok(());
        }
        if path.is_file() {
            // Filter by file name glob when one is provided.
            if let Some(fre) = file_re {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !fre.is_match(&name) {
                    return Ok(());
                }
            }
            if let Ok(content) = fs::read_to_string(path) {
                let lines: Vec<&str> = content.lines().collect();
                for (idx, line) in lines.iter().enumerate() {
                    if out.len() >= limit {
                        break;
                    }
                    if re.is_match(line) {
                        let ctx_before = (0..context_lines)
                            .filter_map(|d| {
                                let li = idx.checked_sub(context_lines - d)?;
                                Some((li + 1, lines[li].to_string()))
                            })
                            .collect();
                        let ctx_after = (1..=context_lines)
                            .filter_map(|d| {
                                let li = idx + d;
                                if li < lines.len() {
                                    Some((li + 1, lines[li].to_string()))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        out.push(GrepMatch {
                            path: path.to_path_buf(),
                            line: idx + 1,
                            content: line.to_string(),
                            context_before: ctx_before,
                            context_after: ctx_after,
                        });
                    }
                }
            }
        } else if path.is_dir() {
            let mut entries: Vec<_> = fs::read_dir(path)?.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                if out.len() >= limit {
                    break;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if Self::should_skip_name(&name) {
                    continue;
                }
                Self::walk_for_grep(&entry.path(), re, file_re, context_lines, limit, out)?;
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

/// Convert a glob pattern to a regex string suitable for [`Regex::new`].
///
/// Supported glob metacharacters:
/// - `**` followed by `/` — match zero or more path segments (`(?:[^/]+/)* `)
/// - `**` at end — match anything (`.*`)
/// - `*`  — match any sequence of non-separator characters (`[^/]*`)
/// - `?`  — match any single non-separator character (`[^/]`)
///
/// All other regex metacharacters in the pattern are escaped.
fn glob_to_regex_string(pattern: &str) -> String {
    let mut result = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                if i + 2 < chars.len() && chars[i + 2] == '/' {
                    // **/ — zero or more directory segments
                    result.push_str("(?:[^/]+/)*");
                    i += 3;
                } else {
                    // ** at end or without trailing slash — match anything
                    result.push_str(".*");
                    i += 2;
                }
            }
            '*' => {
                result.push_str("[^/]*");
                i += 1;
            }
            '?' => {
                result.push_str("[^/]");
                i += 1;
            }
            c => {
                if ".+^${}()|[]\\".contains(c) {
                    result.push('\\');
                }
                result.push(c);
                i += 1;
            }
        }
    }
    result.push('$');
    result
}

fn symbol_patterns() -> &'static Vec<(SymbolKind, Regex)> {
    static PATTERNS: OnceLock<Vec<(SymbolKind, Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            (
                SymbolKind::Function,
                Regex::new(r"(?m)^(?:pub\s+)?(?:async\s+)?fn\s+(\w+)")
                    .expect("valid function regex"),
            ),
            (
                SymbolKind::Struct,
                Regex::new(r"(?m)^(?:pub\s+)?struct\s+(\w+)").expect("valid struct regex"),
            ),
            (
                SymbolKind::Enum,
                Regex::new(r"(?m)^(?:pub\s+)?enum\s+(\w+)").expect("valid enum regex"),
            ),
            (
                SymbolKind::Impl,
                Regex::new(r"(?m)^impl(?:<[^>]+>)?\s+(\w+)").expect("valid impl regex"),
            ),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn repository_map_respects_depth_and_ignores_common_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(root.join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
        fs::write(root.join("src/nested/mod.rs"), "pub fn b() {}\n").unwrap();

        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("target/ignored.rs"), "pub fn nope() {}\n").unwrap();

        let tools = CodeTools::default();
        let map_depth_1 = tools.repository_map(root, 1).unwrap();
        assert_eq!(map_depth_1.file_types.get("toml").copied().unwrap_or(0), 1);
        assert_eq!(map_depth_1.file_types.get("rs").copied().unwrap_or(0), 1);

        let map_depth_2 = tools.repository_map(root, 2).unwrap();
        assert_eq!(map_depth_2.file_types.get("rs").copied().unwrap_or(0), 2);
        assert_eq!(map_depth_2.file_types.get("toml").copied().unwrap_or(0), 1);
    }

    #[test]
    fn symbols_extracts_basic_rust_like_items() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        fs::write(
            &file,
            "pub async fn foo() {}\nstruct Bar {}\nenum E { A }\nimpl Bar {}\n",
        )
        .unwrap();

        let tools = CodeTools::default();
        let syms = tools.symbols(&file).unwrap();
        let mut names: Vec<_> = syms.iter().map(|s| (s.kind, s.name.clone())).collect();
        names.sort_by(|a, b| a.1.cmp(&b.1));

        assert!(names.contains(&(SymbolKind::Function, "foo".to_string())));
        assert!(names.contains(&(SymbolKind::Struct, "Bar".to_string())));
        assert!(names.contains(&(SymbolKind::Enum, "E".to_string())));
        assert!(names.contains(&(SymbolKind::Impl, "Bar".to_string())));
    }

    #[test]
    fn references_and_definition_work_and_skip_non_utf8_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/main.rs"),
            "fn main() { let _x = MyType; }\n// MyType used here\n",
        )
        .unwrap();

        // Non-UTF8 file should not cause an error for recursive searches.
        let mut f = fs::File::create(root.join("src/binary.bin")).unwrap();
        f.write_all(&[0xff, 0xfe, 0xfd]).unwrap();

        // Ignored directory should be skipped.
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("target/ignored.rs"), "MyType\n").unwrap();

        let tools = CodeTools::default();
        let refs = tools.references("MyType", root).unwrap();
        assert_eq!(refs.len(), 2);
        assert!(
            refs.iter()
                .all(|h| !h.path.to_string_lossy().contains("target"))
        );

        let def = tools.definition("main", root).unwrap();
        assert!(def.is_some());
        let def = def.unwrap();
        assert_eq!(def.kind, SymbolKind::Function);
        assert!(def.path.to_string_lossy().contains("main.rs"));
    }

    #[test]
    fn cargo_dependencies_parses_common_sections() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "x"
version = "0.1.0"

[dependencies]
serde = "1"
tokio = { version = "1", features = ["rt"] }

[dev-dependencies]
tempfile = { workspace = true }
"#,
        )
        .unwrap();

        let tools = CodeTools::default();
        let groups = tools.cargo_dependencies(root).unwrap();
        assert_eq!(groups.len(), 2);
        let deps = groups.iter().find(|g| g.section == "dependencies").unwrap();
        assert!(
            deps.dependencies
                .iter()
                .any(|d| d.name == "serde" && d.version == "1")
        );
        assert!(
            deps.dependencies
                .iter()
                .any(|d| d.name == "tokio" && d.version == "1")
        );

        let dev = groups
            .iter()
            .find(|g| g.section == "dev-dependencies")
            .unwrap();
        assert!(
            dev.dependencies
                .iter()
                .any(|d| d.name == "tempfile" && d.source == "workspace")
        );
    }
}
