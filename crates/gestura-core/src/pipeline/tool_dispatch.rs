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

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecoverableToolLoopPattern {
    FileWriteMissingContent,
    TaskCreateMissingName,
    TaskUpdateStatusMissingExplicitStatus,
}

impl AgentPipeline {
    fn normalize_optional_tool_string(raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("none")
            || trimmed.eq_ignore_ascii_case("null")
        {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn strip_xml_comments(raw: &str) -> String {
        let mut output = String::new();
        let mut cursor = 0usize;
        let open = "<!--";
        let close = "-->";

        while let Some(start_rel) = raw[cursor..].find(open) {
            let start = cursor + start_rel;
            output.push_str(&raw[cursor..start]);
            let comment_start = start + open.len();
            let Some(end_rel) = raw[comment_start..].find(close) else {
                return output.trim().to_string();
            };
            cursor = comment_start + end_rel + close.len();
        }

        output.push_str(&raw[cursor..]);
        output.trim().to_string()
    }

    fn normalize_task_reference(raw: &str) -> Option<String> {
        let stripped = Self::strip_parameter_fragments(&Self::strip_xml_comments(raw));
        let trimmed = stripped
            .trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '\\');

        for token in trimmed.split(|c: char| {
            c.is_whitespace() || matches!(c, '<' | '>' | ',' | ';' | '(' | ')' | '[' | ']')
        }) {
            let candidate = token
                .trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '\\');
            if let Ok(uuid) = uuid::Uuid::parse_str(candidate) {
                return Some(uuid.to_string());
            }
        }

        Self::normalize_optional_tool_string(trimmed)
    }

    fn normalize_tool_path(raw: &str) -> Option<String> {
        let stripped = Self::strip_parameter_fragments(raw);
        let trimmed = stripped.trim();
        let trimmed = trimmed
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                trimmed
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(trimmed)
            .trim();
        Self::normalize_optional_tool_string(trimmed)
    }

    fn normalize_file_tool_arguments(args: serde_json::Value) -> serde_json::Value {
        let mut obj = match args {
            serde_json::Value::Object(map) => map,
            other => return other,
        };

        let mut recovered = serde_json::Map::new();
        for value in obj.values() {
            if let Some(raw) = value.as_str() {
                for (name, content) in Self::extract_parameter_fragments(raw) {
                    recovered
                        .entry(name)
                        .or_insert_with(|| serde_json::Value::String(content));
                }
            }
        }

        for (key, value) in recovered {
            obj.entry(key).or_insert(value);
        }

        if let Some(operation) = obj.get("operation").and_then(|value| value.as_str()) {
            let normalized = Self::strip_parameter_fragments(operation)
                .trim()
                .to_ascii_lowercase();
            if !normalized.is_empty() {
                obj.insert(
                    "operation".to_string(),
                    serde_json::Value::String(normalized),
                );
            }
        }

        if let Some(path) = obj.get("path").and_then(|value| value.as_str())
            && let Some(normalized) = Self::normalize_tool_path(path)
        {
            obj.insert("path".to_string(), serde_json::Value::String(normalized));
        }

        let operation = obj
            .get("operation")
            .and_then(|value| value.as_str())
            .unwrap_or("read")
            .to_string();

        for (alias, canonical) in [
            ("old_str", "old"),
            ("new_str", "new"),
            ("replacement", "new"),
            ("pattern", "old"),
            ("content", "new"),
        ] {
            if obj.get(canonical).is_none()
                && let Some(value) = obj.get(alias).cloned()
            {
                obj.insert(canonical.to_string(), value);
            }
        }

        if operation == "write" && obj.get("content").is_none() {
            for alias in [
                "contents", "text", "data", "body", "value", "new", "new_str",
            ] {
                if let Some(value) = obj.get(alias).cloned() {
                    obj.insert("content".to_string(), value);
                    break;
                }
            }
        }

        serde_json::Value::Object(obj)
    }

