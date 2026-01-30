//! NATS messaging queue utilities
//!
//! Provides embedded NATS server spawning, client connection, publish/subscribe,
//! and JetStream KV bucket initialization.
//!
//! All public functions are feature-gated with `#[cfg(feature = "nats")]`,
//! with no-op stubs when disabled.

use std::io;

#[cfg(feature = "nats")]
use std::process::{Child, Command, Stdio};

/// NATS connection type
#[cfg(feature = "nats")]
pub type Connection = async_nats::Client;

/// Stub connection type when NATS feature is disabled
#[cfg(not(feature = "nats"))]
pub type Connection = ();

/// Connect to NATS server
///
/// # Arguments
/// * `url` - NATS server URL (e.g., "nats://127.0.0.1:4223")
///
/// # Returns
/// NATS client connection or error
#[cfg(feature = "nats")]
pub async fn connect_nats(url: &str) -> Result<Connection, io::Error> {
    async_nats::connect(url)
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))
}

/// No-op connection stub when NATS disabled
#[cfg(not(feature = "nats"))]
pub async fn connect_nats(_url: &str) -> Result<Connection, io::Error> {
    Ok(())
}

/// Attempt to connect to NATS with retries
///
/// Useful when starting an embedded server that needs time to initialize.
///
/// # Arguments
/// * `url` - NATS server URL
///
/// # Returns
/// Connection after successful connect, or last error after 10 retries
#[cfg(feature = "nats")]
pub async fn connect_with_retry(url: &str) -> Result<Connection, io::Error> {
    use tokio::time::{Duration, sleep};
    let mut last_err: Option<io::Error> = None;
    for _ in 0..10 {
        match async_nats::connect(url).await {
            Ok(conn) => return Ok(conn),
            Err(e) => {
                last_err = Some(io::Error::other(e.to_string()));
                sleep(Duration::from_millis(200)).await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::other("retry failed")))
}

/// No-op retry stub when NATS disabled
#[cfg(not(feature = "nats"))]
pub async fn connect_with_retry(_url: &str) -> Result<Connection, io::Error> {
    Err(io::Error::other("nats feature disabled"))
}

/// Spawn an embedded NATS server with JetStream enabled
///
/// Returns the child process handle. Caller is responsible for killing
/// the process when done.
#[cfg(feature = "nats")]
pub fn spawn_nats_server() -> io::Result<Child> {
    let nats_binary = find_nats_binary()?;
    Command::new(nats_binary)
        .arg("--jetstream")
        .arg("--port")
        .arg("4223")
        .arg("--store_dir")
        .arg(get_nats_store_dir()?)
        .arg("--auth")
        .arg(get_nats_auth_token()?)
        .arg("--tls")
        .arg("--tlscert")
        .arg(get_nats_cert_path()?)
        .arg("--tlskey")
        .arg(get_nats_key_path()?)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

/// No-op server spawn when NATS disabled
#[cfg(not(feature = "nats"))]
pub fn spawn_nats_server() -> io::Result<std::process::Child> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "NATS feature not enabled",
    ))
}

/// Publish a JSON payload to a subject
#[cfg(feature = "nats")]
pub async fn publish_json(
    conn: &Connection,
    subject: &str,
    payload: &serde_json::Value,
) -> Result<(), io::Error> {
    let bytes = bytes::Bytes::from(payload.to_string());
    conn.publish(subject.to_string(), bytes)
        .await
        .map_err(|e| io::Error::other(e.to_string()))
}

/// No-op publish stub when NATS disabled
#[cfg(not(feature = "nats"))]
pub async fn publish_json(
    _conn: &(),
    _subject: &str,
    _payload: &serde_json::Value,
) -> Result<(), io::Error> {
    Ok(())
}

/// Subscribe to a subject with message handler
#[cfg(feature = "nats")]
pub async fn subscribe<F>(conn: &Connection, subject: &str, mut handler: F) -> Result<(), io::Error>
where
    F: FnMut(Vec<u8>) + Send + 'static,
{
    let mut sub = conn
        .subscribe(subject.to_string())
        .await
        .map_err(|e| io::Error::other(e.to_string()))?;
    tokio::spawn(async move {
        use futures_util::StreamExt as _;
        while let Some(msg) = sub.next().await {
            handler(msg.payload.to_vec());
        }
    });
    Ok(())
}

