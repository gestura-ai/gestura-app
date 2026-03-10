use super::environment::{default_cleanup_policy, sanitize_component};
use super::*;
use gestura_core_agents::AgentExecutionMode;
use gestura_core_tools::git::GitTools;

impl<M: OrchestratorAgentManager> AgentOrchestrator<M> {
    pub(crate) fn bootstrap_persisted_state(&self) {
        let Some(root) = self.default_workspace_dir.as_ref() else {
            return;
        };

        let mut runs = load_persisted_runs(root);
        let mut environments: HashMap<String, EnvironmentRecord> =
            load_persisted_environments(root)
                .into_iter()
                .map(|environment| (environment.id.clone(), environment))
                .collect();

        let mut generated = Vec::new();
        for run in &mut runs {
            let run_id = run.id.clone();
            let session_id = run.session_id.clone();
            let workspace_dir = run.workspace_dir.clone();
            for task_record in &mut run.tasks {
                if task_record.environment_id.is_empty() {
                    let record = legacy_environment_record(
                        &run_id,
                        session_id.clone(),
                        workspace_dir.clone(),
                        task_record,
                    );
                    task_record.environment_id = record.id.clone();
                    task_record.environment = record.summary();
                    generated.push(record);
                }
            }
        }
        for record in generated {
            environments.entry(record.id.clone()).or_insert(record);
        }

        let mut index = HashMap::new();
        for run in &runs {
            for task_record in &run.tasks {
                index.insert(task_record.task.id.clone(), run.id.clone());
            }
        }

        if let Ok(mut guard) = self.supervisor_runs.try_lock() {
            *guard = runs.into_iter().map(|run| (run.id.clone(), run)).collect();
        }
        if let Ok(mut guard) = self.environments.try_lock() {
            *guard = environments;
        }
        if let Ok(mut guard) = self.task_run_index.try_lock() {
            *guard = index;
        }

        let _ = self.reconcile_persisted_state_sync();
    }

    pub async fn reconcile_orchestrator_state(&self) -> Result<(), String> {
        self.reconcile_persisted_state().await
    }

    async fn reconcile_persisted_state(&self) -> Result<(), String> {
        let mut runs = self.supervisor_runs.lock().await;
        let mut environments = self.environments.lock().await;
        let mut run_updates = Vec::new();
        let mut environment_updates = Vec::new();
        let observer = self.observer.read().await.clone();

        for run in runs.values_mut() {
            let mut run_changed = false;
            for task_record in &mut run.tasks {
                if matches!(task_record.state, SupervisorTaskState::Running) {
                    task_record.state = SupervisorTaskState::Blocked;
                    if !task_record
                        .blocked_reasons
                        .iter()
                        .any(|reason| reason == "execution interrupted during restart")
                    {
                        task_record
                            .blocked_reasons
                            .push("execution interrupted during restart".to_string());
                    }
                    task_record.updated_at = Utc::now();
                    run_changed = true;
                }

                if let Some(environment) = environments.get_mut(&task_record.environment_id) {
                    let mut recovery_summary = None;
                    reconcile_environment_record(environment, Some(task_record));
                    task_record.environment = environment.summary();
                    task_record.updated_at = Utc::now();
                    if task_record.environment.recovery_action.is_some() {
                        run_changed = true;
                    }
                    if let Some(action) = task_record.environment.recovery_action {
                        recovery_summary = Some((action, recovery_message(environment)));
                    }
                    environment_updates.push(environment.clone());
                    if let (Some(observer), Some((action, summary))) = (&observer, recovery_summary)
                    {
                        let observer = observer.clone();
                        let environment_id = environment.id.clone();
                        tokio::spawn(async move {
                            observer
                                .on_environment_recovery(environment_id, action, summary)
                                .await;
                        });
                    }
                }
            }

            if run_changed {
                run.status = recalculate_run_status(run);
                run.updated_at = Utc::now();
                run_updates.push(run.clone());
            }
        }

        for environment in environments.values_mut() {
            let owned = runs.values().any(|run| {
                run.tasks
                    .iter()
                    .any(|task_record| task_record.environment_id == environment.id)
            });
            if !owned {
                environment.health = EnvironmentHealth::Orphaned;
                environment.recovery_status = RecoveryStatus::Pending;
                environment.recovery_action = Some(RecoveryAction::QueueCleanup);
                environment.updated_at = Utc::now();
                environment_updates.push(environment.clone());
                if let Some(observer) = &observer {
                    let observer = observer.clone();
                    let environment_id = environment.id.clone();
                    let summary = recovery_message(environment);
                    tokio::spawn(async move {
                        observer
                            .on_environment_recovery(
                                environment_id,
                                RecoveryAction::QueueCleanup,
                                summary,
                            )
                            .await;
                    });
                }
            }
        }

        drop(environments);
        drop(runs);

        for run in run_updates {
            self.persist_run(&run)?;
        }
        for environment in environment_updates {
            self.persist_environment_record(&environment).await?;
        }

        Ok(())
    }

