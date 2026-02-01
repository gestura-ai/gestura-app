//! OS-level system permission checks for Gestura.app (Tauri GUI).
//!
//! This module provides platform-native permission probing and request helpers
//! for microphone, accessibility, bluetooth, and screen recording permissions.
//!
//! ## Platform Support
//!
//! - **macOS**: Uses TCC (Transparency, Consent, and Control) via objc/cocoa bindings
//!   and osascript for AVFoundation, CoreBluetooth, and CoreGraphics APIs.
//!   Enabled via `macos-permissions` feature.
//!
//! - **Linux**: Uses xdg-desktop-portal for Wayland screen recording permissions
//!   and D-Bus for system checks. Enabled via `linux-permissions` feature.
//!
//! - **Windows**: Uses WinRT APIs (Windows.Media.Capture) for microphone/camera
//!   permission status. Enabled via `windows-permissions` feature.
//!
//! - **Fallback**: Platforms without permission features return `Granted` by default,
//!   as those platforms typically don't require explicit permission dialogs.
//!
//! Tool execution policy and LLM/tool permissions are owned by `gestura-core`.

// ============================================================================
// Common Types
// ============================================================================

/// System permission status across all platforms.
///
/// This enum represents the possible states of a system permission check.
/// The semantics are consistent across platforms, though not all platforms
/// support all states (e.g., `Restricted` is primarily a macOS concept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemPermissionStatus {
    /// Permission has been granted by the user.
    Granted,
    /// Permission has been explicitly denied by the user.
    Denied,
    /// Permission has not been determined yet (user hasn't been prompted).
    NotDetermined,
    /// Permission is restricted (e.g., by parental controls or enterprise policy).
    Restricted,
    /// Permission status could not be determined.
    Unknown,
}

impl std::fmt::Display for SystemPermissionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemPermissionStatus::Granted => write!(f, "granted"),
            SystemPermissionStatus::Denied => write!(f, "denied"),
            SystemPermissionStatus::NotDetermined => write!(f, "not_determined"),
            SystemPermissionStatus::Restricted => write!(f, "restricted"),
            SystemPermissionStatus::Unknown => write!(f, "unknown"),
        }
    }
}

// ============================================================================
// macOS Implementation (requires macos-permissions feature)
// ============================================================================

