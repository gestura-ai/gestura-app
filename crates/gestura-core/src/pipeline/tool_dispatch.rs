use super::*;

/// Pending tool call being accumulated during streaming
pub(super) struct PendingToolCall {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) arguments: String,
    pub(super) start_time: Instant,
}

/// Context used by `AgentPipeline::finalize_pending_tool_call`.
///
/// This bundles together the mutable per-iteration state and runtime references required to
/// complete a pending tool call (permission checks, execution, streaming events, and recording).
pub(super) struct FinalizePendingToolCallCtx<'a> {
    pub(super) workspace: Option<&'a SessionWorkspace>,
    pub(super) session_id: Option<String>,
    pub(super) permission_level: PermissionLevel,
    pub(super) cancel_token: &'a CancellationToken,
    pub(super) tool_calls_in_iteration: &'a mut Vec<ToolCallRecord>,
    pub(super) response: &'a mut AgentResponse,
    pub(super) tx: &'a mpsc::Sender<StreamChunk>,
}

impl AgentPipeline {
    /// Execute a tool by name with given arguments.
    ///
    /// Note: If a workspace is provided in `ctx`, all file paths and shell commands are sandboxed
    /// to that directory. Paths outside the workspace will be rejected.
    pub(super) async fn finalize_pending_tool_call(
        &self,
        pending: PendingToolCall,
        ctx: FinalizePendingToolCallCtx<'_>,
    ) {
        let FinalizePendingToolCallCtx {
            workspace,
            session_id,
            permission_level,
            cancel_token,
            tool_calls_in_iteration,
            response,
            tx,
        } = ctx;
        tracing::debug!(
            tool = %pending.name,
            args_len = pending.arguments.len(),
            permission_level = ?permission_level,
            "[ToolDispatch] finalize_pending_tool_call entry"
        );

        let policy = crate::tools::policy::evaluate_tool_call(
            permission_level,
            &pending.name,
            &pending.arguments,
        );

        let policy_label = match &policy.decision {
            crate::tools::policy::ToolCallDecision::Allowed => "Allowed",
            crate::tools::policy::ToolCallDecision::Blocked { .. } => "Blocked",
            crate::tools::policy::ToolCallDecision::RequiresConfirmation(_) => {
                "RequiresConfirmation"
            }
        };
        tracing::debug!(
            tool = %pending.name,
            decision = policy_label,
            is_write = policy.is_write_operation,
            "[ToolDispatch] Policy decision"
        );

        if let crate::tools::policy::ToolCallDecision::Blocked { reason } = &policy.decision {
            let _ = tx
                .send(StreamChunk::ToolBlocked {
                    tool_name: pending.name.clone(),
                    reason: reason.clone(),
                })
                .await;

            // Emit a tool result so the UI can finalize the tool card.
            let _ = tx
                .send(StreamChunk::ToolCallResult {
                    name: pending.name.clone(),
                    success: false,
                    output: reason.clone(),
                    duration_ms: 0,
                })
                .await;

            let record = ToolCallRecord {
                id: pending.id,
                name: pending.name,
                arguments: pending.arguments,
                result: ToolResult::Skipped(reason.clone()),
                duration_ms: 0,
            };
            tool_calls_in_iteration.push(record.clone());
            response.tool_calls.push(record);
            return;
        }

        if let crate::tools::policy::ToolCallDecision::RequiresConfirmation(info) = &policy.decision
        {
            // Tool requires confirmation (write operation in Restricted mode).
            // We pause tool execution until the UI approves/denies.

            // Session/persisted fast paths (Claude Code parity): if the user has already
            // allowed/blocked this tool for the session, or has a persisted allow rule,
            // skip the confirmation dialog.
            if let Some(session_id) = session_id.as_deref() {
                if TOOL_CONFIRMATIONS.is_tool_allowed_for_session(session_id, &pending.name) {
                    // Already allowed for this session: proceed to execution.
                } else if TOOL_CONFIRMATIONS.is_tool_blocked_for_session(session_id, &pending.name)
                {
                    let duration_ms = pending.start_time.elapsed().as_millis() as u64;
                    let msg = "Skipped: tool blocked for session".to_string();
                    let _ = tx
                        .send(StreamChunk::ToolCallResult {
                            name: pending.name.clone(),
                            success: false,
                            output: msg.clone(),
                            duration_ms,
                        })
                        .await;

                    let record = ToolCallRecord {
                        id: pending.id,
                        name: pending.name,
                        arguments: pending.arguments,
                        result: ToolResult::Skipped(msg),
                        duration_ms,
                    };
                    tool_calls_in_iteration.push(record.clone());
                    response.tool_calls.push(record);
                    return;
                }
            }

            // Persisted permission fast path: if the user previously chose "Allow always"
            // for this tool, proceed without prompting.
            match self
                .permission_manager
                .check(&pending.name, "execute", Some(&pending.arguments))
            {
                Ok(check) if check.allowed => {
                    // Allowed: proceed to execution.
                }
                Ok(_) => {
                    // No persisted allow rule: continue to confirmation.
                }
                Err(e) => {
                    tracing::warn!(error = %e, tool = %pending.name, "Permission check failed; falling back to confirmation");
                }
            }

            // If the tool is allowed for this session or via persisted permissions, the
            // early returns/branches above will have proceeded; otherwise continue to prompt.
            let needs_confirmation = match session_id.as_deref() {
                Some(sid) if TOOL_CONFIRMATIONS.is_tool_allowed_for_session(sid, &pending.name) => {
                    false
                }
                _ => match self.permission_manager.check(
                    &pending.name,
                    "execute",
                    Some(&pending.arguments),
                ) {
                    Ok(check) => !check.allowed,
                    Err(_) => true,
                },
            };

            if !needs_confirmation {
                // Approved: continue to normal execution flow below.
            } else {
                const CONFIRMATION_TIMEOUT_SECS: u64 = 300;
                let confirmation_id = format!("tool_confirm_{}", uuid::Uuid::new_v4());

                // Register pending confirmation before emitting the event, so the UI can
                // resolve it immediately without racing.
                let rx = TOOL_CONFIRMATIONS.register(
                    confirmation_id.clone(),
                    session_id.clone(),
                    pending.name.clone(),
                    pending.arguments.clone(),
                );

                let _ = tx
                    .send(StreamChunk::ToolConfirmationRequired {
                        confirmation_id: confirmation_id.clone(),
                        tool_name: pending.name.clone(),
                        tool_args: pending.arguments.clone(),
                        description: info.description.clone(),
                        risk_level: info.risk_level,
                        category: info.category.clone(),
                    })
                    .await;

                // Await UI decision with timeout and cancellation.
                let default_decision = crate::tool_confirmation::ToolConfirmationDecision::DenyOnce;
                let decision: crate::tool_confirmation::ToolConfirmationDecision = tokio::select! {
                    decision = rx => decision.unwrap_or(default_decision),
                    _ = tokio::time::sleep(std::time::Duration::from_secs(CONFIRMATION_TIMEOUT_SECS)) => {
                        TOOL_CONFIRMATIONS.abandon(&confirmation_id);
                        default_decision
                    }
                    _ = async {
                        while !cancel_token.is_cancelled() {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    } => {
                        TOOL_CONFIRMATIONS.abandon(&confirmation_id);
                        default_decision
                    }
                };

                // Apply scoped policy decisions.
                if let Some(session_id) = session_id.as_deref() {
                    TOOL_CONFIRMATIONS.apply_session_policy_decision(
                        session_id,
                        &pending.name,
                        decision,
                    );
                }
                if decision == crate::tool_confirmation::ToolConfirmationDecision::AllowAlways
                    && let Err(e) = self.permission_manager.grant(
                        &pending.name,
                        "execute",
                        crate::tools::permissions::PermissionScope::Global,
                        None,
                    )
                {
                    tracing::warn!(
                        error = %e,
                        tool = %pending.name,
                        "Failed to persist AllowAlways permission"
                    );
                }

                if !decision.is_allowed() {
                    let duration_ms = pending.start_time.elapsed().as_millis() as u64;
                    let msg = format!(
                        "Skipped: tool confirmation denied/timed-out (id: {})",
                        confirmation_id
                    );
                    let _ = tx
                        .send(StreamChunk::ToolCallResult {
                            name: pending.name.clone(),
                            success: false,
                            output: msg.clone(),
                            duration_ms,
                        })
                        .await;

                    let record = ToolCallRecord {
                        id: pending.id,
                        name: pending.name,
                        arguments: pending.arguments,
                        result: ToolResult::Skipped(msg),
                        duration_ms,
                    };
                    tool_calls_in_iteration.push(record.clone());
                    response.tool_calls.push(record);
                    return;
                }
                // Approved: continue to normal execution flow below.
            }
        }

        // After confirmation granted, before side effects:
        // 1. Create checkpoint for write operations (best-effort)
        if policy.is_write_operation
            && let Some(sid) = session_id.as_deref()
        {
            self.try_create_checkpoint_before_tool(sid, &pending.name);
        }

        // 2. Run PreTool hook (if enabled) - failures skip tool execution
        let hook_engine = self.create_hook_engine();
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: workspace.map(|w| w.root().to_path_buf()),
                session_id: session_id.clone(),
                tool_name: Some(pending.name.clone()),
                tool_arguments_json: Some(pending.arguments.clone()),
                ..Default::default()
            };
            if let Err(e) = engine.run(HookEvent::PreTool, &hook_ctx).await {
                tracing::warn!(
                    tool = %pending.name,
                    error = %e,
                    "PreTool hook failed; skipping tool execution"
                );
                let duration_ms = pending.start_time.elapsed().as_millis() as u64;
                let msg = format!("Skipped: PreTool hook failed: {}", e);
                let _ = tx
                    .send(StreamChunk::ToolCallResult {
                        name: pending.name.clone(),
                        success: false,
                        output: msg.clone(),
                        duration_ms,
                    })
                    .await;

                let record = ToolCallRecord {
                    id: pending.id,
                    name: pending.name,
                    arguments: pending.arguments,
                    result: ToolResult::Skipped(msg),
                    duration_ms,
                };
                tool_calls_in_iteration.push(record.clone());
                response.tool_calls.push(record);
                return;
            }
        }

