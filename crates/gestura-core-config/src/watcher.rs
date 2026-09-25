//! Configuration file watcher for runtime configuration reloading
//!
//! Provides file watching capabilities for hot-reload of non-critical settings.

use crate::types::AppConfig;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

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

/// The watched file: the path to read, plus every spelling of it that a
/// backend may report. FSEvents on macOS delivers resolved paths
/// (`/private/var/…` for a `/var/…` symlink), inotify and Windows deliver
/// the path as it was watched, so an event is ours if it matches either.
struct WatchedFile {
    path: PathBuf,
    aliases: Vec<PathBuf>,
}

impl WatchedFile {
    fn new(path: PathBuf) -> Self {
        let mut aliases = vec![path.clone()];
        let canonical = path.canonicalize().ok().or_else(|| {
            // The file may not exist yet (watching for its creation).
            let parent = path.parent()?.canonicalize().ok()?;
            Some(parent.join(path.file_name()?))
        });
        if let Some(canonical) = canonical
            && !aliases.contains(&canonical)
        {
            aliases.push(canonical);
        }
        Self { path, aliases }
    }

    fn matches(&self, event: &Event) -> bool {
        event
            .paths
            .iter()
            .any(|p| self.aliases.iter().any(|alias| alias == p))
    }
}

/// How long a burst of change events must be quiet before the file is
/// re-read. Long enough to cover an editor's write-rename-chmod sequence.
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(100);

impl ConfigWatcher {
    fn lock_debounce(debounce: &Mutex<DebounceState>) -> std::sync::MutexGuard<'_, DebounceState> {
        // A poisoned lock only means another reload thread panicked; the
        // timestamp inside is still usable.
        debounce
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

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
        let watched = Arc::new(WatchedFile::new(config_path.clone()));

        let watched_clone = watched.clone();
        let debounce_clone = debounce.clone();
        let tx_clone = tx.clone();

        // `notify` invokes this callback on its own thread — on macOS from an
        // `extern "C"` FSEvents callback, where a panic cannot unwind and
        // aborts the whole process. There is no Tokio runtime on that thread,
        // so the work is handed to a plain thread and uses only blocking
        // primitives. Events for other files in the watched directory are
        // filtered out here, before any thread is spawned.
        let watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = &res
                && !watched_clone.matches(event)
            {
                return;
            }
            let tx = tx_clone.clone();
            let watched = watched_clone.clone();
            let debounce = debounce_clone.clone();
            std::thread::Builder::new()
                .name("gestura-config-reload".into())
                .spawn(move || Self::handle_event(res, &watched, &tx, &debounce))
                .map(drop)
                .unwrap_or_else(|e| tracing::error!("config reload thread failed to start: {e}"));
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

    /// Handles one watcher event. Runs on a plain thread (never inside the
    /// Tokio runtime), so it only uses blocking primitives; `blocking_send`
    /// is safe here for the same reason.
    fn handle_event(
        res: Result<Event, notify::Error>,
        watched: &WatchedFile,
        tx: &mpsc::Sender<ConfigChangeEvent>,
        debounce: &Arc<Mutex<DebounceState>>,
    ) {
        let config_path = watched.path.as_path();
        match res {
            Ok(event) => {
                if !watched.matches(&event) {
                    return;
                }
                match event.kind {
                    EventKind::Create(_) | EventKind::Modify(_) => {
                        // Trailing-edge debounce: an editor's save is several
                        // events in quick succession, and the file may still
                        // be half-written at the first one. Each event stamps
                        // itself, waits out the window, and only the newest
                        // stamp reloads — so the content that is read is the
                        // final one.
                        let stamp = Instant::now();
                        Self::lock_debounce(debounce).last_event = Some(stamp);
                        std::thread::sleep(DEBOUNCE_WINDOW);
                        if Self::lock_debounce(debounce).last_event != Some(stamp) {
                            return;
                        }
                        Self::reload_and_emit(config_path, tx);
                    }
                    EventKind::Remove(_) => {
                        tracing::warn!("Config file was deleted: {:?}", config_path);
                        let _ = tx.blocking_send(ConfigChangeEvent::Deleted);
                    }
                    _ => {}
                }
            }
            Err(e) => {
                tracing::error!("File watch error: {}", e);
                let _ = tx.blocking_send(ConfigChangeEvent::Error(format!("Watch error: {}", e)));
            }
        }
    }

    /// Parses the file in the same format `AppConfig::load_from_path` uses
    /// (JSON for a `.json` extension, YAML otherwise — the default config is
    /// `config.yaml`), but reports parse failures instead of defaulting.
    fn parse_config(config_path: &Path, content: &str) -> Result<AppConfig, String> {
        let is_json = config_path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
        let mut config: AppConfig = if is_json {
            serde_json::from_str(content).map_err(|e| e.to_string())?
        } else {
            serde_yaml::from_str(content).map_err(|e| e.to_string())?
        };
        config.normalize_derived_defaults();
        Ok(config.apply_env_overrides())
    }

