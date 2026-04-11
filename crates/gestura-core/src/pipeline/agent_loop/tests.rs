#![allow(clippy::question_mark)]
#![allow(clippy::too_many_arguments)]
use super::*;

#[test]
fn meaningful_final_text_requires_real_summary_content() {
    assert!(!AgentPipeline::has_meaningful_final_text(""));
    assert!(!AgentPipeline::has_meaningful_final_text("done"));
    assert!(AgentPipeline::has_meaningful_final_text(
        "Built the app, ran the tests, and verified the hello world window renders correctly."
    ));
}

#[test]
fn build_and_test_request_requires_both_successful_verifications() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Success("ok".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo test"}).to_string(),
            result: ToolResult::Success("ok".to_string()),
            duration_ms: 1,
        },
    ];

    assert!(AgentPipeline::is_missing_requested_build_and_test(
        true,
        &tool_calls[..1]
    ));
    assert!(!AgentPipeline::is_missing_requested_build_and_test(
        true,
        &tool_calls
    ));
}

#[test]
fn failed_test_command_does_not_satisfy_build_and_test_requirement() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Success("ok".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo test"}).to_string(),
            result: ToolResult::Error("tests failed".to_string()),
            duration_ms: 1,
        },
    ];

    assert!(AgentPipeline::is_missing_requested_build_and_test(
        true,
        &tool_calls
    ));
}

#[test]
fn frontend_source_mutation_requires_frontend_capable_build_verification() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "hello-world/src/main.js",
                "content": "document.querySelector('#app').textContent = 'Hello world';\n"
            })
            .to_string(),
            result: ToolResult::Success("Wrote hello-world/src/main.js".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Success("ok".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo test --quiet"}).to_string(),
            result: ToolResult::Success("ok".to_string()),
            duration_ms: 1,
        },
    ];

    assert!(AgentPipeline::is_missing_requested_build_and_test(
        true,
        &tool_calls
    ));
}

#[test]
fn build_test_requirement_must_be_derived_from_user_request_not_full_prompt() {
    let user_request = "Please rewrite README.md and summarize what changed.";
    let assembled_prompt = format!(
        "System: Use available tools when needed. For repository changes, build and test it before finishing.\nUser: {user_request}"
    );

    assert!(!AgentPipeline::prompt_requires_build_and_test(user_request));
    assert!(AgentPipeline::prompt_requires_build_and_test(
        &assembled_prompt
    ));
}

#[test]
fn first_turn_plan_only_response_for_tracked_execution_is_not_terminal() {
    assert!(AgentPipeline::should_force_initial_execution_without_tools(
        false,
        true,
        true,
        true,
        "I will first plan the project structure and then implement it.",
        0,
        Some(12),
    ));

    assert!(
        !AgentPipeline::should_force_initial_execution_without_tools(
            false,
            true,
            true,
            true,
            "I need one clarification from you before I can continue.",
            0,
            Some(12),
        )
    );
}

#[test]
fn synthetic_final_summary_reports_tool_activity_transparently() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let summary = pipeline
        .build_synthetic_final_summary(
            &[
                ToolCallRecord {
                    id: "1".to_string(),
                    name: "file".to_string(),
                    arguments: "{}".to_string(),
                    result: ToolResult::Success("created src/main.rs".to_string()),
                    duration_ms: 12,
                },
                ToolCallRecord {
                    id: "2".to_string(),
                    name: "shell".to_string(),
                    arguments: "{}".to_string(),
                    result: ToolResult::Error("cargo build failed".to_string()),
                    duration_ms: 40,
                },
            ],
            IncompleteRunReason::MissingTerminalSummary,
        )
        .expect("summary should be generated");

    assert!(summary.contains("2 tool call(s)"));
    assert!(summary.contains("1 succeeded, 1 failed, 0 skipped"));
    assert!(summary.contains("without producing a proper wrap-up for the user"));
    assert!(summary.contains("Final status from the observed run: mixed results"));
    assert!(summary.contains("Last tool `shell` failed"));
    assert!(summary.contains("run a shell command"));
    assert!(summary.contains("Review the recorded tool activity above for the detailed outputs."));
    assert!(!summary.contains("cargo build failed"));
}

#[test]
fn synthetic_final_summary_reports_latest_verification_outcomes() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let summary = pipeline
        .build_synthetic_final_summary(
            &[
                ToolCallRecord {
                    id: "1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({
                        "command": "cargo test --manifest-path src-tauri/Cargo.toml"
                    })
                    .to_string(),
                    result: ToolResult::Success("test result: ok. 0 passed; 0 failed".to_string()),
                    duration_ms: 12,
                },
                ToolCallRecord {
                    id: "2".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({
                        "command": "npm run tauri build -- --bundles app"
                    })
                    .to_string(),
                    result: ToolResult::Error("bundle build failed".to_string()),
                    duration_ms: 40,
                },
                ToolCallRecord {
                    id: "3".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({
                        "command": "cargo test --manifest-path src-tauri/Cargo.toml"
                    })
                    .to_string(),
                    result: ToolResult::Success("test result: ok. 4 passed; 0 failed".to_string()),
                    duration_ms: 52,
                },
            ],
            IncompleteRunReason::MissingTerminalSummary,
        )
        .expect("summary should be generated");

    assert!(summary.contains(
        "Final status from the observed run: the latest verification finished successfully after earlier failed attempts."
    ));
    assert!(summary.contains(
        "the latest observed verification command `cargo test --manifest-path src-tauri/Cargo.toml` succeeded after earlier failing attempts such as `npm run tauri build -- --bundles app`"
    ));
}

#[test]
fn synthetic_final_summary_deduplicates_replayed_tool_calls() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let repeated = ToolCallRecord {
        id: "dup-shell".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({
            "command": "cargo test -p gestura-core --lib"
        })
        .to_string(),
        result: ToolResult::Success("test result: ok".to_string()),
        duration_ms: 18,
    };

    let summary = pipeline
        .build_synthetic_final_summary(
            &[repeated.clone(), repeated],
            IncompleteRunReason::MissingTerminalSummary,
        )
        .expect("summary should be generated");

    assert!(
        summary.contains(
            "The observed run covered 1 tool call(s) (1 succeeded, 0 failed, 0 skipped)."
        )
    );
}

#[test]
fn synthetic_final_summary_can_report_iteration_budget_exhaustion() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let summary = pipeline
        .build_synthetic_final_summary(
            &[ToolCallRecord {
                id: "1".to_string(),
                name: "shell".to_string(),
                arguments: "{}".to_string(),
                result: ToolResult::Success("cargo build".to_string()),
                duration_ms: 12,
            }],
            IncompleteRunReason::IterationBudgetExhausted { max_iterations: 30 },
        )
        .expect("summary should be generated");

    assert!(summary.contains("iteration budget limit (30)"));
    assert!(summary.contains("before I could finish the request cleanly"));
}

#[test]
fn synthetic_final_summary_reports_skipped_loop_breaker_call_without_stop_reason() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let summary = pipeline
            .build_synthetic_final_summary(
                &[ToolCallRecord {
                    id: "1".to_string(),
                    name: "file".to_string(),
                    arguments: serde_json::json!({
                        "operation": "write",
                        "path": "sample-app/app/main.py",
                        "pattern": "none",
                        "start": 1
                    })
                    .to_string(),
                    result: ToolResult::Skipped(
                        "Loop breaker: skipped a repeated malformed `file.write` call without `content`."
                            .to_string(),
                    ),
                    duration_ms: 1,
                }],
                IncompleteRunReason::MissingTerminalSummary,
            )
            .expect("summary should be generated");

    assert!(summary.contains("without producing a proper wrap-up for the user"));
    assert!(summary.contains("Last tool `file` was skipped while trying to write a file"));
}

#[test]
fn detects_deferred_remaining_work_in_status_updates() {
    assert!(AgentPipeline::text_defers_remaining_work(
        "Remaining: initialize the project and build it. Next turn will resume with the highest-priority incomplete subtask."
    ));
    assert!(AgentPipeline::text_defers_remaining_work(
        "No code edits, builds, or tests executed yet."
    ));
    assert!(!AgentPipeline::text_defers_remaining_work(
        "Implemented the UI, ran the tests, and everything passed successfully."
    ));
}

#[test]
fn detects_complete_word_in_successful_final_text() {
    assert!(AgentPipeline::text_signals_completed_work(
        "All requested steps are complete and the generated project is ready."
    ));
}

#[test]
fn detects_when_text_is_a_real_user_blocker_or_question() {
    assert!(AgentPipeline::text_signals_user_blocker_or_question(
        "I need your confirmation before I overwrite the existing project."
    ));
    assert!(AgentPipeline::text_signals_user_blocker_or_question(
        "Which directory would you like me to use?"
    ));
    assert!(!AgentPipeline::text_signals_user_blocker_or_question(
        "Reviewing ls output and preparing the next implementation step."
    ));
}

#[test]
fn forced_execution_prompt_requires_real_time_task_tracking() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_forced_execution_prompt(
        "Inspect the project and continue execution.",
        "Created the scaffold, but have not built the app yet.",
        None,
        None,
    );

    assert!(prompt.contains("runtime-selected current task"));
    assert!(prompt.contains("Keep task status aligned with actual execution evidence"));
    assert!(
        prompt.contains(
            "Do not mark the root task complete until every planned subtask is completed"
        )
    );
}

#[test]
fn forced_execution_prompt_focuses_highest_priority_incomplete_subtask() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-forced-focus-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut first = crate::Task::new(
        &session_id,
        "Plan Tauri implementation steps",
        "Plan first",
        Some(root.id.clone()),
    );
    let mut second = crate::Task::new(
        &session_id,
        "Implement Hello World UI",
        "Implement second",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    first.sort_order = 0;
    second.sort_order = 10;

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(first.clone());
    task_list.add_task(second.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_forced_execution_prompt(
        "Continue the run.",
        "The project is partially complete.",
        Some(&session_id),
        Some(&root.id),
    );

    assert!(prompt.contains("Runtime task state:"));
    assert!(
        prompt.contains(
            "Current runtime-selected task: Plan Tauri implementation steps [not_started]"
        )
    );
    assert!(prompt.contains(
        "Only batch tasks together when the runtime explicitly marks them as parallel-safe."
    ));
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        Some(first.id.clone())
    );
}

#[tokio::test]
async fn flush_buffered_iteration_text_emits_and_updates_response_content() {
    let (tx, mut rx) = mpsc::channel(4);
    let mut response = AgentResponse::empty();
    let mut buffered = "First pass summary.".to_string();

    AgentPipeline::flush_buffered_iteration_text(&tx, &mut response, &mut buffered).await;

    assert!(buffered.is_empty());
    assert_eq!(response.content, "First pass summary.");
    match rx.recv().await {
        Some(StreamChunk::Text(text)) => assert_eq!(text, "First pass summary."),
        other => panic!("expected buffered text chunk, got {other:?}"),
    }
}

#[tokio::test]
async fn flush_buffered_iteration_text_is_noop_for_empty_buffer() {
    let (tx, mut rx) = mpsc::channel(1);
    let mut response = AgentResponse::with_content("Visible text");
    let mut buffered = String::new();

    AgentPipeline::flush_buffered_iteration_text(&tx, &mut response, &mut buffered).await;

    assert_eq!(response.content, "Visible text");
    assert!(rx.try_recv().is_err());
}

#[test]
fn restoring_execution_mode_clears_tool_free_summary_latches() {
    let mut force_tool_free_final_summary = true;
    let mut forced_execution_after_empty_response = true;
    let mut forced_final_summary_requested = true;

    AgentPipeline::restore_execution_mode_after_forced_summary(
        &mut force_tool_free_final_summary,
        &mut forced_execution_after_empty_response,
        &mut forced_final_summary_requested,
    );

    assert!(!force_tool_free_final_summary);
    assert!(!forced_execution_after_empty_response);
    assert!(!forced_final_summary_requested);
}

#[test]
fn without_tool_schema_removes_task_entries_for_all_providers() {
    let task = crate::tools::registry::find_tool("task").expect("task tool");
    let shell = crate::tools::registry::find_tool("shell").expect("shell tool");
    let schemas = crate::tools::schemas::build_provider_tool_schemas(&[task, shell]);

    let filtered = AgentPipeline::without_tool_schema(&schemas, "task");

    assert_eq!(filtered.openai.len(), 1);
    assert_eq!(filtered.openai_responses.len(), 1);
    assert_eq!(filtered.anthropic.len(), 1);
    assert_eq!(filtered.gemini.len(), 1);
    assert_eq!(filtered.openai[0]["function"]["name"], "shell");
    assert_eq!(filtered.openai_responses[0]["name"], "shell");
    assert_eq!(filtered.anthropic[0]["name"], "shell");
    assert_eq!(filtered.gemini[0]["name"], "shell");
}

#[test]
fn required_verification_retry_schemas_keep_shell_only_for_all_providers() {
    let task = crate::tools::registry::find_tool("task").expect("task tool");
    let file = crate::tools::registry::find_tool("file").expect("file tool");
    let code = crate::tools::registry::find_tool("code_edit_files").expect("code tool");
    let shell = crate::tools::registry::find_tool("shell").expect("shell tool");
    let schemas = crate::tools::schemas::build_provider_tool_schemas(&[task, file, code, shell]);

    let filtered = AgentPipeline::required_verification_retry_schemas(&schemas);

    assert_eq!(filtered.openai.len(), 1);
    assert_eq!(filtered.openai_responses.len(), 1);
    assert_eq!(filtered.anthropic.len(), 1);
    assert_eq!(filtered.gemini.len(), 1);
    assert_eq!(filtered.openai[0]["function"]["name"], "shell");
    assert_eq!(filtered.openai_responses[0]["name"], "shell");
    assert_eq!(filtered.anthropic[0]["name"], "shell");
    assert_eq!(filtered.gemini[0]["name"], "shell");
}

#[test]
fn task_tool_is_suspended_after_task_loop_breaker_skip() {
    let tool_calls = vec![ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: serde_json::json!({
                "operation": "create"
            })
            .to_string(),
            result: ToolResult::Skipped(
                "Loop breaker: skipped a repeated malformed `task.create` call without a valid `name` after 2 prior similar malformed attempts in this run."
                    .to_string(),
            ),
            duration_ms: 1,
        }];

    assert!(AgentPipeline::should_suspend_task_tool(&tool_calls));
}

#[test]
fn task_tool_is_suspended_after_two_consecutive_malformed_task_errors() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: serde_json::json!({
                "operation": "create"
            })
            .to_string(),
            result: ToolResult::Error(
                "Missing required field 'name' for create operation".to_string(),
            ),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "task".to_string(),
            arguments: serde_json::json!({
                "operation": "create"
            })
            .to_string(),
            result: ToolResult::Error(
                "Missing required field 'name' for create operation".to_string(),
            ),
            duration_ms: 1,
        },
    ];

    assert!(AgentPipeline::should_suspend_task_tool(&tool_calls));
}

#[test]
fn task_tool_is_suspended_after_two_consecutive_malformed_task_update_errors() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: serde_json::json!({
                "operation": "update",
                "task_id": "abc123",
                "status": "completed"
            })
            .to_string(),
            result: ToolResult::Error(
                "Missing required update fields for update operation".to_string(),
            ),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "task".to_string(),
            arguments: serde_json::json!({
                "operation": "update",
                "task_id": "abc123"
            })
            .to_string(),
            result: ToolResult::Error(
                "Missing required update fields for update operation".to_string(),
            ),
            duration_ms: 1,
        },
    ];

    assert!(AgentPipeline::should_suspend_task_tool(&tool_calls));
}

#[test]
fn file_tool_is_not_suspended_after_file_loop_breaker_skip() {
    let tool_calls = vec![ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "app/main.py",
                "pattern": "none"
            })
            .to_string(),
            result: ToolResult::Skipped(
                "Loop breaker: skipped a repeated malformed `file.write` call without `content` after 2 prior similar non-successful attempts in this run."
                    .to_string(),
            ),
            duration_ms: 1,
        }];

    assert!(!AgentPipeline::should_suspend_file_tool(&tool_calls));
}

#[test]
fn file_tool_is_not_suspended_after_repeated_malformed_file_edit_calls() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "edit",
                "path": "app/main.py",
                "pattern": "None",
                "start": "replace the heading"
            })
            .to_string(),
            result: ToolResult::Error(
                "Missing required field 'old' for file edit operation".to_string(),
            ),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "edit",
                "path": "app/main.py",
                "pattern": "None",
                "start": "replace the heading"
            })
            .to_string(),
            result: ToolResult::Error(
                "Missing required field 'new' for file edit operation".to_string(),
            ),
            duration_ms: 1,
        },
    ];

    assert!(!AgentPipeline::should_suspend_file_tool(&tool_calls));
}

#[test]
fn file_tool_is_suspended_after_sustained_malformed_mutation_streak() {
    let tool_calls = vec![
            ToolCallRecord {
                id: "1".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "edit",
                    "path": "app/main.py",
                    "pattern": "print('hello')",
                    "start": 1,
                })
                .to_string(),
                result: ToolResult::Error(
                    "Missing required field 'old' for file edit operation".to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "read",
                    "path": "app/main.py",
                })
                .to_string(),
                result: ToolResult::Success("print('hello')".to_string()),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "3".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "write",
                    "path": "app/main.py",
                    "start": 1,
                })
                .to_string(),
                result: ToolResult::Error(
                    "Missing required field 'content' for file write operation".to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "4".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "edit",
                    "path": "app/main.py",
                    "pattern": "none",
                    "start": 1,
                })
                .to_string(),
                result: ToolResult::Skipped(
                    "Loop breaker: skipped a repeated malformed `file.edit` call without valid `old`/`new` replacement text after 3 prior similar non-successful attempts in this run."
                        .to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "5".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "write",
                    "path": "app/main.py",
                    "start": 1,
                })
                .to_string(),
                result: ToolResult::Skipped(
                    "Loop breaker: skipped a repeated malformed `file.write` call without `content` after 4 prior similar non-successful attempts in this run."
                        .to_string(),
                ),
                duration_ms: 1,
            },
        ];

    assert!(AgentPipeline::should_suspend_file_tool(&tool_calls));
}

#[test]
fn file_tool_suspension_counts_edit_file_alias_calls() {
    let tool_calls = vec![
            ToolCallRecord {
                id: "1".to_string(),
                name: "edit_file".to_string(),
                arguments: serde_json::json!({
                    "path": "app/main.py",
                    "pattern": "old"
                })
                .to_string(),
                result: ToolResult::Error(
                    "Missing required field 'new' for file edit operation".to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "edit_file".to_string(),
                arguments: serde_json::json!({
                    "path": "app/main.py",
                    "pattern": "old"
                })
                .to_string(),
                result: ToolResult::Error(
                    "Missing required field 'new' for file edit operation".to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "3".to_string(),
                name: "edit_file".to_string(),
                arguments: serde_json::json!({
                    "path": "app/main.py",
                    "pattern": "old"
                })
                .to_string(),
                result: ToolResult::Skipped(
                    "Loop breaker: skipped a repeated malformed `file.edit` call without valid `old`/`new` replacement text after 2 prior similar non-successful attempts in this run."
                        .to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "4".to_string(),
                name: "edit_file".to_string(),
                arguments: serde_json::json!({
                    "path": "app/main.py",
                    "pattern": "old"
                })
                .to_string(),
                result: ToolResult::Skipped(
                    "Loop breaker: skipped a repeated malformed `file.edit` call without valid `old`/`new` replacement text after 3 prior similar non-successful attempts in this run."
                        .to_string(),
                ),
                duration_ms: 1,
            },
        ];

    assert!(AgentPipeline::should_suspend_file_tool(&tool_calls));
}

#[test]
fn successful_file_mutation_resets_file_tool_suspension_streak() {
    let tool_calls = vec![
            ToolCallRecord {
                id: "1".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "write",
                    "path": "app/main.py",
                    "start": 1,
                })
                .to_string(),
                result: ToolResult::Error(
                    "Missing required field 'content' for file write operation".to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "edit",
                    "path": "app/main.py",
                    "old": "print('hello')",
                    "new": "print('hello world')",
                })
                .to_string(),
                result: ToolResult::Success("Updated app/main.py".to_string()),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "3".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "edit",
                    "path": "app/main.py",
                    "pattern": "none",
                    "start": 1,
                })
                .to_string(),
                result: ToolResult::Error(
                    "Missing required field 'old' for file edit operation".to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "4".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "write",
                    "path": "app/main.py",
                    "start": 1,
                })
                .to_string(),
                result: ToolResult::Skipped(
                    "Loop breaker: skipped a repeated malformed `file.write` call without `content` after 2 prior similar non-successful attempts in this run."
                        .to_string(),
                ),
                duration_ms: 1,
            },
        ];

    assert!(!AgentPipeline::should_suspend_file_tool(&tool_calls));
}

#[test]
fn task_tool_disabled_instruction_mentions_runtime_reconciliation() {
    let prompt = AgentPipeline::with_task_tool_disabled_instruction("User: update index.html");

    assert!(prompt.contains("`task` tool is disabled for the rest of this run"));
    assert!(prompt.contains("runtime will reconcile that bookkeeping automatically"));
}

