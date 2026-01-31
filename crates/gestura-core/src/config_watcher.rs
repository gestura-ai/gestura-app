//! Configuration file watcher for runtime configuration reloading
//!
//! Provides file watching capabilities for hot-reload of non-critical settings.

use crate::config::AppConfig;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, mpsc};

/// Events emitted when configuration changes
#[derive(Debug, Clone)]
pub enum ConfigChangeEvent {
    /// Configuration was successfully updated
    Updated(Box<AppConfig>),
    /// Error occurred while watching or loading configuration
    Error(String),
    /// Configuration file was deleted
    Deleted,
}

/// Configuration file watcher
pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
    config_path: PathBuf,
}

struct DebounceState {
    last_event: Option<Instant>,
}

impl ConfigWatcher {
    /// Create a new configuration watcher
    pub fn new() -> Result<(Self, mpsc::Receiver<ConfigChangeEvent>), String> {
        Self::with_path(AppConfig::default_path())
    }

    /// Create a configuration watcher for a specific path
    pub fn with_path(
        config_path: PathBuf,
    ) -> Result<(Self, mpsc::Receiver<ConfigChangeEvent>), String> {
        let (tx, rx) = mpsc::channel(32);
        let debounce = Arc::new(Mutex::new(DebounceState { last_event: None }));

        let config_path_clone = config_path.clone();
        let debounce_clone = debounce.clone();
        let tx_clone = tx.clone();

        let watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            let tx = tx_clone.clone();
            let config_path = config_path_clone.clone();
            let debounce = debounce_clone.clone();
            tokio::spawn(async move {
                Self::handle_event(res, &config_path, &tx, &debounce).await;
            });
        })
        .map_err(|e| format!("Failed to create file watcher: {}", e))?;

        let watch_path = config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| config_path.clone());

        let mut watcher = watcher;
        watcher
            .watch(&watch_path, RecursiveMode::NonRecursive)
            .map_err(|e| format!("Failed to watch config directory: {}", e))?;

        tracing::info!("Started watching config file: {:?}", config_path);
        Ok((
            Self {
                _watcher: watcher,
                config_path,
            },
            rx,
        ))
    }

    async fn handle_event(
        res: Result<Event, notify::Error>,
        config_path: &PathBuf,
        tx: &mpsc::Sender<ConfigChangeEvent>,
        debounce: &Arc<Mutex<DebounceState>>,
    ) {
        match res {
            Ok(event) => {
                if !event.paths.iter().any(|p| p == config_path) {
                    return;
                }
                match event.kind {
                    EventKind::Create(_) | EventKind::Modify(_) => {
                        let should_reload = {
                            let mut state = debounce.lock().await;
                            let now = Instant::now();
                            if let Some(last) = state.last_event
                                && now.duration_since(last) < Duration::from_millis(100)
                            {
                                return;
                            }
                            state.last_event = Some(now);
                            true
                        };
                        if should_reload {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            Self::reload_and_emit(config_path, tx).await;
                        }
                    }
                    EventKind::Remove(_) => {
                        tracing::warn!("Config file was deleted: {:?}", config_path);
                        let _ = tx.send(ConfigChangeEvent::Deleted).await;
                    }
                    _ => {}
                }
            }
            Err(e) => {
                tracing::error!("File watch error: {}", e);
                let _ = tx
                    .send(ConfigChangeEvent::Error(format!("Watch error: {}", e)))
                    .await;
            }
        }
    }

    async fn reload_and_emit(config_path: &PathBuf, tx: &mpsc::Sender<ConfigChangeEvent>) {
        match std::fs::read_to_string(config_path) {
            Ok(content) => match serde_json::from_str::<AppConfig>(&content) {
                Ok(config) => {
                    let config = config.apply_env_overrides();
                    tracing::info!("Configuration reloaded successfully");
                    let _ = tx.send(ConfigChangeEvent::Updated(Box::new(config))).await;
                }
                Err(e) => {
                    tracing::error!("Failed to parse config file: {}", e);
                    let _ = tx
                        .send(ConfigChangeEvent::Error(format!("Parse error: {}", e)))
                        .await;
                }
            },
            Err(e) => {
                tracing::error!("Failed to read config file: {}", e);
                let _ = tx
                    .send(ConfigChangeEvent::Error(format!("Read error: {}", e)))
                    .await;
            }
        }
    }

    /// Get the path being watched
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }
}

/// Settings that can be hot-reloaded without restart
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotReloadableSettings {
    pub theme_mode: String,
    pub accent: Option<String>,
    pub sound_enabled: bool,
    pub haptic_enabled: bool,
    pub sound_volume: u8,
    pub haptic_intensity: u8,
    pub developer_mode: bool,
    pub verbose_ble_logging: bool,
}

impl From<&AppConfig> for HotReloadableSettings {
    fn from(config: &AppConfig) -> Self {
        Self {
            theme_mode: config.ui.theme_mode.clone(),
            accent: config.ui.accent.clone(),
            sound_enabled: config.notifications.sound_enabled,
            haptic_enabled: config.notifications.haptic_enabled,
            sound_volume: config.notifications.sound_volume,
            haptic_intensity: config.notifications.haptic_intensity,
            developer_mode: config.developer.developer_mode,
            verbose_ble_logging: config.developer.verbose_ble_logging,
        }
    }
}

impl HotReloadableSettings {
    /// Check if settings differ from another config
    pub fn differs_from(&self, other: &AppConfig) -> bool {
        *self != HotReloadableSettings::from(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_hot_reloadable_settings_from_config() {
        let config = AppConfig::default();
        let settings = HotReloadableSettings::from(&config);
        assert_eq!(settings.theme_mode, config.ui.theme_mode);
        assert_eq!(settings.sound_enabled, config.notifications.sound_enabled);
    }

    #[test]
    fn test_hot_reloadable_settings_differs() {
        let config = AppConfig::default();
        let settings = HotReloadableSettings::from(&config);
        assert!(!settings.differs_from(&config));

        let mut modified = config.clone();
        modified.ui.theme_mode = "dark".to_string();
        assert!(settings.differs_from(&modified));
    }

    #[tokio::test]
    async fn test_config_watcher_creation() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");
        let config = AppConfig::default();
        std::fs::write(&config_path, serde_yaml::to_string(&config).unwrap()).unwrap();

        let result = ConfigWatcher::with_path(config_path.clone());
        assert!(result.is_ok());
        let (watcher, _rx) = result.unwrap();
        assert_eq!(watcher.config_path(), &config_path);
    }

    #[test]
    fn test_config_change_event_variants() {
        let config = AppConfig::default();
        let _updated = ConfigChangeEvent::Updated(Box::new(config));
        let _error = ConfigChangeEvent::Error("test error".to_string());
        let _deleted = ConfigChangeEvent::Deleted;
    }
}
