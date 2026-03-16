use super::*;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

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
    CodeBatchEditMissingEdits,
    TaskCreateMissingName,
    TaskUpdateStatusMissingExplicitStatus,
}

impl AgentPipeline {
    fn title_case_slug(raw: &str) -> Option<String> {
        let words = raw
            .split(|c: char| matches!(c, '-' | '_' | ' '))
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();

        if words.is_empty() || words.len() > 8 {
            return None;
        }

        Some(
            words
                .into_iter()
                .map(|word| {
                    let mut chars = word.chars();
                    let Some(first) = chars.next() else {
                        return String::new();
                    };

                    let mut formatted = first.to_ascii_uppercase().to_string();
                    formatted.push_str(&chars.as_str().to_ascii_lowercase());
                    formatted
                })
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    fn recover_task_create_name(raw: &str) -> Option<String> {
        let candidate = Self::normalize_task_reference(raw)?;
        if uuid::Uuid::parse_str(&candidate).is_ok()
            || candidate.len() > 80
            || candidate.contains('/')
            || !candidate.chars().any(|ch| ch.is_ascii_alphabetic())
        {
            return None;
        }

        let slug_like = candidate
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ' '));

        if slug_like {
            return Self::title_case_slug(&candidate);
        }

        None
    }

    fn derive_task_name_from_description(raw: &str) -> Option<String> {
        let description = Self::normalize_optional_tool_string(raw)?;
        let sentence = description
            .split(['\n', '.', '!', '?'])
            .next()
            .unwrap_or(description.as_str())
            .trim();
        if sentence.is_empty() || sentence.len() > 80 {
            return None;
        }
        Some(sentence.to_string())
    }

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

