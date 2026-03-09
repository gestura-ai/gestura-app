//! Context management CLI commands
//!
//! Provides commands for analyzing requests and managing context caching.

use clap::Subcommand;
use colored::Colorize;
use gestura_core::context::{ContextCategory, ContextManager, RequestAnalyzer};

/// Context management actions
#[derive(Debug, Subcommand)]
pub enum ContextAction {
    /// Analyze a request to determine needed context
    Analyze {
        /// The request to analyze
        request: String,
    },
    /// Show context system status
    Status,
    /// List available context categories
    Categories,
    /// Clear all context caches
    Clear,
}

/// Run context command
pub fn run(action: ContextAction) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        ContextAction::Analyze { request } => analyze_request(&request),
        ContextAction::Status => show_status(),
        ContextAction::Categories => list_categories(),
        ContextAction::Clear => clear_caches(),
    }
}

fn analyze_request(request: &str) -> Result<(), Box<dyn std::error::Error>> {
    let analyzer = RequestAnalyzer::new();
    let analysis = analyzer.analyze(request);

    println!("{}\n", "Request Analysis".bright_cyan().bold().underline());

    println!("{}", "═".repeat(60).bright_black());
    println!();

    println!("{}: {}", "Request".bright_white(), request);
    println!();

    // Categories
    println!("{}", "Detected Categories".bright_yellow());
    if analysis.categories.is_empty() {
        println!("  {}", "(none)".dimmed());
    } else {
        for cat in &analysis.categories {
            let cat_str = format!("{:?}", cat);
            let icon = category_icon(*cat);
            println!("  {} {}", icon, cat_str.bright_white());
        }
    }
    println!();

    // Tools
    println!("{}", "Suggested Tools".bright_yellow());
    if analysis.suggested_tools.is_empty() {
        println!("  {}", "(none - general conversation)".dimmed());
    } else {
        for tool in &analysis.suggested_tools {
            println!("  ● {}", tool.bright_white());
        }
    }
    println!();

    // Entities
    if !analysis.entities.is_empty() {
        println!("{}", "Extracted Entities".bright_yellow());
        for entity in &analysis.entities {
            println!(
                "  {} [{}]: {}",
                "→".bright_black(),
                format!("{:?}", entity.entity_type).bright_cyan(),
                entity.value.bright_white()
            );
        }
        println!();
    }

    // Flags
    println!("{}", "Analysis Flags".bright_yellow());
    let needs_tools = if analysis.needs_tools {
        "✓".green()
    } else {
        "✗".red()
    };
    let is_followup = if analysis.is_followup {
        "✓".green()
    } else {
        "✗".red()
    };
    println!("  Needs Tools: {}", needs_tools);
    println!("  Is Follow-up: {}", is_followup);
    let confidence_pct = (analysis.confidence * 100.0) as u32;
    println!(
        "  Confidence: {}%",
        confidence_pct.to_string().bright_white()
    );

    Ok(())
}

fn show_status() -> Result<(), Box<dyn std::error::Error>> {
    let manager = ContextManager::new();
    let stats = manager.cache_stats();

    println!(
        "{}\n",
        "Context Manager Status".bright_cyan().bold().underline()
    );
    println!("{}", "═".repeat(50).bright_black());
    println!();

    println!("{}", "Cache Statistics".bright_yellow());
    println!(
        "  Context Cache: {} / {} entries",
        stats.context_cache.size.to_string().bright_white(),
        stats.context_cache.max_size
    );
    println!(
        "  File Cache:    {} / {} entries",
        stats.file_cache.size.to_string().bright_white(),
        stats.file_cache.max_size
    );
    println!(
        "  History Cache: {} / {} entries",
        stats.history_cache.size.to_string().bright_white(),
        stats.history_cache.max_size
    );
    println!();

    println!("{}", "Features".bright_yellow());
    println!("  {} Request analysis without LLM", "✓".green());
    println!("  {} Category-based tool filtering", "✓".green());
    println!("  {} Smart context caching with TTL", "✓".green());
    println!("  {} Entity extraction (paths, URLs)", "✓".green());
    println!("  {} Follow-up detection", "✓".green());

    Ok(())
}

fn list_categories() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}\n",
        "Context Categories".bright_cyan().bold().underline()
    );
    println!("{}", "═".repeat(50).bright_black());
    println!();

    let categories = [
        (
            ContextCategory::FileSystem,
            "File system operations (read, write, edit)",
        ),
        (ContextCategory::Shell, "Shell command execution"),
        (ContextCategory::Git, "Git version control operations"),
        (ContextCategory::Code, "Code analysis (symbols, references)"),
        (ContextCategory::Web, "Web fetching and search"),
        (ContextCategory::Voice, "Voice and audio processing"),
        (ContextCategory::Config, "Configuration management"),
        (ContextCategory::Session, "Session and history"),
        (ContextCategory::Tools, "Tool introspection"),
        (ContextCategory::Agent, "Agent orchestration"),
        (ContextCategory::Mcp, "MCP protocol operations"),
        (ContextCategory::A2a, "A2A protocol operations"),
        (ContextCategory::Task, "Task management for current session"),
        (
            ContextCategory::Screen,
            "Screen capture and recording (screenshot, screen_record)",
        ),
        (ContextCategory::General, "General conversation (no tools)"),
    ];

    for (cat, desc) in categories {
        let icon = category_icon(cat);
        println!("{} {}", icon, format!("{:?}", cat).bright_white());
        println!("  {}", desc.dimmed());
    }

    Ok(())
}

fn clear_caches() -> Result<(), Box<dyn std::error::Error>> {
    let manager = ContextManager::new();
    manager.clear_caches();
    println!("{}", "✓ All context caches cleared".green());
    Ok(())
}

fn category_icon(cat: ContextCategory) -> &'static str {
    match cat {
        ContextCategory::FileSystem => "📁",
        ContextCategory::Shell => "🖥️",
        ContextCategory::Git => "🔀",
        ContextCategory::Code => "💻",
        ContextCategory::Web => "🌐",
        ContextCategory::Voice => "🎤",
        ContextCategory::Config => "⚙️",
        ContextCategory::Session => "📜",
        ContextCategory::Tools => "🔧",
        ContextCategory::Agent => "🤖",
        ContextCategory::Mcp => "🔌",
        ContextCategory::A2a => "🔗",
        ContextCategory::Task => "✅",
        ContextCategory::Screen => "🎥",
        ContextCategory::General => "💬",
    }
}
