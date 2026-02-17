//! File operations tool
//!
//! Provides file system operations:
//! - read: Read file contents
//! - write: Write content to file
//! - edit: Edit file with str_replace
//! - search: Search files with regex
//! - list: List files in directory
//! - tree: Show directory tree
//! - add: Add files to context
//! - drop: Remove files from context
//! - context: Show current file context

use super::super::Result;
use colored::Colorize;
use gestura_core::tools::file::{FileTools, TreeNode};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Global file tools instance
static FILE_TOOLS: OnceLock<FileTools> = OnceLock::new();

fn get_file_tools() -> &'static FileTools {
    FILE_TOOLS.get_or_init(FileTools::new)
}

/// File subcommand options
pub enum FileSubcommand {
    Read {
        path: PathBuf,
        lines: Option<String>,
    },
    Write {
        path: PathBuf,
        content: String,
    },
    Edit {
        path: PathBuf,
        old_str: String,
        new_str: String,
    },
    Search {
        pattern: String,
        path: Option<PathBuf>,
        recursive: bool,
    },
    List {
        path: Option<PathBuf>,
        all: bool,
    },
    Tree {
        path: Option<PathBuf>,
        max_depth: Option<usize>,
    },
    Add {
        paths: Vec<PathBuf>,
    },
    Drop {
        paths: Vec<PathBuf>,
    },
    Context,
}

/// Run file subcommand
pub fn run(cmd: FileSubcommand) -> Result<()> {
    match cmd {
        FileSubcommand::Read { path, lines } => run_read(&path, lines.as_deref()),
        FileSubcommand::Write { path, content } => run_write(&path, &content),
        FileSubcommand::Edit {
            path,
            old_str,
            new_str,
        } => run_edit(&path, &old_str, &new_str),
        FileSubcommand::Search {
            pattern,
            path,
            recursive,
        } => run_search(&pattern, path.as_deref(), recursive),
        FileSubcommand::List { path, all } => run_list(path.as_deref(), all),
        FileSubcommand::Tree { path, max_depth } => run_tree(path.as_deref(), max_depth),
        FileSubcommand::Add { paths } => run_add(&paths),
        FileSubcommand::Drop { paths } => run_drop(&paths),
        FileSubcommand::Context => run_context(),
    }
}