    fn looks_like_full_file_content(raw: &str) -> bool {
        let trimmed = raw.trim();
        trimmed.contains('\n') || trimmed.len() >= 120
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

    fn normalize_tool_operation(raw: &str) -> Option<String> {
        let stripped = Self::strip_parameter_fragments(raw);
        let trimmed = stripped
            .trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
            .trim();

        if trimmed.is_empty() {
            return None;
        }

        let operation = trimmed
            .split_whitespace()
            .next()
            .unwrap_or(trimmed)
            .trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
            .to_ascii_lowercase();

        (!operation.is_empty()).then_some(operation)
    }

    fn workspace_path_suffix_components(path: &Path) -> Option<Vec<String>> {
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                std::path::Component::Normal(value) => {
                    components.push(value.to_string_lossy().to_string())
                }
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_) => return None,
            }
        }

        (!components.is_empty()).then_some(components)
    }

    fn path_ends_with_components(path: &Path, suffix: &[String]) -> bool {
        let components = path
            .components()
            .filter_map(|component| match component {
                std::path::Component::Normal(value) => Some(value.to_string_lossy().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();

        components.ends_with(suffix)
    }

    fn find_unique_workspace_suffix_match(
        root: &Path,
        suffix: &[String],
        directories_only: bool,
    ) -> Option<PathBuf> {
        fn visit(
            current: &Path,
            root: &Path,
            suffix: &[String],
            directories_only: bool,
            found: &mut Option<PathBuf>,
            ambiguous: &mut bool,
        ) {
            if *ambiguous {
                return;
            }

            let metadata = match fs::metadata(current) {
                Ok(metadata) => metadata,
                Err(_) => return,
            };
            let is_dir = metadata.is_dir();

            if (!directories_only || is_dir)
                && let Ok(relative) = current.strip_prefix(root)
                && !relative.as_os_str().is_empty()
                && AgentPipeline::path_ends_with_components(relative, suffix)
            {
                if found.is_some() {
                    *ambiguous = true;
                    return;
                }
                *found = Some(current.to_path_buf());
            }

            if !is_dir {
                return;
            }

            let entries = match fs::read_dir(current) {
                Ok(entries) => entries,
                Err(_) => return,
            };

            for entry in entries.flatten() {
                visit(
                    &entry.path(),
                    root,
                    suffix,
                    directories_only,
                    found,
                    ambiguous,
                );
                if *ambiguous {
                    return;
                }
            }
        }

        let mut found = None;
        let mut ambiguous = false;
        visit(
            root,
            root,
            suffix,
            directories_only,
            &mut found,
            &mut ambiguous,
        );

        (!ambiguous).then_some(found).flatten()
    }

    fn recover_workspace_read_path_by_suffix(
        ws: &SessionWorkspace,
        requested: &Path,
    ) -> Option<PathBuf> {
        let suffix = Self::workspace_path_suffix_components(requested)?;
        let candidate = Self::find_unique_workspace_suffix_match(ws.root(), &suffix, false)?;
        ws.resolve_path_for_read(candidate.strip_prefix(ws.root()).ok()?)
            .ok()
    }

    fn recover_workspace_write_path_by_suffix(
        ws: &SessionWorkspace,
        requested: &Path,
    ) -> Option<PathBuf> {
        let parent = requested.parent()?;
        if parent.as_os_str().is_empty() {
            return None;
        }

        let file_name = requested.file_name()?;
        let suffix = Self::workspace_path_suffix_components(parent)?;
        let parent_dir = Self::find_unique_workspace_suffix_match(ws.root(), &suffix, true)?;
        let candidate = parent_dir.join(file_name);
        ws.resolve_path_for_write(candidate.strip_prefix(ws.root()).ok()?)
            .ok()
    }

    fn resolve_workspace_read_path(ws: &SessionWorkspace, raw: &str) -> Result<PathBuf, String> {
        let requested = Path::new(raw);
        ws.resolve_path_for_read(requested).or_else(|err| {
            Self::recover_workspace_read_path_by_suffix(ws, requested)
                .ok_or_else(|| err.to_string())
        })
    }

    fn resolve_workspace_write_path(ws: &SessionWorkspace, raw: &str) -> Result<PathBuf, String> {
        let requested = Path::new(raw);
        ws.resolve_path_for_write(requested).or_else(|err| {
            Self::recover_workspace_write_path_by_suffix(ws, requested)
                .ok_or_else(|| err.to_string())
        })
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
            if let Some(normalized) = Self::normalize_tool_operation(operation) {
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

            if obj.get("content").is_none()
                && let Some(pattern) = obj.get("pattern").and_then(|value| value.as_str())
                && Self::looks_like_full_file_content(pattern)
            {
                obj.insert(
                    "content".to_string(),
                    serde_json::Value::String(pattern.to_string()),
                );
            }
        }

        serde_json::Value::Object(obj)
    }

    fn normalize_code_edit_entry(value: serde_json::Value) -> Option<serde_json::Value> {
        let mut obj = match value {
            serde_json::Value::Object(map) => map,
            _ => return None,
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

        if obj.get("path").is_none() {
            for alias in ["file", "file_path", "filepath", "target", "target_path"] {
                if let Some(value) = obj.get(alias).cloned() {
                    obj.insert("path".to_string(), value);
                    break;
                }
            }
        }

        if let Some(path) = obj.get("path").and_then(|value| value.as_str())
            && let Some(normalized) = Self::normalize_tool_path(path)
        {
            obj.insert("path".to_string(), serde_json::Value::String(normalized));
        }

        for (alias, canonical) in [
            ("old", "old_str"),
            ("find", "old_str"),
            ("search", "old_str"),
            ("target_text", "old_str"),
            ("new", "new_str"),
            ("replacement", "new_str"),
            ("replace", "new_str"),
            ("text", "new_str"),
            ("content", "new_str"),
            ("body", "new_str"),
            ("value", "new_str"),
        ] {
            if obj.get(canonical).is_none()
                && let Some(value) = obj.get(alias).cloned()
            {
                obj.insert(canonical.to_string(), value);
            }
        }

        Some(serde_json::Value::Object(obj))
    }

    fn normalize_code_edit_entries(value: serde_json::Value) -> Option<serde_json::Value> {
        match value {
            serde_json::Value::Array(entries) => Some(serde_json::Value::Array(
                entries
                    .into_iter()
                    .filter_map(Self::normalize_code_edit_entry)
                    .collect(),
            )),
            serde_json::Value::Object(_) => Self::normalize_code_edit_entry(value)
                .map(|entry| serde_json::Value::Array(vec![entry])),
            _ => None,
        }
    }

    fn normalize_code_tool_arguments(args: serde_json::Value) -> serde_json::Value {
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

        if let Some(operation) = obj.get("operation").and_then(|value| value.as_str())
            && let Some(normalized) = Self::normalize_tool_operation(operation)
        {
            let normalized = match normalized.as_str() {
                "edit" | "edits" | "replace" | "replace_all" => "batch_edit",
                "read_many" | "multi_read" | "multiread" => "batch_read",
                other => other,
            };
            obj.insert(
                "operation".to_string(),
                serde_json::Value::String(normalized.to_string()),
            );
        }

        if let Some(path) = obj.get("path").and_then(|value| value.as_str())
            && let Some(normalized) = Self::normalize_tool_path(path)
        {
            obj.insert("path".to_string(), serde_json::Value::String(normalized));
        }

        if obj.get("pattern").is_none() {
            for alias in ["query", "regex", "glob_pattern"] {
                if let Some(value) = obj.get(alias).cloned() {
                    obj.insert("pattern".to_string(), value);
                    break;
                }
            }
        }

        if obj.get("paths").is_none() {
            for alias in ["files", "file_paths"] {
                if let Some(value) = obj.get(alias).cloned() {
                    obj.insert("paths".to_string(), value);
                    break;
                }
            }
        }

        let operation = obj
            .get("operation")
            .and_then(|value| value.as_str())
            .unwrap_or("stats")
            .to_string();

        if operation == "batch_read"
            && obj.get("paths").is_none()
            && let Some(path) = obj.get("path").cloned()
        {
            obj.insert("paths".to_string(), serde_json::Value::Array(vec![path]));
        }

        let edits_source = obj
            .get("edits")
            .cloned()
            .or_else(|| obj.get("changes").cloned())
            .or_else(|| obj.get("operations").cloned())
            .or_else(|| obj.get("replacements").cloned())
            .or_else(|| {
                (operation == "batch_edit"
                    && obj.get("path").is_some()
                    && (obj.get("old_str").is_some() || obj.get("old").is_some())
                    && (obj.get("new_str").is_some()
                        || obj.get("new").is_some()
                        || obj.get("replacement").is_some()))
                .then(|| serde_json::Value::Object(obj.clone()))
            });

        if let Some(edits_value) = edits_source
            && let Some(normalized) = Self::normalize_code_edit_entries(edits_value)
        {
            obj.insert("edits".to_string(), normalized);
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

    fn format_missing_code_batch_edit_edits_error(args: &serde_json::Value) -> String {
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
            "operation": "batch_edit",
            "edits": [{
                "path": "src/index.html",
                "old_str": "<h1>Hello</h1>",
                "new_str": "<h1>Hello World</h1>"
            }]
        })
        .to_string();

        format!(
            "Missing required field 'edits' for code batch_edit operation. `batch_edit` requires an `edits` array even for a single change, and each edit must include `path`, `old_str`, and `new_str`. Provided fields: {provided_fields}. Retry with {example}. Prefer the canonical `edits` array shape on the next attempt; if you use aliases like `changes`, ensure each entry still resolves to `path`, `old_str`, and `new_str`."
        )
    }

    #[cfg(test)]
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
            "code" => {
                let args = serde_json::from_str::<serde_json::Value>(arguments).ok()?;
                let args = Self::normalize_code_tool_arguments(args);
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("stats");

                let batch_edits = args
                    .get("edits")
                    .and_then(|v| v.as_array())
                    .map(|entries| {
                        entries
                            .iter()
                            .filter(|entry| {
                                entry.get("path").and_then(|v| v.as_str()).is_some()
                                    && entry.get("old_str").and_then(|v| v.as_str()).is_some()
                                    && entry.get("new_str").and_then(|v| v.as_str()).is_some()
                            })
                            .count()
                    })
                    .unwrap_or_default();

                if operation == "batch_edit" && batch_edits == 0 {
                    Some(RecoverableToolLoopPattern::CodeBatchEditMissingEdits)
                } else {
                    None
                }
            }
            "task" | "tasks" => {
                let args = serde_json::from_str::<serde_json::Value>(arguments).ok()?;
                let args = Self::normalize_task_tool_arguments(args);
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("list");
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
        let record_pattern =
            Self::detect_recoverable_tool_loop_pattern(&record.name, &record.arguments);
        if record_pattern.as_ref() != Some(pattern) {
            return false;
        }

        match pattern {
            RecoverableToolLoopPattern::FileWriteMissingContent => {
                matches!(record.result, ToolResult::Error(_) | ToolResult::Skipped(_))
            }
            RecoverableToolLoopPattern::CodeBatchEditMissingEdits => {
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
            RecoverableToolLoopPattern::CodeBatchEditMissingEdits => format!(
                "Loop breaker: skipped a repeated malformed `code.batch_edit` call without a valid `edits` array after {prior_attempts} prior similar malformed attempts in this run. The agent is still running. Do not retry `batch_edit` until you can provide `edits` as an array of objects with `path`, `old_str`, and `new_str`. If you only have one replacement, wrap it in a one-element `edits` array; otherwise choose a different next step such as reading the file, preparing the exact replacement strings, or using `file.edit` for a single targeted change."
            ),
            RecoverableToolLoopPattern::TaskCreateMissingName => format!(
                "Loop breaker: skipped a repeated malformed `task.create` call without a valid `name` after {prior_attempts} prior similar malformed attempts in this run. The agent is still running. Do not retry `create` without a specific task name. If you need task tracking, send one corrected `create` call with a concrete `name` and preferably a useful `description`; otherwise continue the real implementation work."
            ),
            RecoverableToolLoopPattern::TaskUpdateStatusMissingExplicitStatus => format!(
                "Loop breaker: skipped a repeated malformed `task.update_status` call without explicit `status` after {prior_attempts} prior similar malformed attempts in this run. The agent is still running. Do not retry `update_status` without `status`. If you intend a status change, send one corrected call with both `task_id` and `status`; otherwise continue the real implementation or verification work instead of repeating task bookkeeping."
            ),
        }
    }

    fn recoverable_tool_loop_skip_threshold(pattern: &RecoverableToolLoopPattern) -> usize {
        match pattern {
            RecoverableToolLoopPattern::FileWriteMissingContent
            | RecoverableToolLoopPattern::CodeBatchEditMissingEdits
            | RecoverableToolLoopPattern::TaskCreateMissingName
            | RecoverableToolLoopPattern::TaskUpdateStatusMissingExplicitStatus => 1,
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

        (prior_attempts >= Self::recoverable_tool_loop_skip_threshold(&pattern))
            .then(|| Self::build_recoverable_tool_loop_breaker_message(&pattern, prior_attempts))
    }

    fn extract_shell_command_from_arguments(arguments: &str) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(arguments)
            .ok()
            .and_then(|value| {
                value
                    .get("command")
                    .and_then(|command| command.as_str())
                    .map(str::to_string)
            })
            .or_else(|| (!arguments.trim().is_empty()).then(|| arguments.to_string()))
    }

    fn extract_file_operation_and_path(arguments: &str) -> Option<(String, String)> {
        let args = serde_json::from_str::<serde_json::Value>(arguments).ok()?;
        let args = Self::normalize_file_tool_arguments(args);
        let operation = args
            .get("operation")
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| {
                if args.get("content").is_some() {
                    "write"
                } else {
                    "read"
                }
            })
            .to_ascii_lowercase();
        let path = args
            .get("path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(".")
            .to_string();
        Some((operation, path))
    }

    fn is_tauri_init_command(command: &str) -> bool {
        let normalized = command.to_ascii_lowercase();
        normalized.contains("cargo tauri init") || normalized.contains("cargo-tauri init")
    }

    fn is_tauri_scaffold_command(command: &str) -> bool {
        let normalized = command.to_ascii_lowercase();
        normalized.contains("create-tauri-app") || Self::is_tauri_init_command(&normalized)
    }

    fn is_create_tauri_app_command(command: &str) -> bool {
        command.to_ascii_lowercase().contains("create-tauri-app")
    }

    fn is_create_tauri_app_help_command(command: &str) -> bool {
        let normalized = command.to_ascii_lowercase();
        Self::is_create_tauri_app_command(command)
            && (normalized.contains(" --help")
                || normalized.ends_with("--help")
                || normalized.contains(" -h"))
    }

    fn is_create_tauri_app_latest_command(command: &str) -> bool {
        command
            .to_ascii_lowercase()
            .contains("create-tauri-app@latest")
    }

    fn is_known_good_tauri_alternate_scaffold_command(command: &str) -> bool {
        let normalized = command.to_ascii_lowercase();
        Self::is_create_tauri_app_help_command(command)
            || (Self::is_create_tauri_app_command(command)
                && !Self::is_create_tauri_app_latest_command(command)
                && normalized.contains(" --yes")
                && normalized.contains("--template")
                && normalized.contains("--tauri-version"))
            || (Self::is_tauri_init_command(command) && normalized.contains(" --ci"))
    }

    fn is_manual_tauri_scaffold_shell_command(command: &str) -> bool {
        let normalized = command.to_ascii_lowercase();
        let targets_tauri_files = normalized.contains("src-tauri")
            || normalized.contains("tauri.conf.json")
            || normalized.contains("src/index.html")
            || normalized.contains("src/main.js")
            || normalized.contains("src/styles.css")
            || normalized.contains("package.json")
            || normalized.contains("cargo.toml");
        let uses_manual_shell_synthesis = normalized.contains("<<")
            || normalized.contains("cat >")
            || normalized.contains("tee ")
            || (normalized.contains("mkdir -p") && normalized.contains("src-tauri"))
            || (normalized.contains("cargo init") && normalized.contains("src-tauri"));

        targets_tauri_files
            && uses_manual_shell_synthesis
            && !Self::is_tauri_scaffold_command(command)
    }

    fn is_non_tty_tauri_scaffold_failure(record: &ToolCallRecord) -> bool {
        if record.name != "shell" {
            return false;
        }

        let Some(command) = Self::extract_shell_command_from_arguments(&record.arguments) else {
            return false;
        };
        if !Self::is_create_tauri_app_command(&command) {
            return false;
        }

        matches!(
            &record.result,
            ToolResult::Error(message)
                if {
                    let normalized = message.to_ascii_lowercase();
                    normalized.contains("not a terminal")
                        || normalized.contains("likely waited for interactive input")
                }
        )
    }

    fn count_non_tty_tauri_scaffold_failures<'a, I>(prior_records: I) -> usize
    where
        I: IntoIterator<Item = &'a ToolCallRecord>,
    {
        prior_records
            .into_iter()
            .filter(|record| Self::is_non_tty_tauri_scaffold_failure(record))
            .count()
    }

    fn build_tauri_scaffold_recovery_message(prior_failures: usize) -> String {
        format!(
            "Loop breaker: skipped a fallback shell scaffolding command after {prior_failures} prior non-interactive `create-tauri-app` failure(s) in this run. Do not synthesize a Tauri project with `cat <<EOF`, `tee`, `cargo init src-tauri`, or `mkdir`-only shell scripts. Use one specific alternate strategy next: run `npx create-tauri-app --help` (or `npx create-tauri-app@latest --help`) to confirm supported flags, then run a single non-interactive scaffold command like `npx create-tauri-app <app-dir> --yes --template vanilla --tauri-version 2`; if you already have a frontend project, use `cargo tauri init --ci ...` instead. After scaffold succeeds, move on to editing files and running build/test verification."
        )
    }

    fn extract_tauri_scaffold_target(command: &str) -> Option<String> {
        let tokens = command.split_whitespace().collect::<Vec<_>>();
        let scaffold_index = tokens
            .iter()
            .position(|token| token.to_ascii_lowercase().contains("create-tauri-app"))?;

        tokens
            .iter()
            .skip(scaffold_index + 1)
            .find(|token| !token.starts_with('-'))
            .map(|token| {
                token
                    .trim_matches(|ch| matches!(ch, '"' | '\''))
                    .to_string()
            })
            .filter(|token| !token.is_empty())
    }

    fn repeated_redundant_tool_call_skip_message<'a, I>(
        name: &str,
        arguments: &str,
        prior_records: I,
    ) -> Option<String>
    where
        I: IntoIterator<Item = &'a ToolCallRecord>,
    {
        if name == "shell" {
            let command = Self::extract_shell_command_from_arguments(arguments)?;
            let prior_records = prior_records.into_iter().collect::<Vec<_>>();

            let prior_non_tty_failures =
                Self::count_non_tty_tauri_scaffold_failures(prior_records.iter().copied());

            if prior_non_tty_failures >= 2 && Self::is_manual_tauri_scaffold_shell_command(&command)
            {
                return Some(Self::build_tauri_scaffold_recovery_message(
                    prior_non_tty_failures,
                ));
            }

            if prior_non_tty_failures >= 2
                && Self::is_create_tauri_app_command(&command)
                && !Self::is_known_good_tauri_alternate_scaffold_command(&command)
            {
                return Some(Self::build_tauri_scaffold_recovery_message(
                    prior_non_tty_failures,
                ));
            }

            if !Self::is_tauri_init_command(&command) {
                return None;
            }

            let prior_successes = prior_records
                .iter()
                .copied()
                .filter(|record| record.name == "shell")
                .filter(|record| matches!(record.result, ToolResult::Success(_)))
                .filter_map(|record| Self::extract_shell_command_from_arguments(&record.arguments))
                .filter(|prior_command| Self::is_tauri_init_command(prior_command))
                .count();

            return (prior_successes >= 1).then(|| {
                format!(
                    "Loop breaker: skipped a redundant repeated `cargo tauri init` scaffold command after {prior_successes} prior successful init command(s) in this run. The Tauri scaffold already exists. Do not run `cargo tauri init` again; move on to editing files and running build/test verification."
                )
            });
        }

        if name == "file" {
            let (operation, path) = Self::extract_file_operation_and_path(arguments)?;
            if !matches!(operation.as_str(), "list" | "tree") {
                return None;
            }

            let prior_successes = prior_records
                .into_iter()
                .filter(|record| record.name == "file")
                .filter(|record| matches!(record.result, ToolResult::Success(_)))
                .filter_map(|record| Self::extract_file_operation_and_path(&record.arguments))
                .filter(|(prior_operation, prior_path)| {
                    prior_operation == &operation && prior_path == &path
                })
                .count();

            return (prior_successes >= 2).then(|| {
                format!(
                    "Loop breaker: skipped a redundant repeated `file.{operation}` inspection of `{path}` after {prior_successes} prior successful identical inspections in this run. The scaffold is already visible. Do not keep inspecting the same path; move on to reading the specific file you need to edit, making the implementation change, or running build/test verification next."
                )
            });
        }

        None
    }

    fn harden_noninteractive_shell_command(
        command: &str,
        env: Option<HashMap<String, String>>,
    ) -> (String, Option<HashMap<String, String>>) {
        let normalized = command.to_ascii_lowercase();
        if !normalized.contains("create-tauri-app") {
            return (command.to_string(), env);
        }

        let mut hardened_command = command.trim().to_string();
        if !normalized.contains(" --yes") && !normalized.contains(" -y") {
            hardened_command.push_str(" --yes");
        }

        let mut hardened_env = env.unwrap_or_default();
        hardened_env
            .entry("CI".to_string())
            .or_insert_with(|| "true".to_string());
        hardened_env
            .entry("FORCE_COLOR".to_string())
            .or_insert_with(|| "0".to_string());

        (hardened_command, Some(hardened_env))
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
            .filter_map(|marker| {
                raw[cursor..]
                    .find(marker)
                    .map(|idx| (cursor + idx, *marker))
            })
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
            .filter_map(|marker| {
                raw[cursor..]
                    .find(marker)
                    .map(|idx| (cursor + idx, *marker))
            })
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

        if stderr.to_ascii_lowercase().contains("not a terminal") {
            let guidance = "Shell command failed because it expected a terminal. Retry with non-interactive flags and environment such as `--yes`, `CI=1`, and `FORCE_COLOR=0`, or choose a fully non-interactive command.";
            if combined.is_empty() {
                return format!("Exit {exit_code}: {guidance}");
            }

            return format!("Exit {exit_code}: {guidance}\n\n{combined}");
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

        if let Some(raw) = obj.get("operation").and_then(|value| value.as_str()) {
            if let Some(value) = Self::normalize_tool_operation(raw) {
                obj.insert("operation".to_string(), serde_json::Value::String(value));
            } else {
                obj.remove("operation");
            }
        }

        for key in ["name", "description", "status"] {
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

        let operation = obj
            .get("operation")
            .and_then(|value| value.as_str())
            .unwrap_or("list");

        if operation == "create" {
            let missing_name = obj
                .get("name")
                .and_then(|value| value.as_str())
                .is_none_or(|name| name.trim().is_empty());

            if missing_name {
                if let Some(name) = obj
                    .get("task_id")
                    .and_then(|value| value.as_str())
                    .and_then(Self::recover_task_create_name)
                    .or_else(|| {
                        obj.get("description")
                            .and_then(|value| value.as_str())
                            .and_then(Self::derive_task_name_from_description)
                    })
                {
                    obj.insert("name".to_string(), serde_json::Value::String(name));
                }
            }

            obj.remove("task_id");
        }

        serde_json::Value::Object(obj)
    }

    fn default_shell_timeout_secs(command: &str) -> u64 {
        let normalized = command.to_ascii_lowercase();
        let long_running_markers = [
            "cargo check",
            "cargo build",
            "cargo test",
            "cargo clippy",
            "cargo tauri",
            "tauri build",
            "tauri dev",
            "npm install",
            "npm run",
            "pnpm install",
            "pnpm run",
            "yarn install",
            "yarn build",
            "yarn test",
            "npx create-",
            "npx tauri",
        ];

        if long_running_markers
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            300
        } else {
            60
        }
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

        if let Some(message) = Self::repeated_redundant_tool_call_skip_message(
            &pending.name,
            &pending.arguments,
            response.tool_calls.iter(),
        ) {
            tracing::warn!(
                tool = %pending.name,
                "[ToolDispatch] Loop breaker skipped redundant repeated tool call"
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
                let args = Self::normalize_code_tool_arguments(args);
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
                            .filter_map(|v| {
                                let raw = v.as_str()?;
                                if let Some(ws) = workspace {
                                    Self::resolve_workspace_read_path(ws, raw)
                                        .ok()
                                        .map(|path| path.to_string_lossy().to_string())
                                } else {
                                    Some(raw.to_string())
                                }
                            })
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
                                let mut edit =
                                    serde_json::from_value::<crate::tools::code::EditOp>(v.clone())
                                        .ok()?;
                                if let Some(ws) = workspace {
                                    let resolved =
                                        Self::resolve_workspace_write_path(ws, &edit.path).ok()?;
                                    edit.path = resolved.to_string_lossy().to_string();
                                }
                                Some(edit)
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
                                Self::format_missing_code_batch_edit_edits_error(&args),
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
                let (command, env) = Self::harden_noninteractive_shell_command(command, env);

                // Optional timeout (seconds). Use a longer default for build/test/install flows.
                let timeout_secs = args
                    .get("timeout_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_else(|| Self::default_shell_timeout_secs(&command));

                // Streaming path: send real-time output chunks to the frontend.
                if let Some(tx) = stream_tx {
                    use crate::tools::shell_streaming;

                    match shell_streaming::execute_streaming(
                        &command,
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
                        &command,
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
                let (command, env) = Self::harden_noninteractive_shell_command(arguments, None);

                // Streaming path for raw-argument commands.
                if let Some(tx) = stream_tx {
                    use crate::tools::shell_streaming;

                    match shell_streaming::execute_streaming(
                        &command,
                        cwd.as_deref(),
                        env.as_ref(),
                        Some(Self::default_shell_timeout_secs(&command)),
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
                    match shell_async::execute_command_with_options(
                        &command,
                        cwd.as_deref(),
                        env.as_ref(),
                        Some(Self::default_shell_timeout_secs(&command)),
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
                        "read" => Self::resolve_workspace_read_path(ws, path_str),
                        "write" | "edit" => Self::resolve_workspace_write_path(ws, path_str),
                        _ => ws
                            .resolve_path(Path::new(path_str))
                            .map_err(|e| e.to_string()),
                    };

                    match resolved {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return ToolResult::Error(format!(
                                "Path '{}' could not be resolved within workspace: {}",
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
                            Some(status) => {
                                match manager.update_task_status(session_id, task_id, status) {
                                    Ok(_) => ToolResult::Success(format!(
                                        "Updated task {} status to {:?}",
                                        task_id, status
                                    )),
                                    Err(e) => ToolResult::Error(format!(
                                        "Failed to update task status: {}",
                                        e
                                    )),
                                }
                            }
                            None => match manager.get_task(session_id, task_id) {
                                Ok(Some(task)) if task.status == crate::TaskStatus::NotStarted => {
                                    match manager.update_task_status(
                                        session_id,
                                        task_id,
                                        crate::TaskStatus::InProgress,
                                    ) {
                                        Ok(_) => ToolResult::Success(format!(
                                            "Recovered omitted `status` on task {} by promoting it from NotStarted to InProgress. Future `update_status` calls should include explicit `status`.",
                                            task_id
                                        )),
                                        Err(e) => ToolResult::Error(format!(
                                            "Failed to recover missing task status for {}: {}",
                                            task_id, e
                                        )),
                                    }
                                }
                                Ok(Some(task)) => ToolResult::Skipped(format!(
                                    "Skipped malformed `task.update_status` without explicit `status` for task {} because its current status is {:?} and no safe status inference is possible. Continue the real implementation work; if you truly need a status change, retry once with both `task_id` and `status`.",
                                    task_id, task.status
                                )),
                                Ok(None) => ToolResult::Error(format!(
                                    "Task {} not found for update_status operation",
                                    task_id
                                )),
                                Err(e) => ToolResult::Error(format!(
                                    "Failed to inspect current task status for {}: {}",
                                    task_id, e
                                )),
                            },
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
                "Important: task-tracking errors must not block implementation work. The runtime already keeps the tracked root task aligned with overall run progress, so do not use `task` just to preserve momentum. If a task operation fails, continue the real work with file/shell/code tools and only retry the task tool when you have one exact bookkeeping action ready. For `create`, provide a specific `name` and preferably a concrete `description`; for `update_status`, always include both `task_id` and `status`.\n",
            );
        }

        let had_task_tool_success = tool_calls.iter().any(|tool_call| {
            tool_call.name == "task" && matches!(tool_call.result, ToolResult::Success(_))
        });

        if had_task_tool_success {
            prompt.push_str(
                "Important: a successful task update is only bookkeeping. Do not repeat the same task update just to confirm it. After a task update succeeds, continue with the next concrete implementation or verification step unless a different task genuinely changed and you know its exact next status.\n",
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
                "Important: if `task.update_status` failed because `status` was omitted, treat that as malformed arguments, not a reason to keep looping on task bookkeeping. The malformed task-update arguments are echoed above. If you already know the new status, send one corrected `update_status` call with both `task_id` and `status`; otherwise do not call `task` on the next step and continue the real work now.\n",
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

        let had_code_tool_error = tool_calls.iter().any(|tool_call| {
            tool_call.name == "code" && matches!(tool_call.result, ToolResult::Error(_))
        });

        if had_code_tool_error {
            prompt.push_str(
                "Important: code-tool errors must not block implementation work. For `code.batch_edit`, always send `edits` as an array, even for one change, and each entry must include `path`, `old_str`, and `new_str`. If you only need one targeted replacement in one file, `file.edit` may be simpler.\n",
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

        let had_missing_code_batch_edit_edits_error = tool_calls.iter().any(|tool_call| {
            tool_call.name == "code"
                && matches!(
                    &tool_call.result,
                    ToolResult::Error(message)
                        if message.contains("Missing required field 'edits' for code batch_edit operation")
                )
        });

        if had_missing_code_batch_edit_edits_error {
            prompt.push_str(
                "Important: the malformed code batch-edit arguments are echoed above. Do not retry `code.batch_edit` without a valid `edits` array. Retry only with one corrected `edits` array containing objects shaped like `{\"path\":...,\"old_str\":...,\"new_str\":...}`; aliases like `changes` are only safe if they still normalize into that canonical shape. For one targeted single-file replacement, `file.edit` is also acceptable.\n",
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

        let had_non_tty_shell_error = tool_calls.iter().any(|tool_call| {
            tool_call.name == "shell"
                && matches!(
                    &tool_call.result,
                    ToolResult::Error(message) if message.to_ascii_lowercase().contains("not a terminal")
                )
        });

        if had_non_tty_shell_error {
            prompt.push_str(
                "Important: a shell command failed because it expected an interactive terminal. Retry with a fully non-interactive command shape (for example explicit `--yes` flags plus env like `CI=1` and `FORCE_COLOR=0`) instead of repeating the same scaffold command unchanged.\n",
            );
        }

        let non_tty_tauri_failures = Self::count_non_tty_tauri_scaffold_failures(tool_calls.iter());
        if non_tty_tauri_failures >= 2 {
            prompt.push_str(
                "Important: `create-tauri-app` has already failed multiple times because the shell is non-interactive. Stop retrying the same scaffold shape and do not manually synthesize Tauri files with `cat <<EOF`, `tee`, `cargo init src-tauri`, or `mkdir`-only shell scripts. Use one specific alternate strategy next: run `npx create-tauri-app --help` (or `npx create-tauri-app@latest --help`) to confirm flags, then run a single `npx create-tauri-app <app-dir> --yes --template vanilla --tauri-version 2` command; if you already have a frontend project, use `cargo tauri init --ci ...` instead. After the scaffold succeeds, move directly to edits and build/test verification.\n",
            );
        }

        let had_redundant_tauri_init_skip = tool_calls.iter().any(|tool_call| {
            tool_call.name == "shell"
                && matches!(
                    &tool_call.result,
                    ToolResult::Skipped(message)
                        if message.contains("redundant repeated `cargo tauri init`")
                )
        });

        if had_redundant_tauri_init_skip {
            prompt.push_str(
                "Important: the Tauri scaffold has already been initialized in this run. Do not call `cargo tauri init` again. Move on to editing the frontend files and then run the requested build/test verification.\n",
            );
        }

        let successful_tauri_scaffold_target = tool_calls.iter().find_map(|tool_call| {
            (tool_call.name == "shell" && matches!(tool_call.result, ToolResult::Success(_)))
                .then(|| Self::extract_shell_command_from_arguments(&tool_call.arguments))
                .flatten()
                .filter(|command| Self::is_tauri_scaffold_command(command))
                .and_then(|command| Self::extract_tauri_scaffold_target(&command))
        });

        if let Some(target) = successful_tauri_scaffold_target {
            prompt.push_str(&format!(
                "Important: the Tauri scaffold now exists at `{target}`. Do not spend another turn repeatedly listing or treeing that directory. Read only the minimum files needed to implement the request (for example `src/index.html`, `src/main.js`, `src/styles.css`, and `src-tauri/src/lib.rs` or `src-tauri/src/main.rs`), then make the Hello World change and run the remaining build/test verification.\n"
            ));
        }

        let had_redundant_file_inspection_skip = tool_calls.iter().any(|tool_call| {
            tool_call.name == "file"
                && matches!(
                    &tool_call.result,
                    ToolResult::Skipped(message)
                        if message.contains("redundant repeated `file.")
                )
        });

        if had_redundant_file_inspection_skip {
            prompt.push_str(
                "Important: repeated file inspections of the same path are now being skipped. Stop re-listing the scaffold root and move directly to the next concrete action: read the exact file you need, edit it, then run the requested verification commands.\n",
            );
        }

        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::AgentPipeline;
    use crate::config::AppConfig;
    use crate::pipeline::{ToolCallRecord, ToolResult};
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
            "task_id": "setup-tauri-env",
            "parent_id": "e4d5f1a0-c20d-4562-aef3-b1ca3ffbdb8c"
        }));

        assert_eq!(
            normalized.get("name").and_then(|v| v.as_str()),
            Some("Setup Tauri Env")
        );
        assert!(normalized.get("task_id").is_none());
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
    fn harden_noninteractive_shell_command_normalizes_create_tauri_app() {
        let (command, env) = AgentPipeline::harden_noninteractive_shell_command(
            "npx create-tauri-app@latest hello-world --template vanilla",
            None,
        );

        assert!(command.contains("--yes"));
        let env = env.expect("env should be injected");
        assert_eq!(env.get("CI"), Some(&"true".to_string()));
        assert_eq!(env.get("FORCE_COLOR"), Some(&"0".to_string()));
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
    fn normalize_code_tool_arguments_recovers_batch_edit_aliases() {
        let normalized = AgentPipeline::normalize_code_tool_arguments(json!({
            "operation": "edit",
            "changes": [{
                "file": "\"src/index.html\"",
                "old": "<h1>Hello</h1>",
                "replacement": "<h1>Hello, Gestura</h1>",
            }]
        }));

        assert_eq!(
            normalized.get("operation").and_then(|v| v.as_str()),
            Some("batch_edit")
        );
        let edits = normalized
            .get("edits")
            .and_then(|value| value.as_array())
            .expect("edits array should be recovered");
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].get("path").and_then(|v| v.as_str()),
            Some("src/index.html")
        );
        assert_eq!(
            edits[0].get("old_str").and_then(|v| v.as_str()),
            Some("<h1>Hello</h1>")
        );
        assert_eq!(
            edits[0].get("new_str").and_then(|v| v.as_str()),
            Some("<h1>Hello, Gestura</h1>")
        );
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
        assert!(message.contains(
            "Do not omit `status` to ask the runtime to infer or preserve the current state"
        ));
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
        assert!(message.contains(
            "Do not rely on the runtime to invent placeholder names like 'Untitled Task'"
        ));
    }

    #[test]
    fn missing_code_batch_edit_edits_error_explains_how_to_recover() {
        let message = AgentPipeline::format_missing_code_batch_edit_edits_error(&json!({
            "operation": "batch_edit",
            "path": "src/index.html",
            "note": "replace the greeting",
        }));

        assert!(message.contains("Missing required field 'edits' for code batch_edit operation"));
        assert!(message.contains("`batch_edit` requires an `edits` array"));
        assert!(message.contains("\"operation\":\"batch_edit\""));
        assert!(message.contains("Prefer the canonical `edits` array shape"));
    }

    #[test]
    fn repeated_malformed_tool_call_skip_message_trips_on_second_attempt() {
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
    fn repeated_malformed_tool_call_skip_message_does_not_trip_on_first_attempt() {
        let malformed_args = json!({
            "operation": "write",
            "path": "tauri-hello-world/src/index.html",
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
    fn repeated_malformed_task_update_without_status_trips_on_second_attempt() {
        let malformed_args = json!({
            "operation": "update_status",
            "task_id": "6073f304-388d-408c-82d0-f49f8679656a",
        })
        .to_string();

        let prior_records = vec![crate::pipeline::ToolCallRecord {
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
    fn repeated_malformed_task_create_without_name_trips_on_second_attempt() {
        let malformed_args = json!({
            "operation": "create",
            "task_id": "\"tauri-hello-world\" wait no, task_id not for create.",
        })
        .to_string();

        let prior_records = vec![crate::pipeline::ToolCallRecord {
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
    fn repeated_malformed_code_batch_edit_without_edits_trips_on_second_attempt() {
        let malformed_args = json!({
            "operation": "batch_edit",
            "path": "src/index.html",
            "note": "replace the heading",
        })
        .to_string();

        let prior_records = vec![crate::pipeline::ToolCallRecord {
            id: "1".to_string(),
            name: "code".to_string(),
            arguments: malformed_args.clone(),
            result: crate::pipeline::ToolResult::Error(
                AgentPipeline::format_missing_code_batch_edit_edits_error(&json!({
                    "operation": "batch_edit",
                    "path": "src/index.html",
                    "note": "replace the heading",
                })),
            ),
            duration_ms: 1,
        }];

        let message = AgentPipeline::repeated_malformed_tool_call_skip_message(
            "code",
            &malformed_args,
            &prior_records,
        )
        .expect("second malformed batch_edit should trip loop breaker");

        assert!(message.contains("Loop breaker:"));
        assert!(message.contains("code.batch_edit"));
        assert!(message.contains("`edits` array"));
    }

    #[test]
    fn repeated_redundant_tauri_init_trips_after_first_success() {
        let args = json!({
            "command": "cargo tauri init --ci --app-name hello-world --window-title \"Hello World\" --force"
        })
        .to_string();
        let prior_records = vec![ToolCallRecord {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: args.clone(),
            result: ToolResult::Success(String::new()),
            duration_ms: 1,
        }];

        let message = AgentPipeline::repeated_redundant_tool_call_skip_message(
            "shell",
            &args,
            &prior_records,
        )
        .expect("second successful tauri init should trip loop breaker");

        assert!(message.contains("redundant repeated `cargo tauri init`"));
        assert!(message.contains("move on to editing files and running build/test verification"));
    }

    #[test]
    fn manual_tauri_shell_scaffold_is_blocked_after_repeated_non_tty_failures() {
        let prior_records = vec![
            ToolCallRecord {
                id: "1".to_string(),
                name: "shell".to_string(),
                arguments: json!({
                    "command": "npx create-tauri-app@latest hello-tauri --template vanilla --yes"
                })
                .to_string(),
                result: ToolResult::Error("Exit 1: Shell command failed because it expected a terminal. stderr: IO error: not a terminal".to_string()),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "shell".to_string(),
                arguments: json!({
                    "command": "npx create-tauri-app@latest hello-tauri --template vanilla --yes"
                })
                .to_string(),
                result: ToolResult::Error("Exit 1: command likely waited for interactive input".to_string()),
                duration_ms: 1,
            },
        ];
        let manual_args = json!({
            "command": "mkdir -p hello-tauri/src-tauri/src && cat <<'EOF' > hello-tauri/src-tauri/Cargo.toml\n[package]\nname = \"hello-tauri\"\nEOF"
        })
        .to_string();

        let message = AgentPipeline::repeated_redundant_tool_call_skip_message(
            "shell",
            &manual_args,
            &prior_records,
        )
        .expect("manual tauri shell fallback should be blocked after repeated non-tty failures");

        assert!(message.contains("Do not synthesize a Tauri project with `cat <<EOF`"));
        assert!(message.contains("npx create-tauri-app --help"));
        assert!(message.contains("cargo tauri init --ci"));
    }

    #[test]
    fn repeated_create_tauri_app_latest_is_blocked_after_two_non_tty_failures() {
        let prior_records = vec![
            ToolCallRecord {
                id: "1".to_string(),
                name: "shell".to_string(),
                arguments: json!({
                    "command": "npx create-tauri-app@latest hello-tauri --template vanilla --yes"
                })
                .to_string(),
                result: ToolResult::Error("Exit 1: stderr: IO error: not a terminal".to_string()),
                duration_ms: 1,
            },
            ToolCallRecord {
                id: "2".to_string(),
                name: "shell".to_string(),
                arguments: json!({
                    "command": "npx create-tauri-app@latest hello-tauri --template vanilla --yes"
                })
                .to_string(),
                result: ToolResult::Error(
                    "Exit 124: command likely waited for interactive input".to_string(),
                ),
                duration_ms: 1,
            },
        ];
        let retry_args = json!({
            "command": "npx create-tauri-app@latest hello-tauri --template vanilla --yes"
        })
        .to_string();

        let message = AgentPipeline::repeated_redundant_tool_call_skip_message(
            "shell",
            &retry_args,
            &prior_records,
        )
        .expect("same create-tauri-app retry should be blocked after repeated non-tty failures");

        assert!(message.contains("Use one specific alternate strategy next"));
        assert!(
            message.contains(
                "npx create-tauri-app <app-dir> --yes --template vanilla --tauri-version 2"
            )
        );
    }

    #[test]
    fn repeated_file_tree_inspection_trips_after_two_successes() {
        let args = json!({
            "operation": "tree",
            "path": "hello-tauri"
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
        assert!(message.contains("hello-tauri"));
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
    async fn update_status_without_status_recovers_not_started_task_to_in_progress() {
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
            crate::pipeline::ToolResult::Success(output) => output,
            other => panic!("expected success, got {other:?}"),
        };

        assert!(output.contains("Recovered omitted `status`"));
        assert!(output.contains("NotStarted to InProgress"));

        let task_after = manager
            .get_task(&session_id, &task.id)
            .expect("get task")
            .expect("task exists");
        assert_eq!(task_after.status, crate::TaskStatus::InProgress);
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
            crate::pipeline::ToolResult::Skipped(output) => output,
            other => panic!("expected skipped result, got {other:?}"),
        };

        assert!(
            output.contains("Skipped malformed `task.update_status` without explicit `status`")
        );
        assert!(output.contains("current status is InProgress"));

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
            crate::pipeline::ToolResult::Skipped(output) => output,
            other => panic!("expected skipped result, got {other:?}"),
        };

        assert!(
            output.contains("Skipped malformed `task.update_status` without explicit `status`")
        );
        assert!(output.contains("current status is Completed"));

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
    async fn file_edit_recovers_unique_nested_workspace_suffix_path() {
        let temp = TempDir::new().expect("temp dir");
        let session_id = format!("file-edit-suffix-test-{}", uuid::Uuid::new_v4());
        let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
            .expect("workspace");
        let project_dir = temp.path().join("hello-world").join("src");
        std::fs::create_dir_all(&project_dir).expect("create nested src dir");
        let file_path = project_dir.join("index.html");
        std::fs::write(&file_path, "<h1>Hello</h1>\n").expect("write seed file");
        let pipeline = AgentPipeline::new(AppConfig::default());

        let result = pipeline
            .execute_file_tool(
                &json!({
                    "operation": "edit",
                    "path": "src/index.html",
                    "old": "<h1>Hello</h1>",
                    "new": "<h1>Hello, Gestura</h1>",
                })
                .to_string(),
                Some(&workspace),
            )
            .await;

        match result {
            crate::pipeline::ToolResult::Success(output) => {
                assert!(output.contains("src/index.html"));
                let updated = std::fs::read_to_string(&file_path).expect("read updated file");
                assert!(updated.contains("Hello, Gestura"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn code_batch_edit_accepts_changes_aliases() {
        let temp = TempDir::new().expect("temp dir");
        let session_id = format!("code-edit-alias-test-{}", uuid::Uuid::new_v4());
        let workspace = SessionWorkspace::from_directory(&session_id, temp.path().to_path_buf())
            .expect("workspace");
        let file_path = temp.path().join("index.html");
        std::fs::write(&file_path, "<h1>Hello</h1>\n").expect("write seed file");
        let pipeline = AgentPipeline::new(AppConfig::default());

        let result = pipeline
            .execute_code_tool(
                &json!({
                    "operation": "edit",
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
            crate::pipeline::ToolResult::Success(output) => {
                assert!(output.contains("index.html"));
                let updated = std::fs::read_to_string(&file_path).expect("read updated file");
                assert!(updated.contains("Hello, Gestura"));
            }
            other => panic!("expected success, got {other:?}"),
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
        let file_path = project_dir.join("index.html");
        std::fs::write(&file_path, "<h1>Hello</h1>\n").expect("write seed file");
        let pipeline = AgentPipeline::new(AppConfig::default());

        let result = pipeline
            .execute_code_tool(
                &json!({
                    "operation": "batch_edit",
                    "edits": [{
                        "path": "src/index.html",
                        "old_str": "<h1>Hello</h1>",
                        "new_str": "<h1>Hello, Gestura</h1>",
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
    async fn file_write_accepts_multiline_pattern_as_content_fallback() {
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
        assert!(prompt.contains(
            "runtime already keeps the tracked root task aligned with overall run progress"
        ));
        assert!(prompt.contains(
            "For `create`, provide a specific `name` and preferably a concrete `description`"
        ));
        assert!(prompt.contains("always include both `task_id` and `status`"));
        assert!(
            prompt.contains("Arguments: {\"operation\":\"update_status\",\"task_id\":\"abc\"}")
        );
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

        assert!(prompt.contains("if `task.update_status` failed because `status` was omitted"));
        assert!(prompt.contains("not a reason to keep looping on task bookkeeping"));
        assert!(prompt.contains("do not call `task` on the next step"));
        assert!(
            prompt.contains("Arguments: {\"operation\":\"update_status\",\"task_id\":\"abc\"}")
        );
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
                    "Loop breaker: skipped a repeated malformed `task.create` call without a valid `name` after 1 prior similar malformed attempts in this run."
                        .to_string(),
                ),
                duration_ms: 1,
            }],
        );

        assert!(prompt.contains("task tracking as temporarily disabled for this run"));
        assert!(prompt.contains(
            "continue the real implementation/build/test work with file/shell/code tools"
        ));
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
                    "Loop breaker: skipped a repeated malformed `file.write` call without `content` after 1 prior similar non-successful attempts in this run. The agent is still running.".to_string(),
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
    fn continuation_prompt_pushes_implementation_after_successful_tauri_scaffold() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let prompt = pipeline.build_tool_continuation_prompt(
            "User: create a tauri hello world app and build/test it",
            "I scaffolded the app and will inspect the files.",
            &[ToolCallRecord {
                id: "1".to_string(),
                name: "shell".to_string(),
                arguments: json!({
                    "command": "npx create-tauri-app@latest hello-tauri --yes --template vanilla --tauri-version 2"
                })
                .to_string(),
                result: ToolResult::Success("Scaffold created".to_string()),
                duration_ms: 1,
            }],
        );

        assert!(prompt.contains("the Tauri scaffold now exists at `hello-tauri`"));
        assert!(
            prompt
                .contains("Do not spend another turn repeatedly listing or treeing that directory")
        );
        assert!(
            prompt.contains(
                "make the Hello World change and run the remaining build/test verification"
            )
        );
    }

    #[test]
    fn continuation_prompt_explains_redundant_file_inspection_skip() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let prompt = pipeline.build_tool_continuation_prompt(
            "User: finish the tauri app",
            "I will inspect the scaffold again.",
            &[ToolCallRecord {
                id: "1".to_string(),
                name: "file".to_string(),
                arguments: json!({
                    "operation": "tree",
                    "path": "hello-tauri"
                })
                .to_string(),
                result: ToolResult::Skipped(
                    "Loop breaker: skipped a redundant repeated `file.tree` inspection of `hello-tauri` after 2 prior successful identical inspections in this run.".to_string(),
                ),
                duration_ms: 1,
            }],
        );

        assert!(
            prompt.contains("repeated file inspections of the same path are now being skipped")
        );
        assert!(prompt.contains("Stop re-listing the scaffold root"));
        assert!(prompt.contains("edit it, then run the requested verification commands"));
    }

    #[test]
    fn continuation_prompt_forces_specific_recovery_after_repeated_non_tty_tauri_failures() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let prompt = pipeline.build_tool_continuation_prompt(
            "User: create a tauri app and build/test it",
            "I will try another shell fallback.",
            &[
                ToolCallRecord {
                    id: "1".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({
                        "command": "npx create-tauri-app@latest hello-tauri --template vanilla --yes"
                    })
                    .to_string(),
                    result: ToolResult::Error(
                        "Exit 1: stderr: IO error: not a terminal".to_string(),
                    ),
                    duration_ms: 1,
                },
                ToolCallRecord {
                    id: "2".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({
                        "command": "npx create-tauri-app@latest hello-tauri --template vanilla --yes"
                    })
                    .to_string(),
                    result: ToolResult::Error(
                        "Exit 124: command likely waited for interactive input".to_string(),
                    ),
                    duration_ms: 1,
                },
            ],
        );

        assert!(prompt.contains("`create-tauri-app` has already failed multiple times"));
        assert!(prompt.contains("do not manually synthesize Tauri files with `cat <<EOF`"));
        assert!(prompt.contains("npx create-tauri-app --help"));
        assert!(
            prompt.contains(
                "npx create-tauri-app <app-dir> --yes --template vanilla --tauri-version 2"
            )
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