#[test]
fn open_subtask_continuation_is_suppressed_when_task_tool_is_suspended() {
    assert!(!AgentPipeline::should_force_open_subtask_continuation(
        OpenSubtaskContinuationInput {
            saw_any_tool_calls: true,
            open_descendant_summary: OpenDescendantSummary {
                not_started: 1,
                ..OpenDescendantSummary::default()
            },
            task_tool_suspended: true,
            iteration_content: "Implemented the requested change and summarized the result.",
            iteration: 2,
            max_iterations: Some(8),
        }
    ));
    assert!(
        !AgentPipeline::should_force_deferred_tracked_work_continuation(
            true,
            OpenDescendantSummary {
                not_started: 1,
                ..OpenDescendantSummary::default()
            },
            true,
            "Remaining: clean up task bookkeeping next turn.",
            2,
            Some(8),
        )
    );
    assert!(
        !AgentPipeline::should_force_meaningful_incomplete_tracked_work_continuation(
            true,
            Some(&TrackedTaskRuntimeState {
                snapshot: crate::streaming::TaskRuntimeSnapshot {
                    root_task_id: "root".to_string(),
                    current_task: Some(crate::streaming::TaskRuntimeTaskView {
                        id: "task-1".to_string(),
                        name: "Extract the information that matters".to_string(),
                        status: "in_progress".to_string(),
                    }),
                    ready_tasks: vec![crate::streaming::TaskRuntimeTaskView {
                        id: "task-2".to_string(),
                        name: "Summarize findings and next steps".to_string(),
                        status: "ready".to_string(),
                    }],
                    parallel_ready_tasks: Vec::new(),
                    blocked_tasks: Vec::new(),
                    open_tasks: vec![crate::streaming::TaskRuntimeTaskView {
                        id: "task-1".to_string(),
                        name: "Extract the information that matters".to_string(),
                        status: "in_progress".to_string(),
                    }],
                    completed_tasks: Vec::new(),
                    missing_requirements: vec!["source mutation not yet verified".to_string()],
                    status_message: "Work remains open".to_string(),
                },
                open_descendant_summary: OpenDescendantSummary {
                    in_progress: 1,
                    ..OpenDescendantSummary::default()
                },
                completion_ready: false,
            }),
            true,
            "The run is still in progress and the next unresolved step is extracting the relevant information.",
            2,
            Some(8),
        )
    );
}

#[test]
fn completed_tool_iteration_can_finalize_after_successful_tool_results() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "file".to_string(),
        arguments: serde_json::json!({"operation": "read", "path": "README.md"}).to_string(),
        result: ToolResult::Success("done".to_string()),
        duration_ms: 1,
    }];

    assert!(AgentPipeline::should_finalize_completed_tool_iteration(
        false,
        false,
        "Completed the requested README rewrite and verified the final result.",
        &tool_calls,
        &tool_calls,
        OpenDescendantSummary::default(),
        false,
    ));
}

#[test]
fn completed_tool_iteration_does_not_finalize_with_only_not_started_descendants() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "file".to_string(),
        arguments: serde_json::json!({"operation": "read", "path": "README.md"}).to_string(),
        result: ToolResult::Success("done".to_string()),
        duration_ms: 1,
    }];

    assert!(!AgentPipeline::should_finalize_completed_tool_iteration(
        false,
        false,
        "Completed the requested README rewrite and verified the final result.",
        &tool_calls,
        &tool_calls,
        OpenDescendantSummary {
            not_started: 1,
            ..OpenDescendantSummary::default()
        },
        true,
    ));
}

#[test]
fn completed_tool_iteration_does_not_finalize_with_in_progress_descendants() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "file".to_string(),
        arguments: serde_json::json!({"operation": "read", "path": "README.md"}).to_string(),
        result: ToolResult::Success("done".to_string()),
        duration_ms: 1,
    }];

    assert!(!AgentPipeline::should_finalize_completed_tool_iteration(
        false,
        false,
        "Completed the requested README rewrite and verified the final result.",
        &tool_calls,
        &tool_calls,
        OpenDescendantSummary {
            in_progress: 1,
            ..OpenDescendantSummary::default()
        },
        false,
    ));
}

#[test]
fn open_subtask_continuation_resumes_work_for_successful_summary_with_only_not_started_descendants()
{
    assert!(AgentPipeline::should_force_open_subtask_continuation(
        OpenSubtaskContinuationInput {
            saw_any_tool_calls: true,
            open_descendant_summary: OpenDescendantSummary {
                not_started: 2,
                ..OpenDescendantSummary::default()
            },
            task_tool_suspended: false,
            iteration_content: "Completed the requested app update and verified the final result.",
            iteration: 4,
            max_iterations: Some(8),
        }
    ));
}

#[test]
fn open_subtask_continuation_persists_when_success_still_requires_file_mutation() {
    assert!(AgentPipeline::should_force_open_subtask_continuation(
        OpenSubtaskContinuationInput {
            saw_any_tool_calls: true,
            open_descendant_summary: OpenDescendantSummary {
                not_started: 2,
                ..OpenDescendantSummary::default()
            },
            task_tool_suspended: false,
            iteration_content: "Completed the scaffold setup and verified the build result.",
            iteration: 4,
            max_iterations: Some(8),
        }
    ));
}

#[test]
fn meaningful_incomplete_no_tool_summary_forces_continuation() {
    assert!(
        AgentPipeline::should_force_meaningful_incomplete_tracked_work_continuation(
            true,
            Some(&TrackedTaskRuntimeState {
                snapshot: crate::streaming::TaskRuntimeSnapshot {
                    root_task_id: "root".to_string(),
                    current_task: Some(crate::streaming::TaskRuntimeTaskView {
                        id: "task-1".to_string(),
                        name: "Extract the information that matters".to_string(),
                        status: "in_progress".to_string(),
                    }),
                    ready_tasks: vec![crate::streaming::TaskRuntimeTaskView {
                        id: "task-2".to_string(),
                        name: "Summarize findings and next steps".to_string(),
                        status: "ready".to_string(),
                    }],
                    parallel_ready_tasks: Vec::new(),
                    blocked_tasks: Vec::new(),
                    open_tasks: vec![crate::streaming::TaskRuntimeTaskView {
                        id: "task-1".to_string(),
                        name: "Extract the information that matters".to_string(),
                        status: "in_progress".to_string(),
                    }],
                    completed_tasks: Vec::new(),
                    missing_requirements: vec!["source mutation not yet verified".to_string()],
                    status_message: "Work remains open".to_string(),
                },
                open_descendant_summary: OpenDescendantSummary {
                    in_progress: 1,
                    ..OpenDescendantSummary::default()
                },
                completion_ready: false,
            }),
            false,
            "The run is still in progress and the next unresolved step is extracting the relevant information before I summarize the findings.",
            2,
            Some(8),
        )
    );
}

#[test]
fn repeated_no_tool_open_subtask_stall_escalates_quickly_for_completion_like_retries() {
    let open_descendant_summary = OpenDescendantSummary {
        not_started: 1,
        in_progress: 1,
        ..OpenDescendantSummary::default()
    };
    let fingerprint =
        AgentPipeline::no_tool_open_subtask_fingerprint(None, open_descendant_summary)
            .expect("fingerprint should exist for open descendants");
    let mut last_fingerprint = None;
    let mut streak = 0usize;

    AgentPipeline::update_stagnation_streak(
        fingerprint.clone(),
        &mut last_fingerprint,
        &mut streak,
    );
    assert_eq!(streak, 1);
    assert!(!AgentPipeline::should_escalate_no_tool_open_subtask_stall(
        true,
        true,
        "Completed the review and wrapped up the task.",
        open_descendant_summary,
        false,
        false,
        streak,
        3,
        Some(8),
    ));

    AgentPipeline::update_stagnation_streak(fingerprint, &mut last_fingerprint, &mut streak);
    assert_eq!(streak, 2);
    assert!(AgentPipeline::should_escalate_no_tool_open_subtask_stall(
        true,
        true,
        "Completed the review and wrapped up the task.",
        open_descendant_summary,
        false,
        false,
        streak,
        4,
        Some(8),
    ));
}

#[test]
fn repeated_no_tool_open_subtask_stall_waits_longer_for_generic_research_prose() {
    let open_descendant_summary = OpenDescendantSummary {
        not_started: 1,
        in_progress: 1,
        ..OpenDescendantSummary::default()
    };

    assert!(!AgentPipeline::should_escalate_no_tool_open_subtask_stall(
        true,
        true,
        "The search results suggest the market is fragmented, with stronger consumer demand around automation bundles and energy savings.",
        open_descendant_summary,
        false,
        false,
        3,
        4,
        Some(8),
    ));
    assert!(AgentPipeline::should_escalate_no_tool_open_subtask_stall(
        true,
        true,
        "The search results suggest the market is fragmented, with stronger consumer demand around automation bundles and energy savings.",
        open_descendant_summary,
        false,
        false,
        4,
        5,
        Some(8),
    ));
}

#[test]
fn repeated_no_tool_open_subtask_stall_does_not_re_escalate_after_final_summary_requested() {
    assert!(!AgentPipeline::should_escalate_no_tool_open_subtask_stall(
        true,
        true,
        "Still summarizing the open work.",
        OpenDescendantSummary {
            not_started: 1,
            ..OpenDescendantSummary::default()
        },
        false,
        true,
        4,
        6,
        Some(8),
    ));
}

#[test]
fn no_tool_open_subtask_stall_resets_when_runtime_shape_changes() {
    let mut last_fingerprint = None;
    let mut streak = 0usize;
    let first = AgentPipeline::no_tool_open_subtask_fingerprint(
        None,
        OpenDescendantSummary {
            not_started: 1,
            ..OpenDescendantSummary::default()
        },
    )
    .expect("fingerprint should exist");
    let second = AgentPipeline::no_tool_open_subtask_fingerprint(
        None,
        OpenDescendantSummary {
            not_started: 1,
            in_progress: 1,
            ..OpenDescendantSummary::default()
        },
    )
    .expect("fingerprint should exist");

    AgentPipeline::update_stagnation_streak(first, &mut last_fingerprint, &mut streak);
    AgentPipeline::update_stagnation_streak(second, &mut last_fingerprint, &mut streak);

    assert_eq!(streak, 1);
}

#[test]
fn meaningful_final_text_rejects_internal_parameter_markup() {
    let leaked_response = concat!(
        "Summary: Scaffolded the app and verified the build.\n\n",
        "<parameter name=\"operation\">update_status</parameter>\n",
        "<parameter name=\"task_id\">abc</parameter>"
    );

    assert!(!AgentPipeline::has_meaningful_final_text(leaked_response));
    assert!(
        !AgentPipeline::final_response_signals_successful_completion(
            false,
            false,
            leaked_response,
            &[],
        )
    );
}

#[test]
fn stalled_tool_loop_after_file_suspension_forces_tool_free_summary() {
    let all_tool_calls = vec![
            ToolCallRecord {
                id: "1".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "write",
                    "path": "README.md",
                    "pattern": "none"
                })
                .to_string(),
                result: ToolResult::Skipped(
                    "Loop breaker: skipped a repeated malformed `file.write` call without `content` after 2 prior similar non-successful attempts in this run."
                        .to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({"command": "cat README.md"}).to_string(),
                result: ToolResult::Success("README contents".to_string()),
                duration_ms: 1,
            },
        ];

    let iteration_tool_calls = vec![ToolCallRecord {
        id: "3".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({"command": "cat README.md"}).to_string(),
        result: ToolResult::Success("README contents".to_string()),
        duration_ms: 1,
    }];

    assert!(
        AgentPipeline::should_force_tool_free_final_summary_after_stalled_tool_loop(
            false,
            "",
            &all_tool_calls,
            &iteration_tool_calls,
            OpenDescendantSummary::default(),
            ToolSuspensionState {
                task: false,
                file: true,
                code: false,
            },
            3,
        )
    );
}

#[test]
fn stalled_tool_loop_with_open_descendants_does_not_force_tool_free_summary() {
    let tool_calls = vec![ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: serde_json::json!({"operation": "update_status"}).to_string(),
            result: ToolResult::Skipped(
                "Loop breaker: skipped a repeated malformed `task.update_status` call without `status` after 2 prior similar non-successful attempts in this run.".to_string(),
            ),
            duration_ms: 1,
        }];

    assert!(
        !AgentPipeline::should_force_tool_free_final_summary_after_stalled_tool_loop(
            false,
            "",
            &tool_calls,
            &tool_calls,
            OpenDescendantSummary {
                not_started: 1,
                ..OpenDescendantSummary::default()
            },
            ToolSuspensionState {
                task: true,
                file: false,
                code: false,
            },
            3,
        )
    );
}

#[test]
fn stalled_tool_loop_does_not_force_tool_free_summary_without_loop_breaker_signal() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({"command": "echo done"}).to_string(),
        result: ToolResult::Success("done".to_string()),
        duration_ms: 1,
    }];

    assert!(
        !AgentPipeline::should_force_tool_free_final_summary_after_stalled_tool_loop(
            false,
            "",
            &tool_calls,
            &tool_calls,
            OpenDescendantSummary::default(),
            ToolSuspensionState::default(),
            3,
        )
    );
}

#[test]
fn trailing_repeated_successful_verification_command_detects_repeated_cargo_check() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Success("Finished cargo check".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Success("Finished cargo check again".to_string()),
            duration_ms: 1,
        },
    ];

    assert_eq!(
        AgentPipeline::trailing_repeated_successful_verification_command(&tool_calls, 2).as_deref(),
        Some("cargo check")
    );
}

#[test]
fn stalled_tool_loop_forces_required_verification_after_repeated_cargo_check() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Success("Finished cargo check".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Success("Finished cargo check again".to_string()),
            duration_ms: 1,
        },
    ];

    assert!(
        AgentPipeline::should_force_required_verification_after_stalled_tool_loop(
            true,
            false,
            "",
            &tool_calls,
            &tool_calls[1..],
            OpenDescendantSummary::default(),
            3,
        )
    );
}

#[test]
fn stalled_tool_loop_forces_required_verification_after_long_code_batch_read_streak() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "code".to_string(),
        arguments: serde_json::json!({
            "operation": "batch_read",
            "paths": ["hello-world/app/main.py", "hello-world/README.md"],
        })
        .to_string(),
        result: ToolResult::Success("[]".to_string()),
        duration_ms: 1,
    }];

    assert!(
        AgentPipeline::should_force_required_verification_after_stalled_tool_loop(
            true,
            false,
            "",
            &tool_calls,
            &tool_calls,
            OpenDescendantSummary::default(),
            6,
        )
    );
}

#[test]
fn stalled_tool_loop_forces_required_verification_after_long_silent_shell_streak() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({
            "command": "dotnet new console -n hello-world"
        })
        .to_string(),
        result: ToolResult::Success("Template created!".to_string()),
        duration_ms: 1,
    }];

    assert!(
        AgentPipeline::should_force_required_verification_after_stalled_tool_loop(
            true,
            false,
            "",
            &tool_calls,
            &tool_calls,
            OpenDescendantSummary::default(),
            5,
        )
    );
}

#[test]
fn required_verification_waits_while_actionable_descendant_work_remains() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({"command": "cargo check"}).to_string(),
        result: ToolResult::Success("Finished cargo check".to_string()),
        duration_ms: 1,
    }];

    assert!(
        !AgentPipeline::should_force_required_verification_after_stalled_tool_loop(
            true,
            false,
            "",
            &tool_calls,
            &tool_calls,
            OpenDescendantSummary {
                in_progress: 1,
                ..OpenDescendantSummary::default()
            },
            5,
        )
    );
}

#[test]
fn build_required_verification_prompt_warns_against_repeating_successful_cargo_check() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Success("Finished cargo check".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Success("Finished cargo check again".to_string()),
            duration_ms: 1,
        },
    ];

    let prompt = pipeline.build_required_verification_prompt(
        "User: build and test the app",
        "Reviewing results.",
        &tool_calls,
    );

    assert!(prompt.contains("missing a successful test command"));
    assert!(prompt.contains("already-successful verification command `cargo check`"));
    assert!(prompt.contains("Run a real test command next"));
}

#[test]
fn build_required_verification_prompt_demands_repo_appropriate_build_for_changed_work() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "service/main.py",
                "content": "print('hello world')\n"
            })
            .to_string(),
            result: ToolResult::Success("Wrote service/main.py".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "pytest -q"}).to_string(),
            result: ToolResult::Success("tests ok".to_string()),
            duration_ms: 1,
        },
    ];

    let prompt = pipeline.build_required_verification_prompt(
        "User: build and test the project",
        "Tests passed.",
        &tool_calls,
    );

    assert!(prompt.contains(
        "successful build/check command appropriate for the changed part of the project"
    ));
    assert!(prompt.contains("build and test this project"));
}

#[test]
fn build_and_test_completion_status_recognizes_python_verification_commands() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "python -m build"}).to_string(),
            result: ToolResult::Success("built wheel".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "pytest -q"}).to_string(),
            result: ToolResult::Success("tests ok".to_string()),
            duration_ms: 1,
        },
    ];

    assert_eq!(
        AgentPipeline::build_and_test_completion_status(&tool_calls),
        (true, true, true, true)
    );
}

#[test]
fn build_and_test_completion_status_accepts_tauri_build_plus_cargo_test_for_frontend_changes() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "edit_file".to_string(),
            arguments: serde_json::json!({
                "path": "hello-tauri/src/main.js",
                "old_string": "Hello",
                "new_string": "Hello world",
            })
            .to_string(),
            result: ToolResult::Success(
                serde_json::json!({"changed": true, "path": "hello-tauri/src/main.js"}).to_string(),
            ),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo tauri build --debug"}).to_string(),
            result: ToolResult::Success("tauri build ok".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo test --quiet"}).to_string(),
            result: ToolResult::Success("tests ok".to_string()),
            duration_ms: 1,
        },
    ];

    assert_eq!(
        AgentPipeline::build_and_test_completion_status(&tool_calls),
        (true, true, true, true)
    );
}

#[test]
fn build_and_test_completion_status_accepts_wrapped_tauri_build_plus_wrapped_cargo_test_for_frontend_changes()
 {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "edit_file".to_string(),
            arguments: serde_json::json!({
                "path": "hello-tauri/src/main.js",
                "old_string": "Hello",
                "new_string": "Hello world",
            })
            .to_string(),
            result: ToolResult::Success(
                serde_json::json!({"changed": true, "path": "hello-tauri/src/main.js"}).to_string(),
            ),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": "cd hello-tauri && npm run tauri build"
            })
            .to_string(),
            result: ToolResult::Success("tauri build ok".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": "cd hello-tauri/src-tauri && cargo test --quiet"
            })
            .to_string(),
            result: ToolResult::Success("tests ok".to_string()),
            duration_ms: 1,
        },
    ];

    assert_eq!(
        AgentPipeline::build_and_test_completion_status(&tool_calls),
        (true, true, true, true)
    );
}

#[test]
fn scaffold_detection_recognizes_non_js_init_commands() {
    assert!(AgentPipeline::is_scaffold_or_init_shell_command_text(
        "dotnet new mvc -n hello-world"
    ));
    assert!(AgentPipeline::is_scaffold_or_init_shell_command_text(
        "django-admin startproject hello_world"
    ));
}

#[test]
fn scaffold_detection_ignores_help_and_dry_run_probes() {
    assert!(!AgentPipeline::is_scaffold_or_init_shell_command_text(
        "npx create-tauri-app --help"
    ));
    assert!(!AgentPipeline::is_successful_mutating_shell_tool_call(
        &ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "npx create-tauri-app --help"}).to_string(),
            result: ToolResult::Success("usage printed".to_string()),
            duration_ms: 1,
        }
    ));
    assert_eq!(
        AgentPipeline::build_and_test_completion_status(&[ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo test --help"}).to_string(),
            result: ToolResult::Success("usage printed".to_string()),
            duration_ms: 1,
        }]),
        (false, false, false, false)
    );
}

#[test]
fn stalled_mutation_execution_prompt_demands_a_concrete_edit_next() {
    let pipeline = AgentPipeline::new(AppConfig::default());

    let prompt = pipeline.build_stalled_mutation_execution_prompt(
        "Create the app and keep going.",
        "Read index.html and main.js.",
        None,
        None,
    );

    assert!(prompt.contains("stuck in read-only inspection"));
    assert!(prompt.contains("`edit_file` or `write_file`"));
    assert!(prompt.contains("Stop rereading scaffold or source files"));
}

#[test]
fn forced_final_summary_prompt_mentions_missing_verification_and_open_subtasks() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({"command": "cargo check"}).to_string(),
        result: ToolResult::Success("Finished cargo check".to_string()),
        duration_ms: 1,
    }];

    let prompt = pipeline.build_forced_final_summary_prompt(
        "Build and test the app.",
        "Still summarizing.",
        true,
        true,
        &tool_calls,
        &[],
        OpenDescendantSummary {
            not_started: 2,
            in_progress: 1,
            blocked: 0,
        },
    );

    assert!(prompt.contains("did not observe a successful test command"));
    assert!(prompt.contains("Do not claim the project is fully verified, ready, or complete"));
    assert!(prompt.contains("source mutation not yet verified"));
    assert!(prompt.contains("Tracked task bookkeeping still shows open subtasks"));
    assert!(prompt.contains("not started: 2, in progress: 1, blocked: 0"));
}

