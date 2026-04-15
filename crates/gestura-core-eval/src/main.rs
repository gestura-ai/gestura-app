//! `gestura-eval` — standalone evaluation binary.
//!
//! Tests agent CLIs as black-box subprocesses across 8 standardised scenarios.
//! Supports multiple agentic interfaces (Gestura, Claude Code, Augment, Codex, OpenCode)
//! via TOML agent profiles. The binary is intentionally separate from the `gestura` CLI
//! so evaluation logic never ships inside the product binary.
//!
//! # Modes
//!
//! ## Single-agent (default — existing behaviour unchanged)
//! ```bash
//! gestura-eval                                       # gestura-full profile
//! gestura-eval --agent claude-code-full              # specific profile
//! gestura-eval --agent gestura-full --dry-run        # check-logic only
//! gestura-eval --agent codex-full --json             # JSON output
//! gestura-eval --agent augment-full --verbose        # all responses
//! ```
//!
//! ## Multi-agent suite
//! ```bash
//! gestura-eval suite                                 # all 15 profiles
//! gestura-eval suite --families gestura,claude-code  # filter by family
//! gestura-eval suite --agents gestura-full,codex-sandboxed
//! gestura-eval suite --output-dir ./eval-results     # save JSON + HTML
//! gestura-eval suite --format html                   # HTML only
//! gestura-eval suite --dry-run                       # no LLM calls
//! ```
//!
//! ## Report from saved files (no agent invocations)
//! ```bash
//! gestura-eval report --from ./eval-results/2026-04-14
//! gestura-eval report --from ./eval-results/2026-04-07 --from ./eval-results/2026-04-14
//! gestura-eval report --from ./eval-results --format html --output-dir ./reports
//! ```

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use gestura_core_eval::{
    comparison::ComparisonEngine,
    config::{EvalConfig, BUILTIN_AGENT_IDS},
    html_report,
    orchestrator::{MultiRunOrchestrator, ProfileSelector, SuiteRunPlan},
    progress::{ProgressCallback, ProgressEvent},
    report::EvalReport,
    CliEvalRunner, CliRunnerOptions, EvalScenarioSuite,
};

// ─── Top-level CLI ────────────────────────────────────────────────────────────

/// Gestura evaluation harness — runs standardised scenarios against any agent CLI.
#[derive(Parser, Debug)]
#[command(name = "gestura-eval", author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<SubCommand>,

    // ── Single-agent flags (only used when no subcommand is given) ──────────

    /// Built-in agent profile (see --list-agents). Defaults to `gestura-full`.
    #[arg(long, value_name = "AGENT_ID", env = "GESTURA_EVAL_AGENT")]
    agent: Option<String>,

    /// Path to a custom agent profile TOML (merged on top of baseline).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Override the agent binary path.
    #[arg(long = "bin", value_name = "PATH", env = "GESTURA_BIN")]
    bin_override: Option<PathBuf>,

    /// Run only this scenario (use --list to see IDs).
    #[arg(long, value_name = "ID")]
    scenario: Option<String>,

    /// Skip subprocess calls; validate check logic on stub responses only.
    #[arg(long)]
    dry_run: bool,

    /// Emit JSON output (suitable for CI / cross-tool comparison).
    #[arg(long)]
    json: bool,

    /// List available scenario IDs and exit.
    #[arg(long)]
    list: bool,

    /// List built-in agent profile IDs and exit.
    #[arg(long)]
    list_agents: bool,

    /// Suppress all non-report output.
    #[arg(long, short)]
    quiet: bool,

    /// Show full response and all check results for every variation.
    #[arg(long, short)]
    verbose: bool,

    /// Show agent response only for failed variations.
    #[arg(long)]
    show_breaking: bool,
}

// ─── Subcommands ──────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
enum SubCommand {
    /// Run multiple agent profiles against the full scenario suite.
    Suite(SuiteArgs),

    /// Generate a comparison report from previously saved JSON files.
    Report(ReportArgs),
}

// ─── `suite` args ─────────────────────────────────────────────────────────────

