//! Speech-to-text provider abstraction.
//!
//! This mirrors the `llm_provider` module: a small trait, provider implementations,
//! and a selection function based on `AppConfig`.

use std::path::Path;
#[cfg(feature = "voice-local")]
use std::path::PathBuf;

use crate::speech::TranscriptionResult;
use gestura_core_config::AppConfig;
use gestura_core_foundation::AppError;
use gestura_core_foundation::secrets::{SecretKey, SecretProvider};
use gestura_core_sessions::chat_sessions::SessionVoiceConfig;

/// Normalize an optional string override by trimming and treating empty/whitespace as `None`.
fn normalize_override(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

/// Resolve the effective STT provider id, applying session overrides.
///
/// Session overrides take precedence when present and non-empty.
fn resolve_effective_provider(config: &AppConfig, session: Option<&SessionVoiceConfig>) -> String {
    normalize_override(session.and_then(|s| s.provider.as_deref()))
        .map(str::to_string)
        .unwrap_or_else(|| config.voice.provider.clone())
}

/// Resolve the effective OpenAI transcription model, applying session overrides.
///
/// Precedence:
/// 1) `session.model` (trimmed, non-empty)
/// 2) `config.voice.openai_model`
/// 3) default (`gpt-4o-transcribe`)
fn resolve_effective_openai_model(
    config: &AppConfig,
    session: Option<&SessionVoiceConfig>,
) -> String {
    if let Some(m) = normalize_override(session.and_then(|s| s.model.as_deref())) {
        return m.to_string();
    }

    config
        .voice
        .openai_model
        .clone()
        .unwrap_or_else(|| "gpt-4o-transcribe".to_string())
}

/// Unified STT interface (async).
///
/// Provider implementations must be `Send + Sync` so they can be used behind
/// a `Box<dyn SttProvider>` across async boundaries.
#[async_trait::async_trait]
pub trait SttProvider: Send + Sync {
    /// Returns a stable provider id for logs/telemetry.
    fn provider_id(&self) -> &'static str;

    /// Transcribe an audio file into text.
    async fn transcribe_file(&self, audio_path: &Path) -> Result<TranscriptionResult, AppError>;
}

/// A provider that returns a helpful error when STT is not configured.
pub struct UnconfiguredSttProvider {
    message: String,
}

impl UnconfiguredSttProvider {
    /// Create a new unconfigured provider with a user-facing message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait::async_trait]
impl SttProvider for UnconfiguredSttProvider {
    fn provider_id(&self) -> &'static str {
        "unconfigured"
    }

    async fn transcribe_file(&self, _audio_path: &Path) -> Result<TranscriptionResult, AppError> {
        Err(AppError::Voice(self.message.clone()))
    }
}

/// OpenAI (or OpenAI-compatible) STT provider.
pub struct OpenAiSttProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

impl OpenAiSttProvider {
    /// Build the OpenAI transcription endpoint URL from the configured base URL.
    pub fn transcription_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/v1/audio/transcriptions")
    }
}

#[async_trait::async_trait]
impl SttProvider for OpenAiSttProvider {
    fn provider_id(&self) -> &'static str {
        "openai"
    }

    async fn transcribe_file(&self, audio_path: &Path) -> Result<TranscriptionResult, AppError> {
        let client = reqwest::Client::new();

        let bytes = std::fs::read(audio_path)
            .map_err(|e| AppError::Voice(format!("Failed to read audio file: {e}")))?;
        let file_name = audio_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("audio.wav")
            .to_string();

        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str("audio/wav")
            .map_err(|e| AppError::Voice(format!("Invalid multipart audio part: {e}")))?;

        let form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .part("file", part);

        let resp = client
            .post(self.transcription_url())
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| AppError::Voice(format!("OpenAI STT request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Voice(format!(
                "OpenAI STT API error {status}: {body}"
            )));
        }

        #[derive(serde::Deserialize)]
        struct WhisperResponse {
            text: String,
        }

        let result: WhisperResponse = resp
            .json()
            .await
            .map_err(|e| AppError::Voice(format!("Failed to parse OpenAI STT response: {e}")))?;

        Ok(TranscriptionResult {
            text: result.text,
            duration_secs: 0.0,
            audio_path: Some(audio_path.to_path_buf()),
            provider: "openai-whisper".to_string(),
        })
    }
}

/// Local Whisper provider (whisper-rs / whisper.cpp models).
#[cfg(feature = "voice-local")]
pub struct LocalWhisperProvider {
    pub model_path: PathBuf,
}