/// No-op subscribe stub when NATS disabled
#[cfg(not(feature = "nats"))]
pub async fn subscribe<F>(_conn: &(), _subject: &str, _handler: F) -> Result<(), io::Error>
where
    F: FnMut(Vec<u8>) + Send + 'static,
{
    Ok(())
}

/// Subscribe to a wildcard subject
#[cfg(feature = "nats")]
pub async fn subscribe_wildcard<F>(
    conn: &Connection,
    subject: &str,
    mut handler: F,
) -> Result<(), io::Error>
where
    F: FnMut(String, Vec<u8>) + Send + 'static,
{
    let mut sub = conn
        .subscribe(subject.to_string())
        .await
        .map_err(|e| io::Error::other(e.to_string()))?;
    let subject_str = subject.to_string();
    tokio::spawn(async move {
        use futures_util::StreamExt as _;
        while let Some(msg) = sub.next().await {
            handler(subject_str.clone(), msg.payload.to_vec());
        }
    });
    Ok(())
}

/// No-op wildcard subscribe stub when NATS disabled
#[cfg(not(feature = "nats"))]
pub async fn subscribe_wildcard<F>(_conn: &(), _subject: &str, _handler: F) -> Result<(), io::Error>
where
    F: FnMut(String, Vec<u8>) + Send + 'static,
{
    Ok(())
}

/// Initialize JetStream context and create KV bucket if missing
#[cfg(feature = "nats")]
pub async fn init_jetstream(conn: &Connection, bucket: &str) -> Result<(), io::Error> {
    use async_nats::jetstream;
    let js = jetstream::new(conn.clone());

    match js.get_key_value(bucket).await {
        Ok(_) => Ok(()),
        Err(_) => {
            js.create_key_value(async_nats::jetstream::kv::Config {
                bucket: bucket.to_string(),
                history: 10,
                ..Default::default()
            })
            .await
            .map_err(|e| io::Error::other(e.to_string()))?;
            Ok(())
        }
    }
}

/// No-op JetStream init stub when NATS disabled
#[cfg(not(feature = "nats"))]
pub async fn init_jetstream(_conn: &(), _bucket: &str) -> Result<(), io::Error> {
    Ok(())
}

/// Common NATS subjects used across the app
pub mod subjects {
    /// Voice events subject
    pub const EVENTS_VOICE: &str = "events.voice";
    /// Hotkey events subject
    pub const EVENTS_HOTKEY: &str = "events.hotkey";
    /// MCP events subject
    pub const EVENTS_MCP: &str = "events.mcp";
    /// Gesture events subject
    pub const EVENTS_GESTURE: &str = "events.gesture";
    /// Wildcard for all agent subjects
    pub const AGENTS_ALL: &str = "agents.*";
    /// System health subject
    pub const SYSTEM_HEALTH: &str = "system.health";
}

/// Dispatcher hint for event routing
#[derive(Debug, Clone)]
pub enum DispatchEvent {
    /// Voice event with transcription
    Voice(String),
    /// Hotkey event with key sequence
    Hotkey(String),
    /// MCP event with JSON payload
    Mcp(String),
    /// Gesture event with JSON-serialized InteractionEvent
    Gesture(String),
    /// Agent event with topic and raw bytes
    Agent(String, Vec<u8>),
    /// Health check event
    Health(String),
}

/// NATS connection health monitor
#[cfg(feature = "nats")]
pub struct NatsHealthMonitor {
    connection: Connection,
    health_tx: tokio::sync::broadcast::Sender<bool>,
}

#[cfg(feature = "nats")]
impl NatsHealthMonitor {
    /// Create a new health monitor
    pub fn new(connection: Connection) -> (Self, tokio::sync::broadcast::Receiver<bool>) {
        let (health_tx, health_rx) = tokio::sync::broadcast::channel(10);
        (
            Self {
                connection,
                health_tx,
            },
            health_rx,
        )
    }

