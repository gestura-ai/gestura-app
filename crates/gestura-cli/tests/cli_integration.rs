//! CLI integration tests for gestura
//!
//! These tests verify that CLI commands work correctly end-to-end.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
    dir.push(format!("run-{}", nanos));

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
