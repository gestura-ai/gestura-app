//! Code analysis tool
//!
//! Provides code analysis operations:
//! - map: Generate repository map
//! - symbols: List symbols in file
//! - references: Find references to symbol
//! - definition: Find symbol definition
//! - lint: Run linter
//! - test: Run tests
//! - deps: Show dependencies
//! - stats: Show code statistics

use super::super::Result;
use colored::Colorize;
use gestura_core::error::AppError;
use gestura_core::tools::code::CodeTools;
use gestura_core::tools::code::SymbolKind;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Global code tools instance
static CODE_TOOLS: OnceLock<CodeTools> = OnceLock::new();

/// Get the global [`CodeTools`] instance.
fn get_code_tools() -> &'static CodeTools {
    CODE_TOOLS.get_or_init(CodeTools::default)
}

/// Code subcommand options
pub enum CodeSubcommand {
    Map {
        path: Option<PathBuf>,
        depth: Option<usize>,
    },
    Symbols {
        path: PathBuf,
    },
    References {
        symbol: String,
        path: Option<PathBuf>,
    },
    Definition {
        symbol: String,
        path: Option<PathBuf>,
    },
    Lint {
        path: Option<PathBuf>,
        fix: bool,
    },
    Test {
        path: Option<PathBuf>,
        filter: Option<String>,
    },
    Deps {
        path: Option<PathBuf>,
    },
    Stats {
        path: Option<PathBuf>,
    },
}

/// Run code subcommand
pub fn run(cmd: CodeSubcommand) -> Result<()> {
    match cmd {
        CodeSubcommand::Map { path, depth } => run_map(path.as_deref(), depth),
        CodeSubcommand::Symbols { path } => run_symbols(&path),
        CodeSubcommand::References { symbol, path } => run_references(&symbol, path.as_deref()),
        CodeSubcommand::Definition { symbol, path } => run_definition(&symbol, path.as_deref()),
        CodeSubcommand::Lint { path, fix } => run_lint(path.as_deref(), fix),
        CodeSubcommand::Test { path, filter } => run_test(path.as_deref(), filter.as_deref()),
        CodeSubcommand::Deps { path } => run_deps(path.as_deref()),
        CodeSubcommand::Stats { path } => run_stats(path.as_deref()),
    }
}

/// Render a lightweight repository "map" (file type histogram + key files).
///
/// Business logic is owned by `gestura-core`; CLI is responsible only for formatting.
fn run_map(path: Option<&Path>, depth: Option<usize>) -> Result<()> {
    let root = path.unwrap_or(Path::new("."));
    let max_depth = depth.unwrap_or(2);

    println!("{}", "Repository Map".bold().underline());
    println!();

    let map = get_code_tools().repository_map(root, max_depth)?;

    // Sort by count
    let mut sorted: Vec<_> = map.file_types.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));

    println!("{}", "File Types:".dimmed());
    let max = sorted.first().map(|x| x.1).unwrap_or(1).max(1);
    for (ext, count) in sorted.iter().take(10) {
        let bar_len = (*count as f64 / max as f64 * 20.0) as usize;
        let bar = "█".repeat(bar_len);
        println!("  {:>8} {:>4} {}", ext.cyan(), count, bar.green());
    }

    println!();
    println!("{}", "Key Files:".dimmed());

    for file in map.key_files_found {
        println!("  {} {}", "✓".green(), file.cyan());
    }

    Ok(())
}

/// List top-level symbols for a file.
///
/// Extraction is performed by `gestura-core`; CLI is responsible only for formatting.
fn run_symbols(path: &Path) -> Result<()> {
    println!(
        "{} {}",
        "Symbols in".bold(),
        path.display().to_string().cyan()
    );
    println!();

    let syms = get_code_tools().symbols(path)?;

    println!("{}", "Functions:".dimmed());
    for s in syms
        .iter()
        .filter(|s| matches!(s.kind, SymbolKind::Function))
    {
        println!("  {} {}", "fn".blue(), s.name);
    }

    println!();
    println!("{}", "Structs:".dimmed());
    for s in syms.iter().filter(|s| matches!(s.kind, SymbolKind::Struct)) {
        println!("  {} {}", "struct".yellow(), s.name);
    }

    println!();
    println!("{}", "Enums:".dimmed());
    for s in syms.iter().filter(|s| matches!(s.kind, SymbolKind::Enum)) {
        println!("  {} {}", "enum".magenta(), s.name);
    }

    println!();
    println!("{}", "Implementations:".dimmed());
    for s in syms.iter().filter(|s| matches!(s.kind, SymbolKind::Impl)) {
        println!("  {} {}", "impl".cyan(), s.name);
    }

    Ok(())
}