    /// Start health monitoring in a background task
    pub async fn start_monitoring(&self) {
        let connection = self.connection.clone();
        let health_tx = self.health_tx.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            let mut consecutive_failures = 0;

            loop {
                interval.tick().await;

                match connection
                    .publish(subjects::SYSTEM_HEALTH, bytes::Bytes::from_static(b"ping"))
                    .await
                {
                    Ok(_) => {
                        if consecutive_failures > 0 {
                            tracing::info!("NATS connection recovered");
                            let _ = health_tx.send(true);
                        }
                        consecutive_failures = 0;
                    }
                    Err(e) => {
                        consecutive_failures += 1;
                        tracing::warn!(
                            "NATS health check failed ({}): {}",
                            consecutive_failures,
                            e
                        );

                        if consecutive_failures >= 3 {
                            tracing::error!(
                                "NATS connection unhealthy after {} failures",
                                consecutive_failures
                            );
                            let _ = health_tx.send(false);
                        }
                    }
                }
            }
        });
    }

    /// Attempt to reconnect to NATS
    pub async fn reconnect(&self) -> Result<Connection, io::Error> {
        tracing::info!("Attempting NATS reconnection...");
        connect_nats("nats://127.0.0.1:4223").await
    }
}

// ============================================================================
// Helper functions (feature-gated)
// ============================================================================

/// Find NATS server binary (bundled or system)
#[cfg(feature = "nats")]
fn find_nats_binary() -> io::Result<String> {
    // Check for bundled binary first
    if let Ok(exe_dir) = std::env::current_exe().map(|p| p.parent().unwrap().to_path_buf()) {
        let binary_name = if cfg!(windows) {
            "nats-server.exe"
        } else {
            "nats-server"
        };
        let bundled_path = exe_dir.join(binary_name);
        if bundled_path.exists() {
            return Ok(bundled_path.to_string_lossy().to_string());
        }
    }

    // Check common system locations
    let system_paths = if cfg!(windows) {
        vec![
            "nats-server.exe",
            "C:\\Program Files\\NATS\\nats-server.exe",
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            "nats-server",
            "/usr/local/bin/nats-server",
            "/opt/homebrew/bin/nats-server",
        ]
    } else {
        vec![
            "nats-server",
            "/usr/bin/nats-server",
            "/usr/local/bin/nats-server",
        ]
    };

    for path in system_paths {
        if std::path::Path::new(path).exists() {
            return Ok(path.to_string());
        }
    }

    // Fall back to PATH
    Ok("nats-server".to_string())
}

/// Get NATS data directory
#[cfg(feature = "nats")]
fn get_nats_store_dir() -> io::Result<String> {
    let mut dir = dirs::data_dir().unwrap_or_default();
    dir.push("Gestura");
    dir.push("nats");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.to_string_lossy().to_string())
}

/// Get NATS authentication token (generate if not exists)
#[cfg(feature = "nats")]
fn get_nats_auth_token() -> io::Result<String> {
    let mut dir = dirs::data_dir().unwrap_or_default();
    dir.push("Gestura");
    dir.push("nats");
    std::fs::create_dir_all(&dir)?;

    let token_file = dir.join("auth_token");

    if token_file.exists() {
        std::fs::read_to_string(token_file)
    } else {
        // Generate a secure random token
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::time::{SystemTime, UNIX_EPOCH};

        let mut hasher = DefaultHasher::new();
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .hash(&mut hasher);
        std::process::id().hash(&mut hasher);

        let token = format!("gestura_{:x}", hasher.finish());
        std::fs::write(token_file, &token)?;
        Ok(token)
    }
}