/// Run a multi-agent comparison suite.
///
/// By default runs all 15 built-in profiles sequentially with live terminal
/// progress. Each finished profile is saved as JSON immediately so a partial
/// run is never lost.
#[derive(clap::Args, Debug)]
struct SuiteArgs {
    /// Agent families to include, comma-separated (e.g. `gestura,claude-code`).
    /// Expands to all three mode variants (full/sandboxed/iterative).
    /// When omitted, all 5 families run.
    #[arg(long, value_delimiter = ',', value_name = "FAMILY")]
    families: Vec<String>,

    /// Explicit agent profile IDs, comma-separated.
    /// Takes precedence over --families.
    #[arg(long, value_delimiter = ',', value_name = "AGENT_ID")]
    agents: Vec<String>,

    /// Restrict to specific scenario IDs, comma-separated (e.g. `s1_simple_query,s3_planning`).
    #[arg(long, value_delimiter = ',', value_name = "ID")]
    scenario: Vec<String>,

    /// Directory to write per-agent JSON reports, comparison JSON, and HTML report.
    /// Created if it does not exist.
    #[arg(long, value_name = "DIR")]
    output_dir: Option<PathBuf>,

    /// Skip subprocess calls; validate check logic on stub responses only.
    #[arg(long)]
    dry_run: bool,

    /// Override the agent binary path for all profiles.
    #[arg(long = "bin", value_name = "PATH")]
    bin_override: Option<PathBuf>,

    /// Output format. Defaults to `all` when --output-dir is given, `text` otherwise.
    #[arg(long, value_name = "FORMAT")]
    format: Option<OutputFormat>,

    /// Suppress per-variation live output (only show profile progress bars).
    #[arg(long, short)]
    quiet: bool,

    /// Minimum pause between variation subprocess calls, in milliseconds.
    /// Overrides `execution.delay_between_variations_ms` in every profile.
    /// Use to stay within per-minute token-rate limits without editing TOMLs.
    /// Example: --variation-delay-ms 3000  (3 s between calls ≈ safe for most tiers)
    #[arg(long, value_name = "MS")]
    variation_delay_ms: Option<u64>,
}

// ─── `report` args ────────────────────────────────────────────────────────────

/// Generate a comparison report from saved per-agent JSON files.
///
/// Pass `--from` once per directory (or file).  All valid EvalReport JSON
/// files found are loaded and compared.  Pass `--from` twice to compare two
/// separate runs for regression tracking.
#[derive(clap::Args, Debug)]
struct ReportArgs {
    /// Path to a directory of per-agent JSON files, or a single JSON file.
    /// Repeat to merge multiple runs.
    #[arg(long, value_name = "PATH", required = true)]
    from: Vec<PathBuf>,

    /// Output format (default: text).
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    format: OutputFormat,

    /// Write HTML / JSON output here (stdout if omitted).
    #[arg(long, value_name = "DIR")]
    output_dir: Option<PathBuf>,
}

// ─── Output format ────────────────────────────────────────────────────────────

#[derive(ValueEnum, Debug, Clone, Default)]
enum OutputFormat {
    #[default]
    Text,
    Json,
    Html,
    All,
}

// ─── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    let args = Cli::parse();

    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let suite = EvalScenarioSuite::load_builtin();

    // ── Informational flags (always available, exit immediately) ─────────────
    if args.list {
        println!("{}", "Available scenarios:".bold());
        for s in &suite.scenarios {
            println!("  {:<28} {}", s.id.cyan(), s.description.dimmed());
        }
        return;
    }

    if args.list_agents {
        println!("{}", "Built-in agent profiles:".bold());
        for id in BUILTIN_AGENT_IDS {
            match EvalConfig::load_builtin(id) {
                Ok(cfg) => {
                    let mode_tag = format!("{:?}", cfg.agent.mode).to_lowercase();
                    let auth_tag = if cfg.agent.requires_manual_auth {
                        " [manual-auth]".red().to_string()
                    } else {
                        String::new()
                    };
                    println!(
                        "  {:<28} [{:<12}]{} {}",
                        id.cyan(),
                        mode_tag.yellow(),
                        auth_tag,
                        cfg.agent.description.dimmed()
                    );
                }
                Err(e) => println!("  {:<28} (error: {e})", id),
            }
        }
        return;
    }

    // ── Dispatch to subcommand ────────────────────────────────────────────────
    match args.command {
        Some(SubCommand::Suite(suite_args)) => run_suite(suite_args, suite),
        Some(SubCommand::Report(report_args)) => run_report(report_args),
        None => run_single_agent(args, suite),
    }
}

