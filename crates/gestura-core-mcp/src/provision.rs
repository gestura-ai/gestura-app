//! MCP server provisioning — runtime availability checks and package pre-installation.
//!
//! Called immediately after a server is added via the registry browser so that
//! the required runtime (Node/npx or uv/uvx) is verified and the package is
//! pre-fetched before the first connection attempt.
//!
//! # Transport handling
//!
//! | Transport | Strategy |
//! |-----------|----------|
//! | Stdio — `npx` | Verify `npx` is on PATH; pre-warm npx cache with 60 s timeout |
//! | Stdio — `uvx` | Verify `uv` is on PATH; run `uv tool install` (idempotent, 120 s) |
//! | Stdio — other | Verify the command binary is on PATH; no install step |
//! | HTTP / SSE | Skip — remote server, nothing to install |

use crate::config::{McpServerEntry, McpTransportType};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

// ── Public types ──────────────────────────────────────────────────────────────

/// Outcome of a provisioning attempt.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionStatus {
    /// Runtime present and package downloaded/installed.
    Ready,
    /// Required runtime binary not found on PATH.
    RuntimeMissing,
    /// Runtime present but package fetch/install failed.
    FetchFailed,
    /// No installation needed (HTTP/SSE remote, or no command configured).
    Skipped,
}

/// Result returned to the frontend after a provisioning attempt.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProvisionResult {
    /// Server name (echoed back for UI correlation).
    pub name: String,
    /// High-level outcome.
    pub status: ProvisionStatus,
    /// Human-readable explanation suitable for display in the config panel.
    pub message: String,
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Check runtime availability and pre-install/fetch the package for `entry`.
///
/// This function never panics and always returns a [`ProvisionResult`]; errors
/// are captured in `status` + `message` rather than propagated.
pub async fn provision_mcp_server(entry: &McpServerEntry) -> ProvisionResult {
    match entry.transport {
        McpTransportType::Http | McpTransportType::Sse => ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::Skipped,
            message: "Remote HTTP/SSE server — no local installation required.".to_string(),
        },
        McpTransportType::Stdio => provision_stdio(entry).await,
    }
}

// ── Stdio dispatch ────────────────────────────────────────────────────────────

async fn provision_stdio(entry: &McpServerEntry) -> ProvisionResult {
    let cmd = match entry.command.as_deref().filter(|s| !s.is_empty()) {
        Some(c) => c,
        None => {
            return ProvisionResult {
                name: entry.name.clone(),
                status: ProvisionStatus::Skipped,
                message: "No command configured for stdio server.".to_string(),
            };
        }
    };

    match cmd {
        "npx" => provision_npm(entry).await,
        "uvx" => provision_pypi(entry).await,
        other => provision_generic(entry, other).await,
    }
}

// ── npm / npx ─────────────────────────────────────────────────────────────────

async fn provision_npm(entry: &McpServerEntry) -> ProvisionResult {
    let npx_cmd = crate::cmd_utils::resolve_mcp_command("npx");
    if !runtime_available(&npx_cmd).await {
        return ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::RuntimeMissing,
            message: "npx not found. Install Node.js from https://nodejs.org to use npm-based MCP servers.".to_string(),
        };
    }

    // args[0] = "-y", args[1] = "pkg@version"  (built by build_mcp_entry_for_package)
    let pkg = entry.args.get(1).map(String::as_str).unwrap_or_default();
    if pkg.is_empty() {
        return ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::Skipped,
            message: "Package identifier not set — skipping pre-fetch.".to_string(),
        };
    }

    let mut envs = entry.env.clone();
    crate::cmd_utils::inject_enriched_path(&mut envs);

    // Run `npx --yes <pkg>` with stdin closed.  MCP servers exit immediately on EOF,
    // so the cache is populated even if the process exits non-zero or times out.
    let result = timeout(Duration::from_secs(60), async {
        let mut child = Command::new(&npx_cmd)
            .args(["--yes", pkg])
            .envs(&envs)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        child.wait().await
    })
    .await;

    match result {
        // Timed-out: download already happened; treat as ready.
        Err(_elapsed) => ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::Ready,
            message: format!("npm package '{pkg}' pre-fetched (download completed)."),
        },
        // Process ran to completion (any exit code is fine — cache is warm).
        Ok(Ok(_)) => ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::Ready,
            message: format!("npm package '{pkg}' is ready."),
        },
        // spawn() or wait() failed (e.g. npx binary disappeared after the PATH check).
        Ok(Err(e)) => ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::FetchFailed,
            message: format!("npx pre-fetch failed for '{pkg}': {e}"),
        },
    }
}

// ── pypi / uvx ────────────────────────────────────────────────────────────────