#[test]
fn forced_final_summary_prompt_requests_progress_narration_when_runtime_work_remains() {
    let pipeline = AgentPipeline::new(AppConfig::default());

    let prompt = pipeline.build_forced_final_summary_prompt(
        "Implement the feature.",
        "I updated the files and ran validation.",
        false,
        false,
        &[],
        &["root task completion is still blocked: dependencies remain open".to_string()],
        OpenDescendantSummary::default(),
    );

    assert!(prompt.contains("detailed in-progress status narration"));
    assert!(prompt.contains("overall request is still in progress"));
    assert!(prompt.contains("Do not use closing-success wording"));
    assert!(prompt.contains("root task completion is still blocked"));
}

#[test]
fn forced_final_summary_prompt_requests_detailed_closeout_when_work_is_done() {
    let pipeline = AgentPipeline::new(AppConfig::default());

    let prompt = pipeline.build_forced_final_summary_prompt(
        "Finish the request.",
        "I completed the implementation and validation.",
        false,
        false,
        &[],
        &[],
        OpenDescendantSummary::default(),
    );

    assert!(prompt.contains("detailed final closeout"));
    assert!(prompt.contains("concrete artifacts or files produced or changed"));
    assert!(!prompt.contains("concise final status update"));
}

#[test]
fn tool_free_final_summary_prompt_requests_progress_narration_when_work_remains() {
    let pipeline = AgentPipeline::new(AppConfig::default());

    let prompt = pipeline.build_tool_free_final_summary_prompt(
        "Implement the feature.",
        "I updated the files and ran validation.",
        false,
        false,
        &[],
        &["root task completion is still blocked: dependencies remain open".to_string()],
        OpenDescendantSummary::default(),
    );

    assert!(prompt.contains("best direct in-progress status narration"));
    assert!(prompt.contains("overall task is not complete yet"));
    assert!(!prompt.contains("best direct closing summary"));
}

#[test]
fn tool_free_final_summary_prompt_requests_detailed_closeout_when_work_is_done() {
    let pipeline = AgentPipeline::new(AppConfig::default());

    let prompt = pipeline.build_tool_free_final_summary_prompt(
        "Finish the request.",
        "I completed the implementation and validation.",
        false,
        false,
        &[],
        &[],
        OpenDescendantSummary::default(),
    );

    assert!(prompt.contains("best direct detailed closing summary"));
    assert!(!prompt.contains("best direct closing summary you can"));
}

#[test]
fn stalled_tool_loop_forces_tool_free_summary_after_repeated_file_reads() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({"operation": "read", "path": "README.md"}).to_string(),
            result: ToolResult::Success("one".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({"operation": "read", "path": "README.md"}).to_string(),
            result: ToolResult::Success("two".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({"operation": "read", "path": "README.md"}).to_string(),
            result: ToolResult::Success("three".to_string()),
            duration_ms: 1,
        },
    ];

    assert!(
        AgentPipeline::should_force_tool_free_final_summary_after_stalled_tool_loop(
            false,
            "",
            &tool_calls,
            &tool_calls[2..],
            OpenDescendantSummary::default(),
            ToolSuspensionState::default(),
            3,
        )
    );
}

#[test]
fn stalled_read_only_loop_forces_execution_when_mutation_is_still_required() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({"operation": "read", "path": "app/main.py"}).to_string(),
            result: ToolResult::Success("one".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({"operation": "read", "path": "app/main.py"}).to_string(),
            result: ToolResult::Success("two".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({"operation": "read", "path": "app/main.py"}).to_string(),
            result: ToolResult::Success("three".to_string()),
            duration_ms: 1,
        },
    ];

    assert!(
        AgentPipeline::should_force_mutating_execution_after_stalled_inspection(
            true,
            "",
            &tool_calls,
            &tool_calls[2..],
            3,
        )
    );
}

#[test]
fn stalled_read_file_loop_forces_execution_when_mutation_is_still_required() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "app/main.py"}).to_string(),
            result: ToolResult::Success("one".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "app/main.py"}).to_string(),
            result: ToolResult::Success("two".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "app/main.py"}).to_string(),
            result: ToolResult::Success("three".to_string()),
            duration_ms: 1,
        },
    ];

    assert!(
        AgentPipeline::should_force_mutating_execution_after_stalled_inspection(
            true,
            "",
            &tool_calls,
            &tool_calls[2..],
            3,
        )
    );
}

#[test]
fn required_verification_waits_until_mutation_has_been_observed() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({"command": "cargo check -p gestura-gui"}).to_string(),
        result: ToolResult::Success("Finished cargo check".to_string()),
        duration_ms: 1,
    }];

    assert!(
        !AgentPipeline::should_force_required_verification_after_stalled_tool_loop(
            true,
            true,
            "",
            &tool_calls,
            &tool_calls,
            OpenDescendantSummary::default(),
            5,
        )
    );
}

#[test]
fn stalled_read_only_loop_does_not_force_execution_after_successful_edit() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "edit",
                "path": "app/main.py",
                "old": "print('hello')",
                "new": "print('hello world')"
            })
            .to_string(),
            result: ToolResult::Success("Updated app/main.py".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({"operation": "read", "path": "app/main.py"}).to_string(),
            result: ToolResult::Success("print('hello world')".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({"operation": "read", "path": "app/main.py"}).to_string(),
            result: ToolResult::Success("print('hello world')".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "4".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({"operation": "read", "path": "app/main.py"}).to_string(),
            result: ToolResult::Success("print('hello world')".to_string()),
            duration_ms: 1,
        },
    ];

    assert!(
        !AgentPipeline::should_force_mutating_execution_after_stalled_inspection(
            true,
            "",
            &tool_calls,
            &tool_calls[3..],
            3,
        )
    );
}

#[test]
fn stalled_tool_loop_forces_tool_free_summary_after_repeated_shell_cat() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cat README.md"}).to_string(),
            result: ToolResult::Success("one".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cat README.md"}).to_string(),
            result: ToolResult::Success("two".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cat README.md"}).to_string(),
            result: ToolResult::Success("three".to_string()),
            duration_ms: 1,
        },
    ];

    assert!(
        AgentPipeline::should_force_tool_free_final_summary_after_stalled_tool_loop(
            false,
            "",
            &tool_calls,
            &tool_calls[2..],
            OpenDescendantSummary::default(),
            ToolSuspensionState::default(),
            3,
        )
    );
}

#[test]
fn stalled_tool_loop_forces_tool_free_summary_after_long_low_value_streak() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "file".to_string(),
        arguments: serde_json::json!({"operation": "read", "path": "README.md"}).to_string(),
        result: ToolResult::Success("README".to_string()),
        duration_ms: 1,
    }];

    assert!(
        AgentPipeline::should_force_tool_free_final_summary_after_stalled_tool_loop(
            false,
            "",
            &tool_calls,
            &tool_calls,
            OpenDescendantSummary::default(),
            ToolSuspensionState::default(),
            6,
        )
    );
}

#[test]
fn post_verification_read_only_follow_up_forces_tool_free_summary() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Success("Finished cargo check".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo test"}).to_string(),
            result: ToolResult::Success("Finished cargo test".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({"operation": "read", "path": "src/main.js"}).to_string(),
            result: ToolResult::Success("const main = true;".to_string()),
            duration_ms: 1,
        },
    ];

    assert!(
        AgentPipeline::should_force_tool_free_final_summary_after_stalled_tool_loop(
            true,
            "",
            &tool_calls,
            &tool_calls[2..],
            OpenDescendantSummary::default(),
            ToolSuspensionState::default(),
            1,
        )
    );
}

#[test]
fn completion_ready_tool_iteration_without_terminal_text_forces_tool_free_summary() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "edit_file".to_string(),
            arguments: serde_json::json!({
                "path": "tauri-hello-world/src/main.js",
                "old_string": "Hello",
                "new_string": "Hello world",
            })
            .to_string(),
            result: ToolResult::Success("updated file".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments:
                serde_json::json!({"command": "cd tauri-hello-world && npm run tauri build"})
                    .to_string(),
            result: ToolResult::Success("build ok".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "shell".to_string(),
            arguments:
                serde_json::json!({"command": "cd tauri-hello-world/src-tauri && cargo test"})
                    .to_string(),
            result: ToolResult::Success("tests ok".to_string()),
            duration_ms: 1,
        },
    ];
    let runtime_state = TrackedTaskRuntimeState {
        snapshot: crate::streaming::TaskRuntimeSnapshot {
            root_task_id: "root-task".to_string(),
            current_task: None,
            ready_tasks: Vec::new(),
            parallel_ready_tasks: Vec::new(),
            blocked_tasks: Vec::new(),
            open_tasks: Vec::new(),
            completed_tasks: vec![crate::streaming::TaskRuntimeTaskView {
                id: "verify-task".to_string(),
                name: "Test Tauri application".to_string(),
                status: "completed".to_string(),
            }],
            missing_requirements: Vec::new(),
            status_message: "All tracked tasks are complete.".to_string(),
        },
        open_descendant_summary: OpenDescendantSummary::default(),
        completion_ready: true,
    };

    assert!(
        AgentPipeline::should_force_tool_free_final_summary_after_completion_ready_tool_iteration(
            true,
            true,
            "",
            &tool_calls,
            &tool_calls,
            Some(&runtime_state),
            OpenDescendantSummary::default(),
        )
    );
}

#[test]
fn completion_ready_tool_iteration_guard_does_not_fire_with_open_descendants() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({"command": "cargo test"}).to_string(),
        result: ToolResult::Success("tests ok".to_string()),
        duration_ms: 1,
    }];
    let runtime_state = TrackedTaskRuntimeState {
        snapshot: crate::streaming::TaskRuntimeSnapshot {
            root_task_id: "root-task".to_string(),
            current_task: Some(crate::streaming::TaskRuntimeTaskView {
                id: "verify-task".to_string(),
                name: "Verify remaining details".to_string(),
                status: "in_progress".to_string(),
            }),
            ready_tasks: Vec::new(),
            parallel_ready_tasks: Vec::new(),
            blocked_tasks: Vec::new(),
            open_tasks: vec![crate::streaming::TaskRuntimeTaskView {
                id: "verify-task".to_string(),
                name: "Verify remaining details".to_string(),
                status: "in_progress".to_string(),
            }],
            completed_tasks: Vec::new(),
            missing_requirements: Vec::new(),
            status_message: "Verification is still active.".to_string(),
        },
        open_descendant_summary: OpenDescendantSummary {
            in_progress: 1,
            ..OpenDescendantSummary::default()
        },
        completion_ready: false,
    };

    assert!(
        !AgentPipeline::should_force_tool_free_final_summary_after_completion_ready_tool_iteration(
            true,
            false,
            "",
            &tool_calls,
            &tool_calls,
            Some(&runtime_state),
            OpenDescendantSummary {
                in_progress: 1,
                ..OpenDescendantSummary::default()
            },
        )
    );
}

#[test]
fn empty_terminal_retry_is_disabled_once_runtime_is_completion_ready() {
    assert!(
        !AgentPipeline::should_retry_execution_after_empty_terminal_response(
            true,
            false,
            false,
            true,
            2,
            Some(8),
        )
    );
}

#[test]
fn empty_terminal_retry_still_runs_when_runtime_is_not_completion_ready() {
    assert!(
        AgentPipeline::should_retry_execution_after_empty_terminal_response(
            true,
            false,
            false,
            false,
            2,
            Some(8),
        )
    );
}

#[test]
fn runtime_snapshot_status_message_reports_terminal_closeout_when_ready_to_summarize() {
    let status = AgentPipeline::runtime_snapshot_status_message(None, &[], &[], &[]);

    assert_eq!(
        status,
        "Tracked work is closed out and ready for the final user summary"
    );
}

#[test]
fn tool_iteration_finalization_ignores_build_test_words_from_continuation_prompt() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({"command": "cat README.md"}).to_string(),
        result: ToolResult::Success("README contents".to_string()),
        duration_ms: 1,
    }];

    assert!(AgentPipeline::should_finalize_completed_tool_iteration(
        false,
        false,
        "Completed the requested README rewrite and verified the final result.",
        &tool_calls,
        &tool_calls,
        OpenDescendantSummary::default(),
        false,
    ));
}

#[test]
fn stalled_tool_loop_force_ignores_build_test_words_from_continuation_prompt() {
    let all_tool_calls = vec![
            ToolCallRecord {
                id: "1".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "write",
                    "path": "README.md",
                    "pattern": "none"
                })
                .to_string(),
                result: ToolResult::Skipped(
                    "Loop breaker: skipped a repeated malformed `file.write` call without `content` after 2 prior similar non-successful attempts in this run."
                        .to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({"command": "cat README.md"}).to_string(),
                result: ToolResult::Success("README contents".to_string()),
                duration_ms: 1,
            },
        ];

    let iteration_tool_calls = vec![ToolCallRecord {
        id: "3".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({"command": "cat README.md"}).to_string(),
        result: ToolResult::Success("README contents".to_string()),
        duration_ms: 1,
    }];

    assert!(
        AgentPipeline::should_force_tool_free_final_summary_after_stalled_tool_loop(
            false,
            "",
            &all_tool_calls,
            &iteration_tool_calls,
            OpenDescendantSummary::default(),
            ToolSuspensionState {
                task: false,
                file: true,
                code: false,
            },
            3,
        )
    );
}

#[test]
fn file_tool_disabled_instruction_is_appended_to_prompt() {
    let prompt = AgentPipeline::with_file_tool_disabled_instruction("User: update index.html");

    assert!(prompt.contains("`write_file` and `edit_file` are disabled for the rest of this run"));
    assert!(prompt.contains("Do not call `write_file` or `edit_file` again"));
    assert!(
        prompt.contains("The generic `file` tool is only for read/list/tree/search inspection")
    );
}

#[test]
fn code_tool_is_not_suspended_after_code_loop_breaker_skip() {
    let tool_calls = vec![ToolCallRecord {
            id: "1".to_string(),
            name: "code".to_string(),
            arguments: serde_json::json!({
                "operation": "batch_edit",
                "path": "app/main.py"
            })
            .to_string(),
            result: ToolResult::Skipped(
                "Loop breaker: skipped a repeated malformed `code.batch_edit` call without a valid `edits` array after 2 prior similar malformed attempts in this run."
                    .to_string(),
            ),
            duration_ms: 1,
        }];

    assert!(!AgentPipeline::should_suspend_code_tool(&tool_calls));
}

#[test]
fn code_tool_disabled_instruction_is_appended_to_prompt() {
    let prompt = AgentPipeline::with_code_tool_disabled_instruction("User: update index.html");

    assert!(prompt.contains("code-tool family is disabled for the rest of this run"));
    assert!(prompt.contains("Do not call `code` or any `code_*` tool again"));
}

#[test]
fn split_code_tool_failures_do_not_suspend_code_tool_family() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "code_edit_files".to_string(),
            arguments: serde_json::json!({
                "changes": [{
                    "path": "src/lib.rs"
                }]
            })
            .to_string(),
            result: ToolResult::Error(
                "Missing required field 'edits' for code batch_edit operation".to_string(),
            ),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "code_edit_files".to_string(),
            arguments: serde_json::json!({
                "changes": [{
                    "path": "src/lib.rs"
                }]
            })
            .to_string(),
            result: ToolResult::Error(
                "Missing required field 'edits' for code batch_edit operation".to_string(),
            ),
            duration_ms: 1,
        },
    ];

    assert!(!AgentPipeline::should_suspend_code_tool(&tool_calls));
}

#[test]
fn active_task_open_descendants_detects_nested_open_tasks() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-descendants-{}", uuid::Uuid::new_v4());

    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut child = crate::Task::new(&session_id, "Child", "Child", Some(root.id.clone()));
    let grandchild = crate::Task::new(
        &session_id,
        "Grandchild",
        "Grandchild",
        Some(child.id.clone()),
    );
    child.set_status(crate::TaskStatus::Completed);
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child);
    task_list.add_task(grandchild);
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    let summary = AgentPipeline::tracked_open_descendant_summary(Some(&session_id), Some(&root.id));
    assert!(summary.has_open());
    assert_eq!(
        summary,
        OpenDescendantSummary {
            not_started: 1,
            ..OpenDescendantSummary::default()
        }
    );
}

#[test]
fn tracked_task_closeout_note_reports_completed_root_after_reconciliation() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-closeout-note-complete-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::Completed);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    let note = AgentPipeline::tracked_task_closeout_note(Some(&session_id), Some(&root.id))
        .expect("closeout note should be present");

    assert_eq!(
        note,
        "From the tracked task state, everything is closed out now: every subtask is terminal and the overall task is complete."
    );
}

#[test]
fn tracked_task_closeout_note_reports_highest_priority_incomplete_subtask() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-closeout-note-open-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut first = crate::Task::new(
        &session_id,
        "Plan Tauri implementation steps",
        "Plan first",
        Some(root.id.clone()),
    );
    let mut second = crate::Task::new(
        &session_id,
        "Implement Hello World UI",
        "Implement second",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    first.sort_order = 0;
    second.sort_order = 10;

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(first.clone());
    task_list.add_task(second);
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    let note = AgentPipeline::tracked_task_closeout_note(Some(&session_id), Some(&root.id))
        .expect("closeout note should be present");

    assert!(note.contains("overall request is still in_progress"));
    assert!(note.contains("Plan Tauri implementation steps [not_started]"));
}

#[test]
fn mark_tracked_task_in_progress_preserves_current_open_descendant() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-current-descendant-{}", uuid::Uuid::new_v4());

    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut child = crate::Task::new(
        &session_id,
        "Implement",
        "Implement the requested change",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    child.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(child.id.clone()))
        .expect("set current task");

    AgentPipeline::mark_tracked_task_in_progress(Some(&session_id), Some(&root.id));

    let current_task = manager
        .get_current_task_id(&session_id)
        .expect("current task lookup should succeed")
        .expect("current task should be preserved");
    assert_eq!(current_task, child.id);
}

#[test]
fn mark_tracked_task_in_progress_does_not_reopen_completed_root() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-do-not-reopen-root-{}", uuid::Uuid::new_v4());

    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::Completed);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    AgentPipeline::mark_tracked_task_in_progress(Some(&session_id), Some(&root.id));

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
}

#[test]
fn tool_activity_promotes_default_plan_from_planning_to_implementation() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-phase-progress-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut plan = crate::Task::new(
        &session_id,
        "Plan the implementation approach",
        "Review the request and choose an approach",
        Some(root.id.clone()),
    );
    let implement = crate::Task::new(
        &session_id,
        "Implement the requested changes",
        "Make the requested code changes",
        Some(root.id.clone()),
    );
    let verify = crate::Task::new(
        &session_id,
        "Build and test the result",
        "Run verification commands",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    plan.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(plan.clone());
    task_list.add_task(implement.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "app/main.py",
                "content": "print('hello')",
            })
            .to_string(),
            result: ToolResult::Success("wrote app/main.py".to_string()),
            duration_ms: 1,
        }],
    );

    let stored_plan = manager
        .get_task(&session_id, &plan.id)
        .expect("plan lookup should succeed")
        .expect("plan should exist");
    let stored_implement = manager
        .get_task(&session_id, &implement.id)
        .expect("implementation lookup should succeed")
        .expect("implementation should exist");
    assert_eq!(stored_plan.status, crate::TaskStatus::Completed);
    assert_eq!(stored_implement.status, crate::TaskStatus::InProgress);
}

#[test]
fn default_auto_tracked_subtasks_classify_by_execution_kind_even_when_request_mentions_plan_and_implement()
 {
    let request = "I want to create a small Tauri GUI that says hello world. Please carefully plan and implement, then build and test it.";
    let tasks = [
        crate::Task::new(
            "session",
            "Plan the implementation approach",
            format!(
                "Review the request, confirm the concrete implementation approach, and identify the next executable step for:\n\n{}",
                request
            ),
            None,
        ),
        crate::Task::new(
            "session",
            "Implement the requested changes",
            format!(
                "Carry out the requested file, code, or scaffold changes for:\n\n{}",
                request
            ),
            None,
        ),
        crate::Task::new(
            "session",
            "Build and test the result",
            format!(
                "Run the relevant build and test steps, fix any regressions, and confirm the request is complete for:\n\n{}",
                request
            ),
            None,
        ),
    ];

    assert_eq!(
        AgentPipeline::task_execution_profile(&tasks[0], true).execution_kind,
        TaskExecutionKind::Planning
    );
    assert_eq!(
        AgentPipeline::task_execution_profile(&tasks[1], true).execution_kind,
        TaskExecutionKind::Implementation
    );
    assert_eq!(
        AgentPipeline::task_execution_profile(&tasks[2], true).execution_kind,
        TaskExecutionKind::Verification
    );
}

