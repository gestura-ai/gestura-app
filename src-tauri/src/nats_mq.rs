//! Embedded NATS client utilities (Stage 1/3)
//! - Provides spawn for `nats-server --jetstream` as a child process
//! - Connects a client to NATS and exposes simple publish/subscribe helpers

#[cfg(feature = "nats")]
use std::process::{Child, Command, Stdio};

use std::io;

#[cfg(feature = "nats")]
pub type Connection = async_nats::Client;

#[cfg(not(feature = "nats"))]
pub type Connection = ();

/// Connect to NATS server
#[cfg(feature = "nats")]
pub async fn connect_nats(url: &str) -> Result<Connection, std::io::Error> {
    async_nats::connect(url)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::ConnectionRefused, e))
}

#[cfg(not(feature = "nats"))]
pub async fn connect_nats(_url: &str) -> Result<Connection, std::io::Error> {
    Ok(())
}

/// Spawn an embedded NATS server with JetStream enabled.
/// Returns the child process handle.
#[cfg(feature = "nats")]
pub fn spawn_nats_server() -> std::io::Result<Child> {
    // Try bundled binary first, then PATH
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

/// Spawn NATS server (no-op when nats feature disabled)
#[cfg(not(feature = "nats"))]
pub fn spawn_nats_server() -> std::io::Result<std::process::Child> {
    // Return a dummy child process for compatibility
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "NATS feature not enabled",
    ))
}

/// Find NATS server binary (bundled or system)
#[cfg(feature = "nats")]
fn find_nats_binary() -> std::io::Result<String> {
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
fn get_nats_store_dir() -> std::io::Result<String> {
    let mut dir = dirs::data_dir().unwrap_or_default();
    dir.push("Gestura");
    dir.push("nats");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.to_string_lossy().to_string())
}