    fn reconcile_persisted_state_sync(&self) -> Result<(), String> {
        let Ok(mut runs) = self.supervisor_runs.try_lock() else {
            return Ok(());
        };
        let Ok(mut environments) = self.environments.try_lock() else {
            return Ok(());
        };

        let mut run_updates = Vec::new();
        let mut environment_updates = Vec::new();

        for run in runs.values_mut() {
            let mut run_changed = false;
            for task_record in &mut run.tasks {
                if matches!(task_record.state, SupervisorTaskState::Running) {
                    task_record.state = SupervisorTaskState::Blocked;
                    if !task_record
                        .blocked_reasons
                        .iter()
                        .any(|reason| reason == "execution interrupted during restart")
                    {
                        task_record
                            .blocked_reasons
                            .push("execution interrupted during restart".to_string());
                    }
                    task_record.updated_at = Utc::now();
                    run_changed = true;
                }

                if let Some(environment) = environments.get_mut(&task_record.environment_id) {
                    reconcile_environment_record(environment, Some(task_record));
                    task_record.environment = environment.summary();
                    task_record.updated_at = Utc::now();
                    if task_record.environment.recovery_action.is_some() {
                        run_changed = true;
                    }
                    environment_updates.push(environment.clone());
                }
            }

            if run_changed {
                run.status = recalculate_run_status(run);
                run.updated_at = Utc::now();
                run_updates.push(run.clone());
            }
        }

        for environment in environments.values_mut() {
            let owned = runs.values().any(|run| {
                run.tasks
                    .iter()
                    .any(|task_record| task_record.environment_id == environment.id)
            });
            if !owned {
                environment.health = EnvironmentHealth::Orphaned;
                environment.recovery_status = RecoveryStatus::Pending;
                environment.recovery_action = Some(RecoveryAction::QueueCleanup);
                environment.updated_at = Utc::now();
                environment_updates.push(environment.clone());
            }
        }

        drop(environments);
        drop(runs);

        for run in run_updates {
            self.persist_run(&run)?;
        }
        for environment in environment_updates {
            persist_environment_to_disk(&environment.spec.workspace_root, &environment)?;
        }

        Ok(())
    }
}

fn legacy_environment_record(
    run_id: &str,
    session_id: Option<String>,
    workspace_dir: Option<PathBuf>,
    task_record: &SupervisorTaskRecord,
) -> EnvironmentRecord {
    let now = Utc::now();
    let environment_id = format!(
        "env_{}_{}_{}",
        sanitize_component(run_id),
        sanitize_component(&task_record.task.id),
        sanitize_component(&task_record.task.agent_id),
    );

    EnvironmentRecord {
        id: environment_id.clone(),
        spec: EnvironmentSpec {
            id: environment_id,
            execution_mode: task_record.environment.execution_mode.clone(),
            workspace_root: task_record
                .task
                .workspace_dir
                .clone()
                .or(workspace_dir)
                .unwrap_or_else(|| task_record.environment.root_dir.clone()),
            prepared_path: task_record.environment.root_dir.clone(),
            session_id,
            run_id: run_id.to_string(),
            task_id: task_record.task.id.clone(),
            agent_id: task_record.task.agent_id.clone(),
            cleanup_policy: default_cleanup_policy(&task_record.environment.execution_mode),
            write_access: task_record.environment.write_access,
            git_worktree: task_record
                .environment
                .worktree_path
                .as_ref()
                .map(|worktree_path| GitWorktreeSpec {
                    repo_root: task_record
                        .task
                        .workspace_dir
                        .clone()
                        .unwrap_or_else(|| task_record.environment.root_dir.clone()),
                    base_branch: task_record
                        .environment
                        .branch_name
                        .clone()
                        .unwrap_or_else(|| "main".to_string()),
                    worktree_branch: task_record
                        .environment
                        .branch_name
                        .clone()
                        .unwrap_or_else(|| "main".to_string()),
                    worktree_path: worktree_path.clone(),
                    create_branch_if_missing: true,
                }),
            remote_url: task_record.environment.remote_url.clone(),
        },
        state: if matches!(task_record.state, SupervisorTaskState::Running) {
            EnvironmentState::InUse
        } else {
            EnvironmentState::Ready
        },
        health: task_record.environment.health,
        prepared_path: task_record.environment.root_dir.clone(),
        lease: None,
        cleanup_result: task_record.environment.cleanup_result.clone(),
        recovery_status: task_record.environment.recovery_status,
        recovery_action: task_record.environment.recovery_action,
        failure: task_record.environment.failure.clone(),
        created_at: task_record.created_at,
        updated_at: task_record.updated_at,
        last_verified_at: Some(now),
        metadata: None,
    }
}

