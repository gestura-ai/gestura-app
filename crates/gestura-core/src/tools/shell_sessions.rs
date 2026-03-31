//! PTY-backed shell sessions for reusable agent execution and interactive terminals.
//!
//! This module manages two related session types:
//! - **automation sessions** used by the agent for reusable shell execution
//! - **interactive sessions** used by the GUI terminal manager for real typing

use crate::error::{AppError, Result};
use crate::streaming::{ShellOutputStream, ShellProcessState, ShellSessionState, StreamChunk};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::shell_streaming::{ShellRuntimeFailureKind, StreamingCommandResult};

/// Public summary for a managed PTY shell session.
#[derive(Debug, Clone)]
pub struct ShellSessionHandle {
    /// Stable identifier for the long-lived PTY shell session.
    pub shell_session_id: String,
    /// Best-effort tracked working directory for the PTY shell session.
    pub cwd: Option<String>,
}

/// Public metadata for a managed PTY shell session.
#[derive(Debug, Clone)]
pub struct ShellSessionMetadata {
    /// Stable identifier for the long-lived PTY shell session.
    pub shell_session_id: String,
    /// Best-effort tracked working directory for the PTY shell session.
    pub cwd: Option<String>,
    /// Whether the session was created for direct user interaction.
    pub interactive: bool,
    /// Whether the session is reserved for the user-facing terminal manager.
    pub user_managed: bool,
    /// Whether the session can currently be reused by the automation pool.
    pub available_for_reuse: bool,
}

/// Execution policy for PTY-backed shell commands.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShellExecutionOptions {
    /// Maximum wall-clock time allowed before the command is considered long-running.
    pub timeout_secs: Option<u64>,
    /// Whether shell activity may extend execution past `timeout_secs`.
    pub allow_long_running: bool,
    /// Maximum quiet period allowed before an activity-aware command is treated as stalled.
    pub stall_timeout_secs: Option<u64>,
}