/// Get NATS authentication token (generate if not exists)
#[cfg(feature = "nats")]
fn get_nats_auth_token() -> std::io::Result<String> {
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
fn get_nats_cert_path() -> std::io::Result<String> {
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
fn get_nats_key_path() -> std::io::Result<String> {
    let mut dir = dirs::data_dir().unwrap_or_default();
    dir.push("Gestura");
    dir.push("nats");
    dir.push("certs");

    let key_file = dir.join("server.key");
    Ok(key_file.to_string_lossy().to_string())
}

/// Generate self-signed certificate for NATS TLS
#[cfg(feature = "nats")]
fn generate_self_signed_cert(cert_dir: &std::path::Path) -> std::io::Result<()> {
    // For now, create dummy cert files
    // In production, use a proper certificate generation library
    let cert_content = r#"-----BEGIN CERTIFICATE-----
MIICljCCAX4CCQDAOxKQdVzuuTANBgkqhkiG9w0BAQsFADCBjTELMAkGA1UEBhMC
VVMxCzAJBgNVBAgMAkNBMRYwFAYDVQQHDA1TYW4gRnJhbmNpc2NvMRMwEQYDVQQK
DApHZXN0dXJhIEFwcDEQMA4GA1UECwwHU2VydmljZTEQMA4GA1UEAwwHZ2VzdHVy
YTEgMB4GCSqGSIb3DQEJARYRYWRtaW5AZ2VzdHVyYS5hcHAwHhcNMjQwMTAxMDAw
MDAwWhcNMjUwMTAxMDAwMDAwWjCBjTELMAkGA1UEBhMCVVMxCzAJBgNVBAgMAkNB
MRYwFAYDVQQHDA1TYW4gRnJhbmNpc2NvMRMwEQYDVQQKDApHZXN0dXJhIEFwcDEQ
MA4GA1UECwwHU2VydmljZTEQMA4GA1UEAwwHZ2VzdHVyYTEgMB4GCSqGSIb3DQEJ
ARYRYWRtaW5AZ2VzdHVyYS5hcHAwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEK
AoIBAQC7vJwf4R2qN8F5M9sEGFxmPiKIXkQYsXkLDcHidFdxoL8UVkBPQxB+oqsJ
-----END CERTIFICATE-----"#;

    let key_content = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7vJwf4R2qN8F5
M9sEGFxmPiKIXkQYsXkLDcHidFdxoL8UVkBPQxB+oqsJvJwf4R2qN8F5M9sEGFxm
PiKIXkQYsXkLDcHidFdxoL8UVkBPQxB+oqsJvJwf4R2qN8F5M9sEGFxmPiKIXkQY
sXkLDcHidFdxoL8UVkBPQxB+oqsJvJwf4R2qN8F5M9sEGFxmPiKIXkQYsXkLDcHi
dFdxoL8UVkBPQxB+oqsJvJwf4R2qN8F5M9sEGFxmPiKIXkQYsXkLDcHidFdxoL8U
VkBPQxB+oqsJAgMBAAECggEABNiODKIXkQYsXkLDcHidFdxoL8UVkBPQxB+oqsJ
-----END PRIVATE KEY-----"#;

    std::fs::write(cert_dir.join("server.crt"), cert_content)?;
    std::fs::write(cert_dir.join("server.key"), key_content)?;

    tracing::info!("Generated self-signed certificate for NATS TLS");
    Ok(())
}

/// Attempt to connect to NATS, retrying a few times while the server starts.
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

/// Fallback when the `nats` feature is disabled: returns an error on connect.
#[cfg(not(feature = "nats"))]
pub async fn connect_with_retry(_url: &str) -> Result<Connection, io::Error> {
    Err(io::Error::other("nats feature disabled"))
}

/// Publish a JSON payload to a subject.
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

/// Fallback publish that is a no-op when `nats` feature is disabled.
#[cfg(not(feature = "nats"))]
pub async fn publish_json(
    _conn: &(),
    _subject: &str,
    _payload: &serde_json::Value,
) -> Result<(), io::Error> {
    Ok(())
}

/// Subscribe to a subject and provide each message as bytes to a handler closure.
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
/// Initialize JetStream context and a KV bucket (create if missing).
#[cfg(feature = "nats")]
pub async fn init_jetstream(conn: &Connection, bucket: &str) -> Result<(), io::Error> {
    use async_nats::jetstream;
    let js = jetstream::new(conn.clone());

    // Create KV bucket if it doesn't exist
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

#[cfg(not(feature = "nats"))]
pub async fn init_jetstream(_conn: &(), _bucket: &str) -> Result<(), io::Error> {
    Ok(())
}

/// Fallback subscribe that is a no-op when `nats` feature is disabled.
#[cfg(not(feature = "nats"))]
pub async fn subscribe<F>(_conn: &(), _subject: &str, _handler: F) -> Result<(), io::Error>
where
    F: FnMut(Vec<u8>) + Send + 'static,
{
    Ok(())
}

/// Subscribe to a wildcard subject and forward raw bytes to a handler.
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

#[cfg(not(feature = "nats"))]
pub async fn subscribe_wildcard<F>(_conn: &(), _subject: &str, _handler: F) -> Result<(), io::Error>
where
    F: FnMut(String, Vec<u8>) + Send + 'static,
{
    Ok(())
}

/// Common subjects used across the app (JetStream suggested)
pub mod subjects {
    pub const EVENTS_VOICE: &str = "events.voice";
    pub const EVENTS_HOTKEY: &str = "events.hotkey";
    pub const EVENTS_MCP: &str = "events.mcp";
    pub const AGENTS_ALL: &str = "agents.*";
    pub const SYSTEM_HEALTH: &str = "system.health";
}

/// Dispatcher hint for event routing
#[derive(Debug, Clone)]
pub enum DispatchEvent {
    Voice(String),
    Hotkey(String),
    Mcp(String),
    Agent(String, Vec<u8>),
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

    /// Start health monitoring
    pub async fn start_monitoring(&self) {
        let connection = self.connection.clone();
        let health_tx = self.health_tx.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            let mut consecutive_failures = 0;

            loop {
                interval.tick().await;

                // Test connection with a simple publish
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

    /// Attempt to reconnect
    pub async fn reconnect(&self) -> Result<Connection, std::io::Error> {
        tracing::info!("Attempting NATS reconnection...");
        connect_nats("nats://127.0.0.1:4223").await
    }
}
