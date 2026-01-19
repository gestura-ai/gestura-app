//! Knowledge CLI Commands
//!
//! Commands for managing and querying the agent knowledge system.
//! Knowledge items provide specialized expertise that can be loaded
//! on-demand based on the user's query.

use colored::Colorize;
use gestura_core::knowledge::{KnowledgeQuery, KnowledgeStore, register_builtin_knowledge};

use super::Result;
use crate::KnowledgeAction;

/// Run the knowledge command
pub fn run(action: &KnowledgeAction) -> Result<()> {
    match action {
        KnowledgeAction::List { category } => list_knowledge(category.as_deref()),
        KnowledgeAction::Show { id } => show_knowledge(id),
        KnowledgeAction::Search { query, limit } => search_knowledge(query, *limit),
        KnowledgeAction::Categories => list_categories(),
        KnowledgeAction::Status => show_status(),
    }
}

/// List all knowledge items
fn list_knowledge(category: Option<&str>) -> Result<()> {
    let store = KnowledgeStore::with_default_dir();
    register_builtin_knowledge(&store);

    println!("{}", "Knowledge Items".bold().cyan());
    println!("{}", "═".repeat(60));
    println!();

    let items = if let Some(cat) = category {
        store.list_by_category(cat)
    } else {
        store.list()
    };

    if items.is_empty() {
        println!("{}", "No knowledge items found.".yellow());
        return Ok(());
    }

    // Group by category
    let mut by_category: std::collections::HashMap<String, Vec<_>> =
        std::collections::HashMap::new();
    for item in items {
        by_category
            .entry(item.category.clone())
            .or_default()
            .push(item);
    }

    let mut categories: Vec<_> = by_category.keys().cloned().collect();
    categories.sort();

    for cat in categories {
        println!("{}", format!("  {}", cat.to_uppercase()).bold().yellow());
        if let Some(items) = by_category.get(&cat) {
            for item in items {
                let status = if item.enabled {
                    "●".green()
                } else {
                    "○".dimmed()
                };
                println!(
                    "    {} {} - {}",
                    status,
                    item.id.cyan(),
                    item.description.dimmed()
                );
                if !item.triggers.is_empty() {
                    let triggers = item
                        .triggers
                        .iter()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("      {} {}", "triggers:".dimmed(), triggers.dimmed());
                }
            }
        }
        println!();
    }

    println!("{}: {}", "Total".bold(), store.count());
    Ok(())
}

/// Show details of a specific knowledge item
fn show_knowledge(id: &str) -> Result<()> {
    let store = KnowledgeStore::with_default_dir();
    register_builtin_knowledge(&store);

    let item = store
        .get(id)
        .ok_or_else(|| format!("Knowledge item not found: {}", id))?;

    println!("{}", item.name.bold().cyan());
    println!("{}", "═".repeat(60));
    println!();

    println!("{}: {}", "ID".bold(), item.id);
    println!("{}: {}", "Category".bold(), item.category);
    println!("{}: {}", "Priority".bold(), item.priority);
    println!(
        "{}: {}",
        "Enabled".bold(),
        if item.enabled {
            "Yes".green()
        } else {
            "No".red()
        }
    );
    println!();

    println!("{}", "Description".bold().yellow());
    println!("  {}", item.description);
    println!();

    if !item.triggers.is_empty() {
        println!("{}", "Triggers".bold().yellow());
        for trigger in &item.triggers {
            println!("  {} {}", "•".cyan(), trigger);
        }
        println!();
    }

    if !item.references.is_empty() {
        println!("{}", "References".bold().yellow());
        for reference in &item.references {
            println!(
                "  {} {} ({})",
                "•".cyan(),
                reference.topic,
                reference.path.dimmed()
            );
            println!("    Load when: {:?}", reference.load_when);
        }
        println!();
    }

    if !item.core_content.is_empty() {
        println!("{}", "Core Content".bold().yellow());
        println!("{}", "─".repeat(60));
        // Show first 30 lines of content
        for line in item.core_content.lines().take(30) {
            println!("{}", line);
        }
        if item.core_content.lines().count() > 30 {
            println!("{}", "... (truncated)".dimmed());
        }
    }

    Ok(())
}

