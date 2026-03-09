//! MCP Manager — Search, evaluate, install, and manage MCP servers
//!
//! Queries the official MCP registry at <https://registry.modelcontextprotocol.io/v0/servers>
//! to discover servers by keyword, evaluate them in detail, and install them into the local
//! `.mcp.json` configuration (Claude Desktop / Gestura compatible format).
//!
//! No dependency on `gestura-core-mcp` is introduced here (circular-dep prevention).
//! Config types are defined locally and serialize to the same `.mcp.json` wire format.

use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ── Registry API ─────────────────────────────────────────────────────────────

const REGISTRY_BASE: &str = "https://registry.modelcontextprotocol.io/v0";

// ── Registry API response types ───────────────────────────────────────────────

/// Top-level list response from the registry.
#[derive(Debug, Deserialize)]
struct RegistryListResponse {
    servers: Vec<RegistryEntry>,
    metadata: Option<RegistryMetadata>,
}

#[derive(Debug, Deserialize)]
struct RegistryMetadata {
    #[serde(rename = "nextCursor")]
    next_cursor: Option<String>,
    #[allow(dead_code)]
    count: Option<u64>,
}

/// One entry in the registry list (wraps the server object + registry meta).
#[derive(Debug, Deserialize)]
struct RegistryEntry {
    server: RegistryServer,
    #[serde(rename = "_meta")]
    meta: Option<RegistryEntryMeta>,
}

#[derive(Debug, Deserialize, Default)]
struct RegistryEntryMeta {
    #[serde(rename = "io.modelcontextprotocol.registry/official")]
    official: Option<RegistryOfficialMeta>,
}

#[derive(Debug, Deserialize)]
struct RegistryOfficialMeta {
    status: Option<String>,
    #[serde(rename = "publishedAt")]
    published_at: Option<String>,
}

/// A server record from the registry.
#[derive(Debug, Deserialize)]
pub struct RegistryServer {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    #[serde(rename = "websiteUrl")]
    pub website_url: Option<String>,
    pub packages: Option<Vec<RegistryPackage>>,
    pub remotes: Option<Vec<RegistryRemote>>,
    pub repository: Option<RegistryRepository>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryPackage {
    #[serde(rename = "registryType")]
    pub registry_type: String, // "npm" | "pypi" | "oci"
    pub identifier: Option<String>,
    pub version: Option<String>,
    #[serde(rename = "runtimeHint")]
    pub runtime_hint: Option<String>,
    pub transport: Option<RegistryTransport>,
    #[serde(rename = "environmentVariables")]
    pub environment_variables: Option<Vec<RegistryEnvVar>>,
    #[serde(rename = "packageArguments")]
    pub package_arguments: Option<Vec<RegistryPackageArg>>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryRemote {
    #[serde(rename = "type")]
    pub transport_type: String, // "streamable-http" | "sse"
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryTransport {
    #[serde(rename = "type")]
    pub transport_type: String,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryEnvVar {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "isRequired")]
    pub is_required: Option<bool>,
    #[serde(rename = "isSecret")]
    pub is_secret: Option<bool>,
    pub value: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryPackageArg {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "isRequired")]
    pub is_required: Option<bool>,
    pub value: Option<String>,
    pub default: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryRepository {
    pub url: Option<String>,
    pub source: Option<String>,
}

// ── Local .mcp.json config types ─────────────────────────────────────────────
// These mirror McpServerEntry / McpJsonFile from gestura-core-mcp WITHOUT
// importing from that crate (which would create a circular dependency).

/// A single server entry in `.mcp.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfigEntry {
    /// Transport type: "stdio" or "http".
    #[serde(rename = "type")]
    pub transport: String,
    /// Command to launch (stdio servers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// CLI arguments (stdio servers).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub args: Vec<String>,
    /// Environment variables passed to the server process.
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub env: HashMap<String, String>,
    /// Remote URL (HTTP/SSE servers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Whether the server is enabled (None = treat as enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// Root structure of a `.mcp.json` file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpJsonConfig {
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpConfigEntry>,
}

// ── Operation output ──────────────────────────────────────────────────────────

/// Unified output for all mcp_manager operations.
/// The `workflow_guidance` and `next_steps` fields carry LLM-facing prompts
/// that guide the agent through multi-step MCP discovery and install workflows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpManagerOutput {
    pub operation: String,
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub servers: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_guidance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_steps: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl McpManagerOutput {
    fn err(operation: &str, message: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            operation: operation.to_string(),
            success: false,
            message: message.into(),
            servers: None,
            config_path: None,
            workflow_guidance: None,
            next_steps: None,
            error: Some(detail.into()),
        }
    }
}