mod imp {
    use super::*;
    use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
    use std::collections::HashMap;
    use std::io::{ErrorKind, Read, Write};
    use std::sync::{
        Arc, Mutex as StdMutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::sync::{Mutex, broadcast, mpsc};

    const DEFAULT_ROWS: u16 = 24;
    const DEFAULT_COLS: u16 = 120;
    const INTERRUPT_GRACE_SECS: u64 = 2;
    const STOP_ESCALATION_MILLIS: u64 = 300;
    const SESSION_EVENT_BUFFER: usize = 512;
    const DEFAULT_EXECUTION_TIMEOUT_SECS: u64 = 300;
    const MIN_STALL_TIMEOUT_SECS: u64 = 30;
    const MAX_STALL_TIMEOUT_SECS: u64 = 300;
    const MAX_QUIET_WAIT_CYCLES_WITHOUT_SIGNAL: u8 = 2;
    const SHELL_OUTPUT_SEND_TIMEOUT: Duration = Duration::from_millis(100);
    const STATUS_CHUNK_SEND_TIMEOUT: Duration = Duration::from_millis(100);
    const STALL_SIGNAL_TAIL_BYTES: usize = 4096;
    const SHELL_READY_TIMEOUT: Duration = Duration::from_secs(5);
    const PTY_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
    const PTY_KILL_TIMEOUT: Duration = Duration::from_secs(5);

    const INTERACTIVE_PROMPT_PATTERNS: &[&str] = &[
        "ok to proceed?",
        "need to install the following packages",
        "would you like to continue",
        "press enter to continue",
        "press any key to continue",
        "select an option",
        "(y/n)",
        "[y/n]",
        "yes/no",
        "enter password",
        "enter passphrase",
        "password:",
        "passphrase:",
    ];

    const ERROR_OUTPUT_PATTERNS: &[&str] = &[
        "command not found",
        "no such file or directory",
        "permission denied",
        "not recognized as an internal or external command",
        "is not recognized as an internal or external command",
        "npm err!",
        "traceback (most recent call last)",
        "syntax error",
        "fatal:",
        "panic:",
        "exception:",
    ];

    async fn send_shell_output_chunk_best_effort(
        tx: &mpsc::Sender<StreamChunk>,
        chunk: StreamChunk,
    ) {
        match tokio::time::timeout(SHELL_OUTPUT_SEND_TIMEOUT, tx.send(chunk)).await {
            Ok(Ok(())) | Ok(Err(_)) => {}
            Err(_) => {
                tracing::debug!(
                    timeout_ms = SHELL_OUTPUT_SEND_TIMEOUT.as_millis(),
                    "Dropping PTY shell output chunk because the stream receiver is not draining fast enough"
                );
            }
        }
    }

    async fn send_stream_chunk_best_effort(
        tx: &mpsc::Sender<StreamChunk>,
        chunk: StreamChunk,
        chunk_kind: &'static str,
    ) {
        match tokio::time::timeout(SHELL_OUTPUT_SEND_TIMEOUT, tx.send(chunk)).await {
            Ok(Ok(())) | Ok(Err(_)) => {}
            Err(_) => {
                tracing::debug!(
                    timeout_ms = SHELL_OUTPUT_SEND_TIMEOUT.as_millis(),
                    chunk_kind,
                    "Dropping PTY stream chunk because the receiver is not draining fast enough"
                );
            }
        }
    }

    async fn send_status_chunk_best_effort(tx: &mpsc::Sender<StreamChunk>, message: String) {
        match tokio::time::timeout(
            STATUS_CHUNK_SEND_TIMEOUT,
            tx.send(StreamChunk::Status { message }),
        )
        .await
        {
            Ok(Ok(())) | Ok(Err(_)) => {}
            Err(_) => {
                tracing::debug!(
                    timeout_ms = STATUS_CHUNK_SEND_TIMEOUT.as_millis(),
                    "Dropping PTY status chunk because the stream receiver is not draining fast enough"
                );
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum SessionMode {
        Automation,
        Interactive,
    }

    #[derive(Default)]
    struct ManagerState {
        sessions: HashMap<String, Arc<ShellSession>>,
        pools: HashMap<String, Vec<String>>,
        process_index: HashMap<String, String>,
    }

    pub(super) struct ShellSessionManager {
        state: Mutex<ManagerState>,
    }

    type SharedWriter = Arc<StdMutex<Box<dyn Write + Send>>>;

    struct ShellSession {
        shell_session_id: String,
        pool_key: String,
        mode: SessionMode,
        master: Mutex<Box<dyn MasterPty + Send>>,
        writer: SharedWriter,
        command_lock: Mutex<()>,
        active_sender: Arc<StdMutex<Option<mpsc::UnboundedSender<String>>>>,
        event_tx: broadcast::Sender<StreamChunk>,
        killer: Arc<StdMutex<Box<dyn ChildKiller + Send + Sync>>>,
        closed: Arc<AtomicBool>,
        claimed_by_user: Arc<AtomicBool>,
        user_stop_requested: Arc<AtomicBool>,
        state: Arc<StdMutex<ShellSessionState>>,
        working_directory: Arc<StdMutex<Option<String>>>,
        active_process_id: Arc<StdMutex<Option<String>>>,
        active_command: Arc<StdMutex<Option<String>>>,
    }

    struct ParsedChunk {
        output: String,
        exit_code: Option<i32>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StallSignal {
        None,
        InteractivePrompt,
        ErrorOutput,
    }

    impl StallSignal {
        fn runtime_failure_kind(self) -> Option<ShellRuntimeFailureKind> {
            match self {
                Self::None => None,
                Self::InteractivePrompt => Some(ShellRuntimeFailureKind::WaitingForInput),
                Self::ErrorOutput => Some(ShellRuntimeFailureKind::ErrorOutput),
            }
        }

        fn status_message(self, shell_session_id: &str, command: &str) -> String {
            match self {
                Self::None => format!(
                    "Shell runtime status: session `{shell_session_id}` saw no prompt or error indicator during a quiet period for `{command}` and will continue waiting."
                ),
                Self::InteractivePrompt => format!(
                    "Shell runtime status: session `{shell_session_id}` classified quiet command `{command}` as waiting_for_input and is interrupting it."
                ),
                Self::ErrorOutput => format!(
                    "Shell runtime status: session `{shell_session_id}` classified quiet command `{command}` as error_output and is interrupting it."
                ),
            }
        }
    }

    struct CommandCompletion {
        process_id: String,
        stdout: String,
        exit_code: i32,
        process_state: ShellProcessState,
        duration_ms: u64,
        session_state: ShellSessionState,
        failure_kind: Option<ShellRuntimeFailureKind>,
    }

    struct SessionOutputParser {
        pending: String,
        started: bool,
        start_marker: String,
        done_prefix: String,
    }

    static MANAGER: OnceLock<ShellSessionManager> = OnceLock::new();

    fn manager() -> &'static ShellSessionManager {
        MANAGER.get_or_init(|| ShellSessionManager {
            state: Mutex::new(ManagerState::default()),
        })
    }

    pub(super) async fn create_session(
        pool_key: &str,
        initial_cwd: Option<&str>,
        tx: Option<mpsc::Sender<StreamChunk>>,
    ) -> Result<ShellSessionHandle> {
        let session = spawn_session(pool_key, initial_cwd, SessionMode::Interactive).await?;
        manager().insert_session(session.clone()).await;
        session.set_state(ShellSessionState::Idle)?;
        if let Some(tx) = tx {
            session.subscribe(tx);
        }
        session.emit_session_lifecycle();
        Ok(session.handle())
    }

    #[allow(dead_code)]
    pub(super) async fn execute_in_session(
        pool_key: &str,
        initial_cwd: Option<&str>,
        command: &str,
        command_cwd: Option<&str>,
        timeout_secs: Option<u64>,
        tx: mpsc::Sender<StreamChunk>,
    ) -> Result<StreamingCommandResult> {
        execute_in_session_with_options(
            pool_key,
            initial_cwd,
            command,
            command_cwd,
            ShellExecutionOptions {
                timeout_secs,
                ..ShellExecutionOptions::default()
            },
            tx,
        )
        .await
    }

    pub(super) async fn execute_in_session_with_options(
        pool_key: &str,
        initial_cwd: Option<&str>,
        command: &str,
        command_cwd: Option<&str>,
        options: ShellExecutionOptions,
        tx: mpsc::Sender<StreamChunk>,
    ) -> Result<StreamingCommandResult> {
        let session = acquire_execution_session(pool_key, initial_cwd).await?;
        session.execute(command, command_cwd, options, tx).await
    }

    fn default_stall_timeout_secs(timeout_secs: u64) -> u64 {
        timeout_secs.clamp(MIN_STALL_TIMEOUT_SECS, MAX_STALL_TIMEOUT_SECS)
    }

    pub(super) async fn stop_process(process_id: &str) -> Result<Option<ShellSessionHandle>> {
        let session = manager().find_session_by_process(process_id).await;
        if let Some(session) = session {
            session.request_stop().await?;
            return Ok(Some(session.handle()));
        }
        Ok(None)
    }

    pub(super) async fn stop_session(shell_session_id: &str) -> Result<Option<ShellSessionHandle>> {
        let session = manager().remove_session(shell_session_id).await;
        if let Some(session) = session {
            session.user_stop_requested.store(true, Ordering::SeqCst);
            session.set_state(ShellSessionState::Stopping)?;
            session.emit_session_lifecycle();
            session.terminate().await?;
            session.set_active_command(None, None)?;
            session.set_state(ShellSessionState::Stopped)?;
            session.emit_session_lifecycle();
            return Ok(Some(session.handle()));
        }
        Ok(None)
    }

    pub(super) async fn shutdown_session(pool_key: &str) -> Result<()> {
        let sessions = manager().remove_pool(pool_key).await;
        for session in sessions {
            session.user_stop_requested.store(true, Ordering::SeqCst);
            session.set_state(ShellSessionState::Stopping)?;
            session.emit_session_lifecycle();
            session.terminate().await?;
            session.set_active_command(None, None)?;
            session.set_state(ShellSessionState::Stopped)?;
            session.emit_session_lifecycle();
        }
        Ok(())
    }

    pub(super) async fn send_input(shell_session_id: &str, data: &str) -> Result<()> {
        let Some(session) = manager().find_session(shell_session_id).await else {
            return Err(AppError::Session(format!(
                "unknown shell session: {shell_session_id}"
            )));
        };
        session.send_input(data).await
    }

    pub(super) async fn resize_session(shell_session_id: &str, cols: u16, rows: u16) -> Result<()> {
        let Some(session) = manager().find_session(shell_session_id).await else {
            return Err(AppError::Session(format!(
                "unknown shell session: {shell_session_id}"
            )));
        };
        session.resize(cols, rows).await
    }

    pub(super) async fn describe_session(
        shell_session_id: &str,
    ) -> Result<Option<ShellSessionMetadata>> {
        Ok(manager()
            .find_session(shell_session_id)
            .await
            .map(|session| session.metadata()))
    }

    pub(super) async fn claim_session(
        shell_session_id: &str,
    ) -> Result<Option<ShellSessionMetadata>> {
        let Some(session) = manager().find_session(shell_session_id).await else {
            return Ok(None);
        };

        session.claim_for_user();
        Ok(Some(session.metadata()))
    }

    pub(super) async fn subscribe_session(
        shell_session_id: &str,
        tx: mpsc::Sender<StreamChunk>,
    ) -> Result<Option<ShellSessionMetadata>> {
        let Some(session) = manager().find_session(shell_session_id).await else {
            return Ok(None);
        };

        session.subscribe(tx);
        Ok(Some(session.metadata()))
    }

    async fn spawn_session(
        pool_key: &str,
        initial_cwd: Option<&str>,
        mode: SessionMode,
    ) -> Result<Arc<ShellSession>> {
        let pool_key_owned = pool_key.to_string();
        let initial_cwd_owned = initial_cwd.map(ToOwned::to_owned);
        tokio::task::spawn_blocking(move || {
            ShellSession::spawn(&pool_key_owned, initial_cwd_owned.as_deref(), mode)
        })
        .await
        .map_err(|error| AppError::Session(format!("failed to spawn PTY session task: {error}")))?
    }

    async fn acquire_execution_session(
        pool_key: &str,
        initial_cwd: Option<&str>,
    ) -> Result<Arc<ShellSession>> {
        if let Some(session) = manager().acquire_idle_session(pool_key).await? {
            return Ok(session);
        }

        let session = spawn_session(pool_key, initial_cwd, SessionMode::Automation).await?;
        manager().insert_session(session.clone()).await;
        session.set_state(ShellSessionState::Idle)?;
        session.mark_busy_if_idle()?;
        Ok(session)
    }

    impl ShellSessionManager {
        async fn insert_session(&self, session: Arc<ShellSession>) {
            let mut state = self.state.lock().await;
            let shell_session_id = session.shell_session_id.clone();
            state
                .pools
                .entry(session.pool_key.clone())
                .or_default()
                .push(shell_session_id.clone());
            state.sessions.insert(shell_session_id, session);
        }

        async fn acquire_idle_session(&self, pool_key: &str) -> Result<Option<Arc<ShellSession>>> {
            let mut state = self.state.lock().await;
            prune_pool_locked(&mut state, pool_key);
            let session_ids = state.pools.get(pool_key).cloned().unwrap_or_default();
            for shell_session_id in session_ids {
                if let Some(session) = state.sessions.get(&shell_session_id).cloned()
                    && session.mark_busy_if_idle()?
                {
                    return Ok(Some(session));
                }
            }
            Ok(None)
        }

        async fn register_process(&self, process_id: String, shell_session_id: String) {
            let mut state = self.state.lock().await;
            state.process_index.insert(process_id, shell_session_id);
        }

        async fn unregister_process(&self, process_id: &str) {
            let mut state = self.state.lock().await;
            state.process_index.remove(process_id);
        }

        async fn find_session(&self, shell_session_id: &str) -> Option<Arc<ShellSession>> {
            let state = self.state.lock().await;
            state.sessions.get(shell_session_id).cloned()
        }

        async fn find_session_by_process(&self, process_id: &str) -> Option<Arc<ShellSession>> {
            let state = self.state.lock().await;
            let shell_session_id = state.process_index.get(process_id)?.clone();
            state.sessions.get(&shell_session_id).cloned()
        }

        async fn remove_session(&self, shell_session_id: &str) -> Option<Arc<ShellSession>> {
            let mut state = self.state.lock().await;
            let session = state.sessions.remove(shell_session_id)?;
            state.process_index.retain(|_, sid| sid != shell_session_id);
            if let Some(pool) = state.pools.get_mut(&session.pool_key) {
                pool.retain(|sid| sid != shell_session_id);
                if pool.is_empty() {
                    state.pools.remove(&session.pool_key);
                }
            }
            Some(session)
        }

        async fn remove_pool(&self, pool_key: &str) -> Vec<Arc<ShellSession>> {
            let mut state = self.state.lock().await;
            let session_ids = state.pools.remove(pool_key).unwrap_or_default();
            let mut removed = Vec::with_capacity(session_ids.len());
            for shell_session_id in session_ids {
                if let Some(session) = state.sessions.remove(&shell_session_id) {
                    state
                        .process_index
                        .retain(|_, sid| sid != &shell_session_id);
                    removed.push(session);
                }
            }
            removed
        }
    }

    impl ShellSession {
        fn spawn(
            pool_key: &str,
            initial_cwd: Option<&str>,
            mode: SessionMode,
        ) -> Result<Arc<Self>> {
            let pty_system = native_pty_system();
            let pair = pty_system
                .openpty(PtySize {
                    rows: DEFAULT_ROWS,
                    cols: DEFAULT_COLS,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|error| AppError::Session(format!("failed to open PTY: {error:#}")))?;

            let mut builder = build_shell_command(mode);
            if let Some(dir) = initial_cwd {
                builder.cwd(dir);
            }
            builder.env("TERM", "xterm-256color");
            builder.env("GESTURA_PTY", "1");

            let child = pair.slave.spawn_command(builder).map_err(|error| {
                AppError::Session(format!("failed to spawn PTY shell: {error:#}"))
            })?;

            let reader = pair.master.try_clone_reader().map_err(|error| {
                AppError::Session(format!("failed to clone PTY reader: {error:#}"))
            })?;
            let mut writer = pair.master.take_writer().map_err(|error| {
                AppError::Session(format!("failed to take PTY writer: {error:#}"))
            })?;

            let ready_marker = matches!(mode, SessionMode::Automation)
                .then(|| format!("__GESTURA_READY_{}__", uuid::Uuid::new_v4()));
            let (ready_tx, ready_rx) = if ready_marker.is_some() {
                let (tx, rx) = std::sync::mpsc::channel();
                (Some(tx), Some(rx))
            } else {
                (None, None)
            };

            let shell_session_id = format!("shell-{}", uuid::Uuid::new_v4());
            let active_sender = Arc::new(StdMutex::new(None));
            let active_process_id = Arc::new(StdMutex::new(None));
            let closed = Arc::new(AtomicBool::new(false));
            let claimed_by_user = Arc::new(AtomicBool::new(false));
            let killer = Arc::new(StdMutex::new(child.clone_killer()));
            let (event_tx, _) = broadcast::channel(SESSION_EVENT_BUFFER);

            spawn_reader_loop(
                reader,
                ReaderLoopContext {
                    active_sender: active_sender.clone(),
                    active_process_id: active_process_id.clone(),
                    closed: closed.clone(),
                    claimed_by_user: claimed_by_user.clone(),
                    event_tx: event_tx.clone(),
                    shell_session_id: shell_session_id.clone(),
                    emit_raw_output: matches!(mode, SessionMode::Interactive),
                    ready_marker: ready_marker.clone(),
                    ready_tx,
                },
            );
            spawn_wait_loop(child, closed.clone(), shell_session_id.clone());

            prepare_shell(mode, &mut writer, ready_marker.as_deref())?;

            if let Some(ready_rx) = ready_rx {
                match ready_rx.recv_timeout(SHELL_READY_TIMEOUT) {
                    Ok(Ok(())) => {}
                    Ok(Err(message)) => {
                        closed.store(true, Ordering::SeqCst);
                        return Err(AppError::Session(message));
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        closed.store(true, Ordering::SeqCst);
                        return Err(AppError::Session(
                            "timed out waiting for PTY shell initialization".to_string(),
                        ));
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        closed.store(true, Ordering::SeqCst);
                        return Err(AppError::Session(
                            "PTY shell initialization watcher disconnected unexpectedly"
                                .to_string(),
                        ));
                    }
                }
            }

            Ok(Arc::new(Self {
                shell_session_id,
                pool_key: pool_key.to_string(),
                mode,
                master: Mutex::new(pair.master),
                writer: Arc::new(StdMutex::new(writer)),
                command_lock: Mutex::new(()),
                active_sender,
                event_tx,
                killer,
                closed,
                claimed_by_user,
                user_stop_requested: Arc::new(AtomicBool::new(false)),
                state: Arc::new(StdMutex::new(ShellSessionState::Starting)),
                working_directory: Arc::new(StdMutex::new(initial_cwd.map(ToOwned::to_owned))),
                active_process_id,
                active_command: Arc::new(StdMutex::new(None)),
            }))
        }

        fn handle(&self) -> ShellSessionHandle {
            ShellSessionHandle {
                shell_session_id: self.shell_session_id.clone(),
                cwd: self.current_working_directory(),
            }
        }

        fn metadata(&self) -> ShellSessionMetadata {
            ShellSessionMetadata {
                shell_session_id: self.shell_session_id.clone(),
                cwd: self.current_working_directory(),
                interactive: true,
                user_managed: self.is_user_managed(),
                available_for_reuse: self.is_available_for_reuse(),
            }
        }

        fn is_interactive(&self) -> bool {
            matches!(self.mode, SessionMode::Interactive)
        }

        fn is_user_managed(&self) -> bool {
            self.is_interactive() || self.claimed_by_user.load(Ordering::SeqCst)
        }

        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }

        fn current_working_directory(&self) -> Option<String> {
            self.working_directory
                .lock()
                .ok()
                .and_then(|guard| guard.clone())
        }

        fn current_active_process_id(&self) -> Option<String> {
            self.active_process_id
                .lock()
                .ok()
                .and_then(|guard| guard.clone())
        }

        fn current_active_command(&self) -> Option<String> {
            self.active_command
                .lock()
                .ok()
                .and_then(|guard| guard.clone())
        }

        fn state_value(&self) -> ShellSessionState {
            self.state
                .lock()
                .ok()
                .map(|guard| *guard)
                .unwrap_or(ShellSessionState::Failed)
        }

        fn is_available_for_reuse(&self) -> bool {
            matches!(self.mode, SessionMode::Automation)
                && !self.is_user_managed()
                && matches!(self.state_value(), ShellSessionState::Idle)
                && !self.is_closed()
        }

        fn set_state(&self, state: ShellSessionState) -> Result<()> {
            let mut guard = self
                .state
                .lock()
                .map_err(|_| AppError::Session("failed to lock shell session state".to_string()))?;
            *guard = state;
            Ok(())
        }

        fn mark_busy_if_idle(&self) -> Result<bool> {
            if self.is_closed()
                || !matches!(self.mode, SessionMode::Automation)
                || self.is_user_managed()
            {
                return Ok(false);
            }
            let mut guard = self
                .state
                .lock()
                .map_err(|_| AppError::Session("failed to lock shell session state".to_string()))?;
            if !matches!(*guard, ShellSessionState::Idle) {
                return Ok(false);
            }
            *guard = ShellSessionState::Busy;
            Ok(true)
        }

        fn claim_for_user(&self) {
            if self.is_interactive() {
                return;
            }

            if self
                .claimed_by_user
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                self.emit_session_lifecycle();
            }
        }

        fn set_active_command(
            &self,
            process_id: Option<String>,
            command: Option<String>,
        ) -> Result<()> {
            let mut process_guard = self.active_process_id.lock().map_err(|_| {
                AppError::Session("failed to lock active shell process id".to_string())
            })?;
            *process_guard = process_id;

            let mut command_guard = self.active_command.lock().map_err(|_| {
                AppError::Session("failed to lock active shell command".to_string())
            })?;
            *command_guard = command;
            Ok(())
        }

        fn set_active_sender(&self, sender: Option<mpsc::UnboundedSender<String>>) -> Result<()> {
            let mut guard = self.active_sender.lock().map_err(|_| {
                AppError::Session("failed to lock active PTY command sender".to_string())
            })?;
            *guard = sender;
            Ok(())
        }

        fn update_working_directory(&self, cwd: Option<String>) -> Result<()> {
            let mut guard = self.working_directory.lock().map_err(|_| {
                AppError::Session("failed to lock shell session working directory".to_string())
            })?;
            *guard = cwd;
            Ok(())
        }

        fn subscribe(&self, tx: mpsc::Sender<StreamChunk>) {
            let mut rx = self.event_tx.subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(chunk) => {
                            if tx.send(chunk).await.is_err() {
                                return;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                }
            });
        }

        fn emit_broadcast(&self, chunk: StreamChunk) {
            let _ = self.event_tx.send(chunk);
        }

        fn session_lifecycle_chunk(&self) -> StreamChunk {
            StreamChunk::ShellSessionLifecycle {
                shell_session_id: self.shell_session_id.clone(),
                state: self.state_value(),
                cwd: self.current_working_directory(),
                active_process_id: self.current_active_process_id(),
                active_command: self.current_active_command(),
                available_for_reuse: self.is_available_for_reuse(),
                interactive: self.is_interactive(),
                user_managed: self.is_user_managed(),
            }
        }

        fn emit_session_lifecycle(&self) {
            self.emit_broadcast(self.session_lifecycle_chunk());
        }

        async fn emit_session_lifecycle_to(&self, tx: &mpsc::Sender<StreamChunk>) {
            let chunk = self.session_lifecycle_chunk();
            self.emit_broadcast(chunk.clone());
            send_stream_chunk_best_effort(tx, chunk, "shell-session-lifecycle").await;
        }

        async fn await_detached_pty_operation(
            &self,
            thread_label: &str,
            operation_name: &str,
            timeout: Duration,
            operation: impl FnOnce() -> Result<()> + Send + 'static,
        ) -> Result<()> {
            let shell_session_id = self.shell_session_id.clone();
            let thread_name = format!("gestura-pty-{thread_label}-{shell_session_id}");
            let (result_tx, result_rx) = tokio::sync::oneshot::channel();

            std::thread::Builder::new()
                .name(thread_name)
                .spawn(move || {
                    let _ = result_tx.send(operation());
                })
                .map_err(|error| {
                    AppError::Session(format!(
                        "failed to spawn PTY {operation_name} task for shell session `{shell_session_id}`: {error}"
                    ))
                })?;

            match tokio::time::timeout(timeout, result_rx).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(AppError::Session(format!(
                    "PTY {operation_name} task ended before reporting completion for shell session `{shell_session_id}`"
                ))),
                Err(_) => Err(AppError::Session(format!(
                    "timed out waiting for PTY {operation_name} to finish for shell session `{shell_session_id}`"
                ))),
            }
        }

        async fn write_to_pty(&self, data: Vec<u8>) -> Result<()> {
            let writer = self.writer.clone();
            self.await_detached_pty_operation("write", "write", PTY_WRITE_TIMEOUT, move || {
                let mut writer = writer.lock().map_err(|_| {
                    AppError::Session("PTY shell writer lock poisoned unexpectedly".to_string())
                })?;
                writer.write_all(&data).map_err(AppError::Io)?;
                writer.flush().map_err(AppError::Io)
            })
            .await
        }

        async fn send_input(&self, data: &str) -> Result<()> {
            if self.is_closed() {
                return Err(AppError::Session(
                    "PTY shell session closed unexpectedly; retry to create a fresh shell"
                        .to_string(),
                ));
            }

            self.claim_for_user();

            if data.is_empty() {
                return Ok(());
            }

            self.write_to_pty(data.as_bytes().to_vec()).await
        }

        async fn resize(&self, cols: u16, rows: u16) -> Result<()> {
            let cols = cols.max(1);
            let rows = rows.max(1);
            let master = self.master.lock().await;
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|error| AppError::Session(format!("failed to resize PTY: {error:#}")))
        }

        async fn execute(
            &self,
            command: &str,
            command_cwd: Option<&str>,
            options: ShellExecutionOptions,
            tx: mpsc::Sender<StreamChunk>,
        ) -> Result<StreamingCommandResult> {
            let _command_guard = self.command_lock.lock().await;
            if self.is_closed() {
                return Err(AppError::Session(
                    "PTY shell session closed unexpectedly; retry to create a fresh shell"
                        .to_string(),
                ));
            }

            let process_id = uuid::Uuid::new_v4().to_string();
            let start_marker = format!("__GESTURA_START_{process_id}__");
            let done_prefix = format!("__GESTURA_DONE_{process_id}__:");
            let wrapped = wrap_command(command, command_cwd, &start_marker, &done_prefix);
            let timeout_secs = options
                .timeout_secs
                .unwrap_or(DEFAULT_EXECUTION_TIMEOUT_SECS);
            let timeout = Duration::from_secs(timeout_secs);
            let stall_timeout = Duration::from_secs(
                options
                    .stall_timeout_secs
                    .unwrap_or_else(|| default_stall_timeout_secs(timeout_secs)),
            );
            let start = Instant::now();

            let (chunk_tx, mut chunk_rx) = mpsc::unbounded_channel::<String>();
            self.user_stop_requested.store(false, Ordering::SeqCst);
            self.set_active_sender(Some(chunk_tx))?;
            self.set_active_command(Some(process_id.clone()), Some(command.to_string()))?;
            self.set_state(ShellSessionState::Busy)?;
            manager()
                .register_process(process_id.clone(), self.shell_session_id.clone())
                .await;
            self.emit_session_lifecycle_to(&tx).await;

            if let Err(error) = self.write_to_pty(wrapped.into_bytes()).await {
                self.set_state(ShellSessionState::Failed)?;
                self.set_active_sender(None)?;
                self.set_active_command(None, None)?;
                manager().unregister_process(&process_id).await;
                self.emit_session_lifecycle_to(&tx).await;
                return Err(error);
            }

            let start_chunk = StreamChunk::ShellLifecycle {
                process_id: process_id.clone(),
                shell_session_id: Some(self.shell_session_id.clone()),
                state: ShellProcessState::Started,
                exit_code: None,
                duration_ms: None,
                command: command.to_string(),
                cwd: command_cwd.map(ToOwned::to_owned),
            };
            self.emit_broadcast(start_chunk.clone());
            send_stream_chunk_best_effort(&tx, start_chunk, "shell-lifecycle").await;

            let mut parser = SessionOutputParser::new(start_marker, done_prefix);
            let mut stdout = String::new();
            let mut timed_out = false;
            let mut interrupted = false;
            let mut interrupt_started_at: Option<Instant> = None;
            let mut last_activity_at: Option<Instant> = None;
            let mut continued_wait_anchor_at: Option<Instant> = None;
            let mut continued_wait_cycles = 0_u8;
            let mut recent_output_tail = String::new();
            let mut runtime_failure_kind: Option<ShellRuntimeFailureKind> = None;

            loop {
                let wait_for = if interrupted {
                    Duration::from_secs(INTERRUPT_GRACE_SECS)
                        .checked_sub(
                            interrupt_started_at
                                .expect("interrupt timestamp must be set")
                                .elapsed(),
                        )
                        .unwrap_or(Duration::from_secs(0))
                } else if options.allow_long_running && start.elapsed() >= timeout {
                    let idle_for = last_activity_at
                        .or(continued_wait_anchor_at)
                        .map(|timestamp| timestamp.elapsed())
                        .unwrap_or_else(|| start.elapsed());
                    stall_timeout
                        .checked_sub(idle_for)
                        .unwrap_or(Duration::from_secs(0))
                } else {
                    timeout
                        .checked_sub(start.elapsed())
                        .unwrap_or(Duration::from_secs(0))
                };

                match tokio::time::timeout(wait_for, chunk_rx.recv()).await {
                    Ok(Some(chunk)) => {
                        last_activity_at = Some(Instant::now());
                        continued_wait_anchor_at = None;
                        continued_wait_cycles = 0;
                        let parsed = parser.push(&chunk);
                        if !parsed.output.is_empty() {
                            stdout.push_str(&parsed.output);
                            append_recent_output_tail(&mut recent_output_tail, &parsed.output);
                            let output_chunk = StreamChunk::ShellOutput {
                                process_id: process_id.clone(),
                                shell_session_id: Some(self.shell_session_id.clone()),
                                stream: ShellOutputStream::Stdout,
                                data: parsed.output,
                            };
                            self.emit_broadcast(output_chunk.clone());
                            send_shell_output_chunk_best_effort(&tx, output_chunk).await;
                        }

                        if let Some(exit_code) = parsed.exit_code {
                            let user_stopped =
                                self.user_stop_requested.swap(false, Ordering::SeqCst);
                            let duration_ms = start.elapsed().as_millis() as u64;
                            let process_state = if timed_out {
                                ShellProcessState::Failed
                            } else if user_stopped {
                                ShellProcessState::Stopped
                            } else if exit_code == 0 {
                                ShellProcessState::Completed
                            } else {
                                ShellProcessState::Failed
                            };
                            let reported_exit = if timed_out {
                                124
                            } else if user_stopped && exit_code == 0 {
                                130
                            } else {
                                exit_code
                            };
                            let session_state = if self.is_closed() {
                                if user_stopped {
                                    ShellSessionState::Stopped
                                } else {
                                    ShellSessionState::Failed
                                }
                            } else {
                                ShellSessionState::Idle
                            };

                            return self
                                .finish_command(
                                    &tx,
                                    command,
                                    command_cwd,
                                    CommandCompletion {
                                        process_id,
                                        stdout,
                                        exit_code: reported_exit,
                                        process_state,
                                        duration_ms,
                                        session_state,
                                        failure_kind: if timed_out {
                                            runtime_failure_kind
                                                .or(Some(ShellRuntimeFailureKind::TimedOut))
                                        } else {
                                            runtime_failure_kind
                                        },
                                    },
                                )
                                .await;
                        }
                    }
                    Ok(None) => {
                        let flushed = parser.finish();
                        if !flushed.is_empty() {
                            stdout.push_str(&flushed);
                            let output_chunk = StreamChunk::ShellOutput {
                                process_id: process_id.clone(),
                                shell_session_id: Some(self.shell_session_id.clone()),
                                stream: ShellOutputStream::Stdout,
                                data: flushed,
                            };
                            self.emit_broadcast(output_chunk.clone());
                            send_shell_output_chunk_best_effort(&tx, output_chunk).await;
                        }

                        let user_stopped = self.user_stop_requested.swap(false, Ordering::SeqCst);
                        let duration_ms = start.elapsed().as_millis() as u64;
                        let exit_code = if user_stopped { 130 } else { -1 };
                        let process_state = if user_stopped {
                            ShellProcessState::Stopped
                        } else {
                            ShellProcessState::Failed
                        };
                        let session_state = if user_stopped {
                            ShellSessionState::Stopped
                        } else {
                            ShellSessionState::Failed
                        };

                        return self
                            .finish_command(
                                &tx,
                                command,
                                command_cwd,
                                CommandCompletion {
                                    process_id,
                                    stdout,
                                    exit_code,
                                    process_state,
                                    duration_ms,
                                    session_state,
                                    failure_kind: if timed_out {
                                        runtime_failure_kind
                                            .or(Some(ShellRuntimeFailureKind::TimedOut))
                                    } else {
                                        runtime_failure_kind
                                    },
                                },
                            )
                            .await;
                    }
                    Err(_) if !interrupted => {
                        if options.allow_long_running && start.elapsed() >= timeout {
                            let idle_for = last_activity_at
                                .or(continued_wait_anchor_at)
                                .map(|timestamp| timestamp.elapsed())
                                .unwrap_or_else(|| start.elapsed());

                            if idle_for < stall_timeout {
                                continue;
                            }

                            let stall_signal =
                                inspect_stall_signal(&recent_output_tail, parser.buffered_output());
                            runtime_failure_kind = stall_signal.runtime_failure_kind();
                            send_status_chunk_best_effort(
                                &tx,
                                stall_signal.status_message(&self.shell_session_id, command),
                            )
                            .await;

                            match stall_signal {
                                StallSignal::None => {
                                    if continued_wait_cycles >= MAX_QUIET_WAIT_CYCLES_WITHOUT_SIGNAL
                                    {
                                        tracing::info!(
                                            shell_session_id = %self.shell_session_id,
                                            process_id = %process_id,
                                            command = %command,
                                            timeout_secs,
                                            stall_timeout_secs = stall_timeout.as_secs(),
                                            "Interrupting quiet PTY command after repeated quiet periods without prompt/error indicators"
                                        );
                                    } else {
                                        continued_wait_anchor_at = Some(Instant::now());
                                        continued_wait_cycles += 1;
                                        tracing::debug!(
                                            shell_session_id = %self.shell_session_id,
                                            process_id = %process_id,
                                            command = %command,
                                            timeout_secs,
                                            stall_timeout_secs = stall_timeout.as_secs(),
                                            "Long-running PTY command is quiet with no prompt/error indicator; continuing to wait"
                                        );
                                        continue;
                                    }
                                }
                                StallSignal::InteractivePrompt => {
                                    tracing::info!(
                                        shell_session_id = %self.shell_session_id,
                                        process_id = %process_id,
                                        command = %command,
                                        "Interrupting quiet PTY command because recent output looks interactive"
                                    );
                                }
                                StallSignal::ErrorOutput => {
                                    tracing::info!(
                                        shell_session_id = %self.shell_session_id,
                                        process_id = %process_id,
                                        command = %command,
                                        "Interrupting quiet PTY command because recent output looks like an error"
                                    );
                                }
                            }
                        }

                        timed_out = true;
                        interrupted = true;
                        interrupt_started_at = Some(Instant::now());
                        self.set_state(ShellSessionState::Interrupting)?;
                        self.emit_session_lifecycle();
                        self.interrupt().await?;
                    }
                    Err(_) => {
                        self.set_state(ShellSessionState::Failed)?;
                        self.emit_session_lifecycle();
                        self.terminate().await?;
                        let flushed = parser.finish();
                        if !flushed.is_empty() {
                            stdout.push_str(&flushed);
                            let output_chunk = StreamChunk::ShellOutput {
                                process_id: process_id.clone(),
                                shell_session_id: Some(self.shell_session_id.clone()),
                                stream: ShellOutputStream::Stdout,
                                data: flushed,
                            };
                            self.emit_broadcast(output_chunk.clone());
                            send_shell_output_chunk_best_effort(&tx, output_chunk).await;
                        }

                        let duration_ms = start.elapsed().as_millis() as u64;
                        return self
                            .finish_command(
                                &tx,
                                command,
                                command_cwd,
                                CommandCompletion {
                                    process_id,
                                    stdout,
                                    exit_code: 124,
                                    process_state: ShellProcessState::Failed,
                                    duration_ms,
                                    session_state: ShellSessionState::Failed,
                                    failure_kind: runtime_failure_kind
                                        .or(Some(ShellRuntimeFailureKind::TimedOut)),
                                },
                            )
                            .await;
                    }
                }
            }
        }

        async fn finish_command(
            &self,
            tx: &mpsc::Sender<StreamChunk>,
            command: &str,
            command_cwd: Option<&str>,
            completion: CommandCompletion,
        ) -> Result<StreamingCommandResult> {
            manager().unregister_process(&completion.process_id).await;
            self.set_active_sender(None)?;
            self.set_active_command(None, None)?;
            self.set_state(completion.session_state)?;
            if matches!(completion.session_state, ShellSessionState::Idle) && command_cwd.is_some()
            {
                self.update_working_directory(command_cwd.map(ToOwned::to_owned))?;
            }

            let lifecycle_chunk = StreamChunk::ShellLifecycle {
                process_id: completion.process_id.clone(),
                shell_session_id: Some(self.shell_session_id.clone()),
                state: completion.process_state,
                exit_code: Some(completion.exit_code),
                duration_ms: Some(completion.duration_ms),
                command: command.to_string(),
                cwd: command_cwd.map(ToOwned::to_owned),
            };
            self.emit_broadcast(lifecycle_chunk.clone());
            send_stream_chunk_best_effort(tx, lifecycle_chunk, "shell-lifecycle").await;
            self.emit_session_lifecycle_to(tx).await;

            Ok(StreamingCommandResult {
                process_id: completion.process_id,
                command: command.to_string(),
                stdout: completion.stdout,
                stderr: String::new(),
                exit_code: completion.exit_code,
                success: matches!(completion.process_state, ShellProcessState::Completed),
                duration_ms: completion.duration_ms,
                failure_kind: completion.failure_kind,
            })
        }

        async fn request_stop(&self) -> Result<()> {
            if self.current_active_process_id().is_none() || self.is_closed() {
                return Ok(());
            }
            self.user_stop_requested.store(true, Ordering::SeqCst);
            self.set_state(ShellSessionState::Interrupting)?;
            self.emit_session_lifecycle();
            if self.interrupt().await.is_err() {
                self.set_state(ShellSessionState::Stopping)?;
                self.emit_session_lifecycle();
                return self.terminate().await;
            }

            tokio::time::sleep(Duration::from_millis(STOP_ESCALATION_MILLIS)).await;
            if self.current_active_process_id().is_some() && !self.is_closed() {
                self.set_state(ShellSessionState::Stopping)?;
                self.emit_session_lifecycle();
                self.terminate().await?;
            }
            Ok(())
        }

        async fn interrupt(&self) -> Result<()> {
            self.write_to_pty(vec![3]).await
        }

        async fn terminate(&self) -> Result<()> {
            if self.closed.swap(true, Ordering::SeqCst) {
                return Ok(());
            }
            let killer = self.killer.clone();
            self.await_detached_pty_operation("kill", "termination", PTY_KILL_TIMEOUT, move || {
                let mut killer = killer
                    .lock()
                    .map_err(|_| AppError::Session("failed to lock PTY killer".to_string()))?;
                match killer.kill() {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(AppError::Io(error)),
                }
            })
            .await
        }
    }

