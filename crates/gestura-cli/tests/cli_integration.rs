//! CLI integration tests for gestura
//!
//! These tests verify that CLI commands work correctly end-to-end.

use assert_cmd::Command;
use gestura_core::agent_sessions::{
    AgentSession, AgentSessionStore, FileAgentSessionStore, MessageSource, SessionFilter,
};
use predicates::prelude::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ISOLATED_HOME_ID: AtomicU64 = AtomicU64::new(0);

fn gestura_data_dir(home: &Path) -> PathBuf {
    home.join(".gestura")
}

#[cfg(windows)]
fn windows_home_components(home: &Path) -> (String, String) {
    let home = home.to_string_lossy().replace('/', "\\");
    if home.len() >= 2 && home.as_bytes()[1] == b':' {
        let drive = home[..2].to_string();
        let rest = &home[2..];
        let path = if rest.is_empty() {
            "\\".to_string()
        } else if rest.starts_with('\\') {
            rest.to_string()
        } else {
            format!("\\{rest}")
        };
        (drive, path)
    } else {
        ("C:".to_string(), "\\".to_string())
    }
}

fn configure_isolated_home_env(cmd: &mut Command, home: &Path) {
    cmd.env("GESTURA_DISABLE_KEYCHAIN", "1");
    cmd.env("GESTURA_HOME_DIR", home);

    // Keep standard home variables aligned for code paths that still consult
    // the platform environment directly.
    cmd.env("HOME", home);
    cmd.env("USERPROFILE", home);

    #[cfg(windows)]
    {
        let (home_drive, home_path) = windows_home_components(home);
        cmd.env("HOMEDRIVE", home_drive);
        cmd.env("HOMEPATH", home_path);
    }
}

/// Get a Command for the gestura binary
#[allow(deprecated)] // cargo_bin is deprecated but we need it for tests
fn gestura() -> Command {
    // IMPORTANT: these integration tests run the *real* gestura binary, which
    // depends on gestura-core as a normal dependency (cfg(test) is false there).
    // Under `--all-features`, the `security` feature enables OS keychain access,
    // which can block/hang in non-interactive contexts.
    //
    // We disable keychain usage and isolate HOME so tests are deterministic.
    let home = isolated_home_dir();
    gestura_with_home(&home)
}

#[allow(deprecated)] // cargo_bin is deprecated but we need it for tests
fn gestura_with_home(home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("gestura").unwrap();

    configure_isolated_home_env(&mut cmd, home);

    cmd
}

fn isolated_home_dir() -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push("gestura-cli-integration-tests");
    dir.push(format!("pid-{}", std::process::id()));
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let unique_id = NEXT_ISOLATED_HOME_ID.fetch_add(1, Ordering::Relaxed);
    dir.push(format!("run-{nanos}-{unique_id}"));

    // Best-effort; tests should still fail loudly later if this can't be created.
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn write_saved_session(home: &Path) -> AgentSession {
    let sessions_dir = gestura_data_dir(home).join("agent_sessions");
    let workspace_dir = home.join("workspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();

    let store = FileAgentSessionStore::new(sessions_dir);
    let mut session = AgentSession::new_with_workspace(workspace_dir, Some("test-model".into()))
        .expect("session should be created");
    session.title = "CLI Session Test".to_string();
    session.add_user_message("verify canonical session store", MessageSource::Text);
    store.save(&session).expect("session should be saved");
    session
}

#[test]
fn test_version() {
    gestura()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("gestura"));
}

#[test]
fn test_help() {
    gestura()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("voice-first AI assistant"))
        .stdout(predicate::str::contains("agent"))
        .stdout(predicate::str::contains("exec"))
        .stdout(predicate::str::contains("listen"))
        .stdout(predicate::str::contains("config"))
        .stdout(predicate::str::contains("health"));
}

#[test]
fn test_config_list() {
    gestura().args(["config", "list"]).assert().success();
}

#[test]
fn test_health() {
    gestura()
        .arg("health")
        .assert()
        .success()
        .stdout(predicate::str::contains("System Health"));
}

#[test]
fn test_completion_bash() {
    gestura()
        .args(["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_gestura"));
}