// ─── Single-agent mode (existing behaviour) ───────────────────────────────────

fn run_single_agent(args: Cli, suite: EvalScenarioSuite) {
    // Load the agent profile.
    let eval_config = if let Some(ref path) = args.config {
        match EvalConfig::load_from_path(path) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("{} failed to load config '{}': {e}", "error:".red().bold(), path.display());
                std::process::exit(1);
            }
        }
    } else {
        let agent_id = args.agent.as_deref().unwrap_or("gestura-full");
        match EvalConfig::load_builtin(agent_id) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!(
                    "{} {e}. Run `gestura-eval --list-agents` to see valid IDs.",
                    "error:".red().bold()
                );
                std::process::exit(1);
            }
        }
    };

    // Auth guard.
    if eval_config.agent.requires_manual_auth
        && !args.dry_run
        && let Some(ref env_var) = eval_config.agent.auth_env_var
    {
        let present = std::env::var(env_var).map(|v| !v.is_empty()).unwrap_or(false);
        if !present {
            eprintln!(
                "{} Agent profile '{}' requires manual authentication.\n\
                 \n\
                 This profile uses an OAuth session token ({env_var}) rather than a\n\
                 static API key. It is excluded from fully automated runs.\n\
                 \n\
                 To run it:\n\
                 \n\
                 1. Log in on a machine with a browser:  auggie login\n\
                 2. Export the session:                  export {env_var}=$(auggie token print)\n\
                 3. Re-run with the token in env:        {env_var}=... gestura-eval --agent {}\n\
                 \n\
                 To validate check logic without auth:   gestura-eval --agent {} --dry-run",
                "error:".red().bold(),
                eval_config.agent.id,
                eval_config.agent.id,
                eval_config.agent.id,
            );
            std::process::exit(2);
        }
    }

    // Validate --scenario filter.
    if let Some(ref id) = args.scenario
        && !suite.scenarios.iter().any(|s| &s.id == id)
    {
        eprintln!(
            "{} Unknown scenario '{}'. Run `gestura-eval --list` to see valid IDs.",
            "error:".red().bold(),
            id
        );
        std::process::exit(1);
    }

    // Single-agent progress: only needed to surface rate-limit retry messages.
    // Everything else is printed by print_text() after the run finishes.
    let single_agent_progress: ProgressCallback = Arc::new(|event| {
        if let ProgressEvent::RateLimitRetry {
            scenario_id, variation_id, attempt, max_attempts, wait_secs, ..
        } = event {
            eprintln!(
                "{} {}/{} rate-limited (429) — waiting {}s  [retry {}/{}]",
                "⏸".yellow(),
                scenario_id,
                variation_id,
                wait_secs,
                attempt,
                max_attempts - 1,
            );
        }
    });

    let opts = CliRunnerOptions {
        eval_config: eval_config.clone(),
        scenario_ids: args.scenario.as_ref().map(|id| vec![id.clone()]).unwrap_or_default(),
        dry_run: args.dry_run,
        bin_override: args.bin_override,
        progress: Some(single_agent_progress),
    };

    let bin = opts.eval_config.resolve_bin(opts.bin_override.as_ref());

    if !args.quiet && !args.json {
        println!("{}", "gestura-eval — cross-interface scenario harness".bold());
        println!("  Agent   : {} [{}]", eval_config.agent.name.cyan(), eval_config.agent.id);
        println!("  Mode    : {}", format!("{:?}", eval_config.agent.mode).to_lowercase().yellow());
        println!("  Binary  : {}", bin.display().to_string().cyan());
        println!("  Model   : {}/{}", eval_config.model.provider, eval_config.model.name);
        if args.dry_run {
            println!("  Run     : {}", "DRY-RUN (no subprocess calls)".yellow());
        }
        if let Some(ref id) = args.scenario {
            println!("  Filter  : scenario {}", id.cyan());
        }
        println!();
    }

    let runner = CliEvalRunner::new(opts);
    let report = runner.run_suite(&suite);

    if args.json || args.quiet {
        report.print_json();
    } else {
        report.print_text(args.verbose, args.show_breaking);
        let s = &report.summary;
        if s.failed_variations > 0 && !args.dry_run {
            eprintln!(
                "\n{} {}/{} variations failed. Re-run with --show-breaking to see failing responses, \
                 --verbose for all responses, or --json for machine-readable details.",
                "note:".yellow(),
                s.failed_variations,
                s.total_variations
            );
        }
    }

    if !args.dry_run && report.summary.failed_variations > 0 {
        std::process::exit(1);
    }
}

