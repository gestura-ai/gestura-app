use super::{AgentPipeline, emit_streaming_tool_keepalive};
use crate::config::AppConfig;
use crate::pipeline::Instant;
use crate::pipeline::{ToolCallRecord, ToolResult};
use crate::session_workspace::SessionWorkspace;
use serde_json::json;
use tempfile::TempDir;

#[cfg(not(target_os = "windows"))]
const STREAMING_SHELL_TOOL_TEST_TIMEOUT: tokio::time::Duration =
    tokio::time::Duration::from_secs(20);
#[cfg(not(target_os = "windows"))]
const STREAMING_SHELL_TOOL_SHUTDOWN_TIMEOUT: tokio::time::Duration =
    tokio::time::Duration::from_secs(5);

#[cfg(not(target_os = "windows"))]
fn silent_shell_test_command() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "ping -n 3 127.0.0.1 >nul"
    }

    #[cfg(not(target_os = "windows"))]
    {
        "sleep 1"
    }
}

#[cfg(not(target_os = "windows"))]
fn cwd_echo_command(relative_dir: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        format!("cd {relative_dir} && cd")
    }

    #[cfg(not(target_os = "windows"))]
    {
        format!("cd {relative_dir} && pwd")
    }
}

#[cfg(not(target_os = "windows"))]
async fn shutdown_shell_session_for_test(pool_key: &str) {
    tokio::time::timeout(
        STREAMING_SHELL_TOOL_SHUTDOWN_TIMEOUT,
        crate::tools::shell_sessions::shutdown_session(pool_key),
    )
    .await
    .expect("timed out shutting down PTY session pool")
    .expect("shutdown PTY session pool");
}

#[cfg(not(target_os = "windows"))]
fn spawn_stream_collector(
    mut rx: tokio::sync::mpsc::Receiver<gestura_core_streaming::StreamChunk>,
) -> tokio::task::JoinHandle<Vec<gestura_core_streaming::StreamChunk>> {
    tokio::spawn(async move {
        let mut chunks = Vec::new();
        while let Some(chunk) = rx.recv().await {
            chunks.push(chunk);
        }
        chunks
    })
}

#[test]
fn normalize_task_tool_arguments_recovers_embedded_parameter_fragments() {
    let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
        "operation": "create",
        "parent_id": "None",
        "status": "notstarted",
        "task_id": "None</parameter><parameter name=\"name\">Create Hello World GUI Application</parameter>\n<parameter name=\"description\">Plan, implement, build, and test a small GUI that displays \"Hello World\"</parameter>",
    }));

    assert_eq!(
        normalized.get("operation").and_then(|v| v.as_str()),
        Some("create")
    );
    assert_eq!(normalized.get("parent_id"), None);
    assert_eq!(normalized.get("task_id"), None);
    assert_eq!(
        normalized.get("name").and_then(|v| v.as_str()),
        Some("Create Hello World GUI Application")
    );
    assert_eq!(
        normalized.get("description").and_then(|v| v.as_str()),
        Some("Plan, implement, build, and test a small GUI that displays \"Hello World\"")
    );
}

#[test]
fn normalize_task_tool_arguments_recovers_operation_from_embedded_parameter_payload() {
    let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
        "operation": "\"create\"\n<parameter name=\"name\">Plan app</parameter>\n<parameter name=\"parent_id\">e4d5f1a0-c20d-4562-aef3-b1ca3ffbdb8c</parameter>"
    }));

    assert_eq!(
        normalized.get("operation").and_then(|v| v.as_str()),
        Some("create")
    );
    assert_eq!(
        normalized.get("name").and_then(|v| v.as_str()),
        Some("Plan app")
    );
    assert_eq!(
        normalized.get("parent_id").and_then(|v| v.as_str()),
        Some("e4d5f1a0-c20d-4562-aef3-b1ca3ffbdb8c")
    );
}

#[test]
fn strip_parameter_fragments_keeps_plain_text_only() {
    let raw = "None<parameter name=\"name\">Task A</parameter> trailing";
    assert_eq!(
        AgentPipeline::strip_parameter_fragments(raw),
        "None trailing"
    );
}

#[test]
fn normalize_task_tool_arguments_sanitizes_malformed_task_ids() {
    let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
        "operation": "update_status",
        "task_id": "5226626b-3fbf-4570-b717-387dc9492f51\\\" ",
    }));

    assert_eq!(
        normalized.get("task_id").and_then(|v| v.as_str()),
        Some("5226626b-3fbf-4570-b717-387dc9492f51")
    );
}

#[test]
fn normalize_task_tool_arguments_recovers_create_name_from_slug_like_task_id() {
    let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
        "operation": "create",
        "task_id": "setup-gui-env",
        "parent_id": "e4d5f1a0-c20d-4562-aef3-b1ca3ffbdb8c"
    }));

    assert_eq!(
        normalized.get("name").and_then(|v| v.as_str()),
        Some("Setup Gui Env")
    );
    assert!(normalized.get("task_id").is_none());
}

#[test]
fn normalize_task_tool_arguments_does_not_recover_create_name_from_single_word_task_id() {
    let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
        "operation": "create",
        "task_id": "malformed"
    }));

    assert!(normalized.get("name").is_none());
    assert!(normalized.get("task_id").is_none());
}

#[test]
fn normalize_task_tool_arguments_does_not_derive_create_name_from_natural_language_task_id() {
    let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
        "operation": "create",
        "task_id": "\"hello-world-gui\" wait no, task_id not for create."
    }));

    assert!(normalized.get("name").is_none());
    assert!(normalized.get("task_id").is_none());
}

#[test]
fn normalize_task_tool_arguments_recovers_create_name_from_title_alias() {
    let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
        "operation": "create",
        "title": "inspect readme and summarize findings",
        "parent_id": "e4d5f1a0-c20d-4562-aef3-b1ca3ffbdb8c"
    }));

    assert_eq!(
        normalized.get("name").and_then(|v| v.as_str()),
        Some("Inspect Readme And Summarize Findings")
    );
}

#[test]
fn normalize_task_tool_arguments_recovers_update_aliases() {
    let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
        "operation": "update",
        "id": "abc123\" ",
        "title": "Refine onboarding task",
        "desc": "Add regression coverage for update aliases"
    }));

    assert_eq!(
        normalized.get("task_id").and_then(|v| v.as_str()),
        Some("abc123")
    );
    assert_eq!(
        normalized.get("name").and_then(|v| v.as_str()),
        Some("Refine onboarding task")
    );
    assert_eq!(
        normalized.get("description").and_then(|v| v.as_str()),
        Some("Add regression coverage for update aliases")
    );
}

#[test]
fn normalize_task_tool_arguments_recovers_delete_task_id_from_id_alias() {
    let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
        "operation": "delete",
        "id": "abc123\" "
    }));

    assert_eq!(
        normalized.get("task_id").and_then(|v| v.as_str()),
        Some("abc123")
    );
}

#[test]
fn default_shell_timeout_extends_build_commands() {
    assert_eq!(
        AgentPipeline::default_shell_timeout_secs("cargo check"),
        300
    );
    assert_eq!(
        AgentPipeline::default_shell_timeout_secs("npm install"),
        300
    );
    assert_eq!(
        AgentPipeline::default_shell_timeout_secs("printf hello"),
        60
    );
}

#[test]
fn default_shell_long_running_mode_tracks_build_commands() {
    assert!(AgentPipeline::default_shell_long_running_allowed(
        "cargo test -p gestura-gui"
    ));
    assert!(AgentPipeline::default_shell_long_running_allowed(
        "npm run build"
    ));
    assert!(!AgentPipeline::default_shell_long_running_allowed(
        "printf hello"
    ));
}

#[test]
fn effective_shell_timeout_clamps_long_running_commands() {
    assert_eq!(
        AgentPipeline::effective_shell_timeout_secs("cargo test -p gestura-gui", Some(120)),
        300
    );
    assert_eq!(
        AgentPipeline::effective_shell_timeout_secs("cargo build --workspace", Some(900)),
        900
    );
    assert_eq!(
        AgentPipeline::effective_shell_timeout_secs("printf hello", Some(5)),
        5
    );
}

#[test]
fn effective_shell_long_running_mode_respects_explicit_override() {
    assert!(!AgentPipeline::effective_shell_long_running_allowed(
        "cargo test",
        Some(false)
    ));
    assert!(AgentPipeline::effective_shell_long_running_allowed(
        "printf hello",
        Some(true)
    ));
}

#[test]
fn harden_noninteractive_shell_command_normalizes_scaffold_command() {
    let (command, env) = AgentPipeline::harden_noninteractive_shell_command(
        "npx create-project-app@latest hello-world --template basic",
        None,
    );

    assert!(command.contains("--yes"));
    let env = env.expect("env should be injected");
    assert_eq!(env.get("CI"), Some(&"true".to_string()));
    assert_eq!(env.get("FORCE_COLOR"), Some(&"0".to_string()));
}

#[test]
fn harden_noninteractive_shell_command_does_not_mutate_echoed_prompt_text() {
    let command = "printf 'Need to install the following packages:\ncreate-project-app@4.6.2\nOk to proceed? (y)\n'; sleep 2";
    let (hardened_command, env) = AgentPipeline::harden_noninteractive_shell_command(command, None);

    assert_eq!(hardened_command, command);
    assert!(env.is_none());
}

#[test]
fn strip_redundant_shell_cwd_prefix_removes_matching_leading_cd() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("sample-app");
    std::fs::create_dir_all(&project_dir).expect("create project dir");
    let workspace = SessionWorkspace::from_directory("shell-strip-cwd", temp.path().to_path_buf())
        .expect("workspace");

    let command = AgentPipeline::strip_redundant_shell_cwd_prefix(
        Some(&workspace),
        "cd sample-app && python -m build",
        Some(project_dir.to_string_lossy().as_ref()),
    );

    assert_eq!(command, "python -m build");
}

#[test]
fn strip_redundant_shell_cwd_prefix_preserves_nonmatching_leading_cd() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("sample-app");
    let other_dir = temp.path().join("other-dir");
    std::fs::create_dir_all(&project_dir).expect("create project dir");
    std::fs::create_dir_all(&other_dir).expect("create other dir");
    let workspace = SessionWorkspace::from_directory("shell-keep-cwd", temp.path().to_path_buf())
        .expect("workspace");

    let command = AgentPipeline::strip_redundant_shell_cwd_prefix(
        Some(&workspace),
        "cd other-dir && npm install --silent",
        Some(project_dir.to_string_lossy().as_ref()),
    );

    assert_eq!(command, "cd other-dir && npm install --silent");
}

