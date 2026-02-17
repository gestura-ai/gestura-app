//! Local IPC for routing the GUI global listen hotkey to a running CLI session.
//!
//! ## Why this exists
//! The GUI registers an OS-wide (global) hotkey via Tauri. When a user is
//! actively working in the terminal, pressing that hotkey should prefer the
//! active CLI session instead of starting a GUI listening session.
//!
//! We intentionally avoid NATS here: this is a lightweight, local-only,
//! best-effort mechanism.

use serde::{Deserialize, Serialize};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// Byte sent by the GUI to request the CLI toggles recording.
const MSG_TOGGLE_RECORDING: u8 = b'T';
/// Byte sent by the CLI to acknowledge receipt.
const MSG_ACK: u8 = b'K';

/// On-disk discovery record for the currently-running CLI hotkey server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliHotkeyEndpoint {
    /// Protocol version string.
    pub version: String,
    /// TCP port bound on 127.0.0.1.
    pub port: u16,
    /// PID of the CLI process that created the endpoint.
    pub pid: u32,
    /// Unix epoch millis when written.
    pub created_at_ms: u128,
}

impl CliHotkeyEndpoint {
    fn new(port: u16) -> Self {
        Self {
            version: "gestura-cli-hotkey-v1".to_string(),
            port,
            pid: std::process::id(),
            created_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        }
    }
}

/// Returns the default port file path used for CLI hotkey discovery.
///
/// We use the OS temp dir to avoid Unix socket path-length issues and because
/// the file is ephemeral (only meaningful while a CLI session is running).
pub fn default_cli_hotkey_port_file() -> PathBuf {
    let suffix = user_suffix();
    std::env::temp_dir().join(format!("gestura-cli-hotkey-{suffix}.json"))
}

fn user_suffix() -> String {
    #[cfg(unix)]
    {
        // Best-effort unique-per-user identifier.
        //
        // We intentionally avoid depending on `nix` user features here; env vars
        // are sufficient for a temp-dir discovery file suffix.
        std::env::var("UID")
            .or_else(|_| std::env::var("USER"))
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "default".to_string())
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>()
    }

    #[cfg(not(unix))]
    {
        std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "default".to_string())
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>()
    }
}

fn localhost_addr(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn write_port_file_atomic(path: &Path, endpoint: &CliHotkeyEndpoint) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(endpoint)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(&tmp, bytes)?;

    // Best-effort atomic replace.
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn read_port_file(path: &Path) -> io::Result<CliHotkeyEndpoint> {
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

/// Try to send a hotkey trigger to a running CLI session.
///
/// Returns:
/// - `Ok(true)` if a CLI server was found and acknowledged the request.
/// - `Ok(false)` if no CLI server is running (or endpoint is stale/unreachable).
/// - `Err(_)` for unexpected I/O errors (rare; callers typically treat as `false`).
pub async fn try_send_hotkey_trigger_to_cli(timeout: Duration) -> io::Result<bool> {
    try_send_hotkey_trigger_to_cli_with_file(&default_cli_hotkey_port_file(), timeout).await
}

/// Same as [`try_send_hotkey_trigger_to_cli`] but allows injecting a custom port file path.
pub async fn try_send_hotkey_trigger_to_cli_with_file(
    port_file: &Path,
    timeout: Duration,
) -> io::Result<bool> {
    let endpoint = match read_port_file(port_file) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };

    let addr = localhost_addr(endpoint.port);
    let mut stream = match tokio::time::timeout(timeout, TcpStream::connect(addr)).await {
        Ok(Ok(s)) => s,
        Ok(Err(_)) | Err(_) => {
            // Stale endpoint; remove so next press falls back quickly.
            let _ = std::fs::remove_file(port_file);
            return Ok(false);
        }
    };

    // Send request + wait for ack.
    stream.write_all(&[MSG_TOGGLE_RECORDING]).await?;
    stream.flush().await?;
    let mut ack = [0u8; 1];
    match tokio::time::timeout(timeout, stream.read_exact(&mut ack)).await {
        Ok(Ok(_)) if ack[0] == MSG_ACK => Ok(true),
        Ok(Ok(_)) => Ok(false),
        Ok(Err(e)) => Err(e),
        Err(_) => Ok(false),
    }
}

/// Guard that keeps the CLI hotkey server alive and removes the discovery file when dropped.
pub struct CliHotkeyServerGuard {
    port_file: PathBuf,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for CliHotkeyServerGuard {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.port_file);
    }
}

/// Start a local TCP server that accepts hotkey triggers and forwards them to the provided channel.
///
/// The server binds to `127.0.0.1:0` (ephemeral) and writes its port to a discovery file.
///
/// - `tx` receives `()` for each incoming trigger.
/// - `port_file` controls the discovery location; use [`default_cli_hotkey_port_file`]
///   for production.
pub async fn start_cli_hotkey_server(
    tx: mpsc::UnboundedSender<()>,
    port_file: PathBuf,
) -> io::Result<CliHotkeyServerGuard> {
    let listener = TcpListener::bind(localhost_addr(0)).await?;
    let port = listener.local_addr()?.port();

    write_port_file_atomic(&port_file, &CliHotkeyEndpoint::new(port))?;

    let task = tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };

            // Handle each connection in its own task so a slow peer doesn't block accept.
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut b = [0u8; 1];
                if sock.read_exact(&mut b).await.is_ok() && b[0] == MSG_TOGGLE_RECORDING {
                    // Ack first (so GUI can decide not to fall back to GUI listening).
                    let _ = sock.write_all(&[MSG_ACK]).await;
                    let _ = sock.flush().await;
                    let _ = tx.send(());
                }
            });
        }
    });

    Ok(CliHotkeyServerGuard { port_file, task })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn send_trigger_roundtrip() {
        let dir = tempdir().unwrap();
        let port_file = dir.path().join("endpoint.json");

        let (tx, mut rx) = mpsc::unbounded_channel::<()>();
        let _guard = start_cli_hotkey_server(tx, port_file.clone())
            .await
            .unwrap();

        let ok = try_send_hotkey_trigger_to_cli_with_file(&port_file, Duration::from_millis(250))
            .await
            .unwrap();
        assert!(ok);

        // Ensure the server forwarded a message.
        let got = tokio::time::timeout(Duration::from_millis(250), rx.recv())
            .await
            .unwrap();
        assert!(got.is_some());
    }
}
