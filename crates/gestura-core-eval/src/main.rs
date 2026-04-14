//! `gestura-eval` — standalone evaluation binary.
//!
//! Tests agent CLIs as black-box subprocesses across 8 standardised scenarios.
//! Supports multiple agentic interfaces (Gestura, Claude Code, Augment, Codex, OpenCode)
//! via TOML agent profiles. The binary is intentionally separate from the `gestura` CLI
//! so evaluation logic never ships inside the product binary.
//!
//! # Usage
//!
//! ```bash
//! # Build the eval binary (and gestura-cli if testing gestura)
//! cargo build -p gestura-cli -p gestura-core-eval
//!
//! # List available scenario IDs
//! ./target/debug/gestura-eval --list
//!
//! # List built-in agent profiles
//! ./target/debug/gestura-eval --list-agents
//!
//! # Dry-run: validate check logic only (no LLM calls)
//! ./target/debug/gestura-eval --dry-run
//!
//! # Full run with the default gestura-full profile
//! ./target/debug/gestura-eval
//!
//! # Run against a specific agent profile
//! ./target/debug/gestura-eval --agent claude-code-full
//! ./target/debug/gestura-eval --agent codex-sandboxed
//!
//! # Load a custom agent profile from disk
//! ./target/debug/gestura-eval --config /path/to/my-agent.toml
//!
//! # Single scenario, JSON output
//! ./target/debug/gestura-eval --agent augment-full --scenario s3_planning --json
//!
//! # Override the binary path
//! ./target/debug/gestura-eval --agent gestura-full --bin /path/to/gestura
//!
//! # Save report for cross-tool comparison
//! ./target/debug/gestura-eval --agent codex-full --json > eval-codex-full.json
//! ```

use std::path::PathBuf;

use clap::Parser;
use colored::Colorize;
use gestura_core_eval::{
    config::{EvalConfig, BUILTIN_AGENT_IDS},
    CliEvalRunner, CliRunnerOptions, EvalScenarioSuite,
};

/// Gestura evaluation harness — runs standardised scenarios against any agent CLI.
#[derive(Parser, Debug)]
#[command(name = "gestura-eval", author, version, about, long_about = None)]
struct Args {
    /// Built-in agent profile to use (see --list-agents for IDs).
    /// Defaults to `gestura-full` when neither --agent nor --config is given.
    #[arg(long, value_name = "AGENT_ID", env = "GESTURA_EVAL_AGENT")]
    agent: Option<String>,

    /// Path to a custom agent profile TOML file (merged on top of baseline).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Override the agent binary path regardless of what the profile specifies.
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

    /// Suppress progress output (implies --json for machine consumers).
    #[arg(long, short)]
    quiet: bool,

    /// Show the agent's full response and all check results (pass and fail)
    /// for every variation, not just failures.  Useful for debugging or
    /// understanding why a passing score does not match expectations.
    /// In non-verbose mode, the response is still shown (truncated) for
    /// failed variations.
    #[arg(long, short)]
    verbose: bool,
}

fn main() {
    let args = Args::parse();

    // Init tracing — emit warnings+ to stderr; eval output goes to stdout.
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let suite = EvalScenarioSuite::load_builtin();

    // --list: print scenario IDs and exit.
    if args.list {
        println!("{}", "Available scenarios:".bold());
        for s in &suite.scenarios {
            println!("  {:<28} {}", s.id.cyan(), s.description.dimmed());
        }
        return;
    }

    // --list-agents: print built-in agent profile IDs and exit.
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

    // Guard: profiles that require manual auth (OAuth session tokens, not static API keys)
    // are excluded from fully automated runs. Fail fast rather than timing out mid-run.
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

    let opts = CliRunnerOptions {
        eval_config: eval_config.clone(),
        scenario_ids: args.scenario.as_ref().map(|id| vec![id.clone()]).unwrap_or_default(),
        dry_run: args.dry_run,
        bin_override: args.bin_override,
    };

    let bin = opts.eval_config.resolve_bin(opts.bin_override.as_ref());

    // Announce what we're doing.
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
        report.print_text(args.verbose);

        let s = &report.summary;
        if s.failed_variations > 0 && !args.dry_run {
            eprintln!(
                "\n{} {}/{} variations failed. Re-run with --verbose for full response context or --json for machine-readable details.",
                "note:".yellow(),
                s.failed_variations,
                s.total_variations
            );
        }
    }

    // Exit non-zero when any variation fails (enables CI integration).
    if !args.dry_run && report.summary.failed_variations > 0 {
        std::process::exit(1);
    }
}