/// Find line-level references to a symbol under a directory tree.
///
/// Search logic is performed by `gestura-core`; CLI is responsible only for formatting.
fn run_references(symbol: &str, path: Option<&Path>) -> Result<()> {
    let search_path = path.unwrap_or(Path::new("."));

    println!(
        "{} {} in {}",
        "References to".bold(),
        symbol.cyan(),
        search_path.display()
    );
    println!("{}", "─".repeat(60).dimmed());

    let hits = get_code_tools().references(symbol, search_path)?;
    for h in &hits {
        println!(
            "{}:{}:{}",
            h.path.display().to_string().cyan(),
            h.line.to_string().yellow(),
            h.content.trim()
        );
    }

    println!("{}", "─".repeat(60).dimmed());
    println!("{} {} references found", "Total:".bold(), hits.len());

    Ok(())
}

/// Find the first definition of a symbol under a directory tree.
///
/// Definition search is performed by `gestura-core`; CLI is responsible only for formatting.
fn run_definition(symbol: &str, path: Option<&Path>) -> Result<()> {
    let search_path = path.unwrap_or(Path::new("."));

    println!("{} {}", "Definition of".bold(), symbol.cyan());
    println!();

    match get_code_tools().definition(symbol, search_path)? {
        Some(hit) => {
            println!(
                "  {} {}:{}",
                "Found:".green(),
                hit.path.display().to_string().cyan(),
                hit.line
            );
            println!("  {}", hit.content.trim());
        }
        None => {
            println!("  {}", "Definition not found".red());
        }
    }

    Ok(())
}

/// Run lints for the current workspace.
///
/// Lint execution is owned by `gestura-core`; CLI is responsible only for formatting.
fn run_lint(path: Option<&Path>, fix: bool) -> Result<()> {
    println!("{}", "Running linter...".bold());

    let root = path.unwrap_or(Path::new("."));
    let result = get_code_tools().cargo_clippy(root, fix)?;

    if !result.stdout.is_empty() {
        print!("{}", result.stdout);
    }
    if !result.stderr.is_empty() {
        eprint!("{}", result.stderr);
    }

    Ok(())
}

/// Run tests for the current workspace.
///
/// Test execution is owned by `gestura-core`; CLI is responsible only for formatting.
fn run_test(path: Option<&Path>, filter: Option<&str>) -> Result<()> {
    println!("{}", "Running tests...".bold());

    let root = path.unwrap_or(Path::new("."));
    let result = get_code_tools().cargo_test(root, filter)?;

    if !result.stdout.is_empty() {
        print!("{}", result.stdout);
    }
    if !result.stderr.is_empty() {
        eprint!("{}", result.stderr);
    }

    Ok(())
}

/// Print dependency sections from a local Cargo.toml (best-effort).
///
/// Dependency parsing is owned by `gestura-core`; CLI is responsible only for formatting.
fn run_deps(path: Option<&Path>) -> Result<()> {
    println!("{}", "Dependencies".bold().underline());
    println!();

    let root = path.unwrap_or(Path::new("."));
    match get_code_tools().cargo_dependencies(root) {
        Ok(groups) => {
            for group in groups {
                println!("{}", format!("[{}]", group.section).cyan());

                for dep in group.dependencies {
                    print!("  {}", dep.name.cyan());
                    if !dep.version.is_empty() {
                        print!(" = {}", dep.version);
                    }
                    if dep.source != "crates.io" && dep.source != "unknown" {
                        print!(" {}", format!("({})", dep.source).dimmed());
                    }
                    println!();
                }

                println!();
            }
        }
        Err(AppError::NotFound(_)) => {
            println!("{}", "No Cargo.toml found".yellow());
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}

/// Compute and display repository statistics.
///
/// Statistics are computed by `gestura-core`; CLI is responsible only for formatting.
fn run_stats(path: Option<&Path>) -> Result<()> {
    let root = path.unwrap_or(Path::new("."));

    println!("{}", "Code Statistics".bold().underline());
    println!();

    let stats = get_code_tools().stats(root)?;

    println!("  {} files", stats.total_files.to_string().cyan());
    println!("  {} total lines", stats.total_lines.to_string().cyan());
    println!("  {} code lines", stats.code_lines.to_string().cyan());
    println!(
        "  {} comment lines",
        stats.comment_lines.to_string().dimmed()
    );
    println!("  {} blank lines", stats.blank_lines.to_string().dimmed());
    println!();

    let mut sorted: Vec<_> = stats.by_language.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1.lines));

    println!("{}", "By language:".dimmed());
    for (lang, lang_stats) in sorted.iter().take(10) {
        println!(
            "  {:>12} {:>5} files {:>8} lines ({} code)",
            lang.cyan(),
            lang_stats.files,
            lang_stats.lines,
            lang_stats.code_lines
        );
    }

    Ok(())
}
