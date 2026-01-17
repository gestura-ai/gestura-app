//! Web fetching tool
//!
//! Provides web operations:
//! - fetch: Fetch URL and convert to markdown
//! - search: Search the web
//! - screenshot: Capture webpage screenshot

use super::super::Result;
use colored::Colorize;
use gestura_core::tools::web::WebTools;
use std::sync::OnceLock;

/// Global web tools instance
static WEB_TOOLS: OnceLock<WebTools> = OnceLock::new();

fn get_web_tools() -> &'static WebTools {
    WEB_TOOLS.get_or_init(WebTools::new)
}

/// Web subcommand options
pub enum WebSubcommand {
    Fetch {
        url: String,
        selector: Option<String>,
        #[allow(dead_code)] // Reserved for future use
        no_images: bool,
    },
    Search {
        query: String,
        num_results: Option<usize>,
    },
    Screenshot {
        url: String,
        output: Option<String>,
    },
}

/// Run web subcommand
pub fn run(cmd: WebSubcommand) -> Result<()> {
    match cmd {
        WebSubcommand::Fetch {
            url,
            selector,
            no_images: _,
        } => run_fetch(&url, selector.as_deref()),
        WebSubcommand::Search { query, num_results } => run_search(&query, num_results),
        WebSubcommand::Screenshot { url, output } => run_screenshot(&url, output.as_deref()),
    }
}

fn run_fetch(url: &str, selector: Option<&str>) -> Result<()> {
    println!("{} {}", "Fetching:".bold(), url.cyan());
    println!("{}", "─".repeat(60).dimmed());

    // Create runtime for async call
    let rt = tokio::runtime::Runtime::new()?;

    let result = rt.block_on(async { get_web_tools().fetch(url).await })?;

    // Convert HTML to text
    let text = get_web_tools().html_to_text(&result.content);

    if let Some(sel) = selector {
        println!(
            "{}",
            format!("(Selector '{}' not yet implemented)", sel).yellow()
        );
    }

    println!(
        "{}",
        format!(
            "Status: {} | Content-Type: {}",
            result.status_code,
            result.content_type.as_deref().unwrap_or("unknown")
        )
        .dimmed()
    );
    println!("{}", "─".repeat(60).dimmed());
    println!("{}", text);

    Ok(())
}

fn run_search(query: &str, num_results: Option<usize>) -> Result<()> {
    let count = num_results.unwrap_or(5);

    println!(
        "{} {} (top {} results)",
        "Searching:".bold(),
        query.cyan(),
        count
    );
    println!();

    // Create runtime for async call
    let rt = tokio::runtime::Runtime::new()?;

    let result = rt.block_on(async { get_web_tools().search(query, Some(count)).await })?;

    if result.results.is_empty() {
        println!("{}", "(No results found)".dimmed());
    } else {
        for (i, item) in result.results.iter().enumerate() {
            println!("{}. {}", (i + 1).to_string().cyan(), item.title.bold());
            println!("   {}", item.url.dimmed());
            println!("   {}", item.snippet);
            println!();
        }
    }

    Ok(())
}

fn run_screenshot(url: &str, output: Option<&str>) -> Result<()> {
    let output_file = output.unwrap_or("screenshot.png");

    println!("{} {}", "Screenshot:".bold(), url.cyan());
    println!("{} {}", "Output:".dimmed(), output_file);
    println!();
    println!(
        "{}",
        "(Screenshot requires headless browser - not yet implemented)".yellow()
    );
    println!();
    println!("{}", "Consider using:".dimmed());
    println!("  • playwright screenshot {} --output {}", url, output_file);
    println!("  • puppeteer or similar headless browser tool");

    Ok(())
}