#[test]
fn runtime_reconciliation_surfaces_current_and_parallel_ready_tasks() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-runtime-snapshot-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let plan_a = crate::Task::new(
        &session_id,
        "Plan the frontend changes",
        "Inspect the UI impact",
        Some(root.id.clone()),
    );
    let plan_b = crate::Task::new(
        &session_id,
        "Investigate backend wiring",
        "Inspect the API impact",
        Some(root.id.clone()),
    );
    let implement = crate::Task::new(
        &session_id,
        "Implement the requested changes",
        "Make the code changes",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(plan_a.clone());
    task_list.add_task(plan_b.clone());
    task_list.add_task(implement.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    let runtime_state = AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        &[],
    )
    .expect("runtime state should be available");

    assert_eq!(runtime_state.snapshot.ready_tasks.len(), 3);
    assert_eq!(runtime_state.snapshot.parallel_ready_tasks.len(), 2);
    assert_eq!(
        runtime_state
            .snapshot
            .current_task
            .as_ref()
            .map(|task| task.id.as_str()),
        Some(plan_a.id.as_str())
    );
    assert!(runtime_state.snapshot.missing_requirements.is_empty());
}

#[test]
fn runtime_reconciliation_keeps_research_in_progress_after_initial_search() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-swot-runtime-focus-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut research = crate::Task::new(
        &session_id,
        "Research 2025-2026 Market Trends",
        "Gather the current market evidence",
        Some(root.id.clone()),
    );
    let _plan = crate::Task::new(
        &session_id,
        "Plan SWOT Structure",
        "Outline the markdown structure",
        Some(root.id.clone()),
    );
    let strengths = crate::Task::new(
        &session_id,
        "Develop Strengths Section",
        "Draft the strengths bullets",
        Some(root.id.clone()),
    );
    let weaknesses = crate::Task::new(
        &session_id,
        "Develop Weaknesses Section",
        "Draft the weaknesses bullets",
        Some(root.id.clone()),
    );
    let _opportunities = crate::Task::new(
        &session_id,
        "Develop Opportunities Section",
        "Draft the opportunities bullets",
        Some(root.id.clone()),
    );
    let _threats = crate::Task::new(
        &session_id,
        "Develop Threats Section",
        "Draft the threats bullets",
        Some(root.id.clone()),
    );
    let _implement = crate::Task::new(
        &session_id,
        "Implement Full SWOT Markdown",
        "Write the final markdown deliverable",
        Some(root.id.clone()),
    );
    let verify = crate::Task::new(
        &session_id,
        "Verify Facts and Cross-Check",
        "Cross-check the final market claims",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    research.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(research.clone());
    task_list.add_task(_plan.clone());
    task_list.add_task(strengths.clone());
    task_list.add_task(weaknesses.clone());
    task_list.add_task(_opportunities.clone());
    task_list.add_task(_threats.clone());
    task_list.add_task(_implement.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(research.id.clone()))
        .expect("set current task");

    AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "web_search".to_string(),
            arguments: serde_json::json!({
                "query": "smart home lighting market trends 2025 2026 forecast",
            })
            .to_string(),
            result: ToolResult::Success("found research sources".to_string()),
            duration_ms: 1,
        }],
    );

    let stored_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verify lookup should succeed")
        .expect("verify should exist");
    let stored_research = manager
        .get_task(&session_id, &research.id)
        .expect("research lookup should succeed")
        .expect("research should exist");
    let stored_weaknesses = manager
        .get_task(&session_id, &weaknesses.id)
        .expect("weaknesses lookup should succeed")
        .expect("weaknesses should exist");
    let current_after_research = manager
        .get_current_task_id(&session_id)
        .expect("current task lookup should succeed")
        .expect("current task should stay focused on research");

    assert_eq!(stored_research.status, crate::TaskStatus::InProgress);
    assert_eq!(stored_verify.status, crate::TaskStatus::NotStarted);
    assert_eq!(stored_weaknesses.status, crate::TaskStatus::NotStarted);
    assert_eq!(current_after_research, research.id);
}

#[test]
fn reconciliation_prefers_existing_ready_current_task_over_first_sorted_ready_task() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-preserve-ready-current-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let alpha = crate::Task::new(
        &session_id,
        "Alpha verification",
        "Sibling ready task that sorts first",
        Some(root.id.clone()),
    );
    let mut beta = crate::Task::new(
        &session_id,
        "Beta research",
        "Current in-progress task should stay focused",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    beta.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(alpha.clone());
    task_list.add_task(beta.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(beta.id.clone()))
        .expect("set current task");

    AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "web_search".to_string(),
            arguments: serde_json::json!({
                "query": "beta task supporting research",
            })
            .to_string(),
            result: ToolResult::Success("found research sources".to_string()),
            duration_ms: 1,
        }],
    );

    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        Some(beta.id)
    );
}

#[test]
fn runtime_reconciliation_advances_to_next_phase_after_explicit_planning_completion() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-swot-sequential-progress-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut plan = crate::Task::new(
        &session_id,
        "Plan SWOT structure",
        "Outline the markdown structure",
        Some(root.id.clone()),
    );
    let research = crate::Task::new(
        &session_id,
        "Research market trends",
        "Gather 2025-2026 market evidence",
        Some(root.id.clone()),
    );
    let _implement = crate::Task::new(
        &session_id,
        "Implement full SWOT",
        "Write the final markdown deliverable",
        Some(root.id.clone()),
    );
    let _verify = crate::Task::new(
        &session_id,
        "Verify and cross-check",
        "Cross-check key claims against supporting sources",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    plan.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(plan.clone());
    task_list.add_task(research.clone());
    task_list.add_task(_implement.clone());
    task_list.add_task(_verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(plan.id.clone()))
        .expect("set current task");

    AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "web_search".to_string(),
            arguments: serde_json::json!({
                "query": "smart home lighting market trends 2025 2026",
            })
            .to_string(),
            result: ToolResult::Success("found market trend sources".to_string()),
            duration_ms: 1,
        }],
    );

    let stored_plan = manager
        .get_task(&session_id, &plan.id)
        .expect("plan lookup should succeed")
        .expect("plan should exist");
    let current_after_plan = manager
        .get_current_task_id(&session_id)
        .expect("current task lookup should succeed")
        .expect("current task should stay on planning until completion is explicit");
    assert_eq!(stored_plan.status, crate::TaskStatus::InProgress);
    assert_eq!(current_after_plan, plan.id);

    manager
        .update_task_status(&session_id, &plan.id, crate::TaskStatus::Completed)
        .expect("explicitly complete planning task");

    let runtime_state = AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        &[],
    )
    .expect("runtime state should be available");

    assert_eq!(
        runtime_state
            .snapshot
            .current_task
            .as_ref()
            .map(|task| task.id.as_str()),
        Some(research.id.as_str())
    );
}

#[test]
fn runtime_reconciliation_keeps_root_open_when_completion_write_is_blocked() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-root-completion-blocked-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let dependency = crate::Task::new(&session_id, "External dependency", "Still open", None);
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(dependency.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .add_task_dependency(&session_id, &root.id, &dependency.id)
        .expect("dependency should be added");

    let runtime_state = AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        &[],
    )
    .expect("runtime state should be available");

    let stored_root = manager
        .get_task(&session_id, &root.id)
        .expect("root lookup should succeed")
        .expect("root should exist");

    assert_eq!(stored_root.status, crate::TaskStatus::InProgress);
    assert!(!runtime_state.completion_ready);
    assert_eq!(runtime_state.open_descendant_summary.total(), 0);
    assert_eq!(
        runtime_state
            .snapshot
            .current_task
            .as_ref()
            .map(|task| task.id.as_str()),
        Some(root.id.as_str())
    );
    assert!(runtime_state.snapshot.ready_tasks.is_empty());
    assert!(
        runtime_state
            .snapshot
            .missing_requirements
            .iter()
            .any(|message| {
                message.contains("root task completion is still blocked")
                    && message.contains("dependencies remain open")
            })
    );
}

#[test]
fn tool_activity_promotes_default_plan_into_verification() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-phase-verify-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut plan = crate::Task::new(
        &session_id,
        "Plan the implementation approach",
        "Review the request and choose an approach",
        Some(root.id.clone()),
    );
    let mut implement = crate::Task::new(
        &session_id,
        "Implement the requested changes",
        "Make the requested code changes",
        Some(root.id.clone()),
    );
    let verify = crate::Task::new(
        &session_id,
        "Build and test the result",
        "Run verification commands",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    plan.set_status(crate::TaskStatus::Completed);
    implement.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(plan.clone());
    task_list.add_task(implement.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        true,
        false,
        Some(&session_id),
        Some(&root.id),
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": "cargo test",
            })
            .to_string(),
            result: ToolResult::Success("tests passed".to_string()),
            duration_ms: 1,
        }],
    );

    let stored_implement = manager
        .get_task(&session_id, &implement.id)
        .expect("implementation lookup should succeed")
        .expect("implementation should exist");
    let stored_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verification lookup should succeed")
        .expect("verification should exist");
    assert_eq!(stored_implement.status, crate::TaskStatus::InProgress);
    assert_eq!(stored_verify.status, crate::TaskStatus::InProgress);
}

#[test]
fn runtime_reconciliation_completes_root_after_wrapped_tauri_build_and_test() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-tauri-root-closeout-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Create Tauri Hello World GUI", "Root", None);
    let mut initialize = crate::Task::new(
        &session_id,
        "Initialize Tauri project",
        "Scaffold the project",
        Some(root.id.clone()),
    );
    let mut implement = crate::Task::new(
        &session_id,
        "Implement Hello World frontend",
        "Update the frontend app",
        Some(root.id.clone()),
    );
    let mut verify = crate::Task::new(
        &session_id,
        "Test and verify application",
        "Run the validation commands",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    initialize.set_status(crate::TaskStatus::Completed);
    implement.set_status(crate::TaskStatus::Completed);
    verify.set_status(crate::TaskStatus::Completed);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(initialize.clone());
    task_list.add_task(implement.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    let runtime_state = AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        true,
        false,
        Some(&session_id),
        Some(&root.id),
        &[
            ToolCallRecord {
                id: "1".to_string(),
                name: "edit_file".to_string(),
                arguments: serde_json::json!({
                    "path": "tauri-hello-world/src/main.js",
                    "old_string": "Hello",
                    "new_string": "Hello world",
                })
                .to_string(),
                result: ToolResult::Success(
                    serde_json::json!({"changed": true, "path": "tauri-hello-world/src/main.js"})
                        .to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({
                    "command": "cd tauri-hello-world && npm run tauri build"
                })
                .to_string(),
                result: ToolResult::Success("tauri build ok".to_string()),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "3".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({
                    "command": "cd tauri-hello-world/src-tauri && cargo test"
                })
                .to_string(),
                result: ToolResult::Success("tests ok".to_string()),
                duration_ms: 1,
            },
        ],
    )
    .expect("runtime state should be available");

    let stored_root = manager
        .get_task(&session_id, &root.id)
        .expect("root lookup should succeed")
        .expect("root should exist");

    assert_eq!(stored_root.status, crate::TaskStatus::Completed);
    assert!(runtime_state.completion_ready);
    assert!(runtime_state.snapshot.current_task.is_none());
    assert!(runtime_state.snapshot.missing_requirements.is_empty());
}

#[test]
fn verification_only_progress_keeps_default_implementation_subtask_open() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-verify-without-mutation-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut plan = crate::Task::new(
        &session_id,
        "Plan the implementation approach",
        "Review the request and choose an approach",
        Some(root.id.clone()),
    );
    let implement = crate::Task::new(
        &session_id,
        "Implement the requested changes",
        "Make the requested code changes",
        Some(root.id.clone()),
    );
    let verify = crate::Task::new(
        &session_id,
        "Build and test the result",
        "Run verification commands",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    plan.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(plan.clone());
    task_list.add_task(implement.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        true,
        false,
        Some(&session_id),
        Some(&root.id),
        &[
            ToolCallRecord {
                id: "1".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({
                    "command": "cargo check -p gestura-gui",
                })
                .to_string(),
                result: ToolResult::Success("check passed".to_string()),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({
                    "command": "cargo test -p gestura-gui -- --quiet",
                })
                .to_string(),
                result: ToolResult::Success("tests passed".to_string()),
                duration_ms: 1,
            },
        ],
    );

    let stored_plan = manager
        .get_task(&session_id, &plan.id)
        .expect("plan lookup should succeed")
        .expect("plan should exist");
    let stored_implement = manager
        .get_task(&session_id, &implement.id)
        .expect("implementation lookup should succeed")
        .expect("implementation should exist");
    let stored_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verification lookup should succeed")
        .expect("verification should exist");

    assert_eq!(stored_plan.status, crate::TaskStatus::Completed);
    assert_eq!(stored_implement.status, crate::TaskStatus::NotStarted);
    assert_eq!(stored_verify.status, crate::TaskStatus::Completed);

    let closeout_note =
        AgentPipeline::tracked_task_closeout_note(Some(&session_id), Some(&root.id))
            .expect("closeout note should be present");
    assert!(closeout_note.contains("overall request is still in_progress"));
    assert!(closeout_note.contains("Implement the requested changes [not_started]"));
}

#[test]
fn tracked_task_reconciliation_completes_root_when_descendants_are_done() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-finalize-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut child = crate::Task::new(&session_id, "Child", "Child", Some(root.id.clone()));
    root.set_status(crate::TaskStatus::InProgress);
    child.set_status(crate::TaskStatus::Completed);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child);
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        "Completed the requested work and verified the final result.",
        &[],
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[test]
fn success_reconciliation_does_not_complete_not_started_build_and_test_task_with_only_build_evidence()
 {
    let session_id = format!("agent-loop-success-closeout-{}", uuid::Uuid::new_v4());
    let verify = crate::Task::new(
        &session_id,
        "Build and test the result",
        "Run build and test commands",
        None,
    );

    let status = AgentPipeline::target_status_for_open_descendant_after_success(
        &session_id,
        &verify,
        "Implemented the requested changes and verified the app.",
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": "cargo check -p gestura-gui --quiet",
            })
            .to_string(),
            result: ToolResult::Success("check passed".to_string()),
            duration_ms: 1,
        }],
    );

    assert_eq!(status, None);
}

#[test]
fn success_reconciliation_does_not_complete_not_started_implementation_task_from_summary_text_alone()
 {
    let session_id = format!("agent-loop-success-impl-closeout-{}", uuid::Uuid::new_v4());
    let implement = crate::Task::new(
        &session_id,
        "Implement Hello World frontend",
        "Create the frontend UI and wire the first view",
        None,
    );

    let status = AgentPipeline::target_status_for_open_descendant_after_success(
        &session_id,
        &implement,
        "Implemented the Hello World frontend and verified the app.",
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({
                "path": "src-tauri/src/main.rs",
            })
            .to_string(),
            result: ToolResult::Error("path does not exist".to_string()),
            duration_ms: 1,
        }],
    );

    assert_eq!(status, None);
}

#[test]
fn success_reconciliation_does_not_complete_not_started_planning_task_from_summary_text_alone() {
    let session_id = format!("agent-loop-success-plan-closeout-{}", uuid::Uuid::new_v4());
    let planning = crate::Task::new(
        &session_id,
        "Plan rollout",
        "Plan the release rollout and checkpoints",
        None,
    );

    let status = AgentPipeline::target_status_for_open_descendant_after_success(
        &session_id,
        &planning,
        "Planned the rollout and everything requested is complete.",
        &[],
    );

    assert_eq!(status, None);
}

#[test]
fn success_reconciliation_keeps_in_progress_verification_open_until_profile_is_satisfied() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-success-profile-{}", uuid::Uuid::new_v4());
    let mut verify = crate::Task::new(
        &session_id,
        "Build and test the result",
        "Run build and test commands",
        None,
    );
    verify.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    manager
        .update_execution_state(&session_id, &verify.id, |state| {
            state.merge_profile(TaskVerificationProfile {
                execution_kind: TaskExecutionKind::Verification,
                requires_build: true,
                requires_test: true,
                ..TaskVerificationProfile::default()
            });
            state.record_evidence(TaskExecutionEvidence::new(
                TaskExecutionEvidenceKind::Build,
                "cargo check -p gestura-gui --quiet",
                Some("shell".to_string()),
                Some("cargo check -p gestura-gui --quiet".to_string()),
            ));
        })
        .expect("execution state update should succeed");

    let stored_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verification lookup should succeed")
        .expect("verification task should exist");

    let status = AgentPipeline::target_status_for_open_descendant_after_success(
        &session_id,
        &stored_verify,
        "Implemented the requested changes and verified the app.",
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": "cargo check -p gestura-gui --quiet",
            })
            .to_string(),
            result: ToolResult::Success("check passed".to_string()),
            duration_ms: 1,
        }],
    );

    assert_eq!(status, None);
}

#[test]
fn success_reconciliation_does_not_complete_verification_from_under_scoped_execution_state() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-success-under-scoped-{}", uuid::Uuid::new_v4());
    let mut verify = crate::Task::new(
        &session_id,
        "Run automated tests",
        "Execute the automated test suite and confirm it passes",
        None,
    );
    verify.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    manager
        .update_execution_state(&session_id, &verify.id, |state| {
            state.merge_profile(TaskVerificationProfile {
                execution_kind: TaskExecutionKind::Verification,
                ..TaskVerificationProfile::default()
            });
            state.record_evidence(TaskExecutionEvidence::new(
                TaskExecutionEvidenceKind::ToolActivity,
                "Reviewed the latest logs without running the test suite",
                Some("read_file".to_string()),
                None,
            ));
        })
        .expect("execution state update should succeed");

    let stored_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verification lookup should succeed")
        .expect("verification task should exist");

    let status = AgentPipeline::target_status_for_open_descendant_after_success(
        &session_id,
        &stored_verify,
        "The requested work is complete and fully verified.",
        &[],
    );

    assert_eq!(status, None);
}

#[test]
fn success_reconciliation_keeps_launch_behavior_task_open_without_launch_evidence() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-success-launch-proof-{}", uuid::Uuid::new_v4());
    let mut verify = crate::Task::new(
        &session_id,
        "Test app launch behavior",
        "Launch the desktop app and confirm the window opens",
        None,
    );
    verify.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    manager
        .update_execution_state(&session_id, &verify.id, |state| {
            state.merge_profile(TaskVerificationProfile {
                execution_kind: TaskExecutionKind::Verification,
                requires_launch_evidence: true,
                ..TaskVerificationProfile::default()
            });
            state.record_evidence(TaskExecutionEvidence::new(
                TaskExecutionEvidenceKind::Test,
                "cargo test",
                Some("shell".to_string()),
                Some("cargo test".to_string()),
            ));
        })
        .expect("execution state update should succeed");

    let stored_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verification lookup should succeed")
        .expect("verification task should exist");

    let status = AgentPipeline::target_status_for_open_descendant_after_success(
        &session_id,
        &stored_verify,
        "Built and tested the app.",
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": "cargo test"
            })
            .to_string(),
            result: ToolResult::Success("test result: ok".to_string()),
            duration_ms: 1,
        }],
    );

    assert_eq!(status, None);
}

#[test]
fn success_reconciliation_keeps_user_closeout_task_open_without_matching_final_summary() {
    let session_id = format!("agent-loop-success-closeout-proof-{}", uuid::Uuid::new_v4());
    let mut summarize = crate::Task::new(
        &session_id,
        "Document results and next steps",
        "Summarize the outcome for the user and list next steps",
        None,
    );
    summarize.set_status(crate::TaskStatus::InProgress);

    let status = AgentPipeline::target_status_for_open_descendant_after_success(
        &session_id,
        &summarize,
        "Built the app and ran cargo test successfully.",
        &[],
    );

    assert_eq!(status, None);
}

#[test]
fn success_reconciliation_keeps_user_closeout_task_open_while_non_closeout_sibling_is_open() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-success-closeout-order-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut summarize = crate::Task::new(
        &session_id,
        "Document results and next steps",
        "Summarize the outcome for the user and list next steps",
        Some(root.id.clone()),
    );
    let mut verify = crate::Task::new(
        &session_id,
        "Test app launch behavior",
        "Launch the desktop app and confirm the window opens",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    summarize.set_status(crate::TaskStatus::InProgress);
    verify.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root);
    task_list.add_task(summarize.clone());
    task_list.add_task(verify);
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    let status = AgentPipeline::target_status_for_open_descendant_after_success(
        &session_id,
        &summarize,
        "Documented results and next steps after implementing the app.",
        &[],
    );

    assert_eq!(status, None);
}

#[test]
fn success_reconciliation_allows_user_closeout_task_when_no_non_closeout_siblings_remain_open() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-success-closeout-ready-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut summarize = crate::Task::new(
        &session_id,
        "Document results and next steps",
        "Summarize the outcome for the user and list next steps",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    summarize.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root);
    task_list.add_task(summarize.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    let status = AgentPipeline::target_status_for_open_descendant_after_success(
        &session_id,
        &summarize,
        "Documented results and next steps after finishing the requested work.",
        &[],
    );

    assert_eq!(status, Some(crate::TaskStatus::Completed));
}