// ─── `suite` subcommand ───────────────────────────────────────────────────────

fn run_suite(args: SuiteArgs, suite: EvalScenarioSuite) {
    // Resolve profiles from selector.
    let selector = ProfileSelector {
        agent_ids: args.agents.clone(),
        families: args.families.clone(),
    };

    let mut profiles = match selector.resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            std::process::exit(1);
        }
    };

    // Apply CLI delay override to every profile so the caller doesn't have to
    // edit TOML files just to add a throttle.
    if let Some(delay_ms) = args.variation_delay_ms {
        for cfg in &mut profiles {
            cfg.execution.delay_between_variations_ms = delay_ms;
        }
    }

    if profiles.is_empty() {
        eprintln!("{} No profiles matched the given --families / --agents filter.", "error:".red().bold());
        std::process::exit(1);
    }

    // Validate scenario filter.
    for id in &args.scenario {
        if !suite.scenarios.iter().any(|s| &s.id == id) {
            eprintln!(
                "{} Unknown scenario '{}'. Run `gestura-eval --list` to see valid IDs.",
                "error:".red().bold(), id
            );
            std::process::exit(1);
        }
    }

    // Create output directory if requested.
    if let Some(ref dir) = args.output_dir
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        eprintln!("{} Could not create output directory '{}': {e}", "error:".red().bold(), dir.display());
        std::process::exit(1);
    }

    // Determine effective format.
    let format = args.format.clone().unwrap_or(match args.output_dir {
        Some(_) => OutputFormat::All,
        None => OutputFormat::Text,
    });

    // Print suite header.
    if !args.quiet {
        println!("{}", "gestura-eval suite — multi-agent comparison".bold());
        println!("  Profiles : {} agents", profiles.len());
        if !args.families.is_empty() {
            println!("  Families : {}", args.families.join(", ").cyan());
        }
        if !args.agents.is_empty() {
            println!("  Agents   : {}", args.agents.join(", ").cyan());
        }
        if !args.scenario.is_empty() {
            println!("  Scenarios: {}", args.scenario.join(", ").cyan());
        }
        if args.dry_run {
            println!("  Run      : {}", "DRY-RUN (no subprocess calls)".yellow());
        }
        if let Some(ref dir) = args.output_dir {
            println!("  Output   : {}", dir.display().to_string().cyan());
        }
        println!();
    }

    // Build progress callback.
    let progress_cb: ProgressCallback = if args.quiet {
        Arc::new(|_| {})
    } else {
        make_terminal_progress(args.quiet)
    };

    // Build and run orchestrator.
    let plan = SuiteRunPlan {
        profiles,
        suite,
        output_dir: args.output_dir.clone(),
        dry_run: args.dry_run,
        bin_override: args.bin_override,
        scenario_ids: args.scenario,
    };

    let reports = MultiRunOrchestrator::new(plan)
        .with_progress(progress_cb)
        .run();

    if reports.is_empty() {
        eprintln!("{} No reports produced — all profiles were skipped.", "warning:".yellow());
        return;
    }

    // Compute comparison.
    let comparison = ComparisonEngine::compare(reports);

    // Output based on format.
    match format {
        OutputFormat::Text => {
            comparison.print_text();
        }
        OutputFormat::Json => {
            if let Some(ref dir) = args.output_dir {
                save_comparison_json(&comparison, dir);
            } else {
                comparison.print_json();
            }
        }
        OutputFormat::Html => {
            match &args.output_dir {
                Some(dir) => save_html_report(&comparison, dir),
                None => {
                    eprintln!("{} --format html requires --output-dir", "error:".red().bold());
                    std::process::exit(1);
                }
            }
        }
        OutputFormat::All => {
            comparison.print_text();
            if let Some(ref dir) = args.output_dir {
                save_comparison_json(&comparison, dir);
                save_html_report(&comparison, dir);
            }
        }
    }

    // Exit non-zero if any agent had failures (for CI).
    let any_failed = comparison.leaderboard.iter()
        .any(|r| r.passed_variations < r.total_variations);
    if !args.dry_run && any_failed {
        std::process::exit(1);
    }
}