// ── Workflow guidance (LLM-facing prompts) ────────────────────────────────────

const SEARCH_GUIDANCE: &str = "\
SEARCH RESULTS WORKFLOW:
1. Review the server list above. Each entry shows: name, description, transport type, and package manager.
2. If results look relevant, use operation=evaluate with the server name (e.g. \"io.github.org/repo\") to get full details including all env vars and install instructions.
3. If no good matches, try a different search query — the registry searches by name prefix. Shorter or more general keywords work better.
4. Once you find the right server, use operation=install to add it to the user's .mcp.json.
5. After install, use operation=enable if the server is disabled, then instruct the user to restart Gestura (or reload MCP connections).
TIPS:
- Prefer servers with 'streamable-http' transport (no local install needed) for quick setup.
- For stdio servers, check required env vars before installing — ask the user for any secrets.
- The server name (e.g. 'io.github.modelcontextprotocol/server-filesystem') is the ID for evaluate/install.";

const EVALUATE_GUIDANCE: &str = "\
EVALUATE RESULTS WORKFLOW:
1. Review the full server details above — pay special attention to:
   - Transport type (stdio = local process, streamable-http = remote, sse = legacy remote)
   - Required environment variables marked isRequired=true (especially secrets like API keys)
   - Package type (npm → needs Node.js/npx, pypi → needs Python/uvx, oci → needs Docker)
2. If the server looks suitable, ask the user for any missing required env vars (especially secrets).
3. Use operation=install with the server name, optional alias, and any env vars the user provides.
4. The install step writes the entry to .mcp.json — no packages are downloaded at this stage.
5. The MCP client will launch/connect the server on next use.
DECISION GUIDE:
- streamable-http server → install with type=http, url=<remote_url>, no command needed.
- npm stdio server → install with command='npx', args=['-y', '<package>'], type=stdio.
- pypi stdio server → install with command='uvx', args=['<package>'], type=stdio.
- oci stdio server → install with command='docker', args=['run', '-i', '--rm', '<image>'], type=stdio.";

const INSTALL_GUIDANCE: &str = "\
INSTALL COMPLETE — NEXT STEPS:
1. The server entry has been written to .mcp.json shown above.
2. If the server requires env vars (API keys, tokens, paths), verify they are set in the entry or exported in the shell environment.
3. Use 'gestura mcp connect <name>' or restart Gestura to activate the new server.
4. Use 'gestura mcp tools <name>' to see the tools provided by the server.
5. Use operation=list to review all configured servers.
6. If something is wrong, use operation=remove to delete the entry and try again.
REMINDER: stdio servers require the runtime to be installed (npx needs Node.js, uvx needs Python+uv, docker needs Docker Desktop running).";

const LIST_GUIDANCE: &str = "\
CONFIGURED SERVERS:
- 'enabled: true/null' = server is active and will be connected on startup.
- 'enabled: false' = server is present but disabled (use operation=enable to re-activate).
- Use operation=evaluate with a registry server name to discover new servers to add.
- Use operation=install to add a new server.
- Use operation=remove to delete a server entry.
- Use 'gestura mcp connect <name>' to connect a server in the current session without restarting.";

// ── Config file helpers ───────────────────────────────────────────────────────

/// Resolve the `.mcp.json` path for a given scope.
/// - "user"    → `~/.mcp.json`
/// - "project" → `.mcp.json` in the current working directory
/// - anything else is treated as a literal file path
fn resolve_config_path(scope: &str) -> PathBuf {
    match scope {
        "user" => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".mcp.json"),
        "project" | "" => PathBuf::from(".mcp.json"),
        other => PathBuf::from(other),
    }
}

