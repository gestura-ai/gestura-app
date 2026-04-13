use std::{collections::HashMap, sync::Arc, sync::OnceLock};

use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Sse, sse::Event},
    routing::{get, post},
};
use chrono::Utc;
use gestura_core::a2a::{A2ATaskEventKind, RemoteTaskContract, RemoteTaskProgress, TaskStatus};
use gestura_core::tools::{ShellTools, find_tool};
use gestura_core::{
    A2AMessage, A2ARequest, A2AServer, A2ATask, AgentPipeline, AgentRequest, AppConfig, Artifact,
    MessagePart, RequestSource, create_gestura_agent_card,
};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};

static A2A_RUNTIME: OnceLock<Arc<A2ARuntime>> = OnceLock::new();

#[derive(Clone)]
struct A2AAppState {
    server: Arc<A2AServer>,
}

#[derive(Clone)]
struct A2ARuntime {
    server: Arc<A2AServer>,
    base_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbeddedExecutionRoute {
    DirectShell,
    Pipeline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedCapabilities {
    allowed_tools: Vec<String>,
    unsupported: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EmbeddedTaskExecutionPlan {
    route: EmbeddedExecutionRoute,
    input: String,
    requested_capabilities: Vec<String>,
    allowed_tools: Vec<String>,
    unsupported_capabilities: Vec<String>,
    tools_enabled: bool,
    workspace: Option<String>,
}

/// Status summary for the embedded GUI A2A runtime.
#[derive(Debug, Clone, Serialize)]
pub struct A2ARuntimeStatus {
    pub enabled: bool,
    pub base_url: Option<String>,
    pub task_count: usize,
}

fn normalize_capability_token(capability: &str) -> String {
    capability
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
}

fn capability_aliases(capability: &str) -> &'static [&'static str] {
    match capability {
        "filesystem" | "files" | "fs" => &["file"],
        "commands" | "command" => &["shell"],
        "internet" | "network" | "http" => &["web", "web_search"],
        "search" => &["web_search"],
        "source_control" | "version_control" | "vcs" => &["git"],
        "codebase" => &["code"],
        "workflow" | "tasks" => &["task"],
        _ => &[],
    }
}

fn push_unique(values: &mut Vec<String>, value: impl Into<String>) {
    let value = value.into();
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn resolve_requested_capabilities(requested_capabilities: &[String]) -> ResolvedCapabilities {
    let mut allowed_tools = Vec::new();
    let mut unsupported = Vec::new();

    for capability in requested_capabilities {
        let normalized = normalize_capability_token(capability);
        if find_tool(&normalized).is_some() {
            push_unique(&mut allowed_tools, normalized);
            continue;
        }

        let aliases = capability_aliases(&normalized);
        if aliases.is_empty() {
            push_unique(&mut unsupported, capability.clone());
            continue;
        }

        let mut resolved_any = false;
        for alias in aliases {
            if find_tool(alias).is_some() {
                push_unique(&mut allowed_tools, alias.to_string());
                resolved_any = true;
            }
        }
        if !resolved_any {
            push_unique(&mut unsupported, capability.clone());
        }
    }

    ResolvedCapabilities {
        allowed_tools,
        unsupported,
    }
}

fn extract_primary_prompt(task: &A2ATask) -> Option<String> {
    task.messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .find_map(|part| match part {
            MessagePart::Text { text } if !text.trim().is_empty() => Some(text.trim().to_string()),
            _ => None,
        })
}

fn compose_pipeline_input(
    prompt: Option<&str>,
    contract: Option<&RemoteTaskContract>,
) -> Option<String> {
    let mut sections = Vec::new();
    if let Some(prompt) = prompt.filter(|prompt| !prompt.trim().is_empty()) {
        sections.push(prompt.trim().to_string());
    }
    if let Some(contract) = contract {
        if sections.is_empty()
            || sections
                .first()
                .is_none_or(|first| first != &contract.objective)
        {
            sections.push(format!("Objective: {}", contract.objective));
        }
        if !contract.acceptance_criteria.is_empty() {
            sections.push(format!(
                "Acceptance criteria:\n- {}",
                contract.acceptance_criteria.join("\n- ")
            ));
        }
        if !contract.constraints.is_empty() {
            sections.push(format!(
                "Constraints:\n- {}",
                contract.constraints.join("\n- ")
            ));
        }
        if !contract.deliverables.is_empty() {
            sections.push(format!(
                "Deliverables:\n- {}",
                contract.deliverables.join("\n- ")
            ));
        }
        if let Some(output_format) = contract.output_format.as_deref() {
            sections.push(format!("Output format: {output_format}"));
        }
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

fn requested_workspace(task: &A2ATask) -> Option<String> {
    task.metadata
        .get("workspaceDir")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn explicit_shell_command(
    task: &A2ATask,
    prompt: Option<&str>,
    allowed_tools: &[String],
) -> Option<String> {
    if !allowed_tools.iter().any(|tool| tool == "shell") {
        return None;
    }

    let metadata_shell = task
        .metadata
        .get("executionKind")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("shell_command"));

    let prompt = prompt?.trim();
    if let Some(command) = prompt.strip_prefix("shell:") {
        let command = command.trim();
        if !command.is_empty() {
            return Some(command.to_string());
        }
    }

    if metadata_shell && !prompt.is_empty() {
        return Some(prompt.to_string());
    }

    None
}

fn build_execution_plan(task: &A2ATask) -> Result<EmbeddedTaskExecutionPlan, String> {
    let primary_prompt = extract_primary_prompt(task);
    let resolved = resolve_requested_capabilities(&task.requested_capabilities);
    let workspace = requested_workspace(task);

    if !resolved.unsupported.is_empty() {
        return Err(format!(
            "Unsupported requested capabilities: {}",
            resolved.unsupported.join(", ")
        ));
    }

    if let Some(command) =
        explicit_shell_command(task, primary_prompt.as_deref(), &resolved.allowed_tools)
    {
        return Ok(EmbeddedTaskExecutionPlan {
            route: EmbeddedExecutionRoute::DirectShell,
            input: command,
            requested_capabilities: task.requested_capabilities.clone(),
            allowed_tools: resolved.allowed_tools,
            unsupported_capabilities: resolved.unsupported,
            tools_enabled: true,
            workspace,
        });
    }

    let input = compose_pipeline_input(primary_prompt.as_deref(), task.contract.as_ref())
        .ok_or_else(|| {
            "Remote task did not include executable text or contract objective".to_string()
        })?;

    Ok(EmbeddedTaskExecutionPlan {
        route: EmbeddedExecutionRoute::Pipeline,
        input,
        requested_capabilities: task.requested_capabilities.clone(),
        allowed_tools: resolved.allowed_tools,
        unsupported_capabilities: resolved.unsupported,
        tools_enabled: !task.requested_capabilities.is_empty(),
        workspace,
    })
}

async fn run_shell_contract(command: &str, workspace: Option<&str>) -> Result<String, String> {
    let command = command.to_string();
    let workspace = workspace.map(ToOwned::to_owned);
    tokio::task::spawn_blocking(move || {
        let tools = ShellTools::new();
        let result = tools
            .run_with_options(
                &command,
                workspace.as_deref().map(std::path::Path::new),
                None,
                Some(60),
            )
            .map_err(|error| error.to_string())?;
        if result.success {
            Ok(result.stdout)
        } else {
            Err(format!(
                "Shell command failed with exit code {}: {}",
                result.exit_code,
                result.stderr.trim()
            ))
        }
    })
    .await
    .map_err(|error| format!("Shell command task join error: {error}"))?
}

async fn execute_pipeline_contract(
    config: AppConfig,
    plan: &EmbeddedTaskExecutionPlan,
) -> Result<String, String> {
    let mut request = AgentRequest::new(plan.input.clone())
        .with_streaming(false)
        .with_source(RequestSource::Orchestrator)
        .with_tools_enabled(plan.tools_enabled);
    if !plan.allowed_tools.is_empty() {
        request = request.with_allowed_tools(plan.allowed_tools.clone());
    }
    if let Some(workspace) = plan.workspace.clone() {
        request = request.with_workspace(workspace);
    }
    AgentPipeline::with_provider_optimized_config(config)
        .process_blocking(request)
        .await
        .map(|response| response.content)
        .map_err(|error| error.to_string())
}

fn route_label(route: EmbeddedExecutionRoute) -> &'static str {
    match route {
        EmbeddedExecutionRoute::DirectShell => "direct_shell",
        EmbeddedExecutionRoute::Pipeline => "pipeline",
    }
}

fn execution_plan_metadata(plan: &EmbeddedTaskExecutionPlan) -> HashMap<String, serde_json::Value> {
    HashMap::from([
        (
            "executionRoute".to_string(),
            serde_json::Value::String(route_label(plan.route).to_string()),
        ),
        (
            "requestedCapabilities".to_string(),
            serde_json::json!(plan.requested_capabilities),
        ),
        (
            "allowedTools".to_string(),
            serde_json::json!(plan.allowed_tools),
        ),
        (
            "toolsEnabled".to_string(),
            serde_json::Value::Bool(plan.tools_enabled),
        ),
    ])
}

fn build_runtime_app(server: Arc<A2AServer>) -> Router {
    Router::new()
        .route("/a2a", post(handle_request))
        .route("/.well-known/agent-card.json", get(agent_card))
        .route("/a2a/events", get(task_events))
        .with_state(A2AAppState { server })
}

fn spawn_runtime_server(listener: TcpListener, app: Router) {
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::error!(%error, "A2A runtime stopped");
        }
    });
}

/// Start the embedded GUI A2A runtime if it is not already running.
pub async fn start_a2a_runtime(config: AppConfig) -> Result<(), String> {
    if A2A_RUNTIME.get().is_some() {
        return Ok(());
    }
    let port = std::env::var("GESTURA_A2A_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(32145);
    let origin = format!("http://127.0.0.1:{port}");
    let base_url = format!("{origin}/a2a");
    let server = Arc::new(A2AServer::new(create_gestura_agent_card(&origin)));
    let runtime = Arc::new(A2ARuntime {
        server: server.clone(),
        base_url,
    });
    let _ = A2A_RUNTIME.set(runtime.clone());
    spawn_worker(server.clone(), config);
    let app = build_runtime_app(server);
    tokio::spawn(async move {
        match TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await {
            Ok(listener) => spawn_runtime_server(listener, app),
            Err(error) => tracing::warn!(%error, port, "Failed to start embedded A2A runtime"),
        }
    });
    Ok(())
}

/// Return the current embedded GUI A2A runtime status.
pub fn a2a_runtime_status() -> A2ARuntimeStatus {
    A2A_RUNTIME
        .get()
        .map(|runtime| A2ARuntimeStatus {
            enabled: true,
            base_url: Some(runtime.base_url.clone()),
            task_count: runtime.server.list_tasks().len(),
        })
        .unwrap_or(A2ARuntimeStatus {
            enabled: false,
            base_url: None,
            task_count: 0,
        })
}

fn spawn_worker(server: Arc<A2AServer>, config: AppConfig) {
    let handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        let events = server.subscribe_events();
        while let Ok(event) = events.recv() {
            if matches!(event.kind, A2ATaskEventKind::Created) {
                let server = server.clone();
                let config = config.clone();
                handle.spawn(async move {
                    process_task(server, config, event.task.id).await;
                });
            }
        }
    });
}

