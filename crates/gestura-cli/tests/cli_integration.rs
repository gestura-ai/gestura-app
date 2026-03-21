//! CLI integration tests for gestura
//!
//! These tests verify that CLI commands work correctly end-to-end.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ISOLATED_HOME_ID: AtomicU64 = AtomicU64::new(0);

/// Get a Command for the gestura binary
#[allow(deprecated)] // cargo_bin is deprecated but we need it for tests
fn gestura() -> Command {
    // IMPORTANT: these integration tests run the *real* gestura binary, which
    // depends on gestura-core as a normal dependency (cfg(test) is false there).
    // Under `--all-features`, the `security` feature enables OS keychain access,
    // which can block/hang in non-interactive contexts.
    //
    // We disable keychain usage and isolate HOME so tests are deterministic.
    let mut cmd = Command::cargo_bin("gestura").unwrap();

    cmd.env("GESTURA_DISABLE_KEYCHAIN", "1");

    let home = isolated_home_dir();
    // `dirs::home_dir()` checks different env vars per platform.
    cmd.env("HOME", &home);
    cmd.env("USERPROFILE", &home);
    cmd.env("HOMEDRIVE", "C:");
    cmd.env("HOMEPATH", "\\");

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

    let output = gestura()
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("HOMEDRIVE", "C:")
        .env("HOMEPATH", "\\")
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

    let sessions_dir = home.join(".gestura").join("agent_sessions");
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

    let tasks_path = home
        .join(".gestura")
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
    let output = gestura()
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("HOMEDRIVE", "C:")
        .env("HOMEPATH", "\\")
        .args(["agent", "--basic"])
        .write_stdin("/task create root_task verify persistence\n/quit\n")
        .assert()
        .success()
        .get_output()
        .clone();

    let sessions_dir = home.join(".gestura").join("agent_sessions");
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

    let tasks_path = home
        .join(".gestura")
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
    gestura()
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("HOMEDRIVE", "C:")
        .env("HOMEPATH", "\\")
        .args(["agent", "--basic"])
        .write_stdin("/task create root_task sync current task\n/quit\n")
        .assert()
        .success();

    let sessions_dir = home.join(".gestura").join("agent_sessions");
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

    let tasks_path = home
        .join(".gestura")
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

    gestura()
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("HOMEDRIVE", "C:")
        .env("HOMEPATH", "\\")
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
