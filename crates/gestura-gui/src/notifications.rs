//! Response completion and MCP feedback notifications
//!
//! Provides sound and haptic notifications when:
//! - LLM response completes
//! - MCP tool requests feedback from user

use crate::haptics::{HapticPattern, HapticRequest};
use gestura_core::config::{AppConfig, AppConfigSecurityExt, NotificationSettings};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager};

/// Global notification manager instance
static NOTIFICATION_MANAGER: OnceLock<NotificationManager> = OnceLock::new();

/// Get the global notification manager
pub fn get_notification_manager() -> &'static NotificationManager {
    NOTIFICATION_MANAGER.get_or_init(NotificationManager::new)
}

/// Notification types for different events
#[derive(Debug, Clone, Copy)]
pub enum NotificationType {
    /// LLM response completed successfully
    ResponseComplete,
    /// MCP tool is requesting user feedback
    McpFeedbackRequest,
    /// Error occurred during processing
    Error,
    /// Voice recording started
    ListeningStarted,
    /// Voice recording stopped
    ListeningStopped,
}

/// Manages sound and haptic notifications
pub struct NotificationManager {
    /// Connected ring device ID (if any)
    connected_ring: std::sync::RwLock<Option<String>>,
}

impl NotificationManager {
    /// Create a new notification manager
    pub fn new() -> Self {
        Self {
            connected_ring: std::sync::RwLock::new(None),
        }
    }

    /// Set the connected ring device ID
    pub fn set_connected_ring(&self, device_id: Option<String>) {
        if let Ok(mut ring) = self.connected_ring.write() {
            *ring = device_id;
        }
    }

    /// Get the connected ring device ID
    pub fn get_connected_ring(&self) -> Option<String> {
        self.connected_ring.read().ok().and_then(|r| r.clone())
    }

    /// Send a notification based on the event type
    pub async fn notify(&self, notification_type: NotificationType, app: Option<&AppHandle>) {
        let config = AppConfig::load_async().await;
        let settings = &config.notifications;

        // Send sound notification
        if settings.sound_enabled {
            self.play_sound(notification_type, settings).await;
        }

        // Send haptic notification
        if settings.haptic_enabled {
            self.send_haptic(notification_type, settings, app).await;
        }

        // Handle MCP feedback special behavior
        if matches!(notification_type, NotificationType::McpFeedbackRequest)
            && settings.mcp_feedback_enabled
            && settings.auto_listen_on_feedback
            && let Some(app) = app
        {
            // Emit event to start listening
            let _ = app.emit("start-listening", ());
        }
    }

    /// Preview a user-selected sound (used by the settings UI).
    pub async fn preview_sound(&self, sound_choice: &str, volume: Option<u8>) {
        let volume = volume.unwrap_or(70).min(100) as f32 / 100.0;
        self.play_custom_sound(sound_choice, volume).await;
    }

    /// Play a sound for the notification type
    async fn play_sound(
        &self,
        notification_type: NotificationType,
        settings: &NotificationSettings,
    ) {
        let volume = settings.sound_volume as f32 / 100.0;

        // Allow users to select the general completion sound.
        if matches!(notification_type, NotificationType::ResponseComplete)
            && settings.notification_sound == "none"
        {
            return;
        }

        let sound_label = match notification_type {
            NotificationType::ResponseComplete => settings.notification_sound.as_str(),
            NotificationType::McpFeedbackRequest => "mcp_feedback",
            NotificationType::Error => "error",
            NotificationType::ListeningStarted => "listening_start",
            NotificationType::ListeningStopped => "listening_stop",
        };

        tracing::debug!(
            "Playing notification sound: {} at volume {}",
            sound_label,
            volume
        );

        // On response completion, use the user-selected sound.
        if matches!(notification_type, NotificationType::ResponseComplete) {
            self.play_custom_sound(&settings.notification_sound, volume)
                .await;
            return;
        }

        // Otherwise play the built-in per-event sound.
        self.play_system_sound(notification_type, volume).await;
    }