    fn format_missing_file_write_content_error(args: &serde_json::Value) -> String {
        let path = args
            .get("path")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("<path>");

        let provided_fields = args
            .as_object()
            .map(|map| {
                let mut keys = map.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                keys.join(", ")
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "none".to_string());

        let invalid_substitutes = [
            "pattern",
            "start",
            "end",
            "recursive",
            "show_hidden",
            "max_matches",
            "max_entries",
            "max_depth",
        ]
        .into_iter()
        .filter(|field| args.get(*field).is_some())
        .collect::<Vec<_>>();

        let write_example = serde_json::json!({
            "operation": "write",
            "path": path,
            "content": "<full file contents here>"
        })
        .to_string();
        let edit_example = serde_json::json!({
            "operation": "edit",
            "path": path,
            "old": "<exact existing text>",
            "new": "<replacement text>"
        })
        .to_string();

        let mut message = format!(
            "Missing required field 'content' for file write operation. `write` requires the full destination file text in `content`. Provided fields: {provided_fields}."
        );

        if !invalid_substitutes.is_empty() {
            message.push_str(&format!(
                " The fields {} are not valid substitutes for file content.",
                invalid_substitutes.join(", ")
            ));
        }

        message.push_str(&format!(
            " Retry with {write_example} if you already know the full file contents, or use {edit_example} for a targeted replacement. Do not retry the same malformed `write` call without adding real `content`."
        ));

        message
    }

    fn format_missing_task_update_status_error(args: &serde_json::Value) -> String {
        let task_id = args
            .get("task_id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("<task-id>");

        let provided_fields = args
            .as_object()
            .map(|map| {
                let mut keys = map.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                keys.join(", ")
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "none".to_string());

        let example = serde_json::json!({
            "operation": "update_status",
            "task_id": task_id,
            "status": "inprogress"
        })
        .to_string();

        format!(
            "Missing required field 'status' for update_status operation. `update_status` requires both `task_id` and `status`. Provided fields: {provided_fields}. Retry with {example} using one of: `notstarted`, `inprogress`, `completed`, or `cancelled`. Do not omit `status` to ask the runtime to infer or preserve the current state; if no status changed, skip the task update and continue the real work."
        )
    }

    fn format_missing_task_create_name_error(args: &serde_json::Value) -> String {
        let provided_fields = args
            .as_object()
            .map(|map| {
                let mut keys = map.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                keys.join(", ")
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "none".to_string());

        let example = serde_json::json!({
            "operation": "create",
            "name": "Build hello world Tauri app",
            "description": "Initialize a small Tauri GUI project, implement a Hello World window, then build and test it"
        })
        .to_string();

        format!(
            "Missing required field 'name' for create operation. `create` requires a specific task `name`; for non-trivial work it should usually also include a concrete `description`. Provided fields: {provided_fields}. Retry with {example}. Do not rely on the runtime to invent placeholder names like 'Untitled Task'."
        )
    }

    fn detect_recoverable_tool_loop_pattern(
        name: &str,
        arguments: &str,
    ) -> Option<RecoverableToolLoopPattern> {
        match name {
            "file" | "read_file" | "write_file" => {
                let args = serde_json::from_str::<serde_json::Value>(arguments).ok()?;
                let args = Self::normalize_file_tool_arguments(args);
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

                if operation == "write" && args.get("content").and_then(|v| v.as_str()).is_none() {
                    Some(RecoverableToolLoopPattern::FileWriteMissingContent)
                } else {
                    None
                }
            }
            "task" | "tasks" => {
                let args = serde_json::from_str::<serde_json::Value>(arguments).ok()?;
                let args = Self::normalize_task_tool_arguments(args);
                let operation = args.get("operation").and_then(|v| v.as_str()).unwrap_or("list");
                let explicit_name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty());
                let explicit_status = args
                    .get("status")
                    .or_else(|| args.get("state"))
                    .or_else(|| args.get("new_status"))
                    .or_else(|| args.get("target_status"))
                    .and_then(|v| v.as_str())
                    .and_then(Self::parse_task_status);

                if operation == "create" && explicit_name.is_none() {
                    Some(RecoverableToolLoopPattern::TaskCreateMissingName)
                } else if operation == "update_status" && explicit_status.is_none() {
                    Some(RecoverableToolLoopPattern::TaskUpdateStatusMissingExplicitStatus)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn is_recoverable_tool_loop_attempt(
        pattern: &RecoverableToolLoopPattern,
        record: &ToolCallRecord,
    ) -> bool {
        let record_pattern = Self::detect_recoverable_tool_loop_pattern(&record.name, &record.arguments);
        if record_pattern.as_ref() != Some(pattern) {
            return false;
        }

        match pattern {
            RecoverableToolLoopPattern::FileWriteMissingContent => {
                matches!(record.result, ToolResult::Error(_) | ToolResult::Skipped(_))
            }
            RecoverableToolLoopPattern::TaskCreateMissingName => {
                matches!(record.result, ToolResult::Error(_) | ToolResult::Skipped(_))
            }
            RecoverableToolLoopPattern::TaskUpdateStatusMissingExplicitStatus => {
                matches!(record.result, ToolResult::Error(_) | ToolResult::Skipped(_))
            }
        }
    }

    fn count_prior_recoverable_tool_loop_attempts<'a, I>(
        pattern: &RecoverableToolLoopPattern,
        prior_records: I,
    ) -> usize
    where
        I: IntoIterator<Item = &'a ToolCallRecord>,
    {
        prior_records
            .into_iter()
            .filter(|record| Self::is_recoverable_tool_loop_attempt(pattern, record))
            .count()
    }

    fn build_recoverable_tool_loop_breaker_message(
        pattern: &RecoverableToolLoopPattern,
        prior_attempts: usize,
    ) -> String {
        match pattern {
            RecoverableToolLoopPattern::FileWriteMissingContent => format!(
                "Loop breaker: skipped a repeated malformed `file.write` call without `content` after {prior_attempts} prior similar non-successful attempts in this run. The agent is still running. Do not retry `write` until you can provide the full destination file text in `content`. Choose a different next step instead: read the existing file, prepare the full file contents and then send one corrected `write`, or use `edit` with `old` and `new` for a targeted change."
            ),
            RecoverableToolLoopPattern::TaskCreateMissingName => format!(
                "Loop breaker: skipped a repeated malformed `task.create` call without a valid `name` after {prior_attempts} prior similar malformed attempts in this run. The agent is still running. Do not retry `create` without a specific task name. If you need task tracking, send one corrected `create` call with a concrete `name` and preferably a useful `description`; otherwise continue the real implementation work."
            ),
            RecoverableToolLoopPattern::TaskUpdateStatusMissingExplicitStatus => format!(
                "Loop breaker: skipped a repeated malformed `task.update_status` call without explicit `status` after {prior_attempts} prior similar malformed attempts in this run. The agent is still running. Do not retry `update_status` without `status`. If you intend a status change, send one corrected call with both `task_id` and `status`; otherwise continue the real implementation or verification work instead of repeating task bookkeeping."
            ),
        }
    }

    pub(super) fn repeated_malformed_tool_call_skip_message<'a, I>(
        name: &str,
        arguments: &str,
        prior_records: I,
    ) -> Option<String>
    where
        I: IntoIterator<Item = &'a ToolCallRecord>,
    {
        let pattern = Self::detect_recoverable_tool_loop_pattern(name, arguments)?;
        let prior_attempts =
            Self::count_prior_recoverable_tool_loop_attempts(&pattern, prior_records);

        (prior_attempts >= 2)
            .then(|| Self::build_recoverable_tool_loop_breaker_message(&pattern, prior_attempts))
    }

    fn should_echo_tool_call_arguments(record: &ToolCallRecord) -> bool {
        matches!(record.result, ToolResult::Error(_) | ToolResult::Skipped(_))
    }

    fn extract_parameter_fragments(raw: &str) -> Vec<(String, String)> {
        let mut extracted = Vec::new();
        let raw = Self::strip_xml_comments(raw);
        let mut cursor = 0usize;
        let open_markers = ["<parameter name=\"", "</parameter name=\""];
        let close = "</parameter>";

        while let Some((start, marker)) = open_markers
            .iter()
            .filter_map(|marker| raw[cursor..].find(marker).map(|idx| (cursor + idx, *marker)))
            .min_by_key(|(idx, _)| *idx)
        {
            let name_start = start + marker.len();
            let Some(name_end_rel) = raw[name_start..].find("\">") else {
                break;
            };
            let name_end = name_start + name_end_rel;
            let value_start = name_end + 2;
            let next_parameter_start = open_markers
                .iter()
                .filter_map(|marker| raw[value_start..].find(marker).map(|idx| value_start + idx))
                .min();
            let closing_parameter_end = raw[value_start..].find(close).map(|idx| value_start + idx);

            let (value_end, next_cursor) = match (closing_parameter_end, next_parameter_start) {
                (Some(close_idx), Some(next_idx)) if next_idx < close_idx => (next_idx, next_idx),
                (Some(close_idx), _) => (close_idx, close_idx + close.len()),
                (None, Some(next_idx)) => (next_idx, next_idx),
                (None, None) => (raw.len(), raw.len()),
            };

            let name = raw[name_start..name_end].trim();
            let value = raw[value_start..value_end].trim();
            if !name.is_empty() && !value.is_empty() {
                extracted.push((name.to_string(), value.to_string()));
            }
            cursor = next_cursor;
        }

        extracted
    }

    fn extract_named_attribute_fragments(raw: &str, names: &[&str]) -> Vec<(String, String)> {
        let mut extracted = Vec::new();
        let raw = Self::strip_xml_comments(raw);

        for name in names {
            for quote in ['"', '\''] {
                let marker = format!("{name}={quote}");
                let mut cursor = 0usize;

                while let Some(start_rel) = raw[cursor..].find(&marker) {
                    let start = cursor + start_rel;
                    if start > 0
                        && raw[..start]
                            .chars()
                            .next_back()
                            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                    {
                        cursor = start + marker.len();
                        continue;
                    }

                    let value_start = start + marker.len();
                    let Some(value_end_rel) = raw[value_start..].find(quote) else {
                        break;
                    };
                    let value_end = value_start + value_end_rel;
                    let value = raw[value_start..value_end].trim();
                    if !value.is_empty() {
                        extracted.push(((*name).to_string(), value.to_string()));
                    }

                    cursor = value_end + quote.len_utf8();
                }
            }
        }

        extracted
    }

    fn strip_parameter_fragments(raw: &str) -> String {
        let raw = Self::strip_xml_comments(raw);
        let mut output = String::new();
        let mut cursor = 0usize;
        let open_markers = ["<parameter name=\"", "</parameter name=\""];
        let close = "</parameter>";

        while let Some((start, marker)) = open_markers
            .iter()
            .filter_map(|marker| raw[cursor..].find(marker).map(|idx| (cursor + idx, *marker)))
            .min_by_key(|(idx, _)| *idx)
        {
            output.push_str(&raw[cursor..start]);
            let name_start = start + marker.len();
            let Some(name_end_rel) = raw[name_start..].find("\">") else {
                return output.trim().to_string();
            };
            let value_start = name_start + name_end_rel + 2;
            let next_parameter_start = open_markers
                .iter()
                .filter_map(|marker| raw[value_start..].find(marker).map(|idx| value_start + idx))
                .min();
            let closing_parameter_end = raw[value_start..]
                .find(close)
                .map(|idx| value_start + idx + close.len());

            cursor = match (closing_parameter_end, next_parameter_start) {
                (Some(close_idx), Some(next_idx)) if next_idx < close_idx => next_idx,
                (Some(close_idx), _) => close_idx,
                (None, Some(next_idx)) => next_idx,
                (None, None) => raw.len(),
            };
        }

        output.push_str(&raw[cursor..]);
        output.replace("</parameter>", "").trim().to_string()
    }

    fn parse_task_status(raw: &str) -> Option<crate::TaskStatus> {
        let normalized = raw
            .trim()
            .to_ascii_lowercase()
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
            .collect::<String>();
        let words = normalized.split_whitespace().collect::<Vec<_>>();
        let joined = words.join("_");

        match joined.as_str() {
            "notstarted" | "not_started" | "todo" => return Some(crate::TaskStatus::NotStarted),
            "blocked" | "waiting" => return Some(crate::TaskStatus::Blocked),
            "inprogress" | "in_progress" | "started" | "running" | "active" => {
                return Some(crate::TaskStatus::InProgress);
            }
            "completed" | "complete" | "done" | "finished" => {
                return Some(crate::TaskStatus::Completed);
            }
            "cancelled" | "canceled" => return Some(crate::TaskStatus::Cancelled),
            _ => {}
        }

        for window in words.windows(2) {
            match window {
                ["not", "started"] => return Some(crate::TaskStatus::NotStarted),
                ["in", "progress"] => return Some(crate::TaskStatus::InProgress),
                _ => {}
            }
        }

        for word in words {
            match word {
                "todo" | "notstarted" => return Some(crate::TaskStatus::NotStarted),
                "blocked" | "waiting" => return Some(crate::TaskStatus::Blocked),
                "inprogress" | "started" | "running" | "active" => {
                    return Some(crate::TaskStatus::InProgress);
                }
                "completed" | "complete" | "done" | "finished" => {
                    return Some(crate::TaskStatus::Completed);
                }
                "cancelled" | "canceled" => return Some(crate::TaskStatus::Cancelled),
                _ => {}
            }
        }

        None
    }

    fn extract_embedded_task_status(raw: &str) -> Option<crate::TaskStatus> {
        let lowered =
            Self::strip_parameter_fragments(&Self::strip_xml_comments(raw)).to_ascii_lowercase();

        for marker in [
            "status is",
            "status:",
            "status=",
            "state is",
            "state:",
            "state=",
            "new status is",
            "new status:",
            "target status is",
            "target status:",
            "mark as",
            "set to",
        ] {
            if let Some(start) = lowered.find(marker) {
                let tail = lowered[start + marker.len()..].trim();
                if let Some(status) = Self::parse_task_status(tail) {
                    return Some(status);
                }
            }
        }

        None
    }

    fn shell_output_looks_interactive(output: &str) -> bool {
        let normalized = output.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }

        [
            "ok to proceed?",
            "need to install the following packages",
            "would you like to continue",
            "press enter to continue",
            "select an option",
            "(y/n)",
            "[y/n]",
            "yes/no",
            "confirm",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
    }

    fn format_shell_failure(exit_code: i32, stdout: &str, stderr: &str) -> String {
        let stdout = stdout.trim_end();
        let stderr = stderr.trim_end();
        let mut sections = Vec::new();
        if !stdout.is_empty() {
            sections.push(format!("stdout:\n{}", stdout));
        }
        if !stderr.is_empty() {
            sections.push(format!("stderr:\n{}", stderr));
        }
        let combined = sections.join("\n\n");

        if exit_code == 124 && Self::shell_output_looks_interactive(&combined) {
            if combined.is_empty() {
                return "Exit 124: Command likely waited for interactive input, but the shell tool is non-interactive and cannot answer prompts. Retry with unattended flags such as `-y`, `--yes`, `CI=1`, or an equivalent non-interactive mode, or ask the user for confirmation.".to_string();
            }

            return format!(
                "Exit 124: Command likely waited for interactive input, but the shell tool is non-interactive and cannot answer prompts. Retry with unattended flags such as `-y`, `--yes`, `CI=1`, or an equivalent non-interactive mode, or ask the user for confirmation.\n\n{}",
                combined
            );
        }

        if combined.is_empty() {
            if exit_code == 124 {
                "Exit 124: Command timed out".to_string()
            } else {
                format!("Exit {exit_code}")
            }
        } else {
            format!("Exit {exit_code}:\n{combined}")
        }
    }

    fn normalize_task_tool_arguments(args: serde_json::Value) -> serde_json::Value {
        let mut obj = match args {
            serde_json::Value::Object(map) => map,
            other => return other,
        };

        let raw_string_values = obj
            .values()
            .filter_map(|value| value.as_str().map(ToOwned::to_owned))
            .collect::<Vec<_>>();

        let mut recovered = serde_json::Map::new();
        for value in obj.values() {
            if let Some(raw) = value.as_str() {
                for (name, content) in Self::extract_parameter_fragments(raw) {
                    recovered
                        .entry(name)
                        .or_insert_with(|| serde_json::Value::String(content));
                }

                for (name, content) in
                    Self::extract_named_attribute_fragments(raw, &["name", "description", "status"])
                {
                    recovered
                        .entry(name)
                        .or_insert_with(|| serde_json::Value::String(content));
                }
            }
        }

        for (key, value) in recovered {
            obj.entry(key).or_insert(value);
        }

        for key in ["operation", "name", "description", "status"] {
            let Some(raw) = obj.get(key).and_then(|value| value.as_str()) else {
                continue;
            };

            let stripped = Self::strip_parameter_fragments(raw);
            match Self::normalize_optional_tool_string(&stripped) {
                Some(value) => {
                    obj.insert(key.to_string(), serde_json::Value::String(value));
                }
                None => {
                    obj.remove(key);
                }
            }
        }

        for key in ["task_id", "parent_id"] {
            let Some(raw) = obj.get(key).and_then(|value| value.as_str()) else {
                continue;
            };

            let stripped = Self::strip_parameter_fragments(raw);
            match Self::normalize_task_reference(&stripped) {
                Some(value) => {
                    obj.insert(key.to_string(), serde_json::Value::String(value));
                }
                None => {
                    obj.remove(key);
                }
            }
        }

        let parsed_status = obj
            .get("status")
            .and_then(|value| value.as_str())
            .and_then(Self::parse_task_status);

        if parsed_status.is_none() {
            if obj.get("status").is_some() {
                obj.remove("status");
            }

            if let Some(recovered_status) = raw_string_values
                .iter()
                .find_map(|raw| Self::extract_embedded_task_status(raw))
            {
                obj.insert(
                    "status".to_string(),
                    serde_json::Value::String(recovered_status.to_string()),
                );
            }
        }

        serde_json::Value::Object(obj)
    }

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

        if let Some(message) = Self::repeated_malformed_tool_call_skip_message(
            &pending.name,
            &pending.arguments,
            response.tool_calls.iter(),
        ) {
            tracing::warn!(
                tool = %pending.name,
                "[ToolDispatch] Loop breaker skipped repeated malformed tool call"
            );
            let duration_ms = pending.start_time.elapsed().as_millis() as u64;
            let _ = tx
                .send(StreamChunk::ToolCallResult {
                    name: pending.name.clone(),
                    success: false,
                    output: message.clone(),
                    duration_ms,
                })
                .await;

            let record = ToolCallRecord {
                id: pending.id,
                name: pending.name,
                arguments: pending.arguments,
                result: ToolResult::Skipped(message),
                duration_ms,
            };
            tool_calls_in_iteration.push(record.clone());
            response.tool_calls.push(record);
            return;
        }

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
                                ToolResult::Error(Self::format_shell_failure(
                                    r.exit_code,
                                    &r.stdout,
                                    &r.stderr,
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
                        Ok(result) => {
                            if result.success {
                                ToolResult::Success(result.stdout)
                            } else {
                                ToolResult::Error(Self::format_shell_failure(
                                    result.exit_code,
                                    &result.stdout,
                                    &result.stderr,
                                ))
                            }
                        }
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
                                ToolResult::Error(Self::format_shell_failure(
                                    r.exit_code,
                                    &r.stdout,
                                    &r.stderr,
                                ))
                            }
                        }
                        Err(e) => ToolResult::Error(e.to_string()),
                    }
                } else {
                    match shell_async::execute_command(arguments, cwd.as_deref()).await {
                        Ok(result) => {
                            if result.success {
                                ToolResult::Success(result.stdout)
                            } else {
                                ToolResult::Error(Self::format_shell_failure(
                                    result.exit_code,
                                    &result.stdout,
                                    &result.stderr,
                                ))
                            }
                        }
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
                let args = Self::normalize_file_tool_arguments(args);
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
                                    Self::format_missing_file_write_content_error(&args),
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
                let args = Self::normalize_task_tool_arguments(args);
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("list");

                match operation {
                    "create" => {
                        let description = args
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let name = args
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|name| !name.is_empty());
                        let Some(name) = name else {
                            return ToolResult::Error(Self::format_missing_task_create_name_error(
                                &args,
                            ));
                        };
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
                        let explicit_status = args
                            .get("status")
                            .or_else(|| args.get("state"))
                            .or_else(|| args.get("new_status"))
                            .or_else(|| args.get("target_status"))
                            .and_then(|v| v.as_str())
                            .and_then(Self::parse_task_status);

                        match explicit_status {
                            Some(status) => match manager.update_task_status(session_id, task_id, status) {
                                Ok(_) => ToolResult::Success(format!(
                                    "Updated task {} status to {:?}",
                                    task_id, status
                                )),
                                Err(e) => {
                                    ToolResult::Error(format!("Failed to update task status: {}", e))
                                }
                            },
                            None => ToolResult::Error(
                                Self::format_missing_task_update_status_error(&args),
                            ),
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
            if Self::should_echo_tool_call_arguments(tool_call) {
                let truncated_args = self.truncate_tool_result(&tool_call.arguments);
                prompt.push_str(&format!(
                    "\nTool {} call:\nArguments: {}\nResult: {}\n",
                    tool_call.name, truncated_args, result_text
                ));
            } else {
                prompt.push_str(&format!(
                    "\nTool {} result:\n{}\n",
                    tool_call.name, result_text
                ));
            }
        }

        let had_task_tool_error = tool_calls.iter().any(|tool_call| {
            tool_call.name == "task" && matches!(tool_call.result, ToolResult::Error(_))
        });

        prompt.push_str(
            "\nUser: Based on the tool results above, continue the original request end-to-end. \
             Synthesize the information, execute any remaining necessary steps, and only mark \
             tasks as completed if the requested deliverable is actually finished and any planned \
             verification has succeeded. Otherwise, keep task status accurate and continue working.\n",
        );

        if had_task_tool_error {
            prompt.push_str(
                "Important: task-tracking errors must not block implementation work. If a task operation fails, continue the real work with file/shell/code tools and only retry the task tool when you can provide the exact required fields. For `create`, provide a specific `name` and preferably a concrete `description`; for `update_status`, always include both `task_id` and `status`.\n",
            );
        }

        let had_task_tool_success = tool_calls.iter().any(|tool_call| {
            tool_call.name == "task" && matches!(tool_call.result, ToolResult::Success(_))
        });

        if had_task_tool_success {
            prompt.push_str(
                "Important: a successful task update is only bookkeeping. Do not repeat the same task update just to confirm it. After a task update succeeds, continue with the next concrete implementation or verification step unless another task's status genuinely changed.\n",
            );
        }

        let had_missing_task_update_status_error = tool_calls.iter().any(|tool_call| {
            tool_call.name == "task"
                && matches!(
                    &tool_call.result,
                    ToolResult::Error(message)
                        if message.contains("Missing required field 'status' for update_status operation")
                )
        });

        if had_missing_task_update_status_error {
            prompt.push_str(
                "Important: if `task.update_status` failed because `status` was omitted, treat that as malformed arguments, not a reason to keep looping on task bookkeeping. The malformed task-update arguments are echoed above. If you intended a status change, send one corrected `update_status` call with both `task_id` and `status`; otherwise continue the real work now.\n",
            );
        }

        let had_missing_task_create_name_error = tool_calls.iter().any(|tool_call| {
            tool_call.name == "task"
                && matches!(
                    &tool_call.result,
                    ToolResult::Error(message)
                        if message.contains("Missing required field 'name' for create operation")
                )
        });

        if had_missing_task_create_name_error {
            prompt.push_str(
                "Important: if `task.create` failed because `name` was missing, treat that as malformed arguments. The malformed task-create arguments are echoed above. Retry only with a specific task `name` and, for non-trivial work, a concrete `description`; do not rely on placeholder tasks.\n",
            );
        }

        let had_task_loop_breaker_skip = tool_calls.iter().any(|tool_call| {
            tool_call.name == "task"
                && matches!(
                    &tool_call.result,
                    ToolResult::Skipped(message) if message.contains("Loop breaker:")
                )
        });

        if had_task_loop_breaker_skip {
            prompt.push_str(
                "Important: repeated malformed task bookkeeping calls triggered the loop breaker. Treat task tracking as temporarily disabled for this run and continue the real implementation/build/test work with file/shell/code tools instead of calling `task` again.\n",
            );
        }

        let had_file_tool_error = tool_calls.iter().any(|tool_call| {
            tool_call.name == "file" && matches!(tool_call.result, ToolResult::Error(_))
        });

        if had_file_tool_error {
            prompt.push_str(
                "Important: file-tool errors must not block implementation work. For `write`, always include the destination `path` and the full file `content` (aliases like `contents` or `text` are okay, but `pattern`/`start` do not make a valid write). For partial updates, use `edit` with `path`, `old`, and `new`; reserve `pattern` for `search`.\n",
            );
        }

        let had_missing_file_write_content_error = tool_calls.iter().any(|tool_call| {
            tool_call.name == "file"
                && matches!(
                    &tool_call.result,
                    ToolResult::Error(message)
                        if message.contains("Missing required field 'content' for file write operation")
                )
        });

        if had_missing_file_write_content_error {
            prompt.push_str(
                "Important: the malformed file-write arguments are echoed above. Do not repeat the same `write` call unchanged. If you do not yet know the full destination file text, read the existing file or prepare the full content first, then send one corrected `write` call with real `content`; otherwise use `edit` for a targeted change. Placeholders like `pattern: \"none\"` or `pattern: \"full content\"` are invalid.\n",
            );
        }

        let had_tool_loop_breaker_skip = tool_calls.iter().any(|tool_call| {
            matches!(
                &tool_call.result,
                ToolResult::Skipped(message) if message.contains("Loop breaker:")
            )
        });

        if had_tool_loop_breaker_skip {
            prompt.push_str(
                "Important: a loop breaker blocked a repeated malformed tool call, but the agent run is still active. Do not retry the blocked malformed call shape again in this turn. Choose a different next step such as reading the file, preparing the missing content, using a more appropriate tool operation, or asking the user one focused question if essential information is still missing.\n",
            );
        }

        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::AgentPipeline;
    use crate::config::AppConfig;
    use crate::session_workspace::SessionWorkspace;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn normalize_task_tool_arguments_recovers_embedded_parameter_fragments() {
        let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
            "operation": "create",
            "parent_id": "None",
            "status": "notstarted",
            "task_id": "None</parameter><parameter name=\"name\">Create Tauri Hello World GUI Application</parameter>\n<parameter name=\"description\">Plan, implement, build, and test a small Tauri v2 GUI that displays \"Hello World\"</parameter>",
        }));

        assert_eq!(
            normalized.get("operation").and_then(|v| v.as_str()),
            Some("create")
        );
        assert_eq!(normalized.get("parent_id"), None);
        assert_eq!(normalized.get("task_id"), None);
        assert_eq!(
            normalized.get("name").and_then(|v| v.as_str()),
            Some("Create Tauri Hello World GUI Application")
        );
        assert_eq!(
            normalized.get("description").and_then(|v| v.as_str()),
            Some(
                "Plan, implement, build, and test a small Tauri v2 GUI that displays \"Hello World\""
            )
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
    fn normalize_task_tool_arguments_recovers_unclosed_status_fragment() {
        let normalized = AgentPipeline::normalize_task_tool_arguments(json!({
            "operation": "update_status",
            "task_id": "1c0a1ed3-e355-4117-9881-3632a2765199\"  <!-- Install Tauri prerequisites -->\n<parameter name=\"status\">inprogress",
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
    fn normalize_file_tool_arguments_sanitizes_paths_and_aliases() {
        let normalized = AgentPipeline::normalize_file_tool_arguments(json!({
            "operation": "EDIT",
            "path": "\"src/index.html\"",
            "old_str": "<h1>Hello</h1>",
            "new_str": "<h1>Hello, Gestura</h1>",
        }));

        assert_eq!(
            normalized.get("operation").and_then(|v| v.as_str()),
            Some("edit")
        );
        assert_eq!(
            normalized.get("path").and_then(|v| v.as_str()),
            Some("src/index.html")
        );
        assert_eq!(
            normalized.get("old").and_then(|v| v.as_str()),
            Some("<h1>Hello</h1>")
        );
        assert_eq!(
            normalized.get("new").and_then(|v| v.as_str()),
            Some("<h1>Hello, Gestura</h1>")
        );
    }

    #[test]
    fn normalize_file_tool_arguments_recovers_write_content_aliases() {
        let normalized = AgentPipeline::normalize_file_tool_arguments(json!({
            "operation": "write",
            "path": "src/index.html",
            "text": "<h1>Hello, Gestura</h1>",
        }));

        assert_eq!(
            normalized.get("content").and_then(|v| v.as_str()),
            Some("<h1>Hello, Gestura</h1>")
        );
    }

    #[test]
    fn missing_file_write_content_error_explains_how_to_recover() {
        let message = AgentPipeline::format_missing_file_write_content_error(&json!({
            "operation": "write",
            "path": "tauri-hello-world/src/index.html",
            "pattern": "none",
            "start": 1,
        }));

        assert!(message.contains("Missing required field 'content' for file write operation"));
        assert!(message.contains("pattern, start"));
        assert!(message.contains("\"content\":\"<full file contents here>\""));
        assert!(message.contains("Do not retry the same malformed `write` call"));
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
        assert!(message.contains("Do not omit `status` to ask the runtime to infer or preserve the current state"));
    }

    #[test]
    fn missing_task_create_name_error_explains_how_to_recover() {
        let message = AgentPipeline::format_missing_task_create_name_error(&json!({
            "operation": "create",
            "task_id": "oops",
        }));

        assert!(message.contains("Missing required field 'name' for create operation"));
        assert!(message.contains("`create` requires a specific task `name`"));
        assert!(message.contains("\"name\":\"Build hello world Tauri app\""));
        assert!(message.contains("Do not rely on the runtime to invent placeholder names like 'Untitled Task'"));
    }

    #[test]
    fn repeated_malformed_tool_call_skip_message_trips_on_third_attempt() {
        let malformed_args = json!({
            "operation": "write",
            "path": "tauri-hello-world/src/index.html",
            "pattern": "none",
            "start": 1,
        })
        .to_string();

        let prior_records = vec![
            crate::pipeline::ToolCallRecord {
                id: "1".to_string(),
                name: "file".to_string(),
                arguments: malformed_args.clone(),
                result: crate::pipeline::ToolResult::Error(
                    AgentPipeline::format_missing_file_write_content_error(&json!({
                        "operation": "write",
                        "path": "tauri-hello-world/src/index.html",
                        "pattern": "none",
                        "start": 1,
                    })),
                ),
                duration_ms: 1,
            },
            crate::pipeline::ToolCallRecord {
                id: "2".to_string(),
                name: "file".to_string(),
                arguments: malformed_args.clone(),
                result: crate::pipeline::ToolResult::Error(
                    AgentPipeline::format_missing_file_write_content_error(&json!({
                        "operation": "write",
                        "path": "tauri-hello-world/src/index.html",
                        "pattern": "full content",
                        "start": 1,
                    })),
                ),
                duration_ms: 1,
            },
        ];

        let message = AgentPipeline::repeated_malformed_tool_call_skip_message(
            "file",
            &malformed_args,
            prior_records.iter(),
        )
        .expect("loop breaker should trigger");

        assert!(message.contains("Loop breaker:"));
        assert!(message.contains("agent is still running"));
        assert!(message.contains(
            "Do not retry `write` until you can provide the full destination file text in `content`"
        ));
    }

    #[test]
    fn repeated_malformed_tool_call_skip_message_does_not_trip_too_early() {
        let malformed_args = json!({
            "operation": "write",
            "path": "tauri-hello-world/src/index.html",
            "pattern": "none",
            "start": 1,
        })
        .to_string();

        let prior_records = vec![crate::pipeline::ToolCallRecord {
            id: "1".to_string(),
            name: "file".to_string(),
            arguments: malformed_args.clone(),
            result: crate::pipeline::ToolResult::Error(
                AgentPipeline::format_missing_file_write_content_error(&json!({
                    "operation": "write",
                    "path": "tauri-hello-world/src/index.html",
                    "pattern": "none",
                    "start": 1,
                })),
            ),
            duration_ms: 1,
        }];

        assert!(
            AgentPipeline::repeated_malformed_tool_call_skip_message(
                "file",
                &malformed_args,
                prior_records.iter(),
            )
            .is_none()
        );
    }

    #[test]
    fn repeated_malformed_task_update_without_status_trips_on_third_attempt() {
        let malformed_args = json!({
            "operation": "update_status",
            "task_id": "6073f304-388d-408c-82d0-f49f8679656a",
        })
        .to_string();

        let prior_records = vec![
            crate::pipeline::ToolCallRecord {
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
            },
            crate::pipeline::ToolCallRecord {
                id: "2".to_string(),
                name: "task".to_string(),
                arguments: malformed_args.clone(),
                result: crate::pipeline::ToolResult::Error(
                    AgentPipeline::format_missing_task_update_status_error(&json!({
                        "operation": "update_status",
                        "task_id": "6073f304-388d-408c-82d0-f49f8679656a",
                    })),
                ),
                duration_ms: 1,
            },
        ];

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
    fn repeated_malformed_task_create_without_name_trips_on_third_attempt() {
        let malformed_args = json!({
            "operation": "create",
            "task_id": "\"tauri-hello-world\" wait no, task_id not for create.",
        })
        .to_string();

        let prior_records = vec![
            crate::pipeline::ToolCallRecord {
                id: "1".to_string(),
                name: "task".to_string(),
                arguments: malformed_args.clone(),
                result: crate::pipeline::ToolResult::Error(
                    AgentPipeline::format_missing_task_create_name_error(&json!({
                        "operation": "create",
                        "task_id": "\"tauri-hello-world\" wait no, task_id not for create.",
                    })),
                ),
                duration_ms: 1,
            },
            crate::pipeline::ToolCallRecord {
                id: "2".to_string(),
                name: "task".to_string(),
                arguments: malformed_args.clone(),
                result: crate::pipeline::ToolResult::Error(
                    AgentPipeline::format_missing_task_create_name_error(&json!({
                        "operation": "create",
                        "task_id": "\"tauri-hello-world\" wait no, task_id not for create.",
                    })),
                ),
                duration_ms: 1,
            },
        ];

        let message = AgentPipeline::repeated_malformed_tool_call_skip_message(
            "task",
            &malformed_args,
            &prior_records,
        )
        .expect("third malformed create should trip loop breaker");

        assert!(message.contains("Loop breaker:"));
        assert!(message.contains("task.create"));
        assert!(message.contains("without a valid `name`"));
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
    async fn update_status_without_status_returns_actionable_error() {
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
        assert!(output.contains("\"status\":\"inprogress\""));

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

        let before = manager.list_tasks(&session_id).expect("list tasks before").len();

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

        let after = manager.list_tasks(&session_id).expect("list tasks after").len();
        assert_eq!(before, after);
    }

    #[tokio::test]
    async fn update_status_without_status_does_not_change_existing_in_progress_task() {
        let temp = TempDir::new().expect("temp dir");
        let session_id = format!("tool-dispatch-noop-test-{}", uuid::Uuid::new_v4());
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

        let task_after = manager
            .get_task(&session_id, &task.id)
            .expect("get task")
            .expect("task exists");
        assert_eq!(task_after.status, crate::TaskStatus::InProgress);
    }

    #[tokio::test]
    async fn update_status_without_status_does_not_change_completed_task() {
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

        let task_after = manager
            .get_task(&session_id, &task.id)
            .expect("get task")
            .expect("task exists");
        assert_eq!(task_after.status, crate::TaskStatus::Completed);
    }

    #[tokio::test]
    async fn file_edit_accepts_old_str_new_str_aliases() {
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
                assert!(output.contains("replacements"));
                let updated = std::fs::read_to_string(&file_path).expect("read updated file");
                assert!(updated.contains("Hello, Gestura"));
            }
            other => panic!("expected success, got {other:?}"),
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
                        "{}\"  <!-- Install Tauri prerequisites -->\n<parameter name=\"status\">inprogress",
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
    async fn file_write_accepts_text_alias_for_content() {
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
                let updated = std::fs::read_to_string(&file_path).expect("read updated file");
                assert!(updated.contains("Hello, Gestura"));
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
                assert!(
                    message.contains("Missing required field 'content' for file write operation")
                );
                assert!(message.contains("pattern, start"));
                assert!(message.contains("\"content\":\"<full file contents here>\""));
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
        assert!(prompt.contains("For `create`, provide a specific `name` and preferably a concrete `description`"));
        assert!(prompt.contains("always include both `task_id` and `status`"));
        assert!(prompt.contains("Arguments: {\"operation\":\"update_status\",\"task_id\":\"abc\"}"));
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

        assert!(prompt.contains("if `task.update_status` failed because `status` was omitted"));
        assert!(prompt.contains("not a reason to keep looping on task bookkeeping"));
        assert!(prompt.contains("Arguments: {\"operation\":\"update_status\",\"task_id\":\"abc\"}"));
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

        assert!(prompt.contains("if `task.create` failed because `name` was missing"));
        assert!(prompt.contains("Retry only with a specific task `name`"));
        assert!(prompt.contains("Arguments: {\"operation\":\"create\",\"task_id\":\"abc\"}"));
    }

    #[test]
    fn continuation_prompt_warns_task_tool_is_temporarily_disabled_after_loop_breaker() {
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
                    "Loop breaker: skipped a repeated malformed `task.create` call without a valid `name` after 2 prior similar malformed attempts in this run."
                        .to_string(),
                ),
                duration_ms: 1,
            }],
        );

        assert!(prompt.contains("task tracking as temporarily disabled for this run"));
        assert!(prompt.contains("continue the real implementation/build/test work with file/shell/code tools"));
    }

    #[test]
    fn continuation_prompt_echoes_missing_status_task_error_arguments() {
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

        assert!(prompt.contains("Tool task call:"));
        assert!(prompt.contains("\"operation\":\"update_status\""));
        assert!(prompt.contains("\"task_id\":\"6073f304-388d-408c-82d0-f49f8679656a\""));
        assert!(prompt.contains("Missing required field 'status' for update_status operation"));
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
                    "path": "tauri-hello-world/src/index.html",
                    "pattern": "none",
                    "start": 1,
                })
                .to_string(),
                result: crate::pipeline::ToolResult::Error(
                    AgentPipeline::format_missing_file_write_content_error(&json!({
                        "operation": "write",
                        "path": "tauri-hello-world/src/index.html",
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
        assert!(prompt.contains("Do not repeat the same `write` call unchanged"));
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
                    "path": "tauri-hello-world/src/index.html",
                    "pattern": "none",
                    "start": 1,
                })
                .to_string(),
                result: crate::pipeline::ToolResult::Skipped(
                    "Loop breaker: skipped a repeated malformed `file.write` call without `content` after 2 prior similar non-successful attempts in this run. The agent is still running.".to_string(),
                ),
                duration_ms: 1,
            }],
        );

        assert!(prompt.contains("loop breaker blocked a repeated malformed tool call"));
        assert!(prompt.contains("agent run is still active"));
        assert!(
            prompt.contains("Do not retry the blocked malformed call shape again in this turn")
        );
    }

    #[test]
    fn shell_failure_format_detects_interactive_timeout_prompts() {
        let message = AgentPipeline::format_shell_failure(
            124,
            "Need to install the following packages:\ncreate-tauri-app@4.6.2\nOk to proceed? (y)\n",
            "",
        );

        assert!(message.contains("likely waited for interactive input"));
        assert!(message.contains("shell tool is non-interactive"));
        assert!(message.contains("Ok to proceed? (y)"));
    }

    #[tokio::test]
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
                    "command": "printf 'Need to install the following packages:\ncreate-tauri-app@4.6.2\nOk to proceed? (y)\n'; sleep 2",
                    "timeout_secs": 1,
                })
                .to_string(),
                Some(&workspace),
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
}