fn load_config(path: &PathBuf) -> Result<McpJsonConfig> {
    if !path.exists() {
        return Ok(McpJsonConfig::default());
    }
    let text = std::fs::read_to_string(path).map_err(AppError::Io)?;
    serde_json::from_str(&text).map_err(|e| {
        AppError::Io(std::io::Error::other(format!(
            "Failed to parse {}: {e}",
            path.display()
        )))
    })
}

fn save_config(path: &PathBuf, config: &McpJsonConfig) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(AppError::Io)?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| AppError::Io(std::io::Error::other(format!("Serialization error: {e}"))))?;
    std::fs::write(path, json).map_err(AppError::Io)
}

// ── HTTP client helper ────────────────────────────────────────────────────────

fn build_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("gestura-mcp-manager/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Io(std::io::Error::other(format!("HTTP client error: {e}"))))
}

// ── Serialise a registry server to a JSON value for output ───────────────────

fn server_to_value(entry: &RegistryEntry) -> serde_json::Value {
    let s = &entry.server;
    let transport_summary = describe_transport(s);
    let status = entry
        .meta
        .as_ref()
        .and_then(|m| m.official.as_ref())
        .and_then(|o| o.status.as_deref())
        .unwrap_or("unknown")
        .to_string();
    let published_at = entry
        .meta
        .as_ref()
        .and_then(|m| m.official.as_ref())
        .and_then(|o| o.published_at.as_deref())
        .unwrap_or("")
        .to_string();

    serde_json::json!({
        "id": s.name,
        "title": s.title.as_deref().unwrap_or(&s.name),
        "description": s.description.as_deref().unwrap_or(""),
        "version": s.version.as_deref().unwrap_or(""),
        "transport": transport_summary,
        "status": status,
        "published_at": published_at,
        "website": s.website_url.as_deref().unwrap_or(""),
        "repository": s.repository.as_ref().and_then(|r| r.url.as_deref()).unwrap_or(""),
    })
}

fn describe_transport(s: &RegistryServer) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(remotes) = &s.remotes {
        for r in remotes {
            parts.push(format!(
                "{} ({})",
                r.transport_type,
                r.url.as_deref().unwrap_or("")
            ));
        }
    }
    if let Some(pkgs) = &s.packages {
        for p in pkgs {
            let hint = p.runtime_hint.as_deref().unwrap_or(&p.registry_type);
            let transport = p
                .transport
                .as_ref()
                .map(|t| t.transport_type.as_str())
                .unwrap_or("stdio");
            parts.push(format!("{} via {} ({})", transport, hint, p.registry_type));
        }
    }
    if parts.is_empty() {
        "unknown".to_string()
    } else {
        parts.join("; ")
    }
}

// ── Operations ────────────────────────────────────────────────────────────────

/// Search the MCP registry by keyword.
pub async fn search(query: &str, limit: usize) -> Result<McpManagerOutput> {
    let client = build_http_client()?;
    let url = format!(
        "{}/servers?limit={}&search={}",
        REGISTRY_BASE,
        limit.min(50),
        urlencoding::encode(query)
    );
    tracing::debug!("MCP registry search: {url}");

    let resp = client.get(&url).send().await.map_err(|e| {
        AppError::Io(std::io::Error::other(format!(
            "Registry request failed: {e}"
        )))
    })?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Ok(McpManagerOutput::err(
            "search",
            format!("Registry returned HTTP {status}"),
            format!("GET {url} → {status}"),
        ));
    }

    let data: RegistryListResponse = resp
        .json()
        .await
        .map_err(|e| AppError::Io(std::io::Error::other(format!("Registry parse error: {e}"))))?;

    let count = data.servers.len();
    let next_cursor = data
        .metadata
        .as_ref()
        .and_then(|m| m.next_cursor.as_deref())
        .unwrap_or("")
        .to_string();
    let servers: Vec<serde_json::Value> = data.servers.iter().map(server_to_value).collect();

    let message = if count == 0 {
        format!("No servers found matching '{query}'. Try a shorter or different keyword.")
    } else {
        format!("Found {count} server(s) matching '{query}'.")
    };

    let mut next_steps = vec![
        "Use operation=evaluate with a server 'id' field to get full details and install instructions.".to_string(),
        "Use operation=install once you have confirmed a server with the user.".to_string(),
    ];
    if !next_cursor.is_empty() {
        next_steps.push(format!(
            "More results available — search again with cursor='{next_cursor}' to see the next page."
        ));
    }

    Ok(McpManagerOutput {
        operation: "search".to_string(),
        success: true,
        message,
        servers: Some(servers),
        config_path: None,
        workflow_guidance: Some(SEARCH_GUIDANCE.to_string()),
        next_steps: Some(next_steps),
        error: None,
    })
}

