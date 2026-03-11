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
        let checkpoints_by_task = self
            .default_workspace_dir
            .as_deref()
            .map(load_persisted_checkpoints)
            .unwrap_or_default()
            .into_iter()
            .map(|checkpoint| (checkpoint.task_id.clone(), checkpoint))
            .collect::<HashMap<_, _>>();
        let mut runs = self.supervisor_runs.lock().await;
        let mut environments = self.environments.lock().await;
        let mut run_updates = Vec::new();
        let mut environment_updates = Vec::new();
        let mut checkpoint_updates = Vec::new();
        let observer = self.observer.read().await.clone();

        for run in runs.values_mut() {
            let mut run_changed = false;
            for task_record in &mut run.tasks {
                if matches!(task_record.state, SupervisorTaskState::Running) {
                    let checkpoint = checkpoints_by_task.get(&task_record.task.id);
                    let blocked_reason = checkpoint
                        .map(restart_blocked_reason_for_checkpoint)
                        .unwrap_or_else(|| "execution interrupted during restart".to_string());
                    task_record.state = SupervisorTaskState::Blocked;
                    if !task_record
                        .blocked_reasons
                        .iter()
                        .any(|reason| reason == &blocked_reason)
                    {
                        task_record.blocked_reasons.push(blocked_reason);
                    }
                    task_record.updated_at = Utc::now();
                    if let Some(checkpoint) = checkpoint {
                        checkpoint_updates.push(checkpoint_for_restart_recovery(checkpoint));
                    }
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
        for checkpoint in checkpoint_updates {
            self.persist_delegated_checkpoint(&checkpoint)?;
        }

        Ok(())
    }

    fn reconcile_persisted_state_sync(&self) -> Result<(), String> {
        let checkpoints_by_task = self
            .default_workspace_dir
            .as_deref()
            .map(load_persisted_checkpoints)
            .unwrap_or_default()
            .into_iter()
            .map(|checkpoint| (checkpoint.task_id.clone(), checkpoint))
            .collect::<HashMap<_, _>>();
        let Ok(mut runs) = self.supervisor_runs.try_lock() else {
            return Ok(());
        };
        let Ok(mut environments) = self.environments.try_lock() else {
            return Ok(());
        };

        let mut run_updates = Vec::new();
        let mut environment_updates = Vec::new();
        let mut checkpoint_updates = Vec::new();

        for run in runs.values_mut() {
            let mut run_changed = false;
            for task_record in &mut run.tasks {
                if matches!(task_record.state, SupervisorTaskState::Running) {
                    let checkpoint = checkpoints_by_task.get(&task_record.task.id);
                    let blocked_reason = checkpoint
                        .map(restart_blocked_reason_for_checkpoint)
                        .unwrap_or_else(|| "execution interrupted during restart".to_string());
                    task_record.state = SupervisorTaskState::Blocked;
                    if !task_record
                        .blocked_reasons
                        .iter()
                        .any(|reason| reason == &blocked_reason)
                    {
                        task_record.blocked_reasons.push(blocked_reason);
                    }
                    task_record.updated_at = Utc::now();
                    if let Some(checkpoint) = checkpoint {
                        checkpoint_updates.push(checkpoint_for_restart_recovery(checkpoint));
                    }
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
        for checkpoint in checkpoint_updates {
            self.persist_delegated_checkpoint(&checkpoint)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppConfig;
    use gestura_core_agents::{AgentManager, AgentRole};
    use std::process::Command;
    use tempfile::tempdir;

    fn test_orchestrator(workspace_root: &Path) -> AgentOrchestrator<AgentManager> {
        AgentOrchestrator::new_with_workspace_root(
            AgentManager::new(workspace_root.join("recovery-tests.db")),
            AppConfig::default(),
            Some(workspace_root.to_path_buf()),
        )
    }

    fn test_task(
        workspace_root: &Path,
        run_id: &str,
        task_id: &str,
        agent_id: &str,
        execution_mode: AgentExecutionMode,
    ) -> DelegatedTask {
        DelegatedTask {
            id: task_id.to_string(),
            agent_id: agent_id.to_string(),
            prompt: format!("Execute {task_id}"),
            context: None,
            required_tools: vec![],
            priority: 1,
            session_id: Some("session-recovery".to_string()),
            directive_id: None,
            tracking_task_id: None,
            run_id: Some(run_id.to_string()),
            parent_task_id: None,
            depends_on: vec![],
            role: Some(AgentRole::Implementer),
            delegation_brief: None,
            planning_only: false,
            approval_required: false,
            reviewer_required: false,
            test_required: false,
            workspace_dir: Some(workspace_root.to_path_buf()),
            execution_mode,
            environment_id: None,
            remote_target: None,
            memory_tags: vec!["recovery-test".to_string()],
            name: Some(task_id.to_string()),
        }
    }

    fn test_task_record(
        task: DelegatedTask,
        environment: &EnvironmentRecord,
        state: SupervisorTaskState,
    ) -> SupervisorTaskRecord {
        let now = Utc::now();
        SupervisorTaskRecord {
            task,
            state,
            approval: TaskApprovalRecord::default(),
            environment_id: environment.id.clone(),
            environment: environment.summary(),
            claimed_by: None,
            attempts: 0,
            blocked_reasons: vec![],
            result: None,
            remote_execution: None,
            messages: vec![],
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
        }
    }

    fn test_checkpoint(task: &DelegatedTask) -> DelegatedTaskCheckpoint {
        let now = Utc::now();
        DelegatedTaskCheckpoint {
            id: delegated_checkpoint_id(&task.id),
            task_id: task.id.clone(),
            run_id: task.run_id.clone(),
            session_id: task.session_id.clone(),
            agent_id: task.agent_id.clone(),
            environment_id: task.environment_id.clone(),
            execution_mode: task.execution_mode.clone(),
            stage: DelegatedCheckpointStage::Running,
            replay_safety: DelegatedReplaySafety::CheckpointResumable,
            resume_disposition: DelegatedResumeDisposition::ResumeFromCheckpoint,
            safe_boundary_label: "delegated task dispatch boundary".to_string(),
            workspace_dir: task.workspace_dir.clone(),
            completed_tool_calls: Vec::new(),
            result_published: false,
            note: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_run(
        run_id: &str,
        workspace_root: &Path,
        task_record: SupervisorTaskRecord,
    ) -> SupervisorRun {
        let now = Utc::now();
        SupervisorRun {
            id: run_id.to_string(),
            name: Some(format!("Run {run_id}")),
            session_id: Some("session-recovery".to_string()),
            workspace_dir: Some(workspace_root.to_path_buf()),
            lead_agent_id: Some("supervisor-1".to_string()),
            parent_run: None,
            child_runs: vec![],
            hierarchy_depth: 0,
            max_hierarchy_depth: default_max_child_supervisor_depth(),
            inherited_policy: None,
            status: SupervisorRunStatus::Running,
            task_summary: SupervisorRunTaskSummary {
                total: 1,
                running: 1,
                ..SupervisorRunTaskSummary::default()
            },
            hierarchy_summary: None,
            tasks: vec![task_record],
            messages: vec![],
            created_at: now,
            updated_at: now,
            completed_at: None,
            metadata: None,
        }
    }

    fn test_environment_record(
        workspace_root: &Path,
        prepared_path: PathBuf,
        execution_mode: AgentExecutionMode,
        git_worktree: Option<GitWorktreeSpec>,
    ) -> EnvironmentRecord {
        let now = Utc::now();
        EnvironmentRecord {
            id: format!(
                "env-{}",
                sanitize_component(prepared_path.to_string_lossy().as_ref())
            ),
            spec: EnvironmentSpec {
                id: "env-spec".to_string(),
                execution_mode,
                workspace_root: workspace_root.to_path_buf(),
                prepared_path: prepared_path.clone(),
                session_id: Some("session-recovery".to_string()),
                run_id: "run-recovery".to_string(),
                task_id: "task-recovery".to_string(),
                agent_id: "agent-recovery".to_string(),
                cleanup_policy: CleanupPolicy::RemoveWhenCleanOtherwiseArchive,
                write_access: true,
                git_worktree,
                remote_url: None,
            },
            state: EnvironmentState::Ready,
            health: EnvironmentHealth::Unknown,
            prepared_path,
            lease: None,
            cleanup_result: None,
            recovery_status: RecoveryStatus::NotRequired,
            recovery_action: None,
            failure: None,
            created_at: now,
            updated_at: now,
            last_verified_at: None,
            metadata: None,
        }
    }

    fn run_git(repo_root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo_root)
            .args(args)
            .status()
            .expect("run git command");
        assert!(
            status.success(),
            "git command failed: git {}",
            args.join(" ")
        );
    }

    fn init_git_repo(repo_root: &Path) {
        run_git(repo_root, &["init", "--initial-branch=main"]);
        run_git(
            repo_root,
            &["config", "user.email", "gestura-tests@example.com"],
        );
        run_git(repo_root, &["config", "user.name", "Gestura Tests"]);
        std::fs::write(repo_root.join("README.md"), "seed\n").expect("write seed file");
        run_git(repo_root, &["add", "README.md"]);
        run_git(repo_root, &["commit", "-m", "Initial commit"]);
    }

    #[test]
    fn test_reconcile_environment_record_blocks_missing_shared_workspace() {
        let temp = tempdir().expect("tempdir");
        let mut environment = test_environment_record(
            temp.path(),
            temp.path().join("missing-shared"),
            AgentExecutionMode::SharedWorkspace,
            None,
        );
        let task = test_task(
            temp.path(),
            "run-shared-missing",
            "task-shared-missing",
            "agent-shared",
            AgentExecutionMode::SharedWorkspace,
        );
        let mut task_record = test_task_record(task, &environment, SupervisorTaskState::Queued);

        reconcile_environment_record(&mut environment, Some(&mut task_record));

        assert_eq!(environment.health, EnvironmentHealth::Missing);
        assert_eq!(
            environment.recovery_status,
            RecoveryStatus::NeedsOperatorAction
        );
        assert_eq!(
            environment.recovery_action,
            Some(RecoveryAction::MarkTaskBlocked)
        );
        assert_eq!(task_record.state, SupervisorTaskState::Blocked);
        assert!(
            task_record
                .blocked_reasons
                .iter()
                .any(|reason| reason.contains("blocked"))
        );
    }

    #[test]
    fn test_reconcile_environment_record_marks_missing_isolated_workspace_for_recreation() {
        let temp = tempdir().expect("tempdir");
        let mut environment = test_environment_record(
            temp.path(),
            temp.path().join("missing-isolated"),
            AgentExecutionMode::IsolatedWorkspace,
            None,
        );
        let task = test_task(
            temp.path(),
            "run-isolated-missing",
            "task-isolated-missing",
            "agent-isolated",
            AgentExecutionMode::IsolatedWorkspace,
        );
        let mut task_record = test_task_record(task, &environment, SupervisorTaskState::Queued);

        reconcile_environment_record(&mut environment, Some(&mut task_record));

        assert_eq!(environment.health, EnvironmentHealth::Missing);
        assert_eq!(environment.recovery_status, RecoveryStatus::Pending);
        assert_eq!(
            environment.recovery_action,
            Some(RecoveryAction::RecreateMissingEnvironment)
        );
        assert_eq!(task_record.state, SupervisorTaskState::Blocked);
        assert!(
            task_record
                .blocked_reasons
                .iter()
                .any(|reason| reason.contains("must be recreated"))
        );
    }

    #[test]
    fn test_reconcile_environment_record_marks_unregistered_worktree_as_drifted() {
        let temp = tempdir().expect("tempdir");
        init_git_repo(temp.path());
        let drifted_path = temp.path().join(".gestura").join("drifted-worktree");
        std::fs::create_dir_all(&drifted_path).expect("create drifted worktree path");
        let git_worktree = GitWorktreeSpec {
            repo_root: temp.path().to_path_buf(),
            base_branch: "main".to_string(),
            worktree_branch: "gestura/session-recovery/run/agent/task".to_string(),
            worktree_path: drifted_path.clone(),
            create_branch_if_missing: true,
        };
        let mut environment = test_environment_record(
            temp.path(),
            drifted_path,
            AgentExecutionMode::GitWorktree,
            Some(git_worktree),
        );
        let task = test_task(
            temp.path(),
            "run-worktree-drifted",
            "task-worktree-drifted",
            "agent-worktree",
            AgentExecutionMode::GitWorktree,
        );
        let mut task_record = test_task_record(task, &environment, SupervisorTaskState::Queued);

        reconcile_environment_record(&mut environment, Some(&mut task_record));

        assert_eq!(environment.health, EnvironmentHealth::Drifted);
        assert_eq!(
            environment.recovery_status,
            RecoveryStatus::NeedsOperatorAction
        );
        assert_eq!(
            environment.recovery_action,
            Some(RecoveryAction::MarkTaskBlocked)
        );
        assert_eq!(task_record.state, SupervisorTaskState::Blocked);
        assert!(
            task_record
                .blocked_reasons
                .iter()
                .any(|reason| reason.contains("drifted"))
        );
    }

    #[tokio::test]
    async fn test_bootstrap_persisted_state_reconciles_restart_and_orphaned_environments() {
        let temp = tempdir().expect("tempdir");
        let orchestrator = test_orchestrator(temp.path());

        let task = test_task(
            temp.path(),
            "run-restart",
            "task-restart",
            "agent-restart",
            AgentExecutionMode::IsolatedWorkspace,
        );
        let environment = orchestrator
            .prepare_environment(&task)
            .await
            .expect("prepare restart environment");
        let leased = orchestrator
            .acquire_environment_lease(&environment.id, &task.id, &task.agent_id)
            .await
            .expect("acquire environment lease");

        let run = test_run(
            "run-restart",
            temp.path(),
            test_task_record(task.clone(), &leased, SupervisorTaskState::Running),
        );
        orchestrator.persist_run(&run).expect("persist run");

        let orphan = orchestrator
            .prepare_environment(&test_task(
                temp.path(),
                "run-orphan",
                "task-orphan",
                "agent-orphan",
                AgentExecutionMode::IsolatedWorkspace,
            ))
            .await
            .expect("prepare orphan environment");

        let recovered = test_orchestrator(temp.path());
        let environments = recovered.environments.lock().await;
        let recovered_environment = environments
            .get(&leased.id)
            .expect("recovered environment should exist");
        let orphan_environment = environments
            .get(&orphan.id)
            .expect("orphan environment should exist");

        assert_eq!(
            recovered_environment.recovery_status,
            RecoveryStatus::Reconciled
        );
        assert_eq!(
            recovered_environment.recovery_action,
            Some(RecoveryAction::ReleaseStaleLease)
        );
        assert_eq!(recovered_environment.state, EnvironmentState::Ready);
        assert!(
            recovered_environment
                .lease
                .as_ref()
                .and_then(|lease| lease.released_at)
                .is_some()
        );

        assert_eq!(orphan_environment.health, EnvironmentHealth::Orphaned);
        assert_eq!(orphan_environment.recovery_status, RecoveryStatus::Pending);
        assert_eq!(
            orphan_environment.recovery_action,
            Some(RecoveryAction::QueueCleanup)
        );
        drop(environments);

        let runs = recovered.supervisor_runs.lock().await;
        let run = runs.get("run-restart").expect("recovered run should exist");
        let record = run.tasks.first().expect("recovered task should exist");
        assert_eq!(record.state, SupervisorTaskState::Blocked);
        assert!(
            record
                .blocked_reasons
                .iter()
                .any(|reason| reason == "execution interrupted during restart")
        );
        assert!(
            record
                .blocked_reasons
                .iter()
                .any(|reason| reason.contains("stale lease released"))
        );
    }

    #[tokio::test]
    async fn test_bootstrap_persisted_state_uses_checkpoint_metadata_for_restart_reason() {
        let temp = tempdir().expect("tempdir");
        let orchestrator = test_orchestrator(temp.path());

        let task = test_task(
            temp.path(),
            "run-resume",
            "task-resume",
            "agent-resume",
            AgentExecutionMode::IsolatedWorkspace,
        );
        let environment = orchestrator
            .prepare_environment(&task)
            .await
            .expect("prepare resume environment");
        let leased = orchestrator
            .acquire_environment_lease(&environment.id, &task.id, &task.agent_id)
            .await
            .expect("acquire environment lease");

        let run = test_run(
            "run-resume",
            temp.path(),
            test_task_record(task.clone(), &leased, SupervisorTaskState::Running),
        );
        orchestrator.persist_run(&run).expect("persist run");
        persist_checkpoint_to_disk(temp.path(), &test_checkpoint(&task))
            .expect("persist checkpoint");

        let recovered = test_orchestrator(temp.path());

        let runs = recovered.supervisor_runs.lock().await;
        let run = runs.get("run-resume").expect("recovered run should exist");
        let record = run.tasks.first().expect("recovered task should exist");
        assert_eq!(record.state, SupervisorTaskState::Blocked);
        assert!(record.blocked_reasons.iter().any(|reason| {
            reason.contains("can resume from checkpoint")
                && reason.contains("delegated task dispatch boundary")
        }));
        drop(runs);

        let checkpoints = load_persisted_checkpoints(temp.path());
        let checkpoint = checkpoints
            .into_iter()
            .find(|checkpoint| checkpoint.task_id == task.id)
            .expect("checkpoint should persist");
        assert_eq!(checkpoint.stage, DelegatedCheckpointStage::Blocked);
        assert_eq!(
            checkpoint.resume_disposition,
            DelegatedResumeDisposition::ResumeFromCheckpoint
        );
        assert_eq!(
            checkpoint.note.as_deref(),
            Some("execution interrupted during restart")
        );
    }
}