        // Execute the tool with workspace sandboxing
        tracing::debug!(
            tool = %pending.name,
            workspace_root = ?workspace.map(|w| w.root().display().to_string()),
            "[ToolDispatch] Calling execute_tool"
        );
        let result = self
            .execute_tool(&pending.name, &pending.arguments, workspace, Some(tx))
            .await;
        let duration_ms = pending.start_time.elapsed().as_millis() as u64;
        tracing::debug!(
            tool = %pending.name,
            success = matches!(result, ToolResult::Success(_)),
            duration_ms = duration_ms,
            "[ToolDispatch] execute_tool completed"
        );

        // Emit structured tool result for frontend display
        let (success, output) = match &result {
            ToolResult::Success(out) => (true, out.trim_end().to_string()),
            ToolResult::Error(e) => (false, e.clone()),
            ToolResult::Skipped(msg) => (false, format!("Skipped: {}", msg)),
        };
        let _ = tx
            .send(StreamChunk::ToolCallResult {
                name: pending.name.clone(),
                success,
                output: output.clone(),
                duration_ms,
            })
            .await;

        // Run PostTool hook (best-effort)
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: workspace.map(|w| w.root().to_path_buf()),
                session_id: session_id.clone(),
                tool_name: Some(pending.name.clone()),
                tool_arguments_json: Some(pending.arguments.clone()),
                tool_success: Some(success),
                tool_output: Some(output),
                ..Default::default()
            };
            self.run_hook_best_effort(engine, HookEvent::PostTool, &hook_ctx)
                .await;
        }

        let record = ToolCallRecord {
            id: pending.id,
            name: pending.name,
            arguments: pending.arguments,
            result,
            duration_ms,
        };

        tool_calls_in_iteration.push(record.clone());
        response.tool_calls.push(record);
    }

    pub(super) async fn execute_tool(
        &self,
        name: &str,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
        stream_tx: Option<&mpsc::Sender<StreamChunk>>,
    ) -> ToolResult {
        let start = Instant::now();
        tracing::info!(
            tool = name,
            workspace = ?workspace.map(|w| w.root()),
            "Executing tool with args: {}",
            arguments
        );

        let result = match name {
            "shell" | "bash" | "execute" => {
                self.execute_shell_tool(arguments, workspace, stream_tx)
                    .await
            }
            "file" | "read_file" | "write_file" => {
                self.execute_file_tool(arguments, workspace).await
            }
            "git" => self.execute_git_tool(arguments, workspace).await,
            "web" | "web_search" => self.execute_web_tool(arguments).await,
            "code" => self.execute_code_tool(arguments, workspace).await,
            "task" | "tasks" => self.execute_task_tool(arguments, workspace).await,
            "screenshot" | "screen_record" => {
                self.execute_screen_tool(name, arguments, workspace).await
            }
            "gui_control" => self.execute_gui_tool(arguments).await,
            "mcp" => self.execute_mcp_manager_tool(arguments).await,
            _ if name.starts_with("mcp__") => self.execute_mcp_tool(name, arguments).await,
            _ => ToolResult::Skipped(format!("Unknown tool: {}", name)),
        };

        let duration = start.elapsed();
        tracing::info!("Tool {} completed in {:?}: {:?}", name, duration, result);

        result
    }

    /// Execute an MCP tool via the global `McpClientRegistry`.
    ///
    /// The tool `name` is expected to follow the `mcp__<server>__<tool>` naming
    /// convention established in `build_mcp_tool_schemas`.
    async fn execute_mcp_tool(&self, name: &str, arguments: &str) -> ToolResult {
        // Parse "mcp__<server>__<tool>" into server and tool names.
        let rest = match name.strip_prefix("mcp__") {
            Some(r) => r,
            None => return ToolResult::Error(format!("Invalid MCP tool name: {name}")),
        };
        let (server_name, tool_name) = match rest.split_once("__") {
            Some((s, t)) => (s, t),
            None => return ToolResult::Error(format!("Invalid MCP tool name format: {name}")),
        };

        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::Error(format!("Invalid MCP tool arguments: {e}"));
            }
        };

        let registry = crate::mcp::get_mcp_client_registry();
        match registry.call_tool(server_name, tool_name, args).await {
            Ok(result) => {
                use crate::mcp::types::ToolResultContent;
                let is_error = result.is_error.unwrap_or(false);
                let text: String = result
                    .content
                    .into_iter()
                    .map(|c| match c {
                        ToolResultContent::Text { text } => text,
                        ToolResultContent::Image { data, mime_type } => {
                            format!("[image: {mime_type}, {} bytes]", data.len())
                        }
                        ToolResultContent::Resource { resource } => {
                            format!("[resource: {}]", resource.uri)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                if is_error {
                    ToolResult::Error(text)
                } else {
                    ToolResult::Success(text)
                }
            }
            Err(e) => ToolResult::Error(format!("MCP tool call failed: {e}")),
        }
    }

    /// Execute the built-in MCP manager tool (search/evaluate/install/enable/disable/list/remove).
    ///
    /// Distinct from `execute_mcp_tool` which invokes tools on *connected* MCP servers via the
    /// `mcp__<server>__<tool>` naming convention. This method handles the `"mcp"` tool name which
    /// routes to the MCP manager — a registry-backed workflow tool for discovering, installing,
    /// and managing MCP server configurations in `.mcp.json`.
    async fn execute_mcp_manager_tool(&self, arguments: &str) -> ToolResult {
        use gestura_core_tools::mcp_manager;

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => match mcp_manager::handle(&args).await {
                Ok(output) => match serde_json::to_string_pretty(&output) {
                    Ok(s) => ToolResult::Success(s),
                    Err(e) => ToolResult::Error(format!("Serialize error: {e}")),
                },
                Err(e) => ToolResult::Error(e.to_string()),
            },
            Err(e) => ToolResult::Error(format!("Invalid arguments: {e}")),
        }
    }

    /// Execute GUI control tool
    async fn execute_gui_tool(&self, arguments: &str) -> ToolResult {
        use gestura_core_tools::gui::{GuiControlRequest, execute_gui_control};
        match serde_json::from_str::<GuiControlRequest>(arguments) {
            Ok(req) => match execute_gui_control(req).await {
                Ok(resp) => ToolResult::Success(resp.message),
                Err(e) => ToolResult::Error(e.to_string()),
            },
            Err(e) => ToolResult::Error(format!("Invalid arguments: {}", e)),
        }
    }

    /// Execute web tool
    async fn execute_web_tool(&self, arguments: &str) -> ToolResult {
        use crate::tools::WebTools;

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("search");

                let web = WebTools::default();
                match operation {
                    "fetch" => {
                        let url = match args.get("url").and_then(|v| v.as_str()) {
                            Some(u) if !u.trim().is_empty() => u,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'url' for web fetch operation"
                                        .to_string(),
                                );
                            }
                        };

                        // Use fetch_and_extract to get structured content instead of raw HTML
                        // This prevents token overflow by extracting only relevant content
                        match web.fetch_and_extract(url).await {
                            Ok(extracted) => {
                                // Return structured extracted content instead of raw HTML
                                let result = serde_json::json!({
                                    "url": url,
                                    "title": extracted.title,
                                    "description": extracted.description,
                                    "content": extracted.main_content,
                                    "links": extracted.links,
                                });
                                match serde_json::to_string_pretty(&result) {
                                    Ok(s) => ToolResult::Success(s),
                                    Err(e) => ToolResult::Error(format!("Serialize error: {e}")),
                                }
                            }
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "search" => {
                        let query = match args.get("query").and_then(|v| v.as_str()) {
                            Some(q) if !q.trim().is_empty() => q,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'query' for web search operation"
                                        .to_string(),
                                );
                            }
                        };

                        let num_results = args
                            .get("num_results")
                            .and_then(|v| v.as_u64())
                            .or_else(|| args.get("max_results").and_then(|v| v.as_u64()))
                            .map(|n| n as usize);

                        match web.search(query, num_results).await {
                            Ok(res) => match serde_json::to_string_pretty(&res) {
                                Ok(s) => ToolResult::Success(s),
                                Err(e) => ToolResult::Error(format!("Serialize error: {e}")),
                            },
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    other => ToolResult::Error(format!("Unknown web operation: {other}")),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {e}")),
        }
    }

    /// Execute code tool with optional workspace sandboxing
    async fn execute_code_tool(
        &self,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        use crate::tools::code_async;

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("stats");

                let raw_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                let raw_path = if raw_path.trim().is_empty() {
                    "."
                } else {
                    raw_path
                };

                // Resolve path within workspace if set
                let resolved_path = if let Some(ws) = workspace {
                    match ws.resolve_path(Path::new(raw_path)) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return ToolResult::Error(format!(
                                "Path '{}' is outside workspace: {}",
                                raw_path, e
                            ));
                        }
                    }
                } else {
                    raw_path.to_string()
                };

                // Optional extra parameters used by specific operations.
                let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
                let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
                let max_depth =
                    args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(4) as usize;
                let max_results = args
                    .get("max_results")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(100) as usize;
                let context_lines = args
                    .get("context_lines")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(2) as usize;
                let case_sensitive = args
                    .get("case_sensitive")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let file_glob: Option<String> = args
                    .get("file_glob")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let fix = args.get("fix").and_then(|v| v.as_bool()).unwrap_or(false);
                let filter: Option<String> = args
                    .get("filter")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                // Collect paths for batch operations.
                let batch_paths: Vec<String> = args
                    .get("paths")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();

                // Collect edits for batch_edit.
                let batch_edits: Vec<crate::tools::code::EditOp> = args
                    .get("edits")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| {
                                serde_json::from_value::<crate::tools::code::EditOp>(v.clone()).ok()
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                match operation {
                    "stats" => match code_async::stats_dir(&resolved_path).await {
                        Ok(s) => ToolResult::Success(s),
                        Err(e) => ToolResult::Error(e.to_string()),
                    },
                    "map" => match code_async::map(&resolved_path, max_depth).await {
                        Ok(s) => ToolResult::Success(s),
                        Err(e) => ToolResult::Error(e.to_string()),
                    },
                    "symbols" => match code_async::symbols(&resolved_path).await {
                        Ok(s) => ToolResult::Success(s),
                        Err(e) => ToolResult::Error(e.to_string()),
                    },
                    "references" => {
                        if symbol.is_empty() {
                            return ToolResult::Error(
                                "Missing required parameter: symbol".to_string(),
                            );
                        }
                        match code_async::references(symbol, &resolved_path).await {
                            Ok(s) => ToolResult::Success(s),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "definition" => {
                        if symbol.is_empty() {
                            return ToolResult::Error(
                                "Missing required parameter: symbol".to_string(),
                            );
                        }
                        match code_async::definition(symbol, &resolved_path).await {
                            Ok(s) => ToolResult::Success(s),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "deps" => match code_async::deps(&resolved_path).await {
                        Ok(s) => ToolResult::Success(s),
                        Err(e) => ToolResult::Error(e.to_string()),
                    },
                    "lint" => match code_async::lint(&resolved_path, fix).await {
                        Ok(s) => ToolResult::Success(s),
                        Err(e) => ToolResult::Error(e.to_string()),
                    },
                    "test" => match code_async::test(&resolved_path, filter).await {
                        Ok(s) => ToolResult::Success(s),
                        Err(e) => ToolResult::Error(e.to_string()),
                    },
                    "glob" => {
                        if pattern.is_empty() {
                            return ToolResult::Error(
                                "Missing required parameter: pattern".to_string(),
                            );
                        }
                        match code_async::glob_search(pattern, &resolved_path, max_results).await {
                            Ok(s) => ToolResult::Success(s),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "grep" => {
                        if pattern.is_empty() {
                            return ToolResult::Error(
                                "Missing required parameter: pattern".to_string(),
                            );
                        }
                        match code_async::grep(
                            pattern,
                            &resolved_path,
                            file_glob,
                            context_lines,
                            case_sensitive,
                            max_results,
                        )
                        .await
                        {
                            Ok(s) => ToolResult::Success(s),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "batch_read" => {
                        if batch_paths.is_empty() {
                            return ToolResult::Error(
                                "Missing required parameter: paths".to_string(),
                            );
                        }
                        match code_async::batch_read(batch_paths).await {
                            Ok(s) => ToolResult::Success(s),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "batch_edit" => {
                        if batch_edits.is_empty() {
                            return ToolResult::Error(
                                "Missing required parameter: edits".to_string(),
                            );
                        }
                        match code_async::batch_edit(batch_edits).await {
                            Ok(s) => ToolResult::Success(s),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "outline" => match code_async::outline(&resolved_path).await {
                        Ok(s) => ToolResult::Success(s),
                        Err(e) => ToolResult::Error(e.to_string()),
                    },
                    other => ToolResult::Error(format!("Unknown code operation: {other}")),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {e}")),
        }
    }

    /// Execute shell tool with workspace sandboxing.
    ///
    /// When `stream_tx` is `Some`, the command is executed via the streaming
    /// path (`shell_streaming`) so that stdout/stderr lines are emitted in
    /// real-time as `ShellOutput` / `ShellLifecycle` chunks.  When `None`,
    /// the legacy blocking path (`shell_async`) is used.
    async fn execute_shell_tool(
        &self,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
        stream_tx: Option<&mpsc::Sender<StreamChunk>>,
    ) -> ToolResult {
        use crate::session_workspace::is_shell_command_allowed;
        use crate::tools::shell_async;
        use std::collections::HashMap;

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let command = args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or(arguments);

                // Security check: validate command against blocklist
                if let Err(reason) = is_shell_command_allowed(command) {
                    tracing::warn!(
                        command = %command,
                        reason = %reason,
                        "Blocked dangerous shell command"
                    );
                    return ToolResult::Error(format!("Command blocked for security: {}", reason));
                }

                // Determine working directory
                let cwd = if let Some(ws) = workspace {
                    if let Some(requested_cwd) = args.get("cwd").and_then(|v| v.as_str()) {
                        match ws.resolve_path_for_read(Path::new(requested_cwd)) {
                            Ok(resolved) => {
                                if !resolved.is_dir() {
                                    return ToolResult::Error(format!(
                                        "cwd '{}' is not a directory",
                                        requested_cwd
                                    ));
                                }
                                Some(resolved.to_string_lossy().to_string())
                            }
                            Err(e) => {
                                return ToolResult::Error(format!(
                                    "Path '{}' is outside workspace: {}",
                                    requested_cwd, e
                                ));
                            }
                        }
                    } else {
                        Some(ws.root().to_string_lossy().to_string())
                    }
                } else {
                    args.get("cwd")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                };

                // Optional env vars
                let env: Option<HashMap<String, String>> =
                    args.get("env").and_then(|v| v.as_object()).map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect::<HashMap<_, _>>()
                    });

                // Optional timeout (seconds). Default to 60s to match prior behavior.
                let timeout_secs = args
                    .get("timeout_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(60);

                // Streaming path: send real-time output chunks to the frontend.
                if let Some(tx) = stream_tx {
                    use crate::tools::shell_streaming;

                    match shell_streaming::execute_streaming(
                        command,
                        cwd.as_deref(),
                        env.as_ref(),
                        Some(timeout_secs),
                        tx.clone(),
                    )
                    .await
                    {
                        Ok(r) => {
                            if r.success {
                                ToolResult::Success(r.stdout)
                            } else {
                                ToolResult::Error(format!(
                                    "Exit {}: {}",
                                    r.exit_code,
                                    r.stderr.trim_end()
                                ))
                            }
                        }
                        Err(e) => ToolResult::Error(e.to_string()),
                    }
                } else {
                    // Legacy non-streaming path.
                    match shell_async::execute_command_with_options(
                        command,
                        cwd.as_deref(),
                        env.as_ref(),
                        Some(timeout_secs),
                    )
                    .await
                    {
                        Ok(output) => ToolResult::Success(output),
                        Err(e) => ToolResult::Error(e.to_string()),
                    }
                }
            }
            Err(_) => {
                // Treat entire arguments as command - also need security check
                if let Err(reason) = is_shell_command_allowed(arguments) {
                    tracing::warn!(
                        command = %arguments,
                        reason = %reason,
                        "Blocked dangerous shell command"
                    );
                    return ToolResult::Error(format!("Command blocked for security: {}", reason));
                }

                let cwd = workspace.map(|ws| ws.root().to_string_lossy().to_string());

                // Streaming path for raw-argument commands.
                if let Some(tx) = stream_tx {
                    use crate::tools::shell_streaming;

                    match shell_streaming::execute_streaming(
                        arguments,
                        cwd.as_deref(),
                        None,
                        Some(60),
                        tx.clone(),
                    )
                    .await
                    {
                        Ok(r) => {
                            if r.success {
                                ToolResult::Success(r.stdout)
                            } else {
                                ToolResult::Error(format!(
                                    "Exit {}: {}",
                                    r.exit_code,
                                    r.stderr.trim_end()
                                ))
                            }
                        }
                        Err(e) => ToolResult::Error(e.to_string()),
                    }
                } else {
                    match shell_async::execute_command(arguments, cwd.as_deref()).await {
                        Ok(output) => ToolResult::Success(output),
                        Err(e) => ToolResult::Error(e.to_string()),
                    }
                }
            }
        }
    }

    /// Execute file tool with workspace sandboxing
    async fn execute_file_tool(
        &self,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        use crate::tools::file_async;

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        if args.get("content").is_some() {
                            "write"
                        } else {
                            "read"
                        }
                    });

                let raw_path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let path_str = if raw_path_str.trim().is_empty() {
                    "."
                } else {
                    raw_path_str
                };

                // Resolve path within workspace if set, using stricter variants depending on operation.
                let resolved_path = if let Some(ws) = workspace {
                    let resolved = match operation {
                        "read" => ws.resolve_path_for_read(Path::new(path_str)),
                        "write" | "edit" => ws.resolve_path_for_write(Path::new(path_str)),
                        _ => ws.resolve_path(Path::new(path_str)),
                    };

                    match resolved {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return ToolResult::Error(format!(
                                "Path '{}' is outside workspace: {}",
                                path_str, e
                            ));
                        }
                    }
                } else {
                    path_str.to_string()
                };

                match operation {
                    "write" => {
                        let content = match args.get("content").and_then(|v| v.as_str()) {
                            Some(c) => c,
                            None => {
                                return ToolResult::Error(
                                    "Missing required field 'content' for file write operation"
                                        .to_string(),
                                );
                            }
                        };

                        match file_async::write_file(&resolved_path, content).await {
                            Ok(_) => ToolResult::Success(format!("Written to {}", raw_path_str)),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "read" => {
                        let start_line = args
                            .get("start")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize);
                        let end_line = args.get("end").and_then(|v| v.as_u64()).map(|v| v as usize);

                        match file_async::read_file_range(&resolved_path, start_line, end_line)
                            .await
                        {
                            Ok(content) => ToolResult::Success(content),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "list" => {
                        let show_hidden = args
                            .get("show_hidden")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let max_entries = args
                            .get("max_entries")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize);

                        match file_async::list_dir(&resolved_path, show_hidden, max_entries).await {
                            Ok(out) => ToolResult::Success(out),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "tree" => {
                        let max_depth = args
                            .get("max_depth")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize);

                        let show_hidden = args
                            .get("show_hidden")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        match file_async::tree_dir(&resolved_path, max_depth, show_hidden).await {
                            Ok(out) => ToolResult::Success(out),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "edit" => {
                        let old_str = match args.get("old").and_then(|v| v.as_str()) {
                            Some(s) if !s.is_empty() => s,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'old' for file edit operation"
                                        .to_string(),
                                );
                            }
                        };
                        let new_str = match args.get("new").and_then(|v| v.as_str()) {
                            Some(s) => s,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'new' for file edit operation"
                                        .to_string(),
                                );
                            }
                        };

                        match file_async::edit_file(&resolved_path, old_str, new_str).await {
                            Ok(out) => ToolResult::Success(out),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "search" => {
                        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
                            Some(p) if !p.trim().is_empty() => p,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'pattern' for file search operation"
                                        .to_string(),
                                );
                            }
                        };
                        let recursive = args
                            .get("recursive")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        let max_matches = args
                            .get("max_matches")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize);

                        match file_async::search_files(
                            pattern,
                            &resolved_path,
                            recursive,
                            max_matches,
                        )
                        .await
                        {
                            Ok(out) => ToolResult::Success(out),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    other => ToolResult::Error(format!(
                        "Unknown file operation: {other}. Supported operations: read, write, edit, list, tree, search"
                    )),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {}", e)),
        }
    }

    /// Execute git tool with workspace sandboxing
    async fn execute_git_tool(
        &self,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        use crate::tools::git_async;

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("status");

                // Only allow the operations we explicitly support in the runtime executor.
                let supported = matches!(
                    operation,
                    "status" | "diff" | "diff-staged" | "log" | "branches"
                );
                if !supported {
                    return ToolResult::Error(format!(
                        "Unknown git operation: {operation}. Supported operations: status, diff, diff-staged, log, branches"
                    ));
                }

                let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

                // Resolve path within workspace if set
                let resolved_path = if let Some(ws) = workspace {
                    match ws.resolve_path(Path::new(path_str)) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return ToolResult::Error(format!(
                                "Path '{}' is outside workspace: {}",
                                path_str, e
                            ));
                        }
                    }
                } else {
                    path_str.to_string()
                };

                match git_async::execute_git(operation, &resolved_path).await {
                    Ok(output) => ToolResult::Success(output),
                    Err(e) => ToolResult::Error(e.to_string()),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {}", e)),
        }
    }

    /// Execute task management tool
    async fn execute_task_tool(
        &self,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        use crate::TaskStatus;

        // Use the process-wide shared TaskManager so all subsystems share one cache.
        let manager = crate::get_global_task_manager();

        // Get session_id from workspace
        let session_id = match workspace {
            Some(ws) => &ws.session_id,
            None => {
                return ToolResult::Error(
                    "Task management requires an active session with workspace".to_string(),
                );
            }
        };

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("list");

                match operation {
                    "create" => {
                        let name = match args.get("name").and_then(|v| v.as_str()) {
                            Some(n) => n,
                            None => {
                                return ToolResult::Error(
                                    "Missing required field 'name' for create operation"
                                        .to_string(),
                                );
                            }
                        };
                        let description = args
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let parent_id = args
                            .get("parent_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        match manager.create_task(session_id, name, description, parent_id) {
                            Ok(task) => ToolResult::Success(format!(
                                "Created task '{}' (ID: {})\nDescription: {}\nStatus: {:?}",
                                task.name, task.id, task.description, task.status
                            )),
                            Err(e) => ToolResult::Error(format!("Failed to create task: {}", e)),
                        }
                    }
                    "update_status" => {
                        let task_id =
                            match args.get("task_id").and_then(|v| v.as_str()) {
                                Some(id) => id,
                                None => return ToolResult::Error(
                                    "Missing required field 'task_id' for update_status operation"
                                        .to_string(),
                                ),
                            };
                        let status_str =
                            match args.get("status").and_then(|v| v.as_str()) {
                                Some(s) => s,
                                None => return ToolResult::Error(
                                    "Missing required field 'status' for update_status operation"
                                        .to_string(),
                                ),
                            };

                        let status = match status_str.to_lowercase().as_str() {
                            "notstarted" | "not_started" => TaskStatus::NotStarted,
                            "blocked" | "waiting" => TaskStatus::Blocked,
                            "inprogress" | "in_progress" => TaskStatus::InProgress,
                            "completed" => TaskStatus::Completed,
                            "cancelled" => TaskStatus::Cancelled,
                            _ => {
                                return ToolResult::Error(format!(
                                    "Invalid status '{}'. Use 'notstarted', 'blocked', 'inprogress', 'completed', or 'cancelled'",
                                    status_str
                                ));
                            }
                        };

                        match manager.update_task_status(session_id, task_id, status) {
                            Ok(_) => ToolResult::Success(format!(
                                "Updated task {} status to {:?}",
                                task_id, status
                            )),
                            Err(e) => {
                                ToolResult::Error(format!("Failed to update task status: {}", e))
                            }
                        }
                    }
                    "update" => {
                        let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
                            Some(id) => id,
                            None => {
                                return ToolResult::Error(
                                    "Missing required field 'task_id' for update operation"
                                        .to_string(),
                                );
                            }
                        };
                        let name = args
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let description = args
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        match manager.update_task(
                            session_id,
                            task_id,
                            name.clone(),
                            description.clone(),
                        ) {
                            Ok(_) => {
                                let mut updates = Vec::new();
                                if let Some(n) = name {
                                    updates.push(format!("name to '{}'", n));
                                }
                                if let Some(d) = description {
                                    updates.push(format!("description to '{}'", d));
                                }
                                ToolResult::Success(format!(
                                    "Updated task {}: {}",
                                    task_id,
                                    updates.join(", ")
                                ))
                            }
                            Err(e) => ToolResult::Error(format!("Failed to update task: {}", e)),
                        }
                    }
                    "delete" => {
                        let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
                            Some(id) => id,
                            None => {
                                return ToolResult::Error(
                                    "Missing required field 'task_id' for delete operation"
                                        .to_string(),
                                );
                            }
                        };

                        match manager.delete_task(session_id, task_id) {
                            Ok(task) => ToolResult::Success(format!(
                                "Deleted task '{}' (ID: {})",
                                task.name, task.id
                            )),
                            Err(e) => ToolResult::Error(format!("Failed to delete task: {}", e)),
                        }
                    }
                    "list" => match manager.list_tasks(session_id) {
                        Ok(tasks) => {
                            if tasks.is_empty() {
                                ToolResult::Success("No tasks found for this session".to_string())
                            } else {
                                let mut output = format!("Found {} task(s):\n\n", tasks.len());
                                for task in tasks {
                                    output.push_str(&format!(
                                        "• {} (ID: {})\n  Status: {:?}\n  Description: {}\n",
                                        task.name, task.id, task.status, task.description
                                    ));
                                    if let Some(parent_id) = &task.parent_id {
                                        output.push_str(&format!("  Parent: {}\n", parent_id));
                                    }
                                    output.push('\n');
                                }
                                ToolResult::Success(output)
                            }
                        }
                        Err(e) => ToolResult::Error(format!("Failed to list tasks: {}", e)),
                    },
                    "get_hierarchy" => match manager.get_hierarchy(session_id) {
                        Ok(hierarchy) => {
                            if hierarchy.is_empty() {
                                ToolResult::Success("No tasks found for this session".to_string())
                            } else {
                                let mut output = format!(
                                    "Task hierarchy ({} root task(s)):\n\n",
                                    hierarchy.len()
                                );
                                for (root, subtasks) in hierarchy {
                                    output.push_str(&format!(
                                        "• {} (ID: {})\n  Status: {:?}\n  Description: {}\n",
                                        root.name, root.id, root.status, root.description
                                    ));
                                    if !subtasks.is_empty() {
                                        output.push_str(&format!(
                                            "  Subtasks ({}):\n",
                                            subtasks.len()
                                        ));
                                        for subtask in subtasks {
                                            output.push_str(&format!(
                                                "    - {} (ID: {}, Status: {:?})\n",
                                                subtask.name, subtask.id, subtask.status
                                            ));
                                        }
                                    }
                                    output.push('\n');
                                }
                                ToolResult::Success(output)
                            }
                        }
                        Err(e) => ToolResult::Error(format!("Failed to get task hierarchy: {}", e)),
                    },
                    _ => ToolResult::Error(format!(
                        "Unknown task operation: {}. Supported: create, update_status, update, delete, list, get_hierarchy",
                        operation
                    )),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {}", e)),
        }
    }

    /// Truncate tool result to prevent token explosion
    /// Execute screen tool (screenshot and screen recording)
    async fn execute_screen_tool(
        &self,
        tool_name: &str,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        use crate::tools::screen_async;
        use crate::tools::screen_async::{
            ScreenshotInlineOptions, ScreenshotReturnMode, ScreenshotReturnOptions,
        };

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                // Determine operation type based on tool name + optional operation field.
                // This prevents, e.g., the `screenshot` tool from being used to start/stop
                // recordings via a spoofed `operation` argument.
                let operation_from_args = args.get("operation").and_then(|v| v.as_str());

                let operation = match tool_name {
                    "screenshot" => operation_from_args.unwrap_or("screenshot"),
                    "screen_record" => match operation_from_args {
                        Some(op) => op,
                        None => {
                            return ToolResult::Error(
                                "Missing required field 'operation' for screen_record".to_string(),
                            );
                        }
                    },
                    other => {
                        return ToolResult::Error(format!("Unknown screen tool: {other}"));
                    }
                };

                // Enforce tool-specific allowed operations.
                match tool_name {
                    "screenshot" => {
                        if !matches!(operation, "screenshot" | "capture") {
                            return ToolResult::Error(format!(
                                "Tool 'screenshot' does not support operation '{operation}'. Supported operations: screenshot, capture"
                            ));
                        }
                    }
                    "screen_record" => {
                        if !matches!(operation, "start" | "stop") {
                            return ToolResult::Error(format!(
                                "Tool 'screen_record' does not support operation '{operation}'. Supported operations: start, stop"
                            ));
                        }
                    }
                    _ => {}
                }

                // Helper: choose an output path when the caller didn't specify one.
                fn default_artifact_path(kind: &str, ext: &str) -> std::path::PathBuf {
                    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
                    let id = uuid::Uuid::new_v4().to_string();
                    std::path::PathBuf::from(".gestura")
                        .join("artifacts")
                        .join("screen")
                        .join(format!("{kind}-{ts}-{id}.{ext}"))
                }

                fn normalize_ext(s: &str) -> String {
                    s.trim().trim_start_matches('.').to_ascii_lowercase()
                }

                fn apply_or_validate_extension(
                    mut path: std::path::PathBuf,
                    desired_ext: Option<&str>,
                ) -> std::result::Result<std::path::PathBuf, String> {
                    let Some(desired_ext) = desired_ext else {
                        return Ok(path);
                    };
                    let desired_ext = normalize_ext(desired_ext);
                    if desired_ext.is_empty() {
                        return Ok(path);
                    }

                    match path.extension().and_then(|e| e.to_str()) {
                        Some(existing) if !existing.is_empty() => {
                            let existing = normalize_ext(existing);
                            if existing != desired_ext {
                                return Err(format!(
                                    "output_path extension '.{existing}' does not match requested output_format '.{desired_ext}'. Either omit output_format or make them match."
                                ));
                            }
                        }
                        _ => {
                            path.set_extension(&desired_ext);
                        }
                    }
                    Ok(path)
                }

                fn parse_screenshot_return_options(
                    args: &serde_json::Value,
                ) -> std::result::Result<ScreenshotReturnOptions, String> {
                    let mut opts = ScreenshotReturnOptions::default();

                    let return_obj = args.get("return").and_then(|v| v.as_object());
                    if return_obj.is_none() {
                        return Ok(opts);
                    }
                    let return_obj = return_obj.unwrap();

                    if let Some(mode) = return_obj.get("mode").and_then(|v| v.as_str()) {
                        match mode.trim() {
                            "path" => opts.mode = ScreenshotReturnMode::Path,
                            "inline_base64" => opts.mode = ScreenshotReturnMode::InlineBase64,
                            other => {
                                return Err(format!(
                                    "Invalid return.mode '{other}'. Supported: path, inline_base64"
                                ));
                            }
                        }
                    }

                    if let Some(inline_obj) = return_obj.get("inline").and_then(|v| v.as_object()) {
                        let mut inline = ScreenshotInlineOptions::default();
                        if let Some(w) = inline_obj.get("max_width").and_then(|v| v.as_u64()) {
                            inline.max_width = Some(w as u32);
                        }
                        if let Some(h) = inline_obj.get("max_height").and_then(|v| v.as_u64()) {
                            inline.max_height = Some(h as u32);
                        }
                        if let Some(m) = inline_obj.get("max_base64_chars").and_then(|v| v.as_u64())
                        {
                            inline.max_base64_chars = m as usize;
                        }
                        if let Some(m) = inline_obj.get("max_result_chars").and_then(|v| v.as_u64())
                        {
                            inline.max_result_chars = m as usize;
                        }
                        opts.inline = inline;
                    }

                    Ok(opts)
                }

                match operation {
                    "screenshot" | "capture" => {
                        let output_format = args
                            .get("output_format")
                            .and_then(|v| v.as_str())
                            .map(|s| {
                                let ext = normalize_ext(s);
                                // Treat jpeg as a synonym for jpg to avoid needless caller friction.
                                if ext == "jpeg" {
                                    "jpg".to_string()
                                } else {
                                    ext
                                }
                            })
                            .filter(|s| !s.is_empty());

                        if let Some(fmt) = output_format.as_deref()
                            && !matches!(fmt, "png" | "jpg")
                        {
                            return ToolResult::Error(format!(
                                "Invalid output_format '{fmt}'. Supported: png, jpg (jpeg is accepted as an alias for jpg)"
                            ));
                        }

                        let screenshot_return = match parse_screenshot_return_options(&args) {
                            Ok(o) => o,
                            Err(e) => return ToolResult::Error(e),
                        };

                        let output_path = args
                            .get("output_path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| {
                                let ext = output_format.as_deref().unwrap_or("png");
                                default_artifact_path("screenshot", ext)
                            });

                        let output_path = match apply_or_validate_extension(
                            output_path,
                            output_format.as_deref(),
                        ) {
                            Ok(p) => p,
                            Err(e) => return ToolResult::Error(e),
                        };

                        // Resolve path within workspace if set
                        let resolved_path = if let Some(ws) = workspace {
                            match ws.resolve_path_for_create(&output_path) {
                                Ok(p) => p.to_string_lossy().to_string(),
                                Err(e) => {
                                    return ToolResult::Error(format!(
                                        "Path '{}' is outside workspace: {}",
                                        output_path.display(),
                                        e
                                    ));
                                }
                            }
                        } else {
                            output_path.to_string_lossy().to_string()
                        };

                        // Parse optional region
                        let region = args.get("region").and_then(|r| {
                            if let Some(obj) = r.as_object() {
                                let x = obj.get("x")?.as_u64()? as u32;
                                let y = obj.get("y")?.as_u64()? as u32;
                                let width = obj.get("width")?.as_u64()? as u32;
                                let height = obj.get("height")?.as_u64()? as u32;
                                Some((x, y, width, height))
                            } else {
                                None
                            }
                        });

                        // Parse optional display
                        let display = args
                            .get("display")
                            .and_then(|v| v.as_u64())
                            .map(|d| d as u32);

                        match screen_async::screenshot_with_options(
                            &resolved_path,
                            region,
                            display,
                            screenshot_return,
                        )
                        .await
                        {
                            Ok(result) => ToolResult::Success(result),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "start" | "start_recording" => {
                        let output_format = args
                            .get("output_format")
                            .and_then(|v| v.as_str())
                            .map(normalize_ext)
                            .filter(|s| !s.is_empty());

                        if let Some(fmt) = output_format.as_deref()
                            && !matches!(fmt, "mp4" | "mov")
                        {
                            return ToolResult::Error(format!(
                                "Invalid output_format '{fmt}'. Supported: mp4, mov"
                            ));
                        }

                        let output_path = args
                            .get("output_path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| {
                                let ext = output_format.as_deref().unwrap_or("mp4");
                                default_artifact_path("screen-recording", ext)
                            });

                        let output_path = match apply_or_validate_extension(
                            output_path,
                            output_format.as_deref(),
                        ) {
                            Ok(p) => p,
                            Err(e) => return ToolResult::Error(e),
                        };

                        // Resolve path within workspace if set
                        let resolved_path = if let Some(ws) = workspace {
                            match ws.resolve_path_for_create(&output_path) {
                                Ok(p) => p.to_string_lossy().to_string(),
                                Err(e) => {
                                    return ToolResult::Error(format!(
                                        "Path '{}' is outside workspace: {}",
                                        output_path.display(),
                                        e
                                    ));
                                }
                            }
                        } else {
                            output_path.to_string_lossy().to_string()
                        };

                        // Parse optional region
                        let region = args.get("region").and_then(|r| {
                            if let Some(obj) = r.as_object() {
                                let x = obj.get("x")?.as_u64()? as u32;
                                let y = obj.get("y")?.as_u64()? as u32;
                                let width = obj.get("width")?.as_u64()? as u32;
                                let height = obj.get("height")?.as_u64()? as u32;
                                Some((x, y, width, height))
                            } else {
                                None
                            }
                        });

                        // Parse optional display
                        let display = args
                            .get("display")
                            .and_then(|v| v.as_u64())
                            .map(|d| d as u32);

                        match screen_async::start_recording(&resolved_path, region, display).await {
                            Ok(result) => ToolResult::Success(result),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "stop" | "stop_recording" => {
                        let recording_id = match args.get("recording_id").and_then(|v| v.as_str()) {
                            Some(id) if !id.trim().is_empty() => id,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'recording_id' for stop_recording operation"
                                        .to_string(),
                                );
                            }
                        };

                        match screen_async::stop_recording(recording_id).await {
                            Ok(result) => ToolResult::Success(result),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    other => ToolResult::Error(format!(
                        "Unknown screen operation: {}. Supported operations: screenshot, start, stop",
                        other
                    )),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {}", e)),
        }
    }

    /// Limits a tool result to `pipeline_config.tool_result_max_chars` with a
    /// truncation indicator so the LLM knows content was omitted.
    pub(super) fn truncate_tool_result(&self, result: &str) -> String {
        let max_chars = self.pipeline_config.tool_result_max_chars;

        if result.len() <= max_chars {
            result.to_string()
        } else {
            // Snap back to the nearest valid char boundary so we never split a
            // multi-byte UTF-8 sequence.
            let boundary = result
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i <= max_chars)
                .last()
                .unwrap_or(max_chars);
            let truncated = &result[..boundary];
            let remaining = result.len() - boundary;

            if self.pipeline_config.log_token_usage {
                tracing::debug!(
                    original_length = result.len(),
                    truncated_length = boundary,
                    remaining_chars = remaining,
                    max_chars = max_chars,
                    "Truncating tool result to prevent token explosion"
                );
            }

            format!(
                "{}\n[... truncated {} more characters ...]",
                truncated, remaining
            )
        }
    }

    /// Build a continuation prompt after tool execution
    pub(super) fn build_tool_continuation_prompt(
        &self,
        original_prompt: &str,
        assistant_response: &str,
        tool_calls: &[ToolCallRecord],
    ) -> String {
        let mut prompt = original_prompt.to_string();

        prompt.push_str(&format!("\nAssistant: {}\n", assistant_response));

        for tool_call in tool_calls {
            let result_text = match &tool_call.result {
                ToolResult::Success(s) => {
                    let truncated = self.truncate_tool_result(s);
                    format!("Success: {}", truncated)
                }
                ToolResult::Error(e) => {
                    let truncated = self.truncate_tool_result(e);
                    format!("Error: {}", truncated)
                }
                ToolResult::Skipped(r) => {
                    let truncated = self.truncate_tool_result(r);
                    format!("Skipped: {}", truncated)
                }
            };
            prompt.push_str(&format!(
                "\nTool {} result:\n{}\n",
                tool_call.name, result_text
            ));
        }

        prompt.push_str(
            "\nUser: Based on the tool results above, provide a complete and helpful response \
             to my original request. Synthesize the information, highlight the key findings, \
             and present a clear answer. If you created any tasks to track this work, mark \
             them as completed now.\n",
        );

        prompt
    }
}
