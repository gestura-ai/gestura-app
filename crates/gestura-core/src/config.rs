//! Configuration management for Gestura
//!
//! Type definitions and pure helpers live in [`gestura_core_config`].
//! This module re-exports them and adds security-dependent methods
//! via the [`AppConfigSecurityExt`] extension trait.
//!
//! ## Backward compatibility
//!
//! Older versions stored configuration as JSON in `~/.gestura/config.json`.
//! On load, if `config.yaml` does not exist but `config.json` does, we
//! automatically migrate the JSON file to YAML.
//!
//! ## Configuration Precedence
//!
//! Configuration values are loaded with the following precedence (highest first):
//! 1. Environment variables (GESTURA_* prefix)
//! 2. Config file (`~/.gestura/config.yaml`)
//! 3. Default values
//!
//! See [`crate::config_env`] for environment variable documentation.

// Re-export everything from the config domain crate so that
// `gestura_core::config::AppConfig` (and all sibling types) keep working.
pub use gestura_core_config::*;

use std::fs;
use std::path::Path;

// These shadow the glob re-export of the same types from `gestura_core_config::*`
// (which in turn re-exports from `gestura_core_foundation`). We import from
// `crate::error` for consistency with other core modules.
#[allow(hidden_glob_reexports)]
use crate::error::{AppError, Result};

#[cfg(feature = "security")]
use crate::security::{create_secure_storage, keychain_access_disabled};

#[cfg(all(feature = "security", not(test)))]
const KEYCHAIN_SERVICE: &str = "gestura";

#[cfg(all(feature = "security", test))]
fn test_keychain_store() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(all(feature = "security", test))]
fn clear_test_keychain_store() {
    if let Ok(mut m) = test_keychain_store().lock() {
        m.clear();
    }
}

/// Helper to get a key from the keychain (sync wrapper for direct keyring usage)
#[cfg(all(feature = "security", test))]
fn get_keychain_secret(key: &str) -> Option<String> {
    test_keychain_store()
        .lock()
        .ok()
        .and_then(|m| m.get(key).cloned())
}

#[cfg(all(feature = "security", not(test)))]
fn get_keychain_secret(key: &str) -> Option<String> {
    use keyring::Entry;
    let entry = match Entry::new(KEYCHAIN_SERVICE, key) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(
                key,
                error = %e,
                "Failed to create keyring entry — keychain may be inaccessible"
            );
            return None;
        }
    };
    match entry.get_password() {
        Ok(pw) => Some(pw),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            tracing::warn!(
                key,
                error = %e,
                "Failed to read secret from keychain — \
                 if another app stored this key you may need to grant access \
                 via the macOS Keychain Access prompt"
            );
            None
        }
    }
}

/// Helper to set a key in the keychain (sync wrapper for direct keyring usage)
#[cfg(all(feature = "security", test))]
fn set_keychain_secret(key: &str, value: &str) -> Result<()> {
    test_keychain_store()
        .lock()
        .map_err(|_| AppError::Internal("Test keychain poisoned".to_string()))?
        .insert(key.to_string(), value.to_string());
    Ok(())
}

/// Read a secret from secure storage using canonical key names, with legacy-key fallback.
///
/// If the secret is found under the legacy key, this will *best-effort* re-store
/// it under the canonical key (self-heal migration) so future reads converge.
#[cfg(feature = "security")]
fn get_keychain_secret_with_legacy_fallback(
    canonical_key: &str,
    legacy_key: &str,
) -> Option<String> {
    if let Some(v) = get_keychain_secret(canonical_key) {
        return (!v.is_empty()).then_some(v);
    }

    let legacy = get_keychain_secret(legacy_key)?;
    if legacy.is_empty() {
        return None;
    }

    if let Err(e) = set_keychain_secret(canonical_key, &legacy) {
        tracing::warn!(
            canonical_key,
            legacy_key,
            error = %e,
            "Failed to self-heal secret from legacy key to canonical key"
        );
    }

    Some(legacy)
}

/// Returns true if secure storage contains a non-empty secret under either the
/// canonical or legacy key.
///
/// Note: this uses the same legacy-fallback + self-heal behavior as reads.
#[cfg(feature = "security")]
fn keychain_has_secret(canonical_key: &str, legacy_key: &str) -> bool {
    get_keychain_secret_with_legacy_fallback(canonical_key, legacy_key).is_some()
}

#[cfg(all(feature = "security", not(test)))]
fn set_keychain_secret(key: &str, value: &str) -> Result<()> {
    use keyring::Entry;
    Entry::new(KEYCHAIN_SERVICE, key)
        .map_err(|e| AppError::Internal(format!("Keyring error: {}", e)))?
        .set_password(value)
        .map_err(|e| AppError::Internal(format!("Keyring error: {}", e)))?;
    Ok(())
}
// ---------------------------------------------------------------------------
// Extension trait: security-dependent methods for AppConfig
// ---------------------------------------------------------------------------