/// Fetch detailed information about a single registry server and generate
/// a concrete install recommendation for the agent to present to the user.
pub async fn evaluate(server_id: &str) -> Result<McpManagerOutput> {
    let client = build_http_client()?;
    // The registry uses URL-encoded server name as the ID path component.
    let encoded = urlencoding::encode(server_id);
    let url = format!("{}/servers/{}", REGISTRY_BASE, encoded);
    tracing::debug!("MCP registry evaluate: {url}");

    let resp = client.get(&url).send().await.map_err(|e| {
        AppError::Io(std::io::Error::other(format!(
            "Registry request failed: {e}"
        )))
    })?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(McpManagerOutput::err(
            "evaluate",
            format!("Server '{server_id}' not found in registry."),
            "HTTP 404 — check the exact server name (use operation=search to find it).".to_string(),
        ));
    }
    if !resp.status().is_success() {
        let status = resp.status();
        return Ok(McpManagerOutput::err(
            "evaluate",
            format!("Registry returned HTTP {status}"),
            format!("GET {url} → {status}"),
        ));
    }

    let entry: RegistryEntry = resp
        .json()
        .await
        .map_err(|e| AppError::Io(std::io::Error::other(format!("Registry parse error: {e}"))))?;

    let s = &entry.server;
    let install_rec = build_install_recommendation(s);
    let required_env = collect_required_env(s);
    let detail = serde_json::json!({
        "id": s.name,
        "title": s.title.as_deref().unwrap_or(&s.name),
        "description": s.description.as_deref().unwrap_or(""),
        "version": s.version.as_deref().unwrap_or(""),
        "website": s.website_url.as_deref().unwrap_or(""),
        "repository": s.repository.as_ref().and_then(|r| r.url.as_deref()).unwrap_or(""),
        "install_recommendation": install_rec,
        "required_env_vars": required_env,
        "packages": s.packages.as_ref().map(|pkgs| pkgs.iter().map(|p| serde_json::json!({
            "type": p.registry_type,
            "identifier": p.identifier.as_deref().unwrap_or(""),
            "version": p.version.as_deref().unwrap_or(""),
            "runtime_hint": p.runtime_hint.as_deref().unwrap_or(""),
            "transport": p.transport.as_ref().map(|t| &t.transport_type).map(|s| s.as_str()).unwrap_or("stdio"),
            "env_vars": p.environment_variables.as_ref().map(|ev| ev.iter().map(|v| serde_json::json!({
                "name": v.name,
                "description": v.description.as_deref().unwrap_or(""),
                "required": v.is_required.unwrap_or(false),
                "secret": v.is_secret.unwrap_or(false),
            })).collect::<Vec<_>>()).unwrap_or_default(),
        })).collect::<Vec<_>>()),
        "remotes": s.remotes.as_ref().map(|rs| rs.iter().map(|r| serde_json::json!({
            "type": r.transport_type,
            "url": r.url.as_deref().unwrap_or(""),
        })).collect::<Vec<_>>()),
    });

    let next_steps = build_evaluate_next_steps(s, server_id);

    Ok(McpManagerOutput {
        operation: "evaluate".to_string(),
        success: true,
        message: format!(
            "Server '{}': {}",
            s.title.as_deref().unwrap_or(&s.name),
            s.description.as_deref().unwrap_or("no description")
        ),
        servers: Some(vec![detail]),
        config_path: None,
        workflow_guidance: Some(EVALUATE_GUIDANCE.to_string()),
        next_steps: Some(next_steps),
        error: None,
    })
}