#[test]
fn test_completion_zsh() {
    gestura()
        .args(["completion", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("#compdef gestura"));
}

#[test]
fn test_completion_fish() {
    gestura()
        .args(["completion", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete -c gestura"));
}

#[test]
fn test_model_whisper_list() {
    gestura()
        .args(["model", "whisper", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Whisper Models"));
}

#[test]
fn test_device_list() {
    gestura().args(["device", "list"]).assert().success();
}

#[test]
fn test_session_list() {
    gestura().args(["session", "list"]).assert().success();
}

#[test]
fn test_mcp_list() {
    gestura().args(["mcp", "list"]).assert().success();
}

#[test]
fn test_privacy_policy() {
    gestura().args(["privacy", "policy"]).assert().success();
}

#[test]
fn test_agent_list() {
    gestura().args(["agent-info", "list"]).assert().success();
}

#[test]
fn test_tools_file_list() {
    gestura()
        .args(["tools", "file", "list", "."])
        .assert()
        .success();
}

#[test]
fn test_tools_git_status() {
    gestura()
        .args(["tools", "git", "status"])
        .assert()
        .success();
}

#[test]
fn test_tools_shell_history() {
    gestura()
        .args(["tools", "shell", "history"])
        .assert()
        .success();
}

#[test]
fn test_tools_permissions_list() {
    gestura()
        .args(["tools", "permissions", "list"])
        .assert()
        .success();
}

#[test]
fn test_invalid_command() {
    gestura()
        .arg("nonexistent-command")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}

// ==================== Session Management Tests ====================

#[test]
fn test_session_resume_help() {
    // Session resume help should work
    gestura()
        .args(["session", "resume", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Resume"));
}

#[test]
fn test_session_fork_help() {
    // Session fork help should work
    gestura()
        .args(["session", "fork", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fork"));
}

#[test]
fn test_session_list_reads_core_agent_session_store() {
    let home = isolated_home_dir();
    let session = write_saved_session(&home);

    gestura_with_home(&home)
        .args(["session", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&session.id));
}

#[test]
fn test_session_fork_persists_new_core_session_identity() {
    let home = isolated_home_dir();
    let session = write_saved_session(&home);

    gestura_with_home(&home)
        .args(["session", "fork", &session.id])
        .assert()
        .success();

    let store = FileAgentSessionStore::new(gestura_data_dir(&home).join("agent_sessions"));
    let sessions = store
        .list(SessionFilter::All)
        .expect("sessions should load from canonical store");

    assert_eq!(sessions.len(), 2, "expected original and forked session");

    let forked_id = sessions
        .iter()
        .map(|info| info.id.as_str())
        .find(|id| *id != session.id)
        .expect("forked session id should exist")
        .to_string();
    let forked = store.load(&forked_id).expect("forked session should load");

    assert_eq!(forked.id, forked_id);
    assert_eq!(forked.title, session.title);
    assert_eq!(forked.message_count(), session.message_count());
    assert_eq!(forked.workspace_dir(), session.workspace_dir());
}

#[test]
fn test_session_delete_removes_from_core_session_store() {
    let home = isolated_home_dir();
    let session = write_saved_session(&home);

    gestura_with_home(&home)
        .args(["session", "delete", &session.id])
        .assert()
        .success();

    let store = FileAgentSessionStore::new(gestura_data_dir(&home).join("agent_sessions"));
    let sessions = store
        .list(SessionFilter::All)
        .expect("sessions should load from canonical store");
    assert!(
        sessions.is_empty(),
        "expected deleted session to be removed"
    );
}

// ==================== Tools Command Tests ====================

#[test]
fn test_tools_code_help() {
    gestura()
        .args(["tools", "code", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Code"));
}

#[test]
fn test_tools_web_help() {
    gestura()
        .args(["tools", "web", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Web"));
}

// ==================== Config Tests ====================

#[test]
fn test_config_get_help() {
    gestura()
        .args(["config", "get", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Get"));
}

#[test]
fn test_config_set_help() {
    gestura()
        .args(["config", "set", "--help"])
        .assert()
        .success();
}

// ==================== Error Handling Tests ====================

#[test]
fn test_exec_empty_command() {
    // Exec with no command should fail gracefully
    gestura().args(["exec"]).assert().failure();
}

#[test]
fn test_agent_help() {
    // Agent subcommand help should work
    gestura()
        .args(["agent", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("agent"));
}

#[test]
fn test_agent_prompt_runs_headless_without_basic_flag() {
    gestura()
        .args(["agent", "--prompt", "/help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commands"))
        .stdout(predicate::str::contains("Session saved."));
}

#[test]
fn test_agent_prompt_file_executes_single_headless_command() {
    let home = isolated_home_dir();
    let prompt_path = home.join("prompt.txt");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(&prompt_path, "/task create root_task from prompt file\n").unwrap();

    let output = gestura_with_home(&home)
        .args([
            "agent",
            "--prompt-file",
            prompt_path
                .to_str()
                .expect("prompt path should be valid utf-8"),
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let sessions_dir = gestura_data_dir(&home).join("agent_sessions");
    let mut session_files = std::fs::read_dir(&sessions_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    session_files.sort();
    assert_eq!(session_files.len(), 1, "expected exactly one saved session");

    let session_id = session_files[0]
        .file_stem()
        .and_then(|value| value.to_str())
        .expect("session file should have a valid stem")
        .to_string();

    let tasks_path = gestura_data_dir(&home)
        .join("tasks")
        .join(format!("{session_id}.json"));
    assert!(
        tasks_path.exists(),
        "expected task file at {}",
        tasks_path.display()
    );

    let task_list: Value = serde_json::from_str(&std::fs::read_to_string(&tasks_path).unwrap())
        .expect("task file should contain valid json");
    let tasks = task_list["tasks"]
        .as_array()
        .expect("task list should contain a tasks array");
    assert!(tasks.iter().any(|task| {
        task["name"].as_str() == Some("root_task")
            && task["description"].as_str() == Some("from prompt file")
    }));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Created task"));
    assert!(stdout.contains("Session saved."));
}

#[test]
fn test_agent_basic_accepts_piped_slash_commands() {
    gestura()
        .args(["agent", "--basic"])
        .write_stdin("/help\n/history\n/quit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Commands"))
        .stdout(predicate::str::contains("Session Statistics"))
        .stdout(predicate::str::contains("Session saved."));
}

#[test]
fn test_agent_basic_task_commands_persist_to_global_task_store() {
    let home = isolated_home_dir();
    let output = gestura_with_home(&home)
        .args(["agent", "--basic"])
        .write_stdin("/task create root_task verify persistence\n/quit\n")
        .assert()
        .success()
        .get_output()
        .clone();

    let sessions_dir = gestura_data_dir(&home).join("agent_sessions");
    let mut session_files = std::fs::read_dir(&sessions_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    session_files.sort();
    assert_eq!(session_files.len(), 1, "expected exactly one saved session");

    let session_id = session_files[0]
        .file_stem()
        .and_then(|value| value.to_str())
        .expect("session file should have a valid stem")
        .to_string();

    let tasks_path = gestura_data_dir(&home)
        .join("tasks")
        .join(format!("{session_id}.json"));
    assert!(
        tasks_path.exists(),
        "expected task file at {}",
        tasks_path.display()
    );

    let task_list: Value = serde_json::from_str(&std::fs::read_to_string(&tasks_path).unwrap())
        .expect("task file should contain valid json");
    let tasks = task_list["tasks"]
        .as_array()
        .expect("task list should contain a tasks array");
    assert!(tasks.iter().any(|task| {
        task["name"].as_str() == Some("root_task")
            && task["description"].as_str() == Some("verify persistence")
    }));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Created task"));
    assert!(stdout.contains("Session saved."));
}

#[test]
fn test_agent_basic_current_task_updates_session_working_memory() {
    let home = isolated_home_dir();
    gestura_with_home(&home)
        .args(["agent", "--basic"])
        .write_stdin("/task create root_task sync current task\n/quit\n")
        .assert()
        .success();

    let sessions_dir = gestura_data_dir(&home).join("agent_sessions");
    let mut session_files = std::fs::read_dir(&sessions_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    session_files.sort();
    assert_eq!(session_files.len(), 1, "expected exactly one saved session");

    let session_path = &session_files[0];
    let session_id = session_path
        .file_stem()
        .and_then(|value| value.to_str())
        .expect("session file should have a valid stem")
        .to_string();

    let tasks_path = gestura_data_dir(&home)
        .join("tasks")
        .join(format!("{session_id}.json"));
    let task_list: Value = serde_json::from_str(&std::fs::read_to_string(&tasks_path).unwrap())
        .expect("task file should contain valid json");
    let task_id = task_list["tasks"]
        .as_array()
        .and_then(|tasks| tasks.first())
        .and_then(|task| task["id"].as_str())
        .expect("expected a created task id")
        .to_string();

    gestura_with_home(&home)
        .args(["agent", "--basic", "--resume", "--session", &session_id])
        .write_stdin(format!("/task current set {task_id}\n/quit\n"))
        .assert()
        .success();

    let session_json: Value = serde_json::from_str(&std::fs::read_to_string(session_path).unwrap())
        .expect("session file should contain valid json");
    assert_eq!(
        session_json["state"]["working_memory"]["active_task_id"].as_str(),
        Some(task_id.as_str())
    );
}

// ==================== Tools Permissions Command Tests ====================

#[test]
fn test_tools_permissions_grant_help() {
    // Permissions grant help should work
    gestura()
        .args(["tools", "permissions", "grant", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Grant"));
}

#[test]
fn test_tools_permissions_check_help() {
    // Permissions check help should work
    gestura()
        .args(["tools", "permissions", "check", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Check"));
}