    async fn play_custom_sound(&self, sound_choice: &str, volume: f32) {
        // "none" means: no sound.
        if sound_choice == "none" {
            return;
        }

        // macOS: map choice -> system sound file.
        #[cfg(target_os = "macos")]
        {
            if let Some(sound_file) = macos_sound_file_for_choice(sound_choice) {
                spawn_macos_afplay(sound_file, volume);
            }
        }

        // Other platforms: currently a no-op (we still log for visibility).
        #[cfg(not(target_os = "macos"))]
        {
            tracing::debug!(
                "Sound preview requested (unsupported platform): choice={}, volume={}",
                sound_choice,
                volume
            );
        }
    }

    async fn play_system_sound(&self, notification_type: NotificationType, volume: f32) {
        #[cfg(target_os = "macos")]
        {
            let sound_file = match notification_type {
                NotificationType::ResponseComplete => "/System/Library/Sounds/Glass.aiff",
                NotificationType::McpFeedbackRequest => "/System/Library/Sounds/Ping.aiff",
                NotificationType::Error => "/System/Library/Sounds/Basso.aiff",
                NotificationType::ListeningStarted => "/System/Library/Sounds/Pop.aiff",
                NotificationType::ListeningStopped => "/System/Library/Sounds/Tink.aiff",
            };
            spawn_macos_afplay(sound_file, volume);
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (notification_type, volume);
        }
    }

    /// Send haptic feedback to the connected ring
    async fn send_haptic(
        &self,
        notification_type: NotificationType,
        settings: &NotificationSettings,
        app: Option<&AppHandle>,
    ) {
        let device_id = match self.get_connected_ring() {
            Some(id) => id,
            None => {
                tracing::debug!("No ring connected, skipping haptic notification");
                return;
            }
        };

        let intensity = settings.haptic_intensity as f32 / 100.0;

        let request = match notification_type {
            NotificationType::ResponseComplete => HapticRequest {
                pattern: HapticPattern::Notification,
                intensity,
                duration_ms: 100,
                repeat_count: 1,
                repeat_delay_ms: 50,
            },
            NotificationType::McpFeedbackRequest => HapticRequest {
                pattern: HapticPattern::Alert,
                intensity,
                duration_ms: 150,
                repeat_count: 3,
                repeat_delay_ms: 100,
            },
            NotificationType::Error => HapticRequest {
                pattern: HapticPattern::Pulse,
                intensity: intensity.min(1.0),
                duration_ms: 100,
                repeat_count: 2,
                repeat_delay_ms: 50,
            },
            NotificationType::ListeningStarted => HapticRequest {
                pattern: HapticPattern::Click,
                intensity: intensity * 0.8,
                duration_ms: 50,
                repeat_count: 0,
                repeat_delay_ms: 0,
            },
            NotificationType::ListeningStopped => HapticRequest {
                pattern: HapticPattern::Click,
                intensity: intensity * 0.6,
                duration_ms: 50,
                repeat_count: 0,
                repeat_delay_ms: 0,
            },
        };

        let Some(app_handle) = app else {
            tracing::debug!("No app handle available, skipping haptic notification");
            return;
        };

        let ring_manager = app_handle.state::<crate::AppState>().ring_manager.clone();
        if let Err(e) = ring_manager.send_haptic(&device_id, request).await {
            tracing::warn!("Failed to send haptic notification: {}", e);
        }
    }
}

#[cfg(target_os = "macos")]
fn spawn_macos_afplay(sound_file: &'static str, volume: f32) {
    use std::process::Command;
    let volume = volume.clamp(0.0, 1.0);
    tokio::spawn(async move {
        let _ = Command::new("afplay")
            .args(["-v", &volume.to_string(), sound_file])
            .spawn();
    });
}

#[cfg(target_os = "macos")]
fn macos_sound_file_for_choice(choice: &str) -> Option<&'static str> {
    // Keep this mapping aligned with the config UI option values.
    match choice {
        // Notification sound options
        "default" | "chime" => Some("/System/Library/Sounds/Glass.aiff"),
        "ping" | "beep" => Some("/System/Library/Sounds/Ping.aiff"),
        "pop" => Some("/System/Library/Sounds/Pop.aiff"),
        "subtle" | "click" => Some("/System/Library/Sounds/Tink.aiff"),
        "success" => Some("/System/Library/Sounds/Hero.aiff"),
        "none" => None,
        _ => Some("/System/Library/Sounds/Glass.aiff"),
    }
}

impl Default for NotificationManager {
    fn default() -> Self {
        Self::new()
    }
}