fn reconcile_environment_record(
    environment: &mut EnvironmentRecord,
    task_record: Option<&mut SupervisorTaskRecord>,
) {
    let path_exists = environment.prepared_path.exists();
    let mut recovery_action = None;

    if let Some(lease) = environment.lease.as_mut()
        && lease.released_at.is_none()
    {
        lease.released_at = Some(Utc::now());
        recovery_action = Some(RecoveryAction::ReleaseStaleLease);
    }

    match environment.spec.execution_mode {
        AgentExecutionMode::GitWorktree => {
            if !path_exists {
                environment.health = EnvironmentHealth::Missing;
                environment.recovery_status = RecoveryStatus::Pending;
                environment.recovery_action = Some(RecoveryAction::RecreateMissingEnvironment);
            } else if let Some(spec) = environment.spec.git_worktree.as_ref() {
                let git = GitTools::new(Some(spec.repo_root.clone()));
                let worktree_ok = git
                    .worktree_list()
                    .map(|worktrees| {
                        worktrees
                            .into_iter()
                            .any(|worktree| worktree.path == spec.worktree_path)
                    })
                    .unwrap_or(false);
                if !worktree_ok {
                    environment.health = EnvironmentHealth::Drifted;
                    environment.recovery_status = RecoveryStatus::NeedsOperatorAction;
                    environment.recovery_action = Some(RecoveryAction::MarkTaskBlocked);
                } else {
                    environment.health = git
                        .is_worktree_clean(&spec.worktree_path)
                        .map(|clean| {
                            if clean {
                                EnvironmentHealth::Clean
                            } else {
                                EnvironmentHealth::Dirty
                            }
                        })
                        .unwrap_or(EnvironmentHealth::Unknown);
                    environment.recovery_status = RecoveryStatus::Reconciled;
                    environment.recovery_action = recovery_action;
                    if !matches!(
                        environment.state,
                        EnvironmentState::Archived
                            | EnvironmentState::Removed
                            | EnvironmentState::Failed
                    ) {
                        environment.state = EnvironmentState::Ready;
                    }
                }
            }
        }
        AgentExecutionMode::IsolatedWorkspace => {
            if !path_exists {
                environment.health = EnvironmentHealth::Missing;
                environment.recovery_status = RecoveryStatus::Pending;
                environment.recovery_action = Some(RecoveryAction::RecreateMissingEnvironment);
            } else {
                environment.health = EnvironmentHealth::Clean;
                environment.recovery_status = RecoveryStatus::Reconciled;
                environment.recovery_action = recovery_action;
                if !matches!(
                    environment.state,
                    EnvironmentState::Archived
                        | EnvironmentState::Removed
                        | EnvironmentState::Failed
                ) {
                    environment.state = EnvironmentState::Ready;
                }
            }
        }
        AgentExecutionMode::SharedWorkspace | AgentExecutionMode::Remote => {
            environment.health = if path_exists {
                EnvironmentHealth::Clean
            } else {
                EnvironmentHealth::Missing
            };
            environment.recovery_status = if path_exists {
                RecoveryStatus::Reconciled
            } else {
                RecoveryStatus::NeedsOperatorAction
            };
            environment.recovery_action = recovery_action.or({
                if path_exists {
                    None
                } else {
                    Some(RecoveryAction::MarkTaskBlocked)
                }
            });
            if path_exists
                && !matches!(
                    environment.state,
                    EnvironmentState::Archived
                        | EnvironmentState::Removed
                        | EnvironmentState::Failed
                )
            {
                environment.state = EnvironmentState::Ready;
            }
        }
    }

    if let Some(task_record) = task_record
        && environment.recovery_action.is_some()
    {
        task_record.state = SupervisorTaskState::Blocked;
        if !task_record
            .blocked_reasons
            .iter()
            .any(|reason| reason == &recovery_message(environment))
        {
            task_record
                .blocked_reasons
                .push(recovery_message(environment));
        }
    }

    environment.updated_at = Utc::now();
    environment.last_verified_at = Some(Utc::now());
}

fn recovery_message(environment: &EnvironmentRecord) -> String {
    match environment.recovery_action {
        Some(RecoveryAction::RecreateMissingEnvironment) => format!(
            "environment {} is missing and must be recreated",
            environment.id
        ),
        Some(RecoveryAction::ReleaseStaleLease) => format!(
            "stale lease released for environment {} after restart",
            environment.id
        ),
        Some(RecoveryAction::ArchiveDirtyEnvironment) => format!(
            "environment {} is dirty and should be archived",
            environment.id
        ),
        Some(RecoveryAction::QueueCleanup) => format!(
            "environment {} is orphaned and queued for cleanup",
            environment.id
        ),
        Some(RecoveryAction::MarkTaskBlocked) => format!(
            "environment {} drifted and blocked the owning task",
            environment.id
        ),
        _ => format!("environment {} reconciled", environment.id),
    }
}