/// Extension trait adding security-dependent methods to [`AppConfig`].
///
/// Import this trait (or `use crate::config::*`) to access `load()`, `save()`,
/// keychain hydration, and secret migration methods on [`AppConfig`].
pub trait AppConfigSecurityExt: Sized {
    /// Load configuration from disk, falling back to defaults if missing (sync).
    fn load() -> Self;
    /// Load configuration from disk asynchronously.
    fn load_async() -> impl std::future::Future<Output = Self> + Send;
    /// Save configuration to disk at an explicit path.
    fn save_to_path(&self, path: impl AsRef<Path>) -> Result<()>;
    /// Save configuration to disk (sync).
    fn save(&self) -> Result<()>;
    /// Save configuration to disk asynchronously.
    fn save_async(&self) -> impl std::future::Future<Output = Result<()>> + Send;
    /// Save configuration to disk at an explicit path (async).
    fn save_to_path_async(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
    /// Load configuration with environment variable overrides applied.
    fn load_with_env() -> Self;
    /// Load configuration asynchronously with environment variable overrides.
    fn load_with_env_async() -> impl std::future::Future<Output = Self> + Send;
    /// Clear secrets from the struct (used before saving to disk).
    #[cfg(feature = "security")]
    fn sanitize_secrets(&mut self);
    /// Check which API key providers have secrets stored in the OS keychain.
    fn api_key_keychain_status() -> Vec<(&'static str, bool)>;
    /// Returns true if the config struct currently contains any plaintext secrets.
    #[cfg(feature = "security")]
    fn has_plaintext_secrets(&self) -> bool;
    /// Load secrets from keystore into the struct (sync).
    #[cfg(feature = "security")]
    fn hydrate_secrets_sync(&mut self) -> Result<()>;
    /// Async version of hydrate secrets.
    #[cfg(feature = "security")]
    fn hydrate_secrets(&mut self) -> impl std::future::Future<Output = Result<()>> + Send;
    /// Move secrets from the struct (if present) to the keychain (sync).
    #[cfg(feature = "security")]
    fn migrate_secrets_sync(&self) -> Result<bool>;
    /// Async version of migrate secrets.
    #[cfg(feature = "security")]
    fn migrate_secrets(&self) -> impl std::future::Future<Output = Result<bool>> + Send;
}

impl AppConfigSecurityExt for AppConfig {
    /// Load configuration from disk, falling back to defaults if missing (sync version).
    ///
    /// If `~/.gestura/config.yaml` is missing but `~/.gestura/config.json` exists,
    /// this will automatically migrate the JSON file to YAML.
    ///
    /// This method also handles migration of secrets to the secure keystore.
    fn load() -> Self {
        let yaml_path = Self::default_path();
        let json_path = Self::legacy_json_path();
        let backup_path = Self::legacy_json_backup_path();
        // Capture the initial state so we can decide whether to perform JSON->YAML
        // migration even if later steps (e.g., secret hydration) create the YAML.
        let needs_format_migration = !yaml_path.exists() && json_path.exists();
        #[allow(unused_mut)] // config is mutated by hydrate_secrets/migrate_secrets
        let mut config = if yaml_path.exists() {
            Self::load_from_path(&yaml_path)
        } else {
            // Check for legacy JSON
            if json_path.exists() {
                if let Ok(s) = fs::read_to_string(&json_path) {
                    // We found JSON but no YAML. We will return this config.
                    // Persisting (format migration) happens below.
                    serde_json::from_str::<Self>(&s).unwrap_or_default()
                } else {
                    Self::default()
                }
            } else {
                Self::default()
            }
        };

        // Hydrate secrets from keychain (if empty in file)
        #[cfg(feature = "security")]
        {
            if keychain_access_disabled() {
                // In non-interactive contexts (CI/integration tests), keychain access can block.
                // When explicitly disabled, skip hydration/migration to avoid hangs and avoid
                // accidentally sanitizing secrets without a secure keystore destination.
                tracing::info!(
                    "Keychain access disabled; skipping secret hydration/migration on config load"
                );
            } else {
                // Detect plaintext secrets in the loaded config *before* hydration.
                // If they exist, we must sanitize persisted YAML even when keychain already has keys.
                let had_plaintext_secrets = config.has_plaintext_secrets();

                // Keychain-first: if secure storage has secrets, they override YAML values.
                let _ = config.hydrate_secrets_sync();

                // Migrate YAML secrets into secure storage only when secure storage is empty.
                let migrated = config.migrate_secrets_sync().unwrap_or(false);

                // If we either migrated secrets OR we detected plaintext secrets in the file,
                // persist a sanitized config back to disk.
                if had_plaintext_secrets || migrated {
                    let _ = config.save();
                }
            }
        }

        // JSON -> YAML format migration should occur regardless of whether the `security` feature
        // is enabled. In security builds, `save()` already refuses to persist plaintext secrets
        // when keychain access is disabled, which prevents accidental data loss.
        if needs_format_migration {
            let _ = config.save();
            // Only move the legacy JSON aside if we successfully created YAML.
            if yaml_path.exists() && json_path.exists() && !backup_path.exists() {
                let _ = fs::rename(&json_path, &backup_path);
            }
        }

        config
    }

    /// Load configuration from disk asynchronously, falling back to defaults if missing.
    ///
    /// This is the preferred method for GUI/Tauri commands to avoid blocking the UI thread.
    async fn load_async() -> Self {
        let yaml_path = Self::default_path();
        let json_path = Self::legacy_json_path();
        let backup_path = Self::legacy_json_backup_path();
        let had_yaml = tokio::fs::try_exists(&yaml_path).await.unwrap_or(false);
        let had_json = tokio::fs::try_exists(&json_path).await.unwrap_or(false);
        let needs_format_migration = !had_yaml && had_json;
        #[allow(unused_mut)] // config is mutated by hydrate_secrets/migrate_secrets
        let mut config = if had_yaml {
            Self::load_from_path_async(&yaml_path).await
        } else if had_json {
            if let Ok(s) = tokio::fs::read_to_string(&json_path).await {
                serde_json::from_str::<Self>(&s).unwrap_or_default()
            } else {
                Self::default()
            }
        } else {
            Self::default()
        };

        // Async hydration/migration
        #[cfg(feature = "security")]
        {
            if keychain_access_disabled() {
                tracing::info!(
                    "Keychain access disabled; skipping secret hydration/migration on async config load"
                );
            } else {
                let had_plaintext_secrets = config.has_plaintext_secrets();

                // Keychain-first precedence.
                let _ = config.hydrate_secrets().await;

                // Migrate YAML secrets into secure storage only if secure storage is empty.
                let migrated = config.migrate_secrets().await.unwrap_or(false);

                if had_plaintext_secrets || migrated {
                    let _ = config.save_async().await;
                }
            }
        }

        // JSON -> YAML format migration should occur regardless of whether the `security` feature
        // is enabled. In security builds, `save_async()` already refuses to persist plaintext
        // secrets when keychain access is disabled.
        if needs_format_migration {
            let _ = config.save_async().await;

            let yaml_exists = tokio::fs::try_exists(&yaml_path).await.unwrap_or(false);
            let json_exists = tokio::fs::try_exists(&json_path).await.unwrap_or(false);
            let backup_exists = tokio::fs::try_exists(&backup_path).await.unwrap_or(false);

            if yaml_exists && json_exists && !backup_exists {
                let _ = tokio::fs::rename(&json_path, &backup_path).await;
            }
        }

        config
    }

    /// Save configuration to disk at an explicit path.
    ///
    /// This handles stripping secrets before writing to disk if security is enabled.
    fn save_to_path(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        #[cfg(feature = "security")]
        {
            if keychain_access_disabled() && self.has_plaintext_secrets() {
                return Err(AppError::Config(
                    "Cannot persist plaintext secrets while keychain access is disabled. \
Set secrets via environment variables or re-enable keychain access (unset GESTURA_DISABLE_KEYCHAIN)."
                        .to_string(),
                ));
            }

            // First, ensure all current secrets are saved to keystore
            let _ = self.migrate_secrets_sync();

            let mut clean_config = self.clone();
            clean_config.sanitize_secrets();
            let data = serde_yaml::to_string(&clean_config)
                .map_err(|e| AppError::Config(format!("Failed to serialize config: {}", e)))?;
            fs::write(path, data)?;
        }

        #[cfg(not(feature = "security"))]
        {
            let data = serde_yaml::to_string(self)
                .map_err(|e| AppError::Config(format!("Failed to serialize config: {}", e)))?;
            fs::write(path, data)?;
        }

        Ok(())
    }

    /// Save configuration to disk (sync version).
    fn save(&self) -> Result<()> {
        self.save_to_path(Self::default_path())
    }

    /// Save configuration to disk asynchronously.
    async fn save_async(&self) -> Result<()> {
        self.save_to_path_async(Self::default_path()).await
    }

    /// Save configuration to disk at an explicit path (async).
    ///
    /// This is the async equivalent of [`AppConfig::save_to_path`].
    async fn save_to_path_async(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        #[cfg(feature = "security")]
        {
            if keychain_access_disabled() && self.has_plaintext_secrets() {
                return Err(AppError::Config(
                    "Cannot persist plaintext secrets while keychain access is disabled. \
Set secrets via environment variables or re-enable keychain access (unset GESTURA_DISABLE_KEYCHAIN)."
                        .to_string(),
                ));
            }

            // First, ensure all current secrets are saved to keystore
            // We ignore the "changed" boolean here as we just want to ensure consistency
            let _ = self.migrate_secrets().await;

            let mut clean_config = self.clone();
            clean_config.sanitize_secrets();
            let data = serde_yaml::to_string(&clean_config)
                .map_err(|e| AppError::Config(format!("Failed to serialize config: {}", e)))?;
            tokio::fs::write(path, data).await?;
        }

        #[cfg(not(feature = "security"))]
        {
            let data = serde_yaml::to_string(self)
                .map_err(|e| AppError::Config(format!("Failed to serialize config: {}", e)))?;
            tokio::fs::write(path, data).await?;
        }

        Ok(())
    }

    // ========================================================================
    // Security Helpers
    // ========================================================================

    /// Clear secrets from the struct (used before saving to disk)
    #[cfg(feature = "security")]
    fn sanitize_secrets(&mut self) {
        if let Some(c) = &mut self.llm.openai {
            c.api_key.clear();
        }
        if let Some(c) = &mut self.llm.anthropic {
            c.api_key.clear();
        }
        if let Some(c) = &mut self.llm.grok {
            c.api_key.clear();
        }
        if let Some(c) = &mut self.llm.gemini {
            c.api_key.clear();
        }

        self.voice.openai_api_key = None;
        self.web_search.serpapi_key = None;
        self.web_search.brave_key = None;
    }

    /// Check which API key providers have secrets stored in the OS keychain.
    ///
    /// Returns a list of `(provider_label, is_present)` tuples for every known
    /// provider.  This is intended for CLI/TUI display — no secret values are
    /// exposed.
    ///
    /// When the `security` feature is disabled (or keychain access is disabled at
    /// runtime) every provider reports `false`.
    #[cfg(feature = "security")]
    fn api_key_keychain_status() -> Vec<(&'static str, bool)> {
        if keychain_access_disabled() {
            return vec![
                ("openai", false),
                ("anthropic", false),
                ("gemini", false),
                ("grok", false),
                ("voice_openai", false),
                ("serpapi", false),
                ("brave", false),
            ];
        }

        vec![
            (
                "openai",
                keychain_has_secret("gestura_llm_openai_api_key", "gestura_api_key_openai"),
            ),
            (
                "anthropic",
                keychain_has_secret("gestura_llm_anthropic_api_key", "gestura_api_key_anthropic"),
            ),
            (
                "gemini",
                keychain_has_secret("gestura_llm_gemini_api_key", "gestura_api_key_gemini"),
            ),
            (
                "grok",
                keychain_has_secret("gestura_llm_grok_api_key", "gestura_api_key_grok"),
            ),
            (
                "voice_openai",
                keychain_has_secret(
                    "gestura_voice_openai_api_key",
                    "gestura_api_key_voice_openai",
                ),
            ),
            (
                "serpapi",
                keychain_has_secret("gestura_web_search_serpapi_key", "gestura_api_key_serpapi"),
            ),
            (
                "brave",
                keychain_has_secret("gestura_web_search_brave_key", "gestura_api_key_brave"),
            ),
        ]
    }

    /// Convenience: returns `false` for all providers when the `security` feature
    /// is not compiled in.
    #[cfg(not(feature = "security"))]
    fn api_key_keychain_status() -> Vec<(&'static str, bool)> {
        vec![
            ("openai", false),
            ("anthropic", false),
            ("gemini", false),
            ("grok", false),
            ("voice_openai", false),
            ("serpapi", false),
            ("brave", false),
        ]
    }

    /// Returns true if the config struct currently contains any plaintext secrets
    /// that should not be persisted to disk.
    #[cfg(feature = "security")]
    fn has_plaintext_secrets(&self) -> bool {
        self.llm
            .openai
            .as_ref()
            .is_some_and(|c| !c.api_key.is_empty())
            || self
                .llm
                .anthropic
                .as_ref()
                .is_some_and(|c| !c.api_key.is_empty())
            || self
                .llm
                .grok
                .as_ref()
                .is_some_and(|c| !c.api_key.is_empty())
            || self
                .llm
                .gemini
                .as_ref()
                .is_some_and(|c| !c.api_key.is_empty())
            || self
                .voice
                .openai_api_key
                .as_deref()
                .is_some_and(|k| !k.is_empty())
            || self
                .web_search
                .serpapi_key
                .as_deref()
                .is_some_and(|k| !k.is_empty())
            || self
                .web_search
                .brave_key
                .as_deref()
                .is_some_and(|k| !k.is_empty())
    }

    /// Load secrets from keystore into the struct (sync)
    #[cfg(feature = "security")]
    fn hydrate_secrets_sync(&mut self) -> Result<()> {
        if keychain_access_disabled() {
            return Ok(());
        }

        // OpenAI
        if let Some(c) = &mut self.llm.openai
            && let Some(secret) = get_keychain_secret_with_legacy_fallback(
                "gestura_llm_openai_api_key",
                "gestura_api_key_openai",
            )
        {
            // Keychain-first: overwrite YAML value if a key exists in secure storage.
            c.api_key = secret;
        }

        // Anthropic
        if let Some(c) = &mut self.llm.anthropic
            && let Some(secret) = get_keychain_secret_with_legacy_fallback(
                "gestura_llm_anthropic_api_key",
                "gestura_api_key_anthropic",
            )
        {
            c.api_key = secret;
        }

        // Grok
        if let Some(c) = &mut self.llm.grok
            && let Some(secret) = get_keychain_secret_with_legacy_fallback(
                "gestura_llm_grok_api_key",
                "gestura_api_key_grok",
            )
        {
            c.api_key = secret;
        }

        // Gemini
        if let Some(c) = &mut self.llm.gemini
            && let Some(secret) = get_keychain_secret_with_legacy_fallback(
                "gestura_llm_gemini_api_key",
                "gestura_api_key_gemini",
            )
        {
            c.api_key = secret;
        }

        // Voice OpenAI
        if let Some(secret) = get_keychain_secret_with_legacy_fallback(
            "gestura_voice_openai_api_key",
            "gestura_api_key_voice_openai",
        ) {
            self.voice.openai_api_key = Some(secret);
        }

        // SerpAPI
        if let Some(secret) = get_keychain_secret_with_legacy_fallback(
            "gestura_web_search_serpapi_key",
            "gestura_api_key_serpapi",
        ) {
            self.web_search.serpapi_key = Some(secret);
        }

        // Brave
        if let Some(secret) = get_keychain_secret_with_legacy_fallback(
            "gestura_web_search_brave_key",
            "gestura_api_key_brave",
        ) {
            self.web_search.brave_key = Some(secret);
        }

        Ok(())
    }

    /// Async version of hydrate secrets
    #[cfg(feature = "security")]
    async fn hydrate_secrets(&mut self) -> Result<()> {
        if keychain_access_disabled() {
            return Ok(());
        }

        let storage = create_secure_storage();

        // Helper macro to reduce boilerplate.
        // Tries canonical key first, then legacy key; if legacy is found, re-stores
        // under canonical key (best-effort) to converge.
        macro_rules! hydrate_with_legacy {
            ($field:expr, $canonical:expr, $legacy:expr) => {
                // Keychain-first: overwrite YAML value if secure storage has a secret.
                let mut found: Option<String> = None;
                if let Ok(Some(secret)) = storage.get_secret($canonical).await {
                    if !secret.is_empty() {
                        found = Some(secret);
                    }
                }
                if found.is_none() {
                    if let Ok(Some(secret)) = storage.get_secret($legacy).await {
                        if !secret.is_empty() {
                            let _ = storage.store_secret($canonical, &secret).await;
                            found = Some(secret);
                        }
                    }
                }
                if let Some(secret) = found {
                    *$field = secret;
                }
            };
            ($field:expr, $canonical:expr, $legacy:expr, option) => {
                let mut found: Option<String> = None;
                if let Ok(Some(secret)) = storage.get_secret($canonical).await {
                    if !secret.is_empty() {
                        found = Some(secret);
                    }
                }
                if found.is_none() {
                    if let Ok(Some(secret)) = storage.get_secret($legacy).await {
                        if !secret.is_empty() {
                            let _ = storage.store_secret($canonical, &secret).await;
                            found = Some(secret);
                        }
                    }
                }
                if let Some(secret) = found {
                    *$field = Some(secret);
                }
            };
        }

        if let Some(c) = &mut self.llm.openai {
            hydrate_with_legacy!(
                &mut c.api_key,
                "gestura_llm_openai_api_key",
                "gestura_api_key_openai"
            );
        }
        if let Some(c) = &mut self.llm.anthropic {
            hydrate_with_legacy!(
                &mut c.api_key,
                "gestura_llm_anthropic_api_key",
                "gestura_api_key_anthropic"
            );
        }
        if let Some(c) = &mut self.llm.grok {
            hydrate_with_legacy!(
                &mut c.api_key,
                "gestura_llm_grok_api_key",
                "gestura_api_key_grok"
            );
        }
        if let Some(c) = &mut self.llm.gemini {
            hydrate_with_legacy!(
                &mut c.api_key,
                "gestura_llm_gemini_api_key",
                "gestura_api_key_gemini"
            );
        }

        hydrate_with_legacy!(
            &mut self.voice.openai_api_key,
            "gestura_voice_openai_api_key",
            "gestura_api_key_voice_openai",
            option
        );
        hydrate_with_legacy!(
            &mut self.web_search.serpapi_key,
            "gestura_web_search_serpapi_key",
            "gestura_api_key_serpapi",
            option
        );
        hydrate_with_legacy!(
            &mut self.web_search.brave_key,
            "gestura_web_search_brave_key",
            "gestura_api_key_brave",
            option
        );

        Ok(())
    }

    /// Move secrets from the struct (if present) to the keychain.
    /// Returns true if any secrets were migrated.
    #[cfg(feature = "security")]
    fn migrate_secrets_sync(&self) -> Result<bool> {
        if keychain_access_disabled() {
            return Ok(false);
        }

        let mut changed = false;

        // OpenAI
        if let Some(c) = &self.llm.openai
            && !c.api_key.is_empty()
        {
            // Do not overwrite an existing keychain secret.
            if !keychain_has_secret("gestura_llm_openai_api_key", "gestura_api_key_openai") {
                set_keychain_secret("gestura_llm_openai_api_key", &c.api_key)?;
                changed = true;
            }
            // Note: We do NOT clear the key here, because we want it available in memory.
            // It will be cleared when save() calls sanitize_secrets().
        }

        // Anthropic
        if let Some(c) = &self.llm.anthropic
            && !c.api_key.is_empty()
            && !keychain_has_secret("gestura_llm_anthropic_api_key", "gestura_api_key_anthropic")
        {
            set_keychain_secret("gestura_llm_anthropic_api_key", &c.api_key)?;
            changed = true;
        }

        // Grok
        if let Some(c) = &self.llm.grok
            && !c.api_key.is_empty()
            && !keychain_has_secret("gestura_llm_grok_api_key", "gestura_api_key_grok")
        {
            set_keychain_secret("gestura_llm_grok_api_key", &c.api_key)?;
            changed = true;
        }

        // Gemini
        if let Some(c) = &self.llm.gemini
            && !c.api_key.is_empty()
            && !keychain_has_secret("gestura_llm_gemini_api_key", "gestura_api_key_gemini")
        {
            set_keychain_secret("gestura_llm_gemini_api_key", &c.api_key)?;
            changed = true;
        }

        // Voice
        if let Some(key) = self.voice.openai_api_key.as_deref()
            && !key.is_empty()
            && !keychain_has_secret(
                "gestura_voice_openai_api_key",
                "gestura_api_key_voice_openai",
            )
        {
            set_keychain_secret("gestura_voice_openai_api_key", key)?;
            changed = true;
        }

        // SerpAPI
        if let Some(key) = self.web_search.serpapi_key.as_deref()
            && !key.is_empty()
            && !keychain_has_secret("gestura_web_search_serpapi_key", "gestura_api_key_serpapi")
        {
            set_keychain_secret("gestura_web_search_serpapi_key", key)?;
            changed = true;
        }

        // Brave
        if let Some(key) = self.web_search.brave_key.as_deref()
            && !key.is_empty()
            && !keychain_has_secret("gestura_web_search_brave_key", "gestura_api_key_brave")
        {
            set_keychain_secret("gestura_web_search_brave_key", key)?;
            changed = true;
        }

        Ok(changed)
    }

    /// Async version of migrate secrets
    #[cfg(feature = "security")]
    async fn migrate_secrets(&self) -> Result<bool> {
        if keychain_access_disabled() {
            return Ok(false);
        }

        let storage = create_secure_storage();
        let mut changed = false;

        async fn storage_has_secret(
            storage: &dyn crate::security::SecureStorage,
            canonical: &str,
            legacy: &str,
        ) -> bool {
            // Prefer canonical
            if let Ok(Some(v)) = storage.get_secret(canonical).await
                && !v.is_empty()
            {
                return true;
            }
            // Fallback legacy + self-heal
            if let Ok(Some(v)) = storage.get_secret(legacy).await
                && !v.is_empty()
            {
                let _ = storage.store_secret(canonical, &v).await;
                return true;
            }
            false
        }

        macro_rules! migrate {
            ($field:expr, $canonical:expr, $legacy:expr) => {
                if !$field.is_empty() {
                    if !storage_has_secret(storage.as_ref(), $canonical, $legacy).await {
                        let _ = storage.store_secret($canonical, $field).await;
                        changed = true;
                    }
                }
            };
            ($field:expr, $canonical:expr, $legacy:expr, option) => {
                if let Some(val) = $field {
                    if !val.is_empty() {
                        if !storage_has_secret(storage.as_ref(), $canonical, $legacy).await {
                            let _ = storage.store_secret($canonical, val).await;
                            changed = true;
                        }
                    }
                }
            };
        }

        if let Some(c) = &self.llm.openai {
            migrate!(
                &c.api_key,
                "gestura_llm_openai_api_key",
                "gestura_api_key_openai"
            );
        }
        if let Some(c) = &self.llm.anthropic {
            migrate!(
                &c.api_key,
                "gestura_llm_anthropic_api_key",
                "gestura_api_key_anthropic"
            );
        }
        if let Some(c) = &self.llm.grok {
            migrate!(
                &c.api_key,
                "gestura_llm_grok_api_key",
                "gestura_api_key_grok"
            );
        }
        if let Some(c) = &self.llm.gemini {
            migrate!(
                &c.api_key,
                "gestura_llm_gemini_api_key",
                "gestura_api_key_gemini"
            );
        }

        migrate!(
            &self.voice.openai_api_key,
            "gestura_voice_openai_api_key",
            "gestura_api_key_voice_openai",
            option
        );
        migrate!(
            &self.web_search.serpapi_key,
            "gestura_web_search_serpapi_key",
            "gestura_api_key_serpapi",
            option
        );
        migrate!(
            &self.web_search.brave_key,
            "gestura_web_search_brave_key",
            "gestura_api_key_brave",
            option
        );

        Ok(changed)
    }

    /// Load configuration with environment variable overrides applied
    ///
    /// This is the recommended way to load configuration as it respects
    /// the full precedence hierarchy: env vars > config file > defaults
    fn load_with_env() -> Self {
        Self::load().apply_env_overrides()
    }

    /// Load configuration asynchronously with environment variable overrides
    async fn load_with_env_async() -> Self {
        Self::load_async().await.apply_env_overrides()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::CompactionStrategy;
    use std::sync::{Mutex, OnceLock};

    #[test]
    fn default_config_has_expected_values() {
        let c = AppConfig::default();
        assert_eq!(c.hotkey_listen, "Ctrl+Space");
        assert_eq!(c.grace_period_secs, 30);
        assert_eq!(c.llm.primary, "anthropic");
        assert_eq!(c.llm.fallback, Some("ollama".to_string()));
        // Ollama should have sensible defaults so it works when selected
        assert!(c.llm.ollama.is_some());
        let ollama = c.llm.ollama.unwrap();
        assert_eq!(ollama.base_url, "http://localhost:11434");
        assert_eq!(ollama.model, "llama3.2");
    }

    #[test]
    fn test_config_get() {
        let c = AppConfig::default();
        assert_eq!(c.get("llm.primary"), Some("anthropic".to_string()));
        assert_eq!(c.get("unknown.key"), None);
    }

    #[test]
    fn test_whisper_model_info() {
        let models = WhisperModelInfo::available_models();
        assert!(!models.is_empty());
        let recommended: Vec<_> = models.iter().filter(|m| m.recommended).collect();
        assert_eq!(recommended.len(), 1);
    }

    #[test]
    fn test_backward_compatibility_without_pipeline_settings() {
        // Create a default config and serialize it
        let default_config = AppConfig::default();
        let mut json_value: serde_json::Value = serde_json::to_value(&default_config).unwrap();

        // Remove the pipeline field to simulate an old config file
        json_value.as_object_mut().unwrap().remove("pipeline");

        // Deserialize should succeed and use default pipeline settings
        let config: AppConfig = serde_json::from_value(json_value).unwrap();

        // Verify pipeline settings have default values
        assert_eq!(config.pipeline.max_history_messages, 10);
        assert_eq!(config.pipeline.auto_compact_threshold_percent, 80);
        assert_eq!(
            config.pipeline.compaction_strategy,
            CompactionStrategy::Summarize
        );
        assert_eq!(config.pipeline.max_context_tokens, 0);
        assert!(config.pipeline.log_token_usage);
    }

    #[test]
    fn test_backward_compatibility_with_partial_pipeline_settings() {
        // Create a default config and serialize it
        let default_config = AppConfig::default();
        let mut json_value: serde_json::Value = serde_json::to_value(&default_config).unwrap();

        // Modify pipeline to only have max_history_messages
        let pipeline_obj = serde_json::json!({
            "max_history_messages": 20
        });
        json_value
            .as_object_mut()
            .unwrap()
            .insert("pipeline".to_string(), pipeline_obj);

        // Deserialize should succeed and use defaults for missing fields
        let config: AppConfig = serde_json::from_value(json_value).unwrap();

        // Verify explicitly set value
        assert_eq!(config.pipeline.max_history_messages, 20);

        // Verify other fields have default values
        assert_eq!(config.pipeline.auto_compact_threshold_percent, 80);
        assert_eq!(
            config.pipeline.compaction_strategy,
            CompactionStrategy::Summarize
        );
        assert_eq!(config.pipeline.max_context_tokens, 0);
        assert!(config.pipeline.log_token_usage);
    }

    #[test]
    fn test_pipeline_settings_serialization_roundtrip() {
        // Create a config with custom pipeline settings
        let mut config = AppConfig::default();
        config.pipeline.max_history_messages = 15;
        config.pipeline.auto_compact_threshold_percent = 75;
        config.pipeline.compaction_strategy = CompactionStrategy::MemoryBank;
        config.pipeline.max_context_tokens = 50000;
        config.pipeline.log_token_usage = false;

        // Serialize to YAML
        let yaml = serde_yaml::to_string(&config).unwrap();

        // Deserialize back
        let deserialized: AppConfig = serde_yaml::from_str(&yaml).unwrap();

        // Verify all pipeline settings are preserved
        assert_eq!(deserialized.pipeline.max_history_messages, 15);
        assert_eq!(deserialized.pipeline.auto_compact_threshold_percent, 75);
        assert_eq!(
            deserialized.pipeline.compaction_strategy,
            CompactionStrategy::MemoryBank
        );
        assert_eq!(deserialized.pipeline.max_context_tokens, 50000);
        assert!(!deserialized.pipeline.log_token_usage);
    }

    /// Global lock used to serialize environment-variable mutation across tests.
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Global lock used to serialize test-keychain mutation across tests.
    ///
    /// The keychain shim used in tests is process-global state, and Rust tests
    /// run in parallel by default.
    #[cfg(feature = "security")]
    fn keychain_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// RAII helper for setting a process-wide environment variable for the duration of a scope.
    ///
    /// ## Safety / Concurrency
    /// Environment variables are process-global state. Tests that use this helper should
    /// serialize access (e.g. by holding `env_lock()`) to avoid concurrent mutation.
    struct ScopedEnvVar {
        key: &'static str,
        old: Option<String>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: String) -> Self {
            let old = std::env::var(key).ok();
            // Rust 2024: mutating process-wide environment variables is `unsafe`.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            match &self.old {
                Some(v) => unsafe {
                    std::env::set_var(self.key, v);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

    #[test]
    #[cfg_attr(
        target_os = "windows",
        ignore = "dirs::home_dir() bypasses env var overrides on Windows"
    )]
    fn migrates_legacy_json_config_to_yaml_on_load() {
        // This test mutates process-wide env vars; serialize it.
        let _guard = env_lock().lock().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let _home = ScopedEnvVar::set("HOME", home.to_string_lossy().to_string());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.to_string_lossy().to_string());
        let _homedrive = ScopedEnvVar::set("HOMEDRIVE", "C:".to_string());
        let _homepath = ScopedEnvVar::set("HOMEPATH", "\\".to_string());

        let gestura_dir = home.join(".gestura");
        fs::create_dir_all(&gestura_dir).unwrap();

        let json_path = gestura_dir.join("config.json");
        let yaml_path = gestura_dir.join("config.yaml");
        let backup_path = gestura_dir.join("config.json.backup");

        // Write legacy JSON config
        let cfg = AppConfig::default();
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        fs::write(&json_path, json).unwrap();
        assert!(!yaml_path.exists());

        // Loading should migrate and return the legacy config contents.
        let loaded = AppConfig::load();
        assert_eq!(loaded, cfg);

        // YAML should exist after migration.
        assert!(yaml_path.exists());

        // Legacy JSON should be backed up (best-effort).
        assert!(!json_path.exists() || backup_path.exists());
    }

    #[test]
    #[cfg(feature = "security")]
    #[cfg_attr(
        target_os = "windows",
        ignore = "dirs::home_dir() bypasses env var overrides on Windows"
    )]
    fn migrates_plaintext_openai_key_to_keystore_and_sanitizes_yaml_on_load() {
        // This test mutates process-wide env vars; serialize it.
        let _guard = env_lock().lock().unwrap();

        // This test also mutates the process-global test keychain store.
        let _keychain_guard = keychain_lock().lock().unwrap();

        clear_test_keychain_store();

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let _home = ScopedEnvVar::set("HOME", home.to_string_lossy().to_string());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.to_string_lossy().to_string());
        let _homedrive = ScopedEnvVar::set("HOMEDRIVE", "C:".to_string());
        let _homepath = ScopedEnvVar::set("HOMEPATH", "\\".to_string());