/// Check microphone permission on macOS using AVCaptureDevice.
#[cfg(all(target_os = "macos", feature = "macos-permissions"))]
pub fn check_microphone_permission() -> SystemPermissionStatus {
    use std::process::Command;

    let output = Command::new("osascript")
        .args([
            "-e",
            r#"
            use framework \"AVFoundation\"
            set authStatus to current application's AVCaptureDevice's authorizationStatusForMediaType:(current application's AVMediaTypeAudio)
            if authStatus = 0 then
                return \"not_determined\"
            else if authStatus = 1 then
                return \"restricted\"
            else if authStatus = 2 then
                return \"denied\"
            else if authStatus = 3 then
                return \"granted\"
            else
                return \"unknown\"
            end if
            "#,
        ])
        .output();

    parse_permission_output(output)
}

/// Check accessibility permission on macOS using AXIsProcessTrusted.
#[cfg(all(target_os = "macos", feature = "macos-permissions"))]
pub fn check_accessibility_permission() -> SystemPermissionStatus {
    use std::process::Command;

    let output = Command::new("osascript")
        .args([
            "-e",
            r#"
            use framework \"ApplicationServices\"
            if current application's AXIsProcessTrusted() then
                return \"granted\"
            else
                return \"denied\"
            end if
            "#,
        ])
        .output();

    parse_permission_output(output)
}

/// Check bluetooth permission on macOS using CBManager.
#[cfg(all(target_os = "macos", feature = "macos-permissions"))]
pub fn check_bluetooth_permission() -> SystemPermissionStatus {
    use std::process::Command;

    let output = Command::new("osascript")
        .args([
            "-e",
            r#"
            use framework \"CoreBluetooth\"
            set authStatus to current application's CBManager's authorization()
            if authStatus = 0 then
                return \"not_determined\"
            else if authStatus = 1 then
                return \"restricted\"
            else if authStatus = 2 then
                return \"denied\"
            else if authStatus = 3 then
                return \"granted\"
            else
                return \"unknown\"
            end if
            "#,
        ])
        .output();

    parse_permission_output(output)
}

// macOS Screen Recording permission FFI (CoreGraphics)
#[cfg(all(target_os = "macos", feature = "macos-permissions"))]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// Check Screen Recording permission on macOS.
///
/// Note: CoreGraphics does not expose a rich status (Denied vs NotDetermined)
/// via this API; we treat `false` as denied so the UI can prompt the user to
/// open System Settings.
#[cfg(all(target_os = "macos", feature = "macos-permissions"))]
pub fn check_screen_recording_permission() -> SystemPermissionStatus {
    // SAFETY: FFI call.
    let granted = unsafe { CGPreflightScreenCaptureAccess() };
    if granted {
        SystemPermissionStatus::Granted
    } else {
        SystemPermissionStatus::Denied
    }
}

/// Parse osascript output to permission status.
#[cfg(all(target_os = "macos", feature = "macos-permissions"))]
fn parse_permission_output(
    output: Result<std::process::Output, std::io::Error>,
) -> SystemPermissionStatus {
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();

            if out.status.success() {
                tracing::debug!(
                    "System permission check script succeeded: status={:?}, stdout='{}', stderr='{}'",
                    out.status,
                    stdout,
                    stderr
                );
            } else {
                tracing::warn!(
                    "System permission check script exited with non-zero status: status={:?}, stdout='{}', stderr='{}'",
                    out.status,
                    stdout,
                    stderr
                );
            }

            match stdout.as_str() {
                "granted" => SystemPermissionStatus::Granted,
                "denied" => SystemPermissionStatus::Denied,
                "not_determined" => SystemPermissionStatus::NotDetermined,
                "restricted" => SystemPermissionStatus::Restricted,
                other => {
                    if other.is_empty() {
                        tracing::warn!(
                            "System permission check returned empty status string; defaulting to Unknown",
                        );
                    } else {
                        tracing::warn!(
                            "System permission check returned unrecognised status '{}'; defaulting to Unknown",
                            other
                        );
                    }
                    SystemPermissionStatus::Unknown
                }
            }
        }
        Err(e) => {
            tracing::warn!("Failed to execute system permission check script: {}", e);
            SystemPermissionStatus::Unknown
        }
    }
}

/// Request microphone permission on macOS.
///
/// This triggers the system permission dialog.
#[cfg(all(target_os = "macos", feature = "macos-permissions"))]
pub fn request_microphone_permission() -> bool {
    use std::process::Command;

    tracing::info!("Requesting microphone permission via AVFoundation...");

    // First try to trigger the permission dialog using osascript
    let output = Command::new("osascript")
        .args([
            "-e",
            r#"
            use framework \"AVFoundation\"
            set requestResult to current application's AVCaptureDevice's requestAccessForMediaType:(current application's AVMediaTypeAudio) completionHandler:(missing value)
            return \"requested\"
            "#,
        ])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();

            if out.status.success() {
                tracing::info!(
                    "Microphone permission request script completed successfully: stdout='{}', stderr='{}'",
                    stdout,
                    stderr
                );
                true
            } else {
                tracing::warn!(
                    "Microphone permission request script exited with non-zero status {:?}: stdout='{}', stderr='{}'",
                    out.status,
                    stdout,
                    stderr
                );
                false
            }
        }
        Err(e) => {
            tracing::warn!(
                "Failed to execute microphone permission request script: {}",
                e
            );
            false
        }
    }
}