fn run_read(path: &Path, lines: Option<&str>) -> Result<()> {
    // Parse line range if provided (e.g., "1-10" or "5")
    let (start_line, end_line) = if let Some(range) = lines {
        if range.contains('-') {
            let parts: Vec<&str> = range.split('-').collect();
            let start: usize = parts[0].parse().unwrap_or(1);
            let end: usize = parts
                .get(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(usize::MAX);
            (Some(start), Some(end))
        } else {
            let line: usize = range.parse().unwrap_or(1);
            (Some(line), Some(line))
        }
    } else {
        (None, None)
    };

    let result = get_file_tools().read(path, start_line, end_line)?;

    println!(
        "{} {}",
        "File:".bold(),
        result.path.display().to_string().cyan()
    );
    println!("{}", "─".repeat(60).dimmed());

    for (i, line) in result.content.lines().enumerate() {
        println!(
            "{:>4} │ {}",
            (result.start_line + i).to_string().dimmed(),
            line
        );
    }

    Ok(())
}

fn run_write(path: &Path, content: &str) -> Result<()> {
    let result = get_file_tools().write(path, content)?;

    let action = if result.created { "Created" } else { "Wrote" };
    println!(
        "{} {} {} bytes to {}",
        "✓".green(),
        action,
        result.bytes_written,
        result.path.display().to_string().cyan()
    );
    Ok(())
}

fn run_edit(path: &Path, old_str: &str, new_str: &str) -> Result<()> {
    let result = get_file_tools().edit(path, old_str, new_str)?;

    println!(
        "{} Edited {}",
        "✓".green(),
        result.path.display().to_string().cyan()
    );
    println!("  Replaced {} occurrence(s)", result.replacements);
    Ok(())
}

fn run_search(pattern: &str, path: Option<&Path>, recursive: bool) -> Result<()> {
    let search_path = path.unwrap_or(Path::new("."));

    println!(
        "{} {} in {}",
        "Searching:".bold(),
        pattern.cyan(),
        search_path.display()
    );
    println!("{}", "─".repeat(60).dimmed());

    let matches = get_file_tools().search(pattern, search_path, recursive)?;

    for m in &matches {
        println!(
            "{}:{}:{}",
            m.path.display().to_string().cyan(),
            m.line_number.to_string().yellow(),
            m.line_content
        );
    }

    println!("{}", "─".repeat(60).dimmed());
    println!("{} {} matches found", "Total:".bold(), matches.len());
    Ok(())
}

fn run_list(path: Option<&Path>, all: bool) -> Result<()> {
    let dir = path.unwrap_or(Path::new("."));

    println!(
        "{} {}",
        "Directory:".bold(),
        dir.display().to_string().cyan()
    );
    println!("{}", "─".repeat(60).dimmed());

    let entries = get_file_tools().list(dir, all)?;

    for entry in entries {
        let type_indicator = if entry.is_dir { "/" } else { "" };
        let size = if let Some(s) = entry.size {
            if !entry.is_dir {
                format!("{:>8}", format_size(s))
            } else {
                "       -".to_string()
            }
        } else {
            "       -".to_string()
        };

        let display_name = if entry.is_dir {
            format!("{}{}", entry.name, type_indicator)
                .blue()
                .to_string()
        } else {
            format!("{}{}", entry.name, type_indicator)
        };

        println!("  {} {}", size.dimmed(), display_name);
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}K", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

fn run_tree(path: Option<&Path>, max_depth: Option<usize>) -> Result<()> {
    let root = path.unwrap_or(Path::new("."));

    // CLI defaults to hiding dotfiles for parity with `list` unless a flag is added.
    let tree = get_file_tools().tree(root, max_depth, false)?;

    println!("{}", tree.name.cyan().bold());
    print_tree_node(&tree, "");

    Ok(())
}

fn print_tree_node(node: &TreeNode, prefix: &str) {
    let count = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last { "    " } else { "│   " };

        if child.is_dir {
            println!("{}{}{}/", prefix, connector, child.name.blue());
            print_tree_node(child, &format!("{}{}", prefix, child_prefix));
        } else {
            println!("{}{}{}", prefix, connector, child.name);
        }
    }
}

fn run_add(paths: &[PathBuf]) -> Result<()> {
    let added = get_file_tools().add_to_context(paths)?;

    for path in paths {
        if added.contains(path) {
            println!(
                "{} Added to context: {}",
                "✓".green(),
                path.display().to_string().cyan()
            );
        } else {
            eprintln!("{} File not found: {}", "⚠".yellow(), path.display());
        }
    }
    println!();
    println!(
        "{}",
        "Files added to agent context. Use /context in agent to view.".dimmed()
    );
    Ok(())
}

fn run_drop(paths: &[PathBuf]) -> Result<()> {
    let removed = get_file_tools().remove_from_context(paths)?;

    for path in &removed {
        println!(
            "{} Removed from context: {}",
            "✓".green(),
            path.display().to_string().cyan()
        );
    }
    Ok(())
}

fn run_context() -> Result<()> {
    println!("{}", "Current File Context".bold().underline());
    println!();

    let context = get_file_tools().get_context()?;
    if context.is_empty() {
        println!("{}", "(No files in context)".dimmed());
    } else {
        for path in context {
            println!("  {}", path.display().to_string().cyan());
        }
    }
    println!();
    println!(
        "{}",
        "Use 'gestura tools file add <PATH>' to add files".dimmed()
    );
    Ok(())
}