    fn prune_pool_locked(state: &mut ManagerState, pool_key: &str) {
        let Some(session_ids) = state.pools.get(pool_key).cloned() else {
            return;
        };

        let mut kept = Vec::with_capacity(session_ids.len());
        for shell_session_id in session_ids {
            let keep = state
                .sessions
                .get(&shell_session_id)
                .is_some_and(|session| !session.is_closed());
            if keep {
                kept.push(shell_session_id);
            } else {
                state.sessions.remove(&shell_session_id);
                state
                    .process_index
                    .retain(|_, sid| sid != &shell_session_id);
            }
        }

        if kept.is_empty() {
            state.pools.remove(pool_key);
        } else {
            state.pools.insert(pool_key.to_string(), kept);
        }
    }

    fn build_shell_command(mode: SessionMode) -> CommandBuilder {
        match mode {
            SessionMode::Interactive => CommandBuilder::new_default_prog(),
            SessionMode::Automation => automation_shell_builder(),
        }
    }

    fn automation_shell_builder() -> CommandBuilder {
        #[cfg(windows)]
        {
            let mut builder = CommandBuilder::new("cmd.exe");
            builder.arg("/Q");
            builder
        }

        #[cfg(not(windows))]
        {
            let shell_path = if std::path::Path::new("/bin/bash").exists() {
                "/bin/bash"
            } else {
                "bash"
            };
            let mut builder = CommandBuilder::new(shell_path);
            builder.arg("--noprofile");
            builder.arg("--norc");
            builder.arg("-i");
            builder
        }
    }