async fn process_task(server: Arc<A2AServer>, config: AppConfig, task_id: String) {
    let Some(task) = server.get_task(&task_id) else {
        return;
    };
    let plan = match build_execution_plan(&task) {
        Ok(plan) => plan,
        Err(error) => {
            let _ = server.add_message(
                &task.id,
                A2AMessage {
                    role: "assistant".into(),
                    parts: vec![MessagePart::Text {
                        text: error.clone(),
                    }],
                },
            );
            let _ = server.update_task_status_with_reason(&task.id, TaskStatus::Blocked, error);
            return;
        }
    };
    let _ = server.update_task_status_with_reason(
        &task.id,
        TaskStatus::Running,
        format!(
            "Embedded GUI runtime executing via {}",
            route_label(plan.route)
        ),
    );
    let _ = server.update_task_progress(
        &task.id,
        RemoteTaskProgress {
            stage: Some("routing".into()),
            message: Some(format!(
                "Resolved {} requested capabilities into {} allowed tools",
                plan.requested_capabilities.len(),
                plan.allowed_tools.len()
            )),
            percent: Some(10),
            updated_at: Utc::now(),
        },
    );
    let execution = match plan.route {
        EmbeddedExecutionRoute::DirectShell => {
            let _ = server.update_task_progress(
                &task.id,
                RemoteTaskProgress {
                    stage: Some("shell".into()),
                    message: Some("Running direct shell contract".into()),
                    percent: Some(50),
                    updated_at: Utc::now(),
                },
            );
            run_shell_contract(&plan.input, plan.workspace.as_deref()).await
        }
        EmbeddedExecutionRoute::Pipeline => {
            let _ = server.update_task_progress(
                &task.id,
                RemoteTaskProgress {
                    stage: Some("pipeline".into()),
                    message: Some("Running capability-scoped agent pipeline".into()),
                    percent: Some(50),
                    updated_at: Utc::now(),
                },
            );
            execute_pipeline_contract(config, &plan).await
        }
    };
    match execution {
        Ok(output) => {
            let _ = server.update_task_progress(
                &task.id,
                RemoteTaskProgress {
                    stage: Some("completed".into()),
                    message: Some("Execution completed".into()),
                    percent: Some(100),
                    updated_at: Utc::now(),
                },
            );
            let mut result_metadata = execution_plan_metadata(&plan);
            result_metadata.insert(
                "mimeType".to_string(),
                serde_json::Value::String("text/plain".to_string()),
            );
            let _ = server.add_message(
                &task.id,
                A2AMessage {
                    role: "assistant".into(),
                    parts: vec![MessagePart::Text {
                        text: output.clone(),
                    }],
                },
            );
            let _ = server.add_artifact(
                &task.id,
                Artifact {
                    name: "result.txt".into(),
                    parts: vec![MessagePart::Text {
                        text: output.clone(),
                    }],
                    metadata: result_metadata,
                },
            );
            let _ = server.add_artifact(
                &task.id,
                Artifact {
                    name: "execution-plan.json".into(),
                    parts: vec![MessagePart::Text {
                        text: serde_json::json!({
                            "route": route_label(plan.route),
                            "requestedCapabilities": plan.requested_capabilities,
                            "allowedTools": plan.allowed_tools,
                            "toolsEnabled": plan.tools_enabled,
                            "workspace": plan.workspace,
                        })
                        .to_string(),
                    }],
                    metadata: HashMap::from([(
                        "mimeType".to_string(),
                        serde_json::Value::String("application/json".to_string()),
                    )]),
                },
            );
            let _ = server.update_task_status_with_reason(
                &task.id,
                TaskStatus::Completed,
                "Embedded GUI runtime completed task",
            );
        }
        Err(error) => {
            let _ = server.add_message(
                &task.id,
                A2AMessage {
                    role: "assistant".into(),
                    parts: vec![MessagePart::Text {
                        text: error.clone(),
                    }],
                },
            );
            let _ = server.update_task_status_with_reason(&task.id, TaskStatus::Failed, error);
        }
    }
}

