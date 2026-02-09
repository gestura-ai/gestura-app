//! Gestura CLI - Voice-first AI Assistant
//!
//! Command-line interface providing feature parity with the GUI application.

use clap::{Parser, Subcommand};
use colored::Colorize;

mod commands;

/// Gestura - Voice-first AI Assistant
#[derive(Parser)]
#[command(name = "gestura")]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, global = true, env = "GESTURA_CONFIG")]
    config: Option<std::path::PathBuf>,

    /// Enable verbose output
    #[arg(short, long, global = true, env = "GESTURA_VERBOSE")]
    verbose: bool,

    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Disable colored output
    #[arg(long, global = true, env = "NO_COLOR")]
    no_color: bool,

    /// Output in JSON format
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive AI chat session
    Chat {
        /// Model to use (e.g., gpt-4o, claude-3-5-sonnet)
        #[arg(short, long)]
        model: Option<String>,

        /// Resume a previous session
        #[arg(long)]
        resume: bool,

        /// Session ID to resume
        #[arg(long)]
        session: Option<String>,

        /// Use basic readline mode instead of TUI
        #[arg(long)]
        basic: bool,

        /// Enable voice input
        #[arg(long)]
        voice: bool,

        /// System prompt to use
        #[arg(long)]
        system: Option<String>,
    },

    /// Execute a single prompt (non-interactive)
    Exec {
        /// The prompt to execute
        prompt: Option<String>,

        /// Read prompt from file
        #[arg(short, long)]
        file: Option<std::path::PathBuf>,

        /// Model to use
        #[arg(short, long)]
        model: Option<String>,
    },

    /// Voice input mode
    Listen {
        /// Transcribe only, don't send to LLM
        #[arg(long)]
        transcribe_only: bool,

        /// Whisper model to use
        #[arg(long, default_value = "base")]
        whisper_model: String,
    },

    /// Configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Model management (Whisper, LLM)
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },

    /// Haptic device management
    Device {
        #[command(subcommand)]
        action: DeviceAction,
    },

    /// MCP server management
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },

    /// A2A (Agent-to-Agent) protocol management
    A2a {
        #[command(subcommand)]
        action: A2aAction,
    },

    /// Knowledge system for agent expertise
    Knowledge {
        #[command(subcommand)]
        action: KnowledgeAction,
    },

    /// Smart context management
    Context {
        #[command(subcommand)]
        action: ContextAction,
    },

    /// Session management
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },

    /// Agent interaction
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },

    /// Built-in system tools for agentic workflows
    Tools {
        #[command(subcommand)]
        action: ToolsAction,
    },

    /// GDPR compliance commands
    Privacy {
        #[command(subcommand)]
        action: PrivacyAction,
    },

    /// System health and diagnostics
    Health,

    /// Generate shell completions or man pages
    Completion {
        /// Shell to generate completions for
        #[arg(value_enum, required_unless_present = "generate_man")]
        shell: Option<clap_complete::Shell>,

        /// Generate man pages to the specified directory
        #[arg(long = "generate-man", value_name = "DIR")]
        generate_man: Option<std::path::PathBuf>,
    },

    /// First-time setup wizard
    Init,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Get a configuration value
    Get { key: String },
    /// Set a configuration value
    Set { key: String, value: String },
    /// List all configuration
    List,
    /// Edit configuration in editor
    Edit,
    /// Reset to defaults
    Reset,
}

#[derive(Subcommand)]
enum ModelAction {
    /// Whisper model management
    Whisper {
        #[command(subcommand)]
        action: WhisperAction,
    },
    /// Test LLM connection
    Test {
        /// Provider to test
        provider: Option<String>,
    },
}

#[derive(Subcommand)]
enum WhisperAction {
    /// List available models
    List,
    /// Download a model
    Download { model: String },
    /// Set active model
    Use { model: String },
}

#[derive(Subcommand)]
enum DeviceAction {
    /// List connected devices
    List,
    /// Scan for devices
    Scan,
    /// Connect to a device
    Connect { device_id: String },
    /// Disconnect from a device
    Disconnect { device_id: Option<String> },
}

