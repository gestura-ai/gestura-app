//! MCP Registry integration — popular server discovery.
//!
//! Fetches and filters the official MCP Registry
//! (<https://registry.modelcontextprotocol.io>) to surface open-source
//! servers. Three transport families are supported: stdio (npm / pypi),
//! streamable-HTTP, and SSE. Two add-ability tiers are exposed per server:
//!
//! * **Quick Add** — server with no required env vars / auth headers; added
//!   enabled and ready to use immediately (`enabled = true`).
//! * **Add (Disabled)** — server that requires at least one env var or auth
//!   header before it can connect; added in a disabled state (`enabled = false`)
//!   so the user can fill in secrets / config in the settings panel and then
//!   enable it.

use crate::config::{McpScope, McpServerEntry, McpTransportType};
use crate::error::Result;
use gestura_core_foundation::error::AppError;

/// A recommended MCP server entry sourced from the official MCP Registry.
///
/// Shaped for the configuration window: carries display metadata (description
/// and repo URL) plus a fully-formed [`McpServerEntry`] ready for 1-click add.
#[derive(serde::Serialize, Debug, Clone)]
pub struct PopularMcpServer {
    /// Display name from the registry (may include dots/slashes).
    pub display_name: String,
    /// Human description from the registry.
    pub description: String,
    /// Source repository URL (open-source signal).
    pub repository_url: String,
    /// Package identifier (npm or pypi) used to invoke the server.
    pub package_identifier: String,
    /// Version string from registry (`server.version`).
    pub version: String,
    /// Fully-formed MCP server config entry (stdio).
    pub tool: McpServerEntry,
}

/// One entry in a paginated registry browse result.
///
/// Unlike [`PopularMcpServer`], this covers **all active** registry servers, not
/// just the no-configuration npm/stdio subset.
///
/// Two optional add-ability tiers are populated by the backend:
///
/// | Field | Transport family | Condition | Button |
/// |-------|-----------------|-----------|--------|
/// | `quick_add` | stdio (npm/pypi) | no required env vars | "Quick Add" (enabled) |
/// | `quick_add` | HTTP / SSE remote | no required auth headers | "Quick Add" (enabled) |
/// | `add_disabled` | stdio (npm/pypi) | has ≥1 required env var | "Add (Disabled)" |
/// | `add_disabled` | HTTP / SSE remote | has ≥1 required header | "Add (Disabled)" |
///
/// At most one of the two will be `Some` for any given entry; stdio packages
/// take priority over remotes within each tier.
#[derive(serde::Serialize, Debug, Clone)]
pub struct RegistryBrowseEntry {
    /// Registry display name (e.g. `"io.github.owner/server-name"`).
    pub display_name: String,
    /// Human-readable description from the registry.
    pub description: String,
    /// Source repository URL; may be an empty string for non-open-source entries.
    pub repository_url: String,
    /// Version string from the registry.
    pub version: String,
    /// If `Some`, this server can be Quick-Added without any additional
    /// configuration.  Covers stdio (npm/pypi) servers with no required env
    /// vars **and** HTTP/SSE remote servers with no required auth headers.
    /// Added with `enabled = true`; ready to use immediately.
    pub quick_add: Option<McpServerEntry>,
    /// If `Some`, this server requires at least one secret or configuration
    /// value before it can connect.  Covers stdio (npm/pypi) servers with
    /// required env vars **and** HTTP/SSE remote servers with required auth
    /// headers.  Added with `enabled = false`; the user fills in values in
    /// the settings panel and then enables it.  Mutually exclusive with
    /// `quick_add`.
    pub add_disabled: Option<McpServerEntry>,
}

/// Paginated response returned by [`browse_mcp_registry`].
#[derive(serde::Serialize, Debug, Clone)]
pub struct RegistryBrowsePage {
    /// Servers on this page.
    pub servers: Vec<RegistryBrowseEntry>,
    /// Opaque cursor for fetching the next page; `None` means no more pages.
    pub next_cursor: Option<String>,
    /// Number of items returned on this page (from registry metadata).
    pub page_count: Option<u64>,
}

/// Return up to `limit` popular, open-source MCP servers that can be added
/// without additional configuration.
///
/// Selection rules (registry-driven):
/// - MCP Registry listing status is **active**
/// - Has a repository URL (open-source signal)
/// - `$schema` field contains `server.schema.json` (current MCP standard)
/// - Has an npm `stdio` package
/// - Declares **zero** environment variables (no required user setup)
///
/// Results are sorted by the curated [`PRIORITY_PACKAGES`] list first, then
/// alphabetically as a tiebreak for servers not in the curated set.
/// Returns an error if fewer than `limit` servers pass all filters.
pub async fn list_popular_mcp_servers(limit: usize) -> Result<Vec<PopularMcpServer>> {
    fetch_popular_mcp_servers_from_registry(limit).await
}

