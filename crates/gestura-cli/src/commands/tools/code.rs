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
use gestura_core::tools::code::CodeTools;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Global code tools instance
static CODE_TOOLS: OnceLock<CodeTools> = OnceLock::new();

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

fn run_map(path: Option<&Path>, depth: Option<usize>) -> Result<()> {
    let root = path.unwrap_or(Path::new("."));
    let max_depth = depth.unwrap_or(2);

    println!("{}", "Repository Map".bold().underline());
    println!();

    // Collect file info
    let mut file_counts: HashMap<String, usize> = HashMap::new();
    count_files(root, &mut file_counts, max_depth, 0)?;

    // Sort by count
    let mut sorted: Vec<_> = file_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    println!("{}", "File Types:".dimmed());
    for (ext, count) in sorted.iter().take(10) {
        let bar_len = (*count as f64 / sorted[0].1 as f64 * 20.0) as usize;
        let bar = "█".repeat(bar_len);
        println!("  {:>8} {:>4} {}", ext.cyan(), count, bar.green());
    }

    println!();
    println!("{}", "Key Files:".dimmed());

    // Look for common important files
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

    for file in key_files {
        let file_path = root.join(file);
        if file_path.exists() {
            println!("  {} {}", "✓".green(), file.cyan());
        }
    }

    Ok(())
}

fn count_files(
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
        let path = entry.path();

        // Skip hidden and common ignore patterns
        if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && (name.starts_with('.') || name == "node_modules" || name == "target")
        {
            continue;
        }

        if path.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("(none)")
                .to_string();
            *counts.entry(ext).or_insert(0) += 1;
        } else if path.is_dir() {
            count_files(&path, counts, max_depth, depth + 1)?;
        }
    }

    Ok(())
}

fn run_symbols(path: &Path) -> Result<()> {
    println!(
        "{} {}",
        "Symbols in".bold(),
        path.display().to_string().cyan()
    );
    println!();

    // Simple regex-based symbol extraction (tree-sitter would be better)
    let content = fs::read_to_string(path)?;

    let fn_regex = regex::Regex::new(r"(?m)^(?:pub\s+)?(?:async\s+)?fn\s+(\w+)")?;
    let struct_regex = regex::Regex::new(r"(?m)^(?:pub\s+)?struct\s+(\w+)")?;
    let enum_regex = regex::Regex::new(r"(?m)^(?:pub\s+)?enum\s+(\w+)")?;
    let impl_regex = regex::Regex::new(r"(?m)^impl(?:<[^>]+>)?\s+(\w+)")?;

    println!("{}", "Functions:".dimmed());
    for cap in fn_regex.captures_iter(&content) {
        println!("  {} {}", "fn".blue(), &cap[1]);
    }

    println!();
    println!("{}", "Structs:".dimmed());
    for cap in struct_regex.captures_iter(&content) {
        println!("  {} {}", "struct".yellow(), &cap[1]);
    }

    println!();
    println!("{}", "Enums:".dimmed());
    for cap in enum_regex.captures_iter(&content) {
        println!("  {} {}", "enum".magenta(), &cap[1]);
    }

    println!();
    println!("{}", "Implementations:".dimmed());
    for cap in impl_regex.captures_iter(&content) {
        println!("  {} {}", "impl".cyan(), &cap[1]);
    }

    Ok(())
}

fn run_references(symbol: &str, path: Option<&Path>) -> Result<()> {
    let search_path = path.unwrap_or(Path::new("."));

    println!(
        "{} {} in {}",
        "References to".bold(),
        symbol.cyan(),
        search_path.display()
    );
    println!("{}", "─".repeat(60).dimmed());

    // Use grep-like search
    let pattern = format!(r"\b{}\b", regex::escape(symbol));
    let regex = regex::Regex::new(&pattern)?;

    fn search_refs(path: &Path, regex: &regex::Regex, count: &mut usize) -> Result<()> {
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(path) {
                for (line_num, line) in content.lines().enumerate() {
                    if regex.is_match(line) {
                        println!(
                            "{}:{}:{}",
                            path.display().to_string().cyan(),
                            (line_num + 1).to_string().yellow(),
                            line.trim()
                        );
                        *count += 1;
                    }
                }
            }
        } else if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|n| n.to_str())
                    && (name.starts_with('.') || name == "node_modules" || name == "target")
                {
                    continue;
                }
                search_refs(&p, regex, count)?;
            }
        }
        Ok(())
    }

    let mut count = 0;
    search_refs(search_path, &regex, &mut count)?;

    println!("{}", "─".repeat(60).dimmed());
    println!("{} {} references found", "Total:".bold(), count);

    Ok(())
}