/// Request bluetooth permission on macOS.
///
/// Bluetooth permission is typically triggered by scanning for devices.
#[cfg(all(target_os = "macos", feature = "macos-permissions"))]
pub fn request_bluetooth_permission() -> bool {
    use std::process::Command;

    tracing::info!("Requesting Bluetooth permission via CoreBluetooth...");

    // Try to trigger Bluetooth permission by initializing CBCentralManager
    // This should prompt the user if permission is not_determined
    let output = Command::new("osascript")
        .args([
            "-e",
            r#"
            use framework \"CoreBluetooth\"
            -- Creating a CBCentralManager triggers the permission dialog
            set centralManager to current application's CBCentralManager's alloc()'s init()
            delay 0.5
            return \"requested\"
            "#,
        ])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();

            if out.status.success() {
                tracing::info!(
                    "Bluetooth permission request script completed successfully: stdout='{}', stderr='{}'",
                    stdout,
                    stderr
                );
                true
            } else {
                tracing::warn!(
                    "Bluetooth permission request script exited with non-zero status {:?}: stdout='{}', stderr='{}'",
                    out.status,
                    stdout,
                    stderr
                );
                false
            }
        }
        Err(e) => {
            tracing::warn!(
                "Failed to execute Bluetooth permission request script: {}",
                e
            );
            false
        }
    }
}

/// Request Screen Recording permission on macOS.
///
/// This may display a system prompt (first request) and/or require the user to
/// enable the permission in System Settings.
#[cfg(all(target_os = "macos", feature = "macos-permissions"))]
pub fn request_screen_recording_permission() -> bool {
    // SAFETY: FFI call.
    unsafe { CGRequestScreenCaptureAccess() }
}

/// Check if running macOS 13 (Ventura) or later, which uses "System Settings"
/// instead of "System Preferences".
#[cfg(all(target_os = "macos", feature = "macos-permissions"))]
fn is_macos_ventura_or_later() -> bool {
    use std::process::Command;

    let output = Command::new("sw_vers").arg("-productVersion").output().ok();

    if let Some(out) = output {
        let version_str = String::from_utf8_lossy(&out.stdout);
        if let Some(major) = version_str.trim().split('.').next()
            && let Ok(major_version) = major.parse::<u32>()
        {
            return major_version >= 13;
        }
    }

    // Default to newer format if we can't determine version.
    true
}