// ─── `report` subcommand ──────────────────────────────────────────────────────

fn run_report(args: ReportArgs) {
    let mut reports: Vec<EvalReport> = Vec::new();

    for path in &args.from {
        let loaded = load_reports_from_path(path);
        if loaded.is_empty() {
            eprintln!("{} No valid EvalReport JSON files found in '{}'", "warning:".yellow(), path.display());
        }
        reports.extend(loaded);
    }

    if reports.is_empty() {
        eprintln!("{} No reports loaded — nothing to compare.", "error:".red().bold());
        std::process::exit(1);
    }

    // Deduplicate by run_id so --from A --from A doesn't double-count.
    let mut seen = std::collections::HashSet::new();
    reports.retain(|r| seen.insert(r.run_id.clone()));

    println!("{} Loaded {} agent report(s)", "info:".cyan(), reports.len());

    let comparison = ComparisonEngine::compare(reports);

    match args.format {
        OutputFormat::Text => {
            comparison.print_text();
        }
        OutputFormat::Json => {
            match &args.output_dir {
                Some(dir) => {
                    std::fs::create_dir_all(dir).ok();
                    save_comparison_json(&comparison, dir);
                    println!("{} Comparison JSON saved to {}", "ok:".green(), dir.display());
                }
                None => comparison.print_json(),
            }
        }
        OutputFormat::Html => {
            match &args.output_dir {
                Some(dir) => {
                    std::fs::create_dir_all(dir).ok();
                    save_html_report(&comparison, dir);
                }
                None => {
                    eprintln!("{} --format html requires --output-dir", "error:".red().bold());
                    std::process::exit(1);
                }
            }
        }
        OutputFormat::All => {
            comparison.print_text();
            if let Some(ref dir) = args.output_dir {
                std::fs::create_dir_all(dir).ok();
                save_comparison_json(&comparison, dir);
                save_html_report(&comparison, dir);
            }
        }
    }
}

// ─── Terminal progress renderer ───────────────────────────────────────────────

struct ProgressState {
    mp: MultiProgress,
    bars: HashMap<String, ProgressBar>,
    quiet: bool,
}

fn make_terminal_progress(quiet: bool) -> ProgressCallback {
    let state = Arc::new(Mutex::new(ProgressState {
        mp: MultiProgress::new(),
        bars: HashMap::new(),
        quiet,
    }));

    Arc::new(move |event| {
        let mut st = state.lock().unwrap();
        handle_progress_event(&mut st, event);
    })
}

