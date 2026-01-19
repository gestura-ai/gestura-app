//! CLI integration tests for gestura
//!
//! These tests verify that CLI commands work correctly end-to-end.

use assert_cmd::Command;
use predicates::prelude::*;

/// Get a Command for the gestura binary
#[allow(deprecated)] // cargo_bin is deprecated but we need it for tests
fn gestura() -> Command {
    Command::cargo_bin("gestura").unwrap()
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
        .stdout(predicate::str::contains("chat"))
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
    gestura().args(["agent", "list"]).assert().success();
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
fn test_chat_help() {
    // Chat subcommand help should work
    gestura()
        .args(["chat", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("chat"));
}