#[test]
fn normalize_task_tool_arguments_recovers_unclosed_status_fragment() {
    let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
        "operation": "update_status",
        "task_id": "1c0a1ed3-e355-4117-9881-3632a2765199\"  <!-- Install GUI prerequisites -->\n<parameter name=\"status\">inprogress",
    }));

    assert_eq!(
        normalized.get("task_id").and_then(|v| v.as_str()),
        Some("1c0a1ed3-e355-4117-9881-3632a2765199")
    );
    assert_eq!(
        normalized.get("status").and_then(|v| v.as_str()),
        Some("inprogress")
    );
}

#[test]
fn normalize_task_tool_arguments_recovers_embedded_natural_language_status() {
    let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
        "operation": "update_status",
        "task_id": "a519ef62-9279-46c0-a650-6c5bd644d107\" status is completed",
    }));

    assert_eq!(
        normalized.get("task_id").and_then(|v| v.as_str()),
        Some("a519ef62-9279-46c0-a650-6c5bd644d107")
    );
    assert_eq!(
        normalized.get("status").and_then(|v| v.as_str()),
        Some("completed")
    );
}

#[test]
fn normalize_file_tool_arguments_sanitizes_paths_and_preserves_canonical_edit_fields() {
    let normalized = AgentPipeline::normalize_file_tool_arguments(json!({
        "operation": "EDIT",
        "path": "\"app/main.py\"",
        "old": "print('hello')",
        "new": "print('hello world')",
    }));

    assert_eq!(
        normalized.get("operation").and_then(|v| v.as_str()),
        Some("edit")
    );
    assert_eq!(
        normalized.get("path").and_then(|v| v.as_str()),
        Some("app/main.py")
    );
    assert_eq!(
        normalized.get("old").and_then(|v| v.as_str()),
        Some("print('hello')")
    );
    assert_eq!(
        normalized.get("new").and_then(|v| v.as_str()),
        Some("print('hello world')")
    );
}

#[test]
fn normalize_file_tool_arguments_recovers_common_edit_aliases() {
    let normalized = AgentPipeline::normalize_file_tool_arguments(json!({
        "operation": "edit",
        "path": "service/main.py",
        "old_str": "print('hello')",
        "replacement": "print('hello world')",
    }));

    assert_eq!(
        normalized.get("old").and_then(|v| v.as_str()),
        Some("print('hello')")
    );
    assert_eq!(
        normalized.get("new").and_then(|v| v.as_str()),
        Some("print('hello world')")
    );
}

#[test]
fn normalize_file_tool_arguments_recovers_write_content_aliases() {
    let normalized = AgentPipeline::normalize_file_tool_arguments(json!({
        "operation": "write",
        "path": "docs/summary.txt",
        "text": "Release summary",
    }));

    assert_eq!(
        normalized.get("content").and_then(|v| v.as_str()),
        Some("Release summary")
    );
    assert_eq!(
        normalized.get("text").and_then(|v| v.as_str()),
        Some("Release summary")
    );
}

#[test]
fn normalize_file_tool_arguments_does_not_recover_inline_edit_replacement() {
    let normalized = AgentPipeline::normalize_file_tool_arguments(json!({
        "operation": "edit",
        "path": "sample-app/app/main.py",
        "pattern": "None",
        "start": "1.0\" No. The correct is: old is print('hello') new is print('hello world')"
    }));

    assert_eq!(
        normalized.get("operation").and_then(|v| v.as_str()),
        Some("edit")
    );
    assert!(normalized.get("old").is_none());
    assert!(normalized.get("new").is_none());
}

#[test]
fn normalize_tool_arguments_for_execution_keeps_strict_file_write_shape() {
    let normalized = AgentPipeline::normalize_tool_arguments_for_execution(
        "file",
        &json!({
            "operation": "write",
            "path": "sample-app/app/main.py",
            "pattern": "None",
            "recursive": false,
            "show_hidden": false,
            "start": 1,
        })
        .to_string(),
    );

    let normalized = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        normalized.get("operation").and_then(|v| v.as_str()),
        Some("write")
    );
    assert_eq!(
        normalized.get("path").and_then(|v| v.as_str()),
        Some("sample-app/app/main.py")
    );
    assert_eq!(
        normalized.get("pattern").and_then(|v| v.as_str()),
        Some("None")
    );
}

#[test]
fn normalize_tool_arguments_for_execution_keeps_strict_code_batch_edit_shape() {
    let normalized = AgentPipeline::normalize_tool_arguments_for_execution(
        "code",
        &json!({
            "operation": "batch_edit",
            "path": "sample-app/app/main.py",
            "pattern": "None",
            "symbol": "None",
        })
        .to_string(),
    );

    let normalized = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        normalized.get("operation").and_then(|v| v.as_str()),
        Some("batch_edit")
    );
    assert_eq!(
        normalized.get("path").and_then(|v| v.as_str()),
        Some("sample-app/app/main.py")
    );
    assert!(normalized.get("paths").is_none());
    assert!(normalized.get("edits").is_none());
}

#[test]
fn normalize_tool_arguments_for_execution_forces_split_code_tool_operation() {
    let normalized = AgentPipeline::normalize_tool_arguments_for_execution(
        "code_edit_files",
        &json!({
            "operation": "stats",
            "edits": [{
                "path": "src/lib.rs",
                "old_str": "fn greet() {}",
                "new_str": "fn greet() { println!(\"hello\"); }"
            }]
        })
        .to_string(),
    );

    let normalized = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        normalized.get("operation").and_then(|v| v.as_str()),
        Some("batch_edit")
    );
    assert!(normalized.get("edits").and_then(|v| v.as_array()).is_some());
}

#[test]
fn normalize_tool_arguments_for_execution_forces_split_file_tool_operation() {
    let normalized = AgentPipeline::normalize_tool_arguments_for_execution(
        "edit_file",
        &json!({
            "operation": "search",
            "path": "src/lib.rs",
            "pattern": "fn greet() {}",
            "replacement": "fn greet() { println!(\"hello\"); }"
        })
        .to_string(),
    );

    let normalized = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        normalized.get("operation").and_then(|v| v.as_str()),
        Some("edit")
    );
    assert_eq!(
        normalized.get("old").and_then(|v| v.as_str()),
        Some("fn greet() {}")
    );
    assert_eq!(
        normalized.get("new").and_then(|v| v.as_str()),
        Some("fn greet() { println!(\"hello\"); }")
    );
}

#[test]
fn normalize_code_tool_arguments_does_not_recover_batch_edit_aliases() {
    let normalized = AgentPipeline::normalize_code_tool_arguments(json!({
        "operation": "edit",
        "changes": [{
            "file": "\"app/main.py\"",
            "old": "print('hello')",
            "replacement": "print('hello world')",
        }]
    }));

    assert_eq!(
        normalized.get("operation").and_then(|v| v.as_str()),
        Some("edit")
    );
    assert!(normalized.get("edits").is_none());
    assert!(normalized.get("changes").is_some());
}

#[test]
fn normalize_tool_arguments_for_execution_drops_placeholder_task_create_name() {
    let normalized = AgentPipeline::normalize_tool_arguments_for_execution(
        "task",
        &json!({
            "operation": "create",
            "name": "None But Omit",
            "description": "placeholder"
        })
        .to_string(),
    );

    let normalized = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        normalized.get("operation").and_then(|v| v.as_str()),
        Some("create")
    );
    assert!(normalized.get("name").is_none());
}

#[test]
fn normalize_file_tool_arguments_recovers_operation_from_embedded_parameter_payload() {
    let normalized = AgentPipeline::normalize_file_tool_arguments(json!({
        "operation": "\"list\"\n<parameter name=\"path\">.</parameter>"
    }));

    assert_eq!(
        normalized.get("operation").and_then(|v| v.as_str()),
        Some("list")
    );
    assert_eq!(normalized.get("path").and_then(|v| v.as_str()), Some("."));
}

#[test]
fn missing_file_write_content_error_explains_how_to_recover() {
    let message = AgentPipeline::format_missing_file_write_content_error(&json!({
        "operation": "write",
        "path": "sample-app/app/main.py",
        "pattern": "none",
        "start": 1,
    }));

    assert!(message.contains("Missing required field 'content' for file write operation"));
    assert!(message.contains("pattern, start"));
    assert!(message.contains("\"content\":\"<full file contents here>\""));
    assert!(message.contains("Do not retry the same malformed write call"));
}

#[test]
fn missing_file_edit_replacement_error_explains_how_to_recover() {
    let message = AgentPipeline::format_missing_file_edit_replacement_error(
        &json!({
            "operation": "edit",
            "path": "sample-app/app/main.py",
            "pattern": "print('hello')",
        }),
        "new",
    );

    assert!(message.contains("Missing required field 'new' for file edit operation"));
    assert!(message.contains("Provided fields: operation, path, pattern"));
    assert!(message.contains("\"old\":\"<exact existing text>\""));
    assert!(message.contains("\"new\":\"<replacement text>\""));
    assert!(message.contains("`old_str`, `new_str`, `pattern`, or `replacement`"));
}

#[test]
fn missing_task_update_status_error_explains_how_to_recover() {
    let message = AgentPipeline::format_missing_task_update_status_error(&json!({
        "operation": "update_status",
        "task_id": "28d3bedc-81b9-45d2-a311-ccbb7d3be111",
    }));

    assert!(message.contains("Missing required field 'status' for update_status operation"));
    assert!(message.contains("`update_status` requires both `task_id` and `status`"));
    assert!(message.contains("\"status\":\"inprogress\""));
    assert!(message.contains(
        "Do not omit `status` to ask the runtime to infer or preserve the current state"
    ));
}

#[test]
fn missing_task_create_name_error_explains_how_to_recover() {
    let message = AgentPipeline::format_missing_task_create_name_error(&json!({
        "operation": "create",
        "parent_id": "root-123",
        "status": "notstarted",
    }));

    assert!(message.contains("Missing required field 'name' for create operation"));
    assert!(message.contains("`create` requires a specific task `name`"));
    assert!(message.contains("\"parent_id\":\"root-123\""));
    assert!(message.contains("\"status\":\"notstarted\""));
    assert!(message.contains("\"name\":\"Build hello world GUI app\""));
    assert!(message.contains(
            "Do not rely on the runtime to invent or preserve placeholder names like 'Untitled Task' or 'None But Omit'"
        ));
}