#[test]
fn render_validation_task_requires_launch_evidence() {
    let task = crate::Task::new(
        "test-session",
        "Validate hello world rendering",
        "Launch the app and confirm the hello world UI renders correctly",
        None,
    );

    assert!(AgentPipeline::task_requires_launch_verification(&task));
}

#[test]
fn visual_verification_task_requires_launch_evidence() {
    let task = crate::Task::new(
        "test-session",
        "Verify the UI displays correctly",
        "Confirm the application shows the expected interface",
        None,
    );

    assert!(AgentPipeline::task_requires_launch_verification(&task));
}

#[test]
fn contingent_fix_task_stays_open_while_verification_sibling_is_open() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-contingent-fix-open-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut fix_task = crate::Task::new(
        &session_id,
        "Fix issues from verification",
        "Address any issues found during build and test verification",
        Some(root.id.clone()),
    );
    let mut verify_task = crate::Task::new(
        &session_id,
        "Build and test the app",
        "Run cargo build and cargo test to verify the implementation",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    fix_task.set_status(crate::TaskStatus::NotStarted);
    verify_task.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root);
    task_list.add_task(fix_task.clone());
    task_list.add_task(verify_task);
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    let status = AgentPipeline::target_status_for_open_descendant_after_success(
        &session_id,
        &fix_task,
        "All verification passed successfully.",
        &[],
    );

    assert_eq!(status, None);
}

#[test]
fn contingent_fix_task_can_complete_when_verification_siblings_are_terminal() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-contingent-fix-ready-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut fix_task = crate::Task::new(
        &session_id,
        "Fix issues from verification",
        "Address any issues found during build and test verification",
        Some(root.id.clone()),
    );
    let mut verify_task = crate::Task::new(
        &session_id,
        "Build and test the app",
        "Run cargo build and cargo test to verify the implementation",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    fix_task.set_status(crate::TaskStatus::NotStarted);
    verify_task.set_status(crate::TaskStatus::Completed);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root);
    task_list.add_task(fix_task.clone());
    task_list.add_task(verify_task);
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    // With verification sibling terminal, the contingent fix task should
    // be eligible for normal reconciliation (not blocked by the guard).
    let _status = AgentPipeline::target_status_for_open_descendant_after_success(
        &session_id,
        &fix_task,
        "All verification passed successfully, no issues found.",
        &[],
    );

    // With the verification sibling terminal, the contingent-fix guard
    // does NOT block. The task falls through to the general reconciliation
    // path (which may or may not complete it depending on other evidence).
    // The important assertion is that we got here without the guard
    // returning None — i.e., the guard is not blocking when siblings
    // are terminal.
    assert!(
        !AgentPipeline::contingent_fix_has_open_verification_siblings(&session_id, &fix_task),
        "contingent-fix guard should not block when verification siblings are terminal"
    );
}

#[test]
fn history_validated_direct_proof_rejects_user_closeout_tasks() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-history-closeout-proof-{}", uuid::Uuid::new_v4());
    let summarize = crate::Task::new(
        &session_id,
        "Document results and next steps",
        "Summarize the outcome for the user and list next steps",
        None,
    );

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(summarize.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    manager
        .update_execution_state(&session_id, &summarize.id, |state| {
            state.record_evidence(TaskExecutionEvidence::new(
                TaskExecutionEvidenceKind::ToolActivity,
                "Compiled notes internally but did not deliver the user-facing summary",
                Some("shell".to_string()),
                Some("printf done".to_string()),
            ));
        })
        .expect("execution state update should succeed");

    assert!(
        !AgentPipeline::history_validated_completion_satisfies_direct_proof(
            &session_id,
            &summarize,
        )
    );
}

#[test]
fn tracked_task_reconciliation_completes_started_descendants_after_success() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-cleanup-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut child = crate::Task::new(&session_id, "Child", "Child", Some(root.id.clone()));
    root.set_status(crate::TaskStatus::InProgress);
    child.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: serde_json::json!({
                "operation": "create"
            })
            .to_string(),
            result: ToolResult::Skipped(
                "Loop breaker: skipped a repeated malformed `task.create` call without a valid `name` after 2 prior similar malformed attempts in this run."
                    .to_string(),
            ),
            duration_ms: 1,
        }];

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        "Completed the requested README rewrite and verified the final result.",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    let updated_child = manager
        .get_task(&session_id, &child.id)
        .expect("task lookup should succeed")
        .expect("child should exist");
    assert_eq!(updated_child.status, crate::TaskStatus::Completed);
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[test]
fn runtime_reconciliation_preserves_completed_root_after_children_finish() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-preserve-completed-root-{}",
        uuid::Uuid::new_v4()
    );

    let root = manager
        .create_task(&session_id, "Root", "Root", None)
        .expect("root task");
    let child = manager
        .create_task(&session_id, "Child", "Child", Some(root.id.clone()))
        .expect("child task");

    manager
        .update_task_status(&session_id, &root.id, crate::TaskStatus::InProgress)
        .expect("mark root in progress");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");
    manager
        .update_task_status(&session_id, &child.id, crate::TaskStatus::Completed)
        .expect("complete child");

    assert_eq!(
        manager
            .get_task(&session_id, &root.id)
            .expect("root lookup should succeed")
            .expect("root should exist")
            .status,
        crate::TaskStatus::Completed
    );

    let runtime_state = AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        true,
        true,
        Some(&session_id),
        Some(&root.id),
        &[],
    )
    .expect("runtime state should be available");

    assert!(runtime_state.completion_ready);
    assert!(runtime_state.snapshot.current_task.is_none());
    assert!(runtime_state.snapshot.open_tasks.is_empty());
    assert!(runtime_state.snapshot.missing_requirements.is_empty());
    assert_eq!(
        manager
            .get_task(&session_id, &root.id)
            .expect("root lookup should succeed")
            .expect("root should exist")
            .status,
        crate::TaskStatus::Completed
    );
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[tokio::test]
async fn async_success_reconciliation_completes_started_descendants_after_success() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-async-cleanup-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut child = crate::Task::new(&session_id, "Child", "Child", Some(root.id.clone()));
    root.set_status(crate::TaskStatus::InProgress);
    child.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let pipeline = AgentPipeline::new(AppConfig::default());
    let tool_calls = vec![ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: serde_json::json!({
                "operation": "create"
            })
            .to_string(),
            result: ToolResult::Skipped(
                "Loop breaker: skipped a repeated malformed `task.create` call without a valid `name` after 2 prior similar malformed attempts in this run."
                    .to_string(),
            ),
            duration_ms: 1,
        }];

    pipeline
        .reconcile_tracked_task_after_success_with_history_validation(
            false,
            false,
            Some(&session_id),
            Some(&root.id),
            "Completed the requested README rewrite and verified the final result.",
            &tool_calls,
        )
        .await;

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    let updated_child = manager
        .get_task(&session_id, &child.id)
        .expect("task lookup should succeed")
        .expect("child should exist");
    assert_eq!(updated_child.status, crate::TaskStatus::Completed);
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[tokio::test]
async fn success_reconciliation_clears_incomplete_correction_when_run_is_actually_done() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-closeout-order-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut child = crate::Task::new(&session_id, "Child", "Child", Some(root.id.clone()));
    root.set_status(crate::TaskStatus::InProgress);
    child.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let pipeline = AgentPipeline::new(AppConfig::default());
    let tool_calls = vec![ToolCallRecord {
            id: "1".to_string(),
            name: "task".to_string(),
            arguments: serde_json::json!({
                "operation": "update_status",
                "task_id": child.id,
            })
            .to_string(),
            result: ToolResult::Skipped(
                "Loop breaker: skipped a repeated malformed `task.update_status` call without explicit `status` after 2 prior similar malformed attempts in this run."
                    .to_string(),
            ),
            duration_ms: 1,
        }];
    let final_response = "Completed the requested README rewrite and verified the final result.";

    let correction_before = AgentPipeline::tracked_task_incomplete_terminal_correction_async(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        final_response,
        &tool_calls,
    )
    .await;
    assert!(correction_before.is_some());

    pipeline
        .reconcile_tracked_task_after_success_with_history_validation(
            false,
            false,
            Some(&session_id),
            Some(&root.id),
            final_response,
            &tool_calls,
        )
        .await;

    let correction_after = AgentPipeline::tracked_task_incomplete_terminal_correction_async(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        final_response,
        &tool_calls,
    )
    .await;
    assert!(correction_after.is_none());
    assert_eq!(
        AgentPipeline::tracked_task_closeout_note_async(Some(&session_id), Some(&root.id))
            .await
            .expect("closeout note should exist"),
        "From the tracked task state, everything is closed out now: every subtask is terminal and the overall task is complete."
    );
}

#[test]
fn tracked_task_reconciliation_does_not_complete_root_after_failure_summary() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-no-finalize-on-failure-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "file".to_string(),
        arguments: serde_json::json!({
            "operation": "write",
            "path": "README.md",
            "pattern": "none",
            "recursive": false,
        })
        .to_string(),
        result: ToolResult::Skipped(
            "Loop breaker: skipped a repeated malformed `file.write` call without `content`."
                .to_string(),
        ),
        duration_ms: 1,
    }];

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        "**Final Status:** Unable to rewrite README.md. No changes were made. The task is incomplete.",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::InProgress);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        Some(root.id.clone())
    );
}

#[test]
fn tracked_task_reconciliation_does_not_complete_root_when_summary_claims_success_but_last_non_task_failed()
 {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-no-finalize-on-hallucinated-success-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "read",
                "path": "README.md",
            })
            .to_string(),
            result: ToolResult::Success("original file contents".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "README.md",
                "pattern": "...",
                "recursive": false,
            })
            .to_string(),
            result: ToolResult::Error(
                "Missing required field 'content' for file write operation.".to_string(),
            ),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "README.md",
                "pattern": "none",
                "recursive": false,
            })
            .to_string(),
            result: ToolResult::Skipped(
                "Loop breaker: skipped a repeated malformed `file.write` call without `content`."
                    .to_string(),
            ),
            duration_ms: 1,
        },
    ];

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        true,
        Some(&session_id),
        Some(&root.id),
        "**Updated README.md**\n\n- Converted instructional note into clean final form\n\nCOMPLETE",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::InProgress);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        Some(root.id.clone())
    );
}

#[test]
fn tool_results_support_successful_completion_rejects_late_failed_mutation_followed_only_by_readback()
 {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "edit_file".to_string(),
            arguments: serde_json::json!({
                "path": "swot_analysis.md",
                "old_string": "draft",
                "new_string": "final",
            })
            .to_string(),
            result: ToolResult::Success(
                serde_json::json!({"changed": true, "path": "swot_analysis.md"}).to_string(),
            ),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "edit_file".to_string(),
            arguments: serde_json::json!({
                "path": "swot_analysis.md",
                "old_string": "missing text",
                "new_string": "replacement",
            })
            .to_string(),
            result: ToolResult::Error("I/O error: String to replace not found in file".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "swot_analysis.md"}).to_string(),
            result: ToolResult::Success("rendered markdown".to_string()),
            duration_ms: 1,
        },
    ];

    assert!(!AgentPipeline::tool_results_support_successful_completion(
        true,
        &tool_calls,
    ));
}

#[test]
fn tool_results_support_successful_completion_rejects_semantically_failed_shell_success() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({
            "command": "playwright test || true; npm run lint"
        })
        .to_string(),
        result: ToolResult::Success(
            "8 failed\n12 passed\nnpm run lint completed successfully".to_string(),
        ),
        duration_ms: 1,
    }];

    assert!(!AgentPipeline::tool_results_support_successful_completion(
        false,
        &tool_calls,
    ));
    assert!(AgentPipeline::tool_call_contradiction_summary(&tool_calls[0]).is_some());
}

#[test]
fn successful_http_probe_with_404_is_treated_as_contradiction() {
    let tool_call = ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({
            "command": "curl -I http://localhost:3000/missing"
        })
        .to_string(),
        result: ToolResult::Success("HTTP/1.1 404 Not Found\ncontent-type: text/html".to_string()),
        duration_ms: 1,
    };

    assert!(!AgentPipeline::tool_call_effective_success(&tool_call));
    assert!(
        AgentPipeline::tool_call_contradiction_summary(&tool_call)
            .is_some_and(|summary| summary.contains("HTTP failure response"))
    );
}

#[test]
fn successful_cargo_test_output_with_zero_failed_is_not_treated_as_contradiction() {
    let tool_call = ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({
            "command": "cargo test --manifest-path src-tauri/Cargo.toml"
        })
        .to_string(),
        result: ToolResult::Success(
            "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s"
                .to_string(),
        ),
        duration_ms: 1,
    };

    assert!(AgentPipeline::tool_call_effective_success(&tool_call));
    assert!(AgentPipeline::tool_call_contradiction_summary(&tool_call).is_none());
}

#[test]
fn build_and_test_completion_status_accepts_composite_verification_with_zero_failed_test_output() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({
            "command": "cargo test --manifest-path src-tauri/Cargo.toml && cargo check --manifest-path src-tauri/Cargo.toml && cargo build --manifest-path src-tauri/Cargo.toml --release"
        })
        .to_string(),
        result: ToolResult::Success(
            "Finished `test` profile [unoptimized + debuginfo] target(s) in 0.39s\nRunning unittests src/lib.rs\n\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\nFinished `dev` profile [unoptimized + debuginfo] target(s) in 0.45s\nFinished `release` profile [optimized] target(s) in 0.46s"
                .to_string(),
        ),
        duration_ms: 1,
    }];

    assert_eq!(
        AgentPipeline::build_and_test_completion_status(&tool_calls),
        (true, true, true, true)
    );
}

#[test]
fn successful_tauri_build_output_with_mismatched_versions_info_is_not_a_contradiction() {
    let tool_call = ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({
            "command": "npm install && cargo test --manifest-path src-tauri/Cargo.toml && cargo check --manifest-path src-tauri/Cargo.toml && npm run tauri build -- --no-bundle --ci"
        })
        .to_string(),
        result: ToolResult::Success(
            "up to date, audited 3 packages in 798ms\n\nfound 0 vulnerabilities\n\nFinished `test` profile [unoptimized + debuginfo] target(s) in 0.44s\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\nInfo Looking up installed tauri packages to check mismatched versions...\nFinished `release` profile [optimized] target(s) in 0.41s\nBuilt application at: /Users/example/src-tauri/target/release/tauri-app"
                .to_string(),
        ),
        duration_ms: 1,
    };

    assert!(AgentPipeline::tool_call_effective_success(&tool_call));
    assert!(AgentPipeline::tool_call_contradiction_summary(&tool_call).is_none());
}

#[test]
fn tracked_task_reconciliation_does_not_complete_mutating_request_after_read_only_successes() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-no-finalize-on-read-only-success-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "read",
                "path": "README.md",
            })
            .to_string(),
            result: ToolResult::Success("original file contents".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "read",
                "path": "README.md",
            })
            .to_string(),
            result: ToolResult::Success("original file contents again".to_string()),
            duration_ms: 1,
        },
    ];

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        true,
        Some(&session_id),
        Some(&root.id),
        "**Updated README.md**\n\n- Converted instructional note into clean final form\n\nCOMPLETE",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::InProgress);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        Some(root.id.clone())
    );
}

#[test]
fn tracked_task_reconciliation_completes_markdown_response_request_after_research_only_successes() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-finalize-markdown-response-after-research-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "web_search".to_string(),
            arguments: serde_json::json!({
                "query": "smart home lighting market trends 2025 2026 forecast",
            })
            .to_string(),
            result: ToolResult::Success("Found supporting market sources".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "web_search".to_string(),
            arguments: serde_json::json!({
                "query": "smart lighting market CAGR 2025 verification",
            })
            .to_string(),
            result: ToolResult::Success("Cross-checked market CAGR claims".to_string()),
            duration_ms: 1,
        },
    ];

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        "**SWOT Analysis**\n\n- Strengths: ...\n- Weaknesses: ...\n- Opportunities: ...\n- Threats: ...\n\nI cross-checked the market claims against multiple independent sources and noted the assumptions inline.",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[test]
fn tracked_task_reconciliation_completes_mutating_request_after_successful_write_and_readback() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-finalize-after-write-success-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "README.md",
                "content": "# Project\n- done\nCOMPLETE\n",
            })
            .to_string(),
            result: ToolResult::Success("Written to README.md".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "read",
                "path": "README.md",
            })
            .to_string(),
            result: ToolResult::Success("# Project\n- done\nCOMPLETE\n".to_string()),
            duration_ms: 1,
        },
    ];

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        true,
        Some(&session_id),
        Some(&root.id),
        "Updated README.md and verified the final result.",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[test]
fn tracked_task_reconciliation_accepts_mutating_shell_scaffold_with_build_and_test() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-shell-scaffold-success-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": "uv init hello-world"
            })
            .to_string(),
            result: ToolResult::Success("created project".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "python -m build"}).to_string(),
            result: ToolResult::Success("build ok".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "pytest -q"}).to_string(),
            result: ToolResult::Success("tests ok".to_string()),
            duration_ms: 1,
        },
    ];

    AgentPipeline::reconcile_tracked_task_after_success(
        true,
        true,
        Some(&session_id),
        Some(&root.id),
        "The sample app is complete. The project was scaffolded, built successfully, and tests passed.",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[test]
fn tracked_task_reconciliation_accepts_generic_verification_for_client_work() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-frontend-backend-only-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "hello-world/client/main.js",
                "content": "document.querySelector('#app').textContent = 'Hello world';\n"
            })
            .to_string(),
            result: ToolResult::Success("Wrote hello-world/client/main.js".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": "cd hello-world/client && npm run build"
            })
            .to_string(),
            result: ToolResult::Error("npm ERR! Missing script: \"build\"".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": "cd hello-world/server && cargo check"
            })
            .to_string(),
            result: ToolResult::Success("build ok".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "4".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": "cd hello-world/server && cargo test --quiet"
            })
            .to_string(),
            result: ToolResult::Success("tests ok".to_string()),
            duration_ms: 1,
        },
    ];

    AgentPipeline::reconcile_tracked_task_after_success(
        true,
        true,
        Some(&session_id),
        Some(&root.id),
        "Client update is complete. The app was implemented, built successfully, and tests passed.",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[test]
fn no_op_file_write_does_not_count_as_successful_mutation() {
    let tool_call = ToolCallRecord {
        id: "1".to_string(),
        name: "file".to_string(),
        arguments: serde_json::json!({
            "operation": "write",
            "path": "index.html",
            "content": "<h1>Hello</h1>\n"
        })
        .to_string(),
        result: ToolResult::Success(
            "Write to index.html made no changes; content already matched the existing file."
                .to_string(),
        ),
        duration_ms: 1,
    };

    assert!(!AgentPipeline::is_successful_mutating_file_tool_call(
        &tool_call
    ));
}

#[test]
fn tracked_task_reconciliation_rejects_noop_source_mutation_even_if_shell_scaffold_build_and_test_succeeded()
 {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-noop-source-mutation-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
            ToolCallRecord {
                id: "1".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({
                    "command": "uv init hello-world"
                })
                .to_string(),
                result: ToolResult::Success("created project".to_string()),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "file".to_string(),
                arguments: serde_json::json!({
                    "operation": "write",
                    "path": "hello-world/src/main.py",
                    "content": "print('hello')"
                })
                .to_string(),
                result: ToolResult::Success(
                    "Write to hello-world/src/main.py made no changes; content already matched the existing file.".to_string(),
                ),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "3".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({"command": "python -m build"}).to_string(),
                result: ToolResult::Success("build ok".to_string()),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "4".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({"command": "pytest -q"}).to_string(),
                result: ToolResult::Success("tests ok".to_string()),
                duration_ms: 1,
            },
        ];

    AgentPipeline::reconcile_tracked_task_after_success(
        true,
        true,
        Some(&session_id),
        Some(&root.id),
        "The sample app is complete. The project was scaffolded, built successfully, and tests passed.",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    let lifecycle = manager
        .get_memory_lifecycle(&session_id, &root.id)
        .expect("memory lifecycle lookup should succeed")
        .expect("root task should record a lifecycle event");
    assert_eq!(updated_root.status, crate::TaskStatus::InProgress);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        Some(root.id)
    );
    assert_eq!(
        lifecycle.events.last().map(|event| event.phase),
        Some(crate::tasks::TaskMemoryPhase::Blocked)
    );
    assert!(
        lifecycle
            .events
            .last()
            .expect("blocked lifecycle event should be present")
            .summary
            .contains("source mutation not yet verified")
    );
}