/// Search for knowledge items matching a query
fn search_knowledge(query_str: &str, limit: usize) -> Result<()> {
    let store = KnowledgeStore::with_default_dir();
    register_builtin_knowledge(&store);

    let query = KnowledgeQuery {
        query: query_str.to_string(),
        limit: Some(limit),
        min_score: Some(0.1),
        ..Default::default()
    };

    let matches = store.find(&query);

    println!(
        "{}",
        format!("Search Results for: \"{}\"", query_str)
            .bold()
            .cyan()
    );
    println!("{}", "═".repeat(60));
    println!();

    if matches.is_empty() {
        println!("{}", "No matching knowledge items found.".yellow());
        println!();
        println!(
            "{}",
            "Try different keywords or check available items with:".dimmed()
        );
        println!("  {} knowledge list", "gestura".cyan());
        return Ok(());
    }

    for (i, m) in matches.iter().enumerate() {
        let score_pct = (m.score * 100.0) as u32;
        let score_color = if score_pct >= 70 {
            "green"
        } else if score_pct >= 40 {
            "yellow"
        } else {
            "red"
        };
        let score_str = format!("{}%", score_pct);
        let score_display = match score_color {
            "green" => score_str.green(),
            "yellow" => score_str.yellow(),
            _ => score_str.red(),
        };

        println!("{}. {} [{}]", i + 1, m.item.name.bold(), score_display);
        println!("   {} {}", "ID:".dimmed(), m.item.id.cyan());
        println!("   {} {}", "Category:".dimmed(), m.item.category);
        if !m.matched_triggers.is_empty() {
            println!(
                "   {} {}",
                "Matched:".dimmed(),
                m.matched_triggers.join(", ").green()
            );
        }
        println!();
    }

    println!("{}: {}", "Found".bold(), matches.len());
    Ok(())
}

/// List all categories
fn list_categories() -> Result<()> {
    let store = KnowledgeStore::with_default_dir();
    register_builtin_knowledge(&store);

    println!("{}", "Knowledge Categories".bold().cyan());
    println!("{}", "═".repeat(40));
    println!();

    let categories = store.categories();
    if categories.is_empty() {
        println!("{}", "No categories found.".yellow());
        return Ok(());
    }

    for cat in &categories {
        let count = store.list_by_category(cat).len();
        println!(
            "  {} {} ({})",
            "•".cyan(),
            cat.bold(),
            format!("{} items", count).dimmed()
        );
    }
    println!();

    println!("{}: {}", "Total categories".bold(), categories.len());
    Ok(())
}

/// Show knowledge system status
fn show_status() -> Result<()> {
    let store = KnowledgeStore::with_default_dir();
    register_builtin_knowledge(&store);

    println!("{}", "Knowledge System Status".bold().cyan());
    println!("{}", "═".repeat(50));
    println!();

    println!(
        "{}: {}",
        "Base Directory".bold(),
        store.base_dir().display()
    );
    println!("{}: {}", "Total Items".bold(), store.count());
    println!("{}: {}", "Categories".bold(), store.categories().len());
    println!();

    println!("{}", "Features".bold().yellow());
    println!("  {} Progressive disclosure pattern", "✓".green());
    println!("  {} On-demand reference loading", "✓".green());
    println!("  {} Trigger-based matching", "✓".green());
    println!("  {} Category organization", "✓".green());
    println!("  {} Built-in knowledge items", "✓".green());
    println!();

    println!("{}", "Built-in Knowledge".bold().yellow());
    for item in store.list() {
        let status = if item.enabled {
            "●".green()
        } else {
            "○".dimmed()
        };
        println!("  {} {} ({})", status, item.name, item.category.dimmed());
    }
    println!();

    println!("{}", "Usage".bold().yellow());
    println!("  {} knowledge list", "gestura".cyan());
    println!("  {} knowledge show <id>", "gestura".cyan());
    println!("  {} knowledge search <query>", "gestura".cyan());
    println!("  {} knowledge categories", "gestura".cyan());

    Ok(())
}