/// Detailed info — alias for evaluate with extra raw JSON.
pub async fn info(server_id: &str) -> Result<McpManagerOutput> {
    evaluate(server_id).await
}

fn collect_required_env(s: &RegistryServer) -> Vec<serde_json::Value> {
    let mut vars: Vec<serde_json::Value> = Vec::new();
    if let Some(pkgs) = &s.packages {
        for p in pkgs {
            if let Some(env_vars) = &p.environment_variables {
                for v in env_vars {
                    vars.push(serde_json::json!({
                        "name": v.name,
                        "description": v.description.as_deref().unwrap_or(""),
                        "required": v.is_required.unwrap_or(false),
                        "secret": v.is_secret.unwrap_or(false),
                    }));
                }
            }
        }
    }
    vars
}

fn build_install_recommendation(s: &RegistryServer) -> serde_json::Value {
    // Prefer streamable-http remote (no local install)
    if let Some(remotes) = &s.remotes
        && let Some(remote) = remotes
            .iter()
            .find(|r| r.transport_type == "streamable-http")
            .or_else(|| remotes.first())
    {
        return serde_json::json!({
            "transport": "http",
            "url": remote.url.as_deref().unwrap_or(""),
            "note": "Remote HTTP server — no local runtime needed.",
        });
    }
    // Fall back to best package option
    if let Some(pkgs) = &s.packages
        && let Some(pkg) = pkgs.first()
    {
        return build_package_recommendation(pkg);
    }
    serde_json::json!({"note": "No clear install path found. Review the server repository manually."})
}

fn build_package_recommendation(p: &RegistryPackage) -> serde_json::Value {
    let id = p.identifier.as_deref().unwrap_or("");
    match p.registry_type.as_str() {
        "npm" => {
            let hint = p.runtime_hint.as_deref().unwrap_or("npx");
            serde_json::json!({
                "transport": "stdio",
                "command": hint,
                "args": ["-y", id],
                "note": format!("npm package — requires Node.js. Run: {hint} -y {id}"),
            })
        }
        "pypi" => {
            let hint = p.runtime_hint.as_deref().unwrap_or("uvx");
            serde_json::json!({
                "transport": "stdio",
                "command": hint,
                "args": [id],
                "note": format!("PyPI package — requires Python + uv. Run: {hint} {id}"),
            })
        }
        "oci" => serde_json::json!({
            "transport": "stdio",
            "command": "docker",
            "args": ["run", "-i", "--rm", id],
            "note": format!("OCI image — requires Docker Desktop running. Image: {id}"),
        }),
        other => serde_json::json!({
            "transport": "stdio",
            "note": format!("Unknown package type '{other}' — install manually from identifier: {id}"),
        }),
    }
}

fn build_evaluate_next_steps(s: &RegistryServer, server_id: &str) -> Vec<String> {
    let mut steps = Vec::new();
    let has_required_env = s.packages.as_ref().is_some_and(|pkgs| {
        pkgs.iter().any(|p| {
            p.environment_variables
                .as_ref()
                .is_some_and(|ev| ev.iter().any(|v| v.is_required.unwrap_or(false)))
        })
    });
    if has_required_env {
        steps.push("This server requires environment variables — ask the user to provide them before installing.".to_string());
    }
    steps.push(format!(
        "To install: use operation=install with server_id=\"{server_id}\" and any required env vars."
    ));
    steps.push("Confirm the install plan with the user before proceeding.".to_string());
    steps
}