#[test]
fn tracked_task_reconciliation_cleans_up_not_started_descendants_after_success() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-finalize-after-stale-placeholder-descendants-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);
    let child = crate::Task::new(
        &session_id,
        "None But Omit",
        "placeholder",
        Some(root.id.clone()),
    );

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "README.md",
                "content": "# Project\n- done\n",
            })
            .to_string(),
            result: ToolResult::Success("Written to README.md".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "read",
                "path": "README.md",
            })
            .to_string(),
            result: ToolResult::Success("# Project\n- done\n".to_string()),
            duration_ms: 1,
        },
    ];

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        true,
        Some(&session_id),
        Some(&root.id),
        "Completed the requested README rewrite and verified the final result.",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    let updated_child = manager
        .get_task(&session_id, &child.id)
        .expect("child lookup should succeed")
        .expect("child should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert_eq!(updated_child.status, crate::TaskStatus::Cancelled);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[tokio::test]
async fn async_success_reconciliation_cleans_up_not_started_descendants_after_success() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-async-finalize-after-stale-placeholder-descendants-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);
    let child = crate::Task::new(
        &session_id,
        "None But Omit",
        "placeholder",
        Some(root.id.clone()),
    );

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let pipeline = AgentPipeline::new(AppConfig::default());
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "README.md",
                "content": "# Project\n- done\n",
            })
            .to_string(),
            result: ToolResult::Success("Written to README.md".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "read",
                "path": "README.md",
            })
            .to_string(),
            result: ToolResult::Success("# Project\n- done\n".to_string()),
            duration_ms: 1,
        },
    ];

    pipeline
        .reconcile_tracked_task_after_success_with_history_validation(
            false,
            true,
            Some(&session_id),
            Some(&root.id),
            "Completed the requested README rewrite and verified the final result.",
            &tool_calls,
        )
        .await;

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    let updated_child = manager
        .get_task(&session_id, &child.id)
        .expect("child lookup should succeed")
        .expect("child should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert_eq!(updated_child.status, crate::TaskStatus::Cancelled);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[test]
fn no_tool_success_response_reconciles_placeholder_descendants_into_terminal_closeout() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-no-tool-success-reconcile-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);
    let child = crate::Task::new(
        &session_id,
        "None But Omit",
        "placeholder",
        Some(root.id.clone()),
    );

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let pipeline = AgentPipeline::new(AppConfig::default());
    let summary = pipeline.tracked_open_descendant_summary_after_success_reconciliation(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        "All requested steps are complete and the generated project is ready.",
        &[],
    );

    assert_eq!(summary, OpenDescendantSummary::default());

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    let updated_child = manager
        .get_task(&session_id, &child.id)
        .expect("child lookup should succeed")
        .expect("child should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert_eq!(updated_child.status, crate::TaskStatus::Cancelled);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[test]
fn no_tool_success_response_does_not_close_descendants_when_required_evidence_is_missing() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-no-tool-success-missing-evidence-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);
    let verify = crate::Task::new(
        &session_id,
        "Run automated tests",
        "Execute the automated test suite and confirm it passes",
        Some(root.id.clone()),
    );

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let pipeline = AgentPipeline::new(AppConfig::default());
    let summary = pipeline.tracked_open_descendant_summary_after_success_reconciliation(
        true,
        false,
        Some(&session_id),
        Some(&root.id),
        "Everything is complete. The project built successfully and tests passed.",
        &[],
    );

    assert!(summary.has_open());

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("root lookup should succeed")
        .expect("root should exist");
    let updated_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verification lookup should succeed")
        .expect("verification task should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::InProgress);
    assert!(!updated_verify.is_terminal());
}

#[test]
fn tracked_task_reconciliation_keeps_root_open_when_non_terminal_descendants_remain() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-terminalize-open-descendants-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);
    let child = crate::Task::new(
        &session_id,
        "Document follow-up rollout plan",
        "Write a separate rollout plan after the implementation summary",
        Some(root.id.clone()),
    );

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        "Completed the requested implementation and verified the final result.",
        &[],
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    let updated_child = manager
        .get_task(&session_id, &child.id)
        .expect("task lookup should succeed")
        .expect("child should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::InProgress);
    assert_eq!(updated_child.status, crate::TaskStatus::NotStarted);
}

#[test]
fn results_review_narration_does_not_report_completion_until_snapshot_is_fully_clear() {
    let snapshot = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: None,
        ready_tasks: vec![crate::streaming::TaskRuntimeTaskView {
            id: "verify".to_string(),
            name: "Verify build results".to_string(),
            status: "not_started".to_string(),
        }],
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        missing_requirements: Vec::new(),
        status_message: "Verification still needs to run".to_string(),
    };

    let change_kind =
        AgentPipeline::results_review_narration_change_kind(Some(&snapshot), None, &[]);

    assert_ne!(change_kind, PublicNarrationChangeKind::Completion);
}

#[test]
fn results_review_narration_prioritizes_failed_latest_tool_over_completion_snapshot() {
    let snapshot = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: None,
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: Vec::new(),
        completed_tasks: vec![crate::streaming::TaskRuntimeTaskView {
            id: "verify".to_string(),
            name: "Build Tauri app".to_string(),
            status: "completed".to_string(),
        }],
        missing_requirements: Vec::new(),
        status_message: "Everything is complete.".to_string(),
    };
    let recent_tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({
            "command": "npm run tauri build -- --bundles app"
        })
        .to_string(),
        result: ToolResult::Error("bundle build failed".to_string()),
        duration_ms: 1,
    }];

    let change_kind = AgentPipeline::results_review_narration_change_kind(
        Some(&snapshot),
        None,
        &recent_tool_calls,
    );

    assert_eq!(change_kind, PublicNarrationChangeKind::Contradiction);
}

#[test]
fn runtime_reconciliation_does_not_reopen_completed_root_when_open_descendants_exist() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-sticky-completed-root-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let child = crate::Task::new(
        &session_id,
        "Follow-up verification",
        "Run the remaining verification",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::Completed);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    let runtime_state = AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        false,
        false,
        Some(&session_id),
        Some(&root.id),
        &[],
    )
    .expect("runtime state should exist");

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert!(runtime_state.open_descendant_summary.has_open());
}

#[test]
fn broad_plan_completion_text_is_detected_without_triggering_on_generic_success() {
    assert!(AgentPipeline::text_signals_broad_plan_completion(
        "All planned deliverables are now finished and the file is ready for review."
    ));
    assert!(AgentPipeline::text_signals_broad_plan_completion(
        "All requested steps are complete."
    ));
    assert!(!AgentPipeline::text_signals_broad_plan_completion(
        "Completed the requested implementation and verified the final result."
    ));
}

#[test]
fn fact_cross_check_tasks_are_generic_verification_not_build_or_test_verification() {
    let task = crate::Task::new(
        "session",
        "Verify facts and cross-check",
        "Cross-check the key claims in the final SWOT output",
        None,
    );

    let profile = AgentPipeline::task_execution_profile(&task, false);

    assert_eq!(profile.execution_kind, TaskExecutionKind::Verification);
    assert!(!profile.requires_build);
    assert!(!profile.requires_test);
}

#[test]
fn tracked_task_reconciliation_keeps_verification_descendant_open_after_broad_plan_completion_claim()
 {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-broad-plan-closeout-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);
    let compile = crate::Task::new(
        &session_id,
        "Compile SWOT Points",
        "Assemble the researched SWOT bullets into final prose",
        Some(root.id.clone()),
    );
    let verify = crate::Task::new(
        &session_id,
        "Verify Facts & Cross-Check",
        "Cross-check the key claims in the final SWOT output",
        Some(root.id.clone()),
    );

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(compile.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "swot-smart-home-lighting.md",
                "content": "# SWOT\n- item\n",
            })
            .to_string(),
            result: ToolResult::Success("Written to swot-smart-home-lighting.md".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "read",
                "path": "swot-smart-home-lighting.md",
            })
            .to_string(),
            result: ToolResult::Success("# SWOT\n- item\n".to_string()),
            duration_ms: 1,
        },
    ];

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        true,
        Some(&session_id),
        Some(&root.id),
        "All planned deliverables are now finished. The SWOT markdown is complete and verified.",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("root lookup should succeed")
        .expect("root should exist");
    let updated_compile = manager
        .get_task(&session_id, &compile.id)
        .expect("compile lookup should succeed")
        .expect("compile task should exist");
    let updated_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verify lookup should succeed")
        .expect("verify task should exist");

    assert_eq!(updated_root.status, crate::TaskStatus::InProgress);
    assert_eq!(updated_compile.status, crate::TaskStatus::Completed);
    assert_eq!(updated_verify.status, crate::TaskStatus::InProgress);
    assert!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed")
            .is_some()
    );
}

#[test]
fn tracked_task_reconciliation_completes_generic_verification_descendant_from_review_evidence() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-generic-verification-closeout-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);
    let verify = crate::Task::new(
        &session_id,
        "Verify facts and cross-check",
        "Cross-check the key claims in the final SWOT output",
        Some(root.id.clone()),
    );

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "smart_home_lighting_swot.md",
                "content": "# SWOT\n- updated\n",
            })
            .to_string(),
            result: ToolResult::Success("Written to smart_home_lighting_swot.md".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({
                "path": "smart_home_lighting_swot.md",
            })
            .to_string(),
            result: ToolResult::Success("# SWOT\n- updated\n".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "web_search".to_string(),
            arguments: serde_json::json!({
                "query": "smart home lighting market SWOT verification",
            })
            .to_string(),
            result: ToolResult::Success("Verified supporting sources".to_string()),
            duration_ms: 1,
        },
    ];

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        true,
        Some(&session_id),
        Some(&root.id),
        "I updated the SWOT markdown, cross-checked the claims against recent sources, and reviewed the final file.",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("root lookup should succeed")
        .expect("root should exist");
    let updated_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verify lookup should succeed")
        .expect("verify task should exist");

    assert_eq!(updated_verify.status, crate::TaskStatus::Completed);
    assert_eq!(updated_root.status, crate::TaskStatus::Completed);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[test]
fn tracked_task_reconciliation_keeps_cross_check_verification_open_without_post_mutation_external_evidence()
 {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-cross-check-needs-fresh-external-proof-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);
    let verify = crate::Task::new(
        &session_id,
        "Verify facts and cross-check",
        "Cross-check the key claims in the final SWOT output",
        Some(root.id.clone()),
    );

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "web_search".to_string(),
            arguments: serde_json::json!({
                "query": "smart home lighting market size 2025"
            })
            .to_string(),
            result: ToolResult::Success("Earlier research result".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "smart_home_lighting_swot.md",
                "content": "# SWOT\n- updated\n",
            })
            .to_string(),
            result: ToolResult::Success("Written to smart_home_lighting_swot.md".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "3".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({
                "path": "smart_home_lighting_swot.md",
            })
            .to_string(),
            result: ToolResult::Success("# SWOT\n- updated\n".to_string()),
            duration_ms: 1,
        },
    ];

    AgentPipeline::reconcile_tracked_task_after_success(
        false,
        true,
        Some(&session_id),
        Some(&root.id),
        "I updated the SWOT markdown and reviewed the final file.",
        &tool_calls,
    );

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("root lookup should succeed")
        .expect("root should exist");
    let updated_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verify lookup should succeed")
        .expect("verify task should exist");

    assert_eq!(updated_verify.status, crate::TaskStatus::InProgress);
    assert_eq!(updated_root.status, crate::TaskStatus::InProgress);
}

#[test]
fn parse_closeout_history_validation_response_accepts_json_fences() {
    let parsed = AgentPipeline::parse_closeout_history_validation_response(
        "```json\n{\"completed_task_ids\":[\"task-1\",\"task-2\"]}\n```",
    )
    .expect("response should parse");

    assert_eq!(parsed.completed_task_ids, vec!["task-1", "task-2"]);
}

#[test]
fn apply_history_validated_descendant_completions_completes_nested_tasks_depth_first() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-history-validated-{}", uuid::Uuid::new_v4());
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut parent = crate::Task::new(
        &session_id,
        "Implement backend",
        "Finish backend work",
        Some(root.id.clone()),
    );
    let mut child = crate::Task::new(
        &session_id,
        "Add endpoint",
        "Ship the nested endpoint",
        Some(parent.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    parent.set_status(crate::TaskStatus::InProgress);
    child.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(parent.clone());
    task_list.add_task(child.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .update_execution_state(&session_id, &parent.id, |state| {
            state.merge_profile(AgentPipeline::task_execution_profile(&parent, false));
            state.record_evidence(TaskExecutionEvidence::new(
                TaskExecutionEvidenceKind::Mutation,
                "Completed the parent implementation work",
                Some("write_file".to_string()),
                None,
            ));
        })
        .expect("parent execution state update should succeed");
    manager
        .update_execution_state(&session_id, &child.id, |state| {
            state.merge_profile(AgentPipeline::task_execution_profile(&child, false));
            state.record_evidence(TaskExecutionEvidence::new(
                TaskExecutionEvidenceKind::Mutation,
                "Completed the child implementation work",
                Some("write_file".to_string()),
                None,
            ));
        })
        .expect("child execution state update should succeed");

    let open_descendants = AgentPipeline::load_open_descendants(&session_id, &root.id)
        .expect("descendants should load");
    let applied = AgentPipeline::apply_history_validated_descendant_completions(
        &session_id,
        &root.id,
        &open_descendants,
        &[parent.id.clone(), child.id.clone()],
    );

    assert_eq!(applied, vec![child.id.clone(), parent.id.clone()]);

    let stored_parent = manager
        .get_task(&session_id, &parent.id)
        .expect("parent lookup should succeed")
        .expect("parent should exist");
    let stored_child = manager
        .get_task(&session_id, &child.id)
        .expect("child lookup should succeed")
        .expect("child should exist");
    assert_eq!(stored_parent.status, crate::TaskStatus::Completed);
    assert_eq!(stored_child.status, crate::TaskStatus::Completed);
}

#[test]
fn apply_history_validated_descendant_completions_skips_cross_check_without_satisfied_direct_proof()
{
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-history-cross-check-open-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut verify = crate::Task::new(
        &session_id,
        "Verify facts and cross-check",
        "Cross-check the key claims in the final SWOT output",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    verify.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .update_execution_state(&session_id, &verify.id, |state| {
            state.record_evidence(TaskExecutionEvidence::new(
                TaskExecutionEvidenceKind::ToolActivity,
                "Reviewed the generated SWOT markdown locally",
                Some("read_file".to_string()),
                None,
            ));
        })
        .expect("execution state update should succeed");

    let open_descendants = AgentPipeline::load_open_descendants(&session_id, &root.id)
        .expect("descendants should load");
    let applied = AgentPipeline::apply_history_validated_descendant_completions(
        &session_id,
        &root.id,
        &open_descendants,
        &[verify.id.clone()],
    );

    let stored_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verify lookup should succeed")
        .expect("verify should exist");

    assert!(applied.is_empty());
    assert_eq!(stored_verify.status, crate::TaskStatus::InProgress);
}

#[test]
fn apply_history_validated_descendant_completions_allows_cross_check_after_satisfied_direct_proof()
{
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-history-cross-check-complete-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut verify = crate::Task::new(
        &session_id,
        "Verify facts and cross-check",
        "Cross-check the key claims in the final SWOT output",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    verify.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .update_execution_state(&session_id, &verify.id, |state| {
            state.merge_profile(AgentPipeline::task_execution_profile(&verify, false));
            state.record_evidence(TaskExecutionEvidence::new(
                TaskExecutionEvidenceKind::Mutation,
                "Updated the SWOT markdown before verification",
                Some("write_file".to_string()),
                None,
            ));
            state.record_evidence(TaskExecutionEvidence::new(
                TaskExecutionEvidenceKind::ToolActivity,
                "Cross-checked the updated SWOT against recent sources",
                Some("web_search".to_string()),
                None,
            ));
        })
        .expect("execution state update should succeed");

    let open_descendants = AgentPipeline::load_open_descendants(&session_id, &root.id)
        .expect("descendants should load");
    let applied = AgentPipeline::apply_history_validated_descendant_completions(
        &session_id,
        &root.id,
        &open_descendants,
        &[verify.id.clone()],
    );

    let stored_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verify lookup should succeed")
        .expect("verify should exist");

    assert_eq!(applied, vec![verify.id.clone()]);
    assert_eq!(stored_verify.status, crate::TaskStatus::Completed);
}

#[test]
fn terminalize_remaining_open_descendants_after_success_closeout_without_broad_claim_completes_started_and_cancels_not_started()
 {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-terminalize-success-closeout-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let mut started = crate::Task::new(
        &session_id,
        "Implement API",
        "Finish the API work",
        Some(root.id.clone()),
    );
    let not_started = crate::Task::new(
        &session_id,
        "Write docs",
        "Document the completed work",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);
    started.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(started.clone());
    task_list.add_task(not_started.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");

    let mut applied = AgentPipeline::terminalize_remaining_open_descendants_after_success_closeout(
        &session_id,
        &root.id,
        false,
    );
    applied.sort_by(|left, right| left.0.cmp(&right.0));

    let mut expected = vec![
        (started.id.clone(), crate::TaskStatus::Completed),
        (not_started.id.clone(), crate::TaskStatus::Cancelled),
    ];
    expected.sort_by(|left, right| left.0.cmp(&right.0));

    assert_eq!(applied, expected);

    let stored_started = manager
        .get_task(&session_id, &started.id)
        .expect("started lookup should succeed")
        .expect("started should exist");
    let stored_not_started = manager
        .get_task(&session_id, &not_started.id)
        .expect("not-started lookup should succeed")
        .expect("not-started should exist");
    assert_eq!(stored_started.status, crate::TaskStatus::Completed);
    assert_eq!(stored_not_started.status, crate::TaskStatus::Cancelled);
}

#[test]
fn terminalize_remaining_open_descendants_after_success_closeout_with_broad_claim_keeps_direct_proof_tasks_open()
 {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-terminalize-broad-plan-closeout-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    let started = crate::Task::new(
        &session_id,
        "Draft final report",
        "Finish the drafted summary",
        Some(root.id.clone()),
    );
    let implied = crate::Task::new(
        &session_id,
        "Verify facts and cross-check",
        "Cross-check the claims in the completed report",
        Some(root.id.clone()),
    );
    let placeholder = crate::Task::new(
        &session_id,
        "TBD",
        "Placeholder follow-up",
        Some(root.id.clone()),
    );
    root.set_status(crate::TaskStatus::InProgress);

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(started.clone());
    task_list.add_task(implied.clone());
    task_list.add_task(placeholder.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .update_task_status(&session_id, &started.id, crate::TaskStatus::InProgress)
        .expect("mark started in progress");

    let mut applied = AgentPipeline::terminalize_remaining_open_descendants_after_success_closeout(
        &session_id,
        &root.id,
        true,
    );
    applied.sort_by(|left, right| left.0.cmp(&right.0));

    let mut expected = vec![
        (started.id.clone(), crate::TaskStatus::Completed),
        (placeholder.id.clone(), crate::TaskStatus::Cancelled),
    ];
    expected.sort_by(|left, right| left.0.cmp(&right.0));

    assert_eq!(applied, expected);

    let stored_implied = manager
        .get_task(&session_id, &implied.id)
        .expect("implied lookup should succeed")
        .expect("implied should exist");
    assert_eq!(stored_implied.status, crate::TaskStatus::NotStarted);
}

#[test]
fn tracked_task_cancellation_marks_root_and_descendants_cancelled() {
    let manager = crate::get_global_task_manager();
    let session_id = format!("agent-loop-cancel-{}", uuid::Uuid::new_v4());
    let root = crate::Task::new(&session_id, "Root", "Root", None);
    let child = crate::Task::new(
        &session_id,
        "Pending child",
        "Pending child",
        Some(root.id.clone()),
    );

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(child.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(root.id.clone()))
        .expect("set current task");

    AgentPipeline::cancel_tracked_task(Some(&session_id), Some(&root.id), "test cancellation");

    let updated_root = manager
        .get_task(&session_id, &root.id)
        .expect("task lookup should succeed")
        .expect("root should exist");
    let updated_child = manager
        .get_task(&session_id, &child.id)
        .expect("task lookup should succeed")
        .expect("child should exist");
    assert_eq!(updated_root.status, crate::TaskStatus::Cancelled);
    assert_eq!(updated_child.status, crate::TaskStatus::Cancelled);
    assert_eq!(
        manager
            .get_current_task_id(&session_id)
            .expect("current task lookup should succeed"),
        None
    );
}

#[test]
fn tool_iteration_stagnation_fingerprint_tracks_repeated_no_progress_generically() {
    let shell_failure = |id: &str, command: &str| ToolCallRecord {
        id: id.to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({"command": command}).to_string(),
        result: ToolResult::Error(
            "Couldn't recognize the current folder as a Tauri project.".to_string(),
        ),
        duration_ms: 1,
    };

    let first = AgentPipeline::tool_iteration_stagnation_fingerprint(
        true,
        false,
        &[shell_failure("1", "cargo tauri build --verbose")],
        None,
    );
    let second = AgentPipeline::tool_iteration_stagnation_fingerprint(
        true,
        false,
        &[shell_failure("2", "cargo tauri init --ci --force")],
        None,
    );

    assert_eq!(first, second);
    assert!(
        first
            .missing_requirements
            .iter()
            .any(|message| message == "build/check command not yet observed")
    );
    assert!(
        first
            .missing_requirements
            .iter()
            .any(|message| message == "test command not yet observed")
    );
    assert!(
        first
            .missing_requirements
            .iter()
            .any(|message| message.starts_with("unresolved blocker:")
                || message.starts_with("unresolved contradiction:"))
    );
}

#[test]
fn stagnation_summary_mentions_repeated_contradictions() {
    let fingerprint = AgentPipeline::tool_iteration_stagnation_fingerprint(
        false,
        false,
        &[ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({
                "command": "curl -I http://localhost:3000/missing"
            })
            .to_string(),
            result: ToolResult::Success("HTTP/1.1 404 Not Found".to_string()),
            duration_ms: 1,
        }],
        None,
    );

    let summary = AgentPipeline::summarize_stagnation_fingerprint(&fingerprint);
    assert!(summary.contains("repeated contradiction"));
}

#[test]
fn tool_iteration_stagnation_fingerprint_distinguishes_distinct_successful_rewrites() {
    let successful_write = |id: &str, content: &str| ToolCallRecord {
        id: id.to_string(),
        name: "file".to_string(),
        arguments: serde_json::json!({
            "operation": "write",
            "path": "crates/gestura-gui/frontend/src/App.tsx",
            "content": content,
        })
        .to_string(),
        result: ToolResult::Success(
            "Written to crates/gestura-gui/frontend/src/App.tsx".to_string(),
        ),
        duration_ms: 1,
    };

    let first = AgentPipeline::tool_iteration_stagnation_fingerprint(
        false,
        true,
        &[successful_write("1", "<h1>Hello</h1>\n")],
        None,
    );
    let second = AgentPipeline::tool_iteration_stagnation_fingerprint(
        false,
        true,
        &[successful_write("2", "<h1>Hello from Gestura</h1>\n")],
        None,
    );

    assert_ne!(first, second);
    assert_ne!(first.outcome_fingerprints, second.outcome_fingerprints);
}

#[test]
fn tool_iteration_stagnation_fingerprint_still_matches_identical_successful_rewrites() {
    let successful_write = |id: &str| ToolCallRecord {
        id: id.to_string(),
        name: "file".to_string(),
        arguments: serde_json::json!({
            "operation": "write",
            "path": "crates/gestura-gui/frontend/src/App.tsx",
            "content": "<h1>Hello</h1>\n",
        })
        .to_string(),
        result: ToolResult::Success(
            "Written to crates/gestura-gui/frontend/src/App.tsx".to_string(),
        ),
        duration_ms: 1,
    };

    let first = AgentPipeline::tool_iteration_stagnation_fingerprint(
        false,
        true,
        &[successful_write("1")],
        None,
    );
    let second = AgentPipeline::tool_iteration_stagnation_fingerprint(
        false,
        true,
        &[successful_write("2")],
        None,
    );

    assert_eq!(first, second);
}

#[test]
fn runtime_snapshot_narration_fingerprint_changes_only_on_material_runtime_deltas() {
    let snapshot = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Inspect the current state and constraints".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        missing_requirements: vec![
            "source mutation not yet verified".to_string(),
            "test command not yet observed".to_string(),
        ],
        status_message: "Inspect task is active".to_string(),
    };

    let (_, first_message, first_fingerprint) =
        AgentPipeline::runtime_snapshot_narration(&snapshot, None);

    let mut wording_only_change = snapshot.clone();
    wording_only_change.status_message = "A different status banner".to_string();
    let (_, second_message, second_fingerprint) =
        AgentPipeline::runtime_snapshot_narration(&wording_only_change, None);

    let mut material_change = snapshot.clone();
    material_change.missing_requirements = vec!["test command not yet observed".to_string()];
    let (_, _, third_fingerprint) =
        AgentPipeline::runtime_snapshot_narration(&material_change, None);

    assert_eq!(first_message, second_message);
    assert_eq!(first_fingerprint, second_fingerprint);
    assert_ne!(first_fingerprint, third_fingerprint);
    assert!(!first_message.contains("source mutation not yet verified"));
}

#[test]
fn runtime_snapshot_narration_surfaces_focus_completion_and_requirement_deltas() {
    let previous = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Inspect the current state and constraints".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        missing_requirements: vec!["test command not yet observed".to_string()],
        status_message: "Inspect task is active".to_string(),
    };
    let current = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "task-2".to_string(),
            name: "Run verification checks".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: vec![crate::streaming::TaskRuntimeTaskView {
            id: "task-3".to_string(),
            name: "Summarize the validation results".to_string(),
            status: "ready".to_string(),
        }],
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: Vec::new(),
        completed_tasks: vec![crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Inspect the current state and constraints".to_string(),
            status: "completed".to_string(),
        }],
        missing_requirements: Vec::new(),
        status_message: "Verification is active".to_string(),
    };

    let (stage, message, _) = AgentPipeline::runtime_snapshot_narration(&current, Some(&previous));

    assert_eq!(stage, crate::streaming::NarrationStage::Verification);
    assert!(message.contains(
            "The focused task shifted from \"Inspect the current state and constraints\" to \"Run verification checks\"."
        ));
    assert!(
        message.contains("Newly finished work: \"Inspect the current state and constraints\".")
    );
    assert!(message.contains("Cleared 1 remaining check."));
    assert!(message.contains("Next up: \"Summarize the validation results\"."));
}