fn run_definition(symbol: &str, path: Option<&Path>) -> Result<()> {
    let search_path = path.unwrap_or(Path::new("."));

    println!("{} {}", "Definition of".bold(), symbol.cyan());
    println!();

    // Look for definition patterns
    let patterns = [
        format!(
            r"(?m)^(?:pub\s+)?(?:async\s+)?fn\s+{}\s*[<(]",
            regex::escape(symbol)
        ),
        format!(
            r"(?m)^(?:pub\s+)?struct\s+{}\s*[<{{]",
            regex::escape(symbol)
        ),
        format!(r"(?m)^(?:pub\s+)?enum\s+{}\s*[<{{]", regex::escape(symbol)),
        format!(r"(?m)^(?:pub\s+)?type\s+{}\s*=", regex::escape(symbol)),
        format!(r"(?m)^(?:pub\s+)?const\s+{}\s*:", regex::escape(symbol)),
    ];

    fn find_def(
        path: &Path,
        patterns: &[regex::Regex],
    ) -> Result<Option<(PathBuf, usize, String)>> {
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(path) {
                for (line_num, line) in content.lines().enumerate() {
                    for pattern in patterns {
                        if pattern.is_match(line) {
                            return Ok(Some((path.to_path_buf(), line_num + 1, line.to_string())));
                        }
                    }
                }
            }
        } else if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|n| n.to_str())
                    && (name.starts_with('.') || name == "node_modules" || name == "target")
                {
                    continue;
                }
                if let Some(result) = find_def(&p, patterns)? {
                    return Ok(Some(result));
                }
            }
        }
        Ok(None)
    }

    let regexes: Vec<_> = patterns
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect();

    match find_def(search_path, &regexes)? {
        Some((path, line, content)) => {
            println!(
                "  {} {}:{}",
                "Found:".green(),
                path.display().to_string().cyan(),
                line
            );
            println!("  {}", content.trim());
        }
        None => {
            println!("  {}", "Definition not found".red());
        }
    }

    Ok(())
}

fn run_lint(_path: Option<&Path>, fix: bool) -> Result<()> {
    println!("{}", "Running linter...".bold());

    // Try cargo clippy for Rust projects
    if Path::new("Cargo.toml").exists() {
        let mut args = vec!["clippy"];
        if fix {
            args.push("--fix");
        }

        let output = std::process::Command::new("cargo").args(&args).output()?;

        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    } else {
        println!("{}", "No supported linter found for this project".yellow());
    }

    Ok(())
}

fn run_test(_path: Option<&Path>, filter: Option<&str>) -> Result<()> {
    println!("{}", "Running tests...".bold());

    if Path::new("Cargo.toml").exists() {
        let mut args = vec!["test"];
        if let Some(f) = filter {
            args.push(f);
        }

        let output = std::process::Command::new("cargo").args(&args).output()?;

        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    } else {
        println!("{}", "No supported test runner found".yellow());
    }

    Ok(())
}

fn run_deps(_path: Option<&Path>) -> Result<()> {
    println!("{}", "Dependencies".bold().underline());
    println!();

    if Path::new("Cargo.toml").exists() {
        let content = fs::read_to_string("Cargo.toml")?;
        let mut in_deps = false;

        for line in content.lines() {
            if line.starts_with("[dependencies]") || line.starts_with("[dev-dependencies]") {
                in_deps = true;
                println!("{}", line.cyan());
            } else if line.starts_with('[') {
                in_deps = false;
            } else if in_deps && !line.trim().is_empty() {
                println!("  {}", line);
            }
        }
    } else {
        println!("{}", "No Cargo.toml found".yellow());
    }

    Ok(())
}

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
    sorted.sort_by(|a, b| b.1.lines.cmp(&a.1.lines));

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