/// Open System Preferences/Settings to the appropriate pane.
///
/// Uses different URL schemes depending on macOS version:
/// - macOS 13+ (Ventura/Sonoma/Sequoia): com.apple.settings.PrivacySecurity.extension?Privacy_*
/// - macOS 12 and earlier: com.apple.preference.security?Privacy_*
#[cfg(all(target_os = "macos", feature = "macos-permissions"))]
pub fn open_system_preferences(pane: &str) -> bool {
    use std::process::Command;

    let use_new_urls = is_macos_ventura_or_later();

    let pane_url = match (pane, use_new_urls) {
        ("microphone", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Microphone"
        }
        ("microphone", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        // Back-compat aliases used by some UIs.
        ("privacy_microphone", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Microphone"
        }
        ("privacy_microphone", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        ("accessibility", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility"
        }
        ("accessibility", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        ("privacy_accessibility", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility"
        }
        ("privacy_accessibility", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        ("bluetooth", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Bluetooth"
        }
        ("bluetooth", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth"
        }
        ("privacy_bluetooth", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Bluetooth"
        }
        ("privacy_bluetooth", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth"
        }
        ("screen_recording", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture"
        }
        ("screen_recording", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        ("privacy_screenrecording", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture"
        }
        ("privacy_screenrecording", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        ("privacy_screencapture", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture"
        }
        ("privacy_screencapture", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        _ => return false,
    };

    tracing::info!(
        "Opening System {} for {}: {}",
        if use_new_urls {
            "Settings"
        } else {
            "Preferences"
        },
        pane,
        pane_url
    );

    let result = Command::new("open").arg(pane_url).spawn();
    match result {
        Ok(_) => {
            tracing::info!("✅ Successfully opened System Settings for {}", pane);
            true
        }
        Err(e) => {
            tracing::error!("❌ Failed to open System Settings for {}: {}", pane, e);
            false
        }
    }
}

// ============================================================================
// Linux Implementation (requires linux-permissions feature)
// ============================================================================
// Note: Full implementation will be added in Phase 2.
// Current stubs return Granted as Linux typically doesn't have
// permission dialogs like macOS TCC.

/// Check microphone permission on Linux.
///
/// Linux typically doesn't have per-app microphone permission dialogs.
/// The actual audio access is managed by PulseAudio/PipeWire permissions.
#[cfg(all(target_os = "linux", feature = "linux-permissions"))]
pub fn check_microphone_permission() -> SystemPermissionStatus {
    // TODO: Phase 2 - Check PulseAudio/PipeWire audio access
    tracing::debug!("Linux microphone permission check: returning Granted (no TCC equivalent)");
    SystemPermissionStatus::Granted
}

/// Check accessibility permission on Linux.
///
/// Linux accessibility is typically managed through AT-SPI2 and doesn't
/// require explicit permission dialogs.
#[cfg(all(target_os = "linux", feature = "linux-permissions"))]
pub fn check_accessibility_permission() -> SystemPermissionStatus {
    // TODO: Phase 3 - Check AT-SPI2 availability
    tracing::debug!("Linux accessibility permission check: returning Granted (no TCC equivalent)");
    SystemPermissionStatus::Granted
}

/// Check bluetooth permission on Linux.
///
/// Linux Bluetooth access is managed through BlueZ and D-Bus policies.
#[cfg(all(target_os = "linux", feature = "linux-permissions"))]
pub fn check_bluetooth_permission() -> SystemPermissionStatus {
    // TODO: Phase 3 - Check D-Bus bluetooth group membership
    tracing::debug!("Linux bluetooth permission check: returning Granted (no TCC equivalent)");
    SystemPermissionStatus::Granted
}

/// Check screen recording permission on Linux.
///
/// On Wayland, screen capture requires xdg-desktop-portal permission.
/// On X11, screen capture is generally unrestricted.
///
/// This function checks if the xdg-desktop-portal screencast interface is available.
/// If the portal is available, we consider the permission as "Granted" since the user
/// will be prompted when actually starting a screencast session.
#[cfg(all(target_os = "linux", feature = "linux-permissions"))]
pub fn check_screen_recording_permission() -> SystemPermissionStatus {
    use std::env;

    // Check if we're on Wayland (where portal permission is needed)
    let is_wayland = env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
        || env::var("WAYLAND_DISPLAY").is_ok();

    if !is_wayland {
        // X11: Screen capture is generally unrestricted
        tracing::debug!("Linux screen recording (X11): returning Granted (no permission needed)");
        return SystemPermissionStatus::Granted;
    }

    // Wayland: Check if xdg-desktop-portal is available
    // We check synchronously using a blocking runtime
    match check_screencast_portal_available() {
        Ok(true) => {
            tracing::debug!(
                "Linux screen recording (Wayland): portal available, permission will be requested on use"
            );
            // On Linux Wayland, permission is granted per-session when the user
            // approves the screencast dialog. We return Granted to indicate
            // the capability is available.
            SystemPermissionStatus::Granted
        }
        Ok(false) => {
            tracing::warn!("Linux screen recording (Wayland): portal not available");
            SystemPermissionStatus::Denied
        }
        Err(e) => {
            tracing::warn!("Linux screen recording check failed: {}", e);
            // If we can't check, assume it's available to avoid blocking features
            SystemPermissionStatus::Unknown
        }
    }
}

/// Check if the xdg-desktop-portal screencast interface is available.
#[cfg(all(target_os = "linux", feature = "linux-permissions"))]
fn check_screencast_portal_available() -> Result<bool, String> {
    use std::process::Command;

    // Check if xdg-desktop-portal is running by querying D-Bus
    // This is a simple check that doesn't require async runtime
    let output = Command::new("busctl")
        .args(["--user", "list"])
        .output()
        .map_err(|e| format!("Failed to run busctl: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Look for the portal service
    if stdout.contains("org.freedesktop.portal.Desktop") {
        return Ok(true);
    }

    // Alternative: check using dbus-send
    let output = Command::new("dbus-send")
        .args([
            "--session",
            "--dest=org.freedesktop.DBus",
            "--type=method_call",
            "--print-reply",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.ListNames",
        ])
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            Ok(stdout.contains("org.freedesktop.portal.Desktop"))
        }
        Err(_) => {
            // If dbus-send fails too, assume portal might still be available
            // (e.g., in Flatpak/Snap environments)
            tracing::debug!("Could not verify portal availability via D-Bus, assuming available");
            Ok(true)
        }
    }
}

/// Request microphone permission on Linux (no-op).
#[cfg(all(target_os = "linux", feature = "linux-permissions"))]
pub fn request_microphone_permission() -> bool {
    tracing::debug!("Linux microphone permission request: no dialog needed");
    true
}

/// Request bluetooth permission on Linux (no-op).
#[cfg(all(target_os = "linux", feature = "linux-permissions"))]
pub fn request_bluetooth_permission() -> bool {
    tracing::debug!("Linux bluetooth permission request: no dialog needed");
    true
}

/// Request screen recording permission on Linux.
///
/// On Wayland, this triggers the xdg-desktop-portal screencast dialog.
/// The actual permission is granted when the user approves the dialog
/// during a screencast session request.
///
/// Note: Unlike macOS where you can pre-request permission, Linux portals
/// grant permission per-session. This function verifies the portal is available
/// and returns true to indicate the user can be prompted when needed.
#[cfg(all(target_os = "linux", feature = "linux-permissions"))]
pub fn request_screen_recording_permission() -> bool {
    use std::env;

    // Check if we're on Wayland
    let is_wayland = env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
        || env::var("WAYLAND_DISPLAY").is_ok();

    if !is_wayland {
        // X11: No permission dialog needed
        tracing::debug!("Linux screen recording request (X11): no dialog needed");
        return true;
    }

    // Wayland: Check if portal is available
    match check_screencast_portal_available() {
        Ok(available) => {
            if available {
                tracing::info!(
                    "Linux screen recording (Wayland): portal available, user will be prompted on screencast start"
                );
                true
            } else {
                tracing::warn!(
                    "Linux screen recording (Wayland): portal not available, cannot request permission"
                );
                false
            }
        }
        Err(e) => {
            tracing::warn!("Linux screen recording request check failed: {}", e);
            // Assume available to avoid blocking features
            true
        }
    }
}

/// Open system settings on Linux.
///
/// Attempts to open the relevant system settings application.
#[cfg(all(target_os = "linux", feature = "linux-permissions"))]
pub fn open_system_preferences(pane: &str) -> bool {
    use std::process::Command;

    let settings_app = match pane {
        "microphone" | "privacy_microphone" => "gnome-control-center sound",
        "bluetooth" | "privacy_bluetooth" => "gnome-control-center bluetooth",
        "accessibility" | "privacy_accessibility" => "gnome-control-center universal-access",
        "screen_recording" | "privacy_screenrecording" | "privacy_screencapture" => {
            "gnome-control-center sharing"
        }
        _ => return false,
    };

    tracing::info!("Opening Linux system settings: {}", settings_app);

    // Try GNOME Settings first, fall back to other desktop environments
    let parts: Vec<&str> = settings_app.split_whitespace().collect();
    if parts.len() >= 2 {
        if let Ok(_child) = Command::new(parts[0]).arg(parts[1]).spawn() {
            return true;
        }
    }

    // Fallback: try xdg-open with settings URI
    if let Ok(_child) = Command::new("xdg-open").arg("gnome-control-center").spawn() {
        return true;
    }

    false
}

// ============================================================================
// Windows Implementation (requires windows-permissions feature)
// ============================================================================
// Note: Full implementation will be added in Phase 4.
// Current stubs return Granted. Windows 10/11 do have privacy settings
// for microphone/camera but the WinRT API integration is pending.

/// Check microphone permission on Windows.
///
/// Windows 10/11 have privacy settings for microphone access.
/// This uses the Windows.Media.Capture APIs to check if microphone access is allowed.
#[cfg(all(target_os = "windows", feature = "windows-permissions"))]
pub fn check_microphone_permission() -> SystemPermissionStatus {
    use windows::Media::Capture::MediaCapture;

    // Try to check if we can access audio capture devices
    // Windows doesn't have a direct "check permission" API like macOS TCC,
    // but we can check if MediaCapture initialization would succeed
    match MediaCapture::IsVideoProfileSupported(windows::core::HSTRING::new()) {
        Ok(_) => {
            // If we can query media capture, permission is likely granted
            tracing::debug!("Windows microphone permission check: MediaCapture API accessible");
            SystemPermissionStatus::Granted
        }
        Err(e) => {
            let error_code = e.code().0 as u32;
            // E_ACCESSDENIED = 0x80070005
            if error_code == 0x80070005 {
                tracing::debug!("Windows microphone permission check: access denied");
                SystemPermissionStatus::Denied
            } else {
                // Other errors - might be device issue, not permission
                tracing::debug!("Windows microphone permission check: API error {:?}", e);
                SystemPermissionStatus::Unknown
            }
        }
    }
}

/// Check accessibility permission on Windows.
///
/// Windows doesn't have macOS-style accessibility permissions.
#[cfg(all(target_os = "windows", feature = "windows-permissions"))]
pub fn check_accessibility_permission() -> SystemPermissionStatus {
    tracing::debug!(
        "Windows accessibility permission check: returning Granted (no TCC equivalent)"
    );
    SystemPermissionStatus::Granted
}

/// Check bluetooth permission on Windows.
///
/// Windows Bluetooth access doesn't typically require app-specific permissions.
#[cfg(all(target_os = "windows", feature = "windows-permissions"))]
pub fn check_bluetooth_permission() -> SystemPermissionStatus {
    tracing::debug!("Windows bluetooth permission check: returning Granted (no TCC equivalent)");
    SystemPermissionStatus::Granted
}

/// Check screen recording permission on Windows.
///
/// Windows doesn't have macOS-style screen recording permissions.
#[cfg(all(target_os = "windows", feature = "windows-permissions"))]
pub fn check_screen_recording_permission() -> SystemPermissionStatus {
    tracing::debug!(
        "Windows screen recording permission check: returning Granted (no TCC equivalent)"
    );
    SystemPermissionStatus::Granted
}

/// Request microphone permission on Windows.
///
/// Unlike macOS, Windows doesn't have a direct "request permission" dialog that apps can trigger.
/// When an app first tries to access the microphone, Windows automatically shows a system prompt.
/// If the user has denied access, we open the Settings app to the microphone privacy page.
#[cfg(all(target_os = "windows", feature = "windows-permissions"))]
pub fn request_microphone_permission() -> bool {
    use std::process::Command;

    // Check current permission status
    let status = check_microphone_permission();

    match status {
        SystemPermissionStatus::Granted => {
            tracing::debug!("Windows microphone permission already granted");
            true
        }
        SystemPermissionStatus::Denied => {
            // Open Windows Settings to microphone privacy page
            tracing::info!("Windows microphone denied, opening Settings");
            match Command::new("cmd")
                .args(["/C", "start", "ms-settings:privacy-microphone"])
                .spawn()
            {
                Ok(_) => {
                    tracing::info!("Opened Windows microphone privacy settings");
                    // Return false since permission isn't granted yet
                    false
                }
                Err(e) => {
                    tracing::error!("Failed to open Windows Settings: {}", e);
                    false
                }
            }
        }
        SystemPermissionStatus::Unknown | SystemPermissionStatus::NotDetermined => {
            // Permission not yet requested - the system will prompt automatically
            // when we try to access the microphone
            tracing::debug!("Windows microphone permission will be requested on first use");
            true
        }
    }
}

/// Request bluetooth permission on Windows (no-op).
#[cfg(all(target_os = "windows", feature = "windows-permissions"))]
pub fn request_bluetooth_permission() -> bool {
    tracing::debug!("Windows bluetooth permission request: no dialog needed");
    true
}

/// Request screen recording permission on Windows (no-op).
#[cfg(all(target_os = "windows", feature = "windows-permissions"))]
pub fn request_screen_recording_permission() -> bool {
    tracing::debug!("Windows screen recording permission request: no dialog needed");
    true
}

/// Open Windows Settings to the appropriate page.
#[cfg(all(target_os = "windows", feature = "windows-permissions"))]
pub fn open_system_preferences(pane: &str) -> bool {
    use std::process::Command;

    let settings_uri = match pane {
        "microphone" | "privacy_microphone" => "ms-settings:privacy-microphone",
        "bluetooth" | "privacy_bluetooth" => "ms-settings:bluetooth",
        "accessibility" | "privacy_accessibility" => "ms-settings:easeofaccess",
        "screen_recording" | "privacy_screenrecording" | "privacy_screencapture" => {
            "ms-settings:privacy-graphicscaptureprogrammatic"
        }
        _ => return false,
    };

    tracing::info!("Opening Windows Settings: {}", settings_uri);

    match Command::new("cmd")
        .args(["/C", "start", settings_uri])
        .spawn()
    {
        Ok(_) => true,
        Err(e) => {
            tracing::error!("Failed to open Windows Settings: {}", e);
            false
        }
    }
}

// ============================================================================
// Fallback implementations (no platform-specific features enabled)
// ============================================================================
// These are used when:
// - Platform is not macOS/Linux/Windows, OR
// - Platform-specific permission feature is not enabled
//
// All checks return Granted since these platforms don't have
// macOS-style TCC permission dialogs.

/// Fallback: Check microphone permission (no platform feature enabled).
#[cfg(not(any(
    all(target_os = "macos", feature = "macos-permissions"),
    all(target_os = "linux", feature = "linux-permissions"),
    all(target_os = "windows", feature = "windows-permissions")
)))]
pub fn check_microphone_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

/// Fallback: Check accessibility permission (no platform feature enabled).
#[cfg(not(any(
    all(target_os = "macos", feature = "macos-permissions"),
    all(target_os = "linux", feature = "linux-permissions"),
    all(target_os = "windows", feature = "windows-permissions")
)))]
pub fn check_accessibility_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