    fn prepare_shell(
        mode: SessionMode,
        writer: &mut Box<dyn Write + Send>,
        ready_marker: Option<&str>,
    ) -> Result<()> {
        if matches!(mode, SessionMode::Interactive) {
            writer.flush().map_err(AppError::Io)?;
            return Ok(());
        }

        #[cfg(windows)]
        {
            writer.write_all(b"prompt $G\r\n").map_err(AppError::Io)?;
            if let Some(ready_marker) = ready_marker {
                writer
                    .write_all(format!("echo {ready_marker}\r\n").as_bytes())
                    .map_err(AppError::Io)?;
            }
        }

        #[cfg(not(windows))]
        {
            let mut init_script = String::from("export PS1=''\nunset PROMPT_COMMAND\nstty -echo\n");
            if let Some(ready_marker) = ready_marker {
                init_script.push_str("printf '");
                init_script.push_str(ready_marker);
                init_script.push_str("\\n'\n");
            }
            writer
                .write_all(init_script.as_bytes())
                .map_err(AppError::Io)?;
        }

        writer.flush().map_err(AppError::Io)
    }

    struct ReaderLoopContext {
        active_sender: Arc<StdMutex<Option<mpsc::UnboundedSender<String>>>>,
        active_process_id: Arc<StdMutex<Option<String>>>,
        closed: Arc<AtomicBool>,
        claimed_by_user: Arc<AtomicBool>,
        event_tx: broadcast::Sender<StreamChunk>,
        shell_session_id: String,
        emit_raw_output: bool,
        ready_marker: Option<String>,
        ready_tx: Option<std::sync::mpsc::Sender<std::result::Result<(), String>>>,
    }