#[test]
fn incomplete_runtime_snapshot_forces_deterministic_public_narration() {
    let snapshot = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root-task".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "verify-task".to_string(),
            name: "Verify facts and cross-check".to_string(),
            status: "not_started".to_string(),
        }),
        ready_tasks: vec![crate::streaming::TaskRuntimeTaskView {
            id: "verify-task".to_string(),
            name: "Verify facts and cross-check".to_string(),
            status: "not_started".to_string(),
        }],
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: vec![crate::streaming::TaskRuntimeTaskView {
            id: "verify-task".to_string(),
            name: "Verify facts and cross-check".to_string(),
            status: "not_started".to_string(),
        }],
        completed_tasks: Vec::new(),
        missing_requirements: vec!["verification still required".to_string()],
        status_message: "Verification remains open".to_string(),
    };

    assert!(
        AgentPipeline::should_force_runtime_snapshot_public_narration(
            PublicNarrationTrigger::ResultsReview,
            Some(&snapshot),
            &[],
        )
    );
    assert!(
        !AgentPipeline::should_force_runtime_snapshot_public_narration(
            PublicNarrationTrigger::BatchStart,
            Some(&snapshot),
            &[],
        )
    );
}

#[test]
fn finalize_public_narration_rejects_completion_claim_when_runtime_is_incomplete() {
    let context_frame = PublicNarrationContextFrame {
        stage: crate::streaming::NarrationStage::Blocked,
        change_kind: PublicNarrationChangeKind::Blocker,
        summary_hint: Some(
            "I still need to clear the remaining verification step before I can close this out."
                .to_string(),
        ),
        reason_hint: Some(
            "The tracked runtime still shows open work, so a success claim would be misleading."
                .to_string(),
        ),
        next_step_hint: Some("Finish the outstanding verification step.".to_string()),
        evidence: vec!["Still need to verify: run the final validation step.".to_string()],
        tracked_work_incomplete: true,
        completion_ready: false,
    };

    let narration = AgentPipeline::finalize_public_narration(
        crate::streaming::NarrationStage::Blocked,
        Some("shell"),
        PublicNarrationDraft {
            title: Some("I've completed the requested work".to_string()),
            message: Some(
                "I've completed the requested work and everything is fully verified.".to_string(),
            ),
            ..PublicNarrationDraft::default()
        },
        &context_frame,
    )
    .expect("narration should still be produced");

    assert!(!AgentPipeline::public_narration_claims_completion(
        &narration.title
    ));
    assert!(!AgentPipeline::public_narration_claims_completion(
        &narration.message
    ));
    assert!(
        narration
            .message
            .to_ascii_lowercase()
            .contains("still need")
    );
}

#[test]
fn shell_batch_start_summary_hint_distinguishes_probe_build_and_test_commands() {
    let probe = AgentPipeline::public_shell_batch_start_summary_hint(
        "npx create-tauri-app --help",
        " for \"Initialize Tauri project\"",
    );
    let build = AgentPipeline::public_shell_batch_start_summary_hint(
        "cargo tauri build --debug",
        " for \"Build Tauri application\"",
    );
    let test = AgentPipeline::public_shell_batch_start_summary_hint(
        "cargo test --quiet",
        " for \"Run verification\"",
    );

    assert!(probe.contains("checking the command surface"));
    assert!(probe.contains("waiting"));
    assert!(build.contains("running a build/check command"));
    assert!(build.contains("waiting"));
    assert!(test.contains("running a test command"));
    assert!(test.contains("waiting"));
}

#[test]
fn shell_batch_start_next_step_hint_waits_for_command_completion() {
    let generic = AgentPipeline::public_shell_batch_start_next_step_hint("cargo fmt --check");
    let test = AgentPipeline::public_shell_batch_start_next_step_hint("cargo test --quiet");

    assert!(generic.starts_with("Once this command finishes"));
    assert!(test.starts_with("Once this test command finishes"));
}

#[test]
fn results_review_with_real_tool_results_keeps_llm_narration_available() {
    let snapshot = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root-task".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "research-task".to_string(),
            name: "Review research findings".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: vec![crate::streaming::TaskRuntimeTaskView {
            id: "research-task".to_string(),
            name: "Review research findings".to_string(),
            status: "in_progress".to_string(),
        }],
        completed_tasks: Vec::new(),
        missing_requirements: vec!["verification still required".to_string()],
        status_message: "Research review is still active".to_string(),
    };
    let recent_tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "web_search".to_string(),
        arguments: serde_json::json!({
            "query": "smart lighting market 2025 consumer drivers"
        })
        .to_string(),
        result: ToolResult::Success("Found relevant results".to_string()),
        duration_ms: 42,
    }];

    assert!(
        !AgentPipeline::should_force_runtime_snapshot_public_narration(
            PublicNarrationTrigger::ResultsReview,
            Some(&snapshot),
            &recent_tool_calls,
        )
    );
}

#[test]
fn redundant_results_review_narration_is_skipped_when_runtime_state_is_unchanged() {
    let snapshot = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root-task".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "research-task".to_string(),
            name: "Review research findings".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: vec![crate::streaming::TaskRuntimeTaskView {
            id: "research-task".to_string(),
            name: "Review research findings".to_string(),
            status: "in_progress".to_string(),
        }],
        completed_tasks: Vec::new(),
        missing_requirements: vec!["verification still required".to_string()],
        status_message: "Research review is still active".to_string(),
    };
    let recent_tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "read_file".to_string(),
        arguments: serde_json::json!({"path": "notes.md"}).to_string(),
        result: ToolResult::Success("latest notes".to_string()),
        duration_ms: 1,
    }];

    assert!(
        AgentPipeline::should_skip_redundant_results_review_narration(
            Some(&snapshot),
            Some(&snapshot),
            &recent_tool_calls,
        )
    );
}

#[test]
fn incomplete_tracked_work_adds_terminal_correction_for_completion_claims() {
    let state = TrackedTaskRuntimeState {
        snapshot: crate::streaming::TaskRuntimeSnapshot {
            root_task_id: "root-task".to_string(),
            current_task: Some(crate::streaming::TaskRuntimeTaskView {
                id: "verify-task".to_string(),
                name: "Verify facts and cross-check".to_string(),
                status: "not_started".to_string(),
            }),
            ready_tasks: vec![crate::streaming::TaskRuntimeTaskView {
                id: "verify-task".to_string(),
                name: "Verify facts and cross-check".to_string(),
                status: "not_started".to_string(),
            }],
            parallel_ready_tasks: Vec::new(),
            blocked_tasks: Vec::new(),
            open_tasks: vec![crate::streaming::TaskRuntimeTaskView {
                id: "verify-task".to_string(),
                name: "Verify facts and cross-check".to_string(),
                status: "not_started".to_string(),
            }],
            completed_tasks: Vec::new(),
            missing_requirements: vec!["verification still required".to_string()],
            status_message: "Verification remains open".to_string(),
        },
        open_descendant_summary: OpenDescendantSummary {
            not_started: 1,
            ..OpenDescendantSummary::default()
        },
        completion_ready: false,
    };

    let correction = AgentPipeline::tracked_task_incomplete_terminal_correction(
        "All planned subtasks are now finished and verified.",
        &state,
    )
    .expect("correction should be generated");

    assert!(correction.contains("I’m not calling this work complete yet"));
    assert!(correction.contains("Verify facts and cross-check"));
    assert!(correction.contains("I still need direct proof for"));
    assert!(correction.contains("There is still queued tracked work"));
}

#[test]
fn incomplete_tracked_work_adds_terminal_correction_for_generic_status_updates() {
    let state = TrackedTaskRuntimeState {
        snapshot: crate::streaming::TaskRuntimeSnapshot {
            root_task_id: "root-task".to_string(),
            current_task: Some(crate::streaming::TaskRuntimeTaskView {
                id: "draft-task".to_string(),
                name: "Draft final answer".to_string(),
                status: "in_progress".to_string(),
            }),
            ready_tasks: vec![crate::streaming::TaskRuntimeTaskView {
                id: "verify-task".to_string(),
                name: "Verify facts and cross-check".to_string(),
                status: "not_started".to_string(),
            }],
            parallel_ready_tasks: Vec::new(),
            blocked_tasks: Vec::new(),
            open_tasks: vec![crate::streaming::TaskRuntimeTaskView {
                id: "verify-task".to_string(),
                name: "Verify facts and cross-check".to_string(),
                status: "not_started".to_string(),
            }],
            completed_tasks: vec![crate::streaming::TaskRuntimeTaskView {
                id: "research-task".to_string(),
                name: "Research the topic".to_string(),
                status: "completed".to_string(),
            }],
            missing_requirements: Vec::new(),
            status_message: "Verification remains open".to_string(),
        },
        open_descendant_summary: OpenDescendantSummary {
            not_started: 1,
            ..OpenDescendantSummary::default()
        },
        completion_ready: false,
    };

    let correction = AgentPipeline::tracked_task_incomplete_terminal_correction(
        "Researched the topic, drafted the summary, and reviewed the generated markdown.",
        &state,
    )
    .expect("correction should be generated for a generic terminal status update");

    assert!(correction.contains("I’m not calling this work complete yet"));
    assert!(correction.contains("Verify facts and cross-check"));
    assert!(correction.contains("The next ready step is"));
}

#[test]
fn runtime_snapshot_narration_surfaces_new_blockers_and_requirements() {
    let previous = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Implement the fix".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        missing_requirements: Vec::new(),
        status_message: "Implementation is active".to_string(),
    };
    let current = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Implement the fix".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: vec![crate::streaming::TaskRuntimeTaskView {
            id: "task-2".to_string(),
            name: "Run the validation command".to_string(),
            status: "blocked".to_string(),
        }],
        open_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        missing_requirements: vec!["validation command not yet observed".to_string()],
        status_message: "Implementation is blocked on validation".to_string(),
    };

    let (stage, message, _) = AgentPipeline::runtime_snapshot_narration(&current, Some(&previous));

    assert_eq!(stage, crate::streaming::NarrationStage::Blocked);
    assert!(message.contains(
            "The latest result raised 1 more check, so I still need more proof before I can close this out."
        ));
    assert!(message.contains("Blocked work now includes \"Run the validation command\"."));
    assert!(
        message.contains("\"Implement the fix\" still needs direct proof before I can close it.")
    );
}

#[test]
fn runtime_snapshot_narration_skips_unchanged_queue_line() {
    let ready_task = crate::streaming::TaskRuntimeTaskView {
        id: "task-2".to_string(),
        name: "Summarize the validation results".to_string(),
        status: "not_started".to_string(),
    };
    let previous = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Implement the fix".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: vec![ready_task.clone()],
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        missing_requirements: vec!["validation command not yet observed".to_string()],
        status_message: "Implementation is active".to_string(),
    };
    let current = crate::streaming::TaskRuntimeSnapshot {
        status_message: "Implementation is still active".to_string(),
        ..previous.clone()
    };

    let (_, message, _) = AgentPipeline::runtime_snapshot_narration(&current, Some(&previous));

    assert!(message.contains(
            "\"Implement the fix\" is not done yet; I still need the required proof before I can close it."
        ));
    assert!(!message.contains("Next up:"));
}

#[test]
fn parse_public_narration_payload_keeps_structured_sections_and_evidence() {
    let payload = AgentPipeline::parse_public_narration_payload(
            r#"{
                "title": "Verification is active",
                "message": "I moved the tracked work into verification after the latest command succeeded.",
                "summary": "The latest results moved the active task into verification.",
                "reason": "That matters because the task still needs direct proof before it can close cleanly.",
                "next_step": "I’ll run the targeted test command next and use that result to decide whether this task is done.",
                "evidence": [
                    "Current step: \"Run targeted verification\".",
                    "Still need to verify: targeted test evidence."
                ]
            }"#,
            crate::streaming::NarrationStage::Verification,
            Some("shell"),
            &PublicNarrationContextFrame {
                stage: crate::streaming::NarrationStage::Verification,
                change_kind: PublicNarrationChangeKind::Confirmation,
                summary_hint: None,
                reason_hint: None,
                next_step_hint: None,
                evidence: Vec::new(),
                tracked_work_incomplete: true,
                completion_ready: false,
            },
        )
        .expect("structured narration payload should parse");

    assert_eq!(payload.title, "Verification is active");
    assert_eq!(
        payload.summary.as_deref(),
        Some("The latest results moved the active task into verification.")
    );
    assert!(
        payload
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("direct proof"))
    );
    assert!(
        payload
            .next_step
            .as_deref()
            .is_some_and(|next_step| next_step.contains("targeted test command next"))
    );
    assert_eq!(payload.evidence.len(), 2);
}

#[test]
fn parse_public_narration_payload_uses_context_hints_when_sections_are_missing() {
    let payload = AgentPipeline::parse_public_narration_payload(
            r#"{
                "title": "Working through verification",
                "message": "I’m reviewing the latest command result before I close this task out."
            }"#,
            crate::streaming::NarrationStage::Verification,
            Some("shell"),
            &PublicNarrationContextFrame {
                stage: crate::streaming::NarrationStage::Verification,
                change_kind: PublicNarrationChangeKind::Confirmation,
                summary_hint: Some(
                    "The latest result kept the work in verification while I confirm the last check."
                        .to_string(),
                ),
                reason_hint: Some(
                    "The task still needs one more piece of proof before it can close."
                        .to_string(),
                ),
                next_step_hint: Some(
                    "I’ll run the targeted validation check next and use that result to decide whether the task is done."
                        .to_string(),
                ),
                evidence: vec![
                    "Current step: \"Run targeted verification\".".to_string(),
                    "Still need to verify: targeted test evidence.".to_string(),
                ],
                tracked_work_incomplete: true,
                completion_ready: false,
            },
        )
        .expect("fallback narration payload should be synthesized");

    assert!(
        payload
            .next_step
            .as_deref()
            .is_some_and(|next_step| next_step.contains("targeted validation check next"))
    );
    assert_eq!(
        payload.summary.as_deref(),
        Some("The latest result kept the work in verification while I confirm the last check.")
    );
    assert_eq!(payload.evidence.len(), 2);
}

#[test]
fn tool_narration_suppresses_bookkeeping_only_task_updates() {
    let snapshot = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Inspect the current state and constraints".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        missing_requirements: vec!["test command not yet observed".to_string()],
        status_message: "Inspect task is active".to_string(),
    };

    assert!(AgentPipeline::tool_narration("task", None, Some(&snapshot)).is_none());
    assert!(AgentPipeline::tool_narration("tasks", None, Some(&snapshot)).is_none());
}

