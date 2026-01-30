//! OS-level system permission checks for Gestura.app (Tauri GUI).
//!
//! This module intentionally contains **only** platform permission probing/request helpers
//! (microphone, accessibility, bluetooth, screen recording, etc.).
//!
//! Tool execution policy and LLM/tool permissions are owned by `gestura-core`.

// ============================================================================
// macOS System Permission Checking
// ============================================================================

/// System permission status for macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemPermissionStatus {
    /// Permission has been granted.
    Granted,
    /// Permission has been denied.
    Denied,
    /// Permission has not been determined yet.
    NotDetermined,
    /// Permission is restricted (e.g., by parental controls).
    Restricted,
    /// Permission status is unknown.
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

/// Check microphone permission on macOS using AVCaptureDevice.
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
pub fn request_screen_recording_permission() -> bool {
    // SAFETY: FFI call.
    unsafe { CGRequestScreenCaptureAccess() }
}

/// Check if running macOS 13 (Ventura) or later, which uses "System Settings"
/// instead of "System Preferences".
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
// Non-macOS fallbacks
// ============================================================================

/// Check microphone permission on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn check_microphone_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

/// Check accessibility permission on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn check_accessibility_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

/// Check bluetooth permission on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn check_bluetooth_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

/// Check screen recording permission on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn check_screen_recording_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

/// Request microphone permission on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn request_microphone_permission() -> bool {
    true
}

/// Request bluetooth permission on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn request_bluetooth_permission() -> bool {
    true
}

/// Request screen recording permission on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn request_screen_recording_permission() -> bool {
    true
}

/// Open system preferences/settings on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn open_system_preferences(_pane: &str) -> bool {
    false
}