/// Install a server into .mcp.json by looking it up in the registry and
/// deriving the best config entry automatically.
///
/// # Parameters (from `args` JSON)
/// - `server_id`   – registry server name (e.g. `"io.github.org/repo"`)
/// - `name`        – local alias in .mcp.json (defaults to last path segment of `server_id`)
/// - `scope`       – `"user"` | `"project"` (default `"project"`)
/// - `transport`   – override transport: `"stdio"` | `"http"` (auto-detected if omitted)
/// - `command`     – override launch command (stdio only)
/// - `args`        – override args array (stdio only)
/// - `url`         – override URL (http only)
/// - `env`         – map of env var name → value
#[allow(clippy::too_many_arguments)]
pub async fn install(
    server_id: &str,
    name: Option<&str>,
    scope: &str,
    transport_override: Option<&str>,
    command_override: Option<&str>,
    args_override: Option<Vec<String>>,
    url_override: Option<&str>,
    env_vars: HashMap<String, String>,
) -> Result<McpManagerOutput> {
    // 1. Fetch server details from registry.
    let client = build_http_client()?;
    let encoded = urlencoding::encode(server_id);
    let url = format!("{}/servers/{}", REGISTRY_BASE, encoded);
    let resp = client.get(&url).send().await.map_err(|e| {
        AppError::Io(std::io::Error::other(format!(
            "Registry request failed: {e}"
        )))
    })?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(McpManagerOutput::err(
            "install",
            format!("Server '{server_id}' not found in registry."),
            "Use operation=search to find the correct server name.".to_string(),
        ));
    }
    if !resp.status().is_success() {
        let status = resp.status();
        return Ok(McpManagerOutput::err(
            "install",
            format!("Registry returned HTTP {status}"),
            format!("GET {url} → {status}"),
        ));
    }

    let entry: RegistryEntry = resp
        .json()
        .await
        .map_err(|e| AppError::Io(std::io::Error::other(format!("Registry parse error: {e}"))))?;
    let s = &entry.server;

    // 2. Derive the config entry.
    let config_entry = derive_config_entry(
        s,
        transport_override,
        command_override,
        args_override,
        url_override,
        env_vars,
    )?;

    // 3. Determine local name.
    let local_name = name.map(|n| n.to_string()).unwrap_or_else(|| {
        server_id
            .rsplit('/')
            .next()
            .unwrap_or(server_id)
            .to_string()
    });

    // 4. Write to .mcp.json.
    let config_path = resolve_config_path(scope);
    let mut config = load_config(&config_path)?;
    config.mcp_servers.insert(local_name.clone(), config_entry);
    save_config(&config_path, &config)?;

    tracing::info!(
        "Installed MCP server '{}' as '{}' in {}",
        server_id,
        local_name,
        config_path.display()
    );

    Ok(McpManagerOutput {
        operation: "install".to_string(),
        success: true,
        message: format!(
            "Installed '{}' as '{}' in {}",
            server_id,
            local_name,
            config_path.display()
        ),
        servers: None,
        config_path: Some(config_path.to_string_lossy().to_string()),
        workflow_guidance: Some(INSTALL_GUIDANCE.to_string()),
        next_steps: Some(vec![
            format!("Run 'gestura mcp connect {local_name}' to connect without restarting."),
            "Or restart Gestura to load the new server automatically.".to_string(),
            format!(
                "Use operation=list (scope={scope}) to verify the entry was written correctly."
            ),
        ]),
        error: None,
    })
}