        let gestura_dir = home.join(".gestura");
        fs::create_dir_all(&gestura_dir).unwrap();

        let yaml_path = gestura_dir.join("config.yaml");

        let secret = "sk-test-openai-1234567890".to_string();
        let mut cfg = AppConfig::default();
        cfg.llm.openai = Some(OpenAiConfig {
            api_key: secret.clone(),
            ..Default::default()
        });

        let yaml = serde_yaml::to_string(&cfg).unwrap();
        assert!(yaml.contains(&secret));
        fs::write(&yaml_path, yaml).unwrap();

        // Load should migrate secrets to secure storage and then sanitize persisted YAML.
        let loaded = AppConfig::load();
        assert_eq!(
            loaded.llm.openai.as_ref().unwrap().api_key,
            secret,
            "loaded config should retain the secret in-memory"
        );

        let persisted = fs::read_to_string(&yaml_path).unwrap();
        assert!(
            !persisted.contains(&secret),
            "persisted YAML must not contain plaintext secrets"
        );

        let persisted_cfg: AppConfig = serde_yaml::from_str(&persisted).unwrap();
        assert!(
            persisted_cfg
                .llm
                .openai
                .as_ref()
                .is_some_and(|c| c.api_key.is_empty()),
            "persisted config should have openai.api_key cleared"
        );

