//! Platform detection utilities.
//!
//! Cross-platform helpers for detecting OS-level settings such as the
//! system color scheme. These are shared between the CLI and GUI crates
//! so that both can respect the user's system preferences.

/// Detect whether the operating system is configured for dark mode.
///
/// Heuristics per platform:
///
/// - **macOS**: `defaults read -g AppleInterfaceStyle` → "Dark" means dark mode.
/// - **Linux**: checks `$GTK_THEME` for a `-dark` suffix, then falls back to
///   `gsettings get org.gnome.desktop.interface color-scheme`.
/// - **Windows**: reads `AppsUseLightTheme` from the registry via `reg query`.
///
/// Returns `true` (dark) when detection fails or the platform is unrecognized.
pub fn detect_system_dark_mode() -> bool {
    detect_system_dark_mode_inner().unwrap_or(true)
}

fn detect_system_dark_mode_inner() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        // `defaults read -g AppleInterfaceStyle` prints "Dark" when dark mode is
        // active and exits with a non-zero status when light mode is active.
        let output = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Some(stdout.trim().eq_ignore_ascii_case("dark"));
    }

    #[cfg(target_os = "linux")]
    {
        // 1. Quick env-var check: many GTK apps export GTK_THEME=Adwaita:dark.
        if let Ok(gtk) = std::env::var("GTK_THEME") {
            let lower = gtk.to_ascii_lowercase();
            if lower.contains("dark") {
                return Some(true);
            }
            if lower.contains("light") {
                return Some(false);
            }
        }
        // 2. XDG portal / GNOME color-scheme (works inside Flatpak too).
        if let Ok(output) = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let lower = stdout.trim().to_ascii_lowercase();
            if lower.contains("dark") {
                return Some(true);
            }
            if lower.contains("light") {
                return Some(false);
            }
        }
        return Some(true); // default dark on Linux
    }

    #[cfg(target_os = "windows")]
    {
        // Registry: HKCU\...\Personalize\AppsUseLightTheme  (DWORD: 0 = dark, 1 = light)
        if let Ok(output) = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "AppsUseLightTheme",
            ])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // The output contains "0x0" for dark, "0x1" for light.
            if stdout.contains("0x0") {
                return Some(true);
            }
            if stdout.contains("0x1") {
                return Some(false);
            }
        }
        return Some(true); // default dark on Windows
    }

    // Unsupported platform — fall back to dark.
    #[allow(unreachable_code)]
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_system_dark_mode_does_not_panic() {
        // The function relies on platform-specific commands; verify it returns a
        // bool without panicking regardless of the environment.
        let _result: bool = detect_system_dark_mode();
    }
}
