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
