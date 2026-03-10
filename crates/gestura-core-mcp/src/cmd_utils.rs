use std::collections::HashMap;
use std::path::PathBuf;

/// Resolves a generic command name to its platform-specific executable variant.
/// On Windows, `npx` becomes `npx.cmd`, `uvx` becomes `uvx.exe`, etc.
#[allow(dead_code)]
pub fn resolve_mcp_command(command: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        if command == "npx" {
            return "npx.cmd".to_string();
        }
        if command == "npm" {
            return "npm.cmd".to_string();
        }
        if command == "uv" {
            return "uv.exe".to_string(); // Or uv.cmd depending on install method
        }
        if command == "uvx" {
            return "uvx.exe".to_string();
        }
    }
    command.to_string()
}

/// Injects additional standard paths into a process environment map,
/// ensuring that GUI apps (which often launch with restricted profiles on macOS/Linux)
/// can still find `npx`, `uv`, `docker` etc., where they are typically installed.
#[allow(dead_code)]
pub fn inject_enriched_path(provided_env: &mut HashMap<String, String>) {
    // Only apply enrichment if PATH doesn't exist explicitly in the user-provided env map
    if provided_env.keys().any(|k| k.eq_ignore_ascii_case("PATH")) {
        return;
    }

    if let Some(existing_path) = std::env::var_os("PATH") {
        let mut paths = std::env::split_paths(&existing_path).collect::<Vec<_>>();

        let extra_paths = vec!["/usr/local/bin", "/opt/homebrew/bin", "/opt/local/bin"];

        for ep in extra_paths {
            let p = PathBuf::from(ep);
            if p.exists() && !paths.contains(&p) {
                paths.push(p);
            }
        }

        // Add ~/.cargo/bin if it exists
        if let Some(mut home) = dirs::home_dir() {
            home.push(".cargo");
            home.push("bin");
            if home.exists() && !paths.contains(&home) {
                paths.push(home);
            }
        }

        if let Ok(new_path) = std::env::join_paths(paths) {
            provided_env.insert("PATH".to_string(), new_path.to_string_lossy().to_string());
        }
    }
}