fn handle_progress_event(st: &mut ProgressState, event: ProgressEvent) {
    match event {
        ProgressEvent::ProfileStarted { agent_id, total_variations } => {
            let pb = st.mp.add(ProgressBar::new(total_variations as u64));
            pb.set_style(
                ProgressStyle::with_template(
                    "  {prefix:<26} {bar:28.cyan/238} {pos:>3}/{len:>3} {percent:>3}%  {elapsed_precise}",
                )
                .unwrap_or_else(|_| ProgressStyle::default_bar())
                .progress_chars("█▉▊▋▌▍▎▏ "),
            );
            let prefix = truncate_id(&agent_id, 24);
            pb.set_prefix(prefix);
            st.bars.insert(agent_id, pb);
        }

        ProgressEvent::VariationDone {
            agent_id,
            scenario_id,
            variation_id,
            passed,
            score,
            duration_ms,
        } => {
            if let Some(pb) = st.bars.get(&agent_id) {
                if !st.quiet {
                    let icon = if passed { "✓".green() } else { "✗".red() };
                    let msg = format!(
                        "    {} {:<20}/{:<3}  {:>5.1}%  {:.1}s",
                        icon,
                        scenario_id,
                        variation_id,
                        score * 100.0,
                        duration_ms as f64 / 1000.0,
                    );
                    pb.println(msg);
                }
                pb.inc(1);
            }
        }

        ProgressEvent::ProfileFinished { agent_id, report } => {
            if let Some(pb) = st.bars.get(&agent_id) {
                let s = &report.summary;
                let score_pct = s.overall_score * 100.0;
                let score_str = format!("{:.1}%", score_pct);
                let colored = if score_pct >= 85.0 {
                    score_str.green().bold().to_string()
                } else if score_pct >= 60.0 {
                    score_str.yellow().bold().to_string()
                } else {
                    score_str.red().bold().to_string()
                };
                pb.finish_with_message(format!(
                    "  {colored}  ({}/{})",
                    s.passed_variations, s.total_variations,
                ));
            }
        }

        ProgressEvent::RateLimitRetry {
            agent_id,
            scenario_id,
            variation_id,
            attempt,
            max_attempts,
            wait_secs,
        } => {
            let msg = format!(
                "    {} {}/{} rate-limited (429) — waiting {}s  [retry {}/{}]",
                "⏸".yellow(),
                scenario_id,
                variation_id,
                wait_secs,
                attempt,
                max_attempts - 1,
            );
            if let Some(pb) = st.bars.get(&agent_id) {
                pb.println(msg);
            } else {
                st.mp.println(msg).ok();
            }
        }

        ProgressEvent::ProfileSkipped { agent_id, reason } => {
            st.mp.println(format!(
                "  {} {:<26} — {}", "⊘".dimmed(), agent_id.dimmed(), reason.dimmed()
            )).ok();
        }

        ProgressEvent::SuiteFinished { elapsed_secs } => {
            st.mp.println(format!(
                "\n  {} Suite finished in {:.1}s",
                "✓".green().bold(),
                elapsed_secs
            )).ok();
        }
    }
}

fn truncate_id(id: &str, max: usize) -> String {
    if id.len() <= max {
        id.to_string()
    } else {
        format!("{}…", &id[..max - 1])
    }
}

// ─── I/O helpers ──────────────────────────────────────────────────────────────

/// Load all `EvalReport` JSON files from a directory, or a single JSON file.
fn load_reports_from_path(path: &std::path::Path) -> Vec<EvalReport> {
    let mut reports = Vec::new();

    if path.is_file() {
        if let Some(r) = try_load_report(path) {
            reports.push(r);
        }
        return reports;
    }

    if path.is_dir() {
        // Collect *.json files sorted by name for deterministic order.
        let mut entries: Vec<_> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                e.path().extension().map(|x| x == "json").unwrap_or(false)
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            if let Some(r) = try_load_report(&entry.path()) {
                reports.push(r);
            }
        }
    }

    reports
}

fn try_load_report(path: &std::path::Path) -> Option<EvalReport> {
    let content = std::fs::read_to_string(path).ok()?;
    // Not an EvalReport — might be a ComparisonReport or other artefact; skip silently.
    serde_json::from_str::<EvalReport>(&content).ok()
}

fn save_comparison_json(comparison: &gestura_core_eval::comparison::ComparisonReport, dir: &std::path::Path) {
    let ts = comparison.timestamp.format("%Y%m%d-%H%M%S").to_string();
    let filename = format!("comparison-{ts}.json");
    let path = dir.join(&filename);
    match serde_json::to_string_pretty(comparison) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, &json) {
                eprintln!("{} Could not write comparison JSON: {e}", "error:".red().bold());
            } else {
                println!("{} Comparison JSON → {}", "saved:".green(), path.display());
            }
        }
        Err(e) => eprintln!("{} Could not serialise comparison: {e}", "error:".red().bold()),
    }
}

fn save_html_report(comparison: &gestura_core_eval::comparison::ComparisonReport, dir: &std::path::Path) {
    let ts = comparison.timestamp.format("%Y%m%d-%H%M%S").to_string();
    let filename = format!("report-{ts}.html");
    let path = dir.join(&filename);
    let html = html_report::generate(comparison);
    if let Err(e) = std::fs::write(&path, html) {
        eprintln!("{} Could not write HTML report: {e}", "error:".red().bold());
    } else {
        println!("{} HTML report   → {}", "saved:".green(), path.display());
    }
}