#[test]
fn missing_task_update_fields_error_explains_how_to_recover() {
    let message = AgentPipeline::format_missing_task_update_fields_error(&json!({
        "operation": "update",
        "task_id": "abc123",
        "status": "completed",
    }));

    assert!(message.contains("Missing required update fields for update operation"));
    assert!(message.contains("at least one of `name` or `description`"));
    assert!(message.contains("\"task_id\":\"abc123\""));
    assert!(message.contains("use `update_status` with both `task_id` and `status` instead"));
}

#[test]
fn missing_code_batch_edit_edits_error_explains_how_to_recover() {
    let message = AgentPipeline::format_missing_code_batch_edit_edits_error(&json!({
        "operation": "batch_edit",
        "path": "src/lib.rs",
        "note": "replace the greeting",
    }));

    assert!(message.contains("Missing required field 'edits' for code batch_edit operation"));
    assert!(message.contains("`batch_edit` requires an `edits` array"));
    assert!(message.contains("\"operation\":\"batch_edit\""));
    assert!(
        message.contains("Do not substitute top-level fields like `path`, `pattern`, or `symbol`")
    );
}

#[test]
fn repeated_malformed_tool_call_skip_message_trips_on_second_attempt() {
    let malformed_args = json!({
        "operation": "write",
        "path": "sample-app/app/main.py",
        "pattern": "replace the greeting later",
        "start": 1,
    })
    .to_string();

    let prior_records = [crate::pipeline::ToolCallRecord {
        id: "1".to_string(),
        name: "file".to_string(),
        arguments: malformed_args.clone(),
        result: crate::pipeline::ToolResult::Error(
            AgentPipeline::format_missing_file_write_content_error(&json!({
                "operation": "write",
                "path": "sample-app/app/main.py",
                "pattern": "replace the greeting later",
                "start": 1,
            })),
        ),
        duration_ms: 1,
    }];

    let message = AgentPipeline::repeated_malformed_tool_call_skip_message(
        "file",
        &malformed_args,
        prior_records.iter(),
    )
    .expect("loop breaker should trigger");

    assert!(message.contains("Loop breaker:"));
    assert!(message.contains("agent is still running"));
    assert!(message.contains(
            "Do not retry `write_file` until you can provide the full destination file text in `content`"
        ));
}

#[test]
fn repeated_malformed_tool_call_skip_message_does_not_trip_on_first_attempt() {
    let malformed_args = json!({
        "operation": "write",
        "path": "sample-app/app/main.py",
        "pattern": "none",
        "start": 1,
    })
    .to_string();

    assert!(
        AgentPipeline::repeated_malformed_tool_call_skip_message("file", &malformed_args, [])
            .is_none()
    );
}

#[test]
fn repeated_malformed_file_edit_without_replacement_does_not_trip_on_first_attempt() {
    let malformed_args = json!({
        "operation": "edit",
        "path": "sample-app/app/main.py",
        "pattern": "print('hello')",
        "start": 1
    })
    .to_string();

    assert!(
        AgentPipeline::repeated_malformed_tool_call_skip_message("file", &malformed_args, [])
            .is_none()
    );
}

#[test]
fn repeated_malformed_task_update_without_status_trips_on_second_attempt() {
    let malformed_args = json!({
        "operation": "update_status",
        "task_id": "6073f304-388d-408c-82d0-f49f8679656a",
    })
    .to_string();

    let prior_records = [crate::pipeline::ToolCallRecord {
        id: "1".to_string(),
        name: "task".to_string(),
        arguments: malformed_args.clone(),
        result: crate::pipeline::ToolResult::Error(
            AgentPipeline::format_missing_task_update_status_error(&json!({
                "operation": "update_status",
                "task_id": "6073f304-388d-408c-82d0-f49f8679656a",
            })),
        ),
        duration_ms: 1,
    }];

    let message = AgentPipeline::repeated_malformed_tool_call_skip_message(
        "task",
        &malformed_args,
        prior_records.iter(),
    )
    .expect("loop breaker should trigger");

    assert!(message.contains("Loop breaker:"));
    assert!(message.contains("task.update_status"));
    assert!(message.contains("agent is still running"));
    assert!(message.contains("Do not retry `update_status` without `status`"));
}

#[test]
fn repeated_omitted_status_task_update_trips_after_prior_malformed_skip() {
    let malformed_args = json!({
        "operation": "update_status",
        "task_id": "6073f304-388d-408c-82d0-f49f8679656a",
    })
    .to_string();

    let prior_records = [crate::pipeline::ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: malformed_args.clone(),
            result: crate::pipeline::ToolResult::Skipped(
                "Skipped malformed `task.update_status` without explicit `status`; retry only with both `task_id` and `status`."
                    .to_string(),
            ),
            duration_ms: 1,
        }];

    let message = AgentPipeline::repeated_malformed_tool_call_skip_message(
        "task",
        &malformed_args,
        prior_records.iter(),
    )
    .expect("loop breaker should trigger after prior no-op success");

    assert!(message.contains("Loop breaker:"));
    assert!(message.contains("task.update_status"));
    assert!(message.contains("Do not retry `update_status` without `status`"));
}

#[test]
fn repeated_malformed_task_update_without_fields_trips_on_second_attempt() {
    let malformed_args = json!({
        "operation": "update",
        "task_id": "6073f304-388d-408c-82d0-f49f8679656a",
        "status": "completed",
    })
    .to_string();

    let prior_records = [crate::pipeline::ToolCallRecord {
        id: "1".to_string(),
        name: "task".to_string(),
        arguments: malformed_args.clone(),
        result: crate::pipeline::ToolResult::Error(
            AgentPipeline::format_missing_task_update_fields_error(&json!({
                "operation": "update",
                "task_id": "6073f304-388d-408c-82d0-f49f8679656a",
                "status": "completed",
            })),
        ),
        duration_ms: 1,
    }];

    let message = AgentPipeline::repeated_malformed_tool_call_skip_message(
        "task",
        &malformed_args,
        prior_records.iter(),
    )
    .expect("loop breaker should trigger");

    assert!(message.contains("Loop breaker:"));
    assert!(message.contains("task.update"));
    assert!(message.contains("Do not retry `update` without at least one field to change"));
    assert!(message.contains("use `update_status` with both `task_id` and `status`"));
}

#[test]
fn repeated_malformed_task_create_without_name_trips_on_second_attempt() {
    let malformed_args = json!({
        "operation": "create",
        "task_id": "\"sample-app\" wait no, task_id not for create.",
    })
    .to_string();

    let prior_records = vec![crate::pipeline::ToolCallRecord {
        id: "1".to_string(),
        name: "task".to_string(),
        arguments: malformed_args.clone(),
        result: crate::pipeline::ToolResult::Error(
            AgentPipeline::format_missing_task_create_name_error(&json!({
                "operation": "create",
                "task_id": "\"sample-app\" wait no, task_id not for create.",
            })),
        ),
        duration_ms: 1,
    }];

    let message = AgentPipeline::repeated_malformed_tool_call_skip_message(
        "task",
        &malformed_args,
        &prior_records,
    )
    .expect("second malformed create should trip loop breaker");

    assert!(message.contains("Loop breaker:"));
    assert!(message.contains("task.create"));
    assert!(message.contains("without a valid `name`"));
}

#[test]
fn repeated_malformed_code_batch_edit_without_edits_does_not_trip_on_first_attempt() {
    let malformed_args = json!({
        "operation": "batch_edit",
        "path": "src/lib.rs",
        "pattern": "fn greet() {}",
        "note": "replace the heading",
    })
    .to_string();

    assert!(
        AgentPipeline::repeated_malformed_tool_call_skip_message("code", &malformed_args, [])
            .is_none()
    );
}

#[test]
fn repeated_redundant_project_init_trips_after_first_success() {
    let args = json!({
        "command": "cargo new hello-world --bin"
    })
    .to_string();
    let prior_records = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: args.clone(),
        result: ToolResult::Success(String::new()),
        duration_ms: 1,
    }];

    let message =
        AgentPipeline::repeated_redundant_tool_call_skip_message("shell", &args, &prior_records)
            .expect("second successful project init should trip loop breaker");

    assert!(message.contains("redundant repeated project scaffold/init command"));
    assert!(message.contains("move on to editing files and running build/test verification"));
}

#[test]
fn manual_project_scaffold_is_blocked_after_repeated_non_tty_failures() {
    let prior_records = vec![
            ToolCallRecord {
                id: "1".to_string(),
                name: "shell".to_string(),
                arguments: json!({
                    "command": "npx create-project-app@latest sample-app --template basic --yes"
                })
                .to_string(),
                result: ToolResult::Error("Exit 1: Shell command failed because it expected a terminal. stderr: IO error: not a terminal".to_string()),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "shell".to_string(),
                arguments: json!({
                    "command": "npx create-project-app@latest sample-app --template basic --yes"
                })
                .to_string(),
                result: ToolResult::Error("Exit 1: command likely waited for interactive input".to_string()),
                duration_ms: 1,
            },
        ];
    let manual_args = json!({
            "command": "mkdir -p sample-app/app && cat <<'EOF' > sample-app/pyproject.toml\n[project]\nname = \"sample-app\"\nEOF"
        })
        .to_string();

    let message = AgentPipeline::repeated_redundant_tool_call_skip_message(
        "shell",
        &manual_args,
        &prior_records,
    )
    .expect("manual project shell fallback should be blocked after repeated non-tty failures");

    assert!(message.contains("Do not synthesize a project structure with `cat <<EOF`"));
    assert!(message.contains("check the scaffold command's `--help` output"));
    assert!(message.contains("documented non-interactive scaffold/init command"));
}