    fn reload_and_emit(config_path: &Path, tx: &mpsc::Sender<ConfigChangeEvent>) {
        match std::fs::read_to_string(config_path) {
            Ok(content) => match Self::parse_config(config_path, &content) {
                Ok(config) => {
                    tracing::info!("Configuration reloaded successfully");
                    let _ = tx.blocking_send(ConfigChangeEvent::Updated(Box::new(config)));
                }
                Err(e) => {
                    tracing::error!("Failed to parse config file: {}", e);
                    let _ =
                        tx.blocking_send(ConfigChangeEvent::Error(format!("Parse error: {}", e)));
                }
            },
            Err(e) => {
                tracing::error!("Failed to read config file: {}", e);
                let _ = tx.blocking_send(ConfigChangeEvent::Error(format!("Read error: {}", e)));
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

    #[cfg(unix)]
    #[test]
    fn watched_file_accepts_the_resolved_spelling_of_a_symlinked_path() {
        let temp_dir = TempDir::new().unwrap();
        let real_dir = temp_dir.path().join("real");
        std::fs::create_dir(&real_dir).unwrap();
        let link_dir = temp_dir.path().join("link");
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

        // Existing file, watched through the symlink.
        let via_link = link_dir.join("config.yaml");
        std::fs::write(&via_link, "{}").unwrap();
        let watched = WatchedFile::new(via_link.clone());
        let resolved = real_dir.canonicalize().unwrap().join("config.yaml");
        assert!(watched.matches(&modify_event(&via_link).unwrap()));
        assert!(watched.matches(&modify_event(&resolved).unwrap()));
        assert!(!watched.matches(&modify_event(&link_dir.join("other.yaml")).unwrap()));

        // Not yet created: the parent still resolves.
        let future = WatchedFile::new(link_dir.join("later.yaml"));
        assert!(
            future.matches(
                &modify_event(&real_dir.canonicalize().unwrap().join("later.yaml")).unwrap()
            )
        );
    }

    fn modify_event(path: &Path) -> Result<Event, notify::Error> {
        use notify::event::{DataChange, ModifyKind};
        Ok(
            Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
                .add_path(path.to_path_buf()),
        )
    }

    /// The watcher callback runs on notify's thread, outside any Tokio
    /// runtime (an `extern "C"` FSEvents callback on macOS, where a panic
    /// aborts the process). Drive the handler from a plain thread and make
    /// sure the event still reaches an async receiver.
    #[tokio::test]
    async fn handle_event_works_outside_the_runtime() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");
        let mut config = AppConfig::default();
        config.ui.theme_mode = "dark".to_string();
        std::fs::write(&config_path, serde_yaml::to_string(&config).unwrap()).unwrap();

        let (tx, mut rx) = mpsc::channel(4);
        let debounce = Arc::new(Mutex::new(DebounceState { last_event: None }));
        let watched = Arc::new(WatchedFile::new(config_path.clone()));

        // Two events inside one debounce window, from two threads (as notify
        // would deliver a save burst): only the newer one may reload.
        let spawn_event = |event_path: PathBuf,
                           watched: Arc<WatchedFile>,
                           tx: mpsc::Sender<ConfigChangeEvent>,
                           debounce: Arc<Mutex<DebounceState>>| {
            std::thread::spawn(move || {
                ConfigWatcher::handle_event(modify_event(&event_path), &watched, &tx, &debounce)
            })
        };
        let first = spawn_event(
            config_path.clone(),
            watched.clone(),
            tx.clone(),
            debounce.clone(),
        );
        std::thread::sleep(Duration::from_millis(10));
        // The newer event arrives under the backend's resolved spelling of
        // the same file (what FSEvents reports on macOS).
        let second = spawn_event(
            config_path.canonicalize().unwrap(),
            watched.clone(),
            tx.clone(),
            debounce.clone(),
        );
        // A different file in the same directory: ignored outright.
        let foreign = spawn_event(
            config_path.with_file_name("other.yaml"),
            watched.clone(),
            tx.clone(),
            debounce.clone(),
        );
        for handle in [first, second, foreign] {
            handle.join().unwrap();
        }
        drop(tx);

        match rx.recv().await {
            Some(ConfigChangeEvent::Updated(reloaded)) => {
                assert_eq!(reloaded.ui.theme_mode, "dark")
            }
            other => panic!("expected Updated, got {other:?}"),
        }
        assert!(
            rx.recv().await.is_none(),
            "the superseded and the foreign event must not be delivered"
        );
    }

    #[tokio::test]
    async fn reload_reports_parse_errors_instead_of_defaulting() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");
        std::fs::write(&config_path, "{ not json").unwrap();

        let (tx, mut rx) = mpsc::channel(4);
        let path = config_path.clone();
        std::thread::spawn(move || ConfigWatcher::reload_and_emit(&path, &tx))
            .join()
            .unwrap();

        match rx.recv().await {
            Some(ConfigChangeEvent::Error(msg)) => assert!(msg.starts_with("Parse error"), "{msg}"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    /// End to end through the real watcher: a write to the watched file must
    /// come back as `Updated` without touching the runtime from notify's
    /// thread. (With the old `tokio::spawn` in the callback this aborted the
    /// whole test binary on macOS.)
    #[tokio::test]
    async fn watcher_delivers_updated_after_a_write() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");
        let config = AppConfig::default();
        std::fs::write(&config_path, serde_yaml::to_string(&config).unwrap()).unwrap();

        let (_watcher, mut rx) = ConfigWatcher::with_path(config_path.clone()).unwrap();
        // Give the backend a moment to arm before the write it must observe.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let mut changed = config.clone();
        changed.ui.theme_mode = "dark".to_string();
        std::fs::write(&config_path, serde_yaml::to_string(&changed).unwrap()).unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let event = tokio::time::timeout_at(deadline, rx.recv())
                .await
                .expect("no config event within 10s")
                .expect("watcher channel closed");
            match event {
                // The very first event may be the initial write on backends
                // that report recent history; keep reading until the change.
                ConfigChangeEvent::Updated(cfg) if cfg.ui.theme_mode == "dark" => break,
                ConfigChangeEvent::Updated(_) => continue,
                other => panic!("unexpected event {other:?}"),
            }
        }
    }
}