fn derive_config_entry(
    s: &RegistryServer,
    transport_override: Option<&str>,
    command_override: Option<&str>,
    args_override: Option<Vec<String>>,
    url_override: Option<&str>,
    env_vars: HashMap<String, String>,
) -> Result<McpConfigEntry> {
    // Prefer remote HTTP unless overridden.
    let prefer_http = transport_override.map_or_else(
        || {
            s.remotes.as_ref().is_some_and(|rs| {
                rs.iter()
                    .any(|r| r.transport_type == "streamable-http" || r.transport_type == "sse")
            })
        },
        |t| t == "http",
    );

    if prefer_http && transport_override != Some("stdio") {
        let remote_url = url_override.map(|u| u.to_string()).or_else(|| {
            s.remotes.as_ref().and_then(|rs| {
                rs.iter()
                    .find(|r| r.transport_type == "streamable-http")
                    .or_else(|| rs.iter().find(|r| r.transport_type == "sse"))
                    .and_then(|r| r.url.clone())
            })
        });

        return Ok(McpConfigEntry {
            transport: "http".to_string(),
            command: None,
            args: vec![],
            env: env_vars,
            url: remote_url,
            enabled: Some(true),
        });
    }

    // stdio via package
    let pkg = s.packages.as_ref().and_then(|pkgs| pkgs.first());

    let (command, args) = if let Some(cmd) = command_override {
        let final_args = args_override.unwrap_or_default();
        (cmd.to_string(), final_args)
    } else if let Some(p) = pkg {
        let id = p.identifier.as_deref().unwrap_or("");
        match p.registry_type.as_str() {
            "npm" => {
                let hint = p.runtime_hint.as_deref().unwrap_or("npx").to_string();
                (hint, vec!["-y".to_string(), id.to_string()])
            }
            "pypi" => {
                let hint = p.runtime_hint.as_deref().unwrap_or("uvx").to_string();
                (hint, vec![id.to_string()])
            }
            "oci" => (
                "docker".to_string(),
                vec![
                    "run".to_string(),
                    "-i".to_string(),
                    "--rm".to_string(),
                    id.to_string(),
                ],
            ),
            _ => {
                return Err(AppError::Io(std::io::Error::other(format!(
                    "Cannot auto-derive install command for package type '{}'. \
                     Provide command/args overrides.",
                    p.registry_type
                ))));
            }
        }
    } else {
        return Err(AppError::Io(std::io::Error::other(
            "No packages or remotes found for this server. Provide command/url overrides.",
        )));
    };

    Ok(McpConfigEntry {
        transport: "stdio".to_string(),
        command: Some(command),
        args,
        env: env_vars,
        url: None,
        enabled: Some(true),
    })
}

/// Set a server's `enabled` flag to `true` in .mcp.json.
pub fn enable(name: &str, scope: &str) -> Result<McpManagerOutput> {
    set_enabled(name, scope, true)
}

/// Set a server's `enabled` flag to `false` in .mcp.json.
pub fn disable(name: &str, scope: &str) -> Result<McpManagerOutput> {
    set_enabled(name, scope, false)
}

fn set_enabled(name: &str, scope: &str, enabled: bool) -> Result<McpManagerOutput> {
    let config_path = resolve_config_path(scope);
    let mut config = load_config(&config_path)?;
    match config.mcp_servers.get_mut(name) {
        None => Ok(McpManagerOutput::err(
            if enabled { "enable" } else { "disable" },
            format!("Server '{name}' not found in {}", config_path.display()),
            "Use operation=list to see configured servers.".to_string(),
        )),
        Some(entry) => {
            entry.enabled = Some(enabled);
            save_config(&config_path, &config)?;
            let verb = if enabled { "enabled" } else { "disabled" };
            Ok(McpManagerOutput {
                operation: if enabled { "enable" } else { "disable" }.to_string(),
                success: true,
                message: format!("Server '{name}' {verb} in {}", config_path.display()),
                servers: None,
                config_path: Some(config_path.to_string_lossy().to_string()),
                workflow_guidance: None,
                next_steps: Some(vec![
                    "Restart Gestura or run 'gestura mcp connect <name>' to apply the change."
                        .to_string(),
                ]),
                error: None,
            })
        }
    }
}

/// List all servers configured in .mcp.json for the given scope.
pub fn list(scope: &str) -> Result<McpManagerOutput> {
    let config_path = resolve_config_path(scope);
    let config = load_config(&config_path)?;
    let servers: Vec<serde_json::Value> = config
        .mcp_servers
        .iter()
        .map(|(name, entry)| {
            serde_json::json!({
                "name": name,
                "transport": entry.transport,
                "command": entry.command.as_deref().unwrap_or(""),
                "args": entry.args,
                "url": entry.url.as_deref().unwrap_or(""),
                "env_keys": entry.env.keys().collect::<Vec<_>>(),
                "enabled": entry.enabled.unwrap_or(true),
            })
        })
        .collect();

    let count = servers.len();
    Ok(McpManagerOutput {
        operation: "list".to_string(),
        success: true,
        message: format!("{count} server(s) configured in {}", config_path.display()),
        servers: Some(servers),
        config_path: Some(config_path.to_string_lossy().to_string()),
        workflow_guidance: Some(LIST_GUIDANCE.to_string()),
        next_steps: None,
        error: None,
    })
}