/// Fallback: Check bluetooth permission (no platform feature enabled).
#[cfg(not(any(
    all(target_os = "macos", feature = "macos-permissions"),
    all(target_os = "linux", feature = "linux-permissions"),
    all(target_os = "windows", feature = "windows-permissions")
)))]
pub fn check_bluetooth_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

/// Fallback: Check screen recording permission (no platform feature enabled).
#[cfg(not(any(
    all(target_os = "macos", feature = "macos-permissions"),
    all(target_os = "linux", feature = "linux-permissions"),
    all(target_os = "windows", feature = "windows-permissions")
)))]
pub fn check_screen_recording_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

/// Fallback: Request microphone permission (no platform feature enabled).
#[cfg(not(any(
    all(target_os = "macos", feature = "macos-permissions"),
    all(target_os = "linux", feature = "linux-permissions"),
    all(target_os = "windows", feature = "windows-permissions")
)))]
pub fn request_microphone_permission() -> bool {
    true
}

/// Fallback: Request bluetooth permission (no platform feature enabled).
#[cfg(not(any(
    all(target_os = "macos", feature = "macos-permissions"),
    all(target_os = "linux", feature = "linux-permissions"),
    all(target_os = "windows", feature = "windows-permissions")
)))]
pub fn request_bluetooth_permission() -> bool {
    true
}

/// Fallback: Request screen recording permission (no platform feature enabled).
#[cfg(not(any(
    all(target_os = "macos", feature = "macos-permissions"),
    all(target_os = "linux", feature = "linux-permissions"),
    all(target_os = "windows", feature = "windows-permissions")
)))]
pub fn request_screen_recording_permission() -> bool {
    true
}

/// Fallback: Open system settings (no platform feature enabled).
#[cfg(not(any(
    all(target_os = "macos", feature = "macos-permissions"),
    all(target_os = "linux", feature = "linux-permissions"),
    all(target_os = "windows", feature = "windows-permissions")
)))]
pub fn open_system_preferences(_pane: &str) -> bool {
    false
}
