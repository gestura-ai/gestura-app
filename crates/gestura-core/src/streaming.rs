//! Streaming LLM provider support for Gestura
//!
//! This module provides streaming capabilities for LLM responses, enabling
//! real-time token-by-token delivery to the frontend with cancellation support.

use crate::config::AppConfig;
use crate::error::AppError;
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Default timeout for streaming LLM API calls
const STREAMING_TIMEOUT_SECS: u64 = 300;

/// A chunk of streaming response
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// A text chunk from the LLM
    Text(String),
    /// Stream completed successfully
    Done,
    /// Stream was cancelled
    Cancelled,
    /// An error occurred
    Error(String),
}

/// Cancellation token for streaming requests
#[derive(Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create a new cancellation token
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Cancel the streaming request
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Check if cancellation has been requested
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a reqwest client for streaming requests
fn create_streaming_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(STREAMING_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Stream a response from OpenAI-compatible API
pub async fn stream_openai(
    api_key: &str,
    base_url: &str,
    model: &str,
    prompt: &str,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    let url = format!("{}/v1/chat/completions", base_url);
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.2,
        "stream": true
    });

    let client = create_streaming_client();
    let response = client
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("OpenAI streaming request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Llm(format!("OpenAI HTTP {}: {}", status, body)));
    }

    let mut stream = response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        if cancel_token.is_cancelled() {
            let _ = tx.send(StreamChunk::Cancelled).await;
            return Ok(());
        }

        match chunk_result {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                for line in text.lines() {
                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };
                    if data == "[DONE]" {
                        let _ = tx.send(StreamChunk::Done).await;
                        return Ok(());
                    }
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(content) = json["choices"][0]["delta"]["content"].as_str()
                        && !content.is_empty()
                    {
                        let _ = tx.send(StreamChunk::Text(content.to_string())).await;
                    }
                }
            }
            Err(e) => {
                let _ = tx
                    .send(StreamChunk::Error(format!("Stream error: {}", e)))
                    .await;
                return Err(AppError::Llm(format!("Stream error: {}", e)));
            }
        }
    }

    let _ = tx.send(StreamChunk::Done).await;
    Ok(())
}

/// Stream a response from Anthropic Claude API
pub async fn stream_anthropic(
    api_key: &str,
    base_url: &str,
    model: &str,
    prompt: &str,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    let url = format!("{}/v1/messages", base_url);
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "messages": [{"role": "user", "content": [{"type": "text", "text": prompt}]}],
        "stream": true
    });

    let client = create_streaming_client();
    let response = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("Anthropic streaming request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Llm(format!(
            "Anthropic HTTP {}: {}",
            status, body
        )));
    }

    let mut stream = response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        if cancel_token.is_cancelled() {
            let _ = tx.send(StreamChunk::Cancelled).await;
            return Ok(());
        }

        match chunk_result {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                for line in text.lines() {
                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };
                    let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
                        continue;
                    };
                    // Check for message_stop event
                    if json["type"] == "message_stop" {
                        let _ = tx.send(StreamChunk::Done).await;
                        return Ok(());
                    }
                    // Extract content from content_block_delta events
                    if json["type"] == "content_block_delta"
                        && let Some(content) = json["delta"]["text"].as_str()
                        && !content.is_empty()
                    {
                        let _ = tx.send(StreamChunk::Text(content.to_string())).await;
                    }
                }
            }
            Err(e) => {
                let _ = tx
                    .send(StreamChunk::Error(format!("Stream error: {}", e)))
                    .await;
                return Err(AppError::Llm(format!("Stream error: {}", e)));
            }
        }
    }

    let _ = tx.send(StreamChunk::Done).await;
    Ok(())
}

