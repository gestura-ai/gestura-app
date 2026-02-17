//! CLI Command Implementations
//!
//! Each subcommand has its own module with a `run` function.

pub mod a2a;
pub mod agent;
pub mod agent_info;
pub mod completion;
pub mod config;
pub mod context;
pub mod device;
pub mod exec;
pub mod health;
pub mod init;
pub mod knowledge;
pub mod listen;
pub mod mcp;
pub mod model;
pub mod privacy;
pub mod session;
pub mod spinner;
pub mod tools;

/// Common error type for CLI commands
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