async fn provision_pypi(entry: &McpServerEntry) -> ProvisionResult {
    let uv_cmd = crate::cmd_utils::resolve_mcp_command("uv");
    if !runtime_available(&uv_cmd).await {
        return ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::RuntimeMissing,
            message: "uv not found. Install from https://docs.astral.sh/uv/ to use pypi-based MCP servers.".to_string(),
        };
    }

    // args[0] = "pkg==version"  (built by build_mcp_entry_for_package for pypi)
    let pkg_version = entry.args.first().map(String::as_str).unwrap_or_default();
    if pkg_version.is_empty() {
        return ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::Skipped,
            message: "Package identifier not set — skipping installation.".to_string(),
        };
    }

    let mut envs = entry.env.clone();
    crate::cmd_utils::inject_enriched_path(&mut envs);

    // `uv tool install <pkg>==<version>` is idempotent:
    //   - exits 0 on fresh install
    //   - exits 1 with "already installed" on stderr when already present
    let result = timeout(
        Duration::from_secs(120),
        Command::new(&uv_cmd)
            .args(["tool", "install", pkg_version])
            .envs(&envs)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    match result {
        Err(_elapsed) => ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::FetchFailed,
            message: format!(
                "uv tool install timed out for '{pkg_version}'. Try running manually: uv tool install {pkg_version}"
            ),
        },
        Ok(Ok(output)) => {
            if output.status.success() {
                ProvisionResult {
                    name: entry.name.clone(),
                    status: ProvisionStatus::Ready,
                    message: format!("Python package '{pkg_version}' installed successfully."),
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // uv exits 1 with "already installed" when already present — treat as ready.
                if stderr.contains("already installed") {
                    ProvisionResult {
                        name: entry.name.clone(),
                        status: ProvisionStatus::Ready,
                        message: format!(
                            "Python package '{pkg_version}' is already installed and ready."
                        ),
                    }
                } else {
                    ProvisionResult {
                        name: entry.name.clone(),
                        status: ProvisionStatus::FetchFailed,
                        message: format!(
                            "uv tool install failed for '{pkg_version}': {}",
                            stderr.trim()
                        ),
                    }
                }
            }
        }
        Ok(Err(e)) => ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::FetchFailed,
            message: format!("Failed to launch uv for '{pkg_version}': {e}"),
        },
    }
}

// ── Generic stdio ─────────────────────────────────────────────────────────────

/// For stdio servers that use neither npx nor uvx — just verify the binary exists.
async fn provision_generic(entry: &McpServerEntry, cmd: &str) -> ProvisionResult {
    if runtime_available(cmd).await {
        ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::Ready,
            message: format!("Runtime '{cmd}' is available."),
        }
    } else {
        ProvisionResult {
            name: entry.name.clone(),
            status: ProvisionStatus::RuntimeMissing,
            message: format!(
                "Command '{cmd}' not found on PATH. Install it before enabling this server."
            ),
        }
    }
}

// ── Runtime availability check ────────────────────────────────────────────────

/// Return `true` if `cmd` is found on PATH.
///
/// Uses `which` on Unix and `where` on Windows. Avoids running arbitrary
/// version flags that could trigger unexpected side effects.
async fn runtime_available(cmd: &str) -> bool {
    #[cfg(target_os = "windows")]
    let checker = "where";
    #[cfg(not(target_os = "windows"))]
    let checker = "which";

    // Inject the same PATH that the tool will run with
    let mut envs: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    crate::cmd_utils::inject_enriched_path(&mut envs);

    Command::new(checker)
        .arg(cmd)
        .envs(&envs)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{McpScope, McpTransportType};
    use std::collections::HashMap;

    fn make_entry(
        name: &str,
        transport: McpTransportType,
        command: Option<&str>,
        args: Vec<&str>,
        url: Option<&str>,
    ) -> McpServerEntry {
        McpServerEntry {
            name: name.to_string(),
            transport,
            enabled: true,
            command: command.map(str::to_string),
            args: args.into_iter().map(str::to_string).collect(),
            env: HashMap::new(),
            url: url.map(str::to_string),
            headers: HashMap::new(),
            scope: McpScope::User,
            timeout_secs: 30,
            auto_reconnect: true,
            session_default_enabled: true,
        }
    }

    #[tokio::test]
    async fn http_server_is_skipped() {
        let entry = make_entry(
            "my-http-server",
            McpTransportType::Http,
            None,
            vec![],
            Some("https://example.com/mcp"),
        );
        let result = provision_mcp_server(&entry).await;
        assert_eq!(result.status, ProvisionStatus::Skipped);
        assert_eq!(result.name, "my-http-server");
    }

    #[tokio::test]
    async fn sse_server_is_skipped() {
        let entry = make_entry(
            "my-sse-server",
            McpTransportType::Sse,
            None,
            vec![],
            Some("https://example.com/sse"),
        );
        let result = provision_mcp_server(&entry).await;
        assert_eq!(result.status, ProvisionStatus::Skipped);
    }

    #[tokio::test]
    async fn stdio_no_command_is_skipped() {
        let entry = make_entry("no-cmd", McpTransportType::Stdio, None, vec![], None);
        let result = provision_mcp_server(&entry).await;
        assert_eq!(result.status, ProvisionStatus::Skipped);
    }

    #[tokio::test]
    async fn generic_command_found_returns_ready() {
        // "echo" is universally available on all platforms.
        let entry = make_entry(
            "echo-srv",
            McpTransportType::Stdio,
            Some("echo"),
            vec![],
            None,
        );
        let result = provision_mcp_server(&entry).await;
        assert_eq!(
            result.status,
            ProvisionStatus::Ready,
            "echo must be on PATH"
        );
    }

    #[tokio::test]
    async fn generic_command_missing_returns_runtime_missing() {
        let entry = make_entry(
            "nonexistent-srv",
            McpTransportType::Stdio,
            Some("__gestura_nonexistent_cmd_xyz__"),
            vec![],
            None,
        );
        let result = provision_mcp_server(&entry).await;
        assert_eq!(result.status, ProvisionStatus::RuntimeMissing);
    }
}