    fn spawn_reader_loop(mut reader: Box<dyn Read + Send>, context: ReaderLoopContext) {
        let ReaderLoopContext {
            active_sender,
            active_process_id,
            closed,
            claimed_by_user,
            event_tx,
            shell_session_id,
            emit_raw_output,
            ready_marker,
            ready_tx,
        } = context;

        let _ = std::thread::Builder::new()
            .name(format!("gestura-pty-reader-{shell_session_id}"))
            .spawn(move || {
                let mut buffer = [0_u8; 4096];
                let mut ready_marker = ready_marker;
                let mut ready_tx = ready_tx;
                let mut ready_buffer = String::new();
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => {
                            if let Some(ready_tx) = ready_tx.take() {
                                let _ = ready_tx
                                    .send(Err("PTY shell closed before initialization completed"
                                        .to_string()));
                            }
                            closed.store(true, Ordering::SeqCst);
                            return;
                        }
                        Ok(read) => {
                            let chunk = String::from_utf8_lossy(&buffer[..read]).into_owned();

                            if let Some(marker) = ready_marker.as_ref() {
                                ready_buffer.push_str(&chunk);
                                if ready_buffer.contains(marker) {
                                    if let Some(ready_tx) = ready_tx.take() {
                                        let _ = ready_tx.send(Ok(()));
                                    }
                                    ready_marker = None;
                                    ready_buffer.clear();
                                } else {
                                    trim_to_tail(&mut ready_buffer, marker.len() + 64);
                                }
                            }

                            let sender = active_sender.lock().ok().and_then(|guard| guard.clone());
                            if let Some(sender) = sender {
                                let _ = sender.send(chunk.clone());
                            }
                            let process_id = active_process_id
                                .lock()
                                .ok()
                                .and_then(|guard| guard.clone());
                            let should_emit_raw_output = emit_raw_output
                                || (claimed_by_user.load(Ordering::SeqCst) && process_id.is_none());
                            if should_emit_raw_output {
                                let _ = event_tx.send(StreamChunk::ShellOutput {
                                    process_id: process_id
                                        .unwrap_or_else(|| shell_session_id.clone()),
                                    shell_session_id: Some(shell_session_id.clone()),
                                    stream: ShellOutputStream::Stdout,
                                    data: chunk,
                                });
                            }
                        }
                        Err(_) => {
                            if let Some(ready_tx) = ready_tx.take() {
                                let _ = ready_tx.send(Err(
                                    "PTY shell reader failed before initialization completed"
                                        .to_string(),
                                ));
                            }
                            closed.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                }
            });
    }

    fn spawn_wait_loop(
        mut child: Box<dyn Child + Send + Sync>,
        closed: Arc<AtomicBool>,
        shell_session_id: String,
    ) {
        let _ = std::thread::Builder::new()
            .name(format!("gestura-pty-wait-{shell_session_id}"))
            .spawn(move || {
                if let Err(error) = child.wait() {
                    tracing::warn!(shell_session_id = %shell_session_id, error = %error, "PTY shell wait failed");
                }
                closed.store(true, Ordering::SeqCst);
            });
    }

    impl SessionOutputParser {
        fn new(start_marker: String, done_prefix: String) -> Self {
            Self {
                pending: String::new(),
                started: false,
                start_marker,
                done_prefix,
            }
        }