#[derive(Subcommand)]
enum McpAction {
    /// List configured MCP servers
    List,
    /// Add an MCP server (Claude Code compatible)
    Add {
        /// Server name (unique identifier)
        name: String,
        /// For stdio: the command to run; for http/sse: the URL
        command_or_url: String,
        /// Transport type (stdio, http, sse). Default: auto-detected.
        #[arg(short, long, default_value = "stdio")]
        transport: String,
        /// Configuration scope (user, project, local). Default: user.
        #[arg(short, long, default_value = "user")]
        scope: String,
        /// Environment variables (KEY=VALUE). Stdio only.
        #[arg(short, long, value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// HTTP headers (Header: Value). Http/SSE only.
        #[arg(long, value_name = "HEADER")]
        header: Vec<String>,
        /// Additional args passed to the command. Stdio only.
        /// Everything after `--` is treated as command args.
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Add an MCP server from a raw JSON string (Claude Code compatible)
    AddJson {
        /// Server name
        name: String,
        /// Raw JSON config string
        json: String,
    },
    /// Get detailed info for a specific MCP server
    Get {
        /// Server name
        name: String,
    },
    /// Remove an MCP server
    Remove { name: String },
    /// Enable an MCP server
    Enable { name: String },
    /// Disable an MCP server
    Disable { name: String },
    /// Show MCP protocol status and capabilities
    Status,
    /// List available prompts
    Prompts,
    /// Show server capabilities
    Capabilities,
}

#[derive(Subcommand)]
enum A2aAction {
    /// Show A2A protocol status
    Status,
    /// List registered agent profiles
    Profiles,
    /// Discover a remote agent
    Discover {
        /// Agent URL to discover
        url: String,
    },
    /// Register a new agent profile
    Register {
        /// Agent ID
        #[arg(long)]
        id: String,
        /// Agent name
        #[arg(long)]
        name: String,
        /// Capabilities (comma-separated)
        #[arg(long)]
        capabilities: Option<String>,
    },
    /// Generate a new auth token for an agent
    Token {
        /// Agent ID to generate token for
        agent_id: String,
        /// Token validity in hours
        #[arg(long, default_value = "24")]
        hours: i64,
    },
    /// Validate a token
    Validate {
        /// Token to validate
        token: String,
    },
    /// List known remote agents
    Agents,
    /// Send a task to a remote agent
    Send {
        /// Agent URL
        #[arg(short, long)]
        url: String,
        /// Message to send
        message: String,
    },
}

#[derive(Subcommand)]
pub enum KnowledgeAction {
    /// List all knowledge items
    List {
        /// Filter by category
        #[arg(short = 'C', long)]
        category: Option<String>,
    },
    /// Show details of a knowledge item
    Show {
        /// Knowledge item ID
        id: String,
    },
    /// Search for knowledge items
    Search {
        /// Search query
        query: String,
        /// Maximum results
        #[arg(short, long, default_value = "5")]
        limit: usize,
    },
    /// List all categories
    Categories,
    /// Show knowledge system status
    Status,
}

#[derive(Subcommand)]
enum ContextAction {
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

#[derive(Subcommand)]
enum SessionAction {
    /// List previous sessions
    List {
        /// Number of sessions to show
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// Resume a previous session
    Resume {
        /// Session ID (or "last" for most recent)
        session: String,
    },
    /// Fork a session into a new conversation
    Fork {
        /// Session ID to fork
        session: String,
    },
    /// Delete a session
    Delete { session: String },
}

#[derive(Subcommand)]
enum AgentAction {
    /// Get agent status
    Status,
    /// Send a message to the agent
    Send {
        /// Message to send
        message: String,
    },
    /// List available agents
    List,
    /// Enable an agent
    Enable {
        /// Agent name
        agent: String,
    },
    /// Disable an agent
    Disable {
        /// Agent name
        agent: String,
    },
    /// Show agent configuration
    Config {
        /// Agent name
        agent: String,
    },
}

#[derive(Subcommand)]
enum PrivacyAction {
    /// Export all user data
    Export {
        /// Output file path
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },
    /// Delete all user data
    Delete {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Show data retention policy
    Policy,
}

#[derive(Subcommand)]
enum ToolsAction {
    /// File operations (read, write, edit, search, list, tree)
    File {
        #[command(subcommand)]
        action: FileToolAction,
    },
    /// Shell command execution
    Shell {
        #[command(subcommand)]
        action: ShellToolAction,
    },
    /// Git operations
    Git {
        #[command(subcommand)]
        action: GitToolAction,
    },
    /// Code analysis
    Code {
        #[command(subcommand)]
        action: CodeToolAction,
    },
    /// Web fetching
    Web {
        #[command(subcommand)]
        action: WebToolAction,
    },
    /// Permission management
    Permissions {
        #[command(subcommand)]
        action: PermissionsToolAction,
    },
    /// Screen capture and recording
    Screen {
        #[command(subcommand)]
        action: ScreenToolAction,
    },
}

#[derive(Subcommand)]
enum FileToolAction {
    /// Read file contents
    Read {
        /// File path
        path: std::path::PathBuf,
        /// Line range (e.g., "1-10" or "5")
        #[arg(short, long)]
        lines: Option<String>,
    },
    /// Write content to file
    Write {
        /// File path
        path: std::path::PathBuf,
        /// Content to write
        content: String,
    },
    /// Edit file with str_replace
    Edit {
        /// File path
        path: std::path::PathBuf,
        /// String to replace
        #[arg(long)]
        old_str: String,
        /// Replacement string
        #[arg(long)]
        new_str: String,
    },
    /// Search for pattern in files
    Search {
        /// Regex pattern
        pattern: String,
        /// Path to search in
        #[arg(short, long)]
        path: Option<std::path::PathBuf>,
        /// Recursive search
        #[arg(short, long)]
        recursive: bool,
    },
    /// List files in directory
    List {
        /// Directory path
        path: Option<std::path::PathBuf>,
        /// Show hidden files
        #[arg(short, long)]
        all: bool,
    },
    /// Show directory tree
    Tree {
        /// Directory path
        path: Option<std::path::PathBuf>,
        /// Maximum depth
        #[arg(short, long)]
        depth: Option<usize>,
    },
    /// Add files to chat context
    Add {
        /// Files to add
        paths: Vec<std::path::PathBuf>,
    },
    /// Remove files from chat context
    Drop {
        /// Files to remove
        paths: Vec<std::path::PathBuf>,
    },
    /// Show current file context
    Context,
}

#[derive(Subcommand)]
enum ShellToolAction {
    /// Run a shell command
    Run {
        /// Command to run
        command: String,
        /// Timeout in seconds
        #[arg(short, long)]
        timeout: Option<u64>,
        /// Suppress output formatting
        #[arg(short, long)]
        quiet: bool,
    },
    /// Test a command without executing
    Test {
        /// Command to test
        command: String,
    },
    /// Show command history
    History {
        /// Number of commands to show
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// Show last command output
    Last,
}

#[derive(Subcommand)]
enum GitToolAction {
    /// Show git status
    Status,
    /// Show git diff
    Diff {
        /// File path
        path: Option<String>,
        /// Show staged changes
        #[arg(long)]
        staged: bool,
    },
    /// Show git log
    Log {
        /// Number of commits
        #[arg(short = 'n', long)]
        count: Option<usize>,
        /// One line per commit
        #[arg(long)]
        oneline: bool,
    },
    /// Create a commit
    Commit {
        /// Commit message
        #[arg(short, long)]
        message: Option<String>,
        /// Stage all changes
        #[arg(short, long)]
        all: bool,
    },
    /// Undo last commit
    Undo {
        /// Keep changes staged
        #[arg(long)]
        soft: bool,
    },
    /// Branch operations
    Branch {
        /// Branch name
        name: Option<String>,
        /// Delete branch
        #[arg(short, long)]
        delete: bool,
    },
    /// Checkout branch or file
    Checkout {
        /// Target branch or file
        target: String,
    },
    /// Stash operations
    Stash {
        /// Action (push, pop, list, drop)
        action: Option<String>,
    },
    /// Show file blame
    Blame {
        /// File path
        path: String,
    },
    /// Show merge conflicts
    Conflicts,
    /// Resolve conflicts
    Resolve {
        /// File path
        path: String,
        /// Resolution strategy (ours, theirs)
        #[arg(long)]
        strategy: Option<String>,
    },
}

#[derive(Subcommand)]
enum CodeToolAction {
    /// Generate repository map
    Map {
        /// Path to analyze
        path: Option<std::path::PathBuf>,
        /// Maximum depth
        #[arg(short, long)]
        depth: Option<usize>,
    },
    /// List symbols in file
    Symbols {
        /// File path
        path: std::path::PathBuf,
    },
    /// Find references to symbol
    References {
        /// Symbol name
        symbol: String,
        /// Path to search
        #[arg(short, long)]
        path: Option<std::path::PathBuf>,
    },
    /// Find symbol definition
    Definition {
        /// Symbol name
        symbol: String,
        /// Path to search
        #[arg(short, long)]
        path: Option<std::path::PathBuf>,
    },
    /// Run linter
    Lint {
        /// Path to lint
        path: Option<std::path::PathBuf>,
        /// Auto-fix issues
        #[arg(long)]
        fix: bool,
    },
    /// Run tests
    Test {
        /// Path to test
        path: Option<std::path::PathBuf>,
        /// Test filter
        #[arg(short, long)]
        filter: Option<String>,
    },
    /// Show dependencies
    Deps {
        /// Path to analyze
        path: Option<std::path::PathBuf>,
    },
    /// Show code statistics
    Stats {
        /// Path to analyze
        path: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum WebToolAction {
    /// Fetch URL and convert to text
    Fetch {
        /// URL to fetch
        url: String,
        /// CSS selector to extract
        #[arg(short, long)]
        selector: Option<String>,
        /// Exclude images
        #[arg(long)]
        no_images: bool,
    },
    /// Search the web
    Search {
        /// Search query
        query: String,
        /// Number of results
        #[arg(short, long)]
        num_results: Option<usize>,
    },
    /// Capture webpage screenshot
    Screenshot {
        /// URL to capture
        url: String,
        /// Output file
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
enum PermissionsToolAction {
    /// List current permissions
    List,
    /// Grant a permission
    Grant {
        /// Permission name
        permission: String,
        /// Scope (e.g., directory path)
        #[arg(short, long)]
        scope: Option<String>,
    },
    /// Revoke a permission
    Revoke {
        /// Permission name
        permission: String,
    },
    /// Reset to defaults
    Reset,
    /// Check if action is allowed
    Check {
        /// Action to check
        action: String,
        /// Target (e.g., file path)
        target: Option<String>,
    },
}

#[derive(Subcommand)]
enum ScreenToolAction {
    /// Capture a screenshot
    Capture {
        /// Output path for screenshot
        path: std::path::PathBuf,
        /// Region to capture (format: x,y,width,height)
        #[arg(short, long)]
        region: Option<String>,
        /// Display number (0 = primary)
        #[arg(short, long)]
        display: Option<u32>,
    },
    /// Start screen recording
    RecordStart {
        /// Output path for recording
        path: std::path::PathBuf,
        /// Region to record (format: x,y,width,height)
        #[arg(short, long)]
        region: Option<String>,
        /// Display number (0 = primary)
        #[arg(short, long)]
        display: Option<u32>,
    },
    /// Stop screen recording
    RecordStop {
        /// Recording ID to stop
        recording_id: String,
    },
}

fn main() {
    let cli = Cli::parse();

    // Respect `--no-color` / `NO_COLOR` across output helpers (`colored`, `console`).
    if cli.no_color {
        colored::control::set_override(false);
        console::set_colors_enabled(false);
    }

    // The TUI uses an alternate screen; any writes to stdout/stderr from tracing can corrupt the
    // layout. When running `chat` in TUI mode, we default tracing output to a sink.
    let is_tui_chat = matches!(&cli.command, Some(Commands::Chat { basic, .. }) if !*basic);

    // Initialize logging
    if cli.verbose {
        if is_tui_chat {
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(std::io::sink)
                .init();
        } else {
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .init();
        }
    } else if !cli.quiet {
        if is_tui_chat {
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::INFO)
                .with_target(false)
                .with_writer(std::io::sink)
                .init();
        } else {
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::INFO)
                .with_target(false)
                .init();
        }
    }

    // Handle commands
    let result = match &cli.command {
        Some(Commands::Chat {
            model,
            resume,
            session,
            basic,
            voice,
            system,
        }) => commands::chat::run(commands::chat::ChatOptions {
            model: model.as_deref(),
            resume: *resume,
            session: session.as_deref(),
            tui: !*basic, // TUI is default, --basic disables it
            voice: *voice,
            system: system.as_deref(),
        }),
        Some(Commands::Exec {
            prompt,
            file,
            model,
        }) => commands::exec::run(prompt.as_deref(), file.as_deref(), model.as_deref()),
        Some(Commands::Listen {
            transcribe_only,
            whisper_model,
        }) => commands::listen::run(*transcribe_only, whisper_model),
        Some(Commands::Config { action }) => commands::config::run(action),
        Some(Commands::Model { action }) => commands::model::run(action),
        Some(Commands::Device { action }) => commands::device::run(action),
        Some(Commands::Mcp { action }) => commands::mcp::run(action),
        Some(Commands::A2a { action }) => commands::a2a::run(action),
        Some(Commands::Knowledge { action }) => commands::knowledge::run(action),
        Some(Commands::Context { action }) => {
            use commands::context::ContextAction as CA;
            let ctx_action = match action {
                ContextAction::Analyze { request } => CA::Analyze {
                    request: request.clone(),
                },
                ContextAction::Status => CA::Status,
                ContextAction::Categories => CA::Categories,
                ContextAction::Clear => CA::Clear,
            };
            commands::context::run(ctx_action)
        }
        Some(Commands::Session { action }) => commands::session::run(action),
        Some(Commands::Agent { action }) => {
            let subcommand = match action {
                AgentAction::Status => commands::agent::AgentSubcommand::Status,
                AgentAction::Send { message } => commands::agent::AgentSubcommand::Send {
                    message: message.clone(),
                },
                AgentAction::List => commands::agent::AgentSubcommand::List,
                AgentAction::Enable { agent } => commands::agent::AgentSubcommand::Enable {
                    agent: agent.clone(),
                },
                AgentAction::Disable { agent } => commands::agent::AgentSubcommand::Disable {
                    agent: agent.clone(),
                },
                AgentAction::Config { agent } => commands::agent::AgentSubcommand::Config {
                    agent: agent.clone(),
                },
            };
            commands::agent::run(subcommand)
        }
        Some(Commands::Tools { action }) => {
            use commands::tools::*;
            let category = match action {
                ToolsAction::File {
                    action: file_action,
                } => {
                    let cmd = match file_action {
                        FileToolAction::Read { path, lines } => file::FileSubcommand::Read {
                            path: path.clone(),
                            lines: lines.clone(),
                        },
                        FileToolAction::Write { path, content } => file::FileSubcommand::Write {
                            path: path.clone(),
                            content: content.clone(),
                        },
                        FileToolAction::Edit {
                            path,
                            old_str,
                            new_str,
                        } => file::FileSubcommand::Edit {
                            path: path.clone(),
                            old_str: old_str.clone(),
                            new_str: new_str.clone(),
                        },
                        FileToolAction::Search {
                            pattern,
                            path,
                            recursive,
                        } => file::FileSubcommand::Search {
                            pattern: pattern.clone(),
                            path: path.clone(),
                            recursive: *recursive,
                        },
                        FileToolAction::List { path, all } => file::FileSubcommand::List {
                            path: path.clone(),
                            all: *all,
                        },
                        FileToolAction::Tree { path, depth } => file::FileSubcommand::Tree {
                            path: path.clone(),
                            max_depth: *depth,
                        },
                        FileToolAction::Add { paths } => file::FileSubcommand::Add {
                            paths: paths.clone(),
                        },
                        FileToolAction::Drop { paths } => file::FileSubcommand::Drop {
                            paths: paths.clone(),
                        },
                        FileToolAction::Context => file::FileSubcommand::Context,
                    };
                    ToolsCategory::File(cmd)
                }
                ToolsAction::Shell {
                    action: shell_action,
                } => {
                    let cmd = match shell_action {
                        ShellToolAction::Run {
                            command,
                            timeout,
                            quiet,
                        } => shell::ShellSubcommand::Run {
                            command: command.clone(),
                            timeout: *timeout,
                            quiet: *quiet,
                        },
                        ShellToolAction::Test { command } => shell::ShellSubcommand::Test {
                            command: command.clone(),
                        },
                        ShellToolAction::History { limit } => {
                            shell::ShellSubcommand::History { limit: *limit }
                        }
                        ShellToolAction::Last => shell::ShellSubcommand::Last,
                    };
                    ToolsCategory::Shell(cmd)
                }
                ToolsAction::Git { action: git_action } => {
                    let cmd = match git_action {
                        GitToolAction::Status => git::GitSubcommand::Status,
                        GitToolAction::Diff { path, staged } => git::GitSubcommand::Diff {
                            path: path.clone(),
                            staged: *staged,
                        },
                        GitToolAction::Log { count, oneline } => git::GitSubcommand::Log {
                            count: *count,
                            oneline: *oneline,
                        },
                        GitToolAction::Commit { message, all } => git::GitSubcommand::Commit {
                            message: message.clone(),
                            all: *all,
                        },
                        GitToolAction::Undo { soft } => git::GitSubcommand::Undo { soft: *soft },
                        GitToolAction::Branch { name, delete } => git::GitSubcommand::Branch {
                            name: name.clone(),
                            delete: *delete,
                        },
                        GitToolAction::Checkout { target } => git::GitSubcommand::Checkout {
                            target: target.clone(),
                        },
                        GitToolAction::Stash { action } => git::GitSubcommand::Stash {
                            action: action.clone(),
                        },
                        GitToolAction::Blame { path } => {
                            git::GitSubcommand::Blame { path: path.clone() }
                        }
                        GitToolAction::Conflicts => git::GitSubcommand::Conflicts,
                        GitToolAction::Resolve { path, strategy } => git::GitSubcommand::Resolve {
                            path: path.clone(),
                            strategy: strategy.clone(),
                        },
                    };
                    ToolsCategory::Git(cmd)
                }
                ToolsAction::Code {
                    action: code_action,
                } => {
                    let cmd = match code_action {
                        CodeToolAction::Map { path, depth } => code::CodeSubcommand::Map {
                            path: path.clone(),
                            depth: *depth,
                        },
                        CodeToolAction::Symbols { path } => {
                            code::CodeSubcommand::Symbols { path: path.clone() }
                        }
                        CodeToolAction::References { symbol, path } => {
                            code::CodeSubcommand::References {
                                symbol: symbol.clone(),
                                path: path.clone(),
                            }
                        }
                        CodeToolAction::Definition { symbol, path } => {
                            code::CodeSubcommand::Definition {
                                symbol: symbol.clone(),
                                path: path.clone(),
                            }
                        }
                        CodeToolAction::Lint { path, fix } => code::CodeSubcommand::Lint {
                            path: path.clone(),
                            fix: *fix,
                        },
                        CodeToolAction::Test { path, filter } => code::CodeSubcommand::Test {
                            path: path.clone(),
                            filter: filter.clone(),
                        },
                        CodeToolAction::Deps { path } => {
                            code::CodeSubcommand::Deps { path: path.clone() }
                        }
                        CodeToolAction::Stats { path } => {
                            code::CodeSubcommand::Stats { path: path.clone() }
                        }
                    };
                    ToolsCategory::Code(cmd)
                }
                ToolsAction::Web { action: web_action } => {
                    let cmd = match web_action {
                        WebToolAction::Fetch {
                            url,
                            selector,
                            no_images,
                        } => web::WebSubcommand::Fetch {
                            url: url.clone(),
                            selector: selector.clone(),
                            no_images: *no_images,
                        },
                        WebToolAction::Search { query, num_results } => {
                            web::WebSubcommand::Search {
                                query: query.clone(),
                                num_results: *num_results,
                            }
                        }
                        WebToolAction::Screenshot { url, output } => {
                            web::WebSubcommand::Screenshot {
                                url: url.clone(),
                                output: output.clone(),
                            }
                        }
                    };
                    ToolsCategory::Web(cmd)
                }
                ToolsAction::Permissions {
                    action: perm_action,
                } => {
                    let cmd = match perm_action {
                        PermissionsToolAction::List => permissions::PermissionsSubcommand::List,
                        PermissionsToolAction::Grant { permission, scope } => {
                            permissions::PermissionsSubcommand::Grant {
                                permission: permission.clone(),
                                scope: scope.clone(),
                            }
                        }
                        PermissionsToolAction::Revoke { permission } => {
                            permissions::PermissionsSubcommand::Revoke {
                                permission: permission.clone(),
                            }
                        }
                        PermissionsToolAction::Reset => permissions::PermissionsSubcommand::Reset,
                        PermissionsToolAction::Check { action, target } => {
                            permissions::PermissionsSubcommand::Check {
                                action: action.clone(),
                                target: target.clone(),
                            }
                        }
                    };
                    ToolsCategory::Permissions(cmd)
                }
                ToolsAction::Screen {
                    action: screen_action,
                } => {
                    let cmd = match screen_action {
                        ScreenToolAction::Capture {
                            path,
                            region,
                            display,
                        } => screen::ScreenSubcommand::Capture {
                            path: path.clone(),
                            region: region.clone(),
                            display: *display,
                        },
                        ScreenToolAction::RecordStart {
                            path,
                            region,
                            display,
                        } => screen::ScreenSubcommand::RecordStart {
                            path: path.clone(),
                            region: region.clone(),
                            display: *display,
                        },
                        ScreenToolAction::RecordStop { recording_id } => {
                            screen::ScreenSubcommand::RecordStop {
                                recording_id: recording_id.clone(),
                            }
                        }
                    };
                    ToolsCategory::Screen(cmd)
                }
            };
            run(category)
        }
        Some(Commands::Privacy { action }) => commands::privacy::run(action),
        Some(Commands::Health) => commands::health::run(),
        Some(Commands::Completion {
            shell,
            generate_man,
        }) => {
            if let Some(dir) = generate_man {
                commands::completion::generate_man_pages(dir).map_err(
                    |e| -> Box<dyn std::error::Error> {
                        format!("Failed to generate man pages: {}", e).into()
                    },
                )
            } else if let Some(s) = shell {
                commands::completion::run(*s);
                Ok(())
            } else {
                Ok(())
            }
        }
        Some(Commands::Init) => commands::init::run(),
        None => {
            // Check if this is the first run - if so, run onboarding first
            if gestura_core::AppConfig::is_first_run() {
                println!(
                    "{}",
                    "Welcome! It looks like this is your first time running Gestura."
                        .cyan()
                        .bold()
                );
                println!();

                // Run the onboarding wizard
                if let Err(e) = commands::init::run() {
                    eprintln!("{} Failed to complete setup: {}", "warning:".yellow(), e);
                    eprintln!("You can run 'gestura init' later to complete setup.");
                    println!();
                }
            }

            // Default to interactive TUI chat if no command specified
            commands::chat::run(commands::chat::ChatOptions {
                tui: true,
                ..Default::default()
            })
        }
    };

    // Handle errors
    if let Err(e) = result {
        if cli.json {
            let error = serde_json::json!({
                "error": true,
                "message": e.to_string()
            });
            eprintln!("{}", serde_json::to_string_pretty(&error).unwrap());
        } else {
            eprintln!("{} {}", "error:".red().bold(), e);
        }
        std::process::exit(1);
    }
}