/// Remove a server entry from .mcp.json.
pub fn remove(name: &str, scope: &str) -> Result<McpManagerOutput> {
    let config_path = resolve_config_path(scope);
    let mut config = load_config(&config_path)?;
    if config.mcp_servers.remove(name).is_none() {
        return Ok(McpManagerOutput::err(
            "remove",
            format!("Server '{name}' not found in {}", config_path.display()),
            "Use operation=list to see configured servers.".to_string(),
        ));
    }
    save_config(&config_path, &config)?;
    Ok(McpManagerOutput {
        operation: "remove".to_string(),
        success: true,
        message: format!("Removed '{name}' from {}", config_path.display()),
        servers: None,
        config_path: Some(config_path.to_string_lossy().to_string()),
        workflow_guidance: None,
        next_steps: Some(vec![
            "Restart Gestura or run 'gestura mcp disconnect <name>' to apply the change."
                .to_string(),
        ]),
        error: None,
    })
}

// ── Main dispatcher ───────────────────────────────────────────────────────────

/// Entry point called by the pipeline tool executor.
///
/// Expected `args` shape (all fields optional unless noted):
/// ```json
/// {
///   "operation": "search" | "evaluate" | "install" | "enable" | "disable" | "list" | "remove" | "info",
///   "query":      "<search terms>",          // search
///   "limit":      20,                         // search (default 20)
///   "cursor":     "<opaque>",                 // search pagination
///   "server_id":  "<registry-name>",          // evaluate, install, info
///   "name":       "<local-alias>",            // install (optional)
///   "scope":      "project" | "user",         // install/enable/disable/list/remove
///   "transport":  "stdio" | "http",           // install override
///   "command":    "npx",                      // install stdio override
///   "args":       ["-y", "package"],          // install stdio override
///   "url":        "https://...",              // install http override
///   "env":        {"KEY": "value"}            // install env vars
/// }
/// ```
pub async fn handle(args: &serde_json::Value) -> Result<McpManagerOutput> {
    let op = args
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("list");

    let scope = args
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("project");

    match op {
        "search" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            search(query, limit).await
        }
        "evaluate" | "info" => {
            let server_id = args
                .get("server_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    AppError::Io(std::io::Error::other(
                        "evaluate requires 'server_id' parameter",
                    ))
                })?;
            evaluate(server_id).await
        }
        "install" => {
            let server_id = args
                .get("server_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    AppError::Io(std::io::Error::other(
                        "install requires 'server_id' parameter",
                    ))
                })?;
            let name = args.get("name").and_then(|v| v.as_str());
            let transport = args.get("transport").and_then(|v| v.as_str());
            let command = args.get("command").and_then(|v| v.as_str());
            let args_list: Option<Vec<String>> =
                args.get("args").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                });
            let url = args.get("url").and_then(|v| v.as_str());
            let env: HashMap<String, String> = args
                .get("env")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            install(
                server_id, name, scope, transport, command, args_list, url, env,
            )
            .await
        }
        "enable" => {
            let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                AppError::Io(std::io::Error::other("enable requires 'name' parameter"))
            })?;
            enable(name, scope)
        }
        "disable" => {
            let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                AppError::Io(std::io::Error::other("disable requires 'name' parameter"))
            })?;
            disable(name, scope)
        }
        "list" => list(scope),
        "remove" => {
            let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                AppError::Io(std::io::Error::other("remove requires 'name' parameter"))
            })?;
            remove(name, scope)
        }
        unknown => Ok(McpManagerOutput::err(
            unknown,
            format!("Unknown mcp operation: '{unknown}'"),
            "Valid operations: search, evaluate, install, enable, disable, list, remove, info"
                .to_string(),
        )),
    }
}