#[cfg(feature = "voice-local")]
#[async_trait::async_trait]
impl SttProvider for LocalWhisperProvider {
    fn provider_id(&self) -> &'static str {
        "local-whisper"
    }

    async fn transcribe_file(&self, audio_path: &Path) -> Result<TranscriptionResult, AppError> {
        // whisper-rs is synchronous and can perform significant CPU work.
        // Run it in a blocking task to avoid stalling the async runtime.
        let model_path = self.model_path.clone();
        let audio_path = audio_path.to_path_buf();

        tokio::task::spawn_blocking(move || {
            use whisper_rs::{
                FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
            };

            let ctx = WhisperContext::new_with_params(
                model_path
                    .to_str()
                    .ok_or_else(|| AppError::Voice("Invalid model path encoding".to_string()))?,
                WhisperContextParameters::default(),
            )
            .map_err(|e| AppError::Voice(format!("Failed to load Whisper model: {e}")))?;

            let samples = crate::speech::load_audio_samples_16khz_mono(&audio_path)?;
            let duration_secs = samples.len() as f32 / 16000.0;

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(Some("en"));
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            params.set_translate(false);
            params.set_no_context(true);
            params.set_single_segment(false);

            let mut state = ctx
                .create_state()
                .map_err(|e| AppError::Voice(format!("Failed to create Whisper state: {e}")))?;
            state
                .full(params, &samples)
                .map_err(|e| AppError::Voice(format!("Whisper transcription failed: {e}")))?;

            let num_segments = state
                .full_n_segments()
                .map_err(|e| AppError::Voice(format!("Failed to get segment count: {e}")))?;
            let mut text = String::new();
            for i in 0..num_segments {
                if let Ok(seg) = state.full_get_segment_text(i) {
                    text.push_str(seg.trim());
                    text.push(' ');
                }
            }
            let text = text.trim().to_string();

            Ok(TranscriptionResult {
                text,
                duration_secs,
                audio_path: Some(audio_path),
                provider: "local-whisper".to_string(),
            })
        })
        .await
        .map_err(|e| AppError::Voice(format!("Local Whisper transcription task failed: {e}")))?
    }
}

/// Select an STT provider from app configuration.
///
/// This is intentionally conservative: if required fields (like API keys or
/// model paths) are missing, it returns an `UnconfiguredSttProvider` that
/// provides an actionable error message.
///
/// If `secrets` is provided, it will be consulted for API keys using
/// [`SecretKey`] fallbacks.
pub async fn select_provider(
    config: &AppConfig,
    secrets: Option<&dyn SecretProvider>,
) -> Box<dyn SttProvider> {
    select_provider_with_session_voice_config(config, None, secrets).await
}

/// Select an STT provider from configuration, applying optional per-session voice overrides.
///
/// This function is the session-aware variant of [`select_provider`]. It implements the
/// **core-owned** precedence rules for per-session overrides:
///
/// - Provider: `session.provider` (trimmed, non-empty) overrides `config.voice.provider`
/// - OpenAI model: `session.model` overrides `config.voice.openai_model` (defaulting to
///   `gpt-4o-transcribe`) **when OpenAI STT is selected**
/// - Local Whisper model: `session.model` is interpreted as a path or filename per
///   [`crate::speech::resolve_whisper_model_path_with_override`] **when local STT is selected**
///
/// This is intentionally conservative: if required fields (like API keys or model paths) are
/// missing, it returns an [`UnconfiguredSttProvider`] with an actionable error message.
///
/// If `secrets` is provided, it will be consulted for API keys using [`SecretKey`] fallbacks.
pub async fn select_provider_with_session_voice_config(
    config: &AppConfig,
    session_voice_config: Option<&SessionVoiceConfig>,
    secrets: Option<&dyn SecretProvider>,
) -> Box<dyn SttProvider> {
    let effective_provider = resolve_effective_provider(config, session_voice_config);

    match effective_provider.as_str() {
        "openai" => {
            let api_key = resolve_openai_stt_api_key(config, secrets).await;

            if api_key.is_empty() {
                return Box::new(UnconfiguredSttProvider::new(
                    "OpenAI STT selected but no API key configured. Set voice.openai_api_key, or store a key in secure storage under 'voice_openai' (preferred) or 'openai'.",
                ));
            }

            let base_url = config
                .voice
                .openai_base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com".to_string());
            let model = resolve_effective_openai_model(config, session_voice_config);

            Box::new(OpenAiSttProvider {
                api_key,
                base_url,
                model,
            })
        }
        "local" => {
            #[cfg(feature = "voice-local")]
            {
                let session_model =
                    normalize_override(session_voice_config.and_then(|s| s.model.as_deref()));

                match crate::speech::resolve_whisper_model_path_with_override(config, session_model)
                {
                    Ok(model_path) => Box::new(LocalWhisperProvider { model_path }),
                    Err(e) => Box::new(UnconfiguredSttProvider::new(e.to_string())),
                }
            }
            #[cfg(not(feature = "voice-local"))]
            {
                Box::new(UnconfiguredSttProvider::new(
                    "Local Whisper selected but the 'voice-local' feature is disabled.",
                ))
            }
        }
        "none" => Box::new(UnconfiguredSttProvider::new(
            "STT provider is disabled (voice.provider=none).",
        )),
        other => Box::new(UnconfiguredSttProvider::new(format!(
            "Unknown STT provider '{other}'. Supported: openai | local | none"
        ))),
    }
}