async fn handle_request(
    State(state): State<A2AAppState>,
    headers: HeaderMap,
    Json(request): Json<A2ARequest>,
) -> impl IntoResponse {
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    Json(state.server.handle_request_with_auth(request, bearer))
}

async fn agent_card(State(state): State<A2AAppState>) -> impl IntoResponse {
    Json(state.server.agent_card.clone())
}

async fn task_events(
    State(state): State<A2AAppState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let receiver = state.server.subscribe_events();
    let (sender, bridge) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(event) = receiver.recv() {
            if sender.send(event).is_err() {
                break;
            }
        }
    });
    let stream = UnboundedReceiverStream::new(bridge).map(|event| {
        Ok(Event::default()
            .event("task")
            .json_data(event)
            .unwrap_or_else(|_| {
                Event::default()
                    .event("task")
                    .data("{\"error\":\"serialization_failed\"}")
            }))
    });
    Sse::new(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gestura_core::a2a::CreateTaskRequest;
    use gestura_core::agents::{
        AgentExecutionMode, AgentManager, AgentRole, DelegatedTask, RemoteAgentTarget,
    };
    use gestura_core::orchestrator::SupervisorTaskState;
    use gestura_core::{A2AClient, AgentOrchestrator, AgentProfile};
    use std::path::PathBuf;

    fn sample_task(prompt: &str, requested_capabilities: Vec<&str>) -> A2ATask {
        A2ATask {
            id: "task-1".to_string(),
            status: TaskStatus::Pending,
            status_reason: None,
            messages: vec![A2AMessage {
                role: "user".to_string(),
                parts: vec![MessagePart::Text {
                    text: prompt.to_string(),
                }],
            }],
            artifacts: vec![],
            retry_count: 0,
            run_id: Some("run-1".to_string()),
            parent_task_id: None,
            role: Some("implementer".to_string()),
            requested_capabilities: requested_capabilities
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
            contract: None,
            idempotency_key: None,
            lease: None,
            progress: None,
            provenance: None,
            audit_log: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    struct TestRuntimeHandle {
        server: Arc<A2AServer>,
        origin: String,
        base_url: String,
    }

    async fn spawn_test_runtime() -> TestRuntimeHandle {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let origin = format!("http://{addr}");
        let server = Arc::new(A2AServer::new(create_gestura_agent_card(&origin)));
        let app = build_runtime_app(server.clone());
        spawn_worker(server.clone(), AppConfig::default());
        spawn_runtime_server(listener, app);
        TestRuntimeHandle {
            server,
            origin: origin.clone(),
            base_url: format!("{origin}/a2a"),
        }
    }

    fn register_test_profile(server: &A2AServer) -> String {
        let mut profile = AgentProfile::new("test-caller", "Test Caller");
        profile.generate_token(1);
        let token = profile.auth_token.clone().expect("auth token");
        let response = server.handle_request(A2ARequest::new(
            "profile/register",
            serde_json::to_value(profile).expect("serialize profile"),
        ));
        assert!(response.error.is_none(), "profile register should succeed");
        token
    }

    async fn wait_for_terminal_status(client: &A2AClient, url: &str, task_id: &str) -> A2ATask {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let task = client
                    .get_task_status(url, task_id)
                    .await
                    .expect("get task status");
                if matches!(
                    task.status,
                    TaskStatus::Completed
                        | TaskStatus::Failed
                        | TaskStatus::Cancelled
                        | TaskStatus::Blocked
                ) {
                    return task;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("task should complete in time")
    }

    async fn wait_for_supervisor_terminal_task(
        orchestrator: &AgentOrchestrator<AgentManager>,
        run_id: &str,
        task_id: &str,
    ) -> gestura_core::orchestrator::SupervisorTaskRecord {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let run = orchestrator
                    .get_supervisor_run(run_id)
                    .await
                    .expect("supervisor run should exist");
                if let Some(record) = run.tasks.iter().find(|record| record.task.id == task_id)
                    && matches!(
                        record.state,
                        SupervisorTaskState::Completed
                            | SupervisorTaskState::Failed
                            | SupervisorTaskState::Blocked
                            | SupervisorTaskState::Cancelled
                    )
                {
                    return record.clone();
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("supervisor task should complete in time")
    }

    fn test_workspace_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "gestura-a2a-runtime-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("create test workspace");
        path
    }

    async fn collect_sse_events(
        events_url: String,
        run_id: String,
        ready_tx: tokio::sync::oneshot::Sender<()>,
    ) -> Vec<gestura_core::a2a::A2ATaskEvent> {
        tokio::time::timeout(std::time::Duration::from_secs(10), async move {
            let response = reqwest::Client::new()
                .get(events_url)
                .send()
                .await
                .expect("open SSE stream")
                .error_for_status()
                .expect("SSE response should be successful");
            let _ = ready_tx.send(());
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut data_lines = Vec::new();
            let mut events = Vec::new();

            while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
                let chunk = chunk.expect("read SSE chunk");
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(newline_index) = buffer.find('\n') {
                    let mut line = buffer.drain(..=newline_index).collect::<String>();
                    if line.ends_with('\n') {
                        line.pop();
                    }
                    if line.ends_with('\r') {
                        line.pop();
                    }

                    if let Some(data) = line.strip_prefix("data: ") {
                        data_lines.push(data.to_string());
                    } else if line.is_empty() && !data_lines.is_empty() {
                        let payload = data_lines.join("\n");
                        data_lines.clear();
                        let event =
                            serde_json::from_str::<gestura_core::a2a::A2ATaskEvent>(&payload)
                                .expect("deserialize SSE event payload");
                        if event.task.run_id.as_deref() == Some(run_id.as_str()) {
                            let is_terminal = matches!(
                                event.kind,
                                A2ATaskEventKind::Completed
                                    | A2ATaskEventKind::Failed
                                    | A2ATaskEventKind::Cancelled
                            );
                            events.push(event);
                            if is_terminal {
                                return events;
                            }
                        }
                    }
                }
            }

            events
        })
        .await
        .expect("SSE stream should produce terminal task event in time")
    }

    #[test]
    fn test_resolve_requested_capabilities_supports_aliases() {
        let resolved = resolve_requested_capabilities(&[
            "filesystem".to_string(),
            "commands".to_string(),
            "search".to_string(),
            "filesystem".to_string(),
        ]);

        assert_eq!(
            resolved.allowed_tools,
            vec![
                "file".to_string(),
                "shell".to_string(),
                "web_search".to_string()
            ]
        );
        assert!(resolved.unsupported.is_empty());
    }

    #[test]
    fn test_build_execution_plan_uses_contract_and_disables_tools_without_grant() {
        let mut task = sample_task("", vec![]);
        task.contract = Some(RemoteTaskContract {
            objective: "Summarize the change".to_string(),
            acceptance_criteria: vec!["Mention the main outcome".to_string()],
            constraints: vec!["Stay concise".to_string()],
            deliverables: vec!["One paragraph".to_string()],
            output_format: Some("text/plain".to_string()),
        });

        let plan = build_execution_plan(&task).expect("build execution plan");
        assert_eq!(plan.route, EmbeddedExecutionRoute::Pipeline);
        assert!(!plan.tools_enabled);
        assert!(plan.input.contains("Objective: Summarize the change"));
        assert!(plan.input.contains("Acceptance criteria:"));
        assert!(plan.input.contains("Constraints:"));
    }

    #[tokio::test]
    async fn test_embedded_runtime_executes_shell_task_end_to_end() {
        let runtime = spawn_test_runtime().await;
        let token = register_test_profile(&runtime.server);
        let client = A2AClient::with_auth(token);

        let card = client
            .discover(&runtime.base_url)
            .await
            .expect("discover embedded runtime");
        assert_eq!(card.url, runtime.base_url);

        let create_request = CreateTaskRequest {
            message: A2AMessage {
                role: "user".to_string(),
                parts: vec![MessagePart::Text {
                    text: "shell: printf a2a-e2e".to_string(),
                }],
            },
            run_id: Some("run-e2e".to_string()),
            parent_task_id: None,
            role: Some("implementer".to_string()),
            requested_capabilities: vec!["commands".to_string()],
            contract: Some(RemoteTaskContract {
                objective: "Run a deterministic shell command".to_string(),
                acceptance_criteria: vec!["Return the exact command output".to_string()],
                constraints: vec!["Do not invoke the LLM pipeline".to_string()],
                deliverables: vec!["Raw command stdout".to_string()],
                output_format: Some("text/plain".to_string()),
            }),
            idempotency_key: Some("e2e-shell-task".to_string()),
            lease_request: None,
            metadata: HashMap::new(),
        };

        let created_task = client
            .create_task_with_request(&runtime.base_url, create_request)
            .await
            .expect("create remote task");
        let final_task =
            wait_for_terminal_status(&client, &runtime.base_url, &created_task.id).await;

        assert_eq!(final_task.status, TaskStatus::Completed);
        assert_eq!(
            final_task.status_reason.as_deref(),
            Some("Embedded GUI runtime completed task")
        );
        assert_eq!(
            final_task
                .progress
                .as_ref()
                .and_then(|progress| progress.percent),
            Some(100)
        );

        let manifest = client
            .list_task_artifacts(&runtime.base_url, &final_task.id)
            .await
            .expect("list task artifacts");
        assert_eq!(manifest.len(), 2);
        assert!(
            manifest
                .iter()
                .any(|artifact| artifact.name == "result.txt")
        );
        assert!(
            manifest
                .iter()
                .any(|artifact| artifact.name == "execution-plan.json")
        );

        let result_artifact = client
            .fetch_task_artifact(&runtime.base_url, &final_task.id, "result.txt")
            .await
            .expect("fetch result artifact");
        let result_text = result_artifact
            .parts
            .iter()
            .find_map(|part| match part {
                MessagePart::Text { text } => Some(text.clone()),
                _ => None,
            })
            .expect("result artifact text");
        assert_eq!(result_text, "a2a-e2e");
        assert_eq!(
            result_artifact.metadata.get("executionRoute"),
            Some(&serde_json::Value::String("direct_shell".to_string()))
        );

        let execution_plan = client
            .fetch_task_artifact(&runtime.base_url, &final_task.id, "execution-plan.json")
            .await
            .expect("fetch execution plan artifact");
        let execution_plan_text = execution_plan
            .parts
            .iter()
            .find_map(|part| match part {
                MessagePart::Text { text } => Some(text.clone()),
                _ => None,
            })
            .expect("execution plan artifact text");
        assert!(execution_plan_text.contains("direct_shell"));
        assert!(execution_plan_text.contains("commands"));
    }

    #[tokio::test]
    async fn test_orchestrator_remote_task_round_trips_through_embedded_runtime() {
        let runtime = spawn_test_runtime().await;
        let auth_token = register_test_profile(&runtime.server);
        let workspace_dir = test_workspace_dir("orchestrator-remote");
        let agent_manager = AgentManager::new(workspace_dir.join("agents.db"));
        let orchestrator = AgentOrchestrator::new_with_workspace_root(
            agent_manager,
            AppConfig::default(),
            Some(workspace_dir.clone()),
        );

        let run_id = format!("run-{}", uuid::Uuid::new_v4());
        let task = DelegatedTask {
            id: format!("task-{}", uuid::Uuid::new_v4()),
            agent_id: "remote-embedded-agent".to_string(),
            prompt: "shell: printf orchestrator-remote-e2e".to_string(),
            context: None,
            required_tools: vec!["shell".to_string()],
            priority: 1,
            session_id: None,
            directive_id: None,
            tracking_task_id: None,
            run_id: Some(run_id.clone()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: false,
            test_required: false,
            workspace_dir: Some(workspace_dir.clone()),
            execution_mode: AgentExecutionMode::Remote,
            environment_id: None,
            remote_target: Some(RemoteAgentTarget {
                url: runtime.base_url.clone(),
                name: Some("embedded-runtime".to_string()),
                auth_token: Some(auth_token),
                capabilities: vec!["shell".to_string()],
            }),
            memory_tags: vec!["integration".to_string()],
            name: Some("Remote shell task".to_string()),
        };

        orchestrator
            .delegate_task(task.clone())
            .await
            .expect("delegate remote task");

        let record = wait_for_supervisor_terminal_task(&orchestrator, &run_id, &task.id).await;
        assert_eq!(record.state, SupervisorTaskState::Completed);

        let result = record.result.expect("supervisor task result");
        assert!(result.success);
        assert_eq!(result.output.trim(), "orchestrator-remote-e2e");

        let remote_execution = record.remote_execution.expect("remote execution mirror");
        assert_eq!(remote_execution.status, "completed");
        assert_eq!(
            remote_execution.status_reason.as_deref(),
            Some("Embedded GUI runtime completed task")
        );
        assert_eq!(
            remote_execution
                .provenance
                .as_ref()
                .and_then(|value| value.caller_agent_id.as_deref()),
            Some("test-caller")
        );
        assert_eq!(
            remote_execution
                .provenance
                .as_ref()
                .map(|value| value.authenticated),
            Some(true)
        );
        assert_eq!(
            remote_execution
                .provenance
                .as_ref()
                .and_then(|value| value.auth_scheme.as_deref()),
            Some("bearer")
        );
        assert_eq!(remote_execution.artifacts.len(), 2);
        assert!(
            remote_execution
                .artifacts
                .iter()
                .any(|artifact| artifact.name == "result.txt")
        );
        assert!(
            remote_execution
                .artifacts
                .iter()
                .any(|artifact| artifact.name == "execution-plan.json")
        );
    }

    #[tokio::test]
    async fn test_embedded_runtime_sse_stream_emits_task_lifecycle_events() {
        let runtime = spawn_test_runtime().await;
        let token = register_test_profile(&runtime.server);
        let client = A2AClient::with_auth(token);
        let run_id = format!("run-sse-{}", uuid::Uuid::new_v4());
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let events_task = tokio::spawn(collect_sse_events(
            format!("{}/a2a/events", runtime.origin),
            run_id.clone(),
            ready_tx,
        ));
        ready_rx
            .await
            .expect("SSE collector should connect before task creation");

        let create_request = CreateTaskRequest {
            message: A2AMessage {
                role: "user".to_string(),
                parts: vec![MessagePart::Text {
                    text: "shell: printf sse-e2e".to_string(),
                }],
            },
            run_id: Some(run_id),
            parent_task_id: None,
            role: Some("implementer".to_string()),
            requested_capabilities: vec!["commands".to_string()],
            contract: None,
            idempotency_key: Some("sse-shell-task".to_string()),
            lease_request: None,
            metadata: HashMap::new(),
        };

        client
            .create_task_with_request(&runtime.base_url, create_request)
            .await
            .expect("create remote task for SSE test");
        let events = events_task
            .await
            .expect("SSE collector task should succeed");

        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, A2ATaskEventKind::Created))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, A2ATaskEventKind::Updated))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, A2ATaskEventKind::Completed))
        );
    }
}