/// Stream a response from Ollama local API
pub async fn stream_ollama(
    base_url: &str,
    model: &str,
    prompt: &str,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    let url = format!("{}/api/chat", base_url);
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": true
    });

    let client = create_streaming_client();
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("Ollama streaming request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Llm(format!("Ollama HTTP {}: {}", status, body)));
    }

    let mut stream = response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        if cancel_token.is_cancelled() {
            let _ = tx.send(StreamChunk::Cancelled).await;
            return Ok(());
        }

        match chunk_result {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                for line in text.lines() {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                        // Check if done
                        if json["done"].as_bool() == Some(true) {
                            let _ = tx.send(StreamChunk::Done).await;
                            return Ok(());
                        }
                        // Extract content from message
                        if let Some(content) = json["message"]["content"].as_str()
                            && !content.is_empty()
                        {
                            let _ = tx.send(StreamChunk::Text(content.to_string())).await;
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx
                    .send(StreamChunk::Error(format!("Stream error: {}", e)))
                    .await;
                return Err(AppError::Llm(format!("Stream error: {}", e)));
            }
        }
    }

    let _ = tx.send(StreamChunk::Done).await;
    Ok(())
}

/// Start a streaming LLM request based on config
pub async fn start_streaming(
    config: &AppConfig,
    prompt: &str,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    match config.llm.primary.as_str() {
        "openai" => {
            if let Some(c) = &config.llm.openai {
                stream_openai(
                    &c.api_key,
                    c.base_url
                        .as_deref()
                        .unwrap_or("https://api.openai.com"),
                    &c.model,
                    prompt,
                    tx,
                    cancel_token,
                )
                .await
            } else {
                // Echo fallback for streaming
                let _ = tx.send(StreamChunk::Text(format!("ECHO: {}", prompt))).await;
                let _ = tx.send(StreamChunk::Done).await;
                Ok(())
            }
        }
        "anthropic" => {
            if let Some(c) = &config.llm.anthropic {
                stream_anthropic(
                    &c.api_key,
                    c.base_url
                        .as_deref()
                        .unwrap_or("https://api.anthropic.com"),
                    &c.model,
                    prompt,
                    tx,
                    cancel_token,
                )
                .await
            } else {
                let _ = tx.send(StreamChunk::Text(format!("ECHO: {}", prompt))).await;
                let _ = tx.send(StreamChunk::Done).await;
                Ok(())
            }
        }
        "grok" => {
            // Grok uses OpenAI-compatible API
            if let Some(c) = &config.llm.grok {
                stream_openai(
                    &c.api_key,
                    c.base_url.as_deref().unwrap_or("https://api.x.ai"),
                    &c.model,
                    prompt,
                    tx,
                    cancel_token,
                )
                .await
            } else {
                let _ = tx.send(StreamChunk::Text(format!("ECHO: {}", prompt))).await;
                let _ = tx.send(StreamChunk::Done).await;
                Ok(())
            }
        }
        "ollama" => {
            if let Some(c) = &config.llm.ollama {
                stream_ollama(&c.base_url, &c.model, prompt, tx, cancel_token).await
            } else {
                let _ = tx.send(StreamChunk::Text(format!("ECHO: {}", prompt))).await;
                let _ = tx.send(StreamChunk::Done).await;
                Ok(())
            }
        }
        _ => {
            // Echo fallback
            let _ = tx.send(StreamChunk::Text(format!("ECHO: {}", prompt))).await;
            let _ = tx.send(StreamChunk::Done).await;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancellation_token() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn test_stream_chunk_types() {
        let (tx, mut rx) = mpsc::channel(10);

        tx.send(StreamChunk::Text("Hello".to_string()))
            .await
            .unwrap();
        tx.send(StreamChunk::Done).await.unwrap();

        if let Some(StreamChunk::Text(text)) = rx.recv().await {
            assert_eq!(text, "Hello");
        } else {
            panic!("Expected Text chunk");
        }

        if let Some(StreamChunk::Done) = rx.recv().await {
            // OK
        } else {
            panic!("Expected Done chunk");
        }
    }
}