        fn push(&mut self, chunk: &str) -> ParsedChunk {
            self.pending.push_str(chunk);

            if !self.started {
                if let Some(consume_end) = find_start_marker(&self.pending, &self.start_marker) {
                    let rest = self.pending[consume_end..].to_string();
                    self.pending = rest;
                    self.started = true;
                } else {
                    trim_to_tail(&mut self.pending, self.start_marker.len() + 8);
                    return ParsedChunk {
                        output: String::new(),
                        exit_code: None,
                    };
                }
            }

            if let Some((output_end, consume_end, exit_code)) =
                find_done_marker(&self.pending, &self.done_prefix)
            {
                let output = self.pending[..output_end].to_string();
                self.pending = self.pending[consume_end..].to_string();
                return ParsedChunk {
                    output,
                    exit_code: Some(exit_code),
                };
            }

            let keep = self.done_prefix.len() + 32;
            if self.pending.len() > keep {
                let flush_len = floor_char_boundary(&self.pending, self.pending.len() - keep);
                let output = self.pending[..flush_len].to_string();
                self.pending = self.pending[flush_len..].to_string();
                ParsedChunk {
                    output,
                    exit_code: None,
                }
            } else {
                ParsedChunk {
                    output: String::new(),
                    exit_code: None,
                }
            }
        }

        fn finish(&mut self) -> String {
            if !self.started {
                self.pending.clear();
                return String::new();
            }
            std::mem::take(&mut self.pending)
        }

        fn buffered_output(&self) -> &str {
            if self.started {
                self.pending.as_str()
            } else {
                ""
            }
        }
    }

    fn append_recent_output_tail(buffer: &mut String, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        buffer.push_str(chunk);
        trim_to_tail(buffer, STALL_SIGNAL_TAIL_BYTES);
    }

    fn inspect_stall_signal(flushed_output_tail: &str, buffered_output_tail: &str) -> StallSignal {
        let mut combined = flushed_output_tail.to_string();
        append_recent_output_tail(&mut combined, buffered_output_tail);
        if combined.trim().is_empty() {
            return StallSignal::None;
        }

        let normalized = combined.to_ascii_lowercase();
        if INTERACTIVE_PROMPT_PATTERNS
            .iter()
            .any(|needle| normalized.contains(needle))
        {
            return StallSignal::InteractivePrompt;
        }

        if normalized
            .lines()
            .map(str::trim)
            .any(|line| line.starts_with("error:"))
            || ERROR_OUTPUT_PATTERNS
                .iter()
                .any(|needle| normalized.contains(needle))
        {
            return StallSignal::ErrorOutput;
        }

        StallSignal::None
    }

    fn find_done_marker(buffer: &str, done_prefix: &str) -> Option<(usize, usize, i32)> {
        let mut search_from = 0;
        while let Some(rel_idx) = buffer[search_from..].find(done_prefix) {
            let idx = search_from + rel_idx;
            let before_ok = idx == 0 || matches!(buffer.as_bytes()[idx - 1], b'\n' | b'\r');
            if !before_ok {
                search_from = idx + done_prefix.len();
                continue;
            }

            let code_start = idx + done_prefix.len();
            let line_end_rel = buffer[code_start..].find('\n')?;
            let consume_end = code_start + line_end_rel + 1;
            let Ok(exit_code) = buffer[code_start..code_start + line_end_rel]
                .trim_end_matches('\r')
                .parse()
            else {
                search_from = idx + done_prefix.len();
                continue;
            };
            let output_end = if idx == 0 {
                0
            } else if buffer.as_bytes()[idx - 1] == b'\n' {
                if idx > 1 && buffer.as_bytes()[idx - 2] == b'\r' {
                    idx - 2
                } else {
                    idx - 1
                }
            } else if buffer.as_bytes()[idx - 1] == b'\r' {
                idx - 1
            } else {
                idx
            };
            return Some((output_end, consume_end, exit_code));
        }

        None
    }

    fn find_start_marker(buffer: &str, start_marker: &str) -> Option<usize> {
        let mut search_from = 0;
        while let Some(rel_idx) = buffer[search_from..].find(start_marker) {
            let idx = search_from + rel_idx;
            let before_ok = idx == 0 || matches!(buffer.as_bytes()[idx - 1], b'\n' | b'\r');
            if !before_ok {
                search_from = idx + start_marker.len();
                continue;
            }

            let after = idx + start_marker.len();
            let consumed = if buffer[after..].starts_with("\r\n") {
                after + 2
            } else if buffer[after..].starts_with('\n') || buffer[after..].starts_with('\r') {
                after + 1
            } else {
                search_from = idx + start_marker.len();
                continue;
            };

            return Some(consumed);
        }

        None
    }

    fn trim_to_tail(value: &mut String, keep: usize) {
        if value.len() <= keep {
            return;
        }
        let start = ceil_char_boundary(value, value.len() - keep);
        *value = value[start..].to_string();
    }

    fn floor_char_boundary(value: &str, index: usize) -> usize {
        let mut index = index.min(value.len());
        while index > 0 && !value.is_char_boundary(index) {
            index -= 1;
        }
        index
    }

    fn ceil_char_boundary(value: &str, index: usize) -> usize {
        let mut index = index.min(value.len());
        while index < value.len() && !value.is_char_boundary(index) {
            index += 1;
        }
        index
    }

    fn wrap_command(
        command: &str,
        command_cwd: Option<&str>,
        start_marker: &str,
        done_prefix: &str,
    ) -> String {
        #[cfg(windows)]
        {
            wrap_command_windows(command, command_cwd, start_marker, done_prefix)
        }

        #[cfg(not(windows))]
        {
            wrap_command_posix(command, command_cwd, start_marker, done_prefix)
        }
    }

    #[cfg(not(windows))]
    fn wrap_command_posix(
        command: &str,
        command_cwd: Option<&str>,
        start_marker: &str,
        done_prefix: &str,
    ) -> String {
        let mut script = format!("printf '{start_marker}\\n'; _GESTURA_STATUS=0; ");
        if let Some(cwd) = command_cwd {
            script.push_str("cd ");
            script.push_str(&quote_posix(cwd));
            script.push_str(" || _GESTURA_STATUS=$?; ");
        }
        script.push_str("if [ \"$_GESTURA_STATUS\" -eq 0 ]; then eval ");
        script.push_str(&quote_posix(command));
        script.push_str("; _GESTURA_STATUS=$?; fi; ");
        script.push_str(&format!(
            "printf '\\n{done_prefix}%s\\n' \"$_GESTURA_STATUS\"; unset _GESTURA_STATUS\n"
        ));
        script
    }

    #[cfg(windows)]
    fn wrap_command_windows(
        command: &str,
        command_cwd: Option<&str>,
        start_marker: &str,
        done_prefix: &str,
    ) -> String {
        let mut script = format!("echo {start_marker}\r\nset GESTURA_STATUS=0\r\n");
        if let Some(cwd) = command_cwd {
            script.push_str("cd /d ");
            script.push_str(&quote_cmd_arg(cwd));
            script.push_str(" || set GESTURA_STATUS=%ERRORLEVEL%\r\n");
        }
        script.push_str("if \"%GESTURA_STATUS%\"==\"0\" ");
        script.push_str(command);
        script.push_str("\r\n");
        script.push_str("if \"%GESTURA_STATUS%\"==\"0\" set GESTURA_STATUS=%ERRORLEVEL%\r\n");
        script.push_str(&format!(
            "echo {done_prefix}%GESTURA_STATUS%\r\nset GESTURA_STATUS=\r\n"
        ));
        script
    }