/// Resolve the API key to use for OpenAI STT.
///
/// Precedence (first non-empty wins):
/// 1) `config.voice.openai_api_key`
/// 2) `secrets[voice_openai]` (if `secrets` provided)
/// 3) `secrets[openai]` (if `secrets` provided)
/// 4) `config.llm.openai.api_key` (back-compat)
async fn resolve_openai_stt_api_key(
    config: &AppConfig,
    secrets: Option<&dyn SecretProvider>,
) -> String {
    let config_key = config.voice.openai_api_key.clone().unwrap_or_default();
    if !config_key.is_empty() {
        return config_key;
    }

    if let Some(secrets) = secrets {
        if let Some(k) = secrets.get_secret(SecretKey::VoiceOpenAi).await
            && !k.is_empty()
        {
            return k;
        }
        if let Some(k) = secrets.get_secret(SecretKey::OpenAi).await
            && !k.is_empty()
        {
            return k;
        }
    }

    // Back-compat: allow re-using LLM OpenAI key if voice key is not set.
    config
        .llm
        .openai
        .as_ref()
        .map(|c| c.api_key.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[derive(Debug, Default)]
    struct TestSecrets(std::collections::HashMap<SecretKey, String>);

    #[async_trait::async_trait]
    impl SecretProvider for TestSecrets {
        async fn get_secret(&self, key: SecretKey) -> Option<String> {
            self.0.get(&key).cloned().filter(|s| !s.is_empty())
        }
    }

    /// Stores raw request bytes captured by the mock HTTP server.
    #[derive(Clone, Default)]
    struct CapturedRequest(Arc<Mutex<Vec<u8>>>);

    impl CapturedRequest {
        /// Take the captured request bytes (leaving the stored value empty).
        fn take(&self) -> Vec<u8> {
            std::mem::take(&mut *self.0.lock().expect("capture lock"))
        }
    }

    /// Find the first index of `needle` within `haystack`.
    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// Capture a single HTTP/1.1 request from `stream`.
    ///
    /// This is intentionally minimal and exists only to support unit tests without
    /// pulling in an HTTP server dependency.
    fn capture_http_request(stream: &mut TcpStream) -> Vec<u8> {
        let mut buf = Vec::<u8>::new();
        let mut tmp = [0u8; 8 * 1024];

        let mut header_end: Option<usize> = None;
        let mut content_length: Option<usize> = None;
        let mut chunked = false;
        let mut sent_continue = false;

        loop {
            match stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                }
                Err(_) => break,
            }

            if header_end.is_none()
                && let Some(pos) = find_subslice(&buf, b"\r\n\r\n")
            {
                let end = pos + 4;
                header_end = Some(end);

                let header_text = String::from_utf8_lossy(&buf[..end]);
                for line in header_text.split("\r\n") {
                    let lower = line.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("content-length:")
                        && let Ok(n) = v.trim().parse::<usize>()
                    {
                        content_length = Some(n);
                    }
                    if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
                        chunked = true;
                    }
                    if lower == "expect: 100-continue" {
                        // If the client expects 100-continue, respond so it will send the body.
                        if !sent_continue {
                            let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
                            let _ = stream.flush();
                            sent_continue = true;
                        }
                    }
                }
            }

            if let Some(h_end) = header_end {
                if let Some(len) = content_length {
                    if buf.len() >= h_end + len {
                        break;
                    }
                } else if chunked {
                    // Very small heuristic for tests: stop when we see the terminating chunk.
                    if find_subslice(&buf[h_end..], b"\r\n0\r\n\r\n").is_some() {
                        break;
                    }
                }
            }
        }

        buf
    }

    /// Start a single-request mock HTTP server.
    ///
    /// Returns the base URL to use for a client, a capture handle to retrieve the
    /// raw request, and a join handle for the server thread.
    fn spawn_mock_http_server(
        status: u16,
        content_type: &'static str,
        body: &'static str,
    ) -> (String, CapturedRequest, thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind tcp listener");
        let addr = listener.local_addr().expect("local addr");

        let captured = CapturedRequest::default();
        let captured_for_thread = captured.clone();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));

            let req = capture_http_request(&mut stream);
            *captured_for_thread.0.lock().expect("capture lock") = req;

            let body_bytes = body.as_bytes();
            let resp = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body_bytes.len()
            );
            stream
                .write_all(resp.as_bytes())
                .and_then(|_| stream.write_all(body_bytes))
                .and_then(|_| stream.flush())
                .expect("write response");
        });

        (format!("http://{addr}"), captured, handle)
    }

    #[test]
    fn openai_transcription_url_uses_base_url() {
        let p = OpenAiSttProvider {
            api_key: "x".into(),
            base_url: "https://example.com/".into(),
            model: "whisper-1".into(),
        };
        assert_eq!(
            p.transcription_url(),
            "https://example.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn resolve_openai_model_prefers_session_then_config_then_default() {
        let mut cfg = AppConfig::default();
        cfg.voice.openai_model = Some("cfg-model".into());

        let session = SessionVoiceConfig {
            provider: None,
            model: Some("session-model".into()),
        };
        assert_eq!(
            resolve_effective_openai_model(&cfg, Some(&session)),
            "session-model"
        );

        let session_blank = SessionVoiceConfig {
            provider: None,
            model: Some("   ".into()),
        };
        assert_eq!(
            resolve_effective_openai_model(&cfg, Some(&session_blank)),
            "cfg-model"
        );

        let mut cfg2 = AppConfig::default();
        cfg2.voice.openai_model = None;
        assert_eq!(
            resolve_effective_openai_model(&cfg2, None),
            "gpt-4o-transcribe"
        );
    }

    #[tokio::test]
    async fn session_provider_override_wins_over_config() {
        let mut cfg = AppConfig::default();
        cfg.voice.provider = "openai".into();
        cfg.voice.openai_api_key = Some("cfg_voice".into());

        let session = SessionVoiceConfig {
            provider: Some("none".into()),
            model: None,
        };

        let p = select_provider_with_session_voice_config(&cfg, Some(&session), None).await;
        assert_eq!(p.provider_id(), "unconfigured");
    }

    #[tokio::test]
    async fn session_provider_override_is_trimmed() {
        let mut cfg = AppConfig::default();
        cfg.voice.provider = "none".into();
        cfg.voice.openai_api_key = Some("cfg_voice".into());

        let session = SessionVoiceConfig {
            provider: Some("  openai  ".into()),
            model: None,
        };

        let p = select_provider_with_session_voice_config(&cfg, Some(&session), None).await;
        assert_eq!(p.provider_id(), "openai");
    }

    #[tokio::test]
    async fn blank_session_provider_override_uses_config_provider() {
        let mut cfg = AppConfig::default();
        cfg.voice.provider = "none".into();

        let session = SessionVoiceConfig {
            provider: Some("   ".into()),
            model: None,
        };

        let p = select_provider_with_session_voice_config(&cfg, Some(&session), None).await;
        assert_eq!(p.provider_id(), "unconfigured");
    }

    #[tokio::test]
    async fn unknown_provider_yields_unconfigured_provider() {
        let mut cfg = AppConfig::default();
        cfg.voice.provider = "wat".into();

        let p = select_provider(&cfg, None).await;
        assert_eq!(p.provider_id(), "unconfigured");
    }

    #[cfg(feature = "voice-local")]
    #[tokio::test]
    async fn session_local_model_path_override_selects_local_provider() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let model_file = tmp.path().join("ggml-tiny.en.bin");
        std::fs::write(&model_file, b"test").expect("write model");

        let mut cfg = AppConfig::default();
        cfg.voice.provider = "openai".into();

        let session = SessionVoiceConfig {
            provider: Some("local".into()),
            model: Some(model_file.to_string_lossy().to_string()),
        };

        let p = select_provider_with_session_voice_config(&cfg, Some(&session), None).await;
        assert_eq!(p.provider_id(), "local-whisper");
    }

    #[tokio::test]
    async fn resolve_openai_key_prefers_voice_config_over_secrets() {
        let mut cfg = AppConfig::default();
        cfg.voice.provider = "openai".into();
        cfg.voice.openai_api_key = Some("cfg_voice".into());

        let mut secrets = TestSecrets::default();
        secrets
            .0
            .insert(SecretKey::VoiceOpenAi, "secret_voice".into());

        let p = select_provider(&cfg, Some(&secrets)).await;
        assert_eq!(p.provider_id(), "openai");
    }

    #[tokio::test]
    async fn resolve_openai_key_uses_voice_secret_then_general_secret_then_llm_fallback() {
        let mut cfg = AppConfig::default();
        cfg.voice.provider = "openai".into();
        cfg.voice.openai_api_key = None;
        cfg.llm.openai = Some(gestura_core_config::OpenAiConfig {
            api_key: "cfg_llm".into(),
            model: "gpt-4o-mini".into(),
            base_url: None,
        });

        // 1) voice secret wins
        let mut s1 = TestSecrets::default();
        s1.0.insert(SecretKey::VoiceOpenAi, "secret_voice".into());
        let p1 = select_provider(&cfg, Some(&s1)).await;
        assert_eq!(p1.provider_id(), "openai");

        // 2) general secret wins when voice secret missing
        let mut s2 = TestSecrets::default();
        s2.0.insert(SecretKey::OpenAi, "secret_general".into());
        let p2 = select_provider(&cfg, Some(&s2)).await;
        assert_eq!(p2.provider_id(), "openai");

        // 3) llm fallback wins when no secrets
        let p3 = select_provider(&cfg, None).await;
        assert_eq!(p3.provider_id(), "openai");
    }

    #[tokio::test]
    async fn openai_selected_without_any_key_is_unconfigured() {
        let mut cfg = AppConfig::default();
        cfg.voice.provider = "openai".into();
        cfg.voice.openai_api_key = None;
        cfg.llm.openai = None;

        let secrets = TestSecrets::default();
        let p = select_provider(&cfg, Some(&secrets)).await;
        assert_eq!(p.provider_id(), "unconfigured");
    }

    #[tokio::test]
    async fn openai_stt_request_includes_bearer_auth_and_model_field() {
        let (base_url, captured, server) =
            spawn_mock_http_server(200, "application/json", r#"{"text":"hello"}"#);

        let tmp = tempfile::tempdir().expect("tempdir");
        let audio_path = tmp.path().join("audio.wav");
        std::fs::write(&audio_path, b"RIFF....WAVEfmt ").expect("write audio");

        let p = OpenAiSttProvider {
            api_key: "TEST_KEY".into(),
            base_url,
            model: "gpt-4o-transcribe".into(),
        };

        let result = tokio::time::timeout(Duration::from_secs(5), p.transcribe_file(&audio_path))
            .await
            .expect("transcribe timeout")
            .expect("transcribe ok");
        assert_eq!(result.text, "hello");

        server.join().expect("server join");
        let req = String::from_utf8_lossy(&captured.take()).to_ascii_lowercase();

        assert!(req.contains("post /v1/audio/transcriptions"));
        assert!(req.contains("authorization: bearer test_key"));
        assert!(req.contains("content-type: multipart/form-data"));
        assert!(req.contains("name=\"model\""));
        assert!(req.contains("gpt-4o-transcribe"));
        assert!(req.contains("name=\"file\""));
    }

    #[tokio::test]
    async fn openai_stt_non_success_status_maps_to_voice_error_with_body() {
        let (base_url, _captured, server) = spawn_mock_http_server(401, "text/plain", "nope");

        let tmp = tempfile::tempdir().expect("tempdir");
        let audio_path = tmp.path().join("audio.wav");
        std::fs::write(&audio_path, b"x").expect("write audio");

        let p = OpenAiSttProvider {
            api_key: "TEST_KEY".into(),
            base_url,
            model: "gpt-4o-transcribe".into(),
        };

        let err = tokio::time::timeout(Duration::from_secs(5), p.transcribe_file(&audio_path))
            .await
            .expect("transcribe timeout")
            .expect_err("expected error");

        server.join().expect("server join");

        match err {
            AppError::Voice(msg) => {
                assert!(msg.contains("401"), "msg={msg}");
                assert!(msg.contains("nope"), "msg={msg}");
            }
            other => panic!("expected AppError::Voice, got {other:?}"),
        }
    }
}