#[test]
fn tool_narration_uses_tool_arguments_for_more_specific_context() {
    let (_, message, _) = AgentPipeline::tool_narration(
        "web_search",
        Some(r#"{"query":"smart lighting market 2025 consumer drivers"}"#),
        None,
    )
    .expect("web_search narration should be available");

    assert!(message.contains("about \"smart lighting market 2025 consumer drivers\""));
}

#[test]
fn shell_tool_narration_stays_anchored_to_the_running_command() {
    let (_, message, _) =
        AgentPipeline::tool_narration("shell", Some(r#"{"command":"cargo test --quiet"}"#), None)
            .expect("shell narration should be available");

    assert!(message.contains("waiting on its result before I choose the next move"));
    assert!(message.contains("cargo test --quiet"));
}

#[test]
fn finalize_public_narration_prefers_goal_driven_title_over_active_task_name() {
    let narration = AgentPipeline::finalize_public_narration(
            crate::streaming::NarrationStage::Context,
            Some("web_search"),
            PublicNarrationDraft {
                message: Some(
                    "I’m comparing the pricing notes against the forecast before I rewrite the market summary."
                        .to_string(),
                ),
                ..PublicNarrationDraft::default()
            },
            &PublicNarrationContextFrame {
                stage: crate::streaming::NarrationStage::Context,
                change_kind: PublicNarrationChangeKind::Discovery,
                summary_hint: None,
                reason_hint: None,
                next_step_hint: None,
                evidence: vec!["Current step: \"Gather the relevant market evidence\".".to_string()],
                tracked_work_incomplete: true,
                completion_ready: false,
            },
        )
        .expect("narration should be finalized");

    assert_eq!(
        narration.title,
        "Comparing the pricing notes against the forecast"
    );
}

#[test]
fn finalize_public_narration_derives_specific_execution_title_from_message() {
    let narration = AgentPipeline::finalize_public_narration(
        crate::streaming::NarrationStage::Execution,
        None,
        PublicNarrationDraft {
            message: Some(
                "I’m updating the task tracking flow before I rerun the task hierarchy checks."
                    .to_string(),
            ),
            ..PublicNarrationDraft::default()
        },
        &PublicNarrationContextFrame {
            stage: crate::streaming::NarrationStage::Execution,
            change_kind: PublicNarrationChangeKind::Continuation,
            summary_hint: None,
            reason_hint: None,
            next_step_hint: None,
            evidence: Vec::new(),
            tracked_work_incomplete: true,
            completion_ready: false,
        },
    )
    .expect("narration should be finalized");

    assert_eq!(narration.title, "Updating the task tracking flow");
    assert_ne!(narration.title, "Working on request");
    assert_ne!(narration.title, "Advancing current step");
}

#[test]
fn title_candidate_from_narration_text_rejects_queue_style_next_step_labels() {
    assert!(
        AgentPipeline::title_candidate_from_narration_text(
            "Next up: \"Implement SWOT in Markdown\" and \"Verify and Cross-Check Facts\"."
        )
        .is_none()
    );
}

#[test]
fn finalize_public_narration_prefers_authored_heading_over_next_step_task_label() {
    let narration = AgentPipeline::finalize_public_narration(
            crate::streaming::NarrationStage::Execution,
            None,
            PublicNarrationDraft {
                message: Some(
                    "**SWOT Analysis Complete** I've finished the current draft in `swot_smart_home_lighting.md` and I'm lining up the remaining cross-check."
                        .to_string(),
                ),
                ..PublicNarrationDraft::default()
            },
            &PublicNarrationContextFrame {
                stage: crate::streaming::NarrationStage::Execution,
                change_kind: PublicNarrationChangeKind::Decision,
                summary_hint: Some("I’m focused on \"Implement SWOT in Markdown\" right now.".to_string()),
                reason_hint: Some(
                    "The tracked plan actually changed, so the user should understand why the focus is moving now."
                        .to_string(),
                ),
                next_step_hint: Some(
                    "Next up: \"Implement SWOT in Markdown\" and \"Verify and Cross-Check Facts\"."
                        .to_string(),
                ),
                evidence: vec![
                    "I’m focused on \"Implement SWOT in Markdown\" right now.".to_string(),
                    "Newly finished work: \"Plan SWOT Structure\" and \"Research 2025-2026 Market Trends\"."
                        .to_string(),
                ],
                tracked_work_incomplete: true,
                completion_ready: false,
            },
        )
        .expect("narration should be finalized");

    assert_eq!(narration.title, "SWOT Analysis Complete");
    assert_ne!(narration.title, "Next up Implement SWOT in Markdown");
}

#[test]
fn sanitize_public_narration_text_removes_wrappers_and_think_blocks() {
    let sanitized = AgentPipeline::sanitize_public_narration_text(
            "<think>hidden reasoning</think>Public narration: I found an existing Cargo workspace, so I’m checking whether Tauri is already configured before I scaffold anything.",
        )
        .expect("narration should sanitize");

    assert!(!sanitized.contains("hidden reasoning"));
    assert!(!sanitized.starts_with("Public narration:"));
    assert!(sanitized.contains("Cargo workspace"));
}

#[test]
fn sanitize_public_narration_text_preserves_markdown_line_structure() {
    let sanitized = AgentPipeline::sanitize_public_narration_text(
            "Public narration: # Verification update\n\n- reviewed failing tests\n- queued a focused rerun\n\n## Next step\nRun the targeted shell command.",
        )
        .expect("markdown narration should sanitize");

    assert_eq!(
        sanitized,
        "# Verification update\n\n- reviewed failing tests\n- queued a focused rerun\n\n## Next step\nRun the targeted shell command."
    );
}

#[test]
fn sanitize_public_narration_text_rejects_raw_json_payload_fragments() {
    assert!(AgentPipeline::sanitize_public_narration_text(
            r#"I’m reading through the research returned for "{ "query": "consumer drivers for smart home lighting products 2025", "results": [{"title": "Study A"}]}" and filtering down to the findings that actually matter for this request."#,
        )
        .is_none());
}

#[test]
fn sanitize_public_narration_title_accepts_short_heading() {
    let title = AgentPipeline::sanitize_public_narration_title("Title: Checking current files")
        .expect("title should sanitize");

    assert_eq!(title, "Checking current files");
}

#[test]
fn sanitize_public_narration_title_accepts_seven_word_heading() {
    let title = AgentPipeline::sanitize_public_narration_title(
        "Title: Reviewing the current implementation state for regressions",
    )
    .expect("seven-word title should sanitize");

    assert_eq!(
        title,
        "Reviewing the current implementation state for regressions"
    );
}

#[test]
fn sanitize_public_narration_title_rejects_truncated_heading() {
    let title =
        AgentPipeline::sanitize_public_narration_title("Title: Researching smart lighting market…");

    assert!(title.is_none());
}

#[test]
fn sanitize_public_narration_title_rejects_more_than_seven_words() {
    let title = AgentPipeline::sanitize_public_narration_title(
        "Title: Reviewing the current implementation state for regressions carefully today",
    );

    assert!(title.is_none());
}

#[test]
fn contextual_public_narration_title_compacts_search_queries_without_ellipsis() {
    let title = AgentPipeline::title_candidate_from_evidence(
        "Observed search query: `smart lighting market 2025 consumer drivers and pricing`.",
    )
    .expect("search query title should compact");

    assert_eq!(
        title,
        "Researching smart lighting market 2025 consumer drivers"
    );
    assert!(!title.ends_with('…'));
}

#[test]
fn build_public_narration_prompt_includes_planning_stage_ordering() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_public_narration_prompt(
        PublicNarrationTrigger::ResultsReview,
        None,
        None,
        &[],
        None,
        &PublicNarrationContextFrame {
            stage: crate::streaming::NarrationStage::Planning,
            change_kind: PublicNarrationChangeKind::Decision,
            summary_hint: Some(
                "I’m breaking the request into tracked subtasks before I start execution."
                    .to_string(),
            ),
            reason_hint: None,
            next_step_hint: None,
            evidence: Vec::new(),
            tracked_work_incomplete: true,
            completion_ready: false,
        },
    );

    assert!(prompt.contains("title: 2 to 7 words"));
    assert!(prompt.contains("Write the message first, then derive the shorter fields from it"));
    assert!(prompt.contains("Narration change type: decision."));
    assert!(prompt.contains(
            "make the message cover these beats in this order: first say that I’m breaking the request into subtasks"
        ));
    assert!(prompt.contains("then explain why the first subtask was chosen"));
    assert!(prompt.contains("then explain what work remains queued behind it"));
    assert!(prompt.contains("then explain what the next verification step will prove"));
}

#[test]
fn build_public_narration_prompt_summarizes_structured_tool_results_without_raw_json() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let tool_call = ToolCallRecord {
        id: "tool-1".to_string(),
        name: "web_search".to_string(),
        arguments: serde_json::json!({
            "query": "consumer drivers for smart home lighting products 2025"
        })
        .to_string(),
        result: ToolResult::Success(
            serde_json::json!({
                "query": "consumer drivers for smart home lighting products 2025",
                "results": [
                    { "title": "Study A" },
                    { "title": "Study B" }
                ]
            })
            .to_string(),
        ),
        duration_ms: 12,
    };
    let tool_arguments = tool_call.arguments.clone();

    let prompt = pipeline.build_public_narration_prompt(
        PublicNarrationTrigger::ResultsReview,
        Some("web_search"),
        Some(tool_arguments.as_str()),
        &[tool_call],
        None,
        &PublicNarrationContextFrame {
            stage: crate::streaming::NarrationStage::Context,
            change_kind: PublicNarrationChangeKind::Discovery,
            summary_hint: None,
            reason_hint: None,
            next_step_hint: None,
            evidence: Vec::new(),
            tracked_work_incomplete: true,
            completion_ready: false,
        },
    );

    assert!(prompt.contains("Never quote raw JSON"));
    assert!(prompt.contains(
        "Result summary: Observed structured search results for the requested query (2 items)."
    ));
    assert!(!prompt.contains("\"query\":"));
    assert!(!prompt.contains("\"results\":"));
}

#[test]
fn results_review_narration_context_uses_structured_result_summary_in_evidence() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let tool_call = ToolCallRecord {
        id: "tool-1".to_string(),
        name: "web_search".to_string(),
        arguments: serde_json::json!({
            "query": "consumer drivers for smart home lighting products 2025"
        })
        .to_string(),
        result: ToolResult::Success(
            serde_json::json!({
                "query": "consumer drivers for smart home lighting products 2025",
                "results": [
                    { "title": "Study A" },
                    { "title": "Study B" }
                ]
            })
            .to_string(),
        ),
        duration_ms: 12,
    };

    let context = pipeline.build_results_review_narration_context_frame(
        crate::streaming::NarrationStage::Context,
        None,
        None,
        &[tool_call],
    );

    assert!(context.evidence.iter().any(|entry| {
        entry.contains("Observed structured search results for the requested query (2 items).")
    }));
    assert!(
        context
            .evidence
            .iter()
            .all(|entry| !entry.contains("\"query\":") && !entry.contains("\"results\":"))
    );
}

#[test]
fn parse_public_narration_payload_reads_json_title_and_message() {
    let payload = AgentPipeline::parse_public_narration_payload(
            r#"{"title":"Reviewing current files","message":"I’m checking the current files before I make the next change."}"#,
            crate::streaming::NarrationStage::Execution,
            Some("file"),
            &PublicNarrationContextFrame {
                stage: crate::streaming::NarrationStage::Execution,
                change_kind: PublicNarrationChangeKind::Discovery,
                summary_hint: None,
                reason_hint: None,
                next_step_hint: None,
                evidence: Vec::new(),
                tracked_work_incomplete: true,
                completion_ready: false,
            },
        )
        .expect("payload should parse");

    assert_eq!(payload.title, "Reviewing current files");
    assert_eq!(
        payload.message,
        "I’m checking the current files before I make the next change."
    );
}

#[test]
fn parse_public_narration_payload_prefers_authored_message_over_composed_sections() {
    let payload = AgentPipeline::parse_public_narration_payload(
            r#"{
                "title":"Following the thread",
                "message":"I found the first concrete branch to inspect, so I’m checking that path before I touch the queued work behind it and I’ll use the result to decide whether the verification step needs to move earlier.",
                "summary":"I’m checking the first branch now.",
                "reason":"That matters because it unlocks the queued work.",
                "next_step":"I’ll verify the branch result next."
            }"#,
            crate::streaming::NarrationStage::Planning,
            Some("file"),
            &PublicNarrationContextFrame {
                stage: crate::streaming::NarrationStage::Planning,
                change_kind: PublicNarrationChangeKind::Discovery,
                summary_hint: None,
                reason_hint: None,
                next_step_hint: None,
                evidence: Vec::new(),
                tracked_work_incomplete: true,
                completion_ready: false,
            },
        )
        .expect("payload should parse");

    assert_eq!(payload.title, "Following the thread");
    assert_eq!(
        payload.message,
        "I found the first concrete branch to inspect, so I’m checking that path before I touch the queued work behind it and I’ll use the result to decide whether the verification step needs to move earlier."
    );
    assert_eq!(
        payload.summary.as_deref(),
        Some("I’m checking the first branch now.")
    );
}

#[test]
fn sanitize_public_narration_text_preserves_detail_without_hard_cap() {
    let message = "I’m tracing the request through the first implementation branch, checking the exact proof that pushed me there, and keeping the queued verification work in view so I can explain the next decision without flattening everything into the same summary sentence for the user while this loop is still moving. I also want to keep the latest confirmed context attached to the exact branch I’m in now, because the result from this step decides whether I keep executing in code, move into a verification pass, or pause to resolve a blocker that only became visible once the latest evidence landed in the session.";

    let sanitized =
        AgentPipeline::sanitize_public_narration_text(message).expect("narration should sanitize");

    assert_eq!(sanitized, message);
    assert!(message.chars().count() > 420);
}

#[test]
fn build_public_narration_prompt_does_not_force_short_message_length() {
    let pipeline = AgentPipeline::new(AppConfig::default());
    let prompt = pipeline.build_public_narration_prompt(
        PublicNarrationTrigger::ResultsReview,
        None,
        None,
        &[],
        None,
        &PublicNarrationContextFrame {
            stage: crate::streaming::NarrationStage::Progress,
            change_kind: PublicNarrationChangeKind::Continuation,
            summary_hint: None,
            reason_hint: None,
            next_step_hint: None,
            evidence: Vec::new(),
            tracked_work_incomplete: true,
            completion_ready: false,
        },
    );

    assert!(prompt.contains("Use however much detail and however many sentences are needed"));
    assert!(
        prompt
            .contains("Prefer a message shape of what changed -> what it means -> what I do next")
    );
    assert!(!prompt.contains("Write 2 to 4 natural first-person sentences"));
}

#[test]
fn sanitize_public_narration_text_rejects_generic_processing_filler() {
    assert!(
        AgentPipeline::sanitize_public_narration_text(
            "Reading through file contents to extract the needed information…"
        )
        .is_none()
    );
    assert!(
        AgentPipeline::sanitize_public_narration_text(
            "Processing command output to extract results and plan next steps…"
        )
        .is_none()
    );
}

#[test]
fn tool_narration_skips_task_bookkeeping_aliases() {
    assert!(AgentPipeline::tool_narration("task_update_status", None, None).is_none());
    assert!(AgentPipeline::tool_narration("task_create", None, None).is_none());
}

#[test]
fn batch_start_narration_fingerprint_changes_when_tool_arguments_change() {
    let snapshot = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Inspect the current state and constraints".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        missing_requirements: vec!["test command not yet observed".to_string()],
        status_message: "Inspect task is active".to_string(),
    };

    let first = AgentPipeline::public_narration_fingerprint(
        PublicNarrationTrigger::BatchStart,
        Some("file"),
        Some("{\"path\":\"src/main.rs\"}"),
        Some(&snapshot),
        &[],
    );
    let second = AgentPipeline::public_narration_fingerprint(
        PublicNarrationTrigger::BatchStart,
        Some("file"),
        Some("{\"path\":\"src/lib.rs\"}"),
        Some(&snapshot),
        &[],
    );

    assert_ne!(first, second);
}

#[test]
fn batch_start_research_fingerprint_stays_stable_within_same_task_phase() {
    let snapshot = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Research the market landscape".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: vec![crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Research the market landscape".to_string(),
            status: "in_progress".to_string(),
        }],
        completed_tasks: Vec::new(),
        missing_requirements: Vec::new(),
        status_message: "Research is active".to_string(),
    };

    let first = AgentPipeline::public_narration_fingerprint(
        PublicNarrationTrigger::BatchStart,
        Some("web_search"),
        Some("{\"query\":\"smart lighting market size 2025\"}"),
        Some(&snapshot),
        &[],
    );
    let second = AgentPipeline::public_narration_fingerprint(
        PublicNarrationTrigger::BatchStart,
        Some("web_search"),
        Some("{\"query\":\"smart lighting competitors hue nanoleaf govee\"}"),
        Some(&snapshot),
        &[],
    );

    assert_eq!(first, second);
}

#[test]
fn tool_narration_fingerprint_changes_when_tool_arguments_change() {
    let snapshot = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Inspect the current state and constraints".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        missing_requirements: vec!["test command not yet observed".to_string()],
        status_message: "Inspect task is active".to_string(),
    };

    let first = AgentPipeline::tool_narration_fingerprint(
        "read_file",
        Some(r#"{"path":"docs/market-2025.md"}"#),
        crate::streaming::NarrationStage::Context,
        Some(&snapshot),
    );
    let second = AgentPipeline::tool_narration_fingerprint(
        "read_file",
        Some(r#"{"path":"docs/market-2026.md"}"#),
        crate::streaming::NarrationStage::Context,
        Some(&snapshot),
    );

    assert_ne!(first, second);
}

#[test]
fn review_narration_fingerprint_changes_when_recent_tool_results_change() {
    let snapshot = crate::streaming::TaskRuntimeSnapshot {
        root_task_id: "root".to_string(),
        current_task: Some(crate::streaming::TaskRuntimeTaskView {
            id: "task-1".to_string(),
            name: "Inspect the current state and constraints".to_string(),
            status: "in_progress".to_string(),
        }),
        ready_tasks: Vec::new(),
        parallel_ready_tasks: Vec::new(),
        blocked_tasks: Vec::new(),
        open_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        missing_requirements: vec!["test command not yet observed".to_string()],
        status_message: "Inspect task is active".to_string(),
    };

    let successful_read = ToolCallRecord {
        id: "1".to_string(),
        name: "file".to_string(),
        arguments: "{\"path\":\"Cargo.toml\"}".to_string(),
        result: ToolResult::Success("workspace members found".to_string()),
        duration_ms: 12,
    };
    let failing_shell = ToolCallRecord {
        id: "2".to_string(),
        name: "shell".to_string(),
        arguments: "cargo test".to_string(),
        result: ToolResult::Error("command failed".to_string()),
        duration_ms: 87,
    };

    let first = AgentPipeline::review_narration_fingerprint(
        Some(&snapshot),
        std::slice::from_ref(&successful_read),
    );
    let second = AgentPipeline::review_narration_fingerprint(
        Some(&snapshot),
        std::slice::from_ref(&failing_shell),
    );

    assert_ne!(first, second);
}

#[test]
fn stagnation_recovery_instruction_demands_materially_different_next_step() {
    let prompt = AgentPipeline::with_stagnation_recovery_instruction(
        "Base prompt",
        3,
        "repeated outcomes: shell:error:missing config",
        &["test command not yet observed".to_string()],
    );

    assert!(prompt.contains("materially different action"));
    assert!(prompt.contains("run appears stalled"));
    assert!(prompt.contains("test command not yet observed"));
}

#[test]
fn build_and_test_completion_status_counts_failed_commands_as_attempts() {
    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Error("compiler error".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo test"}).to_string(),
            result: ToolResult::Error("test failures".to_string()),
            duration_ms: 1,
        },
    ];

    assert_eq!(
        AgentPipeline::build_and_test_completion_status(&tool_calls),
        (true, false, true, false)
    );
}

#[test]
fn build_and_test_completion_status_rejects_masked_successful_test_output() {
    let tool_calls = vec![ToolCallRecord {
        id: "1".to_string(),
        name: "shell".to_string(),
        arguments: serde_json::json!({
            "command": "playwright test || true; npm run lint; npm run validate:all"
        })
        .to_string(),
        result: ToolResult::Success("8 failed\n2 passed\nlint passed\nvalidate passed".to_string()),
        duration_ms: 1,
    }];

    assert_eq!(
        AgentPipeline::build_and_test_completion_status(&tool_calls),
        (false, false, true, false)
    );
}

#[test]
fn failed_verification_attempt_moves_verification_task_in_progress() {
    let manager = crate::get_global_task_manager();
    let session_id = format!(
        "agent-loop-failed-verification-attempt-{}",
        uuid::Uuid::new_v4()
    );
    let mut root = crate::Task::new(&session_id, "Root", "Root", None);
    root.set_status(crate::TaskStatus::InProgress);
    let verify = crate::Task::new(
        &session_id,
        "Run verification checks",
        "Build and test the changed code",
        Some(root.id.clone()),
    );

    let mut task_list = crate::TaskList::new(&session_id);
    task_list.add_task(root.clone());
    task_list.add_task(verify.clone());
    manager
        .replace_task_list(task_list)
        .expect("replace task list");
    manager
        .set_current_task_id(&session_id, Some(verify.id.clone()))
        .expect("set current task");

    let tool_calls = vec![
        ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo check"}).to_string(),
            result: ToolResult::Error("compiler error".to_string()),
            duration_ms: 1,
        },
        ToolCallRecord {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo test"}).to_string(),
            result: ToolResult::Error("test failures".to_string()),
            duration_ms: 1,
        },
    ];

    let runtime_state = AgentPipeline::reconcile_tracked_execution_progress_from_tool_activity(
        true,
        false,
        Some(&session_id),
        Some(&root.id),
        &tool_calls,
    )
    .expect("runtime state should be available");

    let updated_verify = manager
        .get_task(&session_id, &verify.id)
        .expect("verification task lookup should succeed")
        .expect("verification task should exist");
    let execution_state = manager
        .get_execution_state(&session_id, &verify.id)
        .expect("execution state lookup should succeed")
        .expect("execution state should exist");

    assert_eq!(updated_verify.status, crate::TaskStatus::InProgress);
    assert!(execution_state.saw_tool_activity);
    assert!(!execution_state.build_succeeded);
    assert!(!execution_state.test_succeeded);
    assert!(
        runtime_state
            .snapshot
            .missing_requirements
            .iter()
            .any(|message| message == "build/check command not yet observed")
    );
    assert!(
        runtime_state
            .snapshot
            .missing_requirements
            .iter()
            .any(|message| message == "test command not yet observed")
    );
}