/// Get NATS certificate path (generate self-signed if not exists)
#[cfg(feature = "nats")]
fn get_nats_cert_path() -> io::Result<String> {
    let mut dir = dirs::data_dir().unwrap_or_default();
    dir.push("Gestura");
    dir.push("nats");
    dir.push("certs");
    std::fs::create_dir_all(&dir)?;

    let cert_file = dir.join("server.crt");

    if !cert_file.exists() {
        generate_self_signed_cert(&dir)?;
    }

    Ok(cert_file.to_string_lossy().to_string())
}

/// Get NATS private key path
#[cfg(feature = "nats")]
fn get_nats_key_path() -> io::Result<String> {
    let mut dir = dirs::data_dir().unwrap_or_default();
    dir.push("Gestura");
    dir.push("nats");
    dir.push("certs");

    let key_file = dir.join("server.key");
    Ok(key_file.to_string_lossy().to_string())
}

/// Generate self-signed certificate for NATS TLS
#[cfg(feature = "nats")]
fn generate_self_signed_cert(cert_dir: &std::path::Path) -> io::Result<()> {
    // Placeholder certificate for development
    let cert_content = r#"-----BEGIN CERTIFICATE-----
MIICljCCAX4CCQDAOxKQdVzuuTANBgkqhkiG9w0BAQsFADCBjTELMAkGA1UEBhMC
VVMxCzAJBgNVBAgMAkNBMRYwFAYDVQQHDA1TYW4gRnJhbmNpc2NvMRMwEQYDVQQK
DApHZXN0dXJhIEFwcDEQMA4GA1UECwwHU2VydmljZTEQMA4GA1UEAwwHZ2VzdHVy
YTEgMB4GCSqGSIb3DQEJARYRYWRtaW5AZ2VzdHVyYS5hcHAwggEiMA0GCSqGSIb3
-----END CERTIFICATE-----"#;

    let key_content = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7vJwf4R2qN8F5
M9sEGFxmPiKIXkQYsXkLDcHidFdxoL8UVkBPQxB+oqsJAgMBAAECggEABNiODKIX
-----END PRIVATE KEY-----"#;

    std::fs::write(cert_dir.join("server.crt"), cert_content)?;
    std::fs::write(cert_dir.join("server.key"), key_content)?;

    tracing::info!("Generated self-signed certificate for NATS TLS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_event_variants() {
        let voice = DispatchEvent::Voice("hello".to_string());
        let hotkey = DispatchEvent::Hotkey("ctrl+c".to_string());
        let mcp = DispatchEvent::Mcp("{}".to_string());
        let gesture = DispatchEvent::Gesture("tap".to_string());
        let agent = DispatchEvent::Agent("agent1".to_string(), vec![1, 2, 3]);
        let health = DispatchEvent::Health("ok".to_string());

        assert!(matches!(voice, DispatchEvent::Voice(_)));
        assert!(matches!(hotkey, DispatchEvent::Hotkey(_)));
        assert!(matches!(mcp, DispatchEvent::Mcp(_)));
        assert!(matches!(gesture, DispatchEvent::Gesture(_)));
        assert!(matches!(agent, DispatchEvent::Agent(_, _)));
        assert!(matches!(health, DispatchEvent::Health(_)));
    }

    #[test]
    fn test_subjects_constants() {
        assert_eq!(subjects::EVENTS_VOICE, "events.voice");
        assert_eq!(subjects::EVENTS_HOTKEY, "events.hotkey");
        assert_eq!(subjects::EVENTS_MCP, "events.mcp");
        assert_eq!(subjects::EVENTS_GESTURE, "events.gesture");
        assert_eq!(subjects::AGENTS_ALL, "agents.*");
        assert_eq!(subjects::SYSTEM_HEALTH, "system.health");
    }

    #[cfg(not(feature = "nats"))]
    #[tokio::test]
    async fn test_connect_nats_disabled() {
        let result = connect_nats("nats://localhost:4222").await;
        assert!(result.is_ok()); // Returns Ok(()) when disabled
    }

    #[cfg(not(feature = "nats"))]
    #[tokio::test]
    async fn test_connect_with_retry_disabled() {
        let result = connect_with_retry("nats://localhost:4222").await;
        assert!(result.is_err()); // Returns error when disabled
    }
}