        // Second load should hydrate from secure storage (keychain shim in tests).
        let loaded_again = AppConfig::load();
        assert_eq!(
            loaded_again.llm.openai.as_ref().unwrap().api_key,
            secret,
            "second load should hydrate secret from secure storage"
        );

        let persisted2 = fs::read_to_string(&yaml_path).unwrap();
        assert!(!persisted2.contains(&secret));
    }

    #[test]
    #[cfg(feature = "security")]
    #[cfg_attr(
        target_os = "windows",
        ignore = "dirs::home_dir() bypasses env var overrides on Windows"
    )]
    fn keychain_secret_overrides_yaml_and_plaintext_is_sanitized_on_load() {
        // This test mutates process-wide env vars; serialize it.
        let _guard = env_lock().lock().unwrap();

        // This test also mutates the process-global test keychain store.
        let _keychain_guard = keychain_lock().lock().unwrap();

        clear_test_keychain_store();

        // Seed secure storage with a canonical secret.
        let keychain_secret = "sk-keychain-openai-A";
        set_keychain_secret("gestura_llm_openai_api_key", keychain_secret).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let _home = ScopedEnvVar::set("HOME", home.to_string_lossy().to_string());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.to_string_lossy().to_string());
        let _homedrive = ScopedEnvVar::set("HOMEDRIVE", "C:".to_string());
        let _homepath = ScopedEnvVar::set("HOMEPATH", "\\".to_string());

        let gestura_dir = home.join(".gestura");
        fs::create_dir_all(&gestura_dir).unwrap();
        let yaml_path = gestura_dir.join("config.yaml");

        // Write a plaintext YAML secret that must be ignored (keychain-first).
        let yaml_secret = "sk-yaml-openai-B".to_string();
        let mut cfg = AppConfig::default();
        cfg.llm.openai = Some(OpenAiConfig {
            api_key: yaml_secret.clone(),
            ..Default::default()
        });
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        assert!(yaml.contains(&yaml_secret));
        fs::write(&yaml_path, yaml).unwrap();

        // Load should prefer keychain, and must sanitize the on-disk YAML.
        let loaded = AppConfig::load();
        assert_eq!(
            loaded.llm.openai.as_ref().unwrap().api_key,
            keychain_secret,
            "keychain secret must override plaintext YAML value"
        );

        let persisted = fs::read_to_string(&yaml_path).unwrap();
        assert!(
            !persisted.contains(&yaml_secret),
            "persisted YAML must not contain plaintext secrets"
        );
        assert!(
            !persisted.contains(keychain_secret),
            "persisted YAML must not contain hydrated keychain secrets"
        );

        // Secure storage should still contain the original keychain secret.
        let keychain_after = get_keychain_secret("gestura_llm_openai_api_key");
        assert_eq!(keychain_after.as_deref(), Some(keychain_secret));
    }

    #[test]
    #[cfg(feature = "security")]
    fn hydrate_secrets_sync_falls_back_to_legacy_keys_and_self_heals() {
        let _keychain_guard = keychain_lock().lock().unwrap();

        clear_test_keychain_store();

        // Pretend an older release stored the OpenAI key under a legacy key name.
        let legacy_secret = "sk-legacy-openai";
        set_keychain_secret("gestura_api_key_openai", legacy_secret).unwrap();

        let mut cfg = AppConfig::default();
        cfg.llm.openai = Some(OpenAiConfig {
            api_key: String::new(),
            ..Default::default()
        });

        cfg.hydrate_secrets_sync().unwrap();
        assert_eq!(cfg.llm.openai.as_ref().unwrap().api_key, legacy_secret);

        // Self-heal should have copied the secret to the canonical key.
        let canonical = get_keychain_secret("gestura_llm_openai_api_key");
        assert_eq!(canonical.as_deref(), Some(legacy_secret));
    }
}