    #[cfg(not(windows))]
    fn quote_posix(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

    #[cfg(windows)]
    fn quote_cmd_arg(value: &str) -> String {
        format!("\"{}\"", value.replace('"', "\"\""))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const PTY_TEST_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
        const PTY_TEST_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

        fn shell_echo_command(text: &str) -> String {
            #[cfg(windows)]
            {
                format!("echo {text}")
            }

            #[cfg(not(windows))]
            {
                format!("printf {text}")
            }
        }

        fn shell_delayed_echo_command(delay_secs: u64, text: &str) -> String {
            #[cfg(windows)]
            {
                format!(
                    "ping -n {} 127.0.0.1 >nul && echo {text}",
                    delay_secs.saturating_add(1)
                )
            }

            #[cfg(not(windows))]
            {
                format!("sleep {delay_secs} && printf {text}")
            }
        }

        fn interactive_echo_input(text: &str) -> String {
            #[cfg(windows)]
            {
                format!("echo {text}\r\n")
            }

            #[cfg(not(windows))]
            {
                format!("printf {text}\n")
            }
        }

        async fn execute_in_session_for_test(
            pool_key: &str,
            initial_cwd: Option<&str>,
            command: &str,
            command_cwd: Option<&str>,
            timeout_secs: Option<u64>,
            tx: mpsc::Sender<StreamChunk>,
        ) -> Result<StreamingCommandResult> {
            tokio::time::timeout(
                PTY_TEST_COMMAND_TIMEOUT,
                execute_in_session(
                    pool_key,
                    initial_cwd,
                    command,
                    command_cwd,
                    timeout_secs,
                    tx,
                ),
            )
            .await
            .expect("timed out waiting for PTY test command")
        }

        async fn execute_in_session_with_options_for_test(
            pool_key: &str,
            initial_cwd: Option<&str>,
            command: &str,
            command_cwd: Option<&str>,
            options: ShellExecutionOptions,
            tx: mpsc::Sender<StreamChunk>,
        ) -> Result<StreamingCommandResult> {
            tokio::time::timeout(
                PTY_TEST_COMMAND_TIMEOUT,
                execute_in_session_with_options(
                    pool_key,
                    initial_cwd,
                    command,
                    command_cwd,
                    options,
                    tx,
                ),
            )
            .await
            .expect("timed out waiting for PTY test command with custom options")
        }

        async fn stop_session_for_test(shell_session_id: &str) {
            let _ = tokio::time::timeout(PTY_TEST_SHUTDOWN_TIMEOUT, stop_session(shell_session_id))
                .await
                .expect("timed out stopping PTY session")
                .expect("stop PTY session");
        }

        async fn shutdown_session_for_test(pool_key: &str) {
            tokio::time::timeout(PTY_TEST_SHUTDOWN_TIMEOUT, shutdown_session(pool_key))
                .await
                .expect("timed out shutting down PTY session pool")
                .expect("shutdown PTY session pool");
        }

        async fn recv_session_lifecycle(
            rx: &mut mpsc::Receiver<StreamChunk>,
        ) -> (String, ShellSessionState, bool, bool) {
            loop {
                if let StreamChunk::ShellSessionLifecycle {
                    shell_session_id,
                    state,
                    interactive,
                    user_managed,
                    ..
                } = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                    .await
                    .expect("timed out waiting for shell event")
                    .expect("channel closed while waiting for shell event")
                {
                    return (shell_session_id, state, interactive, user_managed);
                }
            }
        }

        async fn recv_command_started(rx: &mut mpsc::Receiver<StreamChunk>) -> (String, String) {
            loop {
                if let StreamChunk::ShellLifecycle {
                    process_id,
                    shell_session_id,
                    state: ShellProcessState::Started,
                    ..
                } = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                    .await
                    .expect("timed out waiting for command start")
                    .expect("channel closed while waiting for command start")
                {
                    return (
                        process_id,
                        shell_session_id.expect("PTY-managed commands should carry session id"),
                    );
                }
            }
        }

        #[test]
        fn parser_extracts_output_and_exit_code() {
            let mut parser = SessionOutputParser::new(
                "__GESTURA_START_abc__".to_string(),
                "__GESTURA_DONE_abc__:".to_string(),
            );

            let first =
                parser.push("printf '__GESTURA_START_abc__\\n'\r\n__GESTURA_START_abc__\r\nhello");
            assert_eq!(first.output, "");
            assert_eq!(first.exit_code, None);

            let second = parser.push(" world\n__GESTURA_DONE_abc__:0\r\n");
            assert_eq!(second.output, "hello world");
            assert_eq!(second.exit_code, Some(0));
        }

        #[test]
        fn parser_extracts_done_marker_when_it_starts_a_new_chunk() {
            let mut parser = SessionOutputParser::new(
                "__GESTURA_START_abc__".to_string(),
                "__GESTURA_DONE_abc__:".to_string(),
            );

            let first = parser.push("__GESTURA_START_abc__\r\n");
            assert_eq!(first.output, "");
            assert_eq!(first.exit_code, None);

            let second = parser.push("__GESTURA_DONE_abc__:0\r\n>");
            assert_eq!(second.output, "");
            assert_eq!(second.exit_code, Some(0));
        }

        #[test]
        fn parser_extracts_done_marker_after_windows_prompt_echo() {
            let mut parser = SessionOutputParser::new(
                "__GESTURA_START_abc__".to_string(),
                "__GESTURA_DONE_abc__:".to_string(),
            );

            let first =
                parser.push(">echo __GESTURA_START_abc__\r\n__GESTURA_START_abc__\r\nwarmup\r\n");
            assert_eq!(first.output, "");
            assert_eq!(first.exit_code, None);

            let second = parser.push("__GESTURA_DONE_abc__:0\r\n>");
            assert_eq!(second.output, "warmup");
            assert_eq!(second.exit_code, Some(0));
        }

        #[test]
        fn parser_accepts_start_marker_after_carriage_return_boundary() {
            let mut parser = SessionOutputParser::new(
                "__GESTURA_START_abc__".to_string(),
                "__GESTURA_DONE_abc__:".to_string(),
            );

            let first = parser.push("noise\r__GESTURA_START_abc__\nhello");
            assert_eq!(first.output, "");
            assert_eq!(first.exit_code, None);

            let second = parser.push("\n__GESTURA_DONE_abc__:0\r\n");
            assert_eq!(second.output, "hello");
            assert_eq!(second.exit_code, Some(0));
        }

        #[test]
        fn parser_skips_malformed_done_marker_before_real_exit_code() {
            let mut parser = SessionOutputParser::new(
                "__GESTURA_START_abc__".to_string(),
                "__GESTURA_DONE_abc__:".to_string(),
            );

            let first = parser.push("__GESTURA_START_abc__\r\nhello\n");
            assert_eq!(first.output, "");
            assert_eq!(first.exit_code, None);

            let second =
                parser.push("__GESTURA_DONE_abc__:%GESTURA_STATUS%\r\n__GESTURA_DONE_abc__:0\r\n");
            assert_eq!(
                second.output,
                "hello\n__GESTURA_DONE_abc__:%GESTURA_STATUS%"
            );
            assert_eq!(second.exit_code, Some(0));
        }

        #[test]
        #[cfg(not(windows))]
        fn wrap_command_changes_directory_before_execution() {
            let script = wrap_command(
                "pwd",
                Some("/tmp/example dir"),
                "__GESTURA_START__",
                "__GESTURA_DONE__:",
            );

            assert!(script.contains("cd '/tmp/example dir'"));
            assert!(script.contains("printf '__GESTURA_START__\\n'"));
            assert!(script.contains("printf '\\n__GESTURA_DONE__:%s\\n'"));
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn create_session_starts_idle_shell_without_command() {
            let (tx, mut rx) = mpsc::channel(64);
            let handle = create_session(
                "pty-create-session",
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(ToOwned::to_owned))
                    .as_deref(),
                Some(tx),
            )
            .await
            .expect("create PTY session");

            let (shell_session_id, state, interactive, user_managed) =
                recv_session_lifecycle(&mut rx).await;
            assert_eq!(shell_session_id, handle.shell_session_id);
            assert_eq!(state, ShellSessionState::Idle);
            assert!(interactive);
            assert!(user_managed);

            stop_session_for_test(&handle.shell_session_id).await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn reuses_idle_session_within_same_pool() {
            let (tx, mut rx) = mpsc::channel(128);
            let first = execute_in_session_for_test(
                "pty-reuse-pool",
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(ToOwned::to_owned))
                    .as_deref(),
                &shell_echo_command("first"),
                None,
                Some(10),
                tx.clone(),
            )
            .await
            .expect("first command result");
            assert!(first.stdout.contains("first"));

            let (_, first_session_id) = recv_command_started(&mut rx).await;

            let second = execute_in_session_for_test(
                "pty-reuse-pool",
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(ToOwned::to_owned))
                    .as_deref(),
                &shell_echo_command("second"),
                None,
                Some(10),
                tx,
            )
            .await
            .expect("second command result");
            assert!(second.stdout.contains("second"));

            let (_, second_session_id) = recv_command_started(&mut rx).await;
            assert_eq!(first_session_id, second_session_id);

            shutdown_session_for_test("pty-reuse-pool").await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn execute_in_session_emits_session_lifecycle_to_caller_stream() {
            let (tx, mut rx) = mpsc::channel(128);
            let result = execute_in_session_for_test(
                "pty-session-lifecycle-stream",
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(ToOwned::to_owned))
                    .as_deref(),
                &shell_echo_command("streamed"),
                None,
                Some(10),
                tx,
            )
            .await
            .expect("command result");
            assert!(result.stdout.contains("streamed"));

            let (busy_session_id, busy_state, interactive, user_managed) =
                recv_session_lifecycle(&mut rx).await;
            assert_eq!(busy_state, ShellSessionState::Busy);
            assert!(!interactive);
            assert!(!user_managed);

            let (_, started_session_id) = recv_command_started(&mut rx).await;
            assert_eq!(started_session_id, busy_session_id);

            let (idle_session_id, idle_state, interactive, user_managed) =
                recv_session_lifecycle(&mut rx).await;
            assert_eq!(idle_session_id, busy_session_id);
            assert_eq!(idle_state, ShellSessionState::Idle);
            assert!(!interactive);
            assert!(!user_managed);

            shutdown_session_for_test("pty-session-lifecycle-stream").await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn allocates_new_session_when_existing_one_is_busy() {
            let (tx_one, mut rx_one) = mpsc::channel(128);
            let (tx_two, mut rx_two) = mpsc::channel(128);
            let first_command = shell_delayed_echo_command(1, "one");
            let second_command = shell_echo_command("two");

            let first = tokio::spawn(async move {
                execute_in_session_for_test(
                    "pty-busy-pool",
                    std::env::current_dir()
                        .ok()
                        .and_then(|p| p.to_str().map(ToOwned::to_owned))
                        .as_deref(),
                    &first_command,
                    None,
                    Some(10),
                    tx_one,
                )
                .await
            });

            let (_, first_session_id) = recv_command_started(&mut rx_one).await;

            let second = tokio::spawn(async move {
                execute_in_session_for_test(
                    "pty-busy-pool",
                    std::env::current_dir()
                        .ok()
                        .and_then(|p| p.to_str().map(ToOwned::to_owned))
                        .as_deref(),
                    &second_command,
                    None,
                    Some(10),
                    tx_two,
                )
                .await
            });

            let (_, second_session_id) = recv_command_started(&mut rx_two).await;
            assert_ne!(first_session_id, second_session_id);

            first.await.expect("first join").expect("first result");
            second.await.expect("second join").expect("second result");

            shutdown_session_for_test("pty-busy-pool").await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn activity_aware_execution_allows_recently_active_long_running_commands() {
            let (tx, _rx) = mpsc::channel(128);
            let command = if cfg!(windows) {
                "echo warmup && ping -n 3 127.0.0.1 >nul && echo progress && ping -n 3 127.0.0.1 >nul && echo done"
            } else {
                "printf warmup && sleep 2 && printf progress && sleep 2 && printf done"
            };

            let result = execute_in_session_with_options_for_test(
                "pty-activity-aware-pool",
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(ToOwned::to_owned))
                    .as_deref(),
                command,
                None,
                ShellExecutionOptions {
                    timeout_secs: Some(1),
                    allow_long_running: true,
                    stall_timeout_secs: Some(3),
                },
                tx,
            )
            .await
            .expect("activity-aware PTY command result");

            assert!(result.success);
            assert!(result.stdout.contains("done"));

            shutdown_session_for_test("pty-activity-aware-pool").await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn execute_in_session_completes_when_caller_stream_is_not_draining() {
            let (tx, _rx) = mpsc::channel(1);

            let result = execute_in_session_for_test(
                "pty-non-draining-caller-stream",
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(ToOwned::to_owned))
                    .as_deref(),
                &shell_echo_command("still-completes"),
                None,
                Some(10),
                tx,
            )
            .await
            .expect("PTY command result despite non-draining caller stream");

            assert!(result.success);
            assert!(result.stdout.contains("still-completes"));

            shutdown_session_for_test("pty-non-draining-caller-stream").await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn activity_aware_execution_reports_continue_wait_before_timing_out_when_quiet_output_has_no_indicator()
         {
            let (tx, mut rx) = mpsc::channel(128);
            let command = if cfg!(windows) {
                "ping -n 6 127.0.0.1 >nul && echo done"
            } else {
                "sleep 4 && printf done"
            };

            let result = execute_in_session_with_options_for_test(
                "pty-quiet-activity-aware-pool",
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(ToOwned::to_owned))
                    .as_deref(),
                command,
                None,
                ShellExecutionOptions {
                    timeout_secs: Some(1),
                    allow_long_running: true,
                    stall_timeout_secs: Some(1),
                },
                tx,
            )
            .await
            .expect("quiet activity-aware PTY command result");

            shutdown_session_for_test("pty-quiet-activity-aware-pool").await;

            assert!(!result.success);
            assert_eq!(result.exit_code, 124);
            assert_eq!(result.failure_kind, Some(ShellRuntimeFailureKind::TimedOut));

            let mut saw_continue_wait_status = false;
            while let Ok(chunk) = rx.try_recv() {
                if let StreamChunk::Status { message } = chunk
                    && message.contains("saw no prompt or error indicator")
                    && message.contains("will continue waiting")
                {
                    saw_continue_wait_status = true;
                    break;
                }
            }
            assert!(saw_continue_wait_status);
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn activity_aware_execution_times_out_after_repeated_quiet_periods_without_signal() {
            let (tx, _rx) = mpsc::channel(128);
            let command = if cfg!(windows) {
                "ping -n 6 127.0.0.1 >nul && echo done"
            } else {
                "sleep 4 && printf done"
            };

            let result = tokio::time::timeout(
                Duration::from_secs(8),
                execute_in_session_with_options(
                    "pty-quiet-timeout-pool",
                    std::env::current_dir()
                        .ok()
                        .and_then(|p| p.to_str().map(ToOwned::to_owned))
                        .as_deref(),
                    command,
                    None,
                    ShellExecutionOptions {
                        timeout_secs: Some(1),
                        allow_long_running: true,
                        stall_timeout_secs: Some(1),
                    },
                    tx,
                ),
            )
            .await
            .expect("quiet PTY command should not hang")
            .expect("quiet PTY timeout result");

            shutdown_session_for_test("pty-quiet-timeout-pool").await;

            assert!(!result.success);
            assert_eq!(result.exit_code, 124);
            assert_eq!(result.failure_kind, Some(ShellRuntimeFailureKind::TimedOut));
        }

        #[test]
        fn stall_signal_maps_interactive_prompt_to_waiting_for_input_failure_kind() {
            assert_eq!(
                StallSignal::InteractivePrompt.runtime_failure_kind(),
                Some(ShellRuntimeFailureKind::WaitingForInput)
            );
            assert!(
                StallSignal::InteractivePrompt
                    .status_message("shell-123", "pnpm add vite")
                    .contains("waiting_for_input")
            );
        }

        #[test]
        fn stall_signal_detects_interactive_prompt_in_buffered_output() {
            assert_eq!(
                inspect_stall_signal(
                    "",
                    "Need to install the following packages:\ncreate-app\nProceed? (y/n)"
                ),
                StallSignal::InteractivePrompt
            );
        }

        #[test]
        fn stall_signal_detects_recent_error_output() {
            assert_eq!(
                inspect_stall_signal(
                    "Compiling dependencies\n",
                    "error: no such file or directory"
                ),
                StallSignal::ErrorOutput
            );
        }

        #[test]
        fn stall_signal_returns_none_when_output_has_no_clear_indicator() {
            assert_eq!(
                inspect_stall_signal("Compiling 24 crates\n", "Still working on build graph"),
                StallSignal::None
            );
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn claiming_automation_session_removes_it_from_reuse_pool() {
            let (tx, mut rx) = mpsc::channel(128);
            let first = execute_in_session_for_test(
                "pty-claim-pool",
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(ToOwned::to_owned))
                    .as_deref(),
                &shell_echo_command("first"),
                None,
                Some(10),
                tx.clone(),
            )
            .await
            .expect("first command result");
            assert!(first.stdout.contains("first"));

            let (_, first_session_id) = recv_command_started(&mut rx).await;

            claim_session(&first_session_id)
                .await
                .expect("claim session")
                .expect("claimed metadata");

            let metadata = describe_session(&first_session_id)
                .await
                .expect("describe claimed session")
                .expect("session metadata");
            assert!(metadata.user_managed);
            assert!(!metadata.available_for_reuse);

            let second = execute_in_session_for_test(
                "pty-claim-pool",
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(ToOwned::to_owned))
                    .as_deref(),
                &shell_echo_command("second"),
                None,
                Some(10),
                tx,
            )
            .await
            .expect("second command result");
            assert!(second.stdout.contains("second"));

            let (_, second_session_id) = recv_command_started(&mut rx).await;
            assert_ne!(first_session_id, second_session_id);

            shutdown_session_for_test("pty-claim-pool").await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn claimed_automation_session_streams_interactive_output_after_attach() {
            let (tx, mut rx) = mpsc::channel(128);
            execute_in_session_for_test(
                "pty-attach-pool",
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(ToOwned::to_owned))
                    .as_deref(),
                &shell_echo_command("base"),
                None,
                Some(10),
                tx,
            )
            .await
            .expect("seed command result");

            let (_, shell_session_id) = recv_command_started(&mut rx).await;

            let (attach_tx, mut attach_rx) = mpsc::channel(128);
            subscribe_session(&shell_session_id, attach_tx)
                .await
                .expect("subscribe to shell session")
                .expect("session metadata after subscribe");

            claim_session(&shell_session_id)
                .await
                .expect("claim session")
                .expect("claimed session metadata");

            send_input(&shell_session_id, &interactive_echo_input("attached"))
                .await
                .expect("send interactive input");

            let mut saw_attached_output = false;
            for _ in 0..12 {
                let chunk = tokio::time::timeout(Duration::from_secs(5), attach_rx.recv())
                    .await
                    .expect("timed out waiting for attached shell output")
                    .expect("attach channel closed unexpectedly");

                if let StreamChunk::ShellOutput { data, .. } = chunk
                    && data.contains("attached")
                {
                    saw_attached_output = true;
                    break;
                }
            }

            assert!(
                saw_attached_output,
                "claimed automation session did not stream attached shell output"
            );

            shutdown_session_for_test("pty-attach-pool").await;
        }
    }
}

/// Start a long-lived interactive PTY shell session immediately.
pub async fn create_session(
    pool_key: &str,
    initial_cwd: Option<&str>,
    tx: Option<mpsc::Sender<StreamChunk>>,
) -> Result<ShellSessionHandle> {
    imp::create_session(pool_key, initial_cwd, tx).await
}

/// Execute a shell command inside a reusable PTY-backed session.
pub async fn execute_in_session(
    pool_key: &str,
    initial_cwd: Option<&str>,
    command: &str,
    command_cwd: Option<&str>,
    timeout_secs: Option<u64>,
    tx: mpsc::Sender<StreamChunk>,
) -> Result<StreamingCommandResult> {
    execute_in_session_with_options(
        pool_key,
        initial_cwd,
        command,
        command_cwd,
        ShellExecutionOptions {
            timeout_secs,
            ..ShellExecutionOptions::default()
        },
        tx,
    )
    .await
}

/// Execute a shell command inside a reusable PTY-backed session with an explicit policy.
pub async fn execute_in_session_with_options(
    pool_key: &str,
    initial_cwd: Option<&str>,
    command: &str,
    command_cwd: Option<&str>,
    options: ShellExecutionOptions,
    tx: mpsc::Sender<StreamChunk>,
) -> Result<StreamingCommandResult> {
    imp::execute_in_session_with_options(pool_key, initial_cwd, command, command_cwd, options, tx)
        .await
}

/// Stop an active PTY-managed command by its command run id.
pub async fn stop_process(process_id: &str) -> Result<Option<ShellSessionHandle>> {
    imp::stop_process(process_id).await
}

/// Stop and remove a long-lived PTY shell session.
pub async fn stop_session(shell_session_id: &str) -> Result<Option<ShellSessionHandle>> {
    imp::stop_session(shell_session_id).await
}

/// Shut down all PTY-backed shell sessions associated with a pool key.
pub async fn shutdown_session(pool_key: &str) -> Result<()> {
    imp::shutdown_session(pool_key).await
}

/// Send raw input bytes to a PTY shell session.
pub async fn send_input(shell_session_id: &str, data: &str) -> Result<()> {
    imp::send_input(shell_session_id, data).await
}

/// Resize a PTY shell session to the given column and row dimensions.
pub async fn resize_session(shell_session_id: &str, cols: u16, rows: u16) -> Result<()> {
    imp::resize_session(shell_session_id, cols, rows).await
}

/// Describe a PTY shell session for frontend/UI enrichment.
pub async fn describe_session(shell_session_id: &str) -> Result<Option<ShellSessionMetadata>> {
    imp::describe_session(shell_session_id).await
}

/// Claim a PTY shell session for direct user management.
pub async fn claim_session(shell_session_id: &str) -> Result<Option<ShellSessionMetadata>> {
    imp::claim_session(shell_session_id).await
}

/// Subscribe to lifecycle/output events for an existing PTY shell session.
pub async fn subscribe_session(
    shell_session_id: &str,
    tx: mpsc::Sender<StreamChunk>,
) -> Result<Option<ShellSessionMetadata>> {
    imp::subscribe_session(shell_session_id, tx).await
}