#[test]
fn repeated_scaffold_retry_is_blocked_after_two_non_tty_failures() {
    let prior_records = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: json!({
                "command": "npx create-project-app@latest sample-app --template basic --yes"
            })
            .to_string(),
            result: ToolResult::Error("Exit 1: stderr: IO error: not a terminal".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: json!({
                "command": "npx create-project-app@latest sample-app --template basic --yes"
            })
            .to_string(),
            result: ToolResult::Error(
                "Exit 124: command likely waited for interactive input".to_string(),
            ),
            duration_ms: 1,
        },
    ];
    let retry_args = json!({
        "command": "npx create-project-app@latest sample-app --template basic --yes"
    })
    .to_string();

    let message = AgentPipeline::repeated_redundant_tool_call_skip_message(
        "shell",
        &retry_args,
        &prior_records,
    )
    .expect("same scaffold retry should be blocked after repeated non-tty failures");

    assert!(message.contains("Use one specific alternate strategy next"));
    assert!(message.contains("check the scaffold command's `--help` output"));
    assert!(message.contains("documented non-interactive scaffold/init command"));
}

#[test]
fn required_verification_retry_blocks_file_tool() {
    let message = AgentPipeline::required_verification_retry_skip_message("file")
        .expect("file should be blocked during required verification retry");

    assert!(message.contains("skipped `file`"));
    assert!(message.contains("only the `shell` tool may be used"));
}

#[test]
fn required_verification_retry_allows_shell_tool() {
    assert!(AgentPipeline::required_verification_retry_skip_message("shell").is_none());
}

#[test]
fn repeated_file_tree_inspection_trips_after_two_successes() {
    let args = json!({
        "operation": "tree",
        "path": "sample-app"
    })
    .to_string();
    let prior_records = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: args.clone(),
            result: ToolResult::Success("{}".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: args.clone(),
            result: ToolResult::Success("{}".to_string()),
            duration_ms: 1,
        },
    ];

    let message =
        AgentPipeline::repeated_redundant_tool_call_skip_message("file", &args, &prior_records)
            .expect("third identical file.tree should trip loop breaker");

    assert!(message.contains("redundant repeated `file.tree` inspection"));
    assert!(message.contains("sample-app"));
    assert!(message.contains("move on to reading the specific file you need to edit"));
}

#[test]
fn parse_task_status_accepts_common_aliases() {
    assert!(matches!(
        AgentPipeline::parse_task_status("in_progress"),
        Some(crate::TaskStatus::InProgress)
    ));
    assert!(matches!(
        AgentPipeline::parse_task_status("status is completed"),
        Some(crate::TaskStatus::Completed)
    ));
    assert!(matches!(
        AgentPipeline::parse_task_status("state: in progress"),
        Some(crate::TaskStatus::InProgress)
    ));
    assert!(matches!(
        AgentPipeline::parse_task_status("done"),
        Some(crate::TaskStatus::Completed)
    ));
    assert!(matches!(
        AgentPipeline::parse_task_status("waiting"),
        Some(crate::TaskStatus::Blocked)
    ));
}