/// Fetch a single page from the MCP Registry with optional full-text search.
///
/// Unlike [`list_popular_mcp_servers`], this function:
/// - Fetches **one page only** (no multi-page accumulation).
/// - Includes **all active servers** regardless of transport or env-var requirements.
/// - Marks servers that pass the zero-config npm/stdio criteria via
///   [`RegistryBrowseEntry::quick_add`] so the UI can surface a "Quick Add" button.
///
/// # Parameters
/// - `query` – Optional search string forwarded to the registry `search=` param.
///   Pass `None` or an empty string to list all servers alphabetically.
/// - `cursor` – Opaque pagination cursor from a previous [`RegistryBrowsePage::next_cursor`].
/// - `limit`  – Number of servers to request per page (typically 20).
pub async fn browse_mcp_registry(
    query: Option<String>,
    cursor: Option<String>,
    limit: usize,
) -> Result<RegistryBrowsePage> {
    const REGISTRY_URL: &str = "https://registry.modelcontextprotocol.io/v0.1/servers";

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .user_agent(format!(
            "Gestura/{} (config window; https://gestura.ai)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(AppError::Http)?;

    let mut url = reqwest::Url::parse(REGISTRY_URL).map_err(|e| AppError::Mcp(e.to_string()))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("version", "latest");
        qp.append_pair("limit", &limit.to_string());
        if let Some(ref q) = query {
            let trimmed = q.trim();
            if !trimmed.is_empty() {
                qp.append_pair("search", trimmed);
            }
        }
        if let Some(ref c) = cursor {
            qp.append_pair("cursor", c);
        }
    }

    let resp = client.get(url).send().await.map_err(AppError::Http)?;
    if !resp.status().is_success() {
        return Err(AppError::Mcp(format!(
            "MCP Registry returned status {}",
            resp.status()
        )));
    }

    let data: serde_json::Value = resp.json().await.map_err(AppError::Http)?;

    let items = data
        .get("servers")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            AppError::Mcp("MCP Registry response missing 'servers' array".to_string())
        })?;

    let servers: Vec<RegistryBrowseEntry> = items
        .iter()
        .filter_map(browse_entry_from_registry_item)
        .collect();

    let next_cursor = data
        .get("metadata")
        .and_then(|m| m.get("nextCursor"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let page_count = data
        .get("metadata")
        .and_then(|m| m.get("count"))
        .and_then(|v| v.as_u64());

    Ok(RegistryBrowsePage {
        servers,
        next_cursor,
        page_count,
    })
}

/// Curated set of 20 production-ready, no-configuration MCP servers in priority order.
///
/// Each entry is an npm package identifier as it appears in the MCP Registry.
/// When sorting registry candidates, servers whose `package_identifier` matches an
/// entry here are surfaced first (lower index = higher priority).  Any server
/// not in the list receives a priority value of [`usize::MAX`] and sorts last,
/// falling back to alphabetical ordering by `(display_name, package_identifier)`.
const PRIORITY_PACKAGES: &[&str] = &[
    "chrome-devtools-mcp",       // ChromeDevTools official — browser debugging
    "@gitkraken/gk",             // GitKraken CLI — mature cross-platform Git
    "computer-use-mcp",          // Full computer control, no API key required
    "defuddle-fetch-mcp-server", // Clean web-content fetching
    "filesystem-mcp",            // File read / create / edit operations
    "shell-exec-mcp",            // Bash command execution
    "xcodebuildmcp",             // Xcode build tooling — iOS/macOS dev
    "docfork",                   // Up-to-date library docs for AI agents
    "reddit-mcp-buddy",          // Reddit browsing — no API keys required
    "@azure/mcp",                // Microsoft Azure official MCP server
    "@google-cloud/gemini-cloud-assist-mcp", // Google Cloud Platform official
    "firebase-tools",            // Firebase / Google official CLI tools
    "@sveltejs/mcp",             // Official Svelte framework tooling
    "@goreleaser/mcp",           // Official GoReleaser — release automation
    "@discourse/mcp",            // Official Discourse — community platforms
    "@alisaitteke/docker-mcp",   // Docker container management
    "mcp-prometheus",            // Prometheus monitoring & alerting
    "mcp-server-code-runner",    // Multi-language code runner
    "@hypothesi/tauri-mcp-server", // Tauri v2 desktop-app tooling
    "@dollhousemcp/mcp-server",  // AI personas, skills & persistent memory
];

/// Return the priority rank of a package identifier.
///
/// Packages in [`PRIORITY_PACKAGES`] return their 0-based index; anything else
/// returns [`usize::MAX`] so it sorts after all curated entries.
fn priority_index(pkg_id: &str) -> usize {
    PRIORITY_PACKAGES
        .iter()
        .position(|&p| p == pkg_id)
        .unwrap_or(usize::MAX)
}

async fn fetch_popular_mcp_servers_from_registry(limit: usize) -> Result<Vec<PopularMcpServer>> {
    use std::collections::{HashMap, HashSet};

    const REGISTRY_URL: &str = "https://registry.modelcontextprotocol.io/v0.1/servers";

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .user_agent(format!(
            "Gestura/{} (config window; https://gestura.ai)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(AppError::Http)?;

    let mut cursor: Option<String> = None;
    // Collect ALL passing candidates before sorting — priority ordering requires
    // the complete candidate set, not just what appears on the first few pages.
    let mut all_candidates: Vec<PopularMcpServer> = Vec::new();

    for _page in 0..15 {
        let mut url =
            reqwest::Url::parse(REGISTRY_URL).map_err(|e| AppError::Mcp(e.to_string()))?;
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("version", "latest");
            qp.append_pair("limit", "100");
            if let Some(ref c) = cursor {
                qp.append_pair("cursor", c);
            }
        }

        let resp = client.get(url).send().await.map_err(AppError::Http)?;

        if !resp.status().is_success() {
            return Err(AppError::Mcp(format!(
                "MCP Registry returned status {}",
                resp.status()
            )));
        }

        let data: serde_json::Value = resp.json().await.map_err(AppError::Http)?;

        let servers = data
            .get("servers")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                AppError::Mcp("MCP Registry response missing 'servers' array".to_string())
            })?;

        for item in servers {
            if let Some(candidate) = popular_candidate_from_registry_item(item) {
                all_candidates.push(candidate);
            }
        }

        cursor = data
            .get("metadata")
            .and_then(|m| m.get("nextCursor"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if cursor.is_none() {
            break;
        }
    }

    // Sort: curated-priority index first, then alphabetical tiebreak.
    all_candidates.sort_by(|a, b| {
        let pa = priority_index(&a.package_identifier);
        let pb = priority_index(&b.package_identifier);
        pa.cmp(&pb)
            .then_with(|| a.display_name.as_str().cmp(b.display_name.as_str()))
            .then_with(|| {
                a.package_identifier
                    .as_str()
                    .cmp(b.package_identifier.as_str())
            })
    });

    // Deduplicate tool names, then take up to `limit`.
    let mut out: Vec<PopularMcpServer> = Vec::with_capacity(limit);
    let mut used_names: HashSet<String> = HashSet::new();
    let mut name_collision_counts: HashMap<String, usize> = HashMap::new();

    for mut c in all_candidates {
        if out.len() >= limit {
            break;
        }
        let base = normalize_mcp_server_name(&c.display_name);
        let mut candidate_name = base.clone();
        if used_names.contains(&candidate_name) {
            let n = name_collision_counts.entry(base.clone()).or_insert(1);
            *n += 1;
            candidate_name = format!("{}-{}", base, *n);
        }
        used_names.insert(candidate_name.clone());
        c.tool.name = candidate_name;
        out.push(c);
    }

    if out.len() != limit {
        return Err(AppError::Mcp(format!(
            "MCP Registry filtering produced {} server(s); expected {}.",
            out.len(),
            limit
        )));
    }

    Ok(out)
}

/// Resolve the source-repository URL for a registry server object.
///
/// Checks the explicit `repository.url` field first.  When that is absent,
/// attempts to infer the GitHub URL from the well-known server-name convention
/// `io.github.{org}/{repo}` used by many official MCP servers.
///
/// Returns `None` when no URL can be determined — the server is excluded from
/// add-able tiers that require an open-source signal.
fn infer_repository_url(server: &serde_json::Value) -> Option<String> {
    // Prefer the explicit registry field.
    let explicit = server
        .get("repository")
        .and_then(|r| r.get("url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if !explicit.is_empty() {
        return Some(explicit.to_string());
    }

    // Fallback: infer from `io.github.{org}/{repo}` naming convention.
    // Example: "io.github.redis/mcp-redis" → "https://github.com/redis/mcp-redis"
    let name = server.get("name").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(rest) = name.strip_prefix("io.github.")
        && let Some(slash_pos) = rest.find('/')
    {
        let org = &rest[..slash_pos];
        let repo = &rest[slash_pos + 1..];
        if !org.is_empty() && !repo.is_empty() {
            return Some(format!("https://github.com/{org}/{repo}"));
        }
    }

    None
}

/// Build a stdio [`McpServerEntry`] from a single package element in the
/// registry `packages` array.
///
/// Supported `registryType` values:
///
/// | Type | Command | Version syntax |
/// |------|---------|----------------|
/// | `npm` | `npx -y` | `identifier@version` |
/// | `pypi` | `uvx` | `identifier==version` |
///
/// Returns `None` for unsupported types, non-stdio transports, or missing
/// required fields.  The returned tuple is `(identifier, entry)` so callers
/// can sort candidates deterministically by identifier.
fn build_mcp_entry_for_package(
    pkg: &serde_json::Value,
    display_name: &str,
    version: &str,
    enabled: bool,
) -> Option<(String, McpServerEntry)> {
    let registry_type = pkg.get("registryType").and_then(|v| v.as_str())?;
    let transport_type = pkg
        .get("transport")
        .and_then(|t| t.get("type"))
        .and_then(|v| v.as_str())?;
    if transport_type != "stdio" {
        return None;
    }

    let identifier = pkg.get("identifier")?.as_str()?.to_string();
    let (command, args) = match registry_type {
        "npm" => (
            "npx".to_string(),
            vec!["-y".to_string(), format!("{}@{}", identifier, version)],
        ),
        "pypi" => (
            "uvx".to_string(),
            vec![format!("{}=={}", identifier, version)],
        ),
        _ => return None,
    };

    Some((
        identifier,
        McpServerEntry {
            name: normalize_mcp_server_name(display_name),
            transport: McpTransportType::Stdio,
            enabled,
            command: Some(command),
            args,
            env: Default::default(),
            url: None,
            headers: Default::default(),
            scope: McpScope::User,
            timeout_secs: 30,
            auto_reconnect: true,
            session_default_enabled: true,
        },
    ))
}

/// Build an HTTP or SSE [`McpServerEntry`] from a single remote element in the
/// registry `remotes` array.
///
/// Supported `type` values:
///
/// | Registry type | Transport |
/// |---------------|-----------|
/// | `streamable-http` | [`McpTransportType::Http`] |
/// | `sse` | [`McpTransportType::Sse`] |
///
/// HTTP headers are intentionally **not** pre-populated in the returned entry.
/// Registry header values contain user-specific placeholder strings such as
/// `"Bearer {smithery_api_key}"` that are meaningless until the user supplies
/// real values.  The `headers` map is left empty so the user can fill it in
/// through the settings panel before enabling the server.
///
/// Returns `None` for unsupported transport types or a missing/empty URL.
fn build_mcp_entry_for_remote(
    remote: &serde_json::Value,
    display_name: &str,
    enabled: bool,
) -> Option<McpServerEntry> {
    let remote_type = remote.get("type").and_then(|v| v.as_str())?;
    let transport = match remote_type {
        "streamable-http" => McpTransportType::Http,
        "sse" => McpTransportType::Sse,
        _ => return None,
    };

    let url = remote
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();

    Some(McpServerEntry {
        name: normalize_mcp_server_name(display_name),
        transport,
        enabled,
        command: None,
        args: Vec::new(),
        env: Default::default(),
        url: Some(url),
        headers: Default::default(), // user fills in via settings panel
        scope: McpScope::User,
        timeout_secs: 30,
        auto_reconnect: true,
        session_default_enabled: true,
    })
}

/// Return `true` if any element of a registry `headers` or `environmentVariables`
/// array declares `isRequired: true`.
fn has_required_fields(arr: Option<&serde_json::Value>) -> bool {
    arr.and_then(|v| v.as_array())
        .map(|items| {
            items.iter().any(|item| {
                item.get("isRequired")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Attempt to build a [`PopularMcpServer`] from a single registry API item.
///
/// Returns `None` if the item does not pass all filter criteria.
fn popular_candidate_from_registry_item(item: &serde_json::Value) -> Option<PopularMcpServer> {
    let server = item.get("server")?;
    let meta = item.get("_meta")?;

    // Require active listing.
    let official = meta
        .get("io.modelcontextprotocol.registry/official")
        .and_then(|v| v.as_object())?;
    if official.get("status").and_then(|v| v.as_str())? != "active" {
        return None;
    }

    // Require a resolvable repository URL (open-source signal).
    let repository_url = infer_repository_url(server)?;

    // Basic registry fields.
    let display_name = server.get("name")?.as_str()?.to_string();
    let description = server
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let version = server.get("version")?.as_str()?.to_string();

    // Ensure it looks like a server schema entry (current MCP standard signal).
    let schema_ok = server
        .get("$schema")
        .and_then(|v| v.as_str())
        .map(|s| s.contains("server.schema.json"))
        .unwrap_or(false);
    if !schema_ok {
        return None;
    }

    // ── Tier 1a: stdio packages (npm / pypi) with no required env vars ──────
    let mut eligible: Vec<(String, McpServerEntry)> = Vec::new();
    if let Some(packages) = server.get("packages").and_then(|v| v.as_array()) {
        for pkg in packages {
            // Allow optional env vars; reject only when isRequired: true.
            if has_required_fields(pkg.get("environmentVariables")) {
                continue;
            }
            if let Some(entry) = build_mcp_entry_for_package(pkg, &display_name, &version, true) {
                eligible.push(entry);
            }
        }
    }

    // ── Tier 1b: HTTP / SSE remotes with no required headers (fallback) ──────
    // Only considered when no qualifying stdio package was found first.
    if eligible.is_empty()
        && let Some(remotes) = server.get("remotes").and_then(|v| v.as_array())
    {
        for remote in remotes {
            // Skip remotes that require auth headers — those go to Add (Disabled).
            if has_required_fields(remote.get("headers")) {
                continue;
            }
            if let Some(entry) = build_mcp_entry_for_remote(remote, &display_name, true) {
                let key = entry.url.clone().unwrap_or_else(|| display_name.clone());
                eligible.push((key, entry));
            }
        }
    }

    if eligible.is_empty() {
        return None;
    }
    // Prefer the lexicographically first identifier / URL for determinism.
    eligible.sort_by(|a, b| a.0.cmp(&b.0));
    let (package_identifier, tool) = eligible.into_iter().next()?;

    Some(PopularMcpServer {
        display_name,
        description,
        repository_url,
        package_identifier,
        version,
        tool,
    })
}

/// Build a [`RegistryBrowseEntry`] from any active registry item.
///
/// Unlike [`popular_candidate_from_registry_item`], this accepts servers of any
/// transport type and any env-var count, showing them all in the browse panel.
/// Delegates to [`popular_candidate_from_registry_item`] for `quick_add`, and
/// to [`disabled_candidate_from_registry_item`] for `add_disabled`.
///
/// Returns `None` only if the item lacks required fields (name, status active).
fn browse_entry_from_registry_item(item: &serde_json::Value) -> Option<RegistryBrowseEntry> {
    let server = item.get("server")?;
    let meta = item.get("_meta")?;

    // Only show active listings.
    let official = meta
        .get("io.modelcontextprotocol.registry/official")
        .and_then(|v| v.as_object())?;
    if official.get("status").and_then(|v| v.as_str())? != "active" {
        return None;
    }

    let display_name = server.get("name")?.as_str()?.to_string();
    let description = server
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let repository_url = infer_repository_url(server).unwrap_or_default();
    let version = server
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Tier 1: zero-config Quick Add.
    let quick_add = popular_candidate_from_registry_item(item).map(|p| p.tool);
    // Tier 2: needs configuration — only populated when Tier 1 is absent.
    let add_disabled = if quick_add.is_none() {
        disabled_candidate_from_registry_item(item)
    } else {
        None
    };

    Some(RegistryBrowseEntry {
        display_name,
        description,
        repository_url,
        version,
        quick_add,
        add_disabled,
    })
}

/// Attempt to build a disabled [`McpServerEntry`] from a registry item that
/// requires at least one configuration secret before it can connect.
///
/// Covers two transport families:
/// * **stdio** (npm / pypi) — requires at least one `isRequired` env var.
/// * **HTTP / SSE remote** — requires at least one `isRequired` auth header.
///
/// The entry is returned with `enabled = false` so it appears in the user's
/// MCP list awaiting configuration.  Returns `None` for servers that already
/// qualify for Quick Add (no required secrets) or for unsupported formats.
fn disabled_candidate_from_registry_item(item: &serde_json::Value) -> Option<McpServerEntry> {
    let server = item.get("server")?;
    let meta = item.get("_meta")?;

    // Active + repository (or inferred) + $schema — same gates as quick_add.
    let official = meta
        .get("io.modelcontextprotocol.registry/official")
        .and_then(|v| v.as_object())?;
    if official.get("status").and_then(|v| v.as_str())? != "active" {
        return None;
    }
    infer_repository_url(server)?; // must be resolvable
    let schema_ok = server
        .get("$schema")
        .and_then(|v| v.as_str())
        .map(|s| s.contains("server.schema.json"))
        .unwrap_or(false);
    if !schema_ok {
        return None;
    }

    let display_name = server.get("name")?.as_str()?.to_string();
    let version = server.get("version")?.as_str()?.to_string();

    // ── Stdio packages (npm / pypi) with at least one required env var ──────
    let mut eligible: Vec<(String, McpServerEntry)> = Vec::new();
    if let Some(packages) = server.get("packages").and_then(|v| v.as_array()) {
        for pkg in packages {
            // Only qualify packages that have at least one *required* env var —
            // those are exactly the ones Quick Add rejects.
            if !has_required_fields(pkg.get("environmentVariables")) {
                // Zero required env vars → Quick Add handles this; skip here.
                continue;
            }
            if let Some(entry) = build_mcp_entry_for_package(pkg, &display_name, &version, false) {
                eligible.push(entry);
            }
        }
    }

    // ── HTTP / SSE remotes with at least one required header (fallback) ──────
    // Only considered when no qualifying stdio package was found first.
    if eligible.is_empty()
        && let Some(remotes) = server.get("remotes").and_then(|v| v.as_array())
    {
        for remote in remotes {
            // Only qualify remotes that require auth headers —
            // those with no required headers qualify for Quick Add instead.
            if !has_required_fields(remote.get("headers")) {
                continue;
            }
            if let Some(entry) = build_mcp_entry_for_remote(remote, &display_name, false) {
                let key = entry.url.clone().unwrap_or_else(|| display_name.clone());
                eligible.push((key, entry));
            }
        }
    }

    if eligible.is_empty() {
        return None;
    }
    eligible.sort_by(|a, b| a.0.cmp(&b.0));
    Some(eligible.into_iter().next()?.1)
}

/// Convert an arbitrary registry name into a safe, stable tool identifier.
///
/// Lowercases all ASCII letters, keeps alphanumerics and `_`/`-`, and
/// replaces runs of other characters with a single `-`.  Leading/trailing
/// dashes are stripped.  Returns `"mcp-server"` if the result would be empty.
pub fn normalize_mcp_server_name(input: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;

    for ch in input.trim().chars() {
        let lower = ch.to_ascii_lowercase();
        let is_ok = lower.is_ascii_alphanumeric() || lower == '_' || lower == '-';
        if is_ok {
            out.push(lower);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }

    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "mcp-server".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popular_candidate_filters_out_required_env_var_servers() {
        let json = serde_json::json!({
            "servers": [
                // [0] Blocked: has a required env var → cannot quick-add.
                {
                    "server": {
                        "$schema": "https://modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                        "name": "com.example/needs-env",
                        "description": "Requires env",
                        "repository": {"url": "https://github.com/example/repo"},
                        "version": "1.0.0",
                        "packages": [
                            {
                                "registryType": "npm",
                                "identifier": "@example/needs-env",
                                "transport": {"type": "stdio"},
                                "environmentVariables": [{"name": "TOKEN", "isRequired": true}]
                            }
                        ]
                    },
                    "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active"}}
                },
                // [1] Allowed: zero env vars.
                {
                    "server": {
                        "$schema": "https://modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                        "name": "com.example/good",
                        "description": "No config",
                        "repository": {"url": "https://github.com/example/good"},
                        "version": "2.3.4",
                        "packages": [
                            {
                                "registryType": "npm",
                                "identifier": "@example/good",
                                "transport": {"type": "stdio"},
                                "environmentVariables": []
                            }
                        ]
                    },
                    "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active"}}
                },
                // [2] Allowed: declares optional env vars only (isRequired absent/false).
                {
                    "server": {
                        "$schema": "https://modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                        "name": "com.example/optional-env",
                        "description": "Optional API key for enhanced rate limits",
                        "repository": {"url": "https://github.com/example/optional-env"},
                        "version": "3.0.0",
                        "packages": [
                            {
                                "registryType": "npm",
                                "identifier": "@example/optional-env",
                                "transport": {"type": "stdio"},
                                "environmentVariables": [
                                    {"name": "API_KEY", "isRequired": false},
                                    {"name": "LOG_LEVEL"}
                                ]
                            }
                        ]
                    },
                    "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active"}}
                }
            ],
            "metadata": {"nextCursor": null}
        });

        let servers = json.get("servers").and_then(|v| v.as_array()).unwrap();

        // Required env var → blocked.
        let a = popular_candidate_from_registry_item(&servers[0]);
        assert!(a.is_none(), "server with required env var must not qualify");

        // Zero env vars → allowed.
        let b = popular_candidate_from_registry_item(&servers[1]).unwrap();
        assert_eq!(b.display_name, "com.example/good");
        assert_eq!(b.package_identifier, "@example/good");
        assert_eq!(b.tool.command.as_deref(), Some("npx"));
        assert_eq!(b.tool.transport, McpTransportType::Stdio);
        assert!(
            b.tool
                .args
                .iter()
                .any(|s| s.contains("@example/good@2.3.4"))
        );

        // Optional-only env vars → allowed (server works without any config).
        let c = popular_candidate_from_registry_item(&servers[2]).unwrap();
        assert_eq!(c.display_name, "com.example/optional-env");
        assert_eq!(c.package_identifier, "@example/optional-env");
        assert_eq!(c.tool.command.as_deref(), Some("npx"));
        // The McpServerEntry env map must be empty — optional vars are not injected.
        assert!(
            c.tool.env.is_empty(),
            "optional env vars must not be pre-populated in the tool entry"
        );
        assert!(
            c.tool
                .args
                .iter()
                .any(|s| s.contains("@example/optional-env@3.0.0"))
        );
    }

    #[test]
    fn disabled_candidate_produced_for_required_env_var_server() {
        let item = serde_json::json!({
            "server": {
                "$schema": "https://modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                "name": "com.example/needs-token",
                "description": "Requires an API token",
                "repository": {"url": "https://github.com/example/needs-token"},
                "version": "1.2.0",
                "packages": [
                    {
                        "registryType": "npm",
                        "identifier": "@example/needs-token",
                        "transport": {"type": "stdio"},
                        "environmentVariables": [{"name": "API_TOKEN", "isRequired": true}]
                    }
                ]
            },
            "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active"}}
        });

        // quick_add must be absent (has required env var).
        let quick = popular_candidate_from_registry_item(&item);
        assert!(
            quick.is_none(),
            "required-env server must not qualify for quick_add"
        );

        // disabled_candidate must be present.
        let disabled = disabled_candidate_from_registry_item(&item).unwrap();
        assert!(
            !disabled.enabled,
            "add_disabled entry must have enabled=false"
        );
        assert!(
            disabled.env.is_empty(),
            "env map must be empty — user fills it in via config panel"
        );
        assert_eq!(disabled.command.as_deref(), Some("npx"));
        assert!(
            disabled
                .args
                .iter()
                .any(|s| s.contains("@example/needs-token@1.2.0"))
        );

        // browse_entry must set quick_add=None and add_disabled=Some.
        let entry = browse_entry_from_registry_item(&item).unwrap();
        assert!(entry.quick_add.is_none());
        assert!(entry.add_disabled.is_some());
        assert!(!entry.add_disabled.unwrap().enabled);
    }

    #[test]
    fn browse_entry_quick_add_suppresses_add_disabled() {
        // A server with zero required env vars should get quick_add but NOT add_disabled.
        let item = serde_json::json!({
            "server": {
                "$schema": "https://modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                "name": "com.example/zero-config",
                "description": "No env required",
                "repository": {"url": "https://github.com/example/zero-config"},
                "version": "2.0.0",
                "packages": [
                    {
                        "registryType": "npm",
                        "identifier": "@example/zero-config",
                        "transport": {"type": "stdio"},
                        "environmentVariables": []
                    }
                ]
            },
            "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active"}}
        });

        let entry = browse_entry_from_registry_item(&item).unwrap();
        assert!(
            entry.quick_add.is_some(),
            "zero-config server must have quick_add"
        );
        assert!(
            entry.add_disabled.is_none(),
            "add_disabled must not be set when quick_add is Some"
        );
    }

    #[test]
    fn pypi_quick_add_uses_uvx_command() {
        // A pypi/stdio server with no required env vars must get Quick Add
        // with `uvx identifier==version` — not npx.
        let item = serde_json::json!({
            "server": {
                "$schema": "https://modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                "name": "io.github.example/pypi-server",
                "description": "A pypi MCP server",
                "repository": {"url": "https://github.com/example/pypi-server"},
                "version": "1.0.0",
                "packages": [
                    {
                        "registryType": "pypi",
                        "identifier": "example-mcp-server",
                        "transport": {"type": "stdio"},
                        "environmentVariables": []
                    }
                ]
            },
            "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active"}}
        });

        let candidate = popular_candidate_from_registry_item(&item).unwrap();
        assert_eq!(candidate.tool.command.as_deref(), Some("uvx"));
        assert!(
            candidate
                .tool
                .args
                .iter()
                .any(|s| s == "example-mcp-server==1.0.0"),
            "uvx arg must be identifier==version, got {:?}",
            candidate.tool.args
        );
        assert!(candidate.tool.enabled);
        assert_eq!(candidate.package_identifier, "example-mcp-server");
    }

    #[test]
    fn pypi_disabled_candidate_uses_uvx_command() {
        // A pypi/stdio server with a required env var must become add_disabled
        // with `uvx identifier==version` — not npx.
        let item = serde_json::json!({
            "server": {
                "$schema": "https://modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                "name": "io.github.redis/mcp-redis",
                "description": "Redis MCP server",
                "version": "0.4.1",
                "packages": [
                    {
                        "registryType": "pypi",
                        "identifier": "redis-mcp-server",
                        "transport": {"type": "stdio"},
                        "environmentVariables": [{"name": "REDIS_URL", "isRequired": true}]
                    }
                ]
            },
            "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active"}}
        });

        // Must not qualify for Quick Add (has required env var).
        let quick = popular_candidate_from_registry_item(&item);
        assert!(
            quick.is_none(),
            "required-env pypi server must not get quick_add"
        );

        // Must get an add_disabled entry using uvx.
        let disabled = disabled_candidate_from_registry_item(&item).unwrap();
        assert!(!disabled.enabled);
        assert_eq!(disabled.command.as_deref(), Some("uvx"));
        assert!(
            disabled.args.iter().any(|s| s == "redis-mcp-server==0.4.1"),
            "uvx arg must be identifier==version, got {:?}",
            disabled.args
        );

        // browse_entry must surface it as add_disabled.
        let entry = browse_entry_from_registry_item(&item).unwrap();
        assert!(entry.quick_add.is_none());
        assert!(entry.add_disabled.is_some());
        // Repo URL inferred from io.github.redis/mcp-redis name.
        assert_eq!(entry.repository_url, "https://github.com/redis/mcp-redis");
    }

    #[test]
    fn infer_repository_url_from_io_github_name() {
        let server_with_repo = serde_json::json!({
            "name": "io.github.foo/bar",
            "repository": {"url": "https://github.com/explicit/repo"}
        });
        // Explicit field wins.
        assert_eq!(
            infer_repository_url(&server_with_repo),
            Some("https://github.com/explicit/repo".to_string())
        );

        let server_no_repo = serde_json::json!({"name": "io.github.redis/mcp-redis"});
        // Inferred from name.
        assert_eq!(
            infer_repository_url(&server_no_repo),
            Some("https://github.com/redis/mcp-redis".to_string())
        );

        let server_unknown = serde_json::json!({"name": "com.example/something"});
        // No repo field and no io.github.* pattern → None.
        assert!(infer_repository_url(&server_unknown).is_none());
    }

    #[test]
    fn normalize_mcp_server_name_is_safe_and_stable() {
        assert_eq!(
            normalize_mcp_server_name("Com.Example/Thing"),
            "com-example-thing"
        );
        assert_eq!(normalize_mcp_server_name("  "), "mcp-server");
    }

    #[test]
    fn priority_index_curated_packages_sort_before_unknowns() {
        // Known curated packages get their 0-based slot.
        assert_eq!(priority_index("chrome-devtools-mcp"), 0);
        assert_eq!(priority_index("@gitkraken/gk"), 1);
        assert_eq!(priority_index("@dollhousemcp/mcp-server"), 19);

        // Unknown packages receive usize::MAX and sort last.
        assert_eq!(priority_index("some-random-server"), usize::MAX);
        assert_eq!(priority_index(""), usize::MAX);

        // Curated packages sort before unknown ones.
        assert!(priority_index("chrome-devtools-mcp") < priority_index("zzz-unknown"));
        assert!(priority_index("@dollhousemcp/mcp-server") < priority_index("zzz-unknown"));
    }

    #[test]
    fn priority_packages_list_has_exactly_20_entries() {
        assert_eq!(PRIORITY_PACKAGES.len(), 20);
    }

    #[test]
    fn priority_packages_are_all_distinct() {
        let mut seen = std::collections::HashSet::new();
        for pkg in PRIORITY_PACKAGES {
            assert!(
                seen.insert(*pkg),
                "duplicate entry in PRIORITY_PACKAGES: {pkg}"
            );
        }
    }

    // ── Remote / HTTP / SSE transport tests ──────────────────────────────────

    /// A streamable-HTTP remote with no required auth headers should qualify for
    /// Quick Add (enabled = true) and produce no Add-Disabled entry.
    #[test]
    fn remote_http_quick_add_no_required_headers() {
        let item = serde_json::json!({
            "server": {
                "$schema": "https://modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                "name": "io.github.example/open-remote",
                "description": "Public HTTP MCP server — no auth required",
                "repository": {"url": "https://github.com/example/open-remote"},
                "version": "1.0.0",
                // No packages array — remote only.
                "remotes": [{
                    "type": "streamable-http",
                    "url": "https://mcp.example.com/server",
                    "headers": [
                        // Optional header — isRequired absent, treated as false.
                        {"name": "X-Trace-Id", "value": "optional-trace", "isRequired": false}
                    ]
                }]
            },
            "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active"}}
        });

        let quick = popular_candidate_from_registry_item(&item);
        assert!(
            quick.is_some(),
            "expected Quick Add candidate for open remote"
        );
        let tool = quick.unwrap().tool;
        assert_eq!(tool.transport, McpTransportType::Http);
        assert_eq!(tool.url.as_deref(), Some("https://mcp.example.com/server"));
        assert!(tool.command.is_none(), "HTTP entry must have no command");
        assert!(tool.headers.is_empty(), "headers must not be pre-populated");
        assert!(tool.enabled, "Quick Add must produce enabled = true");

        let disabled = disabled_candidate_from_registry_item(&item);
        assert!(
            disabled.is_none(),
            "no required headers → should not produce Add-Disabled"
        );
    }

    /// A streamable-HTTP remote that requires an Authorization bearer token
    /// (like Smithery-hosted servers) should produce an Add-Disabled entry and
    /// no Quick-Add entry.
    #[test]
    fn remote_http_disabled_required_header() {
        let item = serde_json::json!({
            "server": {
                "$schema": "https://modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                "name": "ai.smithery/Nekzus-npm-sentinel-mcp",
                "description": "npm sentinel via Smithery",
                // No repository field — inferred from io.github name pattern via
                // the smithery name convention; use explicit repo to keep test simple.
                "repository": {"url": "https://github.com/Nekzus/npm-sentinel-mcp"},
                "version": "1.0.0",
                // No packages array.
                "remotes": [{
                    "type": "streamable-http",
                    "url": "https://server.smithery.ai/@Nekzus/npm-sentinel-mcp/mcp",
                    "headers": [{
                        "name": "Authorization",
                        "value": "Bearer {smithery_api_key}",
                        "isRequired": true,
                        "isSecret": true
                    }]
                }]
            },
            "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active"}}
        });

        let quick = popular_candidate_from_registry_item(&item);
        assert!(
            quick.is_none(),
            "required auth header → must not qualify for Quick Add"
        );

        let disabled = disabled_candidate_from_registry_item(&item);
        assert!(
            disabled.is_some(),
            "expected Add-Disabled entry for Smithery server"
        );
        let entry = disabled.unwrap();
        assert_eq!(entry.transport, McpTransportType::Http);
        assert_eq!(
            entry.url.as_deref(),
            Some("https://server.smithery.ai/@Nekzus/npm-sentinel-mcp/mcp")
        );
        assert!(entry.command.is_none(), "HTTP entry must have no command");
        assert!(
            entry.headers.is_empty(),
            "headers must be left empty for user to fill in"
        );
        assert!(!entry.enabled, "Add-Disabled must produce enabled = false");
    }

    /// An SSE remote that requires an auth header should appear as Add-Disabled
    /// with McpTransportType::Sse (not Http).
    #[test]
    fn remote_sse_disabled_required_header() {
        let item = serde_json::json!({
            "server": {
                "$schema": "https://modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                "name": "io.github.example/sse-server",
                "description": "SSE-based MCP server",
                "repository": {"url": "https://github.com/example/sse-server"},
                "version": "0.9.0",
                "remotes": [{
                    "type": "sse",
                    "url": "https://sse.example.com/mcp",
                    "headers": [{
                        "name": "X-Api-Key",
                        "value": "{api_key}",
                        "isRequired": true
                    }]
                }]
            },
            "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active"}}
        });

        let quick = popular_candidate_from_registry_item(&item);
        assert!(
            quick.is_none(),
            "required header → not a Quick Add candidate"
        );

        let disabled = disabled_candidate_from_registry_item(&item);
        assert!(
            disabled.is_some(),
            "expected Add-Disabled entry for SSE server"
        );
        let entry = disabled.unwrap();
        assert_eq!(entry.transport, McpTransportType::Sse);
        assert_eq!(entry.url.as_deref(), Some("https://sse.example.com/mcp"));
        assert!(entry.command.is_none());
        assert!(entry.headers.is_empty());
        assert!(!entry.enabled);
    }
}