#[tokio::test]
async fn update_status_without_status_returns_actionable_error_for_not_started_task() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("tool-dispatch-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let task = manager
        .create_task(&session_id, "Test task", "desc", None)
        .expect("create task");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_task_tool(
            &json!({
                "operation": "update_status",
                "task_id": task.id,
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Error(output) => output,
        other => panic!("expected error, got {other:?}"),
    };

    assert!(output.contains("Missing required field 'status' for update_status operation"));
    assert!(output.contains("`update_status` requires both `task_id` and `status`"));

    let task_after = manager
        .get_task(&session_id, &task.id)
        .expect("get task")
        .expect("task exists");
    assert_eq!(task_after.status, crate::TaskStatus::NotStarted);
}

#[tokio::test]
async fn create_without_name_returns_actionable_error() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("tool-dispatch-create-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let pipeline = AgentPipeline::new(AppConfig::default());

    let before = manager
        .list_tasks(&session_id)
        .expect("list tasks before")
        .len();

    let result = pipeline
        .execute_task_tool(
            &json!({
                "operation": "create",
                "task_id": "malformed",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Error(output) => output,
        other => panic!("expected error, got {other:?}"),
    };

    assert!(output.contains("Missing required field 'name' for create operation"));

    let after = manager
        .list_tasks(&session_id)
        .expect("list tasks after")
        .len();
    assert_eq!(before, after);
}

#[tokio::test]
async fn create_with_placeholder_name_returns_actionable_error() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!(
        "tool-dispatch-placeholder-create-test-{}",
        uuid::Uuid::new_v4()
    );
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let pipeline = AgentPipeline::new(AppConfig::default());

    let before = manager
        .list_tasks(&session_id)
        .expect("list tasks before")
        .len();

    let result = pipeline
        .execute_task_tool(
            &json!({
                "operation": "create",
                "name": "None But Omit",
                "description": "placeholder",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Error(output) => output,
        other => panic!("expected error, got {other:?}"),
    };

    assert!(output.contains("Missing required field 'name' for create operation"));

    let after = manager
        .list_tasks(&session_id)
        .expect("list tasks after")
        .len();
    assert_eq!(before, after);
}

#[tokio::test]
async fn update_status_without_status_returns_actionable_error_for_in_progress_leaf_task() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("tool-dispatch-autocomplete-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let task = manager
        .create_task(&session_id, "Test task", "desc", None)
        .expect("create task");
    manager
        .update_task_status(&session_id, &task.id, crate::TaskStatus::InProgress)
        .expect("seed in progress status");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_task_tool(
            &json!({
                "operation": "update_status",
                "task_id": task.id,
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Error(output) => output,
        other => panic!("expected error, got {other:?}"),
    };

    assert!(output.contains("Missing required field 'status' for update_status operation"));
    assert!(output.contains("Do not omit `status`"));

    let task_after = manager
        .get_task(&session_id, &task.id)
        .expect("get task")
        .expect("task exists");
    assert_eq!(task_after.status, crate::TaskStatus::InProgress);
}

#[tokio::test]
async fn update_status_without_status_returns_actionable_error_for_in_progress_parent_task() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("tool-dispatch-parent-skip-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let parent = manager
        .create_task(&session_id, "Parent task", "desc", None)
        .expect("create parent task");
    let child = manager
        .create_task(&session_id, "Child task", "desc", Some(parent.id.clone()))
        .expect("create child task");
    manager
        .update_task_status(&session_id, &parent.id, crate::TaskStatus::InProgress)
        .expect("seed parent in progress status");
    manager
        .update_task_status(&session_id, &child.id, crate::TaskStatus::InProgress)
        .expect("seed child in progress status");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_task_tool(
            &json!({
                "operation": "update_status",
                "task_id": parent.id,
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Error(output) => output,
        other => panic!("expected error, got {other:?}"),
    };

    assert!(output.contains("Missing required field 'status' for update_status operation"));
    assert!(output.contains("Retry with"));

    let parent_after = manager
        .get_task(&session_id, &parent.id)
        .expect("get parent task")
        .expect("parent exists");
    assert_eq!(parent_after.status, crate::TaskStatus::InProgress);
}

#[tokio::test]
async fn update_status_without_status_returns_actionable_error_for_completed_task() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("tool-dispatch-completed-noop-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let task = manager
        .create_task(&session_id, "Completed task", "desc", None)
        .expect("create task");
    manager
        .update_task_status(&session_id, &task.id, crate::TaskStatus::Completed)
        .expect("seed completed status");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_task_tool(
            &json!({
                "operation": "update_status",
                "task_id": task.id,
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Error(output) => output,
        other => panic!("expected error, got {other:?}"),
    };

    assert!(output.contains("Missing required field 'status' for update_status operation"));
    assert!(output.contains("skip the task update and continue the real work"));

    let task_after = manager
        .get_task(&session_id, &task.id)
        .expect("get task")
        .expect("task exists");
    assert_eq!(task_after.status, crate::TaskStatus::Completed);
}

#[tokio::test]
async fn file_edit_recovers_old_str_and_new_str_aliases() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-edit-alias-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello</h1>\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "edit",
                "path": "\"index.html\"",
                "old_str": "<h1>Hello</h1>",
                "new_str": "<h1>Hello, Gestura</h1>",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("index.html"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("Hello, Gestura"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn file_edit_recovers_pattern_and_replacement_aliases() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!(
        "file-edit-pattern-replacement-test-{}",
        uuid::Uuid::new_v4()
    );
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Welcome to the app</h1>\n").expect("seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "edit",
                "path": "index.html",
                "pattern": "<h1>Welcome to the app</h1>",
                "replacement": "<h1>Hello, World!</h1>",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("index.html"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("Hello, World!"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
#[cfg(not(target_os = "windows"))]
async fn file_edit_recovers_unique_nested_workspace_suffix_path() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-edit-suffix-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let project_dir = temp.path().join("hello-world").join("src");
    std::fs::create_dir_all(&project_dir).expect("create nested src dir");
    let file_path = project_dir.join("settings.json");
    std::fs::write(&file_path, "{\"greeting\":\"hello\"}\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "edit",
                "path": "src/settings.json",
                "old": "{\"greeting\":\"hello\"}",
                "new": "{\"greeting\":\"hello world\"}",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("src/settings.json"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("hello world"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
#[cfg(not(target_os = "windows"))]
async fn file_edit_recovers_flat_root_settings_path() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-edit-flat-settings-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    std::fs::create_dir_all(temp.path().join("hello-world-app")).expect("create app dir");
    let file_path = temp.path().join("hello-world-app").join("settings.json");
    std::fs::write(&file_path, "{\"greeting\":\"hello\"}\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "edit",
                "path": "hello-world-app/src/settings.json",
                "old": "{\"greeting\":\"hello\"}",
                "new": "{\"greeting\":\"hello world\"}",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("hello-world-app/settings.json"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("hello world"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn file_read_recovers_src_settings_path() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-read-src-settings-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let project_dir = temp.path().join("hello-world");
    std::fs::create_dir_all(project_dir.join("src")).expect("create src dir");
    let file_path = project_dir.join("src").join("settings.json");
    std::fs::write(&file_path, "{\"greeting\":\"hello\"}\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "read",
                "path": "hello-world/settings.json"
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("hello"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
#[cfg(not(target_os = "windows"))]
async fn file_edit_recovers_src_settings_path() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-edit-src-settings-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let project_dir = temp.path().join("hello-world");
    std::fs::create_dir_all(project_dir.join("src")).expect("create src dir");
    let file_path = project_dir.join("src").join("settings.json");
    std::fs::write(&file_path, "{\"greeting\":\"hello\"}\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "edit",
                "path": "hello-world/settings.json",
                "old": "{\"greeting\":\"hello\"}",
                "new": "{\"greeting\":\"hello world\"}",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("hello-world/src/settings.json"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("hello world"));
            assert!(!project_dir.join("settings.json").exists());
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
#[cfg(not(target_os = "windows"))]
async fn file_edit_recovers_common_source_root_main_py_path() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-edit-src-main-py-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let project_dir = temp.path().join("hello-world");
    std::fs::create_dir_all(project_dir.join("src")).expect("create src dir");
    let file_path = project_dir.join("src").join("main.py");
    std::fs::write(&file_path, "print('hello')\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "edit",
                "path": "hello-world/main.py",
                "old": "print('hello')",
                "new": "print('hello world')",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("hello-world/src/main.py"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("hello world"));
            assert!(!project_dir.join("main.py").exists());
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn file_write_prefers_existing_src_settings_path() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-write-src-settings-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let project_dir = temp.path().join("hello-world");
    std::fs::create_dir_all(project_dir.join("src")).expect("create src dir");
    let file_path = project_dir.join("src").join("settings.json");
    std::fs::write(&file_path, "{\"greeting\":\"hello\"}\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "write",
                "path": "hello-world/settings.json",
                "content": "{\"greeting\":\"hello world\"}\n",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("hello-world/settings.json"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert_eq!(updated, "{\"greeting\":\"hello world\"}\n");
            assert!(!project_dir.join("settings.json").exists());
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn file_edit_rejects_tool_chatter_contaminating_old_replacement() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-edit-chatter-sanitize-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("main.js");
    let old = "const appApi = window.appApi;\n\nlet greetInputEl;\nlet greetMsgEl;\n";
    let new = "const appApi = window.appApi;\n\nlet greetMsgEl;\n";
    std::fs::write(&file_path, old).expect("seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
            .execute_file_tool(
                &json!({
                    "operation": "edit",
                    "path": "main.js",
                    "old": format!(
                        "{}\n\nNo, again the format must be strict.\n\nI need to output only the valid XML tags without extra text in the parameters.\n<parameter name=\"new\">ignored</parameter>",
                        old
                    ),
                    "new": new,
                })
                .to_string(),
                Some(&workspace),
            )
            .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("String to replace not found in file"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert_eq!(updated, old);
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn code_batch_edit_rejects_changes_aliases_without_canonical_edits() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-edit-alias-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello</h1>\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_named_code_tool(
            "code_edit_files",
            &json!({
                "changes": [{
                    "file": "index.html",
                    "old": "<h1>Hello</h1>",
                    "new": "<h1>Hello, Gestura</h1>",
                }]
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("Missing required field 'edits'"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("<h1>Hello</h1>"));
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn code_batch_edit_recovers_unique_nested_workspace_suffix_path() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-edit-suffix-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let project_dir = temp.path().join("hello-world").join("src");
    std::fs::create_dir_all(&project_dir).expect("create nested src dir");
    let file_path = project_dir.join("settings.json");
    std::fs::write(&file_path, "{\"greeting\":\"hello\"}\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_code_tool(
            &json!({
                "operation": "batch_edit",
                "edits": [{
                    "path": "src/settings.json",
                    "old_str": "{\"greeting\":\"hello\"}",
                    "new_str": "{\"greeting\":\"hello world\"}",
                }]
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("settings.json"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("hello world"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn code_batch_edit_recovers_flat_root_settings_path() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-edit-flat-settings-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    std::fs::create_dir_all(temp.path().join("hello-world-app")).expect("create app dir");
    let file_path = temp.path().join("hello-world-app").join("settings.json");
    std::fs::write(&file_path, "{\"greeting\":\"hello\"}\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_code_tool(
            &json!({
                "operation": "batch_edit",
                "edits": [{
                    "path": "hello-world-app/src/settings.json",
                    "old_str": "{\"greeting\":\"hello\"}",
                    "new_str": "{\"greeting\":\"hello world\"}",
                }]
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("settings.json"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("hello world"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn code_batch_edit_recovers_src_settings_path() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-edit-src-settings-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let project_dir = temp.path().join("hello-world");
    std::fs::create_dir_all(project_dir.join("src")).expect("create src dir");
    let file_path = project_dir.join("src").join("settings.json");
    std::fs::write(&file_path, "{\"greeting\":\"hello\"}\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_code_tool(
            &json!({
                "operation": "batch_edit",
                "edits": [{
                    "path": "hello-world/settings.json",
                    "old_str": "{\"greeting\":\"hello\"}",
                    "new_str": "{\"greeting\":\"hello world\"}",
                }]
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("settings.json"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("hello world"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn code_batch_edit_rejects_tool_chatter_in_old_str() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-edit-chatter-sanitize-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    let old = "<h1>Welcome to the app</h1>\n";
    let new = "<h1>Hello, World!</h1>\n";
    std::fs::write(&file_path, old).expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_code_tool(
            &json!({
                "operation": "batch_edit",
                "edits": [{
                    "path": "index.html",
                    "old_str": format!(
                        "{}\n\nThe tool result has:\n\n{}\nThis will make the app say hello world.",
                        old,
                        old
                    ),
                    "new_str": new,
                }]
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("failing edit") || output.contains("old_str not found"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert_eq!(updated, old);
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn update_status_succeeds_with_sanitized_task_id() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("tool-dispatch-sanitize-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let task = manager
        .create_task(&session_id, "Test task", "desc", None)
        .expect("create task");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_task_tool(
            &json!({
                "operation": "update_status",
                "task_id": format!("{}\\\" ", task.id),
                "status": "inprogress",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Success(output) => output,
        other => panic!("expected success, got {other:?}"),
    };

    assert!(output.contains(task.id.as_str()));
    assert!(output.contains("status to InProgress"));
}

#[tokio::test]
async fn update_status_succeeds_with_unclosed_embedded_status_fragment() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("tool-dispatch-embedded-status-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let task = manager
        .create_task(&session_id, "Test task", "desc", None)
        .expect("create task");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
            .execute_task_tool(
                &json!({
                    "operation": "update_status",
                    "task_id": format!(
                        "{}\"  <!-- Install GUI prerequisites -->\n<parameter name=\"status\">inprogress",
                        task.id
                    ),
                })
                .to_string(),
                Some(&workspace),
            )
            .await;

    let output = match result {
        crate::pipeline::ToolResult::Success(output) => output,
        other => panic!("expected success, got {other:?}"),
    };

    assert!(output.contains(task.id.as_str()));
    assert!(output.contains("status to InProgress"));
}

#[tokio::test]
async fn update_status_succeeds_with_embedded_natural_language_completed_status() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("tool-dispatch-natural-status-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let task = manager
        .create_task(&session_id, "Leaf task", "desc", None)
        .expect("create task");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_task_tool(
            &json!({
                "operation": "update_status",
                "task_id": format!("{}\" status is completed", task.id),
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Success(output) => output,
        other => panic!("expected success, got {other:?}"),
    };

    assert!(output.contains(task.id.as_str()));
    assert!(output.contains("status to Completed"));
}

#[tokio::test]
async fn update_succeeds_with_id_title_and_desc_aliases() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("tool-dispatch-update-alias-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let task = manager
        .create_task(&session_id, "Original task", "Original description", None)
        .expect("create task");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_task_tool(
            &json!({
                "operation": "update",
                "id": task.id,
                "title": "Renamed task",
                "desc": "Updated description",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Success(output) => output,
        other => panic!("expected success, got {other:?}"),
    };

    assert!(output.contains("Updated task"));
    assert!(output.contains("name to 'Renamed task'"));
    assert!(output.contains("description to 'Updated description'"));

    let task_after = manager
        .get_task(&session_id, &task.id)
        .expect("get task")
        .expect("task exists");
    assert_eq!(task_after.name, "Renamed task");
    assert_eq!(task_after.description, "Updated description");
}

#[tokio::test]
async fn update_without_name_or_description_returns_helpful_error() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("tool-dispatch-update-noop-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let task = manager
        .create_task(&session_id, "Original task", "Original description", None)
        .expect("create task");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_task_tool(
            &json!({
                "operation": "update",
                "task_id": task.id,
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Error(output) => output,
        other => panic!("expected error, got {other:?}"),
    };

    assert!(output.contains("Missing required update fields for update operation"));
    assert!(output.contains("at least one of `name` or `description`"));
}

#[tokio::test]
async fn delete_succeeds_with_id_alias() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("tool-dispatch-delete-alias-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let task = manager
        .create_task(&session_id, "Delete me", "Original description", None)
        .expect("create task");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_task_tool(
            &json!({
                "operation": "delete",
                "id": task.id,
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Success(output) => output,
        other => panic!("expected success, got {other:?}"),
    };

    assert!(output.contains("Deleted task 'Delete me'"));
    let task_after = manager.get_task(&session_id, &task.id).expect("get task");
    assert!(task_after.is_none());
}

#[tokio::test]
async fn task_update_status_split_tool_dispatches_successfully() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("task-update-status-split-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let manager = crate::get_global_task_manager();
    let task = manager
        .create_task(&session_id, "Test task", "desc", None)
        .expect("create task");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_tool(
            "task_update_status",
            &json!({
                "task_id": task.id,
                "status": "completed",
            })
            .to_string(),
            Some(&workspace),
            None,
            None,
        )
        .await;

    let output = match result {
        crate::pipeline::ToolResult::Success(output) => output,
        other => panic!("expected success, got {other:?}"),
    };

    assert!(output.contains("Updated task"));
    assert!(output.contains("Completed"));

    let task_after = manager
        .get_task(&session_id, &task.id)
        .expect("get task")
        .expect("task exists");
    assert_eq!(task_after.status, crate::TaskStatus::Completed);
}

#[tokio::test]
async fn code_read_files_split_tool_reads_files() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-read-files-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello</h1>\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_named_code_tool(
            "code_read_files",
            &json!({"paths": ["index.html"]}).to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("index.html"));
            assert!(output.contains("Hello"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn code_edit_files_split_tool_applies_edits() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-edit-files-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello</h1>\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_named_code_tool(
            "code_edit_files",
            &json!({
                "edits": [{
                    "path": "index.html",
                    "old_str": "<h1>Hello</h1>",
                    "new_str": "<h1>Hello, Gestura</h1>"
                }]
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("index.html"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("Hello, Gestura"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn code_read_files_rejects_directory_paths() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-read-dir-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    std::fs::create_dir_all(temp.path().join("src")).expect("create dir");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_named_code_tool(
            "code_read_files",
            &json!({"paths": ["src"]}).to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("requires a file path"));
            assert!(output.contains("directory"));
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn file_read_rejects_directory_paths_before_execution() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-read-dir-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    std::fs::create_dir_all(temp.path().join("src")).expect("create dir");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({"operation": "read", "path": "src"}).to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("file.read requires a file path"));
            assert!(output.contains("directory"));
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn file_write_recovers_text_alias() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-write-alias-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "write",
                "path": "index.html",
                "text": "<h1>Hello, Gestura</h1>\n",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("index.html"));
            let written = std::fs::read_to_string(&file_path).expect("read written file");
            assert!(written.contains("Hello, Gestura"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn file_write_reports_noop_when_content_is_unchanged() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-write-noop-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello, Gestura</h1>\n").expect("seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "write",
                "path": "index.html",
                "content": "<h1>Hello, Gestura</h1>\n",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("made no changes"));
            assert_eq!(
                std::fs::read_to_string(&file_path).expect("read unchanged file"),
                "<h1>Hello, Gestura</h1>\n"
            );
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn file_edit_reports_noop_when_replacement_is_identical() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-edit-noop-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello, Gestura</h1>\n").expect("seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "edit",
                "path": "index.html",
                "old": "Gestura",
                "new": "Gestura",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("unchanged"));
            assert_eq!(
                std::fs::read_to_string(&file_path).expect("read unchanged file"),
                "<h1>Hello, Gestura</h1>\n"
            );
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn file_edit_requires_canonical_old_and_new_fields() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-edit-inline-recovery-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Welcome to the app</h1>\n").expect("seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
            .execute_file_tool(
                &json!({
                    "operation": "edit",
                    "path": "index.html",
                    "pattern": "None",
                    "start": "1.0\" No. The correct is: old is <h1>Welcome to the app</h1> new is <h1>Hello, World!</h1>"
                })
                .to_string(),
                Some(&workspace),
            )
            .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("Missing required field 'old' for file edit operation"));
            let updated = std::fs::read_to_string(&file_path).expect("read updated file");
            assert!(updated.contains("<h1>Welcome to the app</h1>"));
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn file_write_recovers_full_document_pattern_payload() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-write-pattern-fallback-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "write",
                "path": "index.html",
                "pattern": "<!doctype html>\n<html><body><h1>Hello, Gestura</h1></body></html>\n",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("index.html"));
            let written = std::fs::read_to_string(&file_path).expect("read written file");
            assert!(written.contains("<!doctype html>"));
            assert!(written.contains("Hello, Gestura"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn file_write_recovers_full_document_pattern_even_with_extra_fields() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-write-pattern-benign-fields-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "write",
                "path": "index.html",
                "pattern": "<!doctype html>\n<html><body><h1>Hello, Gestura</h1></body></html>\n",
                "recursive": false,
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Success(output) => {
            assert!(output.contains("index.html"));
            let written = std::fs::read_to_string(&file_path).expect("read written file");
            assert!(written.contains("<!doctype html>"));
            assert!(written.contains("Hello, Gestura"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn file_write_missing_content_error_is_actionable() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-write-missing-content-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "write",
                "path": "index.html",
                "pattern": "full content",
                "start": 1,
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(message) => {
            assert!(message.contains("Missing required field 'content' for file write operation"));
            assert!(message.contains("pattern, start"));
            assert!(message.contains("\"content\":\"<full file contents here>\""));
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn file_write_without_content_errors_for_inspection_shaped_args() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-write-demote-read-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello, Gestura</h1>\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "write",
                "path": "index.html",
                "pattern": "None",
                "recursive": false,
                "show_hidden": false,
                "start": 1,
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("Missing required field 'content' for file write operation"));
            let updated = std::fs::read_to_string(&file_path).expect("read seed file");
            assert!(updated.contains("Hello, Gestura"));
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn file_edit_without_replacement_errors_for_inspection_shaped_args() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-edit-demote-read-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello, Gestura</h1>\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "edit",
                "path": "index.html",
                "pattern": "None",
                "recursive": false,
                "show_hidden": false,
                "start": 1,
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("Missing required field 'old' for file edit operation"));
            let updated = std::fs::read_to_string(&file_path).expect("read seed file");
            assert!(updated.contains("Hello, Gestura"));
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn file_edit_without_new_errors_after_pattern_recovers_old_text() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("file-edit-partial-demote-read-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Welcome to the app</h1>\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_file_tool(
            &json!({
                "operation": "edit",
                "path": "index.html",
                "pattern": "<h1>Welcome to the app</h1>",
                "start": 1,
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("Missing required field 'new' for file edit operation"));
            let updated = std::fs::read_to_string(&file_path).expect("read unchanged file");
            assert_eq!(updated, "<h1>Welcome to the app</h1>\n");
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn code_batch_edit_without_edits_errors_for_inspection_shaped_args() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-edit-demote-batch-read-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello, Gestura</h1>\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_code_tool(
            &json!({
                "operation": "batch_edit",
                "path": "index.html",
                "pattern": "None",
                "symbol": "None",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("Missing required field 'edits'"));
            let updated = std::fs::read_to_string(&file_path).expect("read unchanged file");
            assert!(updated.contains("Hello, Gestura"));
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn code_batch_edit_directory_path_is_rejected_before_execution() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-edit-demote-read-error-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    std::fs::create_dir_all(temp.path().join("sample-app")).expect("create dir");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_code_tool(
            &json!({
                "operation": "batch_edit",
                "path": "sample-app",
                "pattern": "",
                "symbol": "",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("Missing required field 'edits'"));
            assert!(output.contains("path, pattern, symbol"));
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn code_batch_edit_with_pattern_alias_errors_without_edits() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-edit-pattern-alias-test-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello</h1>\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_code_tool(
            &json!({
                "operation": "batch_edit",
                "path": "index.html",
                "pattern": "<h1>Hello</h1>",
                "replacement": "<h1>Hello, Gestura</h1>",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("Missing required field 'edits'"));
            let updated = std::fs::read_to_string(&file_path).expect("read unchanged file");
            assert!(updated.contains("<h1>Hello</h1>"));
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn code_batch_edit_without_replacement_errors_for_partial_edit_intent() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!(
        "code-edit-partial-demote-batch-read-{}",
        uuid::Uuid::new_v4()
    );
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello</h1>\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_code_tool(
            &json!({
                "operation": "batch_edit",
                "path": "index.html",
                "pattern": "<h1>Hello</h1>",
                "note": "replace the heading",
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("Missing required field 'edits'"));
            let updated = std::fs::read_to_string(&file_path).expect("read unchanged file");
            assert_eq!(updated, "<h1>Hello</h1>\n");
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[test]
fn continuation_prompt_warns_task_errors_should_not_block_implementation() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: build the app",
        "I will update the task first.",
        &[crate::pipeline::ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: json!({
                "operation": "update_status",
                "task_id": "abc",
            })
            .to_string(),
            result: crate::pipeline::ToolResult::Error(
                AgentPipeline::format_missing_task_update_status_error(&json!({
                    "operation": "update_status",
                    "task_id": "abc",
                })),
            ),
            duration_ms: 1,
        }],
    );

    assert!(prompt.contains("task-tracking errors must not block implementation work"));
    assert!(
        prompt.contains(
            "runtime already keeps the tracked root task aligned with overall run progress"
        )
    );
    assert!(prompt.contains(
        "For `create`, provide a specific `name` and preferably a concrete `description`"
    ));
    assert!(
        prompt.contains(
            "For `update`, provide `task_id` plus at least one of `name` or `description`"
        )
    );
    assert!(prompt.contains("always include both `task_id` and `status`"));
    assert!(!prompt.contains("Tool task call:"));
    assert!(!prompt.contains("Arguments: {\"operation\":\"update_status\",\"task_id\":\"abc\"}"));
}

#[test]
fn continuation_prompt_warns_not_to_repeat_successful_task_updates() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: build the app",
        "I updated the task status.",
        &[crate::pipeline::ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: "{}".to_string(),
            result: crate::pipeline::ToolResult::Success(
                "Updated task abc status to InProgress".to_string(),
            ),
            duration_ms: 1,
        }],
    );

    assert!(prompt.contains("successful task update is only bookkeeping"));
    assert!(prompt.contains("Do not repeat the same task update"));
    assert!(prompt.contains("you know its exact next status"));
}

#[test]
fn continuation_prompt_warns_missing_task_status_should_not_cause_looping() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: build the app",
        "I updated the task status.",
        &[crate::pipeline::ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: json!({
                "operation": "update_status",
                "task_id": "abc",
            })
            .to_string(),
            result: crate::pipeline::ToolResult::Error(
                AgentPipeline::format_missing_task_update_status_error(&json!({
                    "operation": "update_status",
                    "task_id": "abc",
                })),
            ),
            duration_ms: 1,
        }],
    );

    assert!(prompt.contains("if `task.update_status` was sent without explicit `status`"));
    assert!(prompt.contains("not a reason to keep looping on task bookkeeping"));
    assert!(prompt.contains("do not call `task` on the next step"));
    assert!(prompt.contains("intentionally not echoed back into this prompt"));
    assert!(!prompt.contains("Arguments: {\"operation\":\"update_status\",\"task_id\":\"abc\"}"));
    assert!(!prompt.contains("auto-recovered bookkeeping"));
}

#[test]
fn continuation_prompt_warns_missing_named_task_status_should_not_cause_looping() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: build the app",
        "I updated the task status.",
        &[crate::pipeline::ToolCallRecord {
            id: "1".to_string(),
            name: "task_update_status".to_string(),
            arguments: json!({
                "task_id": "abc",
            })
            .to_string(),
            result: crate::pipeline::ToolResult::Error(
                AgentPipeline::format_missing_task_update_status_error(&json!({
                    "task_id": "abc",
                })),
            ),
            duration_ms: 1,
        }],
    );

    assert!(prompt.contains("task-tracking errors must not block implementation work"));
    assert!(prompt.contains("if `task.update_status` was sent without explicit `status`"));
    assert!(prompt.contains("send one corrected `update_status` call"));
}

#[test]
fn continuation_prompt_warns_missing_task_update_fields_should_not_cause_looping() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: build the app",
        "I updated the task.",
        &[crate::pipeline::ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: json!({
                "operation": "update",
                "task_id": "abc",
                "status": "completed",
            })
            .to_string(),
            result: crate::pipeline::ToolResult::Error(
                AgentPipeline::format_missing_task_update_fields_error(&json!({
                    "operation": "update",
                    "task_id": "abc",
                    "status": "completed",
                })),
            ),
            duration_ms: 1,
        }],
    );

    assert!(prompt.contains("if `task.update` was sent without `name` or `description`"));
    assert!(prompt.contains("not a reason to keep looping on task bookkeeping"));
    assert!(prompt.contains("use `update_status` with both `task_id` and `status`"));
    assert!(prompt.contains("do not call `task` on the next step"));
    assert!(!prompt.contains(
        "Arguments: {\"operation\":\"update\",\"task_id\":\"abc\",\"status\":\"completed\"}"
    ));
}

#[test]
fn continuation_prompt_warns_skipped_missing_task_status_should_not_cause_looping() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
            "User: build the app",
            "I updated the task status.",
            &[crate::pipeline::ToolCallRecord {
                id: "1".to_string(),
                name: "task".to_string(),
                arguments: json!({
                    "operation": "update_status",
                    "task_id": "abc",
                })
                .to_string(),
                result: crate::pipeline::ToolResult::Skipped(
                    "Skipped malformed `task.update_status` without explicit `status`; retry only with both `task_id` and `status`."
                        .to_string(),
                ),
                duration_ms: 1,
            }],
        );

    assert!(prompt.contains("if `task.update_status` was sent without explicit `status`"));
    assert!(prompt.contains("do not call `task` on the next step"));
    assert!(prompt.contains("not a reason to keep looping on task bookkeeping"));
    assert!(prompt.contains("intentionally not echoed back into this prompt"));
    assert!(!prompt.contains("auto-recovered bookkeeping"));
}

#[test]
fn continuation_prompt_warns_missing_task_create_name_should_not_cause_looping() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: build the app",
        "I started planning.",
        &[crate::pipeline::ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: json!({
                "operation": "create",
                "task_id": "abc",
            })
            .to_string(),
            result: crate::pipeline::ToolResult::Error(
                AgentPipeline::format_missing_task_create_name_error(&json!({
                    "operation": "create",
                    "task_id": "abc",
                })),
            ),
            duration_ms: 1,
        }],
    );

    assert!(prompt.contains("if `task.create` was sent without a valid `name`"));
    assert!(prompt.contains("otherwise do not call `task` on the next step"));
    assert!(prompt.contains("intentionally not echoed back into this prompt"));
    assert!(!prompt.contains("Arguments: {\"operation\":\"create\",\"task_id\":\"abc\"}"));
}

#[test]
fn continuation_prompt_keeps_task_tracking_available_after_loop_breaker() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
            "User: build the app",
            "I attempted task creation.",
            &[crate::pipeline::ToolCallRecord {
                id: "1".to_string(),
                name: "task".to_string(),
                arguments: json!({
                    "operation": "create"
                })
                .to_string(),
                result: crate::pipeline::ToolResult::Skipped(
                    "Loop breaker: skipped a repeated malformed `task.create` call without a valid `name` after 1 prior similar malformed attempts in this run."
                        .to_string(),
                ),
                duration_ms: 1,
            }],
        );

    assert!(prompt.contains("task tracking is still available"));
    assert!(prompt.contains("Do not repeat the blocked malformed `task` arguments"));
    assert!(
        prompt.contains(
            "send one corrected `task.create`, `task.update`, or `task.update_status` call"
        )
    );
}

#[test]
fn continuation_prompt_does_not_echo_missing_status_task_error_arguments() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: build the app",
        "I updated the task status.",
        &[crate::pipeline::ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: json!({
                "operation": "update_status",
                "task_id": "6073f304-388d-408c-82d0-f49f8679656a",
            })
            .to_string(),
            result: crate::pipeline::ToolResult::Error(
                AgentPipeline::format_missing_task_update_status_error(&json!({
                    "operation": "update_status",
                    "task_id": "6073f304-388d-408c-82d0-f49f8679656a",
                })),
            ),
            duration_ms: 1,
        }],
    );

    assert!(prompt.contains("Tool task result:"));
    assert!(prompt.contains("Missing required field 'status' for update_status operation"));
    assert!(!prompt.contains("Tool task call:"));
    assert!(!prompt.contains(
            "Arguments: {\"operation\":\"update_status\",\"task_id\":\"6073f304-388d-408c-82d0-f49f8679656a\"}"
        ));
}

#[test]
fn continuation_prompt_warns_write_errors_need_full_content() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: build the app",
        "I'll update the file next.",
        &[crate::pipeline::ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: json!({
                "operation": "write",
                "path": "sample-app/app/main.py",
                "pattern": "none",
                "start": 1,
            })
            .to_string(),
            result: crate::pipeline::ToolResult::Error(
                AgentPipeline::format_missing_file_write_content_error(&json!({
                    "operation": "write",
                    "path": "sample-app/app/main.py",
                    "pattern": "none",
                    "start": 1,
                })),
            ),
            duration_ms: 1,
        }],
    );

    assert!(prompt.contains("file-tool errors must not block implementation work"));
    assert!(prompt.contains("full file `content`"));
    assert!(prompt.contains("`pattern`/`start` do not make a valid write"));
    assert!(prompt.contains("Arguments: {\"operation\":\"write\""));
    assert!(prompt.contains("Do not retry the same malformed write call"));
}

#[test]
fn continuation_prompt_explains_loop_breaker_is_non_fatal() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
            "User: build the app",
            "I will correct the file write.",
            &[crate::pipeline::ToolCallRecord {
                id: "1".to_string(),
                name: "file".to_string(),
                arguments: json!({
                    "operation": "write",
                    "path": "sample-app/app/main.py",
                    "pattern": "none",
                    "start": 1,
                })
                .to_string(),
                result: crate::pipeline::ToolResult::Skipped(
                    "Loop breaker: skipped a repeated malformed `file.write` call without `content` after 1 prior similar non-successful attempts in this run. The agent is still running.".to_string(),
                ),
                duration_ms: 1,
            }],
        );

    assert!(prompt.contains("loop breaker blocked a repeated malformed tool call"));
    assert!(prompt.contains("agent run is still active"));
    assert!(prompt.contains("Do not retry the blocked malformed call shape again in this turn"));
}

#[test]
fn continuation_prompt_pushes_implementation_after_successful_project_scaffold() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: create a hello world app and build/test it",
        "I scaffolded the app and will inspect the files.",
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: json!({
                "command": "npx create-project-app@latest sample-app --yes --template basic"
            })
            .to_string(),
            result: ToolResult::Success("Scaffold created".to_string()),
            duration_ms: 1,
        }],
    );

    assert!(prompt.contains("a project scaffold/init command has already succeeded in this run"));
    assert!(
        prompt
            .contains("Do not spend another turn repeatedly listing or treeing the scaffold root")
    );
    assert!(
            prompt.contains(
                "Prefer the repo's real entrypoints, manifests, and changed surface area over generic example paths"
            )
        );
    assert!(
        prompt.contains("make the requested change and run the remaining build/test verification")
    );
}

#[test]
fn continuation_prompt_explains_redundant_file_inspection_skip() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
            "User: finish the app",
            "I will inspect the scaffold again.",
            &[ToolCallRecord {
                id: "1".to_string(),
                name: "file".to_string(),
                arguments: json!({
                    "operation": "tree",
                    "path": "sample-app"
                })
                .to_string(),
                result: ToolResult::Skipped(
                    "Loop breaker: skipped a redundant repeated `file.tree` inspection of `sample-app` after 2 prior successful identical inspections in this run.".to_string(),
                ),
                duration_ms: 1,
            }],
        );

    assert!(prompt.contains("repeated file inspections of the same path are now being skipped"));
    assert!(prompt.contains("Stop re-listing the scaffold root"));
    assert!(prompt.contains("edit it, then run the requested verification commands"));
}

#[test]
fn continuation_prompt_pushes_full_write_after_malformed_file_edit_loop() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
            "User: create a hello world app",
            "I will try editing the file again.",
            &[
                ToolCallRecord {
                    id: "1".to_string(),
                    name: "file".to_string(),
                    arguments: json!({
                        "operation": "edit",
                        "path": "sample-app/app/main.py",
                        "old": "print('hello')",
                    })
                    .to_string(),
                    result: ToolResult::Error(
                        "Missing required field 'new' for file edit operation".to_string(),
                    ),
                    duration_ms: 1,
                },
                ToolCallRecord {
                    id: "2".to_string(),
                    name: "file".to_string(),
                    arguments: json!({
                        "operation": "edit",
                        "path": "sample-app/app/main.py",
                        "old": "print('hello')",
                    })
                    .to_string(),
                    result: ToolResult::Skipped(
                        "Loop breaker: skipped a repeated malformed `file.edit` call without valid `old`/`new` replacement text after 1 prior similar non-successful attempts in this run.".to_string(),
                    ),
                    duration_ms: 1,
                },
            ],
        );

    assert!(prompt.contains("prefer one corrected `write_file` with full `content`"));
    assert!(prompt.contains("instead of repeating partial edit attempts"));
}

#[test]
fn continuation_prompt_pushes_file_write_after_malformed_code_batch_edit() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: create a hello world app",
        "I will retry the code edit.",
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "code".to_string(),
            arguments: json!({
                "operation": "batch_edit",
                "path": "sample-app/app/main.py",
            })
            .to_string(),
            result: ToolResult::Error(
                "Missing required field 'edits' for code batch_edit operation".to_string(),
            ),
            duration_ms: 1,
        }],
    );

    assert!(prompt.contains("a single `write_file` with full `content` is often simpler"));
}

#[test]
fn continuation_prompt_discourages_meta_review_loops() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: build and test the app",
        "Reviewing results and deciding the next action.",
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Success(
                "Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.2s".to_string(),
            ),
            duration_ms: 1,
        }],
    );

    assert!(prompt.contains("do not spend the next turn narrating meta-progress"));
    assert!(prompt.contains("take the single next concrete tool action now"));
    assert!(prompt.contains("provide one concise final summary and stop"));
}

#[test]
fn continuation_prompt_forces_specific_recovery_after_repeated_non_tty_scaffold_failures() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_tool_continuation_prompt(
        "User: create an app and build/test it",
        "I will try another shell fallback.",
        &[
            ToolCallRecord {
                id: "1".to_string(),
                name: "shell".to_string(),
                arguments: json!({
                    "command": "npx create-project-app@latest sample-app --template basic --yes"
                })
                .to_string(),
                result: ToolResult::Error("Exit 1: stderr: IO error: not a terminal".to_string()),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "shell".to_string(),
                arguments: json!({
                    "command": "npx create-project-app@latest sample-app --template basic --yes"
                })
                .to_string(),
                result: ToolResult::Error(
                    "Exit 124: command likely waited for interactive input".to_string(),
                ),
                duration_ms: 1,
            },
        ],
    );

    assert!(prompt.contains("scaffold/init command has already failed multiple times"));
    assert!(prompt.contains("do not manually synthesize the project"));
    assert!(prompt.contains("check the scaffold tool's `--help`"));
    assert!(prompt.contains("documented non-interactive scaffold/init command"));
}

#[test]
fn shell_failure_format_detects_interactive_timeout_prompts() {
    let message = AgentPipeline::format_shell_failure(
        124,
        "Need to install the following packages:\ncreate-project-app@4.6.2\nOk to proceed? (y)\n",
        "",
        None,
    );

    assert!(message.contains("likely waited for interactive input"));
    assert!(message.contains("Shell runtime classification: waiting_for_input."));
    assert!(message.contains("shell tool is non-interactive"));
    assert!(message.contains("Ok to proceed? (y)"));
}

#[test]
fn shell_failure_format_surfaces_runtime_error_output_classification() {
    let message = AgentPipeline::format_shell_failure(
        124,
        "",
        "error: no such file or directory",
        Some(crate::tools::shell_streaming::ShellRuntimeFailureKind::ErrorOutput),
    );

    assert!(message.contains("Shell runtime classification: error_output."));
    assert!(message.contains("looked like an error"));
    assert!(message.contains("error: no such file or directory"));
}

#[test]
fn shell_failure_format_surfaces_generic_timeout_classification() {
    let message = AgentPipeline::format_shell_failure(
        124,
        "",
        "",
        Some(crate::tools::shell_streaming::ShellRuntimeFailureKind::TimedOut),
    );

    assert!(message.contains("Shell runtime classification: timed_out."));
    assert!(message.contains("Command timed out"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(not(target_os = "windows"))]
async fn shell_tool_timeout_surfaces_interactive_prompt_context() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = TempDir::new().expect("temp dir");
    let workspace =
        SessionWorkspace::from_directory("shell-timeout-test", temp.path().to_path_buf())
            .expect("workspace");

    let result = pipeline
            .execute_tool(
                "shell",
                &json!({
                    "command": "printf 'Need to install the following packages:\ncreate-project-app@4.6.2\nOk to proceed? (y)\n'; sleep 2",
                    "timeout_secs": 1,
                })
                .to_string(),
                Some(&workspace),
                None,
                None,
            )
            .await;

    match result {
        crate::pipeline::ToolResult::Error(message) => {
            assert!(message.contains("likely waited for interactive input"));
            assert!(message.contains("Need to install the following packages"));
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(not(target_os = "windows"))]
async fn streaming_shell_tool_emits_keepalive_for_silent_commands() {
    use gestura_core_streaming::StreamChunk;

    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = TempDir::new().expect("temp dir");
    let workspace =
        SessionWorkspace::from_directory("shell-keepalive-test", temp.path().to_path_buf())
            .expect("workspace");
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let collector = spawn_stream_collector(rx);
    let silent_command = silent_shell_test_command();

    let result = tokio::spawn({
        let tx = tx.clone();
        async move {
            pipeline
                .execute_tool(
                    "shell",
                    &json!({
                        "command": silent_command,
                        "timeout_secs": 5,
                    })
                    .to_string(),
                    Some(&workspace),
                    None,
                    Some(&tx),
                )
                .await
        }
    });

    let tool_result = tokio::time::timeout(STREAMING_SHELL_TOOL_TEST_TIMEOUT, result)
        .await
        .expect("streaming shell keepalive test timed out")
        .expect("shell execution task should join");

    shutdown_shell_session_for_test("shell-keepalive-test").await;
    drop(tx);
    let chunks = tokio::time::timeout(STREAMING_SHELL_TOOL_SHUTDOWN_TIMEOUT, collector)
        .await
        .expect("timed out collecting shell keepalive chunks")
        .expect("shell keepalive collector should join");

    let saw_keepalive = chunks.iter().any(|chunk| {
        matches!(
            chunk,
            StreamChunk::Status { message }
                if message.contains("Tool `shell` still running...")
        )
    });

    assert!(matches!(tool_result, ToolResult::Success(_)));
    assert!(
        saw_keepalive,
        "expected a keepalive status for silent shell work, got {chunks:?}"
    );
}

#[tokio::test]
async fn generic_tool_keepalive_mentions_tool_name() {
    use gestura_core_streaming::StreamChunk;

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let keepalive = tokio::spawn(emit_streaming_tool_keepalive(
        tx,
        Instant::now(),
        "file".to_string(),
    ));

    let chunk = tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("keepalive chunk timeout")
        .expect("keepalive should emit a chunk");

    match chunk {
        StreamChunk::Status { message } => {
            assert!(message.contains("Tool `file` still running..."));
        }
        other => panic!("expected status chunk, got {other:?}"),
    }

    keepalive.abort();
    let _ = keepalive.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(not(target_os = "windows"))]
async fn streaming_shell_tool_strips_matching_leading_cd_from_command() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = TempDir::new().expect("temp dir");
    let workspace =
        SessionWorkspace::from_directory("shell-stream-cwd-test", temp.path().to_path_buf())
            .expect("workspace");
    let app_dir = temp.path().join("sample-app");
    std::fs::create_dir_all(&app_dir).expect("create app dir");
    let canonical_app_dir = std::fs::canonicalize(&app_dir).unwrap_or_else(|_| app_dir.clone());
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let command = cwd_echo_command("sample-app");

    let result = tokio::time::timeout(
        STREAMING_SHELL_TOOL_TEST_TIMEOUT,
        pipeline.execute_tool(
            "shell",
            &json!({
                "command": command,
                "cwd": "sample-app",
                "timeout_secs": 10,
            })
            .to_string(),
            Some(&workspace),
            None,
            Some(&tx),
        ),
    )
    .await
    .expect("streaming shell tool should complete");

    shutdown_shell_session_for_test(&workspace.session_id).await;
    drop(tx);
    tokio::time::timeout(STREAMING_SHELL_TOOL_SHUTDOWN_TIMEOUT, drain)
        .await
        .expect("timed out draining shell stream")
        .expect("join stream drain task");

    match result {
        ToolResult::Success(stdout) => {
            assert!(
                stdout.contains(app_dir.to_string_lossy().as_ref())
                    || stdout.contains(canonical_app_dir.to_string_lossy().as_ref()),
                "expected stdout to include requested cwd, got {stdout:?}"
            );
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(not(target_os = "windows"))]
async fn streaming_shell_tool_uses_agent_session_pool_without_workspace() {
    use gestura_core_streaming::StreamChunk;

    let pipeline = AgentPipeline::new(AppConfig::default());
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("shell-stream-session-only-{}", uuid::Uuid::new_v4());
    let cwd = temp.path().to_string_lossy().to_string();
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let collector = spawn_stream_collector(rx);

    let result = tokio::spawn({
        let tx = tx.clone();
        let session_id = session_id.clone();
        let cwd = cwd.clone();
        async move {
            pipeline
                .execute_tool(
                    "shell",
                    &json!({
                        "command": "printf 'session-owned'",
                        "cwd": cwd,
                        "timeout_secs": 10,
                    })
                    .to_string(),
                    None,
                    Some(session_id.as_str()),
                    Some(&tx),
                )
                .await
        }
    });

    let tool_result = tokio::time::timeout(STREAMING_SHELL_TOOL_TEST_TIMEOUT, result)
        .await
        .expect("streaming shell agent-session pool test timed out")
        .expect("shell execution task should join");

    shutdown_shell_session_for_test(&session_id).await;
    drop(tx);
    let chunks = tokio::time::timeout(STREAMING_SHELL_TOOL_SHUTDOWN_TIMEOUT, collector)
        .await
        .expect("timed out collecting agent-session shell chunks")
        .expect("agent-session collector should join");

    let saw_shell_session_id = chunks.iter().any(|chunk| {
        matches!(
            chunk,
            StreamChunk::ShellLifecycle {
                shell_session_id: Some(_),
                ..
            } | StreamChunk::ShellOutput {
                shell_session_id: Some(_),
                ..
            }
        )
    });

    assert!(matches!(tool_result, ToolResult::Success(_)));
    assert!(
        saw_shell_session_id,
        "expected streamed shell events to include a shell_session_id, got {chunks:?}"
    );
}

#[tokio::test]
async fn code_batch_edit_entry_failure_is_reported_as_error() {
    let temp = TempDir::new().expect("temp dir");
    let session_id = format!("code-edit-entry-error-{}", uuid::Uuid::new_v4());
    let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
        .expect("workspace");
    let file_path = temp.path().join("index.html");
    std::fs::write(&file_path, "<h1>Hello</h1>\n").expect("write seed file");
    let pipeline = AgentPipeline::new(AppConfig::default());

    let result = pipeline
        .execute_code_tool(
            &json!({
                "operation": "batch_edit",
                "edits": [{
                    "path": "index.html",
                    "old_str": "<h1>Missing</h1>",
                    "new_str": "<h1>Hello, Gestura</h1>",
                }]
            })
            .to_string(),
            Some(&workspace),
        )
        .await;

    match result {
        crate::pipeline::ToolResult::Error(output) => {
            assert!(output.contains("code.batch_edit completed with 1 failing edit"));
            assert!(output.contains("old_str not found"));
            let unchanged = std::fs::read_to_string(&file_path).expect("read unchanged file");
            assert_eq!(unchanged, "<h1>Hello</h1>\n");
        }
        other => panic!("expected error, got {other:?}"),
    }
}
